//! End-to-end tests of the `nws-alerts` EDR collection: the REAL routes (position, area,
//! radius, items, items/{id}, collection document), the real `config/edr` directory and a real
//! PostGIS filled by `NwsAlertCatalog` from the downloader's fixture feed. `#[ignore]`d (Docker):
//!
//! ```text
//! cargo test -p edr-api --test nws_alerts -- --ignored --test-threads=1 --nocapture
//! ```
//! or against your own PostGIS: `NWS_TEST_DATABASE_URL=postgres://... `.
//!
//! The clock: handlers use the wall clock for "now", so queries pass `datetime=` to pin the
//! moment the fixture is about (2026-11-01T23:30Z). Freshness (`meta`) is about the wall clock and
//! is driven by writing the poll status directly.

use std::sync::Arc;

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;

use edr_api::{
    availability::AvailabilityCache, config::EdrConfig, handlers, location_cache::LocationCache,
    metrics::MetricsCollector, state::AppState,
};
use grid_processor::{GridDataService, MinioConfig};
use storage::nws_alerts::{parse_feature, ParsedAlert, ZoneRecord, ZoneStatus};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog, NwsAlertCatalog,
};

const FIXTURE: &str =
    include_str!("../../../services/downloader/tests/fixtures/nws_alerts_active.json");
const AT: &str = "2026-11-01T23:30:00Z";

struct Env {
    app: Router,
    catalog: Arc<Catalog>,
    nws: NwsAlertCatalog,
    alerts: Vec<ParsedAlert>,
    _infra: Option<test_utils::containers::TestInfrastructure>,
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 23, 30, 0).unwrap()
}

fn square(w: f64, s: f64, e: f64, n: f64) -> String {
    json!({"type": "Polygon", "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]})
        .to_string()
}

fn zone(key: &str, geom: String) -> ZoneRecord {
    ZoneRecord {
        zone_key: key.into(),
        name: Some(key.into()),
        state: None,
        kind: Some("public".into()),
        status: ZoneStatus::Ok,
        geometry_json: Some(geom),
    }
}

async fn env() -> Env {
    env_with_read_timeout(None).await
}

/// `env()` whose alert reads run under a database time limit, like the service's own.
async fn env_with_read_timeout(read_timeout: Option<std::time::Duration>) -> Env {
    let (url, minio, infra) = match std::env::var("NWS_TEST_DATABASE_URL") {
        Ok(u) => (u, "http://127.0.0.1:1".to_string(), None),
        Err(_) => {
            let infra = test_utils::containers::TestInfrastructure::start().await;
            (infra.postgres_url(), infra.minio_url(), Some(infra))
        }
    };
    std::env::set_var("CONFIG_DIR", "../../config");
    let catalog = Arc::new(Catalog::connect(&url).await.unwrap());
    catalog.migrate().await.unwrap();
    catalog.migrate_observations().await.unwrap();
    catalog.migrate_lightning().await.unwrap();
    catalog.migrate_linear_features().await.unwrap();
    catalog.migrate_segment_conditions().await.unwrap();
    catalog.migrate_nws_alerts().await.unwrap();
    sqlx::query("TRUNCATE nws_alerts, nws_zones, nws_alerts_status")
        .execute(catalog.pool())
        .await
        .unwrap();

    let grid_data_service = GridDataService::new(
        Arc::clone(&catalog),
        MinioConfig {
            endpoint: minio,
            bucket: "unused".into(),
            access_key_id: "x".into(),
            secret_access_key: "x".into(),
            region: "us-east-1".into(),
            allow_http: true,
        },
        64,
    )
    .unwrap();
    let state = Arc::new(AppState {
        observation_catalog: Arc::new(ObservationCatalog::new(catalog.pool_clone())),
        storm_event_catalog: Arc::new(StormEventCatalog::new(catalog.pool_clone())),
        linear_feature_catalog: Arc::new(LinearFeatureCatalog::new(catalog.pool_clone())),
        segment_conditions_catalog: Arc::new(SegmentConditionsCatalog::new(catalog.pool_clone())),
        lightning_catalog: Arc::new(LightningCatalog::new(catalog.pool_clone())),
        nws_alert_catalog: Arc::new(match read_timeout {
            Some(t) => NwsAlertCatalog::new(catalog.pool_clone()).with_read_timeout(t),
            None => NwsAlertCatalog::new(catalog.pool_clone()),
        }),
        collections_snapshot: Arc::new(edr_api::snapshot_cache::SnapshotCache::new(
            std::time::Duration::from_secs(60),
        )),
        catalog: Arc::clone(&catalog),
        grid_data_service,
        edr_config: Arc::new(RwLock::new(
            EdrConfig::load_from_dir("../../config/edr").unwrap(),
        )),
        base_url: "http://localhost:8083/edr".into(),
        location_cache: Arc::new(LocationCache::new(16, 60)),
        availability_cache: Arc::new(AvailabilityCache::new(60)),
        metrics: Arc::new(MetricsCollector::new()),
    });
    // The same routes main.rs registers for these paths.
    let app = Router::new()
        .route(
            "/edr/collections/:collection_id",
            get(handlers::collections::get_collection_handler),
        )
        .route(
            "/edr/collections/:collection_id/position",
            get(handlers::nws_alerts::position_route),
        )
        .route(
            "/edr/collections/:collection_id/area",
            get(handlers::nws_alerts::area_route),
        )
        .route(
            "/edr/collections/:collection_id/radius",
            get(handlers::nws_alerts::radius_route),
        )
        .route(
            "/edr/collections/:collection_id/items",
            get(handlers::items::items_handler),
        )
        .route(
            "/edr/collections/:collection_id/items/:feature_id",
            get(handlers::nws_alerts::item_route_handler),
        )
        .route(
            "/edr/collections/:collection_id/items/:feature_id/conditions",
            get(handlers::linear_features::trail_conditions_timeseries_handler),
        )
        .layer(Extension(state));

    let v: Value = serde_json::from_str(FIXTURE).unwrap();
    let alerts: Vec<ParsedAlert> = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| parse_feature(f).unwrap())
        .collect();
    Env {
        app,
        nws: NwsAlertCatalog::new(catalog.pool_clone()),
        catalog,
        alerts,
        _infra: infra,
    }
}

impl Env {
    /// Ingest the fixture as of 23:30Z, with the Colorado zone and one Maryland zone known.
    async fn load(&self) {
        self.nws
            .apply_feed(&self.alerts, Some(t0()), t0())
            .await
            .unwrap();
        self.nws
            .upsert_zones(
                &[
                    zone("forecast/COZ068", square(-107.5, 37.0, -106.5, 38.0)),
                    zone("county/MDC031", square(-77.3, 39.0, -76.8, 39.4)),
                ],
                t0(),
            )
            .await
            .unwrap();
    }

    fn id(&self, event: &str) -> String {
        self.alerts
            .iter()
            .find(|a| a.event == event)
            .unwrap()
            .id
            .clone()
    }

    async fn get(&self, uri: &str) -> (u16, axum::http::HeaderMap, Value, usize) {
        let resp = self
            .app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, b) = resp.into_parts();
        let bytes = axum::body::to_bytes(b, 64 << 20).await.unwrap();
        (
            parts.status.as_u16(),
            parts.headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            bytes.len(),
        )
    }

    /// A point inside the alert's own polygon.
    async fn inside(&self, id: &str) -> (f64, f64) {
        sqlx::query_as("SELECT ST_X(ST_PointOnSurface(geom)), ST_Y(ST_PointOnSurface(geom)) FROM nws_alerts WHERE id = $1")
            .bind(id)
            .fetch_one(self.catalog.pool())
            .await
            .unwrap()
    }
}

