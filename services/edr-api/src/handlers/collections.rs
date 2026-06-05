//! Collections endpoint handlers.

use axum::{
    extract::{Extension, Path},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use edr_protocol::{
    parameters::Parameter, responses::ExceptionResponse, Collection, CollectionList,
    CustomDimension, DataQueries, Extent, TemporalExtent, VerticalExtent,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::availability::{level_matching, ModelAvailability};
use crate::config::{CollectionDefinition, LevelValue, ModelEdrConfig};
use crate::content_negotiation::check_metadata_accept;
use crate::state::AppState;
use storage::Catalog;

/// Build extent from catalog data for a collection, filtering to only available levels.
///
/// For forecast models (not observations), this also populates custom dimensions
/// for `run` (available model runs) and `forecast-hour` (available forecast hours).
async fn build_extent_from_catalog_filtered(
    catalog: &Catalog,
    model_config: &ModelEdrConfig,
    collection_def: &CollectionDefinition,
    availability: &ModelAvailability,
) -> Extent {
    let model_name = &model_config.model;
    let is_forecast_model = matches!(model_config.data_type, crate::config::DataType::Forecast);

    // Get bounding box from catalog
    let bbox = catalog.get_model_bbox(model_name).await.ok();

    // Get temporal extent from catalog
    let temporal_extent = catalog
        .get_model_temporal_extent(model_name)
        .await
        .ok()
        .flatten();

    // Get all valid times for the values array
    let valid_times = catalog.get_model_valid_times(model_name).await.ok();

    // Build spatial extent
    let spatial_bbox = bbox
        .map(|b| [b.min_x, b.min_y, b.max_x, b.max_y])
        .unwrap_or([-180.0, -90.0, 180.0, 90.0]);

    let mut extent = Extent::with_spatial(spatial_bbox, None);

    // Add temporal extent if available
    if let Some((start, end)) = temporal_extent {
        let start_str = start.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let end_str = end.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut temporal = TemporalExtent::new(Some(start_str), Some(end_str));

        // Add values array with available times
        if let Some(times) = valid_times {
            let time_strings: Vec<String> = times
                .into_iter()
                .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .collect();
            temporal = temporal.with_values(time_strings);
        }

        extent = extent.with_temporal(temporal);
    }

    // Build vertical extent from ACTUALLY AVAILABLE levels only
    let mut level_values: Vec<f64> = Vec::new();
    let mut has_pressure_levels = false;
    let mut has_height_levels = false;

    // Determine VRS based on level_filter type
    let level_type = &collection_def.level_filter.level_type;
    if level_type == "isobaric" || level_type.contains("pressure") {
        has_pressure_levels = true;
    } else if level_type.contains("height") || level_type.contains("ground") {
        has_height_levels = true;
    }

    for param in &collection_def.parameters {
        // Get available levels for this parameter from availability cache
        let Some(available_levels) = availability.get_levels(&param.name) else {
            continue;
        };

        for level in &param.levels {
            // Convert config level to the string format used in catalog
            let level_str = match level {
                LevelValue::Numeric(v) => {
                    level_matching::format_level_string(*v, &collection_def.level_filter)
                }
                LevelValue::Named(s) => {
                    level_matching::format_named_level(s, &collection_def.level_filter)
                }
            };

            // Only include if actually available in catalog
            if available_levels.contains(&level_str) {
                // Extract numeric value for vertical extent
                if let Some(numeric) = level_matching::parse_level_numeric(&level_str) {
                    if !level_values.contains(&numeric) {
                        level_values.push(numeric);
                    }
                }
            }
        }
    }

    // Only add vertical extent if we have numeric levels
    if !level_values.is_empty() {
        // Sort levels
        level_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let vrs = if has_pressure_levels {
            Some("hPa".to_string())
        } else if has_height_levels {
            Some("m".to_string())
        } else {
            None
        };

        let vertical = VerticalExtent::with_levels(level_values, vrs);
        extent = extent.with_vertical(vertical);
    }

    // Add custom dimensions for forecast models (run and forecast-hour)
    if is_forecast_model {
        let mut custom_dims = Vec::new();

        // Get all available runs for this model
        if let Ok(runs) = catalog.get_all_model_runs(model_name).await {
            if !runs.is_empty() {
                let run_values: Vec<String> = runs
                    .iter()
                    .map(|r| r.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                    .collect();
                custom_dims.push(CustomDimension::for_runs(run_values));
            }
        }

        // Get forecast hours from the latest run
        if let Ok(Some(latest_run)) = catalog.get_latest_run_info(model_name).await {
            if !latest_run.forecast_hours.is_empty() {
                custom_dims.push(CustomDimension::for_forecast_hours(
                    latest_run.forecast_hours.clone(),
                ));
            }
        }

        if !custom_dims.is_empty() {
            extent = extent.with_custom(custom_dims);
        }
    }

    extent
}

/// Filter collection parameters to only those with available data.
/// Returns the filtered parameters and the count of how many were available.
fn filter_available_parameters(
    collection_def: &CollectionDefinition,
    availability: &ModelAvailability,
) -> Vec<String> {
    collection_def
        .parameters
        .iter()
        .filter(|p| availability.has_parameter(&p.name))
        .map(|p| p.name.clone())
        .collect()
}

/// Check if collection has multiple vertical levels available.
fn has_multiple_available_vertical_levels(
    collection_def: &CollectionDefinition,
    availability: &ModelAvailability,
) -> bool {
    use std::collections::HashSet;
    let mut unique_levels: HashSet<String> = HashSet::new();

    for param in &collection_def.parameters {
        let Some(available_levels) = availability.get_levels(&param.name) else {
            continue;
        };

        for level in &param.levels {
            let level_str = match level {
                LevelValue::Numeric(v) => {
                    level_matching::format_level_string(*v, &collection_def.level_filter)
                }
                LevelValue::Named(s) => {
                    level_matching::format_named_level(s, &collection_def.level_filter)
                }
            };

            if available_levels.contains(&level_str) {
                // Only count numeric levels for cube query support
                if level_matching::parse_level_numeric(&level_str).is_some() {
                    unique_levels.insert(level_str);
                }
            }
        }
    }

    unique_levels.len() > 1
}

/// Build a Collection for point observation data (METAR, TAF, etc.).
async fn build_point_observation_collection(
    state: &AppState,
    model_config: &ModelEdrConfig,
    collection_def: &CollectionDefinition,
    available_params: &[String],
    obs_count: i64,
) -> Collection {
    let source = model_config
        .observation_source
        .as_deref()
        .unwrap_or("metar");

    // Build description with observation count
    let description = format!(
        "{} ({} observations available)",
        collection_def.description, obs_count
    );

    let mut collection = Collection::new(&collection_def.id)
        .with_title(&collection_def.title)
        .with_description(&description);

    // Build links
    collection.build_links(&state.base_url);

    // Point observations support locations, radius, and area queries (not position, trajectory, etc.)
    // Start with a default and add the queries we support
    let queries = DataQueries::default()
        .with_locations(&state.base_url, &collection_def.id)
        .with_radius(&state.base_url, &collection_def.id)
        .with_area(&state.base_url, &collection_def.id);

    collection = collection.with_data_queries(queries);

    // Get temporal extent from observations
    let temporal_extent = state
        .observation_catalog
        .get_observation_time_range(source)
        .await
        .ok()
        .flatten();

    // CONUS bounding box for US airports
    let spatial_bbox = [-170.0, 15.0, -60.0, 72.0];
    let mut extent = Extent::with_spatial(spatial_bbox, None);

    if let Some((start, end)) = temporal_extent {
        let temporal = TemporalExtent::new(Some(start.to_rfc3339()), Some(end.to_rfc3339()));
        extent = extent.with_temporal(temporal);
    }

    collection = collection.with_extent(extent);

    // Add parameters
    let mut params = HashMap::new();
    for param_name in available_params {
        let param = Parameter::new(param_name, param_name);
        params.insert(param_name.clone(), param);
    }
    collection = collection.with_parameters(params);

    collection
}

/// Build a Collection for point forecast data (TAF).
async fn build_point_forecast_collection(
    state: &AppState,
    _model_config: &ModelEdrConfig,
    collection_def: &CollectionDefinition,
    available_params: &[String],
    taf_count: i64,
) -> Collection {
    // Build description with TAF count
    let description = format!(
        "{} ({} forecasts available)",
        collection_def.description, taf_count
    );

    let mut collection = Collection::new(&collection_def.id)
        .with_title(&collection_def.title)
        .with_description(&description);

    // Build links
    collection.build_links(&state.base_url);

    // Point forecasts support locations, radius, and area queries (same as observations)
    let queries = DataQueries::default()
        .with_locations(&state.base_url, &collection_def.id)
        .with_radius(&state.base_url, &collection_def.id)
        .with_area(&state.base_url, &collection_def.id);

    collection = collection.with_data_queries(queries);

    // Get temporal extent from TAFs (valid time range)
    let temporal_extent = state
        .observation_catalog
        .get_taf_time_range()
        .await
        .ok()
        .flatten();

    // CONUS + Alaska/Hawaii bounding box for US airports
    let spatial_bbox = [-170.0, 15.0, -60.0, 72.0];
    let mut extent = Extent::with_spatial(spatial_bbox, None);

    if let Some((start, end)) = temporal_extent {
        let temporal = TemporalExtent::new(Some(start.to_rfc3339()), Some(end.to_rfc3339()));
        extent = extent.with_temporal(temporal);
    }

    collection = collection.with_extent(extent);

    // Add parameters
    let mut params = HashMap::new();
    for param_name in available_params {
        let param = Parameter::new(param_name, param_name);
        params.insert(param_name.clone(), param);
    }
    collection = collection.with_parameters(params);

    collection
}

/// Build a Collection for storm-event feature data (hail/wind/tornado).
async fn build_storm_event_collection(
    state: &AppState,
    _model_config: &ModelEdrConfig,
    collection_def: &CollectionDefinition,
    available_params: &[String],
    event_count: i64,
) -> Collection {
    let description = format!(
        "{} ({} events available)",
        collection_def.description, event_count
    );

    let mut collection = Collection::new(&collection_def.id)
        .with_title(&collection_def.title)
        .with_description(&description);

    collection.build_links(&state.base_url);

    // Feature collections support radius, area, and locations queries.
    // (items + counties are served via dedicated routes; not standard EDR
    // dataQueries so they are documented in the collection description.)
    let queries = DataQueries::default()
        .with_locations(&state.base_url, &collection_def.id)
        .with_radius(&state.base_url, &collection_def.id)
        .with_area(&state.base_url, &collection_def.id);

    collection = collection.with_data_queries(queries);

    // Temporal extent from the events themselves.
    let temporal_extent = state
        .storm_event_catalog
        .get_event_time_range(&collection_def.id)
        .await
        .ok()
        .flatten();

    // CONUS bounding box.
    let spatial_bbox = [-170.0, 15.0, -60.0, 72.0];
    let mut extent = Extent::with_spatial(spatial_bbox, None);

    if let Some((start, end)) = temporal_extent {
        let temporal = TemporalExtent::new(Some(start.to_rfc3339()), Some(end.to_rfc3339()));
        extent = extent.with_temporal(temporal);
    }

    collection = collection.with_extent(extent);

    let mut params = HashMap::new();
    for param_name in available_params {
        let param = Parameter::new(param_name, param_name);
        params.insert(param_name.clone(), param);
    }
    collection = collection.with_parameters(params);

    collection
}

/// GET /edr/collections - List all collections
///
/// Only returns collections that have data available in the catalog.
/// Parameters and vertical levels are also filtered to only include those with data.
pub async fn list_collections_handler(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    // Check Accept header - return 406 if unsupported format requested
    if let Err(response) = check_metadata_accept(&headers) {
        return response;
    }

    let config = state.edr_config.read().await;

    let mut collections = Vec::new();

    for collection_def in config.all_collections() {
        // Find the model config for this collection
        let Some((model_config, coll_def)) = config.find_collection(&collection_def.id) else {
            continue;
        };

        // Feature collections (storm events: hail/wind/tornado)
        if model_config.data_type.is_feature_data() {
            let event_type = &collection_def.id;
            let count = state
                .storm_event_catalog
                .count_events(event_type)
                .await
                .unwrap_or(0);

            if count == 0 {
                tracing::debug!(
                    "Skipping feature collection {} - no events in database",
                    collection_def.id
                );
                continue;
            }

            let available_params: Vec<String> =
                coll_def.parameters.iter().map(|p| p.name.clone()).collect();

            let collection = build_storm_event_collection(
                &state,
                model_config,
                coll_def,
                &available_params,
                count,
            )
            .await;

            collections.push(collection);
            continue;
        }

        // Point observation collections have different availability checks
        if model_config.data_type.is_point_observation() {
            // Check if there are any observations in the database for this source
            let source = model_config
                .observation_source
                .as_deref()
                .unwrap_or("metar");
            let obs_count = state
                .observation_catalog
                .count_observations(Some(source))
                .await
                .unwrap_or(0);

            if obs_count == 0 {
                tracing::debug!(
                    "Skipping point observation collection {} - no observations for source {}",
                    collection_def.id,
                    source
                );
                continue;
            }

            // For point observations, include all configured parameters
            let available_params: Vec<String> =
                coll_def.parameters.iter().map(|p| p.name.clone()).collect();

            // Build the collection for point observations
            let collection = build_point_observation_collection(
                &state,
                model_config,
                coll_def,
                &available_params,
                obs_count,
            )
            .await;

            collections.push(collection);
            continue;
        }

        // Point forecast collections (TAF) use TAF count from database
        if model_config.data_type.is_point_forecast() {
            // Check if there are any TAFs in the database
            let taf_count = state.observation_catalog.count_tafs().await.unwrap_or(0);

            if taf_count == 0 {
                tracing::debug!(
                    "Skipping point forecast collection {} - no TAFs in database",
                    collection_def.id
                );
                continue;
            }

            // For point forecasts, include all configured parameters
            let available_params: Vec<String> =
                coll_def.parameters.iter().map(|p| p.name.clone()).collect();

            // Build the collection for point forecasts (same structure as point observations)
            let collection = build_point_forecast_collection(
                &state,
                model_config,
                coll_def,
                &available_params,
                taf_count,
            )
            .await;

            collections.push(collection);
            continue;
        }

        // Get availability for this model from cache (for gridded data)
        let availability = state
            .availability_cache
            .get_model_availability(&state.catalog, &model_config.model)
            .await;

        // Skip collection if model has no data
        let Some(availability) = availability else {
            tracing::debug!(
                "Skipping collection {} - no data for model {}",
                collection_def.id,
                model_config.model
            );
            continue;
        };

        // Filter parameters to only those with available data
        let available_params = filter_available_parameters(coll_def, &availability);

        // Skip collection if no parameters have data
        if available_params.is_empty() {
            tracing::debug!(
                "Skipping collection {} - no parameters have data",
                collection_def.id
            );
            continue;
        }

        // Log if some parameters were filtered out
        if available_params.len() < coll_def.parameters.len() {
            let configured: Vec<_> = coll_def.parameters.iter().map(|p| &p.name).collect();
            tracing::debug!(
                "Collection {} filtered params: configured {:?}, available {:?}",
                collection_def.id,
                configured,
                available_params
            );
        }

        let mut collection = Collection::new(&collection_def.id)
            .with_title(&collection_def.title)
            .with_description(&collection_def.description);

        // Build links
        collection.build_links(&state.base_url);

        // Add data queries (position, area, radius, trajectory, corridor, locations, and cube if vertical levels)
        let mut queries = DataQueries::with_position(&state.base_url, &collection_def.id)
            .with_area(&state.base_url, &collection_def.id)
            .with_radius(&state.base_url, &collection_def.id)
            .with_trajectory(&state.base_url, &collection_def.id)
            .with_corridor(&state.base_url, &collection_def.id)
            .with_locations(&state.base_url, &collection_def.id);

        // Only add cube for collections with multiple available vertical levels
        if has_multiple_available_vertical_levels(coll_def, &availability) {
            queries = queries.with_cube(&state.base_url, &collection_def.id);
        }

        collection = collection.with_data_queries(queries);

        // Build extent from catalog data, filtered to available levels
        let extent = build_extent_from_catalog_filtered(
            &state.catalog,
            model_config,
            coll_def,
            &availability,
        )
        .await;
        collection = collection.with_extent(extent);

        // Add only available parameters
        let mut params = HashMap::new();
        for param_name in &available_params {
            let param = Parameter::new(param_name, param_name);
            params.insert(param_name.clone(), param);
        }
        if !params.is_empty() {
            collection = collection.with_parameters(params);
        }

        // Add CRS and formats
        collection = collection
            .with_crs(model_config.settings.supported_crs.clone())
            .with_output_formats(model_config.settings.output_formats.clone());

        collections.push(collection);
    }

    // Add the astro collection (always available, computed on-demand)
    collections.push(build_astro_collection(&state.base_url));

    // Log info-level summary on first request (useful for observability)
    let total_configured = config.all_collections().len();
    if collections.len() < total_configured {
        tracing::info!(
            "EDR availability: serving {}/{} collections (filtered by data availability)",
            collections.len(),
            total_configured
        );
    } else {
        tracing::debug!(
            "Returning {} collections with available data",
            collections.len()
        );
    }

    let list = CollectionList::new(collections, &state.base_url);

    let json = serde_json::to_string_pretty(&list).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// GET /edr/collections/:collection_id - Get a specific collection
///
/// Returns 404 if the collection doesn't exist or has no available data.
pub async fn get_collection_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    // Check Accept header - return 406 if unsupported format requested
    if let Err(response) = check_metadata_accept(&headers) {
        return response;
    }

    // Handle astro collection (computed, always available)
    if collection_id == "astro" {
        let collection = build_astro_collection(&state.base_url);
        let json = serde_json::to_string_pretty(&collection).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "max-age=3600")
            .body(json.into())
            .unwrap();
    }

    let config = state.edr_config.read().await;

    // Find the collection config
    let Some((model_config, collection_def)) = config.find_collection(&collection_id) else {
        let exc = ExceptionResponse::not_found(format!("Collection not found: {}", collection_id));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    };

    // Handle feature collections (storm events) differently
    if model_config.data_type.is_feature_data() {
        let count = state
            .storm_event_catalog
            .count_events(&collection_id)
            .await
            .unwrap_or(0);

        if count == 0 {
            let exc =
                ExceptionResponse::not_found(format!("Collection {} has no events", collection_id));
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }

        let available_params: Vec<String> = collection_def
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect();

        let collection = build_storm_event_collection(
            &state,
            model_config,
            collection_def,
            &available_params,
            count,
        )
        .await;

        let json = serde_json::to_string_pretty(&collection).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "max-age=60")
            .body(json.into())
            .unwrap();
    }

    // Handle point observation collections differently
    if model_config.data_type.is_point_observation() {
        let source = model_config
            .observation_source
            .as_deref()
            .unwrap_or("metar");
        let obs_count = state
            .observation_catalog
            .count_observations(Some(source))
            .await
            .unwrap_or(0);

        if obs_count == 0 {
            let exc = ExceptionResponse::not_found(format!(
                "Collection {} has no observations",
                collection_id
            ));
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }

        let available_params: Vec<String> = collection_def
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect();

        let collection = build_point_observation_collection(
            &state,
            model_config,
            collection_def,
            &available_params,
            obs_count,
        )
        .await;

        let json = serde_json::to_string_pretty(&collection).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "max-age=60")
            .body(json.into())
            .unwrap();
    }

    // Handle point forecast collections (TAF)
    if model_config.data_type.is_point_forecast() {
        let taf_count = state.observation_catalog.count_tafs().await.unwrap_or(0);

        if taf_count == 0 {
            let exc = ExceptionResponse::not_found(format!(
                "Collection {} has no TAF forecasts",
                collection_id
            ));
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }

        let available_params: Vec<String> = collection_def
            .parameters
            .iter()
            .map(|p| p.name.clone())
            .collect();

        let collection = build_point_forecast_collection(
            &state,
            model_config,
            collection_def,
            &available_params,
            taf_count,
        )
        .await;

        let json = serde_json::to_string_pretty(&collection).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "max-age=60")
            .body(json.into())
            .unwrap();
    }

    // Get availability for this model (for gridded data)
    let availability = state
        .availability_cache
        .get_model_availability(&state.catalog, &model_config.model)
        .await;

    // Return 404 if model has no data
    let Some(availability) = availability else {
        let exc = ExceptionResponse::not_found(format!(
            "Collection {} has no available data",
            collection_id
        ));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    };

    // Filter parameters to only those with available data
    let available_params = filter_available_parameters(collection_def, &availability);

    // Return 404 if no parameters have data
    if available_params.is_empty() {
        let exc = ExceptionResponse::not_found(format!(
            "Collection {} has no parameters with available data",
            collection_id
        ));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    }

    let mut collection = Collection::new(&collection_def.id)
        .with_title(&collection_def.title)
        .with_description(&collection_def.description);

    // Build links
    collection.build_links(&state.base_url);

    // Add data queries (position, area, radius, trajectory, corridor, locations, and cube if vertical levels)
    let mut queries = DataQueries::with_position(&state.base_url, &collection_def.id)
        .with_area(&state.base_url, &collection_def.id)
        .with_radius(&state.base_url, &collection_def.id)
        .with_trajectory(&state.base_url, &collection_def.id)
        .with_corridor(&state.base_url, &collection_def.id)
        .with_locations(&state.base_url, &collection_def.id);

    // Only add cube for collections with multiple available vertical levels
    if has_multiple_available_vertical_levels(collection_def, &availability) {
        queries = queries.with_cube(&state.base_url, &collection_def.id);
    }

    collection = collection.with_data_queries(queries);

    // Build extent from catalog data, filtered to available levels
    let extent = build_extent_from_catalog_filtered(
        &state.catalog,
        model_config,
        collection_def,
        &availability,
    )
    .await;
    collection = collection.with_extent(extent);

    // Add only available parameters
    let mut params = HashMap::new();
    for param_name in &available_params {
        let param = Parameter::new(param_name, param_name);
        params.insert(param_name.clone(), param);
    }
    if !params.is_empty() {
        collection = collection.with_parameters(params);
    }

    // Add CRS and formats
    collection = collection
        .with_crs(model_config.settings.supported_crs.clone())
        .with_output_formats(model_config.settings.output_formats.clone());

    let json = serde_json::to_string_pretty(&collection).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// Build the astro collection metadata.
