//! NWS alerts ingest endpoints and retention.
//!
//! * `POST /ingest/nws-alerts`: one poll of `api.weather.gov/alerts/active`
//!   (`features`), or a `not_modified` / `error` report. The answer carries
//!   `missing_zones`, the zone keys (`forecast/KSZ001`) the downloader should fetch.
//! * `POST /ingest/nws-zones`: fetched zone geometries; fills in the geometry of
//!   the alerts that use them.
//! * [`run_retention`]: deletes alerts that ended more than 7 days ago.

use std::sync::Arc;
use std::time::Duration;

use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use storage::nws_alerts::{parse_feature, ZoneRecord, ZoneStatus};
use storage::NwsAlertCatalog;
use tracing::{error, info, warn};

use crate::obs_retention::resolve_hours;
use crate::server::ServerState;

pub const RETENTION_ENV: &str = "NWS_ALERTS_RETENTION_HOURS";
pub const RETENTION_DEFAULT_HOURS: u64 = 168;
/// Phase B serves up to a week of history; never keep less than a day.
pub const RETENTION_FLOOR_HOURS: u64 = 24;
const SWEEP_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
pub struct NwsAlertsIngestRequest {
    /// The feed's `features`, verbatim.
    #[serde(default)]
    pub features: Vec<serde_json::Value>,
    /// The feed's own `updated` stamp (RFC 3339).
    #[serde(default)]
    pub feed_updated: Option<String>,
    /// The poll returned HTTP 304: nothing changed.
    #[serde(default)]
    pub not_modified: bool,
    /// The poll failed; stored alerts are left untouched.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct NwsAlertsIngestResponse {
    pub success: bool,
    pub message: String,
    pub received: usize,
    pub parse_errors: usize,
    pub superseded: u64,
    pub ended: u64,
    pub missing_zones: Vec<String>,
}

fn fail(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<NwsAlertsIngestResponse>) {
    (
        status,
        Json(NwsAlertsIngestResponse {
            success: false,
            message: msg.into(),
            ..Default::default()
        }),
    )
}