fn events(v: &Value) -> Vec<String> {
    v["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["event"].as_str().unwrap().to_string())
        .collect()
}

fn feature<'a>(v: &'a Value, event: &str) -> &'a Value {
    v["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["properties"]["event"] == event)
        .unwrap_or_else(|| panic!("no {event} in {:?}", events(v)))
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn position_finds_polygon_and_zone_alerts_worst_first() {
    let e = env().await;
    e.load().await;

    // inside the Severe Thunderstorm Warning's own polygon
    let svr = e.id("Severe Thunderstorm Warning");
    let (lon, lat) = e.inside(&svr).await;
    let (st, h, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords=POINT({lon}%20{lat})&datetime={AT}"
        ))
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(h["content-type"], "application/geo+json");
    assert_eq!(h["cache-control"], "public, max-age=60");
    let f = feature(&v, "Severe Thunderstorm Warning");
    assert_eq!(f["id"], svr.as_str());
    assert!(f["geometry"].is_null(), "geometry is opt-in on /position");
    assert_eq!(f["properties"]["status"], "active");
    assert_eq!(f["properties"]["significance"], "warning");
    assert_eq!(f["properties"]["color"], "#FFA500");
    assert_eq!(f["properties"]["summary"]["hazard"], "60 mph wind gusts.");
    assert_eq!(v["numberReturned"], v["features"].as_array().unwrap().len());

    // &geometry=true adds it
    let (_, _, g, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords={lon},{lat}&datetime={AT}&geometry=true"
        ))
        .await;
    assert!(feature(&g, "Severe Thunderstorm Warning")["geometry"]["type"].is_string());

    // a point in the Colorado zone: the zone-only Winter Storm Warning, "upcoming" at 23:30Z
    let uri =
        format!("/edr/collections/nws-alerts/position?coords=POINT(-107%2037.5)&datetime={AT}");
    let (_, _, v, _) = e.get(&uri).await;
    let w = feature(&v, "Winter Storm Warning");
    assert_eq!(w["properties"]["status"], "upcoming");
    assert_eq!(w["properties"]["onset"], "2026-11-02T00:00:00Z");
    assert_eq!(w["properties"]["ends"], "2026-11-02T18:00:00Z");
    assert_eq!(w["properties"]["geometry_source"], "zones");
    // worst first: Severe ones precede anything Unknown
    let sev: Vec<&str> = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["severity"].as_str().unwrap_or(""))
        .collect();
    let rank = |s: &str| {
        ["Extreme", "Severe", "Moderate", "Minor", "Unknown"]
            .iter()
            .position(|x| *x == s)
            .unwrap_or(9)
    };
    assert!(sev.windows(2).all(|p| rank(p[0]) <= rank(p[1])), "{sev:?}");

    // the same point, once the warning is over: gone
    let (_, _, later, _) = e
        .get("/edr/collections/nws-alerts/position?coords=POINT(-107%2037.5)&datetime=2026-11-02T19:00:00Z")
        .await;
    assert!(!events(&later).contains(&"Winter Storm Warning".to_string()));

    // outside everything: an empty, well-formed answer
    let (st, _, none, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords=POINT(0%200)&datetime={AT}"
        ))
        .await;
    assert_eq!(st, 200);
    assert_eq!(none["features"], json!([]));
    assert_eq!(none["numberReturned"], 0);
    assert!(none["meta"].is_object());
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn position_filters_and_validation() {
    let e = env().await;
    e.load().await;
    let base =
        format!("/edr/collections/nws-alerts/position?coords=POINT(-107%2037.5)&datetime={AT}");

    let (_, _, all, _) = e.get(&base).await;
    assert!(events(&all).len() >= 2);
    let (_, _, sev, _) = e.get(&format!("{base}&severity=extreme")).await;
    assert!(events(&sev).is_empty());
    let (_, _, sev, _) = e.get(&format!("{base}&severity=Severe,Extreme")).await;
    assert!(!events(&sev).is_empty());
    assert!(sev["features"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["properties"]["severity"] == "Severe"));
    let (_, _, ev, _) = e
        .get(&format!(
            "{base}&event=winter%20storm%20warning,Flood%20Watch"
        ))
        .await;
    let got = events(&ev);
    assert!(got.contains(&"Winter Storm Warning".to_string()));
    assert!(got
        .iter()
        .all(|x| x == "Winter Storm Warning" || x == "Flood Watch"));

    for bad in [
        "/edr/collections/nws-alerts/position".to_string(),
        "/edr/collections/nws-alerts/position?coords=POINT(500%2037)".to_string(),
        format!("{base}&geometry=maybe"),
        // multi-byte text in coords: a 400, never a panic (a euro sign is %E2%82%AC)
        "/edr/collections/nws-alerts/position?coords=1234%E2%82%AC".to_string(),
        "/edr/collections/nws-alerts/radius?coords=%E2%82%AC%E2%82%AC&within=5".to_string(),
        "/edr/collections/nws-alerts/area?coords=POLYG%E2%82%AC((1%201,2%201,2%202,1%201))".to_string(),
        "/edr/collections/nws-alerts/area?coords=POLYG%C3%96N((1%201,2%201,2%202,1%201))".to_string(),
        "/edr/collections/nws-alerts/position?coords=POINT(-107%2037.5)&datetime=2026-11-01T00:00:00Z/2026-11-02T00:00:00Z".to_string(),
        "/edr/collections/nws-alerts/position?coords=POINT(-107%2037.5)&datetime=tomorrow".to_string(),
    ] {
        let (st, _, v, _) = e.get(&bad).await;
        assert_eq!(st, 400, "{bad}: {v}");
    }
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn items_default_active_versus_history_and_geometry() {
    let e = env().await;
    e.load().await;
    let co = "bbox=-108,36.5,-106,38.5";

    let (st, h, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{co}&datetime={AT}"
        ))
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(h["cache-control"], "public, max-age=60");
    assert_eq!(v["limit"], 500);
    let w = feature(&v, "Winter Storm Warning");
    assert!(
        w["geometry"]["type"].is_string(),
        "items include geometry by default"
    );
    assert_eq!(v["numberReturned"], v["features"].as_array().unwrap().len());
    let (_, _, ng, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{co}&datetime={AT}&geometry=false"
        ))
        .await;
    assert!(feature(&ng, "Winter Storm Warning")["geometry"].is_null());

    // an alert the feed stops listing ends; active hides it, status=all + interval shows it
    let sps = e.id("Special Weather Statement");
    let later = t0() + Duration::minutes(1);
    let rest: Vec<ParsedAlert> = e.alerts.iter().filter(|a| a.id != sps).cloned().collect();
    e.nws.apply_feed(&rest, Some(later), later).await.unwrap();
    let after = "2026-11-01T23:31:00Z";
    let sps_spot = e.inside(&sps).await;
    let bb = format!(
        "bbox={},{},{},{}",
        sps_spot.0 - 1.0,
        sps_spot.1 - 1.0,
        sps_spot.0 + 1.0,
        sps_spot.1 + 1.0
    );

    let (_, _, act, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{bb}&datetime={after}"
        ))
        .await;
    assert!(!events(&act).contains(&"Special Weather Statement".to_string()));
    let hist = format!(
        "/edr/collections/nws-alerts/items?{bb}&status=all&datetime=2026-11-01T20:00:00Z/{after}"
    );
    let (_, _, all, _) = e.get(&hist).await;
    assert_eq!(
        feature(&all, "Special Weather Statement")["properties"]["status"],
        "ended"
    );
    // an interval without status=all is a client error, not a silent guess
    let (st, _, _, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{bb}&datetime=2026-11-01T20:00:00Z/{after}"
        ))
        .await;
    assert_eq!(st, 400);
    let (st, _, _, _) = e.get("/edr/collections/nws-alerts/items?bbox=1,2,3").await;
    assert_eq!(st, 400);
    let (st, _, _, _) = e
        .get("/edr/collections/nws-alerts/items?status=sometimes")
        .await;
    assert_eq!(st, 400);

    // limit truncates and says so
    let (_, _, lim, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?datetime={AT}&limit=1"
        ))
        .await;
    assert_eq!(lim["numberReturned"], 1);
    assert_eq!(lim["truncated"], true);
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn one_alert_in_full_by_raw_or_encoded_urn() {
    let e = env().await;
    e.load().await;
    let id = e.id("Severe Thunderstorm Warning");
    assert!(id.starts_with("urn:oid:"));
    let encoded = handlers::nws_alerts::encode_id(&id);

    for path in [id.clone(), encoded.clone()] {
        let (st, h, f, _) = e
            .get(&format!("/edr/collections/nws-alerts/items/{path}"))
            .await;
        assert_eq!(st, 200, "{path}: {f}");
        assert_eq!(h["cache-control"], "public, max-age=60");
        assert_eq!(f["type"], "Feature");
        assert_eq!(f["id"], id.as_str());
        assert!(f["geometry"]["type"].is_string());
        let p = &f["properties"];
        assert!(p["description"].as_str().unwrap().contains("thunderstorm"));
        assert!(p["description_raw"].is_string() && p["instruction_raw"].is_string());
        assert_eq!(p["vtec"][0]["action"], "NEW");
        assert_eq!(p["sections"]["hazard"], "60 mph wind gusts.");
        assert!(p["parameters"].is_object());
        assert!(p["ugc"].is_array() && p["zones"].is_array() && p["references"].is_array());
        assert!(p["superseded_by"].is_null() && p["ended_at"].is_null());
        assert!(p["first_seen_at"].is_string() && p["last_seen_at"].is_string());
        assert_eq!(
            p["links"][0]["href"],
            format!("http://localhost:8083/edr/collections/nws-alerts/items/{encoded}")
        );
        assert!(f["meta"]["source"] == "api.weather.gov");
    }

    let (st, _, _, _) = e
        .get("/edr/collections/nws-alerts/items/urn%3Aoid%3Anope")
        .await;
    assert_eq!(st, 404);
    let (st, _, _, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items/{encoded}?geometry=false"
        ))
        .await;
    assert_eq!(st, 200);

    // other collections do not answer here, and /conditions still routes to trails
    let (st, _, v, _) = e.get("/edr/collections/tornado/items/123").await;
    assert_eq!(st, 404, "{v}");
    let (_, _, _, _) = e.get("/edr/collections/trails/items/1/conditions").await;
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn area_radius_collection_document_and_freshness() {
    let e = env().await;
    e.load().await;

    let (st, _, v, _) = e
        .get(&format!("/edr/collections/nws-alerts/area?coords=POLYGON((-108%2037,-106%2037,-106%2038.5,-108%2038.5,-108%2037))&datetime={AT}"))
        .await;
    assert_eq!(st, 200, "{v}");
    assert!(events(&v).contains(&"Winter Storm Warning".to_string()));
    let (_, _, v, _) = e
        .get(&format!("/edr/collections/nws-alerts/area?coords=POLYGON((10%2010,11%2010,11%2011,10%2010))&datetime={AT}"))
        .await;
    assert!(events(&v).is_empty());

    // 40 km from the edge of the Colorado zone: out at 20 km, in at 100 km
    let near = |within: &str| {
        format!("/edr/collections/nws-alerts/radius?coords=POINT(-106.1%2037.5)&within={within}&within-units=km&datetime={AT}")
    };
    let (_, _, v, _) = e.get(&near("20")).await;
    assert!(!events(&v).contains(&"Winter Storm Warning".to_string()));
    let (_, _, v, _) = e.get(&near("100")).await;
    assert!(events(&v).contains(&"Winter Storm Warning".to_string()));
    let (st, _, _, _) = e
        .get("/edr/collections/nws-alerts/radius?coords=POINT(-106%2037)")
        .await;
    assert_eq!(st, 400, "within is required");

    // freshness: a poll just now is fresh; one ten minutes ago is stale
    e.nws.record_not_modified(Utc::now()).await.unwrap();
    let (_, _, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords=0,0&datetime={AT}"
        ))
        .await;
    assert_eq!(v["meta"]["stale"], false);
    assert!(v["meta"]["data_age_seconds"].as_i64().unwrap() < 30);
    e.nws
        .record_not_modified(Utc::now() - Duration::minutes(10))
        .await
        .unwrap();
    let (_, _, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords=0,0&datetime={AT}"
        ))
        .await;
    assert_eq!(v["meta"]["stale"], true);
    assert!(v["meta"]["data_age_seconds"].as_i64().unwrap() >= 600);
    e.nws
        .record_poll_failure("timeout", Utc::now())
        .await
        .unwrap();
    let (_, _, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords=0,0&datetime={AT}"
        ))
        .await;
    assert_eq!(v["meta"]["last_error"], "timeout");

    // collection document
    let (st, _, c, _) = e.get("/edr/collections/nws-alerts").await;
    assert_eq!(st, 200);
    assert_eq!(c["id"], "nws-alerts");
    for q in ["position", "area", "radius"] {
        assert!(c["data_queries"][q].is_object(), "{q}");
    }
    assert!(c["description"].as_str().unwrap().contains("STALE"));
    assert!(c["extent"]["spatial"]["bbox"].is_array());
}

