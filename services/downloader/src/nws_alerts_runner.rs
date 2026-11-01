//! NWS alerts runner.
//!
//! Polls `GET {base_url}/alerts/active` (GeoJSON, every watch/warning/advisory in
//! force across all NWS areas including AK/HI/PR/Guam/marine) and hands each
//! answer to the ingester, which owns all state:
//!
//! * `POST /ingest/nws-alerts` with the raw features (or `not_modified` on a 304,
//!   or `error` when the poll failed). The ingester answers with
//!   `missing_zones`: zone keys (`forecast/KSZ001`) it has no geometry for.
//! * Those zones are fetched by a **separate task** (`GET {base_url}/zones/{key}`,
//!   bounded concurrency, a pause after each request) and posted to
//!   `POST /ingest/nws-zones` in small batches, so alerts keep flowing at the poll
//!   cadence while the ~800-zone cold start fills in over a minute or two.
//!
//! The runner holds only the `ETag` and the in-memory zone queue; a restart loses
//! nothing (the ingester recomputes `missing_zones` on the next poll).
//!
//! NWS requires a descriptive `User-Agent` with contact details; it is configurable.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::{header, Client, StatusCode};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};
use tracing::{debug, error, info, warn};

const DEFAULT_BASE_URL: &str = "https://api.weather.gov";
const DEFAULT_USER_AGENT: &str = "FolkWeather (folkweather.com, support@folkweather.com)";
const ACCEPT_GEOJSON: &str = "application/geo+json";
/// Zones posted to the ingester per request.
const ZONE_POST_BATCH: usize = 25;
/// How long the zone worker sleeps when nothing nudges it.
const ZONE_IDLE_WAKE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct NwsAlertsConfig {
    pub id: String,
    /// `https://api.weather.gov` (no trailing slash); overridable for tests.
    pub base_url: String,
    pub user_agent: String,
    pub poll_interval_secs: u64,
    pub ingester_url: String,
    /// Zone fetches in flight at once.
    pub zone_concurrency: usize,
    /// Pause after each zone request, per lane.
    pub zone_request_delay_ms: u64,
    pub request_timeout_secs: u64,
}

/// Zone keys waiting for a fetch, deduplicated; keys being fetched right now are
/// not re-queued by the next poll.
#[derive(Default)]
struct ZoneQueue {
    state: Mutex<(BTreeSet<String>, BTreeSet<String>)>, // (pending, inflight)
    notify: Notify,
}

impl ZoneQueue {
    fn add(&self, keys: impl IntoIterator<Item = String>) {
        let mut s = self.state.lock().unwrap();
        let mut added = false;
        for k in keys {
            if !s.1.contains(&k) {
                added |= s.0.insert(k);
            }
        }
        drop(s);
        if added {
            self.notify.notify_one();
        }
    }

    /// Move everything pending to in-flight and return it.
    fn take(&self) -> Vec<String> {
        let mut s = self.state.lock().unwrap();
        let taken: Vec<String> = std::mem::take(&mut s.0).into_iter().collect();
        s.1.extend(taken.iter().cloned());
        taken
    }

    fn done(&self, keys: &[String]) {
        let mut s = self.state.lock().unwrap();
        for k in keys {
            s.1.remove(k);
        }
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.state.lock().unwrap().0.len()
    }
}

pub struct NwsAlertsRunner {
    config: NwsAlertsConfig,
    client: Client,
    etag: Mutex<Option<String>>,
    zones: ZoneQueue,
}

