//! Wind barb rendering and wind component loading.
//!
//! This module provides functions for rendering wind barbs from U (UGRD) and V (VGRD)
//! wind component data. It supports both GRIB2 and Zarr data sources.
//!
//! Key features:
//! - Tile-based wind barb rendering with expanded tile support for seamless boundaries
//! - Level-aware wind component loading (e.g., 10m, 850mb)
//! - Zarr and GRIB2 data source support
//! - Geographic alignment for consistent barb positioning across tiles

use renderer::gradient;
use renderer::barbs::{self, BarbConfig};
use storage::{Catalog, CatalogEntry, GribCache};
use std::time::Instant;
use tracing::info;

use super::loaders::find_parameter_in_grib;
use super::resampling::resample_for_model_geographic;
use crate::state::GridProcessorFactory;

// ============================================================================
// Public rendering functions
// ============================================================================

/// Render wind barbs combining U and V component data using expanded tile rendering.
/// This renders a 3x3 grid of tiles and crops the center to ensure seamless boundaries.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `grid_processor_factory`: Optional factory for Zarr data access
/// - `model`: Weather model name (e.g., "gfs")
/// - `tile_coord`: Optional tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_tile(
    grib_cache: &GribCache,
    catalog: &Catalog,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // First get the catalog entries to check for Zarr metadata
    let u_entry = get_wind_entry(catalog, model, "UGRD", forecast_hour, None).await?;
    let v_entry = get_wind_entry(catalog, model, "VGRD", forecast_hour, None).await?;
    
    // Determine if we can use Zarr (both entries have zarr_metadata and we have a factory)
    let use_zarr = u_entry.zarr_metadata.is_some() 
        && v_entry.zarr_metadata.is_some()
        && grid_processor_factory.is_some();
    
    // Load U and V component data - use Zarr if available, otherwise GRIB
    let (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360) = if use_zarr {
        let factory = grid_processor_factory.unwrap();
        info!(
            model = model,
            u_path = %u_entry.storage_path,
            v_path = %v_entry.storage_path,
            "Loading wind components from Zarr"
        );
        load_wind_components_from_zarr(factory, &u_entry, &v_entry, None).await?
    } else {
        info!(
            model = model,
            "Loading wind components from GRIB (Zarr not available)"
        );
        load_wind_components(grib_cache, catalog, model, forecast_hour).await?
    };
    
    // Determine if we should use expanded rendering
    let (render_bbox, render_width, render_height, needs_crop) = if let Some(coord) = tile_coord {
        let config = ExpandedTileConfig::tiles_3x3();
        let expanded_bbox = expanded_tile_bbox(&coord, &config);
        
        // Calculate actual expanded dimensions
        let (exp_w, exp_h) = wms_common::tile::actual_expanded_dimensions(&coord, &config);
        
        (
            [
                expanded_bbox.min_x as f32,
                expanded_bbox.min_y as f32,
                expanded_bbox.max_x as f32,
                expanded_bbox.max_y as f32,
            ],
            exp_w as usize,
            exp_h as usize,
            Some((coord, config)),
        )
    } else {
        (bbox, width as usize, height as usize, None)
    };
    
    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        "Rendering wind barbs"
    );
    
    // Resample data to the render bbox (model-aware for proper projection handling)
    let u_resampled = resample_for_model_geographic(
        &u_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    let v_resampled = resample_for_model_geographic(
        &v_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    
    // Render wind barbs with geographic alignment
    let config = barbs::BarbConfig::default();
    let barb_pixels = barbs::render_wind_barbs_aligned(
        &u_resampled,
        &v_resampled,
        render_width,
        render_height,
        render_bbox,
        &config,
    );
    
    // Crop to center tile if we used expanded rendering
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        crop_center_tile(&barb_pixels, render_width as u32, &coord, &tile_config)
    } else {
        barb_pixels
    };
    
    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Render wind barbs combining U and V component data with optional level/elevation.
/// This renders a 3x3 grid of tiles and crops the center to ensure seamless boundaries.
///
/// # Arguments
/// - `grib_cache`: GRIB cache for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `grid_processor_factory`: Optional factory for Zarr data access
/// - `model`: Weather model name (e.g., "gfs")
/// - `tile_coord`: Optional tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `level`: Optional vertical level/elevation (e.g., "500 mb", "10 m above ground")
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_tile_with_level(
    grib_cache: &GribCache,
    catalog: &Catalog,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{ExpandedTileConfig, expanded_tile_bbox, crop_center_tile};
    
    // First get the catalog entries to check for Zarr metadata
    let u_entry = get_wind_entry(catalog, model, "UGRD", forecast_hour, level).await?;
    let v_entry = get_wind_entry(catalog, model, "VGRD", forecast_hour, level).await?;
    
    // Determine if we can use Zarr (both entries have zarr_metadata and we have a factory)
    let use_zarr = u_entry.zarr_metadata.is_some() 
        && v_entry.zarr_metadata.is_some()
        && grid_processor_factory.is_some();
    
    // Load U and V component data - use Zarr if available, otherwise GRIB
    let (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360) = if use_zarr {
        let factory = grid_processor_factory.unwrap();
        info!(
            model = model,
            u_path = %u_entry.storage_path,
            v_path = %v_entry.storage_path,
            "Loading wind components from Zarr"
        );
        load_wind_components_from_zarr(factory, &u_entry, &v_entry, None).await?
    } else {
        info!(
            model = model,
            "Loading wind components from GRIB (Zarr not available)"
        );
        load_wind_components_with_level(grib_cache, catalog, model, forecast_hour, level).await?
    };
    
    // Determine if we should use expanded rendering
    let (render_bbox, render_width, render_height, needs_crop) = if let Some(coord) = tile_coord {
        let config = ExpandedTileConfig::tiles_3x3();
        let expanded_bbox = expanded_tile_bbox(&coord, &config);
        
        // Calculate actual expanded dimensions
        let (exp_w, exp_h) = wms_common::tile::actual_expanded_dimensions(&coord, &config);
        
        (
            [
                expanded_bbox.min_x as f32,
                expanded_bbox.min_y as f32,
                expanded_bbox.max_x as f32,
                expanded_bbox.max_y as f32,
            ],
            exp_w as usize,
            exp_h as usize,
            Some((coord, config)),
        )
    } else {
        (bbox, width as usize, height as usize, None)
    };
    
    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        level = ?level,
        "Rendering wind barbs with level"
    );
    
    // Resample data to the render bbox (model-aware for proper projection handling)
    let u_resampled = resample_for_model_geographic(
        &u_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    let v_resampled = resample_for_model_geographic(
        &v_data, grid_width, grid_height,
        render_width, render_height, render_bbox, data_bounds, model, grid_uses_360
    );
    
    // Render wind barbs with geographic alignment
    let config = barbs::BarbConfig::default();
    let barb_pixels = barbs::render_wind_barbs_aligned(
        &u_resampled,
        &v_resampled,
        render_width,
        render_height,
        render_bbox,
        &config,
    );
    
    // Crop to center tile if we used expanded rendering
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        crop_center_tile(&barb_pixels, render_width as u32, &coord, &tile_config)
    } else {
        barb_pixels
    };
    
    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Render wind barbs combining U and V component data
///
/// # Arguments
/// - `storage`: Object storage for retrieving GRIB2 files
/// - `catalog`: Catalog for finding datasets
/// - `grid_processor_factory`: Optional factory for Zarr data access
/// - `model`: Weather model name (e.g., "gfs")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]
/// - `barb_spacing`: Optional spacing between barbs in pixels (default: 50)
///
/// # Returns
/// PNG image data as bytes
pub async fn render_wind_barbs_layer(
    grib_cache: &GribCache,
    catalog: &Catalog,
    grid_processor_factory: Option<&GridProcessorFactory>,
    model: &str,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    barb_spacing: Option<usize>,
    forecast_hour: Option<u32>,
) -> Result<Vec<u8>, String> {
    use tracing::debug;
    
    // Get catalog entries for U and V components
    let u_entry = get_wind_entry(catalog, model, "UGRD", forecast_hour, None).await?;
    let v_entry = get_wind_entry(catalog, model, "VGRD", forecast_hour, None).await?;
    
    // Determine if we can use Zarr (both entries have zarr_metadata and we have a factory)
    let use_zarr = u_entry.zarr_metadata.is_some() 
        && v_entry.zarr_metadata.is_some()
        && grid_processor_factory.is_some();
    
    // Load U and V component data - use Zarr if available, otherwise GRIB
    let (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360) = if use_zarr {
        let factory = grid_processor_factory.unwrap();
        info!(
            model = model,
            u_path = %u_entry.storage_path,
            v_path = %v_entry.storage_path,
            "Loading wind components from Zarr for layer render"
        );
        load_wind_components_from_zarr(factory, &u_entry, &v_entry, bbox).await?
    } else {
        info!(
            model = model,
            "Loading wind components from GRIB for layer render"
        );
        // Fall back to GRIB loading
        let u_grib = grib_cache
            .get(&u_entry.storage_path)
            .await
            .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

        let v_grib = grib_cache
            .get(&v_entry.storage_path)
            .await
            .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

        // Parse GRIB2 messages - use level from catalog entry to find correct message
        let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
        let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;

        // Unpack grid data
        let u_data = u_msg
            .unpack_data()
            .map_err(|e| format!("Unpacking U failed: {}", e))?;

        let v_data = v_msg
            .unpack_data()
            .map_err(|e| format!("Unpacking V failed: {}", e))?;

        let (gh, gw) = u_msg.grid_dims();
        let data_bounds = [
            u_entry.bbox.min_x as f32,
            u_entry.bbox.min_y as f32,
            u_entry.bbox.max_x as f32,
            u_entry.bbox.max_y as f32,
        ];
        let grid_uses_360 = u_entry.bbox.min_x >= 0.0 && u_entry.bbox.max_x > 180.0;
        
        (u_data, v_data, gw as usize, gh as usize, data_bounds, grid_uses_360)
    };
    
    // Debug: Log data statistics
    let u_stats: (f32, f32, f32) = u_data.iter()
        .filter(|v| !v.is_nan())
        .fold((f32::MAX, f32::MIN, 0.0), |(min, max, sum), &v| {
            (min.min(v), max.max(v), sum + v)
        });
    let v_stats: (f32, f32, f32) = v_data.iter()
        .filter(|v| !v.is_nan())
        .fold((f32::MAX, f32::MIN, 0.0), |(min, max, sum), &v| {
            (min.min(v), max.max(v), sum + v)
        });
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_min = u_stats.0,
        u_max = u_stats.1,
        v_min = v_stats.0,
        v_max = v_stats.1,
        "Loaded wind component data"
    );

    // Validate data sizes
    if u_data.len() != grid_width * grid_height {
        return Err(format!(
            "U grid size mismatch: {} vs {}x{}",
            u_data.len(),
            grid_width,
            grid_height
        ));
    }

    if v_data.len() != grid_width * grid_height {
        return Err(format!(
            "V grid size mismatch: {} vs {}x{}",
            v_data.len(),
            grid_width,
            grid_height
        ));
    }

    // Prepare rendering parameters
    let output_width = width as usize;
    let output_height = height as usize;
    let spacing = barb_spacing.unwrap_or(50);

    // If bbox is specified, we need to resample to that region
    let (u_to_render, v_to_render, render_width, render_height) = if let Some(bbox) = bbox {
        info!(
            bbox_min_lon = bbox[0],
            bbox_min_lat = bbox[1],
            bbox_max_lon = bbox[2],
            bbox_max_lat = bbox[3],
            output_width = output_width,
            output_height = output_height,
            "Resampling wind data for bbox"
        );
        
        // Resample from geographic coordinates (model-aware for proper projection handling)
        let u_resampled = resample_for_model_geographic(&u_data, grid_width, grid_height, output_width, output_height, bbox, data_bounds, model, grid_uses_360);
        let v_resampled = resample_for_model_geographic(&v_data, grid_width, grid_height, output_width, output_height, bbox, data_bounds, model, grid_uses_360);
        
        // Debug: check resampled data statistics
        let u_min = u_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MAX, f32::min);
        let u_max = u_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
        let v_min = v_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MAX, f32::min);
        let v_max = v_resampled.iter().cloned().filter(|v| !v.is_nan()).fold(f32::MIN, f32::max);
        
        // Check if data has variation
        let u_range = u_max - u_min;
        let v_range = v_max - v_min;
        
        info!(
            u_min = format!("{:.2}", u_min),
            u_max = format!("{:.2}", u_max),
            u_range = format!("{:.2}", u_range),
            v_min = format!("{:.2}", v_min),
            v_max = format!("{:.2}", v_max),
            v_range = format!("{:.2}", v_range),
            "Resampled wind data range"
        );
        
        if u_range < 0.1 && v_range < 0.1 {
            info!("WARNING: Wind data appears uniform across region - all barbs will point same direction");
        }
        
        // Sample some positions to verify variation
        let positions = renderer::barbs::calculate_barb_positions(output_width, output_height, spacing as u32);
        if positions.len() >= 4 {
            for (i, (x, y)) in positions.iter().take(4).enumerate() {
                let idx = y * output_width + x;
                if idx < u_resampled.len() {
                    let u = u_resampled[idx];
                    let v = v_resampled[idx];
                    let (speed, dir) = renderer::barbs::uv_to_speed_direction(u, v);
                    debug!(
                        pos = i,
                        x = x,
                        y = y,
                        u = u,
                        v = v,
                        speed = speed,
                        dir_deg = dir.to_degrees(),
                        "Barb position sample"
                    );
                }
            }
        }
        
        (u_resampled, v_resampled, output_width, output_height)
    } else {
        // Full globe or resample to output size
        let u_resampled = if grid_width != output_width || grid_height != output_height {
            gradient::resample_grid(&u_data, grid_width, grid_height, output_width, output_height)
        } else {
            u_data.clone()
        };

        let v_resampled = if grid_width != output_width || grid_height != output_height {
            gradient::resample_grid(&v_data, grid_width, grid_height, output_width, output_height)
        } else {
            v_data.clone()
        };

        (u_resampled, v_resampled, output_width, output_height)
    };

    // Render wind barbs
    let config = BarbConfig::default();
    
    info!(
        render_width = render_width,
        render_height = render_height,
        barb_spacing = config.spacing,
        barb_size = config.size,
        "Rendering wind barbs"
    );
    
    // Use geographically-aligned positioning when bbox is available
    // This ensures barbs align across tile boundaries
    let barb_pixels = if let Some(bbox) = bbox {
        barbs::render_wind_barbs_aligned(
            &u_to_render,
            &v_to_render,
            render_width,
            render_height,
            bbox,
            &config,
        )
    } else {
        barbs::render_wind_barbs(
            &u_to_render,
            &v_to_render,
            render_width,
            render_height,
            &config,
        )
    };
    
    // Debug: check rendered pixels
    let non_transparent = barb_pixels.chunks(4).filter(|c| c[3] > 0).count();
    info!(
        non_transparent = non_transparent,
        total_pixels = render_width * render_height,
        "Wind barb rendering complete"
    );

    // Encode as PNG
    renderer::png::create_png(&barb_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

// ============================================================================
// Helper functions
// ============================================================================

/// Get a wind component catalog entry (UGRD or VGRD) for the given parameters.
/// This is a helper function to look up catalog entries for wind components.
pub(crate) async fn get_wind_entry(
    catalog: &Catalog,
    model: &str,
    parameter: &str,  // "UGRD" or "VGRD"
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<CatalogEntry, String> {
    match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available for hour {} level {}", parameter, hour, lev))
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, parameter, hour)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available for hour {}", parameter, hour))
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available at level {}", parameter, lev))
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
                .map_err(|e| format!("Failed to get {}: {}", parameter, e))?
                .ok_or_else(|| format!("No {} data available", parameter))
        }
    }
}

