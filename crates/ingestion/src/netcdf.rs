//! NetCDF file ingestion logic (GOES satellite data).

use bytes::Bytes;
use chrono::{DateTime, Utc};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};
use zarrs_filesystem::FilesystemStore;

use grid_processor::{
    reproject_geostationary_to_geographic, BoundingBox as GpBoundingBox, DownsampleMethod,
    GridProcessorConfig, PyramidConfig, ZarrWriter,
};
use projection::Geostationary;
use storage::{Catalog, CatalogEntry, ObjectStorage};
use wms_common::BoundingBox;

use crate::error::{IngestionError, Result};
use crate::metadata::parse_goes_filename;
use crate::upload::upload_zarr_directory;
use crate::{IngestOptions, IngestionResult};

/// GOES band to parameter/level mapping.
fn band_to_parameter(band: u8) -> (&'static str, &'static str) {
    match band {
        1 => ("CMI_C01", "visible_blue"),   // 0.47µm Blue
        2 => ("CMI_C02", "visible_red"),    // 0.64µm Red (most common visible)
        3 => ("CMI_C03", "visible_veggie"), // 0.86µm Vegetation
        4 => ("CMI_C04", "cirrus"),         // 1.37µm Cirrus
        5 => ("CMI_C05", "snow_ice"),       // 1.6µm Snow/Ice
        6 => ("CMI_C06", "cloud_particle"), // 2.2µm Cloud Particle Size
        7 => ("CMI_C07", "shortwave_ir"),   // 3.9µm Shortwave Window
        8 => ("CMI_C08", "upper_vapor"),    // 6.2µm Upper-Level Water Vapor
        9 => ("CMI_C09", "mid_vapor"),      // 6.9µm Mid-Level Water Vapor
        10 => ("CMI_C10", "low_vapor"),     // 7.3µm Lower-Level Water Vapor
        11 => ("CMI_C11", "cloud_phase"),   // 8.4µm Cloud-Top Phase
        12 => ("CMI_C12", "ozone"),         // 9.6µm Ozone
        13 => ("CMI_C13", "clean_ir"),      // 10.3µm "Clean" Longwave IR
        14 => ("CMI_C14", "ir"),            // 11.2µm Longwave IR
        15 => ("CMI_C15", "dirty_ir"),      // 12.3µm "Dirty" Longwave IR
        16 => ("CMI_C16", "co2"),           // 13.3µm CO2
        _ => ("CMI_C02", "visible_red"),    // Default
    }
}

