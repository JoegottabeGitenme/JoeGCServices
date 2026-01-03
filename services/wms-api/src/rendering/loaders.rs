//! Grid data loading from Zarr format.
//!
//! This module provides functions for loading weather data grids from Zarr arrays,
//! which is the primary storage format for all weather data (GFS, HRRR, MRMS, GOES).
//!
//! Key features:
//! - Efficient partial reads (only loads needed chunks)
//! - Pyramid/multiscale support for resolution-optimized loading
//! - Automatic chunk caching via GridProcessorFactory

use std::time::Instant;
use storage::CatalogEntry;
use tracing::{debug, error, info, instrument};

use super::types::GridData;
use grid_processor::GridProcessorFactory;

// ============================================================================
// ============================================================================
// Zarr loading (primary data loading path)
// ============================================================================

/// Load grid data from Zarr storage.
///
/// This is the primary function for loading grid data. All weather data (GFS, HRRR,
/// MRMS, GOES) is stored in Zarr format and loaded through this path.
///
/// # Arguments
/// * `factory` - GridProcessorFactory for Zarr data access (contains chunk cache)
/// * `entry` - Catalog entry with zarr_metadata
/// * `bbox` - Optional bounding box (for partial reads)
/// * `output_size` - Optional output tile dimensions (for pyramid level selection)
/// * `requires_full_grid` - If true, always read full grid (for non-geographic projections)
///
/// # Returns
/// GridData containing the grid values and dimensions
pub async fn load_grid_data(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    bbox: Option<[f32; 4]>,
    output_size: Option<(usize, usize)>,
    requires_full_grid: bool,
) -> Result<GridData, String> {
    // Verify entry has Zarr metadata
    if entry.zarr_metadata.is_none() {
        return Err(format!(
            "No zarr_metadata in catalog entry for {}/{}/{}. All data must be in Zarr format.",
            entry.model, entry.parameter, entry.level
        ));
    }

    load_grid_data_from_zarr(factory, entry, bbox, output_size, requires_full_grid).await
}

