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
use storage::observations::{Location, Observation, ObservationCatalog, TafForecast, TafPeriod};

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
    /// Significant wave height in meters
    pub wave_height_m: Option<f32>,
    /// Dominant wave period in seconds
    pub dominant_wave_period_s: Option<f32>,
    /// Average wave period in seconds
    pub average_wave_period_s: Option<f32>,
    /// Mean wave direction in degrees
    pub mean_wave_direction_deg: Option<i16>,
    /// Water temperature in Kelvin
    pub water_temp_k: Option<f32>,
    /// Tide / water level in meters
    pub tide_m: Option<f32>,
    /// Water column height in meters (DART tsunami buoys)
    pub water_column_height_m: Option<f32>,
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

/// Request body for /ingest/tafs endpoint.
#[derive(Debug, Deserialize)]
pub struct TafIngestRequest {
    /// Source identifier (e.g., "taf")
    pub source: String,
    /// List of TAF forecasts
    pub forecasts: Vec<TafData>,
}

/// Single TAF forecast from the downloader.
#[derive(Debug, Deserialize)]
pub struct TafData {
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
    /// Issue time (ISO 8601)
    pub issue_time: String,
    /// Start of validity (ISO 8601)
    pub valid_from: String,
    /// End of validity (ISO 8601)
    pub valid_to: String,
    /// Raw TAF text
    pub raw_taf: Option<String>,
    /// Forecast periods
    pub periods: Vec<TafPeriodData>,
}

/// Single TAF period from the downloader.
#[derive(Debug, Deserialize)]
pub struct TafPeriodData {
    /// Start of period (ISO 8601)
    pub period_from: String,
    /// End of period (ISO 8601)
    pub period_to: String,
    /// Change indicator: null, "FM", "BECMG", "TEMPO"
    pub change_indicator: Option<String>,
    /// Probability (30 or 40)
    pub probability: Option<i16>,
    /// Wind direction in degrees
    pub wind_direction_deg: Option<i16>,
    /// Wind speed in m/s
    pub wind_speed_ms: Option<f32>,
    /// Wind gust in m/s
    pub wind_gust_ms: Option<f32>,
    /// Visibility in meters
    pub visibility_m: Option<f32>,
    /// Weather phenomena string
    pub wx_string: Option<String>,
    /// Cloud layers as JSON
    pub cloud_layers: Option<serde_json::Value>,
}

