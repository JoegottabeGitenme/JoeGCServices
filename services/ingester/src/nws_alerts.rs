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
    /// Alerts stored without their own polygon because the database rejected it.
    pub geometry_rejected: usize,
    /// Alerts the database rejected outright (skipped).
    pub failed: usize,
    /// The feed was older than the one already applied: counted as a successful poll, not applied.
    pub stale_ignored: bool,
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
                geometry_rejected = r.geometry_rejected,
                failed = r.failed,
                geometry_failed = r.geometry_failed,
                stale_ignored = r.stale_ignored,
                "NWS alerts applied"
            );
            (
                StatusCode::OK,
                Json(NwsAlertsIngestResponse {
                    success: true,
                    message: if r.stale_ignored {
                        format!("Ignored a stale feed of {} alerts", r.received)
                    } else {
                        format!("Applied {} alerts", r.received)
                    },
                    received: r.received,
                    parse_errors,
                    superseded: r.superseded,
                    ended: r.ended,
                    missing_zones: r.missing_zones,
                    geometry_rejected: r.geometry_rejected,
                    failed: r.failed,
                    stale_ignored: r.stale_ignored,
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
    /// `ok` (default) | `not_found` | `no_geometry` | `error` (the fetch failed transiently).
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
    /// Zones whose geometry the database rejected (stored without it).
    pub zones_invalid_geometry: usize,
    /// Zones the database rejected outright (skipped; they will be asked for again).
    pub zones_failed: usize,
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

/// The stored status of a posted zone: what the downloader said, else judged by whether it
/// brought a geometry. Unknown words are treated as absent.
fn zone_status(status: Option<&str>, has_geometry: bool) -> ZoneStatus {
    match status {
        Some("error") => ZoneStatus::Error,
        Some("not_found") => ZoneStatus::NotFound,
        Some("no_geometry") => ZoneStatus::NoGeometry,
        _ if !has_geometry => ZoneStatus::NoGeometry,
        _ => ZoneStatus::Ok,
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
                zones_invalid_geometry: 0,
                zones_failed: 0,
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
            let status = zone_status(z.status.as_deref(), geometry_json.is_some());
            // A failed fetch carries no data; whatever else came with it is not trusted.
            let geometry_json = geometry_json.filter(|_| status != ZoneStatus::Error);
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
        Ok(r) => {
            if r.invalid_geometry > 0 || r.failed > 0 || r.alerts_failed > 0 {
                warn!(
                    invalid_geometry = r.invalid_geometry,
                    failed = r.failed,
                    alerts_failed = r.alerts_failed,
                    "NWS zones stored with problems"
                );
            }
            (
                StatusCode::OK,
                Json(NwsZonesIngestResponse {
                    success: true,
                    message: format!("Stored {} zones", r.stored),
                    zones_stored: r.stored,
                    alerts_updated: r.alerts_updated,
                    zones_invalid_geometry: r.invalid_geometry,
                    zones_failed: r.failed,
                }),
            )
        }
        Err(e) => {
            error!(error = %e, "NWS zone ingest failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(NwsZonesIngestResponse {
                    success: false,
                    message: e.to_string(),
                    zones_stored: 0,
                    alerts_updated: 0,
                    zones_invalid_geometry: 0,
                    zones_failed: 0,
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
    fn posted_zone_status_mapping() {
        assert_eq!(zone_status(Some("error"), false), ZoneStatus::Error);
        assert_eq!(
            zone_status(Some("error"), true),
            ZoneStatus::Error,
            "a failed fetch is an error whatever else was sent"
        );
        assert_eq!(zone_status(Some("not_found"), false), ZoneStatus::NotFound);
        assert_eq!(
            zone_status(Some("no_geometry"), false),
            ZoneStatus::NoGeometry
        );
        assert_eq!(zone_status(Some("ok"), true), ZoneStatus::Ok);
        assert_eq!(zone_status(None, true), ZoneStatus::Ok);
        assert_eq!(zone_status(Some("ok"), false), ZoneStatus::NoGeometry);
        assert_eq!(zone_status(Some("weird"), true), ZoneStatus::Ok);
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

    /// The router over a real PostGIS: the Docker test containers, or the database in
    /// `NWS_TEST_DATABASE_URL` when set.
    async fn setup() -> (
        axum::Router,
        NwsAlertCatalog,
        Catalog,
        Option<test_utils::containers::TestInfrastructure>,
    ) {
        let (url, infra) = match std::env::var("NWS_TEST_DATABASE_URL") {
            Ok(url) => (url, None),
            Err(_) => {
                let infra = test_utils::containers::TestInfrastructure::start().await;
                (infra.postgres_url(), Some(infra))
            }
        };
        let catalog = Catalog::connect(&url).await.unwrap();
        catalog.migrate().await.unwrap();
        catalog.migrate_observations().await.unwrap();
        catalog.migrate_nws_alerts().await.unwrap();
        sqlx::query("TRUNCATE nws_alerts, nws_zones, nws_alerts_status")
            .execute(catalog.pool())
            .await
            .unwrap();
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
        (build_router(state), nws, catalog, infra)
    }

    /// One alert whose polygon makes PostGIS raise, and a zone batch with one such zone, are
    /// stored around, not refused: the answer is 200 (so the downloader keeps its ETag and goes on
    /// to the next zone batch).
    ///
    /// ```text
    /// cargo test -p ingester --bin ingester -- --ignored nws_alerts::tests::http_bad
    /// ```
    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn http_bad_geometry_in_a_feed_or_zone_batch_is_stored_around() {
        let (router, nws, _catalog, _infra) = setup().await;
        let feed: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let mut features = feed["features"].as_array().unwrap().clone();
        let n = features.len();
        let mut bad = features
            .iter()
            .find(|f| f["properties"]["event"] == "Severe Thunderstorm Warning")
            .unwrap()
            .clone();
        bad["properties"]["id"] = serde_json::json!("urn:test:bad-polygon");
        bad["geometry"] = serde_json::json!({"type": "Polygon", "coordinates": "not an array"});
        features.push(bad);

        let (st, body) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({"features": features, "feed_updated": feed["updated"]}),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["received"], n + 1);
        assert_eq!(body["geometry_rejected"], 1);
        assert_eq!(body["failed"], 0);
        let status = nws.status().await.unwrap().unwrap();
        assert!(status.last_success_at.is_some() && status.last_error.is_none());
        assert_eq!(status.last_alert_count, Some(n as i32 + 1));
        let stored = nws
            .get("urn:test:bad-polygon", false)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.ended_at.is_none());

        let (st, body) = post(&router, "/ingest/nws-zones", serde_json::json!({"zones": [
            {"zone_key": "forecast/COZ068", "status": "ok",
             "geometry": {"type": "Polygon", "coordinates": [[[-107.5,37.0],[-106.5,37.0],[-106.5,38.0],[-107.5,38.0],[-107.5,37.0]]]}},
            {"zone_key": "county/MDC031", "status": "ok",
             "geometry": {"type": "Polygon", "coordinates": "not an array"}},
        ]})).await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["zones_stored"], 2);
        assert_eq!(body["zones_invalid_geometry"], 1);
        assert_eq!(body["zones_failed"], 0);
        assert!(body["alerts_updated"].as_u64().unwrap() >= 1);
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
        let (router, nws, catalog, _infra) = setup().await;

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

        // 1b. A feed older than the one just applied (a lagging CDN node) is acknowledged as a
        // poll but applied to nothing: the alerts it no longer lists are not ended.
        let fewer: Vec<_> = feed["features"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .cloned()
            .collect();
        let (st, body) = post(
            &router,
            "/ingest/nws-alerts",
            serde_json::json!({"features": fewer, "feed_updated": "2026-11-01T23:20:00Z"}),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["stale_ignored"], true);
        assert_eq!(
            (body["ended"].as_u64(), body["superseded"].as_u64()),
            (Some(0), Some(0))
        );

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

        // 2b. A zone whose fetch failed (5xx / timeout) is recorded as an error: stored without
        // geometry, and not offered again by the next poll (it waits its retry delay).
        assert!(missing.contains(&"forecast/PZZ455".to_string()));
        let (st, body) = post(
            &router,
            "/ingest/nws-zones",
            serde_json::json!({"zones": [{"zone_key": "forecast/PZZ455", "status": "error"}]}),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert_eq!(body["zones_stored"], 1);
        let st: String = sqlx::query_scalar(
            "SELECT fetch_status FROM nws_zones WHERE zone_key = 'forecast/PZZ455'",
        )
        .fetch_one(catalog.pool())
        .await
        .unwrap();
        assert_eq!(st, "error");

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
        assert!(
            !again.iter().any(|v| v == "forecast/PZZ455"),
            "a zone that just failed is not asked for again at once"
        );

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