/// Load grid data from a Zarr file stored in MinIO.
///
/// This function uses the GridProcessor abstraction for efficient partial reads
/// from Zarr V3 formatted data. It only loads the chunks needed for the requested
/// bounding box, significantly reducing data transfer for tile requests.
///
/// # Arguments
/// * `factory` - GridProcessorFactory containing MinIO client and shared cache
/// * `entry` - Catalog entry with zarr_metadata
/// * `bbox` - Optional bounding box to load (if None, loads full grid)
/// * `output_size` - Optional output dimensions for pyramid level selection
/// * `requires_full_grid` - If true, always read full grid (for non-geographic projections)
///
/// # Returns
/// GridData containing the grid values and dimensions
#[instrument(skip(factory, entry), fields(
    model = %entry.model,
    parameter = %entry.parameter,
    level = %entry.level,
    storage_path = %entry.storage_path,
))]
pub async fn load_grid_data_from_zarr(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    bbox: Option<[f32; 4]>,
    output_size: Option<(usize, usize)>,
    requires_full_grid: bool,
) -> Result<GridData, String> {
    use grid_processor::{
        create_minio_storage, parse_multiscale_metadata, BoundingBox as GpBoundingBox,
        MultiscaleGridProcessorFactory, ZarrMetadata,
    };

    // Parse zarr_metadata from catalog entry
    let zarr_json = entry.zarr_metadata.as_ref()
        .ok_or_else(|| {
            error!(model = %entry.model, parameter = %entry.parameter, "No zarr_metadata in catalog entry");
            "No zarr_metadata in catalog entry".to_string()
        })?;

    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| {
            error!(model = %entry.model, parameter = %entry.parameter, error = %e, "Failed to parse zarr_metadata");
            format!("Failed to parse zarr_metadata: {}", e)
        })?;

    info!(
        storage_path = %entry.storage_path,
        shape = ?zarr_meta.shape,
        chunk_shape = ?zarr_meta.chunk_shape,
        "Loading grid data from Zarr"
    );

    // Build storage path - the storage_path in catalog points to the Zarr directory
    // e.g., "grids/gfs/20241212_00z/TMP_2m_f006.zarr"
    // zarrs expects paths to start with / for object_store backends
    let zarr_path = if entry.storage_path.starts_with('/') {
        entry.storage_path.clone()
    } else {
        format!("/{}", entry.storage_path)
    };

    // Create MinIO storage using factory's config (avoids re-parsing env vars)
    let minio_config = factory.minio_config();
    let store = create_minio_storage(minio_config).map_err(|e| {
        error!(
            error = %e,
            endpoint = %minio_config.endpoint,
            bucket = %minio_config.bucket,
            "Failed to create MinIO storage"
        );
        format!("Failed to create MinIO storage: {}", e)
    })?;

    // Determine bbox to read
    // For non-geographic projections (Lambert Conformal, etc.), we must read the
    // full grid because the relationship between grid indices and geographic
    // coordinates is non-linear. Partial bbox reads only work for regular lat/lon grids.
    // The `requires_full_grid` flag is set in model YAML configs based on projection.

    let read_bbox = if let Some(bbox_arr) = bbox {
        if !requires_full_grid {
            GpBoundingBox::new(
                bbox_arr[0] as f64,
                bbox_arr[1] as f64,
                bbox_arr[2] as f64,
                bbox_arr[3] as f64,
            )
        } else {
            // For non-geographic grids (HRRR Lambert), always read full grid
            // The resampling step will handle the projection transformation
            debug!(
                model = %entry.model,
                "Using full grid read for non-geographic projection"
            );
            zarr_meta.bbox
        }
    } else {
        // Read full grid
        zarr_meta.bbox
    };

    // Check if this dataset has multiscale (pyramid) data
    // If so, use resolution-aware loading to fetch from the optimal pyramid level
    let multiscale_metadata = parse_multiscale_metadata(zarr_json);

    let (region, pyramid_level_used) = if let (Some(ms_meta), Some(out_size)) =
        (multiscale_metadata, output_size)
    {
        // Use multiscale loading - select optimal pyramid level based on output size
        if ms_meta.num_levels() > 1 {
            let ms_factory = MultiscaleGridProcessorFactory::new(
                store.clone(),
                &zarr_path,
                ms_meta,
                factory.chunk_cache(),
                factory.config().clone(),
            );

            let start = Instant::now();
            let (region, level) = ms_factory
                .read_region_for_output(&read_bbox, out_size)
                .await
                .map_err(|e| {
                    error!(
                        error = %e,
                        zarr_path = %zarr_path,
                        bbox = ?read_bbox,
                        output_size = ?out_size,
                        "Failed to read multiscale Zarr region"
                    );
                    format!("Failed to read multiscale Zarr region: {}", e)
                })?;
            let read_duration = start.elapsed();

            info!(
                width = region.width,
                height = region.height,
                pyramid_level = level,
                read_ms = read_duration.as_millis(),
                output_size = ?out_size,
                "Loaded from pyramid level {} (optimal for output size {:?})",
                level, out_size
            );

            (region, Some(level))
        } else {
            // Only native level available, use standard loading
            let region =
                load_region_from_native(store.clone(), &zarr_path, &zarr_meta, &read_bbox, factory)
                    .await?;
            (region, Some(0u32))
        }
    } else {
        // No multiscale metadata or no output_size specified - use standard single-level loading
        let region =
            load_region_from_native(store.clone(), &zarr_path, &zarr_meta, &read_bbox, factory)
                .await?;
        (region, None)
    };

    if let Some(level) = pyramid_level_used {
        debug!(
            zarr_path = %zarr_path,
            pyramid_level = level,
            region_width = region.width,
            region_height = region.height,
            "Used pyramid level for data loading"
        );
    }

    // Return actual bbox from the region (important for partial reads)
    let actual_bbox = [
        region.bbox.min_lon as f32,
        region.bbox.min_lat as f32,
        region.bbox.max_lon as f32,
        region.bbox.max_lat as f32,
    ];

    info!(
        width = region.width,
        height = region.height,
        data_points = region.data.len(),
        actual_bbox_min_lon = actual_bbox[0],
        actual_bbox_max_lon = actual_bbox[2],
        pyramid_level = ?pyramid_level_used,
        "Loaded Zarr region"
    );

    // Check if source grid uses 0-360 longitude (like GFS)
    // This must be based on the full grid bbox, not the partial region bbox
    let grid_uses_360 = zarr_meta.bbox.min_lon >= 0.0 && zarr_meta.bbox.max_lon > 180.0;

    // Check if this is GOES data (has geostationary projection metadata in zarr)
    // GOES data stored in Zarr is already reprojected to geographic coordinates,
    // so we don't need the goes_projection params for rendering
    let goes_projection = None;

    // Extract native units from Zarr metadata (source of truth)
    let native_units = if zarr_meta.units.is_empty() || zarr_meta.units == "unknown" {
        debug!(
            model = %entry.model,
            parameter = %entry.parameter,
            "Zarr metadata has unknown/empty units"
        );
        String::new()
    } else {
        zarr_meta.units.clone()
    };

    Ok(GridData {
        data: region.data,
        width: region.width,
        height: region.height,
        bbox: Some(actual_bbox),
        goes_projection,
        grid_uses_360,
        native_units,
    })
}

