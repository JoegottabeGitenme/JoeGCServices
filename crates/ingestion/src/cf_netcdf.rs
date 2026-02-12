//! CF-convention NetCDF ingestion logic (NLDAS-2, GLDAS, ERA5, etc.).
//!
//! Handles multi-variable NetCDF files on regular lat/lon grids.
//! Unlike the GOES-specific path, this:
//! - Extracts multiple variables per file
//! - Does NOT reproject (data is already regular geographic)
//! - Reads grid metadata from CF coordinate variables
//! - Uses the generic CF reader from `netcdf-parser`

use bytes::Bytes;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};
use zarrs_filesystem::FilesystemStore;

use grid_processor::{
    BoundingBox as GpBoundingBox, DownsampleMethod, GridProcessorConfig, ProjectionType,
    PyramidConfig, RowOrigin, ZarrWriter,
};
use storage::{Catalog, CatalogEntry, ObjectStorage};
use wms_common::BoundingBox;

use crate::error::{IngestionError, Result};
use crate::metadata::parse_nldas_filename;
use crate::upload::upload_zarr_directory;
use crate::{IngestOptions, IngestionResult};

/// Ingest a CF-convention NetCDF file (multi-variable, regular lat/lon grid).
///
/// Parses the file using the generic CF reader, writes one Zarr pyramid per variable,
/// uploads each to object storage, and registers catalog entries.
pub async fn ingest_cf_netcdf(
    storage: &Arc<ObjectStorage>,
    catalog: &Catalog,
    data: Bytes,
    file_path: &str,
    options: &IngestOptions,
) -> Result<IngestionResult> {
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.nc");

    // Determine model name
    let model = options.model.clone().unwrap_or_else(|| {
        extract_cf_model_from_filename(filename).unwrap_or_else(|| "unknown-cf".to_string())
    });

    info!(
        model = %model,
        file_path = %file_path,
        file_size = data.len(),
        "Ingesting CF-convention NetCDF file"
    );

    // Load the parameter mapping from model config (cf_name -> our parameter name)
    // For now, we'll use a config-driven approach: read what variables to extract
    let var_filter = load_variable_filter_for_model(&model);

    // Parse the NetCDF file using the CF reader
    let dataset = netcdf_parser::load_cf_netcdf(&data, var_filter.as_ref())
        .map_err(|e| IngestionError::NetcdfParse(e.to_string()))?;

    let meta = &dataset.metadata;

    // Override observation time from filename if available, otherwise use file time
    let observation_time = parse_nldas_filename(filename)
        .map(|info| info.observation_time)
        .unwrap_or(meta.time);

    info!(
        width = meta.max_lon as usize,
        height = meta.max_lat as usize,
        variables = dataset.variables.len(),
        observation_time = %observation_time,
        lat_range = format!("[{}, {}]", meta.min_lat, meta.max_lat),
        lon_range = format!("[{}, {}]", meta.min_lon, meta.max_lon),
        "Parsed CF-convention NetCDF"
    );

    // Build bbox from metadata
    let bbox = GpBoundingBox::new(meta.min_lon, meta.min_lat, meta.max_lon, meta.max_lat);

    // Process each variable
    let mut datasets_registered = 0usize;
    let mut parameters = Vec::new();
    let mut total_bytes: u64 = 0;

    // Configure pyramid generation
    let pyramid_config = PyramidConfig::from_env();
    let config = GridProcessorConfig::default();
    let writer = ZarrWriter::new(config);

    for var in &dataset.variables {
        // Map CF variable name to our parameter name
        // For NLDAS, the CF names are already clean (SoilM_0_10cm, etc.)
        let parameter = var.name.clone();

        // Determine level string from variable name
        let level = infer_level_from_variable(&var.name);

        // Determine units
        let units = if var.units.is_empty() {
            "unknown"
        } else {
            &var.units
        };

        // Create Zarr storage path: grids/{model}/{date}/{HH}/{param}.zarr
        let date = observation_time.format("%Y-%m-%d").to_string();
        let hour = observation_time.format("%H").to_string();
        let zarr_storage_path = format!("grids/{}/{}/{}/{}.zarr", model, date, hour, parameter);

        // Write Zarr pyramid to temp dir
        let temp_dir = tempfile::tempdir()?;
        let zarr_path = temp_dir.path().join("grid.zarr");
        std::fs::create_dir_all(&zarr_path)?;

        let store = FilesystemStore::new(&zarr_path).map_err(|e| {
            IngestionError::ZarrWrite(format!("Failed to create filesystem store: {}", e))
        })?;

        // CF reader already flips rows to north-first if lat was ascending
        let downsample_method = DownsampleMethod::Mean;

        let write_result = writer
            .write_multiscale(
                store,
                "/",
                &var.data,
                var.width,
                var.height,
                &bbox,
                &model,
                &parameter,
                &level,
                units,
                observation_time,
                0, // forecast_hour = 0 for observational data
                &pyramid_config,
                downsample_method,
                RowOrigin::North, // CF reader already flipped to north-first
                ProjectionType::Geographic,
            )
            .map_err(|e| IngestionError::ZarrWrite(format!("Failed to write Zarr: {}", e)))?;

        debug!(
            param = %parameter,
            level = %level,
            width = var.width,
            height = var.height,
            pyramid_levels = write_result.num_levels,
            "Wrote CF Zarr grid"
        );

        // Upload to object storage
        let zarr_file_size = upload_zarr_directory(storage, &zarr_path, &zarr_storage_path).await?;
        total_bytes += zarr_file_size;

        info!(
            param = %parameter,
            path = %zarr_storage_path,
            size = zarr_file_size,
            pyramid_levels = write_result.num_levels,
            "Stored CF Zarr grid"
        );

        // Build metadata JSON
        let mut zarr_json = write_result.zarr_metadata.to_json();
        if let serde_json::Value::Object(ref mut map) = zarr_json {
            map.insert(
                "multiscale".to_string(),
                serde_json::to_value(&write_result.multiscale_metadata).unwrap_or_default(),
            );
        }

        // Register in catalog
        let catalog_bbox = BoundingBox::new(bbox.min_lon, bbox.min_lat, bbox.max_lon, bbox.max_lat);

        let entry = CatalogEntry {
            model: model.clone(),
            parameter: parameter.clone(),
            level: level.clone(),
            reference_time: observation_time,
            forecast_hour: 0, // Observational data
            bbox: catalog_bbox,
            storage_path: zarr_storage_path.clone(),
            file_size: zarr_file_size,
            zarr_metadata: Some(zarr_json),
        };

        match catalog.register_dataset(&entry).await {
            Ok(id) => {
                info!(
                    id = %id,
                    parameter = %parameter,
                    level = %level,
                    model = %model,
                    "Registered CF Zarr dataset"
                );
                datasets_registered += 1;
            }
            Err(e) => {
                warn!(
                    error = %e,
                    parameter = %parameter,
                    "Could not register dataset (may already exist)"
                );
            }
        }

        parameters.push(parameter);
    }

    info!(
        model = %model,
        datasets_registered = datasets_registered,
        total_bytes = total_bytes,
        parameters = parameters.len(),
        "CF NetCDF ingestion complete"
    );

    Ok(IngestionResult {
        datasets_registered,
        model,
        reference_time: observation_time,
        parameters,
        bytes_written: total_bytes,
    })
}

