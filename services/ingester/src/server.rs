//! HTTP server for the ingester service.
//!
//! Provides endpoints for:
//! - `POST /ingest` - Ingest a file (called by downloader)
//! - `POST /ingest/observations` - Ingest observation data (METAR, etc.)
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
use tracing::{error, info, warn};
use uuid::Uuid;

use ingestion::{IngestOptions, Ingester, IngestionResult};
use storage::observations::{Location, Observation, ObservationCatalog};

/// Shared state for the HTTP server.
pub struct ServerState {
    /// Core ingester for gridded data
    pub ingester: Ingester,
    /// Observation catalog for point data (METAR, etc.)
    pub observation_catalog: Option<ObservationCatalog>,
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

/// Request body for /ingest/observations endpoint.
#[derive(Debug, Deserialize)]
pub struct ObservationIngestRequest {
    /// Source identifier (e.g., "metar")
    pub source: String,
    /// List of observations
    pub observations: Vec<ObservationData>,
}

/// Single observation from the downloader.
#[derive(Debug, Deserialize)]
pub struct ObservationData {
    /// Location identifier (ICAO code)
    pub location_id: String,
    /// Station name
    pub name: Option<String>,
    /// Longitude
    pub longitude: f64,
    /// Latitude
    pub latitude: f64,
    /// Elevation in meters
    pub elevation_m: Option<f32>,
    /// Observation time (ISO 8601)
    pub obs_time: String,
    /// Receipt time (ISO 8601)
    pub receipt_time: Option<String>,
    /// Temperature in Kelvin
    pub temperature_k: Option<f32>,
    /// Dewpoint in Kelvin
    pub dewpoint_k: Option<f32>,
    /// Wind direction in degrees
    pub wind_direction_deg: Option<i16>,
    /// Wind speed in m/s
    pub wind_speed_mps: Option<f32>,
    /// Wind gust in m/s
    pub wind_gust_mps: Option<f32>,
    /// Visibility in meters
    pub visibility_m: Option<f32>,
    /// Altimeter setting in Pascals
    pub altimeter_pa: Option<f32>,
    /// Sea level pressure in Pascals
    pub sea_level_pressure_pa: Option<f32>,
    /// Cloud layers as JSON
    pub cloud_layers: Option<serde_json::Value>,
    /// Flight category
    pub flight_category: Option<String>,
    /// Weather phenomena string
    pub wx_string: Option<String>,
    /// Raw observation text
    pub raw_observation: Option<String>,
}

/// Response for /ingest/observations endpoint.
#[derive(Debug, Serialize)]
pub struct ObservationIngestResponse {
    pub success: bool,
    pub message: String,
    pub observations_ingested: usize,
    pub locations_updated: usize,
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

/// POST /ingest/observations - Ingest observation data (METAR, etc.)
async fn ingest_observations_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Json(request): Json<ObservationIngestRequest>,
) -> impl IntoResponse {
    let catalog = match &state.observation_catalog {
        Some(c) => c,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ObservationIngestResponse {
                    success: false,
                    message: "Observation catalog not configured".to_string(),
                    observations_ingested: 0,
                    locations_updated: 0,
                }),
            );
        }
    };

    info!(
        source = %request.source,
        count = request.observations.len(),
        "Received observation ingest request"
    );

    let mut locations_to_upsert = Vec::new();
    let mut observations_to_insert = Vec::new();

    for obs_data in &request.observations {
        // Parse observation time
        let obs_time = match chrono::DateTime::parse_from_rfc3339(&obs_data.obs_time) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                warn!(
                    location_id = %obs_data.location_id,
                    obs_time = %obs_data.obs_time,
                    error = %e,
                    "Failed to parse observation time, skipping"
                );
                continue;
            }
        };

        // Parse receipt time if present
        let receipt_time = obs_data.receipt_time.as_ref().and_then(|rt| {
            chrono::DateTime::parse_from_rfc3339(rt)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        // Build location
        let name = obs_data
            .name
            .clone()
            .unwrap_or_else(|| obs_data.location_id.clone());
        let mut location = Location::new(
            obs_data.location_id.clone(),
            name,
            obs_data.longitude,
            obs_data.latitude,
        );

        if let Some(elev) = obs_data.elevation_m {
            location = location.with_elevation(elev);
        }
        location = location.with_type(&request.source);

        locations_to_upsert.push(location);

        // Build observation
        let observation = Observation {
            id: None, // Auto-generated
            location_id: obs_data.location_id.clone(),
            source: request.source.clone(),
            obs_time,
            receipt_time,
            temperature_k: obs_data.temperature_k,
            dewpoint_k: obs_data.dewpoint_k,
            wind_direction_deg: obs_data.wind_direction_deg,
            wind_speed_ms: obs_data.wind_speed_mps,
            wind_gust_ms: obs_data.wind_gust_mps,
            visibility_m: obs_data.visibility_m,
            altimeter_pa: obs_data.altimeter_pa,
            sea_level_pressure_pa: obs_data.sea_level_pressure_pa,
            cloud_layers: obs_data.cloud_layers.clone(),
            flight_category: obs_data.flight_category.clone(),
            wx_string: obs_data.wx_string.clone(),
            raw_text: obs_data.raw_observation.clone(),
            ..Default::default()
        };

        observations_to_insert.push(observation);
    }

    // Upsert locations first
    let locations_updated = match catalog.upsert_locations(&locations_to_upsert).await {
        Ok(count) => count,
        Err(e) => {
            error!(error = %e, "Failed to upsert locations");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ObservationIngestResponse {
                    success: false,
                    message: format!("Failed to upsert locations: {}", e),
                    observations_ingested: 0,
                    locations_updated: 0,
                }),
            );
        }
    };

    // Insert observations
    let insert_result = match catalog.insert_observations(&observations_to_insert).await {
        Ok(result) => result,
        Err(e) => {
            error!(error = %e, "Failed to insert observations");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ObservationIngestResponse {
                    success: false,
                    message: format!("Failed to insert observations: {}", e),
                    observations_ingested: 0,
                    locations_updated,
                }),
            );
        }
    };

    let observations_ingested = insert_result.inserted;

    info!(
        source = %request.source,
        observations_ingested = observations_ingested,
        duplicates_skipped = insert_result.duplicates,
        locations_updated = locations_updated,
        "Observation ingestion completed"
    );

    (
        StatusCode::OK,
        Json(ObservationIngestResponse {
            success: true,
            message: format!(
                "Ingested {} observations ({} duplicates skipped), updated {} locations",
                observations_ingested, insert_result.duplicates, locations_updated
            ),
            observations_ingested,
            locations_updated,
        }),
    )
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
        .route("/ingest/observations", post(ingest_observations_handler))
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