impl NwsAlertsRunner {
    pub fn new(config: NwsAlertsConfig) -> Result<Arc<Self>> {
        let client = Client::builder()
            .user_agent(config.user_agent.clone())
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Arc::new(Self {
            config,
            client,
            etag: Mutex::new(None),
            zones: ZoneQueue::default(),
        }))
    }

    /// Poll forever, with the zone worker alongside, until shutdown.
    pub async fn run_forever(self: Arc<Self>, shutdown: broadcast::Receiver<()>) -> Result<()> {
        info!(
            source = "nws_alerts",
            poll_interval_secs = self.config.poll_interval_secs,
            zone_concurrency = self.config.zone_concurrency,
            "Starting NWS alerts runner"
        );
        let zone_worker = {
            let me = Arc::clone(&self);
            let rx = shutdown.resubscribe();
            tokio::spawn(async move { me.zone_loop(rx).await })
        };

        let mut shutdown = shutdown;
        let interval = Duration::from_secs(self.config.poll_interval_secs);
        loop {
            if let Err(e) = self.poll_once().await {
                warn!(source = "nws_alerts", error = %e, "NWS alerts poll failed; keeping last good state");
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown.recv() => {
                    info!(source = "nws_alerts", "NWS alerts runner shutting down");
                    break;
                }
            }
        }
        zone_worker.abort();
        Ok(())
    }

    fn ingester_endpoint(&self, path: &str) -> String {
        let base = self.config.ingester_url.trim_end_matches("/ingest");
        format!("{}{}", base.trim_end_matches('/'), path)
    }

    async fn post_ingester(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .client
            .post(self.ingester_endpoint(path))
            .json(body)
            .send()
            .await
            .with_context(|| format!("Failed to POST {path} to ingester"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ingester returned {status} for {path}: {text}");
        }
        resp.json::<Value>()
            .await
            .with_context(|| format!("Bad JSON from ingester for {path}"))
    }

    /// Tell the ingester the poll failed (best effort), then return the error.
    async fn report_failure(&self, msg: String) -> Result<()> {
        if let Err(e) = self
            .post_ingester("/ingest/nws-alerts", &json!({ "error": msg }))
            .await
        {
            debug!(source = "nws_alerts", error = %e, "Could not report poll failure to ingester");
        }
        anyhow::bail!(msg)
    }

    fn queue_missing(&self, answer: &Value) {
        let keys: Vec<String> = answer
            .get("missing_zones")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        if !keys.is_empty() {
            debug!(source = "nws_alerts", count = keys.len(), "Zones to fetch");
            self.zones.add(keys);
        }
    }

    /// One poll of the active feed.
    pub async fn poll_once(&self) -> Result<()> {
        let url = format!("{}/alerts/active", self.config.base_url);
        let mut req = self.client.get(&url).header(header::ACCEPT, ACCEPT_GEOJSON);
        if let Some(tag) = self.etag.lock().unwrap().clone() {
            req = req.header(header::IF_NONE_MATCH, tag);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return self.report_failure(format!("GET {url} failed: {e}")).await,
        };

        match resp.status() {
            StatusCode::NOT_MODIFIED => {
                let answer = self
                    .post_ingester("/ingest/nws-alerts", &json!({ "not_modified": true }))
                    .await?;
                self.queue_missing(&answer);
                debug!(source = "nws_alerts", "Alerts unchanged (304)");
                Ok(())
            }
            StatusCode::OK => {
                let new_etag = resp
                    .headers()
                    .get(header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                let body: Value = match resp.json().await {
                    Ok(b) => b,
                    Err(e) => return self.report_failure(format!("Bad alerts JSON: {e}")).await,
                };
                let features = body
                    .get("features")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if features.is_empty() {
                    // NWS always has alerts; an empty answer must not end everything.
                    return self
                        .report_failure("alerts feed had no features".into())
                        .await;
                }
                let count = features.len();
                let answer = self
                    .post_ingester(
                        "/ingest/nws-alerts",
                        &json!({ "features": features, "feed_updated": body.get("updated") }),
                    )
                    .await?;
                // Only remember the ETag once the ingester has the data: otherwise a
                // failed ingest would turn the retry into a 304 and lose the update.
                *self.etag.lock().unwrap() = new_etag;
                self.queue_missing(&answer);
                let n = |k: &str| answer.get(k).and_then(Value::as_u64).unwrap_or(0);
                let (ended, superseded) = (n("ended"), n("superseded"));
                let missing_zones = answer
                    .get("missing_zones")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                info!(
                    source = "nws_alerts",
                    count, ended, superseded, missing_zones, "NWS alerts ingested"
                );
                Ok(())
            }
            other => {
                self.report_failure(format!("GET {url} returned {other}"))
                    .await
            }
        }
    }

    // --- zones ---------------------------------------------------------------

    async fn zone_loop(self: Arc<Self>, mut shutdown: broadcast::Receiver<()>) {
        loop {
            tokio::select! {
                _ = self.zones.notify.notified() => {}
                _ = tokio::time::sleep(ZONE_IDLE_WAKE) => {}
                _ = shutdown.recv() => return,
            }
            if let Err(e) = self.process_zone_queue().await {
                error!(source = "nws_alerts", error = %e, "Zone fill failed");
            }
        }
    }

    /// Fetch and post everything currently queued. Returns zones posted.
    pub async fn process_zone_queue(&self) -> Result<usize> {
        let keys = self.zones.take();
        if keys.is_empty() {
            return Ok(0);
        }
        info!(
            source = "nws_alerts",
            zones = keys.len(),
            "Fetching NWS zones"
        );
        let mut posted = 0usize;
        let mut result = Ok(());
        for group in keys.chunks(ZONE_POST_BATCH) {
            let fetched: Vec<Option<Value>> = stream::iter(group.iter().cloned())
                .map(|k| async move { self.fetch_zone(&k).await })
                .buffer_unordered(self.config.zone_concurrency.max(1))
                .collect()
                .await;
            let zones: Vec<Value> = fetched.into_iter().flatten().collect();
            if !zones.is_empty() {
                match self
                    .post_ingester("/ingest/nws-zones", &json!({ "zones": zones }))
                    .await
                {
                    Ok(_) => posted += zones.len(),
                    Err(e) => {
                        result = Err(e);
                        self.zones.done(group);
                        break;
                    }
                }
            }
            self.zones.done(group);
        }
        // Anything not reached (error mid-way) is simply asked for again by the next poll.
        self.zones.done(&keys);
        result.map(|_| posted)
    }

    /// `None` = transient failure (retried on a later poll); `Some` = a result to store.
    async fn fetch_zone(&self, key: &str) -> Option<Value> {
        let url = format!("{}/zones/{}", self.config.base_url, key);
        let out = match self
            .client
            .get(&url)
            .header(header::ACCEPT, ACCEPT_GEOJSON)
            .send()
            .await
        {
            Err(e) => {
                warn!(source = "nws_alerts", zone = key, error = %e, "Zone fetch failed");
                None
            }
            Ok(resp) => match resp.status() {
                StatusCode::OK => match resp.json::<Value>().await {
                    Ok(f) => {
                        let p = f.get("properties").unwrap_or(&Value::Null);
                        let geometry = f.get("geometry").filter(|g| g.is_object()).cloned();
                        Some(json!({
                            "zone_key": key,
                            "status": if geometry.is_some() { "ok" } else { "no_geometry" },
                            "name": p.get("name"),
                            "state": p.get("state"),
                            "kind": p.get("type"),
                            "geometry": geometry,
                        }))
                    }
                    Err(e) => {
                        warn!(source = "nws_alerts", zone = key, error = %e, "Bad zone JSON");
                        None
                    }
                },
                StatusCode::NOT_FOUND => {
                    debug!(source = "nws_alerts", zone = key, "Zone not found");
                    Some(json!({ "zone_key": key, "status": "not_found" }))
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    warn!(
                        source = "nws_alerts",
                        zone = key,
                        "Rate limited by NWS; backing off"
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    None
                }
                s => {
                    warn!(source = "nws_alerts", zone = key, status = %s, "Zone fetch returned an error status");
                    None
                }
            },
        };
        if self.config.zone_request_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.config.zone_request_delay_ms)).await;
        }
        out
    }
}

