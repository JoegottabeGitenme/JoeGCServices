//! Instances endpoint handlers.

use axum::{
    extract::{Extension, Path},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use edr_protocol::{
    responses::ExceptionResponse, DataQueries, Extent, Instance, InstanceList, TemporalExtent,
};
use std::sync::Arc;

use crate::content_negotiation::check_metadata_accept;
use crate::state::AppState;

/// GET /edr/collections/:collection_id/instances - List all instances
pub async fn list_instances_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    // Check Accept header - return 406 if unsupported format requested
    if let Err(response) = check_metadata_accept(&headers) {
        return response;
    }

    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, _collection_def)) = config.find_collection(&collection_id) else {
        let exc = ExceptionResponse::not_found(format!("Collection not found: {}", collection_id));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    };

    // Query catalog for available model runs
    let model_name = &model_config.model;
    let runs = match state.catalog.get_model_runs_with_counts(model_name).await {
        Ok(runs) => runs,
        Err(e) => {
            tracing::error!("Failed to list model runs: {}", e);
            let exc = ExceptionResponse::internal_error("Failed to list instances");
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }
    };

    // Build instance list
    let mut instances = Vec::new();
    for (reference_time, _count) in runs {
        let run_id = reference_time.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let mut instance = Instance::new(&run_id).with_title(format!(
            "{} run at {}",
            model_name.to_uppercase(),
            run_id
        ));

        // Build links
        instance.build_links(&state.base_url, &collection_id);

        // Add data queries
        let queries = DataQueries::with_position(&state.base_url, &collection_id);
        instance = instance.with_data_queries(queries);

        // Get actual temporal extent from forecast range
        let forecast_range = state
            .catalog
            .get_run_forecast_range(model_name, reference_time)
            .await
            .ok()
            .flatten();
        let (start_str, end_str) = match forecast_range {
            Some((start, end)) => (
                start.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                Some(end.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            ),
            None => (run_id.clone(), None),
        };

        let extent = Extent {
            spatial: None,
            temporal: Some(TemporalExtent::new(Some(start_str), end_str)),
            vertical: None,
        };
        instance = instance.with_extent(extent);

        instances.push(instance);
    }

    let list = InstanceList::new(instances, &state.base_url, &collection_id);

    let json = serde_json::to_string_pretty(&list).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

/// GET /edr/collections/:collection_id/instances/:instance_id - Get a specific instance
pub async fn get_instance_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path((collection_id, instance_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    // Check Accept header - return 406 if unsupported format requested
    if let Err(response) = check_metadata_accept(&headers) {
        return response;
    }

    let config = state.edr_config.read().await;

    // Find the collection
    let Some((model_config, _collection_def)) = config.find_collection(&collection_id) else {
        let exc = ExceptionResponse::not_found(format!("Collection not found: {}", collection_id));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    };

    // Parse instance_id as datetime
    let reference_time = match chrono::DateTime::parse_from_rfc3339(&instance_id) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => {
            let exc = ExceptionResponse::bad_request(format!(
                "Invalid instance ID format: {}. Expected ISO8601 datetime.",
                instance_id
            ));
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }
    };

    // Check if this run exists by querying available runs
    let model_name = &model_config.model;
    let runs = match state.catalog.get_model_runs_with_counts(model_name).await {
        Ok(runs) => runs,
        Err(e) => {
            tracing::error!("Failed to query model runs: {}", e);
            let exc = ExceptionResponse::internal_error("Failed to get instance");
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }
    };

    // Check if the requested run exists
    let run_exists = runs.iter().any(|(rt, _)| *rt == reference_time);
    if !run_exists {
        let exc = ExceptionResponse::not_found(format!(
            "Instance not found: {} for collection {}",
            instance_id, collection_id
        ));
        let json = serde_json::to_string(&exc).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(json.into())
            .unwrap();
    }

    let mut instance = Instance::new(&instance_id).with_title(format!(
        "{} run at {}",
        model_name.to_uppercase(),
        instance_id
    ));

    // Build links
    instance.build_links(&state.base_url, &collection_id);

    // Add data queries
    let queries = DataQueries::with_position(&state.base_url, &collection_id);
    instance = instance.with_data_queries(queries);

    // Get actual temporal extent from forecast range
    let forecast_range = state
        .catalog
        .get_run_forecast_range(model_name, reference_time)
        .await
        .ok()
        .flatten();
    let (start_str, end_str) = match forecast_range {
        Some((start, end)) => (
            start.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            Some(end.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
        ),
        None => (instance_id.clone(), None),
    };

    let extent = Extent {
        spatial: None,
        temporal: Some(TemporalExtent::new(Some(start_str), end_str)),
        vertical: None,
    };
    instance = instance.with_extent(extent);

    let json = serde_json::to_string_pretty(&instance).unwrap_or_default();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "max-age=60")
        .body(json.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use edr_protocol::{Instance, InstanceList};

    #[test]
    fn test_instance_creation() {
        let mut instance =
            Instance::new("2024-12-29T12:00:00Z").with_title("HRRR run at 2024-12-29 12Z");

        instance.build_links("http://localhost:8083/edr", "hrrr-isobaric");

        assert_eq!(instance.id, "2024-12-29T12:00:00Z");
        assert!(instance.links.iter().any(|l| l.rel == "self"));
        assert!(instance.links.iter().any(|l| l.rel == "collection"));
    }

    #[test]
    fn test_instance_list() {
        let instances = vec![
            Instance::new("2024-12-29T12:00:00Z"),
            Instance::new("2024-12-29T06:00:00Z"),
        ];

        let list = InstanceList::new(instances, "http://localhost:8083/edr", "hrrr-isobaric");

        assert_eq!(list.instances.len(), 2);
    }
}
