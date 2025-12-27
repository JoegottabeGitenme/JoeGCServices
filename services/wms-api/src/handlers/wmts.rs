//! WMTS (Web Map Tile Service) request handlers.
//!
//! This module handles WMTS 1.0.0 protocol requests:
//! - GetCapabilities: Returns service metadata and available layers
//! - GetTile: Returns rendered tile images
//!
//! Supports multiple access patterns:
//! - KVP (Key-Value Pair): Standard query parameter format
//! - RESTful: URL path-based format
//! - XYZ: Simplified tile URL format for web mapping libraries

use axum::{
    extract::{Extension, Path, Query},
    http::{header, StatusCode},
    response::Response,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, instrument};

use storage::CacheKey;
use wms_common::{
    tile::{web_mercator_tile_matrix_set, wgs84_tile_to_latlon_bounds},
    BoundingBox, CrsCode, TileCoord,
};

use super::common::{
    convert_png_to_jpeg, convert_png_to_webp, get_wmts_styles_xml_from_file, wmts_exception,
    DimensionParams, WmtsDimensionParams,
};
use crate::layer_config::LayerConfigRegistry;
use crate::model_config::ModelDimensionRegistry;
use crate::state::AppState;
use storage::ParameterAvailability;

// ============================================================================
// WMTS Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct WmtsKvpParams {
    #[serde(rename = "SERVICE")]
    pub service: Option<String>,
    #[serde(rename = "REQUEST")]
    pub request: Option<String>,
    #[serde(rename = "LAYER")]
    pub layer: Option<String>,
    #[serde(rename = "STYLE")]
    pub style: Option<String>,
    #[serde(rename = "TILEMATRIXSET")]
    pub tile_matrix_set: Option<String>,
    #[serde(rename = "TILEMATRIX")]
    pub tile_matrix: Option<String>,
    #[serde(rename = "TILEROW")]
    pub tile_row: Option<u32>,
    #[serde(rename = "TILECOL")]
    pub tile_col: Option<u32>,
    #[serde(rename = "FORMAT")]
    pub format: Option<String>,
    #[serde(rename = "TIME")]
    pub time: Option<String>,
    #[serde(rename = "RUN")]
    pub run: Option<String>,
    #[serde(rename = "FORECAST")]
    pub forecast: Option<String>,
    #[serde(rename = "ELEVATION")]
    pub elevation: Option<String>,
    // GetFeatureInfo parameters
    #[serde(rename = "I")]
    pub i: Option<u32>,
    #[serde(rename = "J")]
    pub j: Option<u32>,
    #[serde(rename = "INFOFORMAT")]
    pub info_format: Option<String>,
}

// ============================================================================
// WMTS Handler Entry Points
// ============================================================================