/// Response sizes, and what simplification buys on a big zone union (a 20 000-vertex "county").
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn response_sizes_and_simplification() {
    let e = env().await;
    e.load().await;
    let all = format!("/edr/collections/nws-alerts/items?datetime={AT}");
    let (_, _, v, n_all) = e.get(&all).await;
    let (_, _, _, n_nogeom) = e.get(&format!("{all}&geometry=false")).await;
    let (_, _, _, n_pos) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords=POINT(-107%2037.5)&datetime={AT}"
        ))
        .await;
    println!(
        "fixture: {} active alerts world bbox: {n_all} B with geometry, {n_nogeom} B without; position at a point: {n_pos} B",
        v["features"].as_array().unwrap().len()
    );

    // a hugely detailed polygon on one alert
    sqlx::query("UPDATE nws_alerts SET geom = ST_Multi(ST_Buffer(ST_SetSRID(ST_MakePoint(-100, 38), 4326), 4.0, 5000)) WHERE id = $1")
        .bind(e.id("Flood Watch"))
        .execute(e.catalog.pool())
        .await
        .unwrap();
    let conus = "bbox=-125,24,-66,50";
    let (_, _, _, raw) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{conus}&datetime={AT}&simplify=0"
        ))
        .await;
    let (_, _, d, dflt) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{conus}&datetime={AT}"
        ))
        .await;
    let (_, _, _, nog) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?{conus}&datetime={AT}&geometry=false"
        ))
        .await;
    println!(
        "CONUS with a 20k-vertex alert: simplify=0 {raw} B; default {dflt} B; no geometry {nog} B"
    );
    assert!(
        dflt * 5 < raw,
        "default simplification must shrink big geometry a lot ({dflt} vs {raw})"
    );
    assert!(
        feature(&d, "Flood Watch")["geometry"]["type"].is_string(),
        "still a polygon"
    );
    // a street-level view keeps detail
    let (_, _, _, close) = e
        .get(&format!(
            "/edr/collections/nws-alerts/items?bbox=-100.1,38.0,-100.0,38.1&datetime={AT}"
        ))
        .await;
    assert!(close > 0);
}

