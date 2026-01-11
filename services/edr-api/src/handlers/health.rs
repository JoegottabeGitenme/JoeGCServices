//! Health and metrics handlers.

use axum::{
    extract::Extension,
    http::{header, StatusCode},
    response::Response,
    Json,
};
use metrics_exporter_prometheus::PrometheusHandle;
use serde::Serialize;
use std::sync::Arc;

use crate::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Serialize)]
pub struct ReadyResponse {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
}

/// GET /health - Basic health check
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

/// GET /ready - Readiness check (verifies database connectivity)
pub async fn ready_handler(Extension(state): Extension<Arc<AppState>>) -> Response {
    // Check database connectivity by listing models
    let db_status = match state.catalog.list_models().await {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("error: {}", e),
    };

    let is_ready = db_status == "ok";

    let response = ReadyResponse {
        ready: is_ready,
        database: Some(db_status),
        storage: Some("ok".to_string()), // TODO: Check MinIO
    };

    let status = if is_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let json = serde_json::to_string(&response).unwrap_or_default();

    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json.into())
        .unwrap()
}

/// GET /metrics - Prometheus metrics (for Prometheus scraping)
pub async fn metrics_handler(
    Extension(prometheus_handle): Extension<PrometheusHandle>,
) -> Response {
    let metrics = prometheus_handle.render();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")
        .body(metrics.into())
        .unwrap()
}

/// GET /api/metrics - JSON metrics (for admin dashboard and Grafana)
pub async fn json_metrics_handler(Extension(state): Extension<Arc<AppState>>) -> Response {
    let snapshot = state.metrics.snapshot().await;

    match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(json.into())
            .unwrap(),
        Err(e) => {
            tracing::error!("Failed to serialize metrics: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(r#"{"error": "Failed to serialize metrics"}"#.into())
                .unwrap()
        }
    }
}

/// GET /api/metrics/heatmap - Query extent heatmap for geographic visualization
pub async fn heatmap_handler(Extension(state): Extension<Arc<AppState>>) -> Response {
    let heatmap = state.metrics.get_query_heatmap().await;

    match serde_json::to_string_pretty(&heatmap) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(json.into())
            .unwrap(),
        Err(e) => {
            tracing::error!("Failed to serialize heatmap: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(r#"{"error": "Failed to serialize heatmap"}"#.into())
                .unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_handler() {
        let response = health_handler().await;
        assert_eq!(response.status, "ok");
    }
}
