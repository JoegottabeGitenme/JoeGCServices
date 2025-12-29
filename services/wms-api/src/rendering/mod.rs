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
use resampling::resample_grid_for_bbox_with_proj;
use std::time::Instant;
use storage::Catalog;
use tracing::info;

// Re-export functions for internal use
pub(crate) use colorscales::render_with_style_file_indexed;

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