/// Helper to load a region from the native (level 0) Zarr array.
/// Used when multiscale is not available or not needed.
async fn load_region_from_native<S>(
    store: S,
    zarr_path: &str,
    zarr_meta: &grid_processor::ZarrMetadata,
    read_bbox: &grid_processor::BoundingBox,
    factory: &GridProcessorFactory,
) -> Result<grid_processor::GridRegion, String>
where
    S: grid_processor::ReadableStorageTraits + Clone + Send + Sync + 'static,
{
    use grid_processor::{GridProcessor, ZarrGridProcessor};

    // Convert zarr_meta to GridMetadata for the processor
    let grid_metadata = grid_processor::GridMetadata {
        model: zarr_meta.model.clone(),
        parameter: zarr_meta.parameter.clone(),
        level: zarr_meta.level.clone(),
        units: zarr_meta.units.clone(),
        reference_time: zarr_meta.reference_time,
        forecast_hour: zarr_meta.forecast_hour,
        bbox: zarr_meta.bbox,
        shape: zarr_meta.shape,
        chunk_shape: zarr_meta.chunk_shape,
        num_chunks: zarr_meta.num_chunks,
        fill_value: zarr_meta.fill_value,
    };

    // For native loading, we need to append /0 to get level 0
    // Check if zarr_path already ends with a level number
    let level_path = if zarr_path.ends_with(".zarr") || zarr_path.ends_with(".zarr/") {
        format!("{}/0", zarr_path.trim_end_matches('/'))
    } else {
        zarr_path.to_string()
    };

    // Create processor with metadata from catalog (avoids metadata fetch from MinIO)
    let processor = ZarrGridProcessor::with_metadata(
        store,
        &level_path,
        grid_metadata.clone(),
        factory.chunk_cache(),
        factory.config().clone(),
    )
    .map_err(|e| {
        error!(
            error = %e,
            level_path = %level_path,
            shape = ?grid_metadata.shape,
            chunk_shape = ?grid_metadata.chunk_shape,
            "Failed to open Zarr array"
        );
        format!("Failed to open Zarr: {}", e)
    })?;

    // Read the region
    let start = std::time::Instant::now();
    let region = processor.read_region(read_bbox).await.map_err(|e| {
        error!(
            error = %e,
            level_path = %level_path,
            bbox = ?read_bbox,
            "Failed to read Zarr region"
        );
        format!("Failed to read Zarr region: {}", e)
    })?;
    let read_duration = start.elapsed();

    debug!(
        level_path = %level_path,
        width = region.width,
        height = region.height,
        read_ms = read_duration.as_millis(),
        "Loaded native Zarr region"
    );

    Ok(region)
}

