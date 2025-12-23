//! Isoline (contour) rendering for weather data.
//!
//! This module provides functions for rendering isolines/contour lines from
//! gridded weather data. It supports various parameters like pressure, temperature,
//! and geopotential height.
//!
//! Key features:
//! - Style-based contour configuration (line color, width, intervals)
//! - Automatic level generation or explicit level specification
//! - Unit transformation (e.g., Pa to hPa, K to C)
//! - Special level highlighting (e.g., 1013 hPa, 0C freezing level)

use renderer::contour;
use renderer::style::ContourStyle;
use storage::Catalog;
use tracing::info;

use super::loaders::load_grid_data;
use super::resampling::resample_grid_for_bbox;
use grid_processor::GridProcessorFactory;

/// Render isolines (contour lines) for a single tile with optional level.
pub async fn render_isolines_tile_with_level(
    catalog: &Catalog,
    grid_processor_factory: &GridProcessorFactory,
    model: &str,
    parameter: &str,
    _tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    style_name: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::crop_center_tile;
    
    // Load style configuration - look up the specific style by name
    let style_config = ContourStyle::from_file_with_style(style_path, style_name)
        .map_err(|e| format!("Failed to load contour style '{}' from {}: {}", style_name, style_path, e))?;
    
    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {} level {}", model, parameter, hour, lev))?
        }
        (Some(hour), None) => {
            catalog
                .find_by_forecast_hour(model, parameter, hour)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?
        }
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
        }
    };
    
    // Load grid data from Zarr storage
    // Pass None for bbox to get full grid (isolines need global min/max for level generation)
    // Also pass None for output_size since we need full resolution for contour extraction
    // Always require full grid for isolines regardless of projection
    let grid_result = load_grid_data(
        grid_processor_factory,
        &entry,
        None,  // No bbox subset - we need full grid for contour level calculation
        None,  // No output_size - use native resolution for accurate contours
        true,  // Force full grid read for isolines
    ).await?;
    
    let grid_data = grid_result.data;
    let grid_width = grid_result.width;
    let grid_height = grid_result.height;
    
    // Find global min/max for level generation
    let (min_val, max_val) = grid_data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
            (min.min(val), max.max(val))
        });
    
    info!(
        parameter = parameter,
        min_val = min_val,
        max_val = max_val,
        grid_width = grid_width,
        grid_height = grid_height,
        "Loaded grid data for isolines"
    );
    
    // For isolines, we don't use expanded rendering because:
    // 1. Contours are continuous and don't need alignment across tiles like wind barbs
    // 2. Expanded rendering at low zoom can cause the bbox to span the entire world,
    //    leading to projection distortion when cropping in pixel space
    // Instead, we render each tile independently
    let (render_bbox, render_width, render_height, needs_crop) = (bbox, width as usize, height as usize, None);
    
    // Use actual bbox from grid data if available, otherwise fall back to entry.bbox
    let data_bounds = grid_result.bbox.unwrap_or_else(|| [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ]);
    
    // Use grid_uses_360 from result (for proper coordinate handling)
    let grid_uses_360 = grid_result.grid_uses_360;
    
    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        expanded = needs_crop.is_some(),
        "Rendering isolines"
    );
    
    // Resample data to the render bbox
    // Use Mercator projection when rendering for Web Mercator display
    let resampled_data_raw = resample_grid_for_bbox(
        &grid_data,
        grid_width,
        grid_height,
        render_width,
        render_height,
        render_bbox,
        data_bounds,
        use_mercator,
        model,
        grid_uses_360,
    );
    
    // Apply transform to convert data to display units (e.g., Pa -> hPa, K -> C)
    // This ensures contour levels (defined in display units) match the data
    let resampled_data: Vec<f32> = resampled_data_raw.iter()
        .map(|&v| renderer::style::apply_transform(v, style_config.transform.as_ref()))
        .collect();
    
    // Also transform min/max for level generation
    let transformed_min = renderer::style::apply_transform(min_val, style_config.transform.as_ref());
    let transformed_max = renderer::style::apply_transform(max_val, style_config.transform.as_ref());
    
    // Log resampled data stats for debugging (now in display units)
    let resampled_valid: Vec<f32> = resampled_data.iter().filter(|v| !v.is_nan()).copied().collect();
    let resampled_min = resampled_valid.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let resampled_max = resampled_valid.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let nan_count = resampled_data.iter().filter(|v| v.is_nan()).count();
    
    info!(
        resampled_min = resampled_min,
        resampled_max = resampled_max,
        valid_count = resampled_valid.len(),
        nan_count = nan_count,
        total = resampled_data.len(),
        transform = ?style_config.transform,
        "Resampled data stats for isolines (in display units)"
    );
    
    // Generate contour levels from style config (using transformed min/max)
    let levels = style_config.generate_levels(transformed_min, transformed_max);
    
    info!(
        num_levels = levels.len(),
        first_level = levels.first().copied().unwrap_or(0.0),
        last_level = levels.last().copied().unwrap_or(0.0),
        "Generated contour levels"
    );
    
    // Build special level configs from style
    // Since data is now in display units, special levels are used directly (no conversion needed)
    let special_levels: Vec<contour::SpecialLevelConfig> = style_config.contour.special_levels
        .as_ref()
        .map(|levels| {
            levels.iter().map(|sl| {
                contour::SpecialLevelConfig {
                    level: sl.value,  // Already in display units
                    line_color: sl.line_color,
                    line_width: sl.line_width,
                    label: sl.label.clone(),
                }
            }).collect()
        })
        .unwrap_or_default();
    
    // No label offset needed since data is already in display units
    let label_unit_offset = 0.0;
    
    // Build ContourConfig
    let contour_config = contour::ContourConfig {
        levels,
        line_width: style_config.contour.line_width,
        line_color: style_config.contour.line_color,
        smoothing_passes: style_config.contour.smoothing_passes.unwrap_or(1),
        labels_enabled: style_config.contour.labels.unwrap_or(false),
        label_font_size: style_config.contour.label_font_size.unwrap_or(10.0),
        label_spacing: style_config.contour.label_spacing.unwrap_or(150.0),
        label_unit_offset,
        special_levels,
    };
    
    // Render contours
    let contour_pixels = contour::render_contours(
        &resampled_data,
        render_width,
        render_height,
        &contour_config,
    );
    
    // Crop to center tile if we used expanded rendering
    let final_pixels = if let Some((coord, tile_config)) = needs_crop {
        crop_center_tile(&contour_pixels, render_width as u32, &coord, &tile_config)
    } else {
        contour_pixels
    };
    
    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}
