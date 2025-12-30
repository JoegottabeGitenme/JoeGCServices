//! Health and metrics handlers.

use std::sync::Arc;
use axum::{
    extract::Extension,
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::Serialize;

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
pub async fn ready_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Response {
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

/// GET /metrics - Prometheus metrics
pub async fn metrics_handler() -> Response {
    // TODO: Integrate with metrics-exporter-prometheus
    let metrics = r#"# HELP edr_requests_total Total EDR API requests
# TYPE edr_requests_total counter
edr_requests_total{endpoint="position"} 0
edr_requests_total{endpoint="collections"} 0

# HELP edr_request_duration_seconds EDR request duration
# TYPE edr_request_duration_seconds histogram
edr_request_duration_seconds_bucket{endpoint="position",le="0.01"} 0
edr_request_duration_seconds_bucket{endpoint="position",le="0.1"} 0
edr_request_duration_seconds_bucket{endpoint="position",le="1"} 0
"#;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4")
        .body(metrics.into())
        .unwrap()
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
