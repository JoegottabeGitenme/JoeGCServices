//! Shared weather data rendering logic.

mod colorscales;
mod isolines;
pub(crate) mod loaders;
mod resampling;
mod sampling;
mod types;
mod wind;

#[cfg(test)]
mod tests;

use crate::metrics::{DataSourceType, MetricsCollector};
use grid_processor::GridProcessorFactory;
use loaders::load_grid_data;
use renderer::numbers::{self, NumbersConfig};
use renderer::style::StyleConfig;
use resampling::{resample_grid_for_bbox, resample_grid_for_bbox_with_proj};
use std::time::Instant;
use storage::{Catalog, CatalogEntry};
use tracing::info;

// Re-export functions for internal use
pub(crate) use colorscales::render_with_style_file_indexed;
// Note: render_by_parameter is still available for fallback scenarios via direct module access

// Re-export public functions from submodules
pub use isolines::render_isolines_tile_with_level;
pub use sampling::query_point_value;
pub use wind::{
    render_wind_barbs_layer, render_wind_barbs_tile, render_wind_barbs_tile_with_level,
};

/// Render weather data with optional style configuration and level.
///
/// This is a convenience wrapper for callers that don't need observation time.
///
/// # Arguments
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector
/// - `grid_processor_factory`: Factory for Zarr-based grid access
/// - `model`: Weather model name
/// - `parameter`: Parameter name (e.g., "TMP", "WIND_SPEED")
/// - `forecast_hour`: Optional forecast hour; if None, uses latest
/// - `level`: Optional level/elevation (e.g., "500 mb", "2 m above ground")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box [min_lon, min_lat, max_lon, max_lat]; if None, renders full globe
/// - `style_file`: Path to style JSON file (from layer config)
/// - `style_name`: Optional style name to apply from configuration
/// - `use_mercator`: Use Web Mercator projection for resampling
/// - `requires_full_grid`: Force full grid read (for non-geographic projections)
///
/// # Returns
/// PNG image data as bytes
pub async fn render_weather_data_with_level(
    catalog: &Catalog,
    metrics: &MetricsCollector,
    grid_processor_factory: &GridProcessorFactory,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_file: &str,
    style_name: Option<&str>,
    use_mercator: bool,
    requires_full_grid: bool,
) -> Result<Vec<u8>, String> {
    render_weather_data(
        catalog,
        metrics,
        model,
        parameter,
        forecast_hour,
        None,
        level,
        width,
        height,
        bbox,
        style_file,
        style_name,
        use_mercator,
        grid_processor_factory,
        requires_full_grid,
    )
    .await
}