/// WMTS KVP (Key-Value Pair) handler
#[instrument(skip(state))]
pub async fn wmts_kvp_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<WmtsKvpParams>,
) -> Response {
    if params.service.as_deref() != Some("WMTS") {
        return wmts_exception(
            "InvalidParameterValue",
            "SERVICE must be WMTS",
            StatusCode::BAD_REQUEST,
        );
    }

    match params.request.as_deref() {
        Some("GetCapabilities") => wmts_get_capabilities(state).await,
        Some("GetTile") => {
            // Validate FORMAT parameter
            let format = params.format.as_deref().unwrap_or("image/png");
            if format != "image/png" && format != "image/jpeg" && format != "image/webp" {
                return wmts_exception(
                    "InvalidParameterValue",
                    &format!("FORMAT '{}' is not supported. Supported formats: image/png, image/jpeg, image/webp", format),
                    StatusCode::BAD_REQUEST,
                );
            }

            // Validate TILEMATRIXSET parameter
            let tile_matrix_set = params
                .tile_matrix_set
                .as_deref()
                .unwrap_or("WebMercatorQuad");
            if tile_matrix_set != "WebMercatorQuad" && tile_matrix_set != "WorldCRS84Quad" {
                return wmts_exception(
                    "InvalidParameterValue",
                    &format!("TILEMATRIXSET '{}' is not supported. Supported: WebMercatorQuad, WorldCRS84Quad", tile_matrix_set),
                    StatusCode::BAD_REQUEST,
                );
            }

            let layer = params.layer.clone().unwrap_or_default();
            let style = params
                .style
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let tile_matrix = params.tile_matrix.clone().unwrap_or_default();
            let tile_row = params.tile_row.unwrap_or(0);
            let tile_col = params.tile_col.unwrap_or(0);
            let z: u32 = tile_matrix.parse().unwrap_or(0);

            // Validate TILEMATRIX (zoom level) - both TileMatrixSets support 0-18
            if z > 18 {
                return wmts_exception(
                    "TileOutOfRange",
                    &format!("TILEMATRIX '{}' is out of range. Valid range: 0-18", z),
                    StatusCode::BAD_REQUEST,
                );
            }

            // Validate TILEROW and TILECOL bounds for the zoom level
            // WorldCRS84Quad has 2:1 aspect ratio: cols = 2^(z+1), rows = 2^z
            // WebMercatorQuad has 1:1 aspect ratio: cols = rows = 2^z
            let (max_cols, max_rows) = if tile_matrix_set == "WorldCRS84Quad" {
                (2u32.pow(z + 1), 2u32.pow(z))
            } else {
                (2u32.pow(z), 2u32.pow(z))
            };

            if tile_row >= max_rows {
                return wmts_exception(
                    "TileOutOfRange",
                    &format!(
                        "TILEROW '{}' is out of range for TILEMATRIX '{}'. Valid range: 0-{}",
                        tile_row,
                        z,
                        max_rows - 1
                    ),
                    StatusCode::BAD_REQUEST,
                );
            }
            if tile_col >= max_cols {
                return wmts_exception(
                    "TileOutOfRange",
                    &format!(
                        "TILECOL '{}' is out of range for TILEMATRIX '{}'. Valid range: 0-{}",
                        tile_col,
                        z,
                        max_cols - 1
                    ),
                    StatusCode::BAD_REQUEST,
                );
            }

            let dimensions = DimensionParams {
                time: params.time.clone(),
                run: params.run.clone(),
                forecast: params.forecast.clone(),
                elevation: params.elevation.clone(),
            };

            let model = layer.split('_').next().unwrap_or("");
            let (forecast_hour, observation_time, _) =
                dimensions.parse_for_layer(model, &state.model_dimensions);

            wmts_get_tile(
                state,
                &layer,
                &style,
                tile_matrix_set,
                z,
                tile_col,
                tile_row,
                forecast_hour,
                observation_time,
                dimensions.elevation.as_deref(),
                format,
            )
            .await
        }
        Some("GetFeatureInfo") => {
            // Validate required parameters
            let layer = match &params.layer {
                Some(l) => l.clone(),
                None => {
                    return wmts_exception(
                        "MissingParameterValue",
                        "LAYER is required",
                        StatusCode::BAD_REQUEST,
                    )
                }
            };
            let tile_matrix = match &params.tile_matrix {
                Some(tm) => tm.clone(),
                None => {
                    return wmts_exception(
                        "MissingParameterValue",
                        "TILEMATRIX is required",
                        StatusCode::BAD_REQUEST,
                    )
                }
            };
            let tile_row = match params.tile_row {
                Some(tr) => tr,
                None => {
                    return wmts_exception(
                        "MissingParameterValue",
                        "TILEROW is required",
                        StatusCode::BAD_REQUEST,
                    )
                }
            };
            let tile_col = match params.tile_col {
                Some(tc) => tc,
                None => {
                    return wmts_exception(
                        "MissingParameterValue",
                        "TILECOL is required",
                        StatusCode::BAD_REQUEST,
                    )
                }
            };
            let i = match params.i {
                Some(i) => i,
                None => {
                    return wmts_exception(
                        "MissingParameterValue",
                        "I is required",
                        StatusCode::BAD_REQUEST,
                    )
                }
            };
            let j = match params.j {
                Some(j) => j,
                None => {
                    return wmts_exception(
                        "MissingParameterValue",
                        "J is required",
                        StatusCode::BAD_REQUEST,
                    )
                }
            };

            let z: u32 = tile_matrix.parse().unwrap_or(0);
            let info_format = params.info_format.as_deref().unwrap_or("application/json");

            // Validate I and J are within tile bounds (0-255 for 256x256 tiles)
            if i >= 256 {
                return wmts_exception(
                    "PointIJOutOfRange",
                    &format!(
                        "I parameter value {} is out of range. Must be between 0 and 255.",
                        i
                    ),
                    StatusCode::BAD_REQUEST,
                );
            }
            if j >= 256 {
                return wmts_exception(
                    "PointIJOutOfRange",
                    &format!(
                        "J parameter value {} is out of range. Must be between 0 and 255.",
                        j
                    ),
                    StatusCode::BAD_REQUEST,
                );
            }

            let dimensions = DimensionParams {
                time: params.time.clone(),
                run: params.run.clone(),
                forecast: params.forecast.clone(),
                elevation: params.elevation.clone(),
            };

            let model = layer.split('_').next().unwrap_or("");
            let (forecast_hour, observation_time, _) =
                dimensions.parse_for_layer(model, &state.model_dimensions);

            wmts_get_feature_info(
                state,
                &layer,
                z,
                tile_col,
                tile_row,
                i,
                j,
                forecast_hour,
                observation_time,
                dimensions.elevation.as_deref(),
                info_format,
            )
            .await
        }
        _ => wmts_exception(
            "MissingParameterValue",
            "REQUEST is required",
            StatusCode::BAD_REQUEST,
        ),
    }
}

/// WMTS RESTful URL handler
#[instrument(skip(state))]
pub async fn wmts_rest_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(path): Path<String>,
    Query(params): Query<WmtsDimensionParams>,
) -> Response {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() < 6 {
        return wmts_exception(
            "InvalidParameterValue",
            "Invalid path",
            StatusCode::BAD_REQUEST,
        );
    }

    // URL format: {layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png
    let layer = parts[0];
    let style = parts[1];
    let tile_matrix_set = parts[2];
    let z: u32 = parts[3].parse().unwrap_or(0);
    let x: u32 = parts[4].parse().unwrap_or(0);
    let last = parts[5];
    let (y_str, _) = last.rsplit_once('.').unwrap_or((last, "png"));
    let y: u32 = y_str.parse().unwrap_or(0);

    // Validate TileMatrixSet
    if tile_matrix_set != "WebMercatorQuad" && tile_matrix_set != "WorldCRS84Quad" {
        return wmts_exception(
            "InvalidParameterValue",
            &format!(
                "TileMatrixSet '{}' is not supported. Supported: WebMercatorQuad, WorldCRS84Quad",
                tile_matrix_set
            ),
            StatusCode::BAD_REQUEST,
        );
    }

    let dimensions = DimensionParams {
        time: params.time.clone(),
        run: params.run.clone(),
        forecast: params.forecast.clone(),
        elevation: params.elevation.clone(),
    };

    let model = layer.split('_').next().unwrap_or("");
    let (forecast_hour, observation_time, _) =
        dimensions.parse_for_layer(model, &state.model_dimensions);

    // REST always uses PNG (format determined by file extension)
    wmts_get_tile(
        state,
        layer,
        style,
        tile_matrix_set,
        z,
        x,
        y,
        forecast_hour,
        observation_time,
        dimensions.elevation.as_deref(),
        "image/png",
    )
    .await
}