/// The feed's "Test Message" (CAP status Test) is stored, but not served unless asked for.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn test_alerts_are_hidden_by_default_and_cap_status_is_exposed() {
    let e = env().await;
    e.load().await;
    let test_id = e.id("Test Message");
    let q = "datetime=2026-11-01T23:30:00Z";
    let md = "POINT(-77.05%2039.2)";
    let routes = [
        format!("/edr/collections/nws-alerts/position?coords={md}&{q}"),
        format!("/edr/collections/nws-alerts/items?bbox=-78,38.5,-76,40&{q}"),
        format!("/edr/collections/nws-alerts/area?coords=POLYGON((-78%2038.5,-76%2038.5,-76%2040,-78%2040,-78%2038.5))&{q}"),
        format!("/edr/collections/nws-alerts/radius?coords={md}&within=50&{q}"),
        // history of the same box
        "/edr/collections/nws-alerts/items?bbox=-78,38.5,-76,40&status=all&datetime=2026-11-01T22:00:00Z/2026-11-01T23:30:00Z".to_string(),
    ];
    for uri in &routes {
        let (st, _, v, _) = e.get(uri).await;
        assert_eq!(st, 200, "{uri}: {v}");
        assert!(
            !events(&v).contains(&"Test Message".to_string()),
            "{uri} must not serve a Test alert by default"
        );
        let (st, _, v, _) = e.get(&format!("{uri}&include_non_actual=true")).await;
        assert_eq!(st, 200, "{uri}: {v}");
        let t = feature(&v, "Test Message");
        assert_eq!(t["id"], test_id.as_str(), "{uri}");
        assert_eq!(t["properties"]["cap_status"], "Test", "{uri}");
        if !uri.contains("status=all") {
            // (a history window is judged against the wall clock, which is long past 23:34Z)
            assert_eq!(
                t["properties"]["status"], "active",
                "the lifecycle status is untouched ({uri})"
            );
        }
        let (st, _, v, _) = e.get(&format!("{uri}&include_non_actual=maybe")).await;
        assert_eq!(st, 400, "{uri}: {v}");
    }

    // Actual alerts say so
    let svr = e.id("Severe Thunderstorm Warning");
    let (lon, lat) = e.inside(&svr).await;
    let (_, _, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/position?coords={lon},{lat}&{q}"
        ))
        .await;
    assert_eq!(
        feature(&v, "Severe Thunderstorm Warning")["properties"]["cap_status"],
        "Actual"
    );

    // /items/{id} returns it regardless
    let encoded = handlers::nws_alerts::encode_id(&test_id);
    let (st, _, f, _) = e
        .get(&format!("/edr/collections/nws-alerts/items/{encoded}"))
        .await;
    assert_eq!(st, 200, "{f}");
    assert_eq!(f["properties"]["cap_status"], "Test");
}