/// Render weather data to a PNG image.
///
/// This is the full-featured rendering function that supports:
/// - Forecast models (GFS, HRRR): Use `forecast_hour` parameter
/// - Observation data (MRMS, GOES): Use `observation_time` parameter
/// - Zarr-format grid data with efficient chunked reads
///
/// # Arguments
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector
/// - `model`: Weather model name
/// - `parameter`: Parameter name
/// - `forecast_hour`: Optional forecast hour for forecast models
/// - `observation_time`: Optional observation time (ISO8601) for observation data
/// - `level`: Optional vertical level
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Optional bounding box
/// - `style_file`: Path to style JSON file (from layer config)
/// - `style_name`: Optional style name within the file
/// - `use_mercator`: Use Web Mercator projection
/// - `grid_processor_factory`: Factory for Zarr-based grid access
/// - `requires_full_grid`: Force full grid read (for non-geographic projections like Lambert)
pub async fn render_weather_data(
    catalog: &Catalog,
    metrics: &MetricsCollector,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    observation_time: Option<chrono::DateTime<chrono::Utc>>,
    level: Option<&str>,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    style_file: &str,
    style_name: Option<&str>,
    use_mercator: bool,
    grid_processor_factory: &GridProcessorFactory,
    requires_full_grid: bool,
) -> Result<Vec<u8>, String> {
    // Record model-specific request
    let render_start = Instant::now();
    let weather_model = crate::metrics::WeatherModel::from_model(model);
    if let Some(wm) = weather_model {
        metrics.record_model_request(wm, parameter);
    }

    // Get dataset based on time specification
    let entry = {
        if let Some(obs_time) = observation_time {
            // Observation mode: find dataset closest to requested observation time
            info!(model = model, parameter = parameter, observation_time = ?obs_time, "Looking up observation data by time");
            let entry = catalog
                .find_by_time(model, parameter, obs_time)
                .await
                .map_err(|e| format!("Catalog query failed: {}", e))?
                .ok_or_else(|| {
                    format!(
                        "No observation data found for {}/{} at time {:?}",
                        model, parameter, obs_time
                    )
                })?;
            info!(
                model = model,
                parameter = parameter,
                requested_time = ?obs_time,
                found_time = ?entry.reference_time,
                storage_path = %entry.storage_path,
                "Found observation dataset"
            );
            entry
        } else {
            // Forecast mode: use forecast_hour and level
            match (forecast_hour, level) {
                (Some(hour), Some(lev)) => {
                    // Find dataset with matching forecast hour and level
                    catalog
                        .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                        .await
                        .map_err(|e| format!("Catalog query failed: {}", e))?
                        .ok_or_else(|| {
                            format!(
                                "No data found for {}/{} at hour {} level {}",
                                model, parameter, hour, lev
                            )
                        })?
                }
                (Some(hour), None) => {
                    // Find dataset with matching forecast hour (any level - defaults to first available)
                    catalog
                        .find_by_forecast_hour(model, parameter, hour)
                        .await
                        .map_err(|e| format!("Catalog query failed: {}", e))?
                        .ok_or_else(|| {
                            format!("No data found for {}/{} at hour {}", model, parameter, hour)
                        })?
                }
                (None, Some(lev)) => {
                    // Get latest run with earliest forecast hour at specific level
                    catalog
                        .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                        .await
                        .map_err(|e| format!("Catalog query failed: {}", e))?
                        .ok_or_else(|| {
                            format!("No data found for {}/{} at level {}", model, parameter, lev)
                        })?
                }
                (None, None) => {
                    // Get latest run with earliest forecast hour (any level)
                    catalog
                        .get_latest_run_earliest_forecast(model, parameter)
                        .await
                        .map_err(|e| format!("Catalog query failed: {}", e))?
                        .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?
                }
            }
        }
    };

    // Load grid data from Zarr storage with efficient chunked reads
    info!(
        parameter = parameter,
        level = %entry.level,
        storage_path = %entry.storage_path,
        model = model,
        "Loading grid data from Zarr"
    );

    let start = Instant::now();
    let grid_result = load_grid_data(
        grid_processor_factory,
        &entry,
        bbox,
        Some((width as usize, height as usize)), // Output tile size for pyramid selection
        requires_full_grid,
    )
    .await?;
    let load_duration = start.elapsed();
    metrics
        .record_grib_load(load_duration.as_micros() as u64)
        .await;

    // Record per-data-source parse metrics for dashboard
    let source_type = DataSourceType::from_model(model);
    metrics
        .record_data_source_parse(&source_type, load_duration.as_micros() as u64)
        .await;

    // Record model-specific fetch metrics
    if let Some(wm) = weather_model {
        metrics.record_model_fetch(wm, entry.file_size, load_duration.as_micros() as u64);
        if let Some(fhr) = forecast_hour {
            metrics.record_model_forecast_hour(wm, fhr);
        }
    }

    let grid_data = grid_result.data;
    let grid_width = grid_result.width;
    let grid_height = grid_result.height;
    let goes_projection = grid_result.goes_projection;

    if grid_data.len() != grid_width * grid_height {
        return Err(format!(
            "Grid data size mismatch: {} vs {}x{}",
            grid_data.len(),
            grid_width,
            grid_height
        ));
    }

    // For tile continuity, resample directly from the geographic grid using coordinates
    // This ensures adjacent tiles sample from the exact same grid points at boundaries
    let rendered_width = width as usize;
    let rendered_height = height as usize;

    // Use actual bbox from grid data if available (for Zarr partial reads),
    // otherwise fall back to entry.bbox (for GRIB2/NetCDF full grid reads)
    let data_bounds = grid_result.bbox.unwrap_or_else(|| {
        [
            entry.bbox.min_x as f32,
            entry.bbox.min_y as f32,
            entry.bbox.max_x as f32,
            entry.bbox.max_y as f32,
        ]
    });

    let start = Instant::now();
    let resampled_data = {
        if let Some(output_bbox) = bbox {
            // Resample grid data to output bbox using projection-aware resampling
            resample_grid_for_bbox_with_proj(
                &grid_data,
                grid_width,
                grid_height,
                rendered_width,
                rendered_height,
                output_bbox,
                data_bounds,
                use_mercator,
                model,
                goes_projection.as_ref(),
                grid_result.grid_uses_360,
            )
        } else {
            // No bbox - resample entire data grid
            if grid_width != rendered_width || grid_height != rendered_height {
                renderer::gradient::resample_grid(
                    &grid_data,
                    grid_width,
                    grid_height,
                    rendered_width,
                    rendered_height,
                )
            } else {
                grid_data.clone()
            }
        }
    };
    let resample_duration = start.elapsed();
    let resample_us = resample_duration.as_micros() as u64;
    metrics.record_resample(resample_us).await;

    // Record model-specific resample timing
    if let Some(wm) = weather_model {
        metrics.record_model_resample(wm, resample_us);
    }

    // Apply color rendering using indexed path for optimal performance
    // This uses pre-computed palettes and outputs palette indices directly
    let start = Instant::now();
    let png = {
        let render_result = render_with_style_file_indexed(
            &resampled_data,
            style_file,
            style_name,
            rendered_width,
            rendered_height,
        )?;

        // Encode to indexed PNG using pre-computed palette
        renderer::png::create_png_from_precomputed(
            &render_result.indices,
            rendered_width,
            rendered_height,
            &render_result.palette,
        )
        .map_err(|e| format!("PNG encoding failed: {}", e))?
    };
    let png_duration = start.elapsed();
    metrics
        .record_png_encode(png_duration.as_micros() as u64)
        .await;

    // Record model-specific render completion metrics
    let total_render_duration = render_start.elapsed();
    if let Some(wm) = weather_model {
        metrics.record_model_render(
            wm,
            parameter,
            total_render_duration.as_micros() as u64,
            true,
        );
        metrics.record_model_png_encode(wm, png_duration.as_micros() as u64);
    }

    Ok(png)
}

