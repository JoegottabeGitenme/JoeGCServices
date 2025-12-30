//! Collections endpoint handlers.

use axum::{
    extract::{Extension, Path},
    http::{header, StatusCode},
    response::Response,
};
use edr_protocol::{
    responses::ExceptionResponse, Collection, CollectionList, DataQueries, Extent, Parameter,
    TemporalExtent, VerticalExtent,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{CollectionDefinition, LevelValue, ModelEdrConfig};
use crate::state::AppState;
use storage::Catalog;

/// Build extent from catalog data for a collection.
async fn build_extent_from_catalog(
    catalog: &Catalog,
    model_config: &ModelEdrConfig,
    collection_def: &CollectionDefinition,
) -> Extent {
    let model_name = &model_config.model;

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

    // Build vertical extent from collection definition
    // Collect all numeric levels from parameters
    let mut level_values: Vec<f64> = Vec::new();
    let mut has_pressure_levels = false;
    let mut has_height_levels = false;

    for param in &collection_def.parameters {
        for level in &param.levels {
            match level {
                LevelValue::Numeric(v) => {
                    if !level_values.contains(v) {
                        level_values.push(*v);
                    }
                }
                LevelValue::Named(s) => {
                    // Try to parse named levels like "surface" -> skip, "1000" -> add
                    if let Ok(v) = s.parse::<f64>() {
                        if !level_values.contains(&v) {
                            level_values.push(v);
                        }
                    }
                }
            }
        }
    }

    // Determine VRS based on level_filter type
    let level_type = &collection_def.level_filter.level_type;
    if level_type == "isobaric" || level_type.contains("pressure") {
        has_pressure_levels = true;
    } else if level_type.contains("height") || level_type.contains("ground") {
        has_height_levels = true;
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

    extent
}

/// GET /edr/collections - List all collections
pub async fn list_collections_handler(Extension(state): Extension<Arc<AppState>>) -> Response {
    let config = state.edr_config.read().await;

    let mut collections = Vec::new();

    for collection_def in config.all_collections() {
        let mut collection = Collection::new(&collection_def.id)
            .with_title(&collection_def.title)
            .with_description(&collection_def.description);

        // Build links
        collection.build_links(&state.base_url);

        // Add data queries (position and area)
        let queries = DataQueries::with_position(&state.base_url, &collection_def.id)
            .with_area(&state.base_url, &collection_def.id);
        collection = collection.with_data_queries(queries);

        // Build extent from catalog data
        if let Some((model_config, coll_def)) = config.find_collection(&collection_def.id) {
            let extent = build_extent_from_catalog(&state.catalog, model_config, coll_def).await;
            collection = collection.with_extent(extent);

            // Add CRS and formats
            collection = collection
                .with_crs(model_config.settings.supported_crs.clone())
                .with_output_formats(model_config.settings.output_formats.clone());
        } else {
            // Fallback to global extent if no model config
            collection =
                collection.with_extent(Extent::with_spatial([-180.0, -90.0, 180.0, 90.0], None));
        }

        collections.push(collection);
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
pub async fn get_collection_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
) -> Response {
    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, collection_def)) = config.find_collection(&collection_id) else {
        let exc = ExceptionResponse::not_found(format!("Collection not found: {}", collection_id));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    };

    let mut collection = Collection::new(&collection_def.id)
        .with_title(&collection_def.title)
        .with_description(&collection_def.description);

    // Build links
    collection.build_links(&state.base_url);

    // Add data queries (position and area)
    let queries = DataQueries::with_position(&state.base_url, &collection_def.id)
        .with_area(&state.base_url, &collection_def.id);
    collection = collection.with_data_queries(queries);

    // Build extent from catalog data
    let extent = build_extent_from_catalog(&state.catalog, model_config, collection_def).await;
    collection = collection.with_extent(extent);

    // Add parameters
    let mut params = HashMap::new();
    for param_def in &collection_def.parameters {
        let param = Parameter::new(&param_def.name, &param_def.name);
        // TODO: Add unit and levels from catalog
        params.insert(param_def.name.clone(), param);
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
        let collections = vec![
            Collection::new("col1").with_title("Collection 1"),
            Collection::new("col2").with_title("Collection 2"),
        ];

        let list = CollectionList::new(collections, "http://localhost:8083/edr");

        assert_eq!(list.collections.len(), 2);
        assert!(list.links.iter().any(|l| l.rel == "self"));
    }

    #[test]
    fn test_data_queries() {
        let queries = DataQueries::with_position("http://localhost:8083/edr", "test");

        assert!(queries.position.is_some());
        let pos = queries.position.unwrap();
        assert!(pos.link.href.contains("/position"));
    }
}