/// Response for /ingest/tafs endpoint.
#[derive(Debug, Serialize)]
pub struct TafIngestResponse {
    pub success: bool,
    pub message: String,
    pub tafs_ingested: usize,
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
            wave_height_m: obs_data.wave_height_m,
            dominant_wave_period_s: obs_data.dominant_wave_period_s,
            average_wave_period_s: obs_data.average_wave_period_s,
            mean_wave_direction_deg: obs_data.mean_wave_direction_deg,
            water_temp_k: obs_data.water_temp_k,
            tide_m: obs_data.tide_m,
            water_column_height_m: obs_data.water_column_height_m,
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

/// POST /ingest/tafs - Ingest TAF forecast data
async fn ingest_tafs_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Json(request): Json<TafIngestRequest>,
) -> impl IntoResponse {
    let catalog = match &state.observation_catalog {
        Some(c) => c,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(TafIngestResponse {
                    success: false,
                    message: "Observation catalog not configured".to_string(),
                    tafs_ingested: 0,
                    locations_updated: 0,
                }),
            );
        }
    };

    info!(
        source = %request.source,
        count = request.forecasts.len(),
        "Received TAF ingest request"
    );

    let mut locations_to_upsert = Vec::new();
    let mut tafs_ingested = 0;
    let mut errors = Vec::new();

    for taf_data in &request.forecasts {
        // Parse times
        let issue_time = match chrono::DateTime::parse_from_rfc3339(&taf_data.issue_time) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                warn!(
                    location_id = %taf_data.location_id,
                    issue_time = %taf_data.issue_time,
                    error = %e,
                    "Failed to parse TAF issue time, skipping"
                );
                continue;
            }
        };

        let valid_from = match chrono::DateTime::parse_from_rfc3339(&taf_data.valid_from) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                warn!(
                    location_id = %taf_data.location_id,
                    valid_from = %taf_data.valid_from,
                    error = %e,
                    "Failed to parse TAF valid_from time, skipping"
                );
                continue;
            }
        };

        let valid_to = match chrono::DateTime::parse_from_rfc3339(&taf_data.valid_to) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(e) => {
                warn!(
                    location_id = %taf_data.location_id,
                    valid_to = %taf_data.valid_to,
                    error = %e,
                    "Failed to parse TAF valid_to time, skipping"
                );
                continue;
            }
        };

        // Build location
        let name = taf_data
            .name
            .clone()
            .unwrap_or_else(|| taf_data.location_id.clone());
        let mut location = Location::new(
            taf_data.location_id.clone(),
            name,
            taf_data.longitude,
            taf_data.latitude,
        );

        if let Some(elev) = taf_data.elevation_m {
            location = location.with_elevation(elev);
        }
        location = location.with_type("airport");

        locations_to_upsert.push(location);

        // Build TafForecast
        let forecast = TafForecast {
            id: None,
            location_id: taf_data.location_id.clone(),
            issue_time,
            valid_from,
            valid_to,
            raw_taf: taf_data.raw_taf.clone(),
            remarks: None,
        };

        // Build TafPeriods
        let mut periods = Vec::new();
        for period_data in &taf_data.periods {
            let period_from = match chrono::DateTime::parse_from_rfc3339(&period_data.period_from) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(e) => {
                    warn!(
                        location_id = %taf_data.location_id,
                        period_from = %period_data.period_from,
                        error = %e,
                        "Failed to parse TAF period_from time, skipping period"
                    );
                    continue;
                }
            };

            let period_to = match chrono::DateTime::parse_from_rfc3339(&period_data.period_to) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(e) => {
                    warn!(
                        location_id = %taf_data.location_id,
                        period_to = %period_data.period_to,
                        error = %e,
                        "Failed to parse TAF period_to time, skipping period"
                    );
                    continue;
                }
            };

            periods.push(TafPeriod {
                id: None,
                period_from,
                period_to,
                change_indicator: period_data.change_indicator.clone(),
                probability: period_data.probability,
                wind_direction_deg: period_data.wind_direction_deg,
                wind_speed_ms: period_data.wind_speed_ms,
                wind_gust_ms: period_data.wind_gust_ms,
                visibility_m: period_data.visibility_m,
                wx_string: period_data.wx_string.clone(),
                cloud_layers: period_data.cloud_layers.clone(),
            });
        }

        // Upsert TAF
        match catalog.upsert_taf(&forecast, &periods).await {
            Ok(_) => {
                tafs_ingested += 1;
            }
            Err(e) => {
                warn!(
                    location_id = %taf_data.location_id,
                    error = %e,
                    "Failed to upsert TAF"
                );
                errors.push(format!("{}: {}", taf_data.location_id, e));
            }
        }
    }

    // Upsert locations
    let locations_updated = match catalog.upsert_locations(&locations_to_upsert).await {
        Ok(count) => count,
        Err(e) => {
            error!(error = %e, "Failed to upsert locations");
            0
        }
    };

    let message = if errors.is_empty() {
        format!(
            "Ingested {} TAFs, updated {} locations",
            tafs_ingested, locations_updated
        )
    } else {
        format!(
            "Ingested {} TAFs, updated {} locations ({} errors)",
            tafs_ingested,
            locations_updated,
            errors.len()
        )
    };

    info!(
        source = %request.source,
        tafs_ingested = tafs_ingested,
        locations_updated = locations_updated,
        errors = errors.len(),
        "TAF ingestion completed"
    );

    (
        StatusCode::OK,
        Json(TafIngestResponse {
            success: errors.is_empty(),
            message,
            tafs_ingested,
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
        .route("/ingest/tafs", post(ingest_tafs_handler))
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // IngestionTracker tests
    // =========================================================================

    #[tokio::test]
    async fn test_tracker_new() {
        let tracker = IngestionTracker::new();
        let status = tracker.get_status().await;
        assert!(status.active.is_empty());
        assert!(status.recent.is_empty());
        assert_eq!(status.total_completed, 0);
    }

    #[tokio::test]
    async fn test_tracker_start() {
        let tracker = IngestionTracker::new();
        tracker.start("test-id", "/path/to/file.grib2").await;

        let status = tracker.get_status().await;
        assert_eq!(status.active.len(), 1);
        assert_eq!(status.active[0].id, "test-id");
        assert_eq!(status.active[0].file_path, "/path/to/file.grib2");
        assert_eq!(status.active[0].status, "processing");
    }

    #[tokio::test]
    async fn test_tracker_complete_success() {
        let tracker = IngestionTracker::new();
        tracker.start("test-id", "/path/to/file.grib2").await;

        tracker
            .complete(
                "test-id",
                true,
                5,
                vec!["TMP".to_string(), "UGRD".to_string()],
                None,
            )
            .await;

        let status = tracker.get_status().await;
        assert!(status.active.is_empty());
        assert_eq!(status.recent.len(), 1);
        assert_eq!(status.recent[0].id, "test-id");
        assert!(status.recent[0].success);
        assert_eq!(status.recent[0].datasets_registered, 5);
        assert_eq!(status.recent[0].parameters.len(), 2);
        assert!(status.recent[0].error_message.is_none());
    }

    #[tokio::test]
    async fn test_tracker_complete_failure() {
        let tracker = IngestionTracker::new();
        tracker.start("test-id", "/path/to/file.grib2").await;

        tracker
            .complete("test-id", false, 0, vec![], Some("Parse error".to_string()))
            .await;

        let status = tracker.get_status().await;
        assert!(status.active.is_empty());
        assert_eq!(status.recent.len(), 1);
        assert!(!status.recent[0].success);
        assert_eq!(status.recent[0].datasets_registered, 0);
        assert_eq!(
            status.recent[0].error_message,
            Some("Parse error".to_string())
        );
    }

    #[tokio::test]
    async fn test_tracker_multiple_ingestions() {
        let tracker = IngestionTracker::new();

        // Start multiple ingestions
        tracker.start("id-1", "/file1.grib2").await;
        tracker.start("id-2", "/file2.grib2").await;
        tracker.start("id-3", "/file3.grib2").await;

        let status = tracker.get_status().await;
        assert_eq!(status.active.len(), 3);

        // Complete some
        tracker
            .complete("id-1", true, 3, vec!["A".into()], None)
            .await;
        tracker
            .complete("id-2", false, 0, vec![], Some("err".into()))
            .await;

        let status = tracker.get_status().await;
        assert_eq!(status.active.len(), 1); // id-3 still active
        assert_eq!(status.recent.len(), 2);
    }

    #[tokio::test]
    async fn test_tracker_max_completed_limit() {
        let tracker = IngestionTracker::new();

        // Complete more than max_completed (100)
        for i in 0..150 {
            let id = format!("id-{}", i);
            tracker.start(&id, &format!("/file{}.grib2", i)).await;
            tracker.complete(&id, true, 1, vec![], None).await;
        }

        let status = tracker.get_status().await;
        // Should be capped at max_completed (100)
        assert_eq!(status.total_completed, 100);
    }

    #[tokio::test]
    async fn test_tracker_complete_nonexistent() {
        let tracker = IngestionTracker::new();

        // Try to complete an ingestion that was never started
        tracker.complete("nonexistent", true, 1, vec![], None).await;

        let status = tracker.get_status().await;
        assert!(status.active.is_empty());
        assert!(status.recent.is_empty()); // Should not add anything
    }

    #[tokio::test]
    async fn test_tracker_duration_calculation() {
        let tracker = IngestionTracker::new();
        tracker.start("test-id", "/file.grib2").await;

        // Small delay to ensure duration > 0
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        tracker.complete("test-id", true, 1, vec![], None).await;

        let status = tracker.get_status().await;
        assert!(status.recent[0].duration_ms >= 10);
    }

    // =========================================================================
    // IngestResponse tests
    // =========================================================================

    #[test]
    fn test_ingest_response_from_result() {
        let result = IngestionResult {
            datasets_registered: 10,
            model: "gfs".to_string(),
            reference_time: Utc::now(),
            parameters: vec!["TMP".to_string(), "UGRD".to_string(), "VGRD".to_string()],
            bytes_written: 1024 * 1024,
        };

        let response = IngestResponse::from(result);
        assert!(response.success);
        assert_eq!(response.datasets_registered, 10);
        assert_eq!(response.model, Some("gfs".to_string()));
        assert!(response.reference_time.is_some());
        assert_eq!(response.parameters.len(), 3);
        assert!(response.message.contains("10 datasets"));
    }

    // =========================================================================
    // Request/Response struct tests
    // =========================================================================

    #[test]
    fn test_ingest_request_deserialize() {
        let json = r#"{
            "file_path": "/data/gfs.grib2",
            "source_url": "https://nomads.ncep.noaa.gov/gfs.grib2",
            "model": "gfs",
            "forecast_hour": 6
        }"#;

        let request: IngestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.file_path, "/data/gfs.grib2");
        assert_eq!(request.model, Some("gfs".to_string()));
        assert_eq!(request.forecast_hour, Some(6));
    }

    #[test]
    fn test_ingest_request_deserialize_minimal() {
        let json = r#"{"file_path": "/data/file.grib2"}"#;

        let request: IngestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.file_path, "/data/file.grib2");
        assert!(request.model.is_none());
        assert!(request.forecast_hour.is_none());
    }

    #[test]
    fn test_observation_ingest_request_deserialize() {
        let json = r#"{
            "source": "metar",
            "observations": [
                {
                    "location_id": "KJFK",
                    "longitude": -73.7789,
                    "latitude": 40.6397,
                    "obs_time": "2024-01-15T12:00:00Z",
                    "temperature_k": 280.0
                }
            ]
        }"#;

        let request: ObservationIngestRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.source, "metar");
        assert_eq!(request.observations.len(), 1);
        assert_eq!(request.observations[0].location_id, "KJFK");
        assert_eq!(request.observations[0].temperature_k, Some(280.0));
    }

    #[test]
    fn test_health_response_serialize() {
        let response = HealthResponse {
            status: "ok".to_string(),
            service: "ingester".to_string(),
            version: "0.1.0".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"service\":\"ingester\""));
    }

    #[test]
    fn test_status_response_serialize() {
        let response = StatusResponse {
            active: vec![ActiveIngestion {
                id: "test".to_string(),
                file_path: "/file.grib2".to_string(),
                started_at: Utc::now(),
                status: "processing".to_string(),
            }],
            recent: vec![],
            total_completed: 0,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"active\""));
        assert!(json.contains("\"recent\""));
        assert!(json.contains("\"total_completed\""));
    }

    #[test]
    fn test_active_ingestion_serialize() {
        let ingestion = ActiveIngestion {
            id: "abc-123".to_string(),
            file_path: "/data/test.grib2".to_string(),
            started_at: Utc::now(),
            status: "processing".to_string(),
        };

        let json = serde_json::to_string(&ingestion).unwrap();
        assert!(json.contains("\"id\":\"abc-123\""));
        assert!(json.contains("\"status\":\"processing\""));
    }

    #[test]
    fn test_completed_ingestion_serialize() {
        let now = Utc::now();
        let completed = CompletedIngestion {
            id: "xyz-789".to_string(),
            file_path: "/data/test.grib2".to_string(),
            started_at: now,
            completed_at: now,
            duration_ms: 1500,
            success: true,
            datasets_registered: 5,
            parameters: vec!["TMP".to_string()],
            error_message: None,
        };

        let json = serde_json::to_string(&completed).unwrap();
        assert!(json.contains("\"duration_ms\":1500"));
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"datasets_registered\":5"));
    }
}
