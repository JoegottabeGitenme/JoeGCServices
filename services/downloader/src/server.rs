//! HTTP server for download status and management API.
//!
//! Provides endpoints for:
//! - Overall download status and statistics
//! - Recent and active downloads list
//! - Download schedule information
//! - Time series data for charts
//! - Prometheus metrics (including cleanup metrics)

use std::sync::Arc;

use axum::{
    extract::{Extension, Query, Request},
    http::{header::HeaderName, StatusCode},
    middleware::{from_fn, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::cleanup::CleanupMetrics;
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
    pub cleanup_metrics: Arc<CleanupMetrics>,
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
        .route(
            "/retry",
            post(retry_handler).route_layer(from_fn(require_admin_header)),
        )
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
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

/// Require `X-Admin-Request: 1` on state-changing requests (see wms-api `admin_guard`).
///
/// A custom header forces a CORS preflight, which a cross-site page cannot pass.
async fn require_admin_header(req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(HeaderName::from_static("x-admin-request"))
        .is_some_and(|v| v.as_bytes() == b"1");
    if !ok {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "missing X-Admin-Request header" })),
        )
            .into_response();
    }
    next.run(req).await
}

/// POST /retry?url=... - Retry a failed download (requires `X-Admin-Request: 1`)
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

/// GET /metrics - Prometheus metrics endpoint
async fn metrics_handler(Extension(state): Extension<Arc<ServerState>>) -> impl IntoResponse {
    let ds = &state.download_state;

    // Get download stats
    let stats = ds.get_stats().await.unwrap_or_default();

    // Build Prometheus metrics output
    let mut output = String::new();

    // Download stats
    output.push_str("# HELP downloader_downloads_pending Number of pending downloads\n");
    output.push_str("# TYPE downloader_downloads_pending gauge\n");
    output.push_str(&format!("downloader_downloads_pending {}\n", stats.pending));

    output.push_str("# HELP downloader_downloads_in_progress Number of downloads in progress\n");
    output.push_str("# TYPE downloader_downloads_in_progress gauge\n");
    output.push_str(&format!(
        "downloader_downloads_in_progress {}\n",
        stats.in_progress
    ));

    output.push_str("# HELP downloader_downloads_failed Number of failed downloads\n");
    output.push_str("# TYPE downloader_downloads_failed gauge\n");
    output.push_str(&format!("downloader_downloads_failed {}\n", stats.failed));

    output.push_str("# HELP downloader_downloads_completed_total Total completed downloads\n");
    output.push_str("# TYPE downloader_downloads_completed_total counter\n");
    output.push_str(&format!(
        "downloader_downloads_completed_total {}\n",
        stats.completed
    ));

    output.push_str("# HELP downloader_bytes_downloaded_total Total bytes downloaded\n");
    output.push_str("# TYPE downloader_bytes_downloaded_total counter\n");
    output.push_str(&format!(
        "downloader_bytes_downloaded_total {}\n",
        stats.total_bytes_downloaded
    ));

    // Per-model download counts (skipped if the state database query fails)
    if let Ok(counts) = ds.get_model_download_counts().await {
        output.push_str(&format_model_downloads(&counts));
    }

    // Next expected run per enabled model, from the configured schedule
    output.push_str(&format_model_next_runs(
        &state.model_schedules,
        chrono::Utc::now(),
    ));

    // Cleanup metrics
    output.push_str(&state.cleanup_metrics.to_prometheus());

    // Service info
    output.push_str("# HELP downloader_info Downloader service information\n");
    output.push_str("# TYPE downloader_info gauge\n");
    output.push_str(&format!(
        "downloader_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    ));

    (
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        output,
    )
}

// ============================================================================
// Helpers
// ============================================================================

/// Escape a Prometheus label value (backslash, double quote, newline).
fn escape_label_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Render `downloader_model_downloads{model,status}` (a gauge: `completed` is the current
/// row count of `completed_downloads`, which cleanup prunes, exactly like the global
/// `downloader_downloads_completed_total`). Rows are sorted for stable output.
fn format_model_downloads(counts: &[crate::state::ModelDownloadCount]) -> String {
    let mut rows: Vec<&crate::state::ModelDownloadCount> = counts.iter().collect();
    rows.sort_by(|a, b| (&a.model, &a.status).cmp(&(&b.model, &b.status)));
    let mut out = String::from(
        "# HELP downloader_model_downloads Downloads per model and status (completed, failed, pending, in_progress, retrying)\n\
         # TYPE downloader_model_downloads gauge\n",
    );
    for r in rows {
        out.push_str(&format!(
            "downloader_model_downloads{{model=\"{}\",status=\"{}\"}} {}\n",
            escape_label_value(&r.model),
            escape_label_value(&r.status),
            r.count
        ));
    }
    out
}

/// Earliest moment strictly after `now` at which a new cycle of this schedule should be
/// available: a cycle hour (UTC) plus `delay_hours`. `None` if the schedule has no cycles.
fn next_expected_run(
    schedule: &crate::scheduler::ModelSchedule,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Duration, TimeZone, Utc};
    let today = now.date_naive();
    let mut best: Option<chrono::DateTime<Utc>> = None;
    // Yesterday's late cycles can still become available today once the delay is added.
    for day_offset in -1i64..=2 {
        let day = today + Duration::days(day_offset);
        for &cycle in &schedule.cycles {
            if cycle > 23 {
                continue;
            }
            let Some(naive) = day.and_hms_opt(cycle, 0, 0) else {
                continue;
            };
            let available =
                Utc.from_utc_datetime(&naive) + Duration::hours(schedule.delay_hours as i64);
            if available > now && best.is_none_or(|b| available < b) {
                best = Some(available);
            }
        }
    }
    best
}

/// Render `downloader_model_next_run_timestamp_seconds{model}` for enabled models.
fn format_model_next_runs(
    schedules: &[crate::scheduler::ModelSchedule],
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let mut out = String::from(
        "# HELP downloader_model_next_run_timestamp_seconds Unix time the next model cycle is expected to be available (cycle time + delay)\n\
         # TYPE downloader_model_next_run_timestamp_seconds gauge\n",
    );
    for m in schedules.iter().filter(|m| m.enabled) {
        if let Some(t) = next_expected_run(m, now) {
            out.push_str(&format!(
                "downloader_model_next_run_timestamp_seconds{{model=\"{}\"}} {}\n",
                escape_label_value(&m.id),
                t.timestamp()
            ));
        }
    }
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn schedule(
        id: &str,
        enabled: bool,
        cycles: Vec<u32>,
        delay_hours: u32,
    ) -> crate::scheduler::ModelSchedule {
        crate::scheduler::ModelSchedule {
            id: id.to_string(),
            name: id.to_string(),
            enabled,
            cycles,
            delay_hours,
            poll_interval_secs: 60,
            bucket: String::new(),
            prefix_template: String::new(),
            file_pattern: String::new(),
            forecast_hours: Vec::new(),
            is_observation: false,
            max_concurrent: 1,
        }
    }

    #[test]
    fn next_run_is_cycle_plus_delay_after_now() {
        use chrono::TimeZone;
        let s = schedule("gfs", true, vec![0, 6, 12, 18], 4);
        // 05:00 -> 00Z cycle is available 04:00 (past); next is 06Z+4h = 10:00
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 13, 5, 0, 0).unwrap();
        assert_eq!(
            next_expected_run(&s, now),
            Some(
                chrono::Utc
                    .with_ymd_and_hms(2026, 11, 13, 10, 0, 0)
                    .unwrap()
            )
        );
        // 23:00 -> 18Z+4h = 22:00 past; next is 00Z+4h tomorrow
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 11, 13, 23, 0, 0)
            .unwrap();
        assert_eq!(
            next_expected_run(&s, now),
            Some(chrono::Utc.with_ymd_and_hms(2026, 11, 14, 4, 0, 0).unwrap())
        );
        // A delay that crosses midnight: 18Z+8h = 02:00 next day
        let s = schedule("x", true, vec![18], 8);
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 13, 1, 0, 0).unwrap();
        assert_eq!(
            next_expected_run(&s, now),
            Some(chrono::Utc.with_ymd_and_hms(2026, 11, 13, 2, 0, 0).unwrap())
        );
        assert_eq!(
            next_expected_run(&schedule("o", true, vec![], 0), now),
            None
        );
    }

    #[test]
    fn next_run_series_skips_disabled_and_escapes() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 13, 5, 0, 0).unwrap();
        let out = format_model_next_runs(
            &[
                schedule("gfs", true, vec![6], 0),
                schedule("off", false, vec![6], 0),
                schedule("q\"x", true, vec![6], 0),
            ],
            now,
        );
        assert!(out.contains("downloader_model_next_run_timestamp_seconds{model=\"gfs\"} "));
        assert!(!out.contains("model=\"off\""));
        assert!(out.contains("model=\"q\\\"x\""));
        assert_eq!(
            out.matches("# TYPE downloader_model_next_run_timestamp_seconds gauge")
                .count(),
            1
        );
    }

    #[test]
    fn model_downloads_series_is_sorted_and_escaped() {
        use crate::state::ModelDownloadCount;
        let c = |m: &str, s: &str, n| ModelDownloadCount {
            model: m.into(),
            status: s.into(),
            count: n,
        };
        let out = format_model_downloads(&[
            c("hrrr", "pending", 2),
            c("gfs", "completed", 9),
            c("a\\b", "failed", 1),
        ]);
        let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            lines,
            vec![
                "downloader_model_downloads{model=\"a\\\\b\",status=\"failed\"} 1",
                "downloader_model_downloads{model=\"gfs\",status=\"completed\"} 9",
                "downloader_model_downloads{model=\"hrrr\",status=\"pending\"} 2",
            ]
        );
        assert_eq!(out.matches("# HELP downloader_model_downloads ").count(), 1);
    }

    async fn router() -> Router {
        let state = Arc::new(ServerState {
            download_state: Arc::new(DownloadState::open_memory().await.unwrap()),
            model_schedules: Vec::new(),
            cleanup_metrics: Arc::new(CleanupMetrics::new()),
        });
        create_router(state)
    }

    fn req(method: &str, header: Option<&str>) -> axum::http::Request<Body> {
        let mut b = axum::http::Request::builder()
            .method(method)
            .uri("/retry?url=http://x/a");
        if let Some(h) = header {
            b = b.header("X-Admin-Request", h);
        }
        b.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn retry_post_without_header_is_forbidden() {
        let resp = router().await.oneshot(req("POST", None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "missing X-Admin-Request header");
    }

    #[tokio::test]
    async fn retry_post_with_header_is_handled() {
        let resp = router()
            .await
            .oneshot(req("POST", Some("1")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn retry_get_is_not_allowed() {
        let resp = router().await.oneshot(req("GET", Some("1"))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