/// Extract the model name from a CF-convention NetCDF filename.
///
/// Supports:
/// - NLDAS-2 Noah: `NLDAS_NOAH0125_H.A20260205.0000.020.nc`
/// - NLDAS-2 Forcing: `NLDAS_FORA0125_H.002.grb.SUB.nc4`
fn extract_cf_model_from_filename(filename: &str) -> Option<String> {
    let upper = filename.to_uppercase();

    if upper.starts_with("NLDAS_NOAH") {
        Some("nldas-noah".to_string())
    } else if upper.starts_with("NLDAS_FORA") || upper.starts_with("NLDAS_FOR") {
        Some("nldas-forcing".to_string())
    } else {
        None
    }
}

/// Load the set of CF variable names to extract for a given model.
///
/// Reads the model's YAML config and collects `cf_name` values from the
/// `parameters` section. If no config is found, returns None (extract all).
fn load_variable_filter_for_model(model: &str) -> Option<HashSet<String>> {
    let config_dir = std::env::var("CONFIG_DIR")
        .map(|d| std::path::PathBuf::from(d).join("models"))
        .unwrap_or_else(|_| std::path::PathBuf::from("config/models"));

    let config_path = config_dir.join(format!("{}.yaml", model));

    if !config_path.exists() {
        debug!(model = %model, "No model config found, extracting all variables");
        return None;
    }

    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            warn!(model = %model, error = %e, "Failed to read model config");
            return None;
        }
    };

    let yaml: serde_yaml::Value = match serde_yaml::from_str(&contents) {
        Ok(y) => y,
        Err(e) => {
            warn!(model = %model, error = %e, "Failed to parse model config YAML");
            return None;
        }
    };

    let parameters = yaml.get("parameters")?.as_sequence()?;

    let mut filter = HashSet::new();
    for param in parameters {
        // Use cf_name if present, otherwise fall back to name
        if let Some(cf_name) = param
            .get("cf_name")
            .and_then(|v| v.as_str())
            .or_else(|| param.get("name").and_then(|v| v.as_str()))
        {
            filter.insert(cf_name.to_string());
        }
    }

    if filter.is_empty() {
        None
    } else {
        debug!(model = %model, variables = filter.len(), "Loaded variable filter from config");
        Some(filter)
    }
}