// ============================================================================
// Wind component loading functions
// ============================================================================

/// Load U and V wind component data from cache/storage with optional level
pub(crate) async fn load_wind_components_with_level(
    grib_cache: &GribCache,
    catalog: &Catalog,
    model: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4], bool), String> {
    // Load U component (UGRD) with level support
    let u_entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, "UGRD", hour, lev)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available for hour {} level {}", hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, "UGRD", hour)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available for hour {}", hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, "UGRD", lev)
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| format!("No UGRD data available at level {}", lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, "UGRD")
                .await
                .map_err(|e| format!("Failed to get UGRD: {}", e))?
                .ok_or_else(|| "No UGRD data available".to_string())?
        }
    };

    // Load V component (VGRD) with level support
    let v_entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, "VGRD", hour, lev)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available for hour {} level {}", hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, "VGRD", hour)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available for hour {}", hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, "VGRD", lev)
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| format!("No VGRD data available at level {}", lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, "VGRD")
                .await
                .map_err(|e| format!("Failed to get VGRD: {}", e))?
                .ok_or_else(|| "No VGRD data available".to_string())?
        }
    };

    // Load GRIB2 files from cache
    let u_grib = grib_cache
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = grib_cache
        .get(&v_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

    // Parse GRIB2 messages - use level from catalog entry to find correct message
    let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
    let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;

    // Unpack grid data
    let u_data = u_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking U failed: {}", e))?;

    let v_data = v_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking V failed: {}", e))?;

    let (grid_height, grid_width) = u_msg.grid_dims();
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_level = %u_entry.level,
        v_level = %v_entry.level,
        "Loaded wind component data with level"
    );

    // Both U and V should have same bbox - use U's bbox
    let data_bounds = [
        u_entry.bbox.min_x as f32,
        u_entry.bbox.min_y as f32,
        u_entry.bbox.max_x as f32,
        u_entry.bbox.max_y as f32,
    ];
    
    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = u_entry.bbox.min_x >= 0.0 && u_entry.bbox.max_x > 180.0;
    
    Ok((u_data, v_data, grid_width as usize, grid_height as usize, data_bounds, grid_uses_360))
}