/// Load from a model YAML; `None` unless `source.type: nws_alerts`.
pub fn load_nws_alerts_config(
    config_path: &std::path::Path,
    ingester_url: &str,
) -> Result<Option<NwsAlertsConfig>> {
    use crate::config::ModelConfig;

    let mc = ModelConfig::load(config_path)?;
    if mc.source.source_type != "nws_alerts" {
        return Ok(None);
    }
    Ok(Some(NwsAlertsConfig {
        id: mc.model.id,
        base_url: mc
            .source
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string(),
        user_agent: mc
            .source
            .user_agent
            .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string()),
        poll_interval_secs: mc.schedule.poll_interval_secs.max(10),
        ingester_url: ingester_url.to_string(),
        zone_concurrency: mc.source.zone_fetch_concurrency.unwrap_or(4),
        zone_request_delay_ms: mc.source.zone_request_delay_ms.unwrap_or(100),
        request_timeout_secs: 60,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header as h, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const FIXTURE: &str = include_str!("../tests/fixtures/nws_alerts_active.json");

    fn cfg(nws: &MockServer, ingester: &MockServer) -> NwsAlertsConfig {
        NwsAlertsConfig {
            id: "nws-alerts".into(),
            base_url: nws.uri(),
            user_agent: "FolkWeather-test (test@example.com)".into(),
            poll_interval_secs: 60,
            ingester_url: ingester.uri(),
            zone_concurrency: 3,
            zone_request_delay_ms: 0,
            request_timeout_secs: 5,
        }
    }

    fn posted_json(reqs: &[Request], p: &str) -> Vec<Value> {
        reqs.iter()
            .filter(|r| r.url.path() == p)
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    }

    async fn ingester_ok(missing: &[&str]) -> MockServer {
        let ing = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ingest/nws-alerts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true, "ended": 2, "superseded": 1, "missing_zones": missing
            })))
            .mount(&ing)
            .await;
        Mock::given(method("POST"))
            .and(path("/ingest/nws-zones"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&ing)
            .await;
        ing
    }

    #[tokio::test]
    async fn polls_with_user_agent_forwards_features_and_uses_etag() {
        let nws = MockServer::start().await;
        let ing = ingester_ok(&["forecast/COZ068", "county/MDC031"]).await;
        // 304 when the ETag is presented, the feed otherwise
        Mock::given(method("GET"))
            .and(path("/alerts/active"))
            .and(h("if-none-match", "\"abc\""))
            .respond_with(ResponseTemplate::new(304))
            .with_priority(1)
            .mount(&nws)
            .await;
        Mock::given(method("GET"))
            .and(path("/alerts/active"))
            .and(h("user-agent", "FolkWeather-test (test@example.com)"))
            .and(h("accept", "application/geo+json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"abc\"")
                    .set_body_raw(FIXTURE, "application/geo+json"),
            )
            .with_priority(2)
            .mount(&nws)
            .await;

        let runner = NwsAlertsRunner::new(cfg(&nws, &ing)).unwrap();
        runner.poll_once().await.unwrap();
        runner.poll_once().await.unwrap(); // second poll: 304

        let posts = posted_json(
            &ing.received_requests().await.unwrap(),
            "/ingest/nws-alerts",
        );
        assert_eq!(posts.len(), 2);
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(
            posts[0]["features"].as_array().unwrap().len(),
            fixture["features"].as_array().unwrap().len()
        );
        assert_eq!(posts[0]["feed_updated"], fixture["updated"]);
        assert_eq!(posts[1], json!({"not_modified": true}));
        assert_eq!(
            runner.zones.pending_len(),
            2,
            "missing zones are queued (deduplicated)"
        );
    }

    #[tokio::test]
    async fn a_failed_poll_is_reported_and_never_sends_features() {
        let nws = MockServer::start().await;
        let ing = ingester_ok(&[]).await;
        Mock::given(method("GET"))
            .and(path("/alerts/active"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&nws)
            .await;
        let runner = NwsAlertsRunner::new(cfg(&nws, &ing)).unwrap();
        assert!(runner.poll_once().await.is_err());
        let posts = posted_json(
            &ing.received_requests().await.unwrap(),
            "/ingest/nws-alerts",
        );
        assert_eq!(posts.len(), 1);
        assert!(posts[0]["error"].as_str().unwrap().contains("503"));
        assert!(posts[0].get("features").is_none());
    }

    #[tokio::test]
    async fn an_empty_feed_is_a_failure_not_an_all_clear() {
        let nws = MockServer::start().await;
        let ing = ingester_ok(&[]).await;
        Mock::given(method("GET"))
            .and(path("/alerts/active"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"type":"FeatureCollection","features":[]})),
            )
            .mount(&nws)
            .await;
        let runner = NwsAlertsRunner::new(cfg(&nws, &ing)).unwrap();
        assert!(runner.poll_once().await.is_err());
        let posts = posted_json(
            &ing.received_requests().await.unwrap(),
            "/ingest/nws-alerts",
        );
        assert!(posts[0].get("features").is_none() && posts[0]["error"].is_string());
    }

    #[tokio::test]
    async fn a_rejected_ingest_does_not_store_the_etag() {
        let nws = MockServer::start().await;
        let ing = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&ing)
            .await;
        Mock::given(method("GET"))
            .and(path("/alerts/active"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"abc\"")
                    .set_body_raw(FIXTURE, "application/geo+json"),
            )
            .mount(&nws)
            .await;
        let runner = NwsAlertsRunner::new(cfg(&nws, &ing)).unwrap();
        assert!(runner.poll_once().await.is_err());
        assert!(runner.etag.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn zones_are_fetched_posted_and_failures_are_handled() {
        let nws = MockServer::start().await;
        let ing = ingester_ok(&[]).await;
        let square = json!({"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[-1.0,-1.0],[1.0,-1.0],[1.0,1.0],[-1.0,1.0],[-1.0,-1.0]]]},
            "properties":{"id":"COZ068","name":"Test Zone","state":"CO","type":"public"}});
        Mock::given(method("GET"))
            .and(path("/zones/forecast/COZ068"))
            .respond_with(ResponseTemplate::new(200).set_body_json(square))
            .mount(&nws)
            .await;
        Mock::given(method("GET"))
            .and(path("/zones/county/GONE01"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&nws)
            .await;
        Mock::given(method("GET"))
            .and(path("/zones/forecast/FLAKY1"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&nws)
            .await;
        Mock::given(method("GET"))
            .and(path("/zones/forecast/NOGEO1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"type":"Feature","geometry":null,"properties":{"name":"x"}}),
                ),
            )
            .mount(&nws)
            .await;

        let runner = NwsAlertsRunner::new(cfg(&nws, &ing)).unwrap();
        runner.zones.add(
            [
                "forecast/COZ068",
                "county/GONE01",
                "forecast/FLAKY1",
                "forecast/NOGEO1",
            ]
            .map(String::from),
        );
        let posted = runner.process_zone_queue().await.unwrap();
        assert_eq!(posted, 3, "the 500 is left for a later poll");

        let zone_posts = posted_json(&ing.received_requests().await.unwrap(), "/ingest/nws-zones");
        let zones: Vec<&Value> = zone_posts
            .iter()
            .flat_map(|p| p["zones"].as_array().unwrap())
            .collect();
        let by = |k: &str| zones.iter().find(|z| z["zone_key"] == k).copied().unwrap();
        assert_eq!(by("forecast/COZ068")["status"], "ok");
        assert_eq!(by("forecast/COZ068")["name"], "Test Zone");
        assert_eq!(by("forecast/COZ068")["kind"], "public");
        assert!(by("forecast/COZ068")["geometry"].is_object());
        assert_eq!(by("county/GONE01")["status"], "not_found");
        assert_eq!(by("forecast/NOGEO1")["status"], "no_geometry");
        // in-flight bookkeeping is cleared, so a later poll can ask for the flaky one again
        runner.zones.add(["forecast/FLAKY1".to_string()]);
        assert_eq!(runner.zones.pending_len(), 1);
    }

    #[test]
    fn config_loads_from_yaml_and_ignores_other_sources() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nws.yaml");
        std::fs::write(&p, include_str!("../../../config/models/nws-alerts.yaml")).unwrap();
        let c = load_nws_alerts_config(&p, "http://ingester:8082")
            .unwrap()
            .unwrap();
        assert_eq!(c.id, "nws-alerts");
        assert_eq!(c.base_url, "https://api.weather.gov");
        assert_eq!(c.poll_interval_secs, 60);
        assert_eq!(c.zone_concurrency, 4);
        assert!(c.user_agent.contains("folkweather"));
        let other = dir.path().join("storm.yaml");
        std::fs::write(
            &other,
            include_str!("../../../config/models/storm-events.yaml"),
        )
        .unwrap();
        assert!(load_nws_alerts_config(&other, "x").unwrap().is_none());
    }
}