/// Infer a level string from the CF variable name.
///
/// NLDAS-2 variable naming conventions:
/// - `SoilM_0_10cm` → "0-10 cm depth"
/// - `SoilT_40_100cm` → "40-100 cm depth"
/// - `SoilM_0_100cm` → "0-100 cm total"
/// - `SoilM_0_200cm` → "0-200 cm total"
/// - Everything else → "surface"
fn infer_level_from_variable(var_name: &str) -> String {
    // Match soil depth patterns: SoilM_0_10cm, SoilT_40_100cm, etc.
    // General pattern: *_{top}_{bottom}cm
    if let Some(rest) = var_name
        .strip_suffix("cm")
        .and_then(|s| s.rsplit_once('_'))
        .and_then(|(prefix, bottom)| {
            prefix.rsplit_once('_').and_then(|(_, top)| {
                let t: u32 = top.parse().ok()?;
                let b: u32 = bottom.parse().ok()?;
                Some((t, b))
            })
        })
    {
        let (top, bottom) = rest;
        if top == 0 && (bottom == 100 || bottom == 200) {
            return format!("0-{} cm total", bottom);
        }
        return format!("{}-{} cm depth", top, bottom);
    }

    // RootMoist is root zone
    if var_name == "RootMoist" {
        return "root zone".to_string();
    }

    // Default: surface
    "surface".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Model Detection ====================

    #[test]
    fn test_extract_cf_model_nldas_noah() {
        assert_eq!(
            extract_cf_model_from_filename("NLDAS_NOAH0125_H.A20260205.0000.020.nc"),
            Some("nldas-noah".to_string())
        );
        assert_eq!(
            extract_cf_model_from_filename("nldas_noah0125_h.a20260205.0000.020.nc"),
            Some("nldas-noah".to_string())
        );
    }

    #[test]
    fn test_extract_cf_model_nldas_forcing() {
        assert_eq!(
            extract_cf_model_from_filename("NLDAS_FORA0125_H.002.grb.SUB.nc4"),
            Some("nldas-forcing".to_string())
        );
        assert_eq!(
            extract_cf_model_from_filename("nldas_fora0125_h.a20260205.0000.020.nc"),
            Some("nldas-forcing".to_string())
        );
    }

    #[test]
    fn test_extract_cf_model_unknown() {
        assert_eq!(extract_cf_model_from_filename("random_file.nc"), None);
        assert_eq!(extract_cf_model_from_filename("goes19_data.nc"), None);
    }

    // ==================== Level Inference ====================

    #[test]
    fn test_infer_level_soil_moisture_layers() {
        assert_eq!(infer_level_from_variable("SoilM_0_10cm"), "0-10 cm depth");
        assert_eq!(infer_level_from_variable("SoilM_10_40cm"), "10-40 cm depth");
        assert_eq!(
            infer_level_from_variable("SoilM_40_100cm"),
            "40-100 cm depth"
        );
        assert_eq!(
            infer_level_from_variable("SoilM_100_200cm"),
            "100-200 cm depth"
        );
    }

    #[test]
    fn test_infer_level_soil_moisture_totals() {
        assert_eq!(infer_level_from_variable("SoilM_0_100cm"), "0-100 cm total");
        assert_eq!(infer_level_from_variable("SoilM_0_200cm"), "0-200 cm total");
    }

    #[test]
    fn test_infer_level_soil_temperature() {
        assert_eq!(infer_level_from_variable("SoilT_0_10cm"), "0-10 cm depth");
        assert_eq!(
            infer_level_from_variable("SoilT_100_200cm"),
            "100-200 cm depth"
        );
    }

    #[test]
    fn test_infer_level_root_zone() {
        assert_eq!(infer_level_from_variable("RootMoist"), "root zone");
    }

    #[test]
    fn test_infer_level_surface_default() {
        assert_eq!(infer_level_from_variable("SWE"), "surface");
        assert_eq!(infer_level_from_variable("AvgSurfT"), "surface");
        assert_eq!(infer_level_from_variable("Evap"), "surface");
        assert_eq!(infer_level_from_variable("Qle"), "surface");
        assert_eq!(infer_level_from_variable("Albedo"), "surface");
    }

    // ==================== Variable Filter ====================

    #[test]
    fn test_load_variable_filter_no_config() {
        // When no config exists, should return None (extract all)
        let filter = load_variable_filter_for_model("nonexistent-model-xyz");
        assert!(filter.is_none());
    }
}