/// Render numeric values at grid points for a parameter.
///
/// # Arguments
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector for performance tracking
/// - `grid_processor_factory`: Factory for Zarr-based grid access
/// - `model`: Weather model name (e.g., "gfs")
/// - `parameter`: Parameter name (e.g., "TMP")
/// - `width`: Output image width
/// - `height`: Output image height
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat]
/// - `style_path`: Path to style configuration file (for color mapping)
/// - `forecast_hour`: Optional forecast hour
/// - `level`: Optional vertical level
/// - `use_mercator`: Use Web Mercator projection
/// - `requires_full_grid`: Force full grid read (for non-geographic projections)
///
/// # Returns
/// PNG image data as bytes
pub async fn render_numbers_tile(
    catalog: &Catalog,
    _metrics: &MetricsCollector,
    grid_processor_factory: &GridProcessorFactory,
    model: &str,
    parameter: &str,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
    requires_full_grid: bool,
) -> Result<Vec<u8>, String> {
    // Load style configuration for color mapping
    let style_json = std::fs::read_to_string(style_path)
        .map_err(|e| format!("Failed to read style file: {}", e))?;
    let style_config: StyleConfig = serde_json::from_str(&style_json)
        .map_err(|e| format!("Failed to parse style JSON: {}", e))?;

    // Get the first style definition (there should only be one in the file)
    let style_def = style_config
        .styles
        .values()
        .next()
        .ok_or_else(|| "No style definition found in style file".to_string())?;

    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => catalog
            .find_by_forecast_hour_and_level(model, parameter, hour, lev)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| {
                format!(
                    "No data found for {}/{} at hour {} level {}",
                    model, parameter, hour, lev
                )
            })?,
        (Some(hour), None) => catalog
            .find_by_forecast_hour(model, parameter, hour)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?,
        (None, Some(lev)) => catalog
            .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?,
        (None, None) => catalog
            .get_latest_run_earliest_forecast(model, parameter)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?,
    };

    // Load grid data from Zarr storage
    let grid_result = load_grid_data(
        grid_processor_factory,
        &entry,
        Some(bbox),
        Some((width as usize, height as usize)), // Output tile size for pyramid selection
        requires_full_grid,
    )
    .await?;

    let (grid_data, grid_width, grid_height) =
        (grid_result.data, grid_result.width, grid_result.height);

    info!(
        parameter = parameter,
        grid_width = grid_width,
        grid_height = grid_height,
        "Loaded grid data for numbers rendering"
    );

    // Get data bounds from catalog entry
    let data_bounds = [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ];

    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = entry.bbox.min_x >= 0.0 && entry.bbox.max_x > 180.0;

    // Resample data to the render bbox
    let resampled_data = resample_grid_for_bbox(
        &grid_data,
        grid_width,
        grid_height,
        width as usize,
        height as usize,
        bbox,
        data_bounds,
        use_mercator,
        model,
        grid_uses_360,
    );

    // Convert flat array to 2D grid for numbers renderer
    let grid_2d: Vec<Vec<f32>> = (0..height as usize)
        .map(|y| {
            (0..width as usize)
                .map(|x| resampled_data[y * width as usize + x])
                .collect()
        })
        .collect();

    // Determine unit conversion (e.g., K to C for temperature)
    let unit_conversion = if parameter.contains("TMP") || parameter.contains("TEMP") {
        Some(273.15) // Kelvin to Celsius
    } else {
        None
    };

    // Build NumbersConfig
    let numbers_config = NumbersConfig {
        spacing: 50, // 50 pixels between numbers
        font_size: 12.0,
        color_stops: style_def.stops.clone(),
        unit_conversion,
    };

    // Render numbers
    let img = numbers::render_numbers(&grid_2d, width, height, &numbers_config);

    // Convert RgbaImage to flat RGBA buffer
    let pixels = img.into_raw();

    // Encode as PNG
    renderer::png::create_png(&pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}