/// `n` random points (1 decimal, lon -120..-70, lat 25..48) from a fixed seed, as the `coords`
/// of an `/area` request: a self-intersecting ring like the one that took the live service
/// minutes to answer.
fn scribble_coords(n: usize) -> String {
    let mut state = 1u64;
    let mut next = |lo: f64, hi: f64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
        ((lo + unit * (hi - lo)) * 10.0).round() / 10.0
    };
    let mut pts: Vec<String> = (0..n)
        .map(|_| format!("{}%20{}", next(-120.0, -70.0), next(25.0, 48.0)))
        .collect();
    pts.push(pts[0].clone());
    format!("POLYGON(({}))", pts.join(","))
}

/// `/area` turns away the rings that would make PostGIS grind or that mean something else than
/// they say, with a 400 and a message, before touching the database; ordinary rings still work.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn area_rejects_rings_that_are_expensive_or_ambiguous_and_answers_ordinary_ones() {
    let e = env().await;
    e.load().await;

    // 200 random vertices: ~2.7 KB of URL, and minutes of ST_MakeValid if it got that far
    let started = std::time::Instant::now();
    let (st, h, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/area?coords={}&geometry=false&datetime={AT}",
            scribble_coords(200)
        ))
        .await;
    assert_eq!(st, 400, "{v}");
    assert_eq!(h["content-type"], "application/json");
    assert!(v["detail"].as_str().unwrap().contains("simple ring"), "{v}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "answered in {:?}",
        started.elapsed()
    );

    let area =
        |coords: &str| format!("/edr/collections/nws-alerts/area?coords={coords}&datetime={AT}");
    let rejected = [
        // bowtie
        ("POLYGON((-108%2037,-106%2039,-106%2037,-108%2039))", "simple ring"),
        // hole
        (
            "POLYGON((-109%2036,-105%2036,-105%2040,-109%2040,-109%2036),(-108%2037,-107%2037,-107%2038,-108%2037))",
            "holes are not supported",
        ),
        // 179E to 179W
        (
            "POLYGON((179%2056,-179%2056,-179%2058,179%2058,179%2056))",
            "antimeridian",
        ),
        // zero area
        ("POLYGON((-108%2037,-107%2037,-106%2037,-108%2037))", "no area"),
    ];
    for (coords, want) in rejected {
        let (st, _, v, _) = e.get(&area(coords)).await;
        assert_eq!(st, 400, "{coords}: {v}");
        assert!(
            v["detail"].as_str().unwrap().contains(want),
            "{coords}: {v}"
        );
    }
    // 501 vertices
    let ring: Vec<String> = (0..501)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 501.0;
            format!("{:.4}%20{:.4}", -107.0 + a.cos(), 38.0 + a.sin())
        })
        .collect();
    let (st, _, v, _) = e
        .get(&area(&format!("POLYGON(({}))", ring.join(","))))
        .await;
    assert_eq!(st, 400, "{v}");
    assert!(
        v["detail"].as_str().unwrap().contains("500 vertices"),
        "{v}"
    );

    // ordinary rings, in both windings, find the Colorado alert
    for coords in [
        "POLYGON((-108%2037,-106%2037,-106%2038.5,-108%2038.5,-108%2037))",
        "POLYGON((-108%2037,-108%2038.5,-106%2038.5,-106%2037,-108%2037))",
    ] {
        let (st, _, v, _) = e.get(&area(coords)).await;
        assert_eq!(st, 200, "{coords}: {v}");
        assert!(
            events(&v).contains(&"Winter Storm Warning".to_string()),
            "{coords}"
        );
    }
}

