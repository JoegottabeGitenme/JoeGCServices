//! GRIB2 file ingestion logic.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;
use tracing::{debug, info, warn};
use zarrs_filesystem::FilesystemStore;

use grid_processor::{
    BoundingBox as GpBoundingBox, DownsampleMethod, GridProcessorConfig, PyramidConfig, RowOrigin,
    ZarrWriter,
};
use projection::LambertConformal;
use storage::{Catalog, CatalogEntry, ObjectStorage};

use crate::error::{IngestionError, Result};
use crate::metadata::{get_bbox_from_grid, get_model_bbox};
use crate::tables::{build_filter_for_model, build_tables_for_model};
use crate::upload::upload_zarr_directory;
use crate::{IngestOptions, IngestionResult};

/// Ingest a GRIB2 file into Zarr format.
///
/// Parses the GRIB2 file, extracts target parameters, writes Zarr pyramids,
/// uploads to object storage, and registers in the catalog.
pub async fn ingest_grib2(
    storage: &Arc<ObjectStorage>,
    catalog: &Catalog,
    data: Bytes,
    file_path: &str,
    options: &IngestOptions,
) -> Result<IngestionResult> {
    let model = options
        .model
        .clone()
        .or_else(|| crate::metadata::extract_model_from_filename(file_path))
        .ok_or_else(|| IngestionError::MissingMetadata("Could not determine model".into()))?;

    let forecast_hour = options
        .forecast_hour
        .or_else(|| crate::metadata::extract_forecast_hour(file_path))
        .unwrap_or(0);

    info!(
        model = %model,
        forecast_hour = forecast_hour,
        file_size = data.len(),
        "Ingesting GRIB2 file"
    );

    // Build GRIB2 lookup tables from model config
    let tables = build_tables_for_model(&model);

    // Build ingestion filter from model config (fail-fast if config is missing/invalid)
    let filter = build_filter_for_model(&model)?;

    // Parse GRIB2 data
    let mut reader = grib2_parser::Grib2Reader::new(data, tables);

    // Track registered parameters
    let mut registered_params: HashSet<String> = HashSet::new();
    let mut datasets_registered = 0usize;
    let mut bytes_written = 0u64;
    let mut grib_reference_time: Option<DateTime<Utc>> = None;
    let mut registered_param_names: HashSet<String> = HashSet::new();

    while let Some(message) = reader.next_message().ok().flatten() {
        // Extract reference time from first message
        if grib_reference_time.is_none() {
            grib_reference_time = Some(message.identification.reference_time);
            info!(
                reference_time = %message.identification.reference_time,
                "Extracted reference time"
            );
        }

        let param = &message.product_definition.parameter_short_name;
        let level = &message.product_definition.level_description;
        let level_type = message.product_definition.level_type;
        let level_value = message.product_definition.level_value;

        let param_level_key = format!("{}:{}", param, level);

        // Check if we should register this parameter (config-driven filtering)
        let should_register = !registered_params.contains(&param_level_key)
            && filter.should_ingest(param, level_type, level_value);

        if !should_register {
            continue;
        }

        let reference_time = grib_reference_time.unwrap_or_else(Utc::now);

        // Sanitize level for path
        let level_sanitized = level.replace([' ', '/'], "_").to_lowercase();

        // Storage path format
        let zarr_storage_path = build_storage_path(
            &model,
            &reference_time,
            param,
            &level_sanitized,
            forecast_hour,
        );

        // Extract grid dimensions
        let width = message.grid_definition.num_points_longitude as usize;
        let height = message.grid_definition.num_points_latitude as usize;

        // Unpack the grid data
        let grid_data = match message.unpack_data() {
            Ok(data) => data,
            Err(e) => {
                warn!(error = %e, param = %param, "Failed to unpack GRIB2 data, skipping");
                continue;
            }
        };

        // Get valid_range for this parameter (required - config validation ensures it exists)
        let valid_range = match filter.get_valid_range(param) {
            Some(range) => range,
            None => {
                // This should not happen if config is valid, but handle gracefully
                warn!(
                    param = %param,
                    "Missing valid_range in config, skipping parameter"
                );
                continue;
            }
        };

        // Convert values outside valid_range to NaN (sentinel value handling)
        let mut out_of_range_count = 0usize;
        let grid_data: Vec<f32> = grid_data
            .into_iter()
            .map(|v| {
                if valid_range.is_valid(v) {
                    v
                } else {
                    out_of_range_count += 1;
                    f32::NAN
                }
            })
            .collect();

        // Log warning if significant portion of data was out of range
        let total_points = grid_data.len();
        let out_of_range_pct = (out_of_range_count as f64 / total_points as f64) * 100.0;
        if out_of_range_count > 0 && out_of_range_pct > 0.1 {
            warn!(
                param = %param,
                level = %level,
                out_of_range = out_of_range_count,
                total = total_points,
                pct = format!("{:.2}%", out_of_range_pct),
                valid_min = valid_range.min,
                valid_max = valid_range.max,
                "Values outside valid_range converted to NaN - check valid_range config"
            );
        }

        if grid_data.len() != width * height {
            warn!(
                expected = width * height,
                actual = grid_data.len(),
                param = %param,
                "Grid data size mismatch, skipping"
            );
            continue;
        }

        // Handle scanning mode differences between GRIB sources and our storage convention.
        //
        // The `grib` crate automatically normalizes scanning modes when decoding values.
        // NDFD uses scan mode 80 (0x50) in the raw file, but the decoded data is already
        // in standard WE:SN order (j=0 at south, j increases going north).
        //
        // Our Lambert projection convention matches this:
        // - i=0 is westernmost column, i increases going east
        // - j=0 is southernmost row, j increases going north
        // - Data index = j * width + i
        //
        // NO transformation is needed for NDFD data.

        // Log diagnostic values to verify correct indexing
        if model == "ndfd" {
            // LA at (-118.2°, 34°N): grid i=239, j=572
            // NY at (-74°, 41°N): grid i=1810, j=856
            let la_idx = 572 * width + 239;
            let ny_idx = 856 * width + 1810;
            let la_val = grid_data.get(la_idx).copied().unwrap_or(f32::NAN);
            let ny_val = grid_data.get(ny_idx).copied().unwrap_or(f32::NAN);
            info!(
                param = %param,
                la_idx = la_idx,
                la_val = la_val,
                ny_idx = ny_idx,
                ny_val = ny_val,
                width = width,
                height = height,
                "NDFD diagnostic: values at known coordinates (LA should be ~68, NY should be ~85 for SKY)"
            );
        }

        // Calculate bounding box and row origin
        // HRRR and NDFD use Lambert Conformal projection where row 0 is at the south (min_lat)
        // Standard geographic grids (GFS, MRMS) have row 0 at the north (max_lat)
        let (gp_bbox, row_origin) = if model == "hrrr" {
            let proj = LambertConformal::hrrr();
            let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
            (
                GpBoundingBox::new(min_lon, min_lat, max_lon, max_lat),
                RowOrigin::South,
            )
        } else if model == "ndfd" {
            let proj = LambertConformal::ndfd();
            let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
            (
                GpBoundingBox::new(min_lon, min_lat, max_lon, max_lat),
                RowOrigin::South,
            )
        } else {
            let grib_bbox = get_bbox_from_grid(&message.grid_definition);
            (
                GpBoundingBox::new(
                    grib_bbox.min_x,
                    grib_bbox.min_y,
                    grib_bbox.max_x,
                    grib_bbox.max_y,
                ),
                RowOrigin::North,
            )
        };

        // Get units from config (defaults to "unknown" if not specified)
        let units = filter.get_units(param);

        // Write Zarr and upload
        match write_and_upload_zarr(
            storage,
            &grid_data,
            width,
            height,
            &gp_bbox,
            &model,
            param,
            level,
            units,
            reference_time,
            forecast_hour,
            &zarr_storage_path,
            row_origin,
        )
        .await
        {
            Ok((zarr_file_size, zarr_metadata)) => {
                // Register in catalog
                let bbox = get_model_bbox(&model);
                let entry = CatalogEntry {
                    model: model.clone(),
                    parameter: param.to_string(),
                    level: level.clone(),
                    reference_time,
                    forecast_hour,
                    bbox,
                    storage_path: zarr_storage_path,
                    file_size: zarr_file_size,
                    zarr_metadata: Some(zarr_metadata),
                };

                match catalog.register_dataset(&entry).await {
                    Ok(id) => {
                        debug!(id = %id, param = %param, level = %level, "Registered Zarr dataset");
                        registered_params.insert(param_level_key);
                        registered_param_names.insert(param.to_string());
                        datasets_registered += 1;
                        bytes_written += zarr_file_size;
                    }
                    Err(e) => {
                        debug!(
                            param = %param,
                            level = %level,
                            error = %e,
                            "Could not register (may already exist)"
                        );
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, param = %param, "Failed to write/upload Zarr, skipping");
            }
        }
    }

    let parameters: Vec<String> = registered_param_names.into_iter().collect();

    info!(
        model = %model,
        datasets = datasets_registered,
        parameters = ?parameters,
        "GRIB2 ingestion complete"
    );

    Ok(IngestionResult {
        datasets_registered,
        model,
        reference_time: grib_reference_time.unwrap_or_else(Utc::now),
        parameters,
        bytes_written,
    })
}

/// Build storage path for a parameter.
fn build_storage_path(
    model: &str,
    reference_time: &DateTime<Utc>,
    param: &str,
    level_sanitized: &str,
    forecast_hour: u32,
) -> String {
    // For observation data like MRMS, use minute-level paths
    // For forecast models, use hourly paths
    let run_date = if model == "mrms" {
        reference_time.format("%Y%m%d_%H%Mz").to_string()
    } else {
        reference_time.format("%Y%m%d_%Hz").to_string()
    };

    format!(
        "grids/{}/{}/{}_{}_f{:03}.zarr",
        model,
        run_date,
        param.to_lowercase(),
        level_sanitized,
        forecast_hour
    )
}

/// Write Zarr pyramid and upload to storage.
async fn write_and_upload_zarr(
    storage: &Arc<ObjectStorage>,
    grid_data: &[f32],
    width: usize,
    height: usize,
    bbox: &GpBoundingBox,
    model: &str,
    param: &str,
    level: &str,
    units: &str,
    reference_time: DateTime<Utc>,
    forecast_hour: u32,
    storage_path: &str,
    row_origin: RowOrigin,
) -> Result<(u64, serde_json::Value)> {
    // Create temporary directory for Zarr output
    let temp_dir = tempfile::tempdir()?;
    let zarr_path = temp_dir.path().join("grid.zarr");
    std::fs::create_dir_all(&zarr_path)?;

    // Create Zarr writer
    let config = GridProcessorConfig::default();
    let writer = ZarrWriter::new(config);

    // Create filesystem store
    let store = FilesystemStore::new(&zarr_path).map_err(|e| {
        IngestionError::ZarrWrite(format!("Failed to create filesystem store: {}", e))
    })?;

    // Configure pyramid generation
    let pyramid_config = PyramidConfig::from_env();
    let downsample_method = DownsampleMethod::for_parameter(param);

    // Write Zarr with pyramid levels
    let write_result = writer
        .write_multiscale(
            store,
            "/",
            grid_data,
            width,
            height,
            bbox,
            model,
            param,
            level,
            units,
            reference_time,
            forecast_hour,
            &pyramid_config,
            downsample_method,
            row_origin,
        )
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to write Zarr: {}", e)))?;

    debug!(
        param = %param,
        level = %level,
        width = width,
        height = height,
        pyramid_levels = write_result.num_levels,
        "Wrote Zarr grid with pyramid levels"
    );

    // Upload to object storage
    let zarr_file_size = upload_zarr_directory(storage, &zarr_path, storage_path).await?;

    info!(
        param = %param,
        level = %level,
        path = %storage_path,
        size = zarr_file_size,
        pyramid_levels = write_result.num_levels,
        "Stored Zarr grid with pyramid levels"
    );

    // Build metadata with multiscale info for pyramid level selection during rendering
    let mut zarr_json = write_result.zarr_metadata.to_json();
    if let serde_json::Value::Object(ref mut map) = zarr_json {
        map.insert(
            "multiscale".to_string(),
            serde_json::to_value(&write_result.multiscale_metadata).unwrap_or_default(),
        );
    }

    Ok((zarr_file_size, zarr_json))
}

/// Decompress gzip-compressed GRIB2 data.
pub fn decompress_gzip(data: &[u8]) -> Result<Bytes> {
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| IngestionError::Decompression(e.to_string()))?;
    Ok(Bytes::from(decompressed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_decompress_gzip_valid() {
        // Create gzip-compressed test data
        let original = b"Hello, GRIB2 world!";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        // Decompress and verify
        let result = decompress_gzip(&compressed).expect("Should decompress");
        assert_eq!(result.as_ref(), original);
    }

    #[test]
    fn test_decompress_gzip_empty() {
        // Compress empty data
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"").unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_gzip(&compressed).expect("Should decompress empty");
        assert!(result.is_empty());
    }

    #[test]
    fn test_decompress_gzip_invalid() {
        // Invalid gzip data should fail
        let invalid = b"not gzip data";
        let result = decompress_gzip(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_decompress_gzip_large() {
        // Test with larger data
        let original: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_gzip(&compressed).expect("Should decompress");
        assert_eq!(result.as_ref(), original.as_slice());
    }

    #[test]
    fn test_build_storage_path_gfs() {
        let reference_time = Utc.with_ymd_and_hms(2024, 12, 17, 12, 0, 0).unwrap();
        let path = build_storage_path("gfs", &reference_time, "TMP", "2m_above_ground", 6);

        assert_eq!(path, "grids/gfs/20241217_12z/tmp_2m_above_ground_f006.zarr");
    }

    #[test]
    fn test_build_storage_path_hrrr() {
        let reference_time = Utc.with_ymd_and_hms(2024, 12, 17, 0, 0, 0).unwrap();
        let path = build_storage_path("hrrr", &reference_time, "UGRD", "10m_above_ground", 12);

        // Note: %Hz format produces "0z" for hour 0 (no leading zero)
        assert_eq!(
            path,
            "grids/hrrr/20241217_00z/ugrd_10m_above_ground_f012.zarr"
        );
    }

    #[test]
    fn test_build_storage_path_mrms() {
        // MRMS uses minute-level timestamps
        let reference_time = Utc.with_ymd_and_hms(2024, 12, 17, 14, 32, 0).unwrap();
        let path = build_storage_path("mrms", &reference_time, "REFL", "surface", 0);

        assert_eq!(path, "grids/mrms/20241217_1432z/refl_surface_f000.zarr");
    }

    #[test]
    fn test_build_storage_path_parameter_lowercase() {
        let reference_time = Utc.with_ymd_and_hms(2024, 12, 17, 0, 0, 0).unwrap();
        let path = build_storage_path("gfs", &reference_time, "CAPE", "surface", 0);

        // Parameter should be lowercase in path
        assert!(path.contains("/cape_"));
    }

    #[test]
    fn test_build_storage_path_forecast_hour_padding() {
        let reference_time = Utc.with_ymd_and_hms(2024, 12, 17, 0, 0, 0).unwrap();

        // Single digit should be zero-padded to 3 digits
        let path = build_storage_path("gfs", &reference_time, "TMP", "surface", 3);
        assert!(path.ends_with("_f003.zarr"));

        // Double digit
        let path = build_storage_path("gfs", &reference_time, "TMP", "surface", 48);
        assert!(path.ends_with("_f048.zarr"));

        // Triple digit
        let path = build_storage_path("gfs", &reference_time, "TMP", "surface", 120);
        assert!(path.ends_with("_f120.zarr"));
    }

    #[test]
    fn test_ingestion_result_bytes_written() {
        // Verify IngestionResult properly tracks bytes_written
        let result = IngestionResult {
            datasets_registered: 5,
            model: "gfs".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 12, 17, 12, 0, 0).unwrap(),
            parameters: vec!["TMP".to_string(), "UGRD".to_string()],
            bytes_written: 1024 * 1024 * 50, // 50 MB
        };

        assert_eq!(result.datasets_registered, 5);
        assert_eq!(result.bytes_written, 52_428_800);
        assert_eq!(result.model, "gfs");
        assert_eq!(result.parameters.len(), 2);
    }

    #[test]
    fn test_ingestion_result_zero_bytes() {
        // When no datasets are registered, bytes_written should be 0
        let result = IngestionResult {
            datasets_registered: 0,
            model: "gfs".to_string(),
            reference_time: Utc::now(),
            parameters: vec![],
            bytes_written: 0,
        };

        assert_eq!(result.datasets_registered, 0);
        assert_eq!(result.bytes_written, 0);
        assert!(result.parameters.is_empty());
    }

    #[test]
    fn test_zarr_json_includes_multiscale_metadata() {
        // This test verifies the JSON-building logic that adds multiscale metadata
        // to the zarr_json for catalog storage. This is critical for pyramid level
        // selection during rendering.
        use grid_processor::types::{BoundingBox, MultiscaleMetadata, PyramidLevel, RowOrigin};
        use grid_processor::writer::ZarrMetadata;

        // Create mock ZarrMetadata (basic metadata without multiscale)
        let zarr_metadata = ZarrMetadata {
            model: "mrms".to_string(),
            parameter: "REFL".to_string(),
            level: "surface".to_string(),
            units: "dBZ".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 12, 17, 14, 32, 0).unwrap(),
            forecast_hour: 0,
            bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
            shape: (7000, 3500),
            chunk_shape: (512, 512),
            num_chunks: (14, 7),
            fill_value: f32::NAN,
            dtype: "float32".to_string(),
            compression: "blosc".to_string(),
            row_origin: RowOrigin::North,
        };

        // Create mock MultiscaleMetadata (pyramid levels)
        let multiscale_metadata = MultiscaleMetadata {
            name: "mrms_REFL".to_string(),
            axes: vec![],
            levels: vec![
                PyramidLevel {
                    level: 0,
                    path: "0".to_string(),
                    shape: (7000, 3500),
                    chunk_shape: (512, 512),
                    scale: 1.0,
                },
                PyramidLevel {
                    level: 1,
                    path: "1".to_string(),
                    shape: (3500, 1750),
                    chunk_shape: (512, 512),
                    scale: 2.0,
                },
                PyramidLevel {
                    level: 2,
                    path: "2".to_string(),
                    shape: (1750, 875),
                    chunk_shape: (512, 512),
                    scale: 4.0,
                },
            ],
            downsample_method: "max".to_string(),
            native_resolution: (0.01, 0.01),
            bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
            row_origin: RowOrigin::North,
        };

        // Simulate the JSON-building logic from write_and_upload_zarr
        let mut zarr_json = zarr_metadata.to_json();
        if let serde_json::Value::Object(ref mut map) = zarr_json {
            map.insert(
                "multiscale".to_string(),
                serde_json::to_value(&multiscale_metadata).unwrap_or_default(),
            );
        }

        // Verify the zarr_json contains the multiscale key
        assert!(
            zarr_json.get("multiscale").is_some(),
            "zarr_json should contain 'multiscale' key"
        );

        // Verify the multiscale metadata can be parsed back
        let parsed_multiscale: MultiscaleMetadata =
            serde_json::from_value(zarr_json.get("multiscale").unwrap().clone())
                .expect("Should parse multiscale metadata");

        assert_eq!(parsed_multiscale.levels.len(), 3);
        assert_eq!(parsed_multiscale.levels[0].level, 0);
        assert_eq!(parsed_multiscale.levels[0].shape, (7000, 3500));
        assert_eq!(parsed_multiscale.levels[1].level, 1);
        assert_eq!(parsed_multiscale.levels[1].scale, 2.0);
        assert_eq!(parsed_multiscale.levels[2].level, 2);
        assert_eq!(parsed_multiscale.levels[2].scale, 4.0);

        // Verify the basic zarr_metadata fields are still present
        assert_eq!(
            zarr_json.get("model").and_then(|v| v.as_str()),
            Some("mrms")
        );
        assert_eq!(
            zarr_json.get("parameter").and_then(|v| v.as_str()),
            Some("REFL")
        );
    }

    #[test]
    fn test_zarr_json_multiscale_used_by_parse_multiscale_metadata() {
        // This test verifies that the multiscale metadata we add to zarr_json
        // can be correctly parsed by the rendering code's parse_multiscale_metadata
        use grid_processor::parse_multiscale_metadata;
        use grid_processor::types::{BoundingBox, MultiscaleMetadata, PyramidLevel, RowOrigin};
        use grid_processor::writer::ZarrMetadata;

        // Create mock metadata
        let zarr_metadata = ZarrMetadata {
            model: "mrms".to_string(),
            parameter: "REFL".to_string(),
            level: "surface".to_string(),
            units: "dBZ".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 12, 17, 14, 32, 0).unwrap(),
            forecast_hour: 0,
            bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
            shape: (7000, 3500),
            chunk_shape: (512, 512),
            num_chunks: (14, 7),
            fill_value: f32::NAN,
            dtype: "float32".to_string(),
            compression: "blosc".to_string(),
            row_origin: RowOrigin::North,
        };

        let multiscale_metadata = MultiscaleMetadata {
            name: "mrms_REFL".to_string(),
            axes: vec![],
            levels: vec![
                PyramidLevel {
                    level: 0,
                    path: "0".to_string(),
                    shape: (7000, 3500),
                    chunk_shape: (512, 512),
                    scale: 1.0,
                },
                PyramidLevel {
                    level: 1,
                    path: "1".to_string(),
                    shape: (3500, 1750),
                    chunk_shape: (512, 512),
                    scale: 2.0,
                },
            ],
            downsample_method: "max".to_string(),
            native_resolution: (0.01, 0.01),
            bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
            row_origin: RowOrigin::North,
        };

        // Build zarr_json with multiscale (same as write_and_upload_zarr)
        let mut zarr_json = zarr_metadata.to_json();
        if let serde_json::Value::Object(ref mut map) = zarr_json {
            map.insert(
                "multiscale".to_string(),
                serde_json::to_value(&multiscale_metadata).unwrap_or_default(),
            );
        }

        // Use the same function that rendering uses to parse multiscale metadata
        let parsed = parse_multiscale_metadata(&zarr_json);

        assert!(
            parsed.is_some(),
            "parse_multiscale_metadata should successfully parse the metadata"
        );

        let parsed = parsed.unwrap();
        assert_eq!(parsed.num_levels(), 2);
        assert_eq!(parsed.native_resolution, (0.01, 0.01));
    }
}