/// Render numbers by directly querying Zarr data at exact grid point locations.
/// This ensures the displayed values exactly match what GetFeatureInfo would return.
async fn render_numbers_direct_zarr(
    factory: &GridProcessorFactory,
    entry: &CatalogEntry,
    width: u32,
    height: u32,
    render_bbox: [f64; 4],
    visible_bbox: Option<[f64; 4]>,
    color_stops: &[renderer::numbers::ColorStop],
    unit_transform: numbers::UnitTransform,
    min_pixel_spacing: u32,
) -> Result<image::RgbaImage, String> {
    use grid_processor::{
        create_minio_storage, GridProcessor, MinioConfig, ZarrGridProcessor, ZarrMetadata,
    };

    // Parse zarr_metadata
    let zarr_json = entry
        .zarr_metadata
        .as_ref()
        .ok_or_else(|| "No zarr_metadata in catalog entry".to_string())?;

    let zarr_meta = ZarrMetadata::from_json(zarr_json)
        .map_err(|e| format!("Failed to parse zarr_metadata: {}", e))?;

    // Extract grid info
    let source_bbox = [
        zarr_meta.bbox.min_lon,
        zarr_meta.bbox.min_lat,
        zarr_meta.bbox.max_lon,
        zarr_meta.bbox.max_lat,
    ];
    let source_dims = zarr_meta.shape;
    let source_uses_360 = zarr_meta.bbox.min_lon >= 0.0 && zarr_meta.bbox.max_lon > 180.0;

    // Build storage path
    let zarr_path = if entry.storage_path.starts_with('/') {
        entry.storage_path.clone()
    } else {
        format!("/{}", entry.storage_path)
    };

    // Create MinIO storage
    let minio_config = MinioConfig::from_env();
    let store = create_minio_storage(&minio_config)
        .map_err(|e| format!("Failed to create MinIO storage: {}", e))?;

    // Create GridMetadata for the processor
    let grid_metadata = grid_processor::GridMetadata {
        model: entry.model.clone(),
        parameter: entry.parameter.clone(),
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

    // Create processor
    let processor = ZarrGridProcessor::with_metadata(
        store,
        &zarr_path,
        grid_metadata,
        factory.chunk_cache(),
        factory.config().clone(),
    )
    .map_err(|e| format!("Failed to open Zarr: {}", e))?;

    // Image types
    use image::{ImageBuffer, Rgba, RgbaImage};
    use imageproc::drawing::draw_text_mut;
    use rusttype::{Font, Scale};

    // Font setup - use the same font as the renderer crate
    let font_data: &[u8] = include_bytes!("../../../../crates/renderer/assets/DejaVuSansMono.ttf");
    let font = Font::try_from_bytes(font_data).expect("Failed to load font");
    let font_size = 12.0f32;
    let scale = Scale::uniform(font_size);

    let mut img: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    let [render_min_lon, render_min_lat, render_max_lon, render_max_lat] = render_bbox;
    let render_lon_range = render_max_lon - render_min_lon;
    let render_lat_range = render_max_lat - render_min_lat;

    let [source_min_lon, source_min_lat, source_max_lon, source_max_lat] = source_bbox;
    let (source_width, source_height) = source_dims;

    // Calculate source grid resolution
    let source_lon_step = (source_max_lon - source_min_lon) / (source_width - 1) as f64;
    let source_lat_step = (source_max_lat - source_min_lat) / (source_height - 1) as f64;

    // Get visible area
    let [vis_min_lon, vis_min_lat, vis_max_lon, vis_max_lat] = visible_bbox.unwrap_or(render_bbox);

    // Helper to convert longitude from -180/180 to 0-360 format if needed
    let to_source_lon = |lon: f64| -> f64 {
        if source_uses_360 && lon < 0.0 {
            lon + 360.0
        } else {
            lon
        }
    };

    // Helper to convert longitude from 0-360 to -180/180 format
    let from_source_lon = |lon: f64| -> f64 {
        if source_uses_360 && lon > 180.0 {
            lon - 360.0
        } else {
            lon
        }
    };

    // Convert render bbox to source coordinates
    let render_min_lon_src = to_source_lon(render_min_lon);
    let render_max_lon_src = to_source_lon(render_max_lon);

    // Calculate step to skip grid points if they would be too close together
    let pixels_per_source_lon = (width as f64 / render_lon_range) * source_lon_step;
    let pixels_per_source_lat = (height as f64 / render_lat_range) * source_lat_step;

    let step_x = ((min_pixel_spacing as f64 / pixels_per_source_lon).ceil() as usize).max(1);
    let step_y = ((min_pixel_spacing as f64 / pixels_per_source_lat).ceil() as usize).max(1);

    // Find source grid indices that fall within the render bbox
    let start_i = ((render_min_lon_src - source_min_lon) / source_lon_step).floor() as i64;
    let end_i = ((render_max_lon_src - source_min_lon) / source_lon_step).ceil() as i64;
    let start_j = ((render_min_lat - source_min_lat) / source_lat_step).floor() as i64;
    let end_j = ((render_max_lat - source_min_lat) / source_lat_step).ceil() as i64;

    // Align to step boundaries for consistent placement across tiles
    let start_i = (start_i / step_x as i64) * step_x as i64;
    let start_j = (start_j / step_y as i64) * step_y as i64;

    tracing::info!(
        source_dims = ?source_dims,
        source_lon_step = source_lon_step,
        source_lat_step = source_lat_step,
        step_x = step_x,
        step_y = step_y,
        start_i = start_i,
        end_i = end_i,
        start_j = start_j,
        end_j = end_j,
        "Rendering numbers with direct Zarr queries"
    );

    // Iterate over source grid points
    let mut j = start_j;
    while j <= end_j {
        if j < 0 || j >= source_height as i64 {
            j += step_y as i64;
            continue;
        }

        let mut i = start_i;
        while i <= end_i {
            if i < 0 || i >= source_width as i64 {
                i += step_x as i64;
                continue;
            }

            // Calculate geographic position of this grid point (in source coordinates)
            let geo_lon_src = source_min_lon + (i as f64 * source_lon_step);
            let geo_lat = source_min_lat + (j as f64 * source_lat_step);

            // Convert to display coordinates (-180 to 180)
            let geo_lon = from_source_lon(geo_lon_src);

            // Check if this position is in the visible area
            let text_buffer_lon = source_lon_step * 0.5;
            let text_buffer_lat = source_lat_step * 0.5;
            if geo_lon < vis_min_lon - text_buffer_lon
                || geo_lon > vis_max_lon + text_buffer_lon
                || geo_lat < vis_min_lat - text_buffer_lat
                || geo_lat > vis_max_lat + text_buffer_lat
            {
                i += step_x as i64;
                continue;
            }

            // Convert geographic to pixel coordinates
            let px = ((geo_lon - render_min_lon) / render_lon_range * width as f64) as i32;
            let py = ((render_max_lat - geo_lat) / render_lat_range * height as f64) as i32;

            // Skip if outside render area
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                i += step_x as i64;
                continue;
            }

            // Query the exact value from Zarr at this grid cell
            // Note: Zarr row is from north to south, so we need to flip the j index
            let row = (source_height - 1) as i64 - j;
            let value = match processor.read_grid_cell(i as usize, row as usize).await {
                Ok(Some(v)) => v,
                Ok(None) => {
                    i += step_x as i64;
                    continue;
                }
                Err(e) => {
                    tracing::debug!(error = %e, col = i, row = row, "Failed to read grid cell");
                    i += step_x as i64;
                    continue;
                }
            };

            // Apply unit transformation
            let display_value = unit_transform.apply(value);

            // Format the value
            let text = renderer::numbers::format_value(display_value);

            // Get color for this value (use transformed value for color mapping)
            let color = renderer::numbers::get_color_for_value(display_value, color_stops);

            // Calculate text dimensions for centering
            let char_width = (font_size * 0.6) as i32;
            let text_width = (text.len() as i32) * char_width;
            let text_height = font_size as i32;

            // Center the text on the grid point
            let centered_px = px - text_width / 2;
            let centered_py = py - text_height / 2;

            // Draw background
            let bg_color = Rgba([255, 255, 255, 220]);
            let padding = 2i32;
            for dy in -padding..(text_height + padding) {
                for dx in -padding..(text_width + padding) {
                    let bx = centered_px + dx;
                    let by = centered_py + dy;
                    if bx >= 0 && bx < width as i32 && by >= 0 && by < height as i32 {
                        img.put_pixel(bx as u32, by as u32, bg_color);
                    }
                }
            }

            // Draw the text
            draw_text_mut(
                &mut img,
                color,
                centered_px,
                centered_py,
                scale,
                &font,
                &text,
            );

            i += step_x as i64;
        }
        j += step_y as i64;
    }

    Ok(img)
}