/// Load U and V wind component data from Zarr storage.
/// 
/// This function loads both UGRD and VGRD components from Zarr, enabling efficient
/// chunked reads for wind barb rendering.
/// 
/// # Arguments
/// * `factory` - Grid processor factory with shared chunk cache
/// * `u_entry` - Catalog entry for U component (UGRD)
/// * `v_entry` - Catalog entry for V component (VGRD)
/// * `bbox` - Optional bounding box for partial reads (used for tile rendering)
/// 
/// # Returns
/// Tuple of (u_data, v_data, grid_width, grid_height, data_bounds, grid_uses_360)
pub(crate) async fn load_wind_components_from_zarr(
    factory: &GridProcessorFactory,
    u_entry: &CatalogEntry,
    v_entry: &CatalogEntry,
    bbox: Option<[f32; 4]>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4], bool), String> {
    use grid_processor::{
        BoundingBox as GpBoundingBox,
        GridProcessor, ZarrGridProcessor, ZarrMetadata,
        MinioConfig, create_minio_storage,
    };
    
    // Parse zarr_metadata for U component
    let u_zarr_json = u_entry.zarr_metadata.as_ref()
        .ok_or_else(|| "No zarr_metadata in U (UGRD) catalog entry".to_string())?;
    let u_zarr_meta = ZarrMetadata::from_json(u_zarr_json)
        .map_err(|e| format!("Failed to parse U zarr_metadata: {}", e))?;
    
    // Parse zarr_metadata for V component
    let v_zarr_json = v_entry.zarr_metadata.as_ref()
        .ok_or_else(|| "No zarr_metadata in V (VGRD) catalog entry".to_string())?;
    let v_zarr_meta = ZarrMetadata::from_json(v_zarr_json)
        .map_err(|e| format!("Failed to parse V zarr_metadata: {}", e))?;
    
    // Verify both grids have the same shape
    if u_zarr_meta.shape != v_zarr_meta.shape {
        return Err(format!(
            "U and V grid shapes don't match: {:?} vs {:?}",
            u_zarr_meta.shape, v_zarr_meta.shape
        ));
    }
    
    info!(
        u_path = %u_entry.storage_path,
        v_path = %v_entry.storage_path,
        shape = ?u_zarr_meta.shape,
        chunk_shape = ?u_zarr_meta.chunk_shape,
        bbox = ?bbox,
        "Loading wind components from Zarr"
    );
    
    // Create MinIO storage
    let minio_config = MinioConfig::from_env();
    let store = create_minio_storage(&minio_config)
        .map_err(|e| format!("Failed to create MinIO storage: {}", e))?;
    
    // Build storage paths
    let u_zarr_path = if u_entry.storage_path.starts_with('/') {
        u_entry.storage_path.clone()
    } else {
        format!("/{}", u_entry.storage_path)
    };
    let v_zarr_path = if v_entry.storage_path.starts_with('/') {
        v_entry.storage_path.clone()
    } else {
        format!("/{}", v_entry.storage_path)
    };
    
    // Create GridMetadata for U processor
    let u_grid_metadata = grid_processor::GridMetadata {
        model: u_zarr_meta.model.clone(),
        parameter: u_zarr_meta.parameter.clone(),
        level: u_zarr_meta.level.clone(),
        units: u_zarr_meta.units.clone(),
        reference_time: u_zarr_meta.reference_time,
        forecast_hour: u_zarr_meta.forecast_hour,
        bbox: u_zarr_meta.bbox,
        shape: u_zarr_meta.shape,
        chunk_shape: u_zarr_meta.chunk_shape,
        num_chunks: u_zarr_meta.num_chunks,
        fill_value: u_zarr_meta.fill_value,
    };
    
    // Create U processor
    let u_processor = ZarrGridProcessor::with_metadata(
        store.clone(),
        &u_zarr_path,
        u_grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| format!("Failed to open U Zarr: {}", e))?;
    
    // Create GridMetadata for V processor
    let v_grid_metadata = grid_processor::GridMetadata {
        model: v_zarr_meta.model.clone(),
        parameter: v_zarr_meta.parameter.clone(),
        level: v_zarr_meta.level.clone(),
        units: v_zarr_meta.units.clone(),
        reference_time: v_zarr_meta.reference_time,
        forecast_hour: v_zarr_meta.forecast_hour,
        bbox: v_zarr_meta.bbox,
        shape: v_zarr_meta.shape,
        chunk_shape: v_zarr_meta.chunk_shape,
        num_chunks: v_zarr_meta.num_chunks,
        fill_value: v_zarr_meta.fill_value,
    };
    
    // Create V processor
    let v_processor = ZarrGridProcessor::with_metadata(
        store,
        &v_zarr_path,
        v_grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    ).map_err(|e| format!("Failed to open V Zarr: {}", e))?;
    
    // Determine bbox to read
    let read_bbox = if let Some(bbox_arr) = bbox {
        GpBoundingBox::new(
            bbox_arr[0] as f64,
            bbox_arr[1] as f64,
            bbox_arr[2] as f64,
            bbox_arr[3] as f64,
        )
    } else {
        // Read full grid
        u_zarr_meta.bbox
    };
    
    // Read both regions
    let start = Instant::now();
    let u_region = u_processor.read_region(&read_bbox).await
        .map_err(|e| format!("Failed to read U Zarr region: {}", e))?;
    let v_region = v_processor.read_region(&read_bbox).await
        .map_err(|e| format!("Failed to read V Zarr region: {}", e))?;
    let read_duration = start.elapsed();
    
    // Verify regions have same dimensions
    if u_region.width != v_region.width || u_region.height != v_region.height {
        return Err(format!(
            "U and V region dimensions don't match: {}x{} vs {}x{}",
            u_region.width, u_region.height, v_region.width, v_region.height
        ));
    }
    
    let grid_width = u_region.width;
    let grid_height = u_region.height;
    
    // Use the bbox from the region (important for partial reads)
    let data_bounds = [
        u_region.bbox.min_lon as f32,
        u_region.bbox.min_lat as f32,
        u_region.bbox.max_lon as f32,
        u_region.bbox.max_lat as f32,
    ];
    
    // Check if source grid uses 0-360 longitude (like GFS)
    // This must be based on the full grid bbox, not the partial region bbox
    let grid_uses_360 = u_zarr_meta.bbox.min_lon >= 0.0 && u_zarr_meta.bbox.max_lon > 180.0;
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_level = %u_entry.level,
        v_level = %v_entry.level,
        read_ms = read_duration.as_millis(),
        grid_uses_360 = grid_uses_360,
        "Loaded wind components from Zarr"
    );
    
    Ok((u_region.data, v_region.data, grid_width, grid_height, data_bounds, grid_uses_360))
}