///
/// The astro collection is special - it computes solar and lunar data on-demand
/// and doesn't rely on grid data or catalog lookups.
fn build_astro_collection(base_url: &str) -> Collection {
    use edr_protocol::SpatialExtent;

    // Build parameters
    let mut params = HashMap::new();

    params.insert(
        "sunrise".to_string(),
        Parameter::new("sunrise", "Sunrise time (Unix timestamp)"),
    );
    params.insert(
        "sunset".to_string(),
        Parameter::new("sunset", "Sunset time (Unix timestamp)"),
    );
    params.insert(
        "solar_noon".to_string(),
        Parameter::new("solar_noon", "Solar noon time (Unix timestamp)"),
    );
    params.insert(
        "sun_altitude".to_string(),
        Parameter::new("sun_altitude", "Sun altitude angle (degrees)"),
    );
    params.insert(
        "sun_azimuth".to_string(),
        Parameter::new("sun_azimuth", "Sun azimuth angle (degrees)"),
    );
    params.insert(
        "moonrise".to_string(),
        Parameter::new("moonrise", "Moonrise time (Unix timestamp)"),
    );
    params.insert(
        "moonset".to_string(),
        Parameter::new("moonset", "Moonset time (Unix timestamp)"),
    );
    params.insert(
        "moon_phase".to_string(),
        Parameter::new("moon_phase", "Moon phase (categorical)"),
    );
    params.insert(
        "moon_illumination".to_string(),
        Parameter::new("moon_illumination", "Moon illumination fraction (0-1)"),
    );
    params.insert(
        "moon_age".to_string(),
        Parameter::new("moon_age", "Moon age in days since new moon"),
    );

    // Build extent - global coverage, any time
    let extent = Extent {
        spatial: Some(SpatialExtent {
            bbox: vec![vec![-180.0, -90.0, 180.0, 90.0]],
            crs: "CRS:84".to_string(),
        }),
        temporal: Some(TemporalExtent {
            interval: vec![vec![None, None]], // Any time
            values: None,
            trs: "http://www.opengis.net/def/trs/BIPM/0/UTC".to_string(),
        }),
        vertical: None,
        custom: None,
    };

    // Build data queries - only position is supported
    let queries = DataQueries::with_position(base_url, "astro");

    let mut collection = Collection::new("astro")
        .with_title("Astronomical Data")
        .with_description(
            "Solar and lunar position data computed on-demand. \
             Includes sunrise, sunset, solar position, moon phase, and lunar position. \
             Data is computed using astronomical algorithms and is available for any \
             location and time without requiring stored data.",
        )
        .with_parameters(params)
        .with_extent(extent)
        .with_data_queries(queries)
        .with_crs(vec!["CRS:84".to_string(), "EPSG:4326".to_string()])
        .with_output_formats(vec![
            "application/vnd.cov+json".to_string(),
            "application/geo+json".to_string(),
        ]);

    collection.build_links(base_url);
    collection
}

#[cfg(test)]
mod tests {
    use edr_protocol::{Collection, CollectionList, DataQueries};

    #[test]
    fn test_collection_creation() {
        let mut collection = Collection::new("test-collection")
            .with_title("Test Collection")
            .with_description("A test collection");

        collection.build_links("http://localhost:8083/edr");

        assert_eq!(collection.id, "test-collection");
        assert!(collection.links.iter().any(|l| l.rel == "self"));
    }

    #[test]
    fn test_collection_list_creation() {
        let collection = Collection::new("test-collection").with_title("Test Collection");

        let list = CollectionList::new(vec![collection], "http://localhost:8083/edr");

        assert_eq!(list.collections.len(), 1);
        assert!(!list.links.is_empty());
    }

    #[test]
    fn test_data_queries_construction() {
        let queries = DataQueries::with_position("http://localhost:8083/edr", "test-collection")
            .with_area("http://localhost:8083/edr", "test-collection")
            .with_radius("http://localhost:8083/edr", "test-collection");

        assert!(queries.position.is_some());
        assert!(queries.area.is_some());
        assert!(queries.radius.is_some());
    }
}