/// Ingest a GOES NetCDF file into Zarr format.
///
/// Parses the NetCDF file, reprojects from geostationary to geographic coordinates,
/// writes Zarr pyramids, uploads to object storage, and registers in the catalog.
pub async fn ingest_netcdf(
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

    // Parse filename to extract metadata
    let file_info = parse_goes_filename(filename);

    // Determine model (satellite)
    let model = options.model.clone().unwrap_or_else(|| {
        file_info
            .as_ref()
            .map(|i| i.satellite.clone())
            .unwrap_or_else(|| extract_satellite_from_filename(filename))
    });

    // Extract band number
    let band = file_info
        .as_ref()
        .map(|i| i.band)
        .unwrap_or_else(|| extract_band_from_filename(filename).unwrap_or(2));

    // Extract observation time
    let observation_time = file_info
        .as_ref()
        .map(|i| i.observation_time)
        .unwrap_or_else(Utc::now);

    let (parameter, level) = band_to_parameter(band);

    info!(
        model = %model,
        band = band,
        parameter = parameter,
        observation_time = %observation_time,
        file_size = data.len(),
        "Ingesting GOES NetCDF file"
    );

    // Parse the NetCDF file
    let (raw_data, width, height, projection, x_offset, y_offset, x_scale, y_scale) =
        netcdf_parser::load_goes_netcdf_from_bytes(&data)
            .map_err(|e| IngestionError::NetcdfParse(e.to_string()))?;

    info!(
        width = width,
        height = height,
        longitude_origin = projection.longitude_origin,
        "Parsed GOES NetCDF data"
    );

    // Create Geostationary projection for reprojection
    let proj = Geostationary::from_goes(
        projection.perspective_point_height,
        projection.semi_major_axis,
        projection.semi_minor_axis,
        projection.longitude_origin,
        x_offset as f64,
        y_offset as f64,
        x_scale as f64,
        y_scale as f64,
        width,
        height,
    );

    // Reproject from geostationary to geographic coordinates
    info!("Reprojecting GOES data from geostationary to geographic coordinates");
    let (reprojected_data, out_width, out_height, gp_bbox) =
        reproject_geostationary_to_geographic(&raw_data, width, height, &proj);

    info!(
        out_width = out_width,
        out_height = out_height,
        bbox = ?gp_bbox,
        "Reprojection complete"
    );

    // Create Zarr storage path: grids/{model}/{date}/{HH}/{param}_{MM}.zarr
    let date = observation_time.format("%Y-%m-%d").to_string();
    let hour = observation_time.format("%H").to_string();
    let minute = observation_time.format("%M").to_string();
    let zarr_storage_path = format!(
        "grids/{}/{}/{}/{}_{}.zarr",
        model, date, hour, parameter, minute
    );

    // Write Zarr and upload
    let (zarr_file_size, zarr_metadata) = write_and_upload_zarr(
        storage,
        &reprojected_data,
        out_width,
        out_height,
        &gp_bbox,
        &model,
        parameter,
        level,
        band,
        observation_time,
        &zarr_storage_path,
    )
    .await?;

    // Register in catalog
    let catalog_bbox = BoundingBox::new(
        gp_bbox.min_lon,
        gp_bbox.min_lat,
        gp_bbox.max_lon,
        gp_bbox.max_lat,
    );

    let entry = CatalogEntry {
        model: model.clone(),
        parameter: parameter.to_string(),
        level: level.to_string(),
        reference_time: observation_time,
        forecast_hour: 0, // Observational data
        bbox: catalog_bbox,
        storage_path: zarr_storage_path.clone(),
        file_size: zarr_file_size,
        zarr_metadata: Some(zarr_metadata),
    };

    match catalog.register_dataset(&entry).await {
        Ok(id) => {
            info!(
                id = %id,
                parameter = parameter,
                model = %model,
                band = band,
                "Registered GOES Zarr dataset"
            );
        }
        Err(e) => {
            warn!(error = %e, "Could not register dataset (may already exist)");
        }
    }

    info!(
        model = %model,
        parameter = parameter,
        band = band,
        "GOES NetCDF ingestion complete"
    );

    Ok(IngestionResult {
        datasets_registered: 1,
        model,
        reference_time: observation_time,
        parameters: vec![parameter.to_string()],
        bytes_written: zarr_file_size,
    })
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
    band: u8,
    observation_time: DateTime<Utc>,
    storage_path: &str,
) -> Result<(u64, serde_json::Value)> {
    // Create temporary directory
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

    // Determine units based on band type
    let units = if band <= 6 {
        "reflectance" // Visible/near-IR bands
    } else {
        "K" // IR bands (brightness temperature)
    };

    // Configure pyramid generation
    let pyramid_config = PyramidConfig::from_env();
    let downsample_method = DownsampleMethod::Mean; // Mean for satellite data

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
            observation_time,
            0, // forecast_hour = 0 for observational data
            &pyramid_config,
            downsample_method,
        )
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to write Zarr: {}", e)))?;

    debug!(
        param = %param,
        width = width,
        height = height,
        pyramid_levels = write_result.num_levels,
        "Wrote GOES Zarr grid with pyramid levels"
    );

    // Upload to object storage
    let zarr_file_size = upload_zarr_directory(storage, &zarr_path, storage_path).await?;

    info!(
        param = %param,
        path = %storage_path,
        size = zarr_file_size,
        pyramid_levels = write_result.num_levels,
        "Stored GOES Zarr grid"
    );

    // Build metadata with multiscale info
    let mut zarr_json = write_result.zarr_metadata.to_json();
    if let serde_json::Value::Object(ref mut map) = zarr_json {
        map.insert(
            "multiscale".to_string(),
            serde_json::to_value(&write_result.multiscale_metadata).unwrap_or_default(),
        );
    }

    Ok((zarr_file_size, zarr_json))
}

/// Extract satellite model from filename.
fn extract_satellite_from_filename(filename: &str) -> String {
    if filename.contains("_G16_") || filename.to_lowercase().contains("goes16") {
        "goes16".to_string()
    } else if filename.contains("_G18_") || filename.to_lowercase().contains("goes18") {
        "goes18".to_string()
    } else {
        "goes16".to_string() // Default
    }
}

/// Extract band number from filename.
fn extract_band_from_filename(filename: &str) -> Option<u8> {
    filename
        .find("M6C")
        .or_else(|| filename.find("M3C"))
        .and_then(|pos| {
            let band_str = filename.get(pos + 3..pos + 5)?;
            band_str.parse::<u8>().ok()
        })
}
