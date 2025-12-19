//! GRIB2 file ingestion logic.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;
use tracing::{debug, info, warn};
use zarrs_filesystem::FilesystemStore;

use grid_processor::{
    BoundingBox as GpBoundingBox, DownsampleMethod, GridProcessorConfig, PyramidConfig, ZarrWriter,
};
use projection::LambertConformal;
use storage::{Catalog, CatalogEntry, ObjectStorage};

use crate::config::{should_ingest_parameter, standard_pressure_levels, target_grib2_parameters};
use crate::error::{IngestionError, Result};
use crate::metadata::{extract_mrms_param, get_bbox_from_grid, get_model_bbox};
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

    // Parse GRIB2 data
    let mut reader = grib2_parser::Grib2Reader::new(data);

    // Track registered parameters
    let mut registered_params: HashSet<String> = HashSet::new();
    let mut datasets_registered = 0usize;
    let mut grib_reference_time: Option<DateTime<Utc>> = None;
    let mut registered_param_names: HashSet<String> = HashSet::new();

    // Get parameter filtering config
    let target_params = target_grib2_parameters();
    let pressure_levels = standard_pressure_levels();

    // For MRMS, extract parameter name from filename
    let mrms_param_name: Option<String> = if model == "mrms" {
        extract_mrms_param(file_path)
    } else {
        None
    };

    while let Some(message) = reader.next_message().ok().flatten() {
        // Extract reference time from first message
        if grib_reference_time.is_none() {
            grib_reference_time = Some(message.identification.reference_time);
            info!(
                reference_time = %message.identification.reference_time,
                "Extracted reference time"
            );
        }

        let grib_param = &message.product_definition.parameter_short_name;
        let param = if model == "mrms" {
            mrms_param_name.as_ref().unwrap_or(grib_param)
        } else {
            grib_param
        };
        let level = &message.product_definition.level_description;
        let level_type = message.product_definition.level_type;
        let level_value = message.product_definition.level_value;

        let param_level_key = format!("{}:{}", param, level);

        // Check if we should register this parameter
        let should_register = if model == "mrms" {
            !registered_params.contains(&param_level_key)
        } else {
            !registered_params.contains(&param_level_key)
                && should_ingest_parameter(param, level_type, level_value, &target_params, &pressure_levels)
        };

        if !should_register {
            continue;
        }

        let reference_time = grib_reference_time.unwrap_or_else(Utc::now);

        // Sanitize level for path
        let level_sanitized = level.replace([' ', '/'], "_").to_lowercase();

        // Storage path format
        let zarr_storage_path = build_storage_path(&model, &reference_time, param, &level_sanitized, forecast_hour);

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

        // Convert sentinel missing values to NaN
        let grid_data: Vec<f32> = grid_data
            .into_iter()
            .map(|v| if v <= -90.0 { f32::NAN } else { v })
            .collect();

        if grid_data.len() != width * height {
            warn!(
                expected = width * height,
                actual = grid_data.len(),
                param = %param,
                "Grid data size mismatch, skipping"
            );
            continue;
        }

        // Calculate bounding box
        let gp_bbox = if model == "hrrr" {
            let proj = LambertConformal::hrrr();
            let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
            GpBoundingBox::new(min_lon, min_lat, max_lon, max_lat)
        } else {
            let grib_bbox = get_bbox_from_grid(&message.grid_definition);
            GpBoundingBox::new(grib_bbox.min_x, grib_bbox.min_y, grib_bbox.max_x, grib_bbox.max_y)
        };

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
            reference_time,
            forecast_hour,
            &zarr_storage_path,
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
        bytes_written: 0, // TODO: track total bytes
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
    reference_time: DateTime<Utc>,
    forecast_hour: u32,
    storage_path: &str,
) -> Result<(u64, serde_json::Value)> {
    // Create temporary directory for Zarr output
    let temp_dir = tempfile::tempdir()?;
    let zarr_path = temp_dir.path().join("grid.zarr");
    std::fs::create_dir_all(&zarr_path)?;

    // Create Zarr writer
    let config = GridProcessorConfig::default();
    let writer = ZarrWriter::new(config);

    // Create filesystem store
    let store = FilesystemStore::new(&zarr_path)
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create filesystem store: {}", e)))?;

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
            "unknown", // units
            reference_time,
            forecast_hour,
            &pyramid_config,
            downsample_method,
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

    Ok((zarr_file_size, write_result.zarr_metadata.to_json()))
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