/// POST /ingest/nws-alerts
pub async fn ingest_nws_alerts_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Json(req): Json<NwsAlertsIngestRequest>,
) -> impl IntoResponse {
    let Some(catalog) = &state.nws_alert_catalog else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "NWS alert catalog not configured",
        );
    };
    let now = Utc::now();

    if let Some(err) = &req.error {
        warn!(error = %err, "NWS alerts poll failed; keeping last good state");
        return match catalog.record_poll_failure(err, now).await {
            Ok(()) => (
                StatusCode::OK,
                Json(NwsAlertsIngestResponse {
                    success: true,
                    message: "Poll failure recorded".into(),
                    ..Default::default()
                }),
            ),
            Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }

    if req.not_modified {
        return match catalog.record_not_modified(now).await {
            Ok(missing) => (
                StatusCode::OK,
                Json(NwsAlertsIngestResponse {
                    success: true,
                    message: "Feed unchanged".into(),
                    missing_zones: missing,
                    ..Default::default()
                }),
            ),
            Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }

    let mut alerts = Vec::with_capacity(req.features.len());
    let mut parse_errors = 0usize;
    for f in &req.features {
        match parse_feature(f) {
            Ok(a) => alerts.push(a),
            Err(e) => {
                parse_errors += 1;
                warn!(error = %e, "Skipping unparseable NWS alert");
            }
        }
    }
    if alerts.is_empty() {
        // NWS always has alerts: nothing usable is a fault, never "all clear".
        let msg = format!(
            "feed had no usable alerts ({} features, {} unparseable)",
            req.features.len(),
            parse_errors
        );
        let _ = catalog.record_poll_failure(&msg, now).await;
        return fail(StatusCode::UNPROCESSABLE_ENTITY, msg);
    }

    let feed_updated = req
        .feed_updated
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));

    match catalog.apply_feed(&alerts, feed_updated, now).await {
        Ok(r) => {
            info!(
                received = r.received,
                superseded = r.superseded,
                ended = r.ended,
                missing_zones = r.missing_zones.len(),
                parse_errors,
                "NWS alerts applied"
            );
            (
                StatusCode::OK,
                Json(NwsAlertsIngestResponse {
                    success: true,
                    message: format!("Applied {} alerts", r.received),
                    received: r.received,
                    parse_errors,
                    superseded: r.superseded,
                    ended: r.ended,
                    missing_zones: r.missing_zones,
                }),
            )
        }
        Err(e) => {
            error!(error = %e, "NWS alerts apply failed");
            let _ = catalog.record_poll_failure(&e.to_string(), now).await;
            fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NwsZonesIngestRequest {
    pub zones: Vec<NwsZoneData>,
}

#[derive(Debug, Deserialize)]
pub struct NwsZoneData {
    /// `forecast/KSZ001`.
    pub zone_key: String,
    /// `ok` (default) | `not_found` | `no_geometry`.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    /// GeoJSON geometry object.
    #[serde(default)]
    pub geometry: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct NwsZonesIngestResponse {
    pub success: bool,
    pub message: String,
    pub zones_stored: usize,
    pub alerts_updated: u64,
}

fn valid_zone_key(k: &str) -> bool {
    match k.split_once('/') {
        Some((t, id)) => {
            !t.is_empty()
                && !id.is_empty()
                && t.chars().all(|c| c.is_ascii_lowercase())
                && id.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// POST /ingest/nws-zones
pub async fn ingest_nws_zones_handler(
    Extension(state): Extension<Arc<ServerState>>,
    Json(req): Json<NwsZonesIngestRequest>,
) -> impl IntoResponse {
    let Some(catalog) = &state.nws_alert_catalog else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(NwsZonesIngestResponse {
                success: false,
                message: "NWS alert catalog not configured".into(),
                zones_stored: 0,
                alerts_updated: 0,
            }),
        );
    };
    let zones: Vec<ZoneRecord> = req
        .zones
        .into_iter()
        .filter(|z| {
            let ok = valid_zone_key(&z.zone_key);
            if !ok {
                warn!(zone_key = %z.zone_key, "Skipping zone with malformed key");
            }
            ok
        })
        .map(|z| {
            let geometry_json = z.geometry.filter(|g| g.is_object()).map(|g| g.to_string());
            let status = match z.status.as_deref() {
                Some("not_found") => ZoneStatus::NotFound,
                Some("no_geometry") => ZoneStatus::NoGeometry,
                _ if geometry_json.is_none() => ZoneStatus::NoGeometry,
                _ => ZoneStatus::Ok,
            };
            ZoneRecord {
                zone_key: z.zone_key,
                name: z.name,
                state: z.state,
                kind: z.kind,
                status,
                geometry_json,
            }
        })
        .collect();

    match catalog.upsert_zones(&zones, Utc::now()).await {
        Ok(updated) => (
            StatusCode::OK,
            Json(NwsZonesIngestResponse {
                success: true,
                message: format!("Stored {} zones", zones.len()),
                zones_stored: zones.len(),
                alerts_updated: updated,
            }),
        ),
        Err(e) => {
            error!(error = %e, "NWS zone ingest failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(NwsZonesIngestResponse {
                    success: false,
                    message: e.to_string(),
                    zones_stored: 0,
                    alerts_updated: 0,
                }),
            )
        }
    }
}

/// Retention window (hours) from a lookup function.
pub fn retention_hours_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> u64 {
    let (hours, warning) = resolve_hours(
        lookup(RETENTION_ENV).as_deref(),
        RETENTION_DEFAULT_HOURS,
        RETENTION_FLOOR_HOURS,
    );
    if let Some(w) = warning {
        warn!(env = RETENTION_ENV, "{w}");
    }
    hours
}

pub fn retention_hours_from_env() -> u64 {
    retention_hours_from_lookup(|k| std::env::var(k).ok())
}

/// Sweep forever; the first sweep runs at startup.
pub async fn run_retention(catalog: NwsAlertCatalog, retention_hours: u64) {
    info!(retention_hours, "NWS alerts retention enabled");
    let mut tick = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        tick.tick().await;
        let cutoff = Utc::now() - chrono::Duration::hours(retention_hours as i64);
        match catalog.delete_retired_before(cutoff).await {
            Ok(0) => {}
            Ok(n) => info!(deleted = n, %cutoff, "NWS alerts retention sweep"),
            Err(e) => warn!(error = %e, "NWS alerts retention sweep failed; will retry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn retention_defaults_to_seven_days() {
        assert_eq!(retention_hours_from_lookup(env(&[])), 168);
    }

    #[test]
    fn retention_floor_and_garbage() {
        assert_eq!(
            retention_hours_from_lookup(env(&[(RETENTION_ENV, "3")])),
            24
        );
        assert_eq!(
            retention_hours_from_lookup(env(&[(RETENTION_ENV, "72")])),
            72
        );
        assert_eq!(
            retention_hours_from_lookup(env(&[(RETENTION_ENV, "0")])),
            168
        );
        assert_eq!(
            retention_hours_from_lookup(env(&[(RETENTION_ENV, "abc")])),
            168
        );
    }

    #[test]
    fn zone_key_validation() {
        assert!(valid_zone_key("forecast/KSZ001"));
        assert!(valid_zone_key("county/KSC045"));
        assert!(!valid_zone_key("KSZ001"));
        assert!(!valid_zone_key("forecast/"));
        assert!(!valid_zone_key("forecast/KS Z"));
        assert!(!valid_zone_key("../etc"));
    }

    // --- HTTP-level test against a real PostGIS (Docker) -------------------------

    use crate::server::build_router;
    use axum::body::Body;
    use axum::http::Request;
    use ingestion::Ingester;
    use storage::{Catalog, ObjectStorage, ObjectStorageConfig};
    use tower::ServiceExt;

    const FIXTURE: &str = include_str!("../../downloader/tests/fixtures/nws_alerts_active.json");

    async fn post(
        router: &axum::Router,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let resp = router
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    /// The downloader's whole conversation with the ingester: a feed, the zones it
    /// asks for, a 304, an error report, and an empty feed that must be refused.
    ///
    /// ```text
    /// cargo test -p ingester -- --ignored nws_alerts::tests::http
    /// ```
    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn http_feed_then_zones_then_not_modified_then_errors() {
        let infra = test_utils::containers::TestInfrastructure::start().await;
        let catalog = Catalog::connect(&infra.postgres_url()).await.unwrap();
        catalog.migrate().await.unwrap();
        catalog.migrate_observations().await.unwrap();
        catalog.migrate_nws_alerts().await.unwrap();
        let storage = Arc::new(
            ObjectStorage::new(&ObjectStorageConfig {
                endpoint: "http://127.0.0.1:1".to_string(),
                bucket: "unused".to_string(),
                access_key_id: "x".to_string(),
                secret_access_key: "x".to_string(),
                region: "us-east-1".to_string(),
                allow_http: true,
            })
            .unwrap(),
        );
        let nws = NwsAlertCatalog::new(catalog.pool_clone());
        let state = Arc::new(ServerState {
            ingester: Ingester::new(storage, catalog.clone()),
            observation_catalog: None,
            storm_event_catalog: None,
            trail_report_catalog: None,
            lightning_catalog: None,
            nws_alert_catalog: Some(nws.clone()),
            tracker: crate::server::IngestionTracker::new(),
        });
        let router = build_router(state);

        let feed: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let n = feed["features"].as_array().unwrap().len();

        // 1. The feed: stored at once; the zones it lacks come back.
        let (st, body) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({
                "features": feed["features"], "feed_updated": feed["updated"]
            }),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["received"], n);
        assert_eq!(body["parse_errors"], 0);
        let missing: Vec<String> = body["missing_zones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(missing.contains(&"forecast/COZ068".to_string()));

        // 2. A zone arrives (and one 404s): geometry fills in.
        let (st, body) = post(&router, "/ingest/nws-zones", serde_json::json!({"zones": [
            {"zone_key": "forecast/COZ068", "status": "ok", "name": "Z", "kind": "public",
             "geometry": {"type": "Polygon", "coordinates": [[[-107.5,37.0],[-106.5,37.0],[-106.5,38.0],[-107.5,38.0],[-107.5,37.0]]]}},
            {"zone_key": "county/MDC031", "status": "not_found"},
            {"zone_key": "../bad", "status": "ok"}
        ]})).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["zones_stored"], 2, "malformed key skipped");
        let hit = nws
            .active_at_point(
                -107.0,
                37.5,
                chrono::Utc
                    .with_ymd_and_hms(2026, 11, 1, 23, 30, 0)
                    .unwrap(),
                false,
            )
            .await
            .unwrap();
        assert!(hit.iter().any(|a| a.event == "Winter Storm Warning"));

        // 3. 304: success, and the still-missing zones are offered again.
        let (st, body) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({"not_modified": true}),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let again = body["missing_zones"].as_array().unwrap();
        assert!(!again.is_empty() && !again.iter().any(|v| v == "forecast/COZ068"));

        // 4. An error report only touches status.
        let (st, _) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({"error": "GET returned 503"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let status = nws.status().await.unwrap().unwrap();
        assert_eq!(status.last_error.as_deref(), Some("GET returned 503"));
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM nws_alerts WHERE ended_at IS NULL AND superseded_by IS NULL",
        )
        .fetch_one(catalog.pool())
        .await
        .unwrap();
        assert!(live > 0);

        // 5. An empty or unusable feed is refused and ends nothing.
        let (st, _) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({"features": []}),
        )
        .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
        let (st, _) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({"features": [{"properties": {}}]}),
        )
        .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
        let live2: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM nws_alerts WHERE ended_at IS NULL AND superseded_by IS NULL",
        )
        .fetch_one(catalog.pool())
        .await
        .unwrap();
        assert_eq!(live, live2);
    }
}