/// A read that the database cancels at its time limit is a 503 with a JSON body on every route
/// that reads alerts; once the contention is gone the same requests work, so the limit neither
/// broke the pool nor stuck to a connection.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_read_that_hits_the_time_limit_is_a_503_and_the_service_recovers() {
    let e = env_with_read_timeout(Some(std::time::Duration::from_millis(300))).await;
    e.load().await;
    let sps = handlers::nws_alerts::encode_id(&e.id("Special Weather Statement"));
    let routes = vec![
        format!("/edr/collections/nws-alerts/position?coords=-107,37.5&datetime={AT}"),
        format!("/edr/collections/nws-alerts/area?coords=POLYGON((-108%2037,-106%2037,-106%2038.5,-108%2038.5,-108%2037))&datetime={AT}"),
        format!("/edr/collections/nws-alerts/radius?coords=POINT(-107%2037.5)&within=50&datetime={AT}"),
        format!("/edr/collections/nws-alerts/items?bbox=-108,36,-106,39&datetime={AT}"),
        format!("/edr/collections/nws-alerts/items/{sps}"),
    ];
    for r in &routes {
        let (st, _, v, _) = e.get(r).await;
        assert_eq!(st, 200, "{r}: {v}");
    }

    // Another session holds the alert tables exclusively, so every read queues until the limit.
    let mut holder = e.catalog.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE nws_alerts, nws_alerts_status IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    for r in &routes {
        let started = std::time::Instant::now();
        let (st, h, v, _) = e.get(r).await;
        assert_eq!(st, 503, "{r}: {v}");
        assert_eq!(h["content-type"], "application/json", "{r}");
        assert_eq!(v["status"], 503, "{r}: {v}");
        assert!(
            v["detail"].as_str().unwrap().contains("too expensive"),
            "{r}: {v}"
        );
        let took = started.elapsed();
        assert!(
            took >= std::time::Duration::from_millis(250)
                && took < std::time::Duration::from_secs(5),
            "{r}: {took:?}"
        );
    }
    // The collection document degrades to "no poll known" instead of failing.
    let (st, _, c, _) = e.get("/edr/collections/nws-alerts").await;
    assert_eq!(st, 200, "{c}");
    assert!(c["description"].as_str().unwrap().contains("STALE"));
    holder.rollback().await.unwrap();

    for r in &routes {
        let (st, _, v, _) = e.get(r).await;
        assert_eq!(st, 200, "{r} after the lock is gone: {v}");
    }
    // No connection of the shared pool kept the limit, so other queries are not cut short.
    let mut held = Vec::new();
    for _ in 0..10 {
        held.push(e.catalog.pool().acquire().await.unwrap());
    }
    for conn in &mut held {
        let t: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        assert_eq!(t, "0");
    }
}

