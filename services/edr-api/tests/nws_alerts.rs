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
        nws_alert_catalog: Arc::new(NwsAlertCatalog::new(catalog.pool_clone())),
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
