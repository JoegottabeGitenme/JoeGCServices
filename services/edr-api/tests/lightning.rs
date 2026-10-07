//! End-to-end tests of the `glm-lightning` EDR collection: the REAL generic
//! `items` / `area` / `radius` dispatchers, the real lightning handlers, the real
//! `config/edr` directory (so `glm.yaml` is validated too) and a real PostGIS.
//!
//! `#[ignore]`d like the other integration tests (CI runs them with Docker).
//! Locally against your own database:
//!
//! ```text
//! docker run -d --name pg-glm -p 15434:5432 -e POSTGRES_PASSWORD=t postgis/postgis:16-3.4
//! LIGHTNING_TEST_DATABASE_URL=postgres://postgres:t@localhost:15434/postgres \
//!   cargo test -p edr-api --test lightning -- --ignored --test-threads=1
//! ```

use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Extension;
use chrono::{Duration, Utc};
use serde_json::Value;
use tokio::sync::RwLock;

use edr_api::{
    availability::AvailabilityCache, config::EdrConfig, handlers, location_cache::LocationCache,
    metrics::MetricsCollector, state::AppState,
};
use grid_processor::{GridDataService, MinioConfig};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog, NewFlash,
};

struct Env {
    state: Arc<AppState>,
    lightning: LightningCatalog,
    _infra: Option<test_utils::containers::TestInfrastructure>,
}

async fn env() -> Env {
    let (url, minio, infra) = match std::env::var("LIGHTNING_TEST_DATABASE_URL") {
        Ok(u) => (u, "http://127.0.0.1:1".to_string(), None),
        Err(_) => {
            let infra = test_utils::containers::TestInfrastructure::start().await;
            (infra.postgres_url(), infra.minio_url(), Some(infra))
        }
    };
    let catalog = Arc::new(Catalog::connect(&url).await.unwrap());
    catalog.migrate().await.unwrap();
    catalog.migrate_observations().await.unwrap();
    catalog.migrate_lightning().await.unwrap();
    // `trails` is a real configured collection the dispatch test needs a table for.
    catalog.migrate_linear_features().await.unwrap();
    catalog.migrate_segment_conditions().await.unwrap();
    sqlx::query("TRUNCATE lightning_flashes, lightning_ingest_progress RESTART IDENTITY")
        .execute(catalog.pool())
        .await
        .unwrap();

    let grid_data_service = GridDataService::new(
        Arc::clone(&catalog),
        MinioConfig {
            endpoint: minio,
            bucket: "unused".to_string(),
            access_key_id: "x".to_string(),
            secret_access_key: "x".to_string(),
            region: "us-east-1".to_string(),
            allow_http: true,
        },
        64,
    )
    .unwrap();

    // The REAL config directory: proves glm.yaml parses and wires up.
    let edr_config = EdrConfig::load_from_dir("../../config/edr").unwrap();
    let lightning = LightningCatalog::new(catalog.pool_clone());
    let state = Arc::new(AppState {
        observation_catalog: Arc::new(ObservationCatalog::new(catalog.pool_clone())),
        storm_event_catalog: Arc::new(StormEventCatalog::new(catalog.pool_clone())),
        linear_feature_catalog: Arc::new(LinearFeatureCatalog::new(catalog.pool_clone())),
        segment_conditions_catalog: Arc::new(SegmentConditionsCatalog::new(catalog.pool_clone())),
        lightning_catalog: Arc::new(LightningCatalog::new(catalog.pool_clone())),
        catalog,
        grid_data_service,
        edr_config: Arc::new(RwLock::new(edr_config)),
        base_url: "http://localhost:8083/edr".to_string(),
        location_cache: Arc::new(LocationCache::new(16, 60)),
        availability_cache: Arc::new(AvailabilityCache::new(60)),
        metrics: Arc::new(MetricsCollector::new()),
    });
    Env {
        state,
        lightning,
        _infra: infra,
    }
}