/// XYZ tile handler for Leaflet/OpenLayers
#[instrument(skip(state))]
pub async fn xyz_tile_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((layer, style, z, x, y)): Path<(String, String, u32, u32, String)>,
    Query(params): Query<WmtsDimensionParams>,
) -> Response {
    let (y_str, _) = y.rsplit_once('.').unwrap_or((&y, "png"));
    let y_val: u32 = y_str.parse().unwrap_or(0);

    let dimensions = DimensionParams {
        time: params.time.clone(),
        run: params.run.clone(),
        forecast: params.forecast.clone(),
        elevation: params.elevation.clone(),
    };

    let model = layer.split('_').next().unwrap_or("");
    let (forecast_hour, observation_time, _) =
        dimensions.parse_for_layer(model, &state.model_dimensions);

    // XYZ always uses WebMercatorQuad and PNG
    wmts_get_tile(
        state,
        &layer,
        &style,
        "WebMercatorQuad",
        z,
        x,
        y_val,
        forecast_hour,
        observation_time,
        dimensions.elevation.as_deref(),
        "image/png",
    )
    .await
}

// ============================================================================
// GetCapabilities
// ============================================================================

async fn wmts_get_capabilities(state: Arc<AppState>) -> Response {
    // Check cache first
    if let Some(cached_xml) = state.capabilities_cache.get_wmts().await {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/xml")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(cached_xml.into())
            .unwrap();
    }

    // Build capabilities from layer configs (config-driven approach)
    // Only include layers that have data in the catalog
    let layer_configs = state.layer_configs.read().await;

    // Collect availability data for each configured layer
    let mut param_availability: HashMap<String, ParameterAvailability> = HashMap::new();

    for model_id in layer_configs.models() {
        if let Some(model_config) = layer_configs.get_model(model_id) {
            for layer in &model_config.layers {
                // Skip composite layers - they're handled separately
                if layer.composite {
                    continue;
                }

                // Check if data exists for this layer
                if let Ok(Some(availability)) = state
                    .catalog
                    .get_parameter_availability(model_id, &layer.parameter)
                    .await
                {
                    let key = format!("{}_{}", model_id, layer.parameter);
                    param_availability.insert(key, availability);
                }
            }
        }
    }

    let xml = build_wmts_capabilities_xml_v2(
        &layer_configs,
        &param_availability,
        &state.model_dimensions,
    );

    // Cache the result
    state.capabilities_cache.set_wmts(xml.clone()).await;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(xml.into())
        .unwrap()
}

// ============================================================================
// GetTile
// ============================================================================