/// Query a single point value from Zarr storage.
///
/// This is optimized for GetFeatureInfo requests - it reads only the single chunk
/// containing the requested point, making it much more efficient than loading
/// the entire grid.
///
/// # Arguments
/// * `factory` - Grid processor factory with chunk cache
/// * `entry` - Catalog entry with zarr_metadata
/// * `lon` - Longitude in degrees (-180 to 180 or 0 to 360)
/// * `lat` - Latitude in degrees (-90 to 90)
///
/// # Returns
/// * `Ok(Some(value))` - The data value at the point
/// * `Ok(None)` - Point is outside grid bounds or contains fill/NaN value
/// * `Err(...)` - Failed to read data
#[instrument(skip(factory, entry), fields(
    model = %entry.model,
    parameter = %entry.parameter,
    level = %entry.level,
    storage_path = %entry.storage_path,
))]
pub async fn query_point_from_zarr(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    lon: f64,
    lat: f64,
) -> Result<Option<f32>, String> {
    use grid_processor::{create_minio_storage, GridProcessor, ZarrGridProcessor, ZarrMetadata};

    // Parse zarr_metadata from catalog entry
    let zarr_json = entry.zarr_metadata.as_ref()
        .ok_or_else(|| {
            error!(model = %entry.model, parameter = %entry.parameter, "No zarr_metadata in catalog entry");
            "No zarr_metadata in catalog entry".to_string()
        })?;

    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| {
            error!(model = %entry.model, parameter = %entry.parameter, error = %e, "Failed to parse zarr_metadata");
            format!("Failed to parse zarr_metadata: {}", e)
        })?;

    // Check if source grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = zarr_meta.bbox.min_lon >= 0.0 && zarr_meta.bbox.max_lon > 180.0;

    // Normalize longitude to match grid coordinate system
    let query_lon = if grid_uses_360 && lon < 0.0 {
        lon + 360.0
    } else if !grid_uses_360 && lon > 180.0 {
        lon - 360.0
    } else {
        lon
    };

    debug!(
        lon = lon,
        lat = lat,
        query_lon = query_lon,
        grid_uses_360 = grid_uses_360,
        grid_bbox = ?zarr_meta.bbox,
        "Querying point from Zarr"
    );

    // Build storage path
    let zarr_path = if entry.storage_path.starts_with('/') {
        entry.storage_path.clone()
    } else {
        format!("/{}", entry.storage_path)
    };

    // Create MinIO storage using factory's config
    let minio_config = factory.minio_config();
    let store = create_minio_storage(minio_config).map_err(|e| {
        error!(error = %e, "Failed to create MinIO storage");
        format!("Failed to create MinIO storage: {}", e)
    })?;

    // Convert zarr_meta to GridMetadata for the processor
    let grid_metadata = grid_processor::GridMetadata {
        model: zarr_meta.model.clone(),
        parameter: zarr_meta.parameter.clone(),
        level: zarr_meta.level.clone(),
        units: zarr_meta.units.clone(),
        reference_time: zarr_meta.reference_time,
        forecast_hour: zarr_meta.forecast_hour,
        bbox: zarr_meta.bbox,
        shape: zarr_meta.shape,
        chunk_shape: zarr_meta.chunk_shape,
        num_chunks: zarr_meta.num_chunks,
        fill_value: zarr_meta.fill_value,
    };

    // Create processor with metadata from catalog
    let processor = ZarrGridProcessor::with_metadata(
        store,
        &zarr_path,
        grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    )
    .map_err(|e| {
        error!(error = %e, zarr_path = %zarr_path, "Failed to open Zarr array");
        format!("Failed to open Zarr: {}", e)
    })?;

    // Query the point value (reads only the chunk containing this point)
    let start = Instant::now();
    let value = processor.read_point(query_lon, lat).await.map_err(|e| {
        error!(error = %e, lon = query_lon, lat = lat, "Failed to read point from Zarr");
        format!("Failed to read point: {}", e)
    })?;
    let read_duration = start.elapsed();

    info!(
        lon = lon,
        lat = lat,
        value = ?value,
        read_ms = read_duration.as_millis(),
        "Zarr point query complete"
    );

    Ok(value)
}
