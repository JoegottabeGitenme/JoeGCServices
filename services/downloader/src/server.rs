//! HTTP server for download status and management API.
//!
//! Provides endpoints for:
//! - Overall download status and statistics
//! - Recent and active downloads list
//! - Download schedule information
//! - Time series data for charts

use std::sync::Arc;

use axum::{
    extract::{Extension, Query},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::scheduler::ModelSchedule;
use crate::state::{DownloadState, DownloadStatus};

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct StatusResponse {
    pub service: String,
    pub status: String,
    pub stats: DownloadStatsResponse,
    pub active_downloads: Vec<ActiveDownload>,
    pub recent_completed: Vec<CompletedDownloadResponse>,
    pub pending_ingestion: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadStatsResponse {
    pub pending: u64,
    pub in_progress: u64,
    pub failed: u64,
    pub completed: u64,
    pub total_bytes_downloaded: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveDownload {
    pub url: String,
    pub filename: String,
    pub model: Option<String>,
    pub status: String,
    pub progress_percent: Option<f64>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub retry_count: u32,
    pub started_at: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletedDownloadResponse {
    pub url: String,
    pub filename: String,
    pub model: Option<String>,
    pub total_bytes: Option<u64>,
    pub completed_at: String,
    pub ingested: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadsListResponse {
    pub active: Vec<ActiveDownload>,
    pub pending: Vec<ActiveDownload>,
    pub failed: Vec<ActiveDownload>,
    pub recent_completed: Vec<CompletedDownloadResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduleResponse {
    pub models: Vec<ModelScheduleInfo>,
    pub next_checks: Vec<NextCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelScheduleInfo {
    pub id: String,
    pub enabled: bool,
    pub bucket: String,
    pub cycles: Vec<u32>,
    pub delay_hours: u32,
    pub poll_interval_secs: u64,
    pub forecast_hours: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NextCheck {
    pub model: String,
    pub next_cycle: String,
    pub expected_available: String,
    pub files_expected: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeSeriesResponse {
    pub hourly: Vec<HourlyDataPoint>,
    pub total_downloads_24h: u64,
    pub total_bytes_24h: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HourlyDataPoint {
    pub hour: String,
    pub download_count: u64,
    pub bytes_downloaded: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetryResponse {
    pub success: bool,
    pub message: String,
}

// ============================================================================
// Query Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DownloadsQuery {
    pub limit: Option<usize>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TimeSeriesQuery {
    pub hours: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct RetryQuery {
    pub url: String,
}

// ============================================================================
// Shared State
// ============================================================================

pub struct ServerState {
    pub download_state: Arc<DownloadState>,
    pub model_schedules: Vec<ModelSchedule>,
}

// ============================================================================
// Router
// ============================================================================

/// Create the status API router.
pub fn create_router(state: Arc<ServerState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/status", get(status_handler))
        .route("/downloads", get(downloads_handler))
        .route("/schedule", get(schedule_handler))
        .route("/timeseries", get(timeseries_handler))
        .route("/retry", get(retry_handler))
        .route("/health", get(health_handler))
        .layer(cors)
        .layer(Extension(state))
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /status - Overall download service status
async fn status_handler(Extension(state): Extension<Arc<ServerState>>) -> impl IntoResponse {
    let ds = &state.download_state;

    // Get stats
    let stats = match ds.get_stats().await {
        Ok(s) => DownloadStatsResponse {
            pending: s.pending,
            in_progress: s.in_progress,
            failed: s.failed,
            completed: s.completed,
            total_bytes_downloaded: s.total_bytes_downloaded,
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    // Get active downloads
    let active = match ds.get_in_progress().await {
        Ok(records) => records
            .into_iter()
            .map(|r| ActiveDownload {
                url: r.url,
                filename: r.filename,
                model: r.model,
                status: status_to_string(r.status),
                progress_percent: r.total_bytes.map(|t| {
                    if t > 0 {
                        (r.downloaded_bytes as f64 / t as f64) * 100.0
                    } else {
                        0.0
                    }
                }),
                downloaded_bytes: r.downloaded_bytes,
                total_bytes: r.total_bytes,
                retry_count: r.retry_count,
                started_at: r.created_at.to_rfc3339(),
                error_message: r.error_message,
            })
            .collect(),
        Err(_) => vec![],
    };

    // Get recent completed
    let recent = match ds.get_recent_completed(10).await {
        Ok(records) => records
            .into_iter()
            .map(|r| CompletedDownloadResponse {
                url: r.url,
                filename: r.filename,
                model: r.model,
                total_bytes: r.total_bytes,
                completed_at: r.completed_at.to_rfc3339(),
                ingested: r.ingested,
            })
            .collect(),
        Err(_) => vec![],
    };

    // Get pending ingestion count
    let pending_ingestion = ds
        .get_pending_ingestion()
        .await
        .map(|v| v.len())
        .unwrap_or(0);

    let response = StatusResponse {
        service: "downloader".to_string(),
        status: if stats.in_progress > 0 {
            "downloading".to_string()
        } else if stats.pending > 0 {
            "pending".to_string()
        } else {
            "idle".to_string()
        },
        stats,
        active_downloads: active,
        recent_completed: recent,
        pending_ingestion,
    };

    Json(response).into_response()
}

/// GET /downloads - List all downloads with filtering
async fn downloads_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Query(params): Query<DownloadsQuery>,
) -> impl IntoResponse {
    let ds = &state.download_state;
    let limit = params.limit.unwrap_or(50);

    // Get active/in-progress
    let active: Vec<ActiveDownload> = ds
        .get_in_progress()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.status == DownloadStatus::InProgress)
        .filter(|r| {
            params
                .model
                .as_ref()
                .is_none_or(|m| r.model.as_ref() == Some(m))
        })
        .take(limit)
        .map(record_to_active)
        .collect();

    // Get pending
    let pending: Vec<ActiveDownload> = ds
        .get_pending()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            params
                .model
                .as_ref()
                .is_none_or(|m| r.model.as_ref() == Some(m))
        })
        .take(limit)
        .map(record_to_active)
        .collect();

    // Get failed
    let failed: Vec<ActiveDownload> = ds
        .get_failed()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            params
                .model
                .as_ref()
                .is_none_or(|m| r.model.as_ref() == Some(m))
        })
        .take(limit)
        .map(record_to_active)
        .collect();

    // Get recent completed
    let recent_completed: Vec<CompletedDownloadResponse> = ds
        .get_recent_completed(limit)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            params
                .model
                .as_ref()
                .is_none_or(|m| r.model.as_ref() == Some(m))
        })
        .map(|r| CompletedDownloadResponse {
            url: r.url,
            filename: r.filename,
            model: r.model,
            total_bytes: r.total_bytes,
            completed_at: r.completed_at.to_rfc3339(),
            ingested: r.ingested,
        })
        .collect();

    Json(DownloadsListResponse {
        active,
        pending,
        failed,
        recent_completed,
    })
}

/// GET /schedule - Show download schedule and upcoming downloads
async fn schedule_handler(Extension(state): Extension<Arc<ServerState>>) -> impl IntoResponse {
    let models: Vec<ModelScheduleInfo> = state
        .model_schedules
        .iter()
        .map(|m| ModelScheduleInfo {
            id: m.id.clone(),
            enabled: m.enabled,
            bucket: m.bucket.clone(),
            cycles: m.cycles.clone(),
            delay_hours: m.delay_hours,
            poll_interval_secs: m.poll_interval_secs,
            forecast_hours: m.forecast_hours.clone(),
        })
        .collect();

    // Calculate next expected checks for each model
    let now = chrono::Utc::now();
    let next_checks: Vec<NextCheck> = state
        .model_schedules
        .iter()
        .filter(|m| m.enabled)
        .map(|m| {
            let current_hour = now.format("%H").to_string().parse::<u32>().unwrap_or(0);

            // Find next cycle
            let next_cycle = m
                .cycles
                .iter()
                .find(|&&c| c > current_hour)
                .copied()
                .unwrap_or_else(|| m.cycles.first().copied().unwrap_or(0));

            let next_cycle_str = format!("{:02}Z", next_cycle);

            // Calculate when data will be available
            let hours_until = if next_cycle > current_hour {
                next_cycle - current_hour
            } else {
                24 - current_hour + next_cycle
            };
            let available_in = hours_until + m.delay_hours;
            let expected_available = format!("~{} hours", available_in);

            NextCheck {
                model: m.id.clone(),
                next_cycle: next_cycle_str,
                expected_available,
                files_expected: m.forecast_hours.len(),
            }
        })
        .collect();

    Json(ScheduleResponse {
        models,
        next_checks,
    })
}

/// GET /timeseries - Get time series data for charts
async fn timeseries_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Query(params): Query<TimeSeriesQuery>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(24);
    let ds = &state.download_state;

    let hourly: Vec<HourlyDataPoint> = match ds.get_hourly_stats(hours).await {
        Ok(stats) => stats
            .into_iter()
            .map(|s| HourlyDataPoint {
                hour: s.hour,
                download_count: s.download_count,
                bytes_downloaded: s.bytes_downloaded,
            })
            .collect(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let total_downloads_24h: u64 = hourly.iter().map(|h| h.download_count).sum();
    let total_bytes_24h: u64 = hourly.iter().map(|h| h.bytes_downloaded).sum();

    Json(TimeSeriesResponse {
        hourly,
        total_downloads_24h,
        total_bytes_24h,
    })
    .into_response()
}

/// GET /retry?url=... - Retry a failed download
async fn retry_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Query(params): Query<RetryQuery>,
) -> impl IntoResponse {
    let ds = &state.download_state;

    match ds.retry_download(&params.url).await {
        Ok(true) => Json(RetryResponse {
            success: true,
            message: "Download queued for retry".to_string(),
        }),
        Ok(false) => Json(RetryResponse {
            success: false,
            message: "Download not found or not in failed state".to_string(),
        }),
        Err(e) => Json(RetryResponse {
            success: false,
            message: format!("Error: {}", e),
        }),
    }
}

/// GET /health - Health check endpoint
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "downloader"
    }))
}

// ============================================================================
// Helpers
// ============================================================================

fn status_to_string(status: DownloadStatus) -> String {
    match status {
        DownloadStatus::Pending => "pending".to_string(),
        DownloadStatus::InProgress => "in_progress".to_string(),
        DownloadStatus::Retrying => "retrying".to_string(),
        DownloadStatus::Completed => "completed".to_string(),
        DownloadStatus::Failed => "failed".to_string(),
    }
}

fn record_to_active(r: crate::state::DownloadRecord) -> ActiveDownload {
    ActiveDownload {
        url: r.url,
        filename: r.filename,
        model: r.model,
        status: status_to_string(r.status),
        progress_percent: r.total_bytes.map(|t| {
            if t > 0 {
                (r.downloaded_bytes as f64 / t as f64) * 100.0
            } else {
                0.0
            }
        }),
        downloaded_bytes: r.downloaded_bytes,
        total_bytes: r.total_bytes,
        retry_count: r.retry_count,
        started_at: r.created_at.to_rfc3339(),
        error_message: r.error_message,
    }
}

/// Start the HTTP server.
pub async fn run_server(state: Arc<ServerState>, port: u16) -> anyhow::Result<()> {
    let app = create_router(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

    info!(port = port, "Starting download status server");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