async fn wmts_get_tile(
    state: Arc<AppState>,
    layer: &str,
    style: &str,
    tile_matrix_set: &str,
    z: u32,
    x: u32,
    y: u32,
    forecast_hour: Option<u32>,
    observation_time: Option<chrono::DateTime<chrono::Utc>>,
    elevation: Option<&str>,
    format: &str,
) -> Response {
    use crate::metrics::Timer;

    state.metrics.record_wmts_request();
    let timer = Timer::start();

    // Parse layer
    let parts: Vec<&str> = layer.split('_').collect();
    let (model, parameter) = if parts.len() >= 2 {
        (parts[0], parts[1..].join("_").to_uppercase())
    } else {
        (layer, "".to_string())
    };

    // Get effective elevation
    let effective_elevation: Option<String> = match elevation {
        Some(elev) => Some(elev.to_string()),
        None => {
            let configs = state.layer_configs.read().await;
            configs
                .get_layer_by_param(model, &parameter)
                .and_then(|l| l.default_level())
                .map(|s| s.to_string())
        }
    };
    let elevation = effective_elevation.as_deref();

    info!(layer = %layer, style = %style, tile_matrix_set = %tile_matrix_set, z = z, x = x, y = y, forecast_hour = ?forecast_hour, elevation = ?elevation, "GetTile request");

    // Build cache key - include TileMatrixSet to avoid cache collisions
    let time_key = forecast_hour
        .map(|h| format!("t{}", h))
        .or_else(|| observation_time.map(|t| format!("obs{}", t.timestamp())));
    let elevation_key = elevation.map(|e| e.replace(' ', "_"));
    let dimension_suffix = match (&time_key, &elevation_key) {
        (Some(t), Some(e)) => Some(format!("{}_{}", t, e)),
        (Some(t), None) => Some(t.clone()),
        (None, Some(e)) => Some(e.clone()),
        (None, None) => None,
    };

    // Use appropriate CRS code based on TileMatrixSet
    let crs_code = if tile_matrix_set == "WorldCRS84Quad" {
        CrsCode::Epsg4326
    } else {
        CrsCode::Epsg3857
    };

    let cache_key = CacheKey::new(
        layer,
        style,
        crs_code,
        BoundingBox::new(x as f64, y as f64, z as f64, 0.0),
        256,
        256,
        dimension_suffix.clone(),
        "png",
    );

    // Cache key includes TileMatrixSet to avoid collisions between different projections
    let cache_key_str = format!(
        "{}:{}:{}:{}_{}_{}:{}",
        tile_matrix_set,
        layer,
        style,
        z,
        x,
        y,
        dimension_suffix.as_deref().unwrap_or("current")
    );

    // Get tile bounds based on TileMatrixSet
    let coord = TileCoord::new(z, x, y);

    let latlon_bbox = if tile_matrix_set == "WorldCRS84Quad" {
        // WorldCRS84Quad uses linear lat/lon mapping
        wgs84_tile_to_latlon_bounds(&coord)
    } else {
        // WebMercatorQuad uses Mercator projection
        let tms = web_mercator_tile_matrix_set();
        let _bbox = match tms.tile_bbox(&coord) {
            Some(bbox) => bbox,
            None => {
                return wmts_exception("TileOutOfRange", "Invalid tile", StatusCode::BAD_REQUEST)
            }
        };
        wms_common::tile::tile_to_latlon_bounds(&coord)
    };
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];

    // Check L1 cache
    if state.optimization_config.l1_cache_enabled {
        if let Some(tile_data) = state.tile_memory_cache.get(&cache_key_str).await {
            state.metrics.record_l1_cache_hit();
            state
                .metrics
                .record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::L1Hit);

            // Convert to requested format
            let (output_data, content_type) = match format {
                "image/jpeg" => match convert_png_to_jpeg(&tile_data) {
                    Ok(jpeg_data) => (jpeg_data, "image/jpeg"),
                    Err(_) => (tile_data.to_vec(), "image/png"),
                },
                "image/webp" => match convert_png_to_webp(&tile_data) {
                    Ok(webp_data) => (webp_data, "image/webp"),
                    Err(_) => (tile_data.to_vec(), "image/png"),
                },
                _ => (tile_data.to_vec(), "image/png"),
            };

            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "max-age=300")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "L1-HIT")
                .body(output_data.into())
                .unwrap();
        }
        state.metrics.record_l1_cache_miss();
    }

    // Check L2 cache
    {
        let mut cache = state.cache.lock().await;
        if let Ok(Some(cached_data)) = cache.get(&cache_key).await {
            state.metrics.record_cache_hit().await;
            state
                .metrics
                .record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::L2Hit);

            if state.optimization_config.l1_cache_enabled {
                let data_bytes = bytes::Bytes::from(cached_data.to_vec());
                state
                    .tile_memory_cache
                    .set(&cache_key_str, data_bytes.clone(), None)
                    .await;
            }

            // Convert to requested format
            let (output_data, content_type) = match format {
                "image/jpeg" => match convert_png_to_jpeg(&cached_data) {
                    Ok(jpeg_data) => (jpeg_data, "image/jpeg"),
                    Err(_) => (cached_data.to_vec(), "image/png"),
                },
                "image/webp" => match convert_png_to_webp(&cached_data) {
                    Ok(webp_data) => (webp_data, "image/webp"),
                    Err(_) => (cached_data.to_vec(), "image/png"),
                },
                _ => (cached_data.to_vec(), "image/png"),
            };

            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "max-age=3600")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "L2-HIT")
                .body(output_data.into())
                .unwrap();
        }
        state.metrics.record_cache_miss().await;
    }

    state
        .metrics
        .record_tile_request_location(&bbox_array, crate::metrics::TileCacheStatus::Miss);

    if parameter.is_empty() {
        return wmts_exception(
            "InvalidParameterValue",
            "Invalid layer format",
            StatusCode::BAD_REQUEST,
        );
    }

    // Validate layer exists in configuration
    {
        let configs = state.layer_configs.read().await;
        if configs.get_layer_by_param(model, &parameter).is_none() {
            // Check if it's a wind barbs layer
            if parameter != "WIND_BARBS" {
                return wmts_exception(
                    "TileNotDefined",
                    &format!("Layer '{}' is not defined", layer),
                    StatusCode::BAD_REQUEST,
                );
            }
        }
    }

    // Check if model requires full grid reads (non-geographic projection)
    let requires_full_grid = state.model_dimensions.requires_full_grid(model);

    // Render the tile
    let result = if parameter == "WIND_BARBS" {
        // Get wind barbs style file
        let wind_style_file = state
            .layer_configs
            .read()
            .await
            .get_style_file_for_parameter(model, "WIND_BARBS");
        crate::rendering::render_wind_barbs_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            Some(coord),
            256,
            256,
            bbox_array,
            forecast_hour,
            elevation,
            Some(&wind_style_file),
            None, // Use default style
        )
        .await
    } else if style == "isolines" {
        if state.model_dimensions.is_observation(model) {
            return wmts_exception(
                "StyleNotDefined",
                "Isolines not supported for observation layers",
                StatusCode::BAD_REQUEST,
            );
        }
        let style_file = state
            .layer_configs
            .read()
            .await
            .get_style_file_for_parameter(model, &parameter);
        crate::rendering::render_isolines_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            &parameter,
            Some(coord),
            256,
            256,
            bbox_array,
            &style_file,
            "isolines",
            forecast_hour,
            elevation,
            true,
        )
        .await
    } else if style == "numbers" {
        let style_file = state
            .layer_configs
            .read()
            .await
            .get_style_file_for_parameter(model, &parameter);
        crate::rendering::render_numbers_tile_with_buffer(
            &state.catalog,
            &state.metrics,
            &state.grid_processor_factory,
            model,
            &parameter,
            Some(coord),
            256,
            256,
            bbox_array,
            &style_file,
            forecast_hour,
            elevation,
            true,
            requires_full_grid,
        )
        .await
    } else {
        let style_file = state
            .layer_configs
            .read()
            .await
            .get_style_file_for_parameter(model, &parameter);
        crate::rendering::render_weather_data(
            &state.catalog,
            &state.metrics,
            model,
            &parameter,
            forecast_hour,
            observation_time,
            elevation,
            256,
            256,
            Some(bbox_array),
            &style_file,
            Some(style),
            true,
            &state.grid_processor_factory,
            requires_full_grid,
        )
        .await
    };

    match result {
        Ok(png_data) => {
            let layer_type = crate::metrics::LayerType::from_layer_and_style(layer, style);
            state
                .metrics
                .record_render_with_type(timer.elapsed_us(), true, layer_type)
                .await;

            // Convert to requested format
            let (output_data, content_type) = match format {
                "image/jpeg" => {
                    match convert_png_to_jpeg(&png_data) {
                        Ok(jpeg_data) => (jpeg_data, "image/jpeg"),
                        Err(e) => {
                            error!(error = %e, "Failed to convert PNG to JPEG");
                            (png_data.clone(), "image/png") // Fallback to PNG
                        }
                    }
                }
                "image/webp" => {
                    match convert_png_to_webp(&png_data) {
                        Ok(webp_data) => (webp_data, "image/webp"),
                        Err(e) => {
                            error!(error = %e, "Failed to convert PNG to WebP");
                            (png_data.clone(), "image/png") // Fallback to PNG
                        }
                    }
                }
                _ => (png_data.clone(), "image/png"),
            };

            // Cache the result (always cache as PNG for simplicity)
            if state.optimization_config.l1_cache_enabled {
                let png_bytes = bytes::Bytes::from(png_data.clone());
                state
                    .tile_memory_cache
                    .set(&cache_key_str, png_bytes, None)
                    .await;
            }

            let cache_data = png_data.clone();
            let state_clone = state.clone();
            let cache_key_clone = cache_key.clone();
            tokio::spawn(async move {
                let mut cache = state_clone.cache.lock().await;
                let _ = cache.set(&cache_key_clone, &cache_data, None).await;
            });

            // Prefetch neighbors
            if state.optimization_config.prefetch_enabled
                && z >= state.optimization_config.prefetch_min_zoom
                && z <= state.optimization_config.prefetch_max_zoom
            {
                spawn_tile_prefetch(
                    state.clone(),
                    layer.to_string(),
                    style.to_string(),
                    coord,
                    state.prefetch_rings,
                );
            }

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "max-age=3600")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header("X-Cache", "MISS")
                .body(output_data.into())
                .unwrap()
        }
        Err(e) => {
            state.metrics.record_render(timer.elapsed_us(), false).await;
            error!(layer = %layer, error = %e, "WMTS tile rendering failed");
            wmts_exception(
                "NoApplicableCode",
                &format!("Rendering failed: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    }
}

// ============================================================================
// GetFeatureInfo
// ============================================================================

async fn wmts_get_feature_info(
    state: Arc<AppState>,
    layer: &str,
    z: u32,
    x: u32,
    y: u32,
    i: u32,
    j: u32,
    forecast_hour: Option<u32>,
    observation_time: Option<chrono::DateTime<chrono::Utc>>,
    elevation: Option<&str>,
    info_format: &str,
) -> Response {
    use wms_protocol::{FeatureInfoResponse, InfoFormat};

    info!(layer = %layer, z = z, x = x, y = y, i = i, j = j, forecast_hour = ?forecast_hour, elevation = ?elevation, "WMTS GetFeatureInfo request");

    // Parse info format
    let format = match InfoFormat::from_mime(info_format) {
        Some(f) => f,
        None => {
            return wmts_exception(
                "InvalidParameterValue",
                &format!("INFOFORMAT '{}' is not supported. Supported formats: application/json, text/html, text/xml, text/plain", info_format),
                StatusCode::BAD_REQUEST,
            );
        }
    };

    // Calculate the geographic bbox for the tile
    let coord = TileCoord::new(z, x, y);
    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);

    // Convert tile pixel coordinates to lat/lon
    // Tile is 256x256 pixels
    let tile_width = latlon_bbox.max_x - latlon_bbox.min_x;
    let tile_height = latlon_bbox.max_y - latlon_bbox.min_y;

    // Note: In web tiles, J=0 is at the top, so we need to flip the y coordinate
    let pixel_lon = latlon_bbox.min_x + (i as f64 / 256.0) * tile_width;
    let pixel_lat = latlon_bbox.max_y - (j as f64 / 256.0) * tile_height; // Flip Y

    // Create a small bbox around the point (single pixel)
    let pixel_width = tile_width / 256.0;
    let pixel_height = tile_height / 256.0;
    let bbox_array = [
        pixel_lon - pixel_width / 2.0,  // min_lon
        pixel_lat - pixel_height / 2.0, // min_lat
        pixel_lon + pixel_width / 2.0,  // max_lon
        pixel_lat + pixel_height / 2.0, // max_lat
    ];

    // Parse layer
    let parts: Vec<&str> = layer.split('_').collect();
    let (model, parameter) = if parts.len() >= 2 {
        (parts[0], parts[1..].join("_").to_uppercase())
    } else {
        return wmts_exception(
            "InvalidParameterValue",
            "Invalid layer format",
            StatusCode::BAD_REQUEST,
        );
    };

    // Get effective elevation
    let effective_elevation: Option<String> = match elevation {
        Some(elev) => Some(elev.to_string()),
        None => {
            let configs = state.layer_configs.read().await;
            configs
                .get_layer_by_param(model, &parameter)
                .and_then(|l| l.default_level())
                .map(|s| s.to_string())
        }
    };

    // Query the point value
    let features = match crate::rendering::query_point_value(
        &state.catalog,
        &state.metrics,
        &state.grid_processor_factory,
        layer,
        bbox_array,
        256,
        256,
        i,
        j,
        "EPSG:4326",
        forecast_hour,
        observation_time,
        effective_elevation.as_deref(),
    )
    .await
    {
        Ok(features) => features,
        Err(e) => {
            error!(layer = %layer, error = %e, "WMTS GetFeatureInfo query failed");
            return wmts_exception(
                "NoApplicableCode",
                &format!("Query failed: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
        }
    };

    let response = FeatureInfoResponse::new(features);

    // Format response based on INFOFORMAT
    let (body, content_type) = match format {
        InfoFormat::Json => match response.to_json() {
            Ok(json) => (json, "application/json"),
            Err(e) => {
                return wmts_exception(
                    "NoApplicableCode",
                    &format!("JSON encoding failed: {}", e),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }
        },
        InfoFormat::Html => (response.to_html(), "text/html"),
        InfoFormat::Xml => (response.to_xml(), "text/xml"),
        InfoFormat::Text => (response.to_text(), "text/plain"),
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body.into())
        .unwrap()
}

// ============================================================================
// Tile Prefetching
// ============================================================================

fn get_tiles_in_rings(center: &TileCoord, rings: u32) -> Vec<TileCoord> {
    let z = center.z;
    let max_tile = 2u32.pow(z) - 1;
    let cx = center.x as i32;
    let cy = center.y as i32;

    let capacity = if rings == 0 {
        0
    } else {
        (rings * (rings + 1) * 4) as usize
    };
    let mut tiles = Vec::with_capacity(capacity);

    for ring in 1..=rings {
        let r = ring as i32;

        for dx in -r..=r {
            let x = cx + dx;
            let y = cy - r;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }

        for dy in -r + 1..=r {
            let x = cx + r;
            let y = cy + dy;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }

        for dx in (-r + 1..=r).rev() {
            let x = cx + dx;
            let y = cy + r;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }

        for dy in (-r + 1..r).rev() {
            let x = cx - r;
            let y = cy + dy;
            if x >= 0 && x <= max_tile as i32 && y >= 0 && y <= max_tile as i32 {
                tiles.push(TileCoord::new(z, x as u32, y as u32));
            }
        }
    }

    tiles
}

fn spawn_tile_prefetch(
    state: Arc<AppState>,
    layer: String,
    style: String,
    center: TileCoord,
    rings: u32,
) {
    let neighbors = get_tiles_in_rings(&center, rings);

    debug!(layer = %layer, z = center.z, x = center.x, y = center.y, tiles = neighbors.len(), "Spawning prefetch");

    for neighbor in neighbors {
        let state = state.clone();
        let layer = layer.clone();
        let style = style.clone();

        tokio::spawn(async move {
            prefetch_single_tile(state, &layer, &style, neighbor).await;
        });
    }
}

async fn prefetch_single_tile(state: Arc<AppState>, layer: &str, style: &str, coord: TileCoord) {
    let cache_key = CacheKey::new(
        layer,
        style,
        CrsCode::Epsg3857,
        BoundingBox::new(coord.x as f64, coord.y as f64, coord.z as f64, 0.0),
        256,
        256,
        None,
        "png",
    );

    {
        let mut cache = state.cache.lock().await;
        if let Ok(Some(_)) = cache.get(&cache_key).await {
            return;
        }
    }

    let parts: Vec<&str> = layer.split('_').collect();
    let (model, parameter) = if parts.len() >= 2 {
        (parts[0], parts[1..].join("_").to_uppercase())
    } else {
        return;
    };

    let latlon_bbox = wms_common::tile::tile_to_latlon_bounds(&coord);
    let bbox_array = [
        latlon_bbox.min_x as f32,
        latlon_bbox.min_y as f32,
        latlon_bbox.max_x as f32,
        latlon_bbox.max_y as f32,
    ];

    let style_file = state
        .layer_configs
        .read()
        .await
        .get_style_file_for_parameter(model, &parameter);

    // Check if model requires full grid reads (non-geographic projection)
    let requires_full_grid = state.model_dimensions.requires_full_grid(model);

    let result = if parameter == "WIND_BARBS" {
        crate::rendering::render_wind_barbs_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            Some(coord),
            256,
            256,
            bbox_array,
            None,
            None,
            Some(&style_file), // style_file is already resolved above
            None,              // Use default style
        )
        .await
    } else if style == "isolines" {
        crate::rendering::render_isolines_tile_with_level(
            &state.catalog,
            &state.grid_processor_factory,
            model,
            &parameter,
            Some(coord),
            256,
            256,
            bbox_array,
            &style_file,
            "isolines",
            None,
            None,
            true,
        )
        .await
    } else if style == "numbers" {
        crate::rendering::render_numbers_tile_with_buffer(
            &state.catalog,
            &state.metrics,
            &state.grid_processor_factory,
            model,
            &parameter,
            Some(coord),
            256,
            256,
            bbox_array,
            &style_file,
            None,
            None,
            true,
            requires_full_grid,
        )
        .await
    } else {
        crate::rendering::render_weather_data(
            &state.catalog,
            &state.metrics,
            model,
            &parameter,
            None,
            None,
            None,
            256,
            256,
            Some(bbox_array),
            &style_file,
            Some(style),
            true,
            &state.grid_processor_factory,
            requires_full_grid,
        )
        .await
    };

    if let Ok(png_data) = result {
        let mut cache = state.cache.lock().await;
        let _ = cache.set(&cache_key, &png_data, None).await;
    }
}

// ============================================================================
// WMTS Capabilities XML Builder (Legacy - kept for reference)
// ============================================================================

fn build_tile_matrices() -> String {
    (0..=18)
        .map(|z| {
            let n = 2u32.pow(z);
            let scale = 559082264.0287178 / (n as f64);
            let max_extent = 20037508.342789244;
            format!(
                r#"      <TileMatrix>
        <ows:Identifier>{}</ows:Identifier>
        <ScaleDenominator>{}</ScaleDenominator>
        <TopLeftCorner>{} {}</TopLeftCorner>
        <TileWidth>256</TileWidth>
        <TileHeight>256</TileHeight>
        <MatrixWidth>{}</MatrixWidth>
        <MatrixHeight>{}</MatrixHeight>
      </TileMatrix>"#,
                z, scale, -max_extent, max_extent, n, n
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build TileMatrix elements for WorldCRS84Quad TileMatrixSet.
/// WorldCRS84Quad uses 2:1 aspect ratio: matrix_width = 2^(z+1), matrix_height = 2^z
fn build_wgs84_tile_matrices() -> String {
    (0..=18)
        .map(|z| {
            let n_rows = 2u32.pow(z);
            let n_cols = 2u32.pow(z + 1); // 2:1 aspect ratio
                                          // Scale denominator based on degrees per pixel
                                          // At zoom 0: 180 degrees / 256 pixels = 0.703125 degrees/pixel
                                          // Standard pixel size is 0.00028m, 1 degree ≈ 111320m at equator
            let scale = 559082264.0287178 / (n_rows as f64);
            format!(
                r#"      <TileMatrix>
        <ows:Identifier>{}</ows:Identifier>
        <ScaleDenominator>{}</ScaleDenominator>
        <TopLeftCorner>-180.0 90.0</TopLeftCorner>
        <TileWidth>256</TileWidth>
        <TileHeight>256</TileHeight>
        <MatrixWidth>{}</MatrixWidth>
        <MatrixHeight>{}</MatrixHeight>
      </TileMatrix>"#,
                z, scale, n_cols, n_rows
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build WMTS capabilities XML from layer configs (config-driven approach).
/// Only includes layers that have data available in the catalog.
fn build_wmts_capabilities_xml_v2(
    layer_configs: &LayerConfigRegistry,
    param_availability: &HashMap<String, ParameterAvailability>,
    dimension_registry: &ModelDimensionRegistry,
) -> String {
    let mut all_layers: Vec<String> = Vec::new();

    for model_id in layer_configs.models() {
        let Some(model_config) = layer_configs.get_model(model_id) else {
            continue;
        };

        let is_observational = dimension_registry.is_observation(model_id);

        // Track availability for composite layer validation (e.g., WIND_BARBS)
        let mut ugrd_availability: Option<&ParameterAvailability> = None;
        let mut vgrd_availability: Option<&ParameterAvailability> = None;

        for layer in &model_config.layers {
            // Skip composite layers for now - handle them after regular layers
            if layer.composite {
                continue;
            }

            let key = format!("{}_{}", model_id, layer.parameter);
            let Some(availability) = param_availability.get(&key) else {
                // No data for this layer - skip it
                continue;
            };

            // Track UGRD/VGRD for wind barbs
            if layer.parameter == "UGRD" {
                ugrd_availability = Some(availability);
            } else if layer.parameter == "VGRD" {
                vgrd_availability = Some(availability);
            }

            let layer_id = format!("{}_{}", model_id, layer.parameter);
            let layer_title = format!("{} - {}", model_config.display_name, layer.title);

            // Build dimensions for this specific layer
            let time_dimensions = build_layer_time_dimensions_wmts(availability, is_observational);
            let elevation_dim = build_layer_elevation_dimension_wmts(&availability.levels);

            // Get styles from style file
            let style_path = layer_configs.get_style_path(layer);
            let styles = get_wmts_styles_xml_from_file(&style_path);

            // Build bounding box
            let (west, east, south, north) = normalize_bbox_wmts(&availability.bbox);

            all_layers.push(format!(
                r#"    <Layer>
      <ows:Title>{}</ows:Title>
      <ows:Identifier>{}</ows:Identifier>
      <ows:WGS84BoundingBox>
        <ows:LowerCorner>{} {}</ows:LowerCorner>
        <ows:UpperCorner>{} {}</ows:UpperCorner>
      </ows:WGS84BoundingBox>
{}
      <Format>image/png</Format>
      <Format>image/jpeg</Format>
      <Format>image/webp</Format>
      <TileMatrixSetLink>
        <TileMatrixSet>WebMercatorQuad</TileMatrixSet>
      </TileMatrixSetLink>
      <TileMatrixSetLink>
        <TileMatrixSet>WorldCRS84Quad</TileMatrixSet>
      </TileMatrixSetLink>
{}{}
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
      <ResourceURL format="image/webp" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.webp"/>
    </Layer>"#,
                layer_title, layer_id,
                west, south, east, north,
                styles,
                time_dimensions, elevation_dim,
                layer_id, layer_id
            ));
        }

        // Handle WIND_BARBS composite layer
        if let (Some(ugrd), Some(vgrd)) = (ugrd_availability, vgrd_availability) {
            // Find common levels between UGRD and VGRD
            let common_levels: Vec<String> = ugrd
                .levels
                .iter()
                .filter(|l| vgrd.levels.contains(l))
                .cloned()
                .collect();

            // Find common times between UGRD and VGRD
            let common_times: Vec<String> = ugrd
                .times
                .iter()
                .filter(|t| vgrd.times.contains(t))
                .cloned()
                .collect();

            // Find common forecast hours
            let common_forecast_hours: Vec<i32> = ugrd
                .forecast_hours
                .iter()
                .filter(|h| vgrd.forecast_hours.contains(h))
                .copied()
                .collect();

            // Only include WIND_BARBS if there's common data
            if !common_times.is_empty() && (!common_levels.is_empty() || is_observational) {
                let wind_availability = ParameterAvailability {
                    times: common_times,
                    forecast_hours: common_forecast_hours,
                    levels: common_levels,
                    bbox: ugrd.bbox.clone(),
                };

                let layer_id = format!("{}_WIND_BARBS", model_id);
                let time_dimensions =
                    build_layer_time_dimensions_wmts(&wind_availability, is_observational);
                let elevation_dim = build_layer_elevation_dimension_wmts(&wind_availability.levels);

                let (west, east, south, north) = normalize_bbox_wmts(&ugrd.bbox);

                all_layers.push(format!(
                    r#"    <Layer>
      <ows:Title>{} - Wind Barbs</ows:Title>
      <ows:Identifier>{}</ows:Identifier>
      <ows:WGS84BoundingBox>
        <ows:LowerCorner>{} {}</ows:LowerCorner>
        <ows:UpperCorner>{} {}</ows:UpperCorner>
      </ows:WGS84BoundingBox>
      <Style isDefault="true"><ows:Identifier>default</ows:Identifier><ows:Title>Default</ows:Title></Style>
      <Format>image/png</Format>
      <Format>image/jpeg</Format>
      <Format>image/webp</Format>
      <TileMatrixSetLink><TileMatrixSet>WebMercatorQuad</TileMatrixSet></TileMatrixSetLink>
      <TileMatrixSetLink><TileMatrixSet>WorldCRS84Quad</TileMatrixSet></TileMatrixSetLink>
{}{}
      <ResourceURL format="image/png" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
      <ResourceURL format="image/webp" resourceType="tile" template="http://localhost:8080/wmts/rest/{}/{{Style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.webp"/>
    </Layer>"#,
                    model_config.display_name, layer_id,
                    west, south, east, north,
                    time_dimensions, elevation_dim,
                    layer_id, layer_id
                ));
            }
        }
    }

    let layers = all_layers.join("\n");
    let webmercator_tile_matrices = build_tile_matrices();
    let wgs84_tile_matrices = build_wgs84_tile_matrices();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Capabilities xmlns="http://www.opengis.net/wmts/1.0"
    xmlns:ows="http://www.opengis.net/ows/1.1"
    xmlns:xlink="http://www.w3.org/1999/xlink"
    version="1.0.0">
  <ows:ServiceIdentification>
    <ows:Title>Weather WMTS Service</ows:Title>
    <ows:ServiceType>OGC WMTS</ows:ServiceType>
    <ows:ServiceTypeVersion>1.0.0</ows:ServiceTypeVersion>
  </ows:ServiceIdentification>
  <ows:OperationsMetadata>
    <ows:Operation name="GetCapabilities">
      <ows:DCP><ows:HTTP><ows:Get xlink:href="http://localhost:8080/wmts?"/></ows:HTTP></ows:DCP>
    </ows:Operation>
    <ows:Operation name="GetTile">
      <ows:DCP><ows:HTTP><ows:Get xlink:href="http://localhost:8080/wmts?"/></ows:HTTP></ows:DCP>
    </ows:Operation>
    <ows:Operation name="GetFeatureInfo">
      <ows:DCP><ows:HTTP><ows:Get xlink:href="http://localhost:8080/wmts?"/></ows:HTTP></ows:DCP>
    </ows:Operation>
  </ows:OperationsMetadata>
  <Contents>
{}
    <TileMatrixSet>
      <ows:Identifier>WebMercatorQuad</ows:Identifier>
      <ows:SupportedCRS>urn:ogc:def:crs:EPSG::3857</ows:SupportedCRS>
{}
    </TileMatrixSet>
    <TileMatrixSet>
      <ows:Identifier>WorldCRS84Quad</ows:Identifier>
      <ows:SupportedCRS>urn:ogc:def:crs:OGC:1.3:CRS84</ows:SupportedCRS>
{}
    </TileMatrixSet>
  </Contents>
</Capabilities>"#,
        layers, webmercator_tile_matrices, wgs84_tile_matrices
    )
}

/// Build time dimension XML for a WMTS layer based on actual data availability.
fn build_layer_time_dimensions_wmts(
    availability: &ParameterAvailability,
    is_observational: bool,
) -> String {
    if is_observational {
        let time_values = if availability.times.is_empty() {
            "        <Value>latest</Value>".to_string()
        } else {
            availability
                .times
                .iter()
                .map(|v| format!("        <Value>{}</Value>", v))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let default = availability
            .times
            .first()
            .map(|s| s.as_str())
            .unwrap_or("latest");
        format!(
            r#"      <Dimension>
        <ows:Identifier>time</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>"#,
            default, time_values
        )
    } else {
        let run_values = if availability.times.is_empty() {
            "        <Value>latest</Value>".to_string()
        } else {
            availability
                .times
                .iter()
                .map(|v| format!("        <Value>{}</Value>", v))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let run_default = availability
            .times
            .first()
            .map(|s| s.as_str())
            .unwrap_or("latest");

        let forecast_values = if availability.forecast_hours.is_empty() {
            "        <Value>0</Value>".to_string()
        } else {
            availability
                .forecast_hours
                .iter()
                .map(|v| format!("        <Value>{}</Value>", v))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let forecast_default = availability.forecast_hours.first().unwrap_or(&0);

        format!(
            r#"      <Dimension>
        <ows:Identifier>run</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>
      <Dimension>
        <ows:Identifier>forecast</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>"#,
            run_default, run_values, forecast_default, forecast_values
        )
    }
}

/// Build elevation dimension XML for WMTS layer.
fn build_layer_elevation_dimension_wmts(levels: &[String]) -> String {
    if levels.len() <= 1 {
        return String::new();
    }

    let mut sorted = levels.to_vec();
    sorted.sort_by(|a, b| {
        let av = a.replace(" mb", "").parse::<i32>().unwrap_or(9999);
        let bv = b.replace(" mb", "").parse::<i32>().unwrap_or(9999);
        bv.cmp(&av)
    });

    let values = sorted
        .iter()
        .map(|v| format!("        <Value>{}</Value>", v))
        .collect::<Vec<_>>()
        .join("\n");
    let default = sorted.first().map(|s| s.as_str()).unwrap_or("");

    format!(
        r#"
      <Dimension>
        <ows:Identifier>elevation</ows:Identifier>
        <Default>{}</Default>
{}
      </Dimension>"#,
        default, values
    )
}

/// Normalize bounding box longitude to -180/180 for WMTS.
fn normalize_bbox_wmts(bbox: &wms_common::BoundingBox) -> (f64, f64, f64, f64) {
    let (west, east) = if bbox.min_x == 0.0 && bbox.max_x == 360.0 {
        (-180.0, 180.0)
    } else {
        let w = if bbox.min_x > 180.0 {
            bbox.min_x - 360.0
        } else {
            bbox.min_x
        };
        let e = if bbox.max_x > 180.0 {
            bbox.max_x - 360.0
        } else {
            bbox.max_x
        };
        (w, e)
    };
    (west, east, bbox.min_y, bbox.max_y)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_tiles_in_rings_ring1() {
        let center = TileCoord::new(4, 8, 8);
        let tiles = get_tiles_in_rings(&center, 1);
        assert_eq!(tiles.len(), 8); // 8 neighboring tiles
    }

    #[test]
    fn test_get_tiles_in_rings_ring2() {
        let center = TileCoord::new(4, 8, 8);
        let tiles = get_tiles_in_rings(&center, 2);
        // Ring 1: 8, Ring 2: 16 = 24 total
        assert_eq!(tiles.len(), 24);
    }

    #[test]
    fn test_get_tiles_edge_handling() {
        // Tile at corner of world (z=2, x=0, y=0)
        let center = TileCoord::new(2, 0, 0);
        let tiles = get_tiles_in_rings(&center, 1);
        // Should only return valid tiles (not negative coords)
        assert!(tiles.len() < 8);
        for tile in &tiles {
            assert!(tile.x < 4); // max for z=2
            assert!(tile.y < 4);
        }
    }

    #[test]
    fn test_build_tile_matrices() {
        let matrices = build_tile_matrices();
        assert!(matrices.contains("<ows:Identifier>0</ows:Identifier>"));
        assert!(matrices.contains("<ows:Identifier>18</ows:Identifier>"));
        assert!(matrices.contains("<TileWidth>256</TileWidth>"));
    }
}