/// Load U and V wind component data from cache/storage
pub(crate) async fn load_wind_components(
    grib_cache: &GribCache,
    catalog: &Catalog,
    model: &str,
    forecast_hour: Option<u32>,
) -> Result<(Vec<f32>, Vec<f32>, usize, usize, [f32; 4], bool), String> {
    // Load U component (UGRD)
    let u_entry = if let Some(hour) = forecast_hour {
        catalog
            .find_by_forecast_hour(model, "UGRD", hour)
            .await
            .map_err(|e| format!("Failed to get UGRD: {}", e))?
            .ok_or_else(|| format!("No UGRD data available for hour {}", hour))?
    } else {
        catalog
            .get_latest_run_earliest_forecast(model, "UGRD")
            .await
            .map_err(|e| format!("Failed to get UGRD: {}", e))?
            .ok_or_else(|| "No UGRD data available".to_string())?
    };

    // Load V component (VGRD)
    let v_entry = if let Some(hour) = forecast_hour {
        catalog
            .find_by_forecast_hour(model, "VGRD", hour)
            .await
            .map_err(|e| format!("Failed to get VGRD: {}", e))?
            .ok_or_else(|| format!("No VGRD data available for hour {}", hour))?
    } else {
        catalog
            .get_latest_run_earliest_forecast(model, "VGRD")
            .await
            .map_err(|e| format!("Failed to get VGRD: {}", e))?
            .ok_or_else(|| "No VGRD data available".to_string())?
    };

    // Load GRIB2 files from cache
    let u_grib = grib_cache
        .get(&u_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load UGRD file: {}", e))?;

    let v_grib = grib_cache
        .get(&v_entry.storage_path)
        .await
        .map_err(|e| format!("Failed to load VGRD file: {}", e))?;

    // Parse GRIB2 messages - use level from catalog entry to find correct message
    let u_msg = find_parameter_in_grib(u_grib, "UGRD", Some(&u_entry.level))?;
    let v_msg = find_parameter_in_grib(v_grib, "VGRD", Some(&v_entry.level))?;

    // Unpack grid data
    let u_data = u_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking U failed: {}", e))?;

    let v_data = v_msg
        .unpack_data()
        .map_err(|e| format!("Unpacking V failed: {}", e))?;

    let (grid_height, grid_width) = u_msg.grid_dims();
    
    info!(
        grid_width = grid_width,
        grid_height = grid_height,
        u_level = %u_entry.level,
        v_level = %v_entry.level,
        "Loaded wind component data"
    );

    // Both U and V should have same bbox - use U's bbox
    let data_bounds = [
        u_entry.bbox.min_x as f32,
        u_entry.bbox.min_y as f32,
        u_entry.bbox.max_x as f32,
        u_entry.bbox.max_y as f32,
    ];

    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = u_entry.bbox.min_x >= 0.0 && u_entry.bbox.max_x > 180.0;

    Ok((u_data, v_data, grid_width as usize, grid_height as usize, data_bounds, grid_uses_360))
}
