//! Catalog check handler for coverage validation.
//!
//! Returns what's actually available in the database, organized to match
//! the EDR collection structure for coverage validation.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::Extension,
    http::{header, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::state::AppState;

/// Response from the catalog check endpoint.
#[derive(Serialize)]
pub struct CatalogCheckResponse {
    /// When this response was generated
    pub generated_at: DateTime<Utc>,
    /// Catalog information organized by model
    pub models: HashMap<String, ModelCatalogInfo>,
}

/// Catalog information for a single model.
#[derive(Serialize)]
pub struct ModelCatalogInfo {
    /// Total number of datasets for this model
    pub total_datasets: u64,
    /// Parameters and their availability
    pub parameters: HashMap<String, ParameterCatalogInfo>,
    /// Model run instances
    pub instances: Vec<InstanceInfo>,
    /// Overall temporal extent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporal_extent: Option<TemporalRange>,
    /// Bounding box [min_x, min_y, max_x, max_y]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
}

/// Catalog information for a single parameter.
#[derive(Serialize)]
pub struct ParameterCatalogInfo {
    /// Available levels for this parameter
    pub levels: Vec<String>,
    /// Number of datasets for this parameter
    pub count: u64,
    /// Available valid times (ISO8601)
    pub valid_times: Vec<String>,
}

/// Information about a model run instance.
#[derive(Serialize)]
pub struct InstanceInfo {
    /// Reference time (model run time) in ISO8601
    pub reference_time: String,
    /// Number of datasets in this run
    pub dataset_count: i64,
    /// Available forecast hours
    pub forecast_hours: Vec<i32>,
}

/// Temporal range with min and max times.
#[derive(Serialize)]
pub struct TemporalRange {
    pub min: String,
    pub max: String,
}

/// GET /edr/catalog-check - Return catalog inventory for coverage validation
pub async fn catalog_check_handler(Extension(state): Extension<Arc<AppState>>) -> Response {
    // Query catalog for all models
    let models = match state.catalog.list_models().await {
        Ok(m) => m,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to list models: {}", e),
            );
        }
    };

    let mut result = HashMap::new();

    for model_name in models {
        // Get parameters for this model
        let params = state
            .catalog
            .list_parameters(&model_name)
            .await
            .unwrap_or_default();

        let mut param_info = HashMap::new();
        let mut total_count: u64 = 0;

        for param in params {
            // Get levels for this parameter
            let levels = state
                .catalog
                .get_available_levels(&model_name, &param)
                .await
                .unwrap_or_default();

            // Get available times for this parameter
            let times = state
                .catalog
                .get_available_times(&model_name, &param)
                .await
                .unwrap_or_default();

            let count = times.len() as u64;
            total_count += count;

            param_info.insert(
                param,
                ParameterCatalogInfo {
                    levels,
                    count,
                    valid_times: times
                        .into_iter()
                        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                        .collect(),
                },
            );
        }

        // Get instances (model runs) with counts
        let runs = state
            .catalog
            .get_model_runs_with_counts(&model_name)
            .await
            .unwrap_or_default();

        // For each run, get the forecast hours
        let mut instances = Vec::new();
        for (ref_time, count) in runs {
            // Get forecast hours for this run
            let forecast_hours = state
                .catalog
                .get_available_forecast_hours(
                    &model_name,
                    &param_info.keys().next().unwrap_or(&String::new()),
                )
                .await
                .unwrap_or_default();

            instances.push(InstanceInfo {
                reference_time: ref_time.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                dataset_count: count,
                forecast_hours,
            });
        }

        // Get temporal extent
        let temporal = state
            .catalog
            .get_model_temporal_extent(&model_name)
            .await
            .ok()
            .flatten()
            .map(|(min, max)| TemporalRange {
                min: min.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                max: max.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            });

        // Get bbox
        let bbox = state
            .catalog
            .get_model_bbox(&model_name)
            .await
            .ok()
            .map(|b| [b.min_x, b.min_y, b.max_x, b.max_y]);

        result.insert(
            model_name,
            ModelCatalogInfo {
                total_datasets: total_count,
                parameters: param_info,
                instances,
                temporal_extent: temporal,
                bbox,
            },
        );
    }

    let response = CatalogCheckResponse {
        generated_at: Utc::now(),
        models: result,
    };

    // Return JSON (no caching)
    let json = serde_json::to_string_pretty(&response).unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
        .header("Access-Control-Allow-Origin", "*")
        .body(json.into())
        .unwrap()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({
        "error": message
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_string().into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_check_response_serialization() {
        let mut params = HashMap::new();
        params.insert(
            "TMP".to_string(),
            ParameterCatalogInfo {
                levels: vec!["surface".to_string(), "1000 mb".to_string()],
                count: 10,
                valid_times: vec!["2025-12-30T00:00:00Z".to_string()],
            },
        );

        let mut models = HashMap::new();
        models.insert(
            "hrrr".to_string(),
            ModelCatalogInfo {
                total_datasets: 100,
                parameters: params,
                instances: vec![InstanceInfo {
                    reference_time: "2025-12-30T00:00:00Z".to_string(),
                    dataset_count: 50,
                    forecast_hours: vec![0, 1, 2, 3],
                }],
                temporal_extent: Some(TemporalRange {
                    min: "2025-12-30T00:00:00Z".to_string(),
                    max: "2025-12-30T18:00:00Z".to_string(),
                }),
                bbox: Some([-125.0, 25.0, -65.0, 50.0]),
            },
        );

        let response = CatalogCheckResponse {
            generated_at: Utc::now(),
            models,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("hrrr"));
        assert!(json.contains("TMP"));
        assert!(json.contains("surface"));
    }
}