// --- an explicit `datetime`: what was in force then, labelled as of then ----------------------

/// 2026-11-01 `h:m`Z.
fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, h, m, 0).unwrap()
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// Both lie in lon -107.5..-106.5; the small one covers only the eastern half.
const BIG: [f64; 4] = [-107.5, 37.0, -106.5, 38.0];
const SMALL: [f64; 4] = [-107.0, 37.0, -106.5, 38.0];
/// In `BIG` only.
const P: (f64, f64) = (-107.25, 37.5);
/// In both.
const Q: (f64, f64) = (-106.75, 37.5);
/// Far from both squares: where the long-lived alert Z is.
const R: (f64, f64) = (-100.0, 40.0);

impl Env {
    /// The fixture's Severe Thunderstorm Warning under `id`, with its own polygon and times.
    fn timed(
        &self,
        id: &str,
        bbox: [f64; 4],
        sent: DateTime<Utc>,
        onset: DateTime<Utc>,
        ends: DateTime<Utc>,
        replaces: &[&str],
    ) -> ParsedAlert {
        let mut a = self
            .alerts
            .iter()
            .find(|a| a.event == "Severe Thunderstorm Warning")
            .unwrap()
            .clone();
        a.id = id.into();
        a.geometry_json = Some(square(bbox[0], bbox[1], bbox[2], bbox[3]));
        a.sent = Some(sent);
        a.effective = Some(sent);
        a.onset = Some(onset);
        a.ends = Some(ends);
        a.expires = Some(ends);
        a.message_type = if replaces.is_empty() {
            "Alert"
        } else {
            "Update"
        }
        .into();
        a.references = replaces.iter().map(|r| r.to_string()).collect();
        a
    }

    /// A (sent 10:00, hazard from 10:30, until 18:00) is replaced at 11:00 by A2 (smaller
    /// polygon); B (sent 10:00, until 18:00) is dropped from the feed at 12:00; Z (sent 10:00,
    /// until 2099) is somewhere else and never changes. Polls at 10:01, 11:01 and 12:00.
    async fn load_timeline(&self) {
        let far = Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap();
        let a = self.timed("urn:test:a", BIG, at(10, 0), at(10, 30), at(18, 0), &[]);
        let b = self.timed("urn:test:b", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
        let z = self.timed(
            "urn:test:z",
            [R.0 - 0.5, R.1 - 0.5, R.0 + 0.5, R.1 + 0.5],
            at(10, 0),
            at(10, 0),
            far,
            &[],
        );
        let a2 = self.timed(
            "urn:test:a2",
            SMALL,
            at(11, 0),
            at(11, 0),
            at(18, 0),
            &["urn:test:a"],
        );
        for (now, feed) in [
            (at(10, 1), vec![a, b.clone(), z.clone()]),
            (at(11, 1), vec![a2.clone(), b, z.clone()]),
            (at(12, 0), vec![a2, z]),
        ] {
            self.nws.apply_feed(&feed, Some(now), now).await.unwrap();
        }
    }

    /// `"id status"` of each alert a route returns for `datetime=t`, sorted.
    async fn then(&self, route: &str, t: &str) -> Vec<String> {
        let sep = if route.contains('?') { '&' } else { '?' };
        let uri = format!("/edr/collections/nws-alerts/{route}{sep}datetime={t}");
        let (st, _, v, _) = self.get(&uri).await;
        assert_eq!(st, 200, "{uri}: {v}");
        let mut out: Vec<String> = v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                format!(
                    "{} {}",
                    f["id"].as_str().unwrap().trim_start_matches("urn:test:"),
                    f["properties"]["status"].as_str().unwrap()
                )
            })
            .collect();
        out.sort();
        out
    }
}

fn pos(xy: (f64, f64)) -> String {
    format!("position?coords=POINT({}%20{})", xy.0, xy.1)
}

fn items_around(xy: (f64, f64)) -> String {
    let d = 0.01;
    format!(
        "items?geometry=false&bbox={},{},{},{}",
        xy.0 - d,
        xy.1 - d,
        xy.0 + d,
        xy.1 + d
    )
}