fn flash(sat: &str, secs_ago: i64, id: i32, lon: f64, lat: f64) -> NewFlash {
    NewFlash {
        satellite: sat.to_string(),
        flash_time: Utc::now() - Duration::seconds(secs_ago),
        flash_id: id,
        lon,
        lat,
        energy_j: Some(4.7e-14),
        quality: 0,
    }
}

async fn body(resp: Response) -> (StatusCode, HeaderMap, Value) {
    let (parts, b) = resp.into_parts();
    let bytes = axum::body::to_bytes(b, 16 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Call the REAL generic items dispatcher with a parsed query string.
async fn items(e: &Env, qs: &str) -> (StatusCode, HeaderMap, Value) {
    let uri: axum::http::Uri = format!("/items?{qs}").parse().unwrap();
    let q = Query::<handlers::items::ItemsQueryParams>::try_from_uri(&uri).unwrap();
    body(
        handlers::items::items_handler(Extension(e.state.clone()), Path("glm-lightning".into()), q)
            .await,
    )
    .await
}

async fn area(e: &Env, qs: &str) -> (StatusCode, HeaderMap, Value) {
    let uri: axum::http::Uri = format!("/area?{qs}").parse().unwrap();
    let q = Query::<handlers::area::AreaQueryParams>::try_from_uri(&uri).unwrap();
    body(
        handlers::area::area_handler(
            Extension(e.state.clone()),
            Path("glm-lightning".into()),
            q,
            HeaderMap::new(),
        )
        .await,
    )
    .await
}

async fn radius(e: &Env, qs: &str) -> (StatusCode, HeaderMap, Value) {
    let uri: axum::http::Uri = format!("/radius?{qs}").parse().unwrap();
    let q = Query::<handlers::radius::RadiusQueryParams>::try_from_uri(&uri).unwrap();
    body(
        handlers::radius::radius_handler(
            Extension(e.state.clone()),
            Path("glm-lightning".into()),
            q,
            HeaderMap::new(),
        )
        .await,
    )
    .await
}

fn ids(v: &Value) -> Vec<i64> {
    v["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_i64().unwrap())
        .collect()
}

async fn seed(e: &Env) {
    e.lightning
        .insert_flashes(&[
            flash("goes-east", 30, 1, -105.25, 40.01), // Boulder, 30 s ago
            flash("goes-east", 90, 2, -104.99, 39.74), // Denver, 90 s ago
            flash("goes-west", 45, 3, -105.00, 39.75), // Denver, seen by West
            flash("goes-east", 1800, 4, -105.25, 40.01), // Boulder, 30 min ago (outside the default 10 min)
            flash("goes-east", 20, 5, -95.0, 35.0),      // Oklahoma
        ])
        .await
        .unwrap();
}

#[tokio::test]
#[ignore]
async fn the_default_query_is_the_last_ten_minutes_of_goes_east_with_the_documented_shape() {
    let e = env().await;
    seed(&e).await;
    let (status, headers, v) = items(&e, "").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("content-type").unwrap(), "application/geo+json");
    assert_eq!(headers.get("cache-control").unwrap(), "max-age=5");
    // east, <=10 min old: flash_ids 1, 2, 5  (3 is west, 4 is 30 min old)
    assert_eq!(v["numberReturned"], 3);
    let props: Vec<i64> = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["properties"]["age_seconds"].as_f64().unwrap() as i64)
        .collect();
    assert!(props.iter().all(|a| *a >= 0 && *a < 600), "{props:?}");
    let f = &v["features"][0];
    assert_eq!(f["type"], "Feature");
    assert_eq!(f["geometry"]["type"], "Point");
    assert_eq!(f["properties"]["satellite"], "goes-east");
    assert!(f["properties"]["flash_time"]
        .as_str()
        .unwrap()
        .ends_with('Z'));
    assert_eq!(f["properties"]["energy_j"], 4.7e-14);
    assert_eq!(v["lastId"], *ids(&v).iter().max().unwrap());
    assert!(v["timeStamp"].as_str().is_some());
}

#[tokio::test]
#[ignore]
async fn satellite_and_window_selection() {
    let e = env().await;
    seed(&e).await;
    assert_eq!(
        items(&e, "satellite=goes-west").await.2["numberReturned"],
        1
    );
    assert_eq!(items(&e, "satellite=both").await.2["numberReturned"], 4);
    assert_eq!(
        items(&e, "window=PT1H").await.2["numberReturned"],
        4,
        "the 30-minute-old east flash is now inside"
    );
    assert_eq!(
        items(&e, "window=PT1M").await.2["numberReturned"],
        2,
        "only the 30 s and 20 s ones"
    );
    assert_eq!(
        items(&e, "satellite=both&window=PT1H").await.2["numberReturned"],
        5
    );
}

#[tokio::test]
#[ignore]
async fn the_cursor_returns_only_newer_flashes_and_pages_without_gaps() {
    let e = env().await;
    seed(&e).await;
    let (_, _, first) = items(&e, "satellite=both&window=PT1H").await;
    let last = first["lastId"].as_i64().unwrap();

    // Nothing new yet: empty, with a null cursor so the client keeps its old one.
    let (_, _, none) = items(&e, &format!("satellite=both&after={last}")).await;
    assert_eq!(none["numberReturned"], 0);
    assert!(none["lastId"].is_null());

    // New flashes arrive; the cursor yields exactly those.
    e.lightning
        .insert_flashes(&[
            flash("goes-east", 2, 100, -105.1, 40.0),
            flash("goes-east", 1, 101, -105.2, 40.0),
        ])
        .await
        .unwrap();
    let (_, _, newer) = items(&e, &format!("satellite=both&after={last}")).await;
    assert_eq!(newer["numberReturned"], 2);
    assert!(ids(&newer).iter().all(|i| *i > last));

    // A truncated page continues from its last id (oldest-first with a cursor).
    let (_, _, p1) = items(&e, "satellite=both&after=0&limit=3").await;
    assert_eq!(
        p1["numberReturned"], 3,
        "numberReturned == limit => truncated"
    );
    let (_, _, p2) = items(
        &e,
        &format!("satellite=both&after={}&limit=3", p1["lastId"]),
    )
    .await;
    let (_, _, p3) = items(
        &e,
        &format!("satellite=both&after={}&limit=3", p2["lastId"]),
    )
    .await;
    let mut all: Vec<i64> = [ids(&p1), ids(&p2), ids(&p3)].concat();
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!(
        (n, all.len()),
        (7, 7),
        "7 flashes in total, each exactly once across pages"
    );
}

#[tokio::test]
#[ignore]
async fn without_a_cursor_a_small_limit_keeps_the_newest_flashes() {
    let e = env().await;
    e.lightning
        .insert_flashes(
            &(0..10)
                .map(|i| flash("goes-east", 100 - i * 10, i as i32, -105.0, 40.0))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    let (_, _, v) = items(&e, "limit=3").await;
    assert_eq!(v["numberReturned"], 3);
    let got: Vec<i64> = ids(&v);
    assert_eq!(got, vec![8, 9, 10], "the newest three, oldest-first");
}

#[tokio::test]
#[ignore]
async fn bbox_area_and_radius_select_the_right_flashes() {
    let e = env().await;
    seed(&e).await;
    // Boulder-ish box: only flash_id 1 (east, recent)
    assert_eq!(
        items(&e, "bbox=-105.5,39.9,-105.0,40.1").await.2["numberReturned"],
        1
    );
    assert_eq!(
        area(&e, "coords=-105.5,39.9,-105.0,40.1").await.2["numberReturned"],
        1
    );
    // Front Range box with both satellites: Boulder + 2 Denver (east+west)
    assert_eq!(
        area(&e, "coords=-105.5,39.5,-104.5,40.2&satellite=both")
            .await
            .2["numberReturned"],
        3
    );

    // Radius: 10 km around Denver -> the east Denver flash only (west excluded by default)
    let (s, _, v) = radius(
        &e,
        "coords=POINT(-104.99%2039.74)&within=10&within-units=km",
    )
    .await;
    assert_eq!((s, v["numberReturned"].as_i64()), (StatusCode::OK, Some(1)));
    // ...and with both satellites, 2
    assert_eq!(
        radius(
            &e,
            "coords=POINT(-104.99%2039.74)&within=10&within-units=km&satellite=both"
        )
        .await
        .2["numberReturned"],
        2
    );
    // default radius is 50 km: Boulder (~40 km from Denver) is inside
    assert_eq!(
        radius(&e, "coords=POINT(-104.99%2039.74)").await.2["numberReturned"],
        2
    );
    // 5 km around Denver excludes Boulder
    assert_eq!(
        radius(&e, "coords=POINT(-104.99%2039.74)&within=5&within-units=km")
            .await
            .2["numberReturned"],
        1
    );
}

#[tokio::test]
#[ignore]
async fn bad_requests_are_400_with_a_message_and_do_not_touch_the_data() {
    let e = env().await;
    seed(&e).await;
    for qs in [
        "satellite=moon",
        "window=banana",
        "window=PT10M&datetime=2026-10-07T20:00:00Z/..",
        "datetime=2026-10-07T20:00:00Z",
        "after=-4",
        "bbox=-105,39,-106,40",
        "bbox=abc",
    ] {
        let (s, _, _) = items(&e, qs).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "items?{qs}");
    }
    assert_eq!(
        radius(&e, "coords=POINT(-105%2040)&within=900&within-units=km")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        radius(&e, "coords=nonsense").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        area(&e, "").await.0,
        StatusCode::BAD_REQUEST,
        "area needs coords"
    );
}

#[tokio::test]
#[ignore]
async fn a_quiet_sky_is_a_valid_empty_answer_and_the_collection_is_still_listed() {
    let e = env().await; // no flashes at all
    let (s, _, v) = items(&e, "").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["numberReturned"], 0);
    assert_eq!(v["features"], serde_json::json!([]));
    assert!(v["lastId"].is_null());

    // Unlike trails (skipped when empty), lightning must always be listed: a
    // collection that vanishes whenever it is not storming looks like an outage.
    let (s, _, list) = body(
        handlers::collections::list_collections_handler(
            Extension(e.state.clone()),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let listed: Vec<&str> = list["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&"glm-lightning"), "{listed:?}");

    let (s, _, c) = body(
        handlers::collections::get_collection_handler(
            Extension(e.state.clone()),
            Path("glm-lightning".into()),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(c["id"], "glm-lightning");
    assert_eq!(
        c["extent"]["spatial"]["bbox"][0],
        serde_json::json!([-125.0, 24.0, -66.0, 50.0])
    );
}

#[tokio::test]
#[ignore]
async fn other_collections_are_not_served_by_the_lightning_handlers() {
    let e = env().await;
    let uri: axum::http::Uri = "/items?satellite=both".parse().unwrap();
    // `trails` is a real configured collection backed by a different source: it must
    // not be hijacked by (or error inside) the lightning dispatch.
    let q = Query::<handlers::items::ItemsQueryParams>::try_from_uri(&uri).unwrap();
    let (s, _, v) = body(
        handlers::items::items_handler(Extension(e.state.clone()), Path("trails".into()), q).await,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["type"], "FeatureCollection");

    // Unknown collection through the lightning handler directly -> 404.
    let q = Query(handlers::lightning::LightningItemsParams::default());
    let (s, _, _) = body(
        handlers::lightning::lightning_items_handler(
            Extension(e.state.clone()),
            Path("nope".into()),
            q,
        )
        .await,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // A non-lightning collection through the lightning handler directly -> 400.
    let q = Query(handlers::lightning::LightningItemsParams::default());
    let (s, _, _) = body(
        handlers::lightning::lightning_items_handler(
            Extension(e.state.clone()),
            Path("trails".into()),
            q,
        )
        .await,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
