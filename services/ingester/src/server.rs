//! HTTP server for the ingester service.
//!
//! Provides endpoints for:
//! - `POST /ingest` - Ingest a file (called by downloader)
//! - `GET /status` - Get active/recent ingestions
//! - `GET /health` - Health check
//! - `GET /metrics` - Prometheus metrics

use axum::{
    extract::{Extension, Json},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};
use uuid::Uuid;

use ingestion::{IngestOptions, Ingester, IngestionResult};

/// Shared state for the HTTP server.
pub struct ServerState {
    /// Core ingester
    pub ingester: Ingester,
    /// Tracking for active/completed ingestions
    pub tracker: IngestionTracker,
}

/// Request body for /ingest endpoint.
#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// Path to the file to ingest
    pub file_path: String,
    /// Source URL (for logging/tracking)
    #[serde(default)]
    #[allow(dead_code)]
    pub source_url: Option<String>,
    /// Override model detection
    #[serde(default)]
    pub model: Option<String>,
    /// Override forecast hour detection
    #[serde(default)]
    pub forecast_hour: Option<u32>,
}

/// Response body for /ingest endpoint.
#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub success: bool,
    pub message: String,
    pub datasets_registered: usize,
    pub model: Option<String>,
    pub reference_time: Option<String>,
    pub parameters: Vec<String>,
}

impl From<IngestionResult> for IngestResponse {
    fn from(result: IngestionResult) -> Self {
        Self {
            success: true,
            message: format!("Ingested {} datasets", result.datasets_registered),
            datasets_registered: result.datasets_registered,
            model: Some(result.model),
            reference_time: Some(result.reference_time.to_rfc3339()),
            parameters: result.parameters,
        }
    }
}

/// Tracking for ingestion operations.
pub struct IngestionTracker {
    active: Mutex<std::collections::HashMap<String, ActiveIngestion>>,
    completed: Mutex<VecDeque<CompletedIngestion>>,
    max_completed: usize,
}

/// An active ingestion operation.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveIngestion {
    pub id: String,
    pub file_path: String,
    pub started_at: DateTime<Utc>,
    pub status: String,
}

/// A completed ingestion operation.
#[derive(Debug, Clone, Serialize)]
pub struct CompletedIngestion {
    pub id: String,
    pub file_path: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub success: bool,
    pub datasets_registered: usize,
    pub parameters: Vec<String>,
    pub error_message: Option<String>,
}

impl IngestionTracker {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(std::collections::HashMap::new()),
            completed: Mutex::new(VecDeque::new()),
            max_completed: 100,
        }
    }

    pub async fn start(&self, id: &str, file_path: &str) {
        let ingestion = ActiveIngestion {
            id: id.to_string(),
            file_path: file_path.to_string(),
            started_at: Utc::now(),
            status: "processing".to_string(),
        };
        self.active.lock().await.insert(id.to_string(), ingestion);
    }

    pub async fn complete(
        &self,
        id: &str,
        success: bool,
        datasets_registered: usize,
        parameters: Vec<String>,
        error_message: Option<String>,
    ) {
        let mut active = self.active.lock().await;
        if let Some(ingestion) = active.remove(id) {
            let completed_at = Utc::now();
            let duration_ms = (completed_at - ingestion.started_at).num_milliseconds() as u64;

            let completed = CompletedIngestion {
                id: ingestion.id,
                file_path: ingestion.file_path,
                started_at: ingestion.started_at,
                completed_at,
                duration_ms,
                success,
                datasets_registered,
                parameters,
                error_message,
            };

            let mut completed_list = self.completed.lock().await;
            completed_list.push_front(completed);

            // Keep only recent entries
            while completed_list.len() > self.max_completed {
                completed_list.pop_back();
            }
        }
    }

    pub async fn get_status(&self) -> StatusResponse {
        let active = self.active.lock().await;
        let completed = self.completed.lock().await;

        StatusResponse {
            active: active.values().cloned().collect(),
            recent: completed.iter().take(20).cloned().collect(),
            total_completed: completed.len(),
        }
    }
}

/// Response for /status endpoint.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub active: Vec<ActiveIngestion>,
    pub recent: Vec<CompletedIngestion>,
    pub total_completed: usize,
}

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
}

/// POST /ingest - Ingest a file
async fn ingest_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Json(request): Json<IngestRequest>,
) -> impl IntoResponse {
    let id = Uuid::new_v4().to_string();

    info!(
        id = %id,
        file_path = %request.file_path,
        model = ?request.model,
        "Received ingest request"
    );

    // Track start
    state.tracker.start(&id, &request.file_path).await;

    // Build options
    let options = IngestOptions {
        model: request.model,
        forecast_hour: request.forecast_hour,
    };

    // Perform ingestion
    match state
        .ingester
        .ingest_file(&request.file_path, options)
        .await
    {
        Ok(result) => {
            info!(
                id = %id,
                datasets = result.datasets_registered,
                parameters = ?result.parameters,
                "Ingestion completed successfully"
            );

            state
                .tracker
                .complete(
                    &id,
                    true,
                    result.datasets_registered,
                    result.parameters.clone(),
                    None,
                )
                .await;

            (StatusCode::OK, Json(IngestResponse::from(result)))
        }
        Err(e) => {
            error!(id = %id, error = %e, "Ingestion failed");

            state
                .tracker
                .complete(&id, false, 0, vec![], Some(e.to_string()))
                .await;

            let response = IngestResponse {
                success: false,
                message: format!("Ingestion failed: {}", e),
                datasets_registered: 0,
                model: None,
                reference_time: None,
                parameters: vec![],
            };

            (StatusCode::INTERNAL_SERVER_ERROR, Json(response))
        }
    }
}

/// GET /status - Get ingestion status
async fn status_handler(Extension(state): Extension<Arc<ServerState>>) -> impl IntoResponse {
    let status = state.tracker.get_status().await;
    Json(status)
}

/// GET /health - Health check
async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
        service: "ingester".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// GET /metrics - Prometheus metrics
async fn metrics_handler() -> impl IntoResponse {
    // For now, return a simple placeholder
    // TODO: Integrate with actual metrics collection
    "# HELP ingester_info Ingester service information\n\
     # TYPE ingester_info gauge\n\
     ingester_info{version=\"0.1.0\"} 1\n"
}

/// Build the HTTP router.
pub fn build_router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/ingest", post(ingest_handler))
        .route("/status", get(status_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .layer(Extension(state))
}

/// Start the HTTP server.
pub async fn start_server(state: Arc<ServerState>, port: u16) -> anyhow::Result<()> {
    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!(port = port, "Starting ingester HTTP server");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