fn area_around(xy: (f64, f64)) -> String {
    let d = 0.01;
    format!(
        "area?coords=POLYGON(({} {},{} {},{} {},{} {},{} {}))",
        xy.0 - d,
        xy.1 - d,
        xy.0 + d,
        xy.1 - d,
        xy.0 + d,
        xy.1 + d,
        xy.0 - d,
        xy.1 + d,
        xy.0 - d,
        xy.1 - d
    )
    .replace(' ', "%20")
}

fn radius_around(xy: (f64, f64)) -> String {
    format!(
        "radius?coords=POINT({}%20{})&within=500&within-units=m",
        xy.0, xy.1
    )
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn an_explicit_datetime_returns_what_was_in_force_then_with_the_status_it_had() {
    let e = env().await;
    e.load_timeline().await;

    // (instant, at P: BIG only, at Q: BIG and SMALL). Same on every route that takes a point,
    // a box, a polygon or a radius. A is "upcoming" until its hazard starts at 10:30; once it
    // is replaced (11:00) it is gone, and a point only the old, larger polygon covered with it.
    let cases: [(DateTime<Utc>, Vec<&str>, Vec<&str>); 11] = [
        (at(9, 30), vec![], vec![]), // nothing issued yet, though the live flags say otherwise
        (
            at(10, 0),
            vec!["a upcoming", "b active"],
            vec!["a upcoming", "b active"],
        ),
        (
            at(10, 15),
            vec!["a upcoming", "b active"],
            vec!["a upcoming", "b active"],
        ),
        (
            at(10, 30),
            vec!["a active", "b active"],
            vec!["a active", "b active"],
        ),
        // before the update: A was in force, though it has been replaced since (and the poll
        // that noticed ran only at 11:01)
        (
            at(10, 59),
            vec!["a active", "b active"],
            vec!["a active", "b active"],
        ),
        (at(11, 0), vec!["b active"], vec!["a2 active", "b active"]),
        (at(11, 30), vec!["b active"], vec!["a2 active", "b active"]),
        // B was dropped from the feed at 12:00
        (at(12, 0), vec![], vec!["a2 active"]),
        (at(17, 59), vec![], vec!["a2 active"]),
        (at(18, 0), vec![], vec![]), // the end time is exclusive
        (at(23, 0), vec![], vec![]),
    ];
    for (t, at_p, at_q) in cases {
        let t = iso(t);
        for route in [pos, items_around, area_around, radius_around] {
            assert_eq!(e.then(&route(P), &t).await, at_p, "{} at P, {t}", route(P));
            assert_eq!(e.then(&route(Q), &t).await, at_q, "{} at Q, {t}", route(Q));
        }
    }

    // include_superseded has no say on status=active: a replaced message in force then is
    // returned either way
    assert_eq!(
        e.then(
            &format!("{}&include_superseded=false", items_around(P)),
            &iso(at(10, 45))
        )
        .await,
        vec!["a active", "b active"]
    );
    // status=all on an instant is the same window and the same labels; the replaced message
    // stays hidden unless asked for (as on any history window), and is "active" as of then
    let all = format!("{}&status=all", items_around(P));
    assert_eq!(e.then(&all, &iso(at(10, 45))).await, vec!["b active"]);
    assert_eq!(
        e.then(&format!("{all}&include_superseded=true"), &iso(at(10, 45)))
            .await,
        vec!["a active", "b active"]
    );

    // /items/{id} is not an instant query: A as it is now, with when it was replaced
    let (st, _, a, _) = e.get("/edr/collections/nws-alerts/items/urn:test:a").await;
    assert_eq!(st, 200, "{a}");
    assert_eq!(a["properties"]["status"], "superseded");
    assert_eq!(a["properties"]["superseded_by"], "urn:test:a2");
    assert_eq!(a["properties"]["superseded_at"], "2026-11-01T11:00:00Z");

    // The long-lived Z: no datetime and `datetime=now` are the live set at the wall clock and
    // agree; an instant in 2098 is still in force, one after 2099 is not
    let live = e.then(&pos(R), "now").await;
    assert_eq!(live, vec!["z active"]);
    let (st, _, none, _) = e
        .get(&format!("/edr/collections/nws-alerts/{}", pos(R)))
        .await;
    assert_eq!(st, 200, "{none}");
    assert_eq!(none["features"][0]["id"], "urn:test:z");
    assert_eq!(none["features"][0]["properties"]["status"], "active");
    assert_eq!(none["numberReturned"], 1);
    assert_eq!(
        e.then(&pos(R), "2098-06-01T00:00:00Z").await,
        vec!["z active"]
    );
    assert!(e.then(&pos(R), "2100-01-01T00:00:00Z").await.is_empty());
    // ...and it was not there before it was issued
    assert!(e.then(&pos(R), &iso(at(9, 59))).await.is_empty());
    assert_eq!(e.then(&pos(R), &iso(at(10, 0))).await, vec!["z active"]);

    // an interval is still not an instant
    let (st, _, v, _) = e
        .get(&format!(
            "/edr/collections/nws-alerts/{}&datetime=2026-11-01T10:00:00Z/2026-11-01T11:00:00Z",
            pos(P)
        ))
        .await;
    assert_eq!(st, 400, "{v}");
}