/// Render numeric values at grid points with tile buffer for seamless edges.
/// This renders a 3x3 grid of tiles and crops the center to ensure numbers
/// at tile boundaries are not clipped.
///
/// # Arguments
/// - `catalog`: Catalog for finding datasets
/// - `metrics`: Metrics collector for performance tracking
/// - `grid_processor_factory`: Factory for Zarr-based grid access
/// - `model`: Weather model name (e.g., "gfs")
/// - `parameter`: Parameter name (e.g., "TMP")
/// - `tile_coord`: Tile coordinate for expanded rendering
/// - `width`: Output image width (single tile)
/// - `height`: Output image height (single tile)
/// - `bbox`: Bounding box [min_lon, min_lat, max_lon, max_lat] for the single tile
/// - `style_path`: Path to style configuration file (for color mapping)
/// - `forecast_hour`: Optional forecast hour
/// - `level`: Optional vertical level
/// - `use_mercator`: Use Web Mercator projection
/// - `requires_full_grid`: Force full grid read (for non-geographic projections)
///
/// # Returns
/// PNG image data as bytes
pub async fn render_numbers_tile_with_buffer(
    catalog: &Catalog,
    _metrics: &MetricsCollector,
    grid_processor_factory: &GridProcessorFactory,
    model: &str,
    parameter: &str,
    tile_coord: Option<wms_common::TileCoord>,
    width: u32,
    height: u32,
    bbox: [f32; 4],
    style_path: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    use_mercator: bool,
    requires_full_grid: bool,
) -> Result<Vec<u8>, String> {
    use wms_common::tile::{tile_bbox, TileBufferConfig};

    // Load style configuration for color mapping
    let style_json = std::fs::read_to_string(style_path)
        .map_err(|e| format!("Failed to read style file: {}", e))?;
    let style_config: StyleConfig = serde_json::from_str(&style_json)
        .map_err(|e| format!("Failed to parse style JSON: {}", e))?;

    // Get the first style definition (there should only be one in the file)
    let style_def = style_config
        .styles
        .values()
        .next()
        .ok_or_else(|| "No style definition found in style file".to_string())?;

    // Get dataset for this parameter, optionally at a specific level
    let entry = match (forecast_hour, level) {
        (Some(hour), Some(lev)) => catalog
            .find_by_forecast_hour_and_level(model, parameter, hour, lev)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| {
                format!(
                    "No data found for {}/{} at hour {} level {}",
                    model, parameter, hour, lev
                )
            })?,
        (Some(hour), None) => catalog
            .find_by_forecast_hour(model, parameter, hour)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{} at hour {}", model, parameter, hour))?,
        (None, Some(lev)) => catalog
            .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{} at level {}", model, parameter, lev))?,
        (None, None) => catalog
            .get_latest_run_earliest_forecast(model, parameter)
            .await
            .map_err(|e| format!("Catalog query failed: {}", e))?
            .ok_or_else(|| format!("No data found for {}/{}", model, parameter))?,
    };

    // Use pixel buffer approach for tile rendering (4x faster than 3x3 expansion)
    let (render_bbox, render_width, render_height, buffer_config) = if let Some(coord) = tile_coord
    {
        let buffer_config = TileBufferConfig::from_env();
        let tile_bounds = tile_bbox(&coord);
        let expanded_bbox = buffer_config.expanded_bbox(&tile_bounds);

        (
            [
                expanded_bbox.min_x as f32,
                expanded_bbox.min_y as f32,
                expanded_bbox.max_x as f32,
                expanded_bbox.max_y as f32,
            ],
            buffer_config.render_width() as usize,
            buffer_config.render_height() as usize,
            Some((coord, buffer_config)),
        )
    } else {
        (bbox, width as usize, height as usize, None)
    };

    info!(
        render_width = render_width,
        render_height = render_height,
        bbox_min_lon = render_bbox[0],
        bbox_max_lon = render_bbox[2],
        buffer_pixels = buffer_config
            .as_ref()
            .map(|(_, c)| c.buffer_pixels)
            .unwrap_or(0),
        "Rendering numbers tile"
    );

    // Load grid data from Zarr storage
    let grid_result = load_grid_data(
        grid_processor_factory,
        &entry,
        Some(render_bbox),
        Some((render_width as usize, render_height as usize)), // Output tile size for pyramid selection
        requires_full_grid,
    )
    .await?;

    let (grid_data, grid_width, grid_height) =
        (grid_result.data, grid_result.width, grid_result.height);

    // Get data bounds from catalog entry
    let data_bounds = [
        entry.bbox.min_x as f32,
        entry.bbox.min_y as f32,
        entry.bbox.max_x as f32,
        entry.bbox.max_y as f32,
    ];

    // Check if grid uses 0-360 longitude (like GFS)
    let grid_uses_360 = entry.bbox.min_x >= 0.0 && entry.bbox.max_x > 180.0;

    // Get full source grid dimensions from zarr_metadata if available
    // This is needed for placing numbers at exact grid point locations
    let source_grid_dims: Option<(usize, usize)> = if let Some(ref zarr_json) = entry.zarr_metadata
    {
        use grid_processor::ZarrMetadata;
        ZarrMetadata::from_json(zarr_json)
            .ok()
            .map(|meta| meta.shape)
    } else {
        None
    };

    // Resample data to the render bbox
    let resampled_data = resample_grid_for_bbox(
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

    // Convert flat array to 2D grid for numbers renderer
    let grid_2d: Vec<Vec<f32>> = (0..render_height)
        .map(|y| {
            (0..render_width)
                .map(|x| resampled_data[y * render_width + x])
                .collect()
        })
        .collect();

    // Determine unit transform from style config
    // The transform field specifies how to convert native units (e.g., Pa) to display units (e.g., hPa)
    let unit_transform = style_def
        .transform
        .as_ref()
        .map(|t| {
            match t.transform_type.to_lowercase().as_str() {
                "k_to_c" | "kelvin_to_celsius" => numbers::UnitTransform::Subtract(273.15),
                "pa_to_hpa" => numbers::UnitTransform::Divide(100.0),
                "m_to_km" => numbers::UnitTransform::Divide(1000.0),
                "mps_to_knots" => numbers::UnitTransform::Divide(0.514444), // multiply by ~1.94
                "linear" => {
                    let scale = t.scale.unwrap_or(1.0);
                    let offset = t.offset.unwrap_or(0.0);
                    numbers::UnitTransform::Linear { scale, offset }
                }
                _ => numbers::UnitTransform::None,
            }
        })
        .unwrap_or_else(|| {
            // Fallback: detect from parameter name for backwards compatibility
            if parameter.contains("TMP") || parameter.contains("TEMP") {
                numbers::UnitTransform::Subtract(273.15)
            } else if parameter.contains("PRES")
                || parameter.contains("PRESS")
                || parameter.contains("PRMSL")
            {
                numbers::UnitTransform::Divide(100.0)
            } else {
                numbers::UnitTransform::None
            }
        });

    // Legacy format for backwards compatibility (None means use unit_transform)
    let unit_conversion: Option<f32> = None;

    // Render numbers at exact source grid point locations
    // For accurate display, we query the Zarr data directly at each grid point
    let final_pixels = if let Some((coord, buf_config)) = buffer_config {
        // Calculate center tile bbox for visibility filtering
        let center_bbox = wms_common::tile::tile_bbox(&coord);
        let visible_bbox = [
            center_bbox.min_x,
            center_bbox.min_y,
            center_bbox.max_x,
            center_bbox.max_y,
        ];

        // Keep source bbox in its original format (0-360 for GFS)
        let source_bbox = [
            data_bounds[0] as f64,
            data_bounds[1] as f64,
            data_bounds[2] as f64,
            data_bounds[3] as f64,
        ];

        // Use full source grid dimensions if available (from zarr_metadata)
        let full_source_dims = source_grid_dims.unwrap_or((grid_width, grid_height));

        // Try to use direct Zarr queries for exact values
        let img = if entry.zarr_metadata.is_some() {
            // Render using direct Zarr queries
            match render_numbers_direct_zarr(
                grid_processor_factory,
                &entry,
                render_width as u32,
                render_height as u32,
                [
                    render_bbox[0] as f64,
                    render_bbox[1] as f64,
                    render_bbox[2] as f64,
                    render_bbox[3] as f64,
                ],
                Some(visible_bbox),
                &style_def.stops,
                unit_transform,
                40, // min_pixel_spacing
            )
            .await
            {
                Ok(img) => img,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to render with direct Zarr queries, falling back to resampled data");
                    // Fall back to resampled data
                    let grid_point_config = numbers::GridPointNumbersConfig {
                        font_size: 12.0,
                        color_stops: style_def.stops.clone(),
                        unit_conversion,
                        unit_transform,
                        render_bbox: [
                            render_bbox[0] as f64,
                            render_bbox[1] as f64,
                            render_bbox[2] as f64,
                            render_bbox[3] as f64,
                        ],
                        source_bbox,
                        source_dims: full_source_dims,
                        visible_bbox: Some(visible_bbox),
                        min_pixel_spacing: 40,
                        source_uses_360: grid_uses_360,
                    };
                    numbers::render_numbers_at_grid_points(
                        &grid_2d,
                        render_width as u32,
                        render_height as u32,
                        &grid_point_config,
                    )
                }
            }
        } else {
            // No Zarr access, use resampled data
            let grid_point_config = numbers::GridPointNumbersConfig {
                font_size: 12.0,
                color_stops: style_def.stops.clone(),
                unit_conversion,
                unit_transform,
                render_bbox: [
                    render_bbox[0] as f64,
                    render_bbox[1] as f64,
                    render_bbox[2] as f64,
                    render_bbox[3] as f64,
                ],
                source_bbox,
                source_dims: full_source_dims,
                visible_bbox: Some(visible_bbox),
                min_pixel_spacing: 40,
                source_uses_360: grid_uses_360,
            };
            numbers::render_numbers_at_grid_points(
                &grid_2d,
                render_width as u32,
                render_height as u32,
                &grid_point_config,
            )
        };

        let expanded_pixels = img.into_raw();

        // Crop to center tile using pixel buffer
        buf_config.crop_to_tile(&expanded_pixels)
    } else {
        // No tile coordinate - fall back to regular rendering
        let numbers_config = NumbersConfig {
            spacing: 50,
            font_size: 12.0,
            color_stops: style_def.stops.clone(),
            unit_conversion,
        };
        let img = numbers::render_numbers(
            &grid_2d,
            render_width as u32,
            render_height as u32,
            &numbers_config,
        );
        img.into_raw()
    };

    // Encode as PNG
    renderer::png::create_png(&final_pixels, width as usize, height as usize)
        .map_err(|e| format!("PNG encoding failed: {}", e))
}
