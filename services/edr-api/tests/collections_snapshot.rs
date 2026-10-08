//! End-to-end tests of the `/collections` snapshot cache: the REAL handler, the REAL
//! `config/edr` directory and a real PostGIS. `#[ignore]`d like the other integration
//! tests (CI runs them with Docker). Locally:
//!
//! ```text
//! docker run -d --name pg-glm -p 15434:5432 -e POSTGRES_PASSWORD=t postgis/postgis:16-3.4
//! LIGHTNING_TEST_DATABASE_URL=postgres://postgres:t@localhost:15434/postgres \
//!   cargo test -p edr-api --test collections_snapshot -- --ignored --test-threads=1
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode};
use axum::Extension;
use serde_json::Value;
use tokio::sync::RwLock;

use edr_api::{
    availability::AvailabilityCache, config::EdrConfig, handlers, location_cache::LocationCache,
    metrics::MetricsCollector, snapshot_cache::SnapshotCache, state::AppState,
};
use grid_processor::{GridDataService, MinioConfig};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog,
};

struct Env {
    state: Arc<AppState>,
    pool: sqlx::PgPool,
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
    catalog.migrate_linear_features().await.unwrap();
    catalog.migrate_segment_conditions().await.unwrap();
    let pool = catalog.pool_clone();
    sqlx::query("TRUNCATE observations, taf_periods, taf_forecasts, locations CASCADE")
        .execute(&pool)
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

    let state = Arc::new(AppState {
        observation_catalog: Arc::new(ObservationCatalog::new(catalog.pool_clone())),
        storm_event_catalog: Arc::new(StormEventCatalog::new(catalog.pool_clone())),
        linear_feature_catalog: Arc::new(LinearFeatureCatalog::new(catalog.pool_clone())),
        segment_conditions_catalog: Arc::new(SegmentConditionsCatalog::new(catalog.pool_clone())),
        lightning_catalog: Arc::new(LightningCatalog::new(catalog.pool_clone())),
        collections_snapshot: Arc::new(SnapshotCache::new(Duration::from_secs(60))),
        catalog,
        grid_data_service,
        edr_config: Arc::new(RwLock::new(
            EdrConfig::load_from_dir("../../config/edr").unwrap(),
        )),
        base_url: "http://localhost:8083/edr".to_string(),
        location_cache: Arc::new(LocationCache::new(16, 60)),
        availability_cache: Arc::new(AvailabilityCache::new(60)),
        metrics: Arc::new(MetricsCollector::new()),
    });
    Env {
        state,
        pool,
        _infra: infra,
    }
}

async fn list(e: &Env) -> (StatusCode, HeaderMap, Value) {
    let resp = handlers::collections::list_collections_handler(
        Extension(e.state.clone()),
        HeaderMap::new(),
    )
    .await;
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 64 << 20).await.unwrap();
    (
        parts.status,
        parts.headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn ids(v: &Value) -> Vec<String> {
    v["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

async fn add_metar_observation(pool: &sqlx::PgPool) {
    sqlx::query("INSERT INTO locations (id, name, location) VALUES ('KDEN', 'Denver', ST_GeogFromText('POINT(-104.67 39.86)'))")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO observations (location_id, source, obs_time) VALUES ('KDEN', 'metar', now())",
    )
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore]
async fn the_listing_is_served_from_the_snapshot_not_recomputed_per_request() {
    let e = env().await;
    let (s, h, first) = list(&e).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get("content-type").unwrap(), "application/json");
    assert_eq!(h.get("cache-control").unwrap(), "max-age=60");
    assert!(ids(&first).contains(&"glm-lightning".to_string()));
    assert!(
        !ids(&first).contains(&"metar".to_string()),
        "no observations yet -> metar not listed"
    );

    // New data appears in the database...
    add_metar_observation(&e.pool).await;

    // ...but within the TTL the listing is served from the snapshot, so it does NOT
    // reflect it yet (that is the trade: staleness of up to the TTL for an instant answer).
    let (_, _, second) = list(&e).await;
    assert!(
        !ids(&second).contains(&"metar".to_string()),
        "must be served from the snapshot"
    );
    assert!(
        first == second,
        "identical body while the snapshot is fresh"
    );

    // After invalidation the next request rebuilds and sees it.
    e.state.collections_snapshot.invalidate().await;
    let (_, _, third) = list(&e).await;
    assert!(
        ids(&third).contains(&"metar".to_string()),
        "a rebuilt snapshot reflects the new data"
    );
}

#[tokio::test]
#[ignore]
async fn a_cached_response_is_fast_even_though_building_it_is_not() {
    let e = env().await;
    let t0 = Instant::now();
    list(&e).await; // cold: pays the whole cost
    let cold = t0.elapsed();
    let t1 = Instant::now();
    for _ in 0..20 {
        list(&e).await;
    }
    let warm_each = t1.elapsed() / 20;
    println!("cold build {cold:?}; cached request {warm_each:?}");
    assert!(
        warm_each < Duration::from_millis(100),
        "cached request took {warm_each:?}"
    );
    assert!(
        warm_each * 3 < cold || cold < Duration::from_millis(300),
        "cache gave no benefit: cold {cold:?} vs warm {warm_each:?}"
    );
}

/// Blank the one legitimately time-dependent field: glm-lightning's temporal extent
/// starts at "now - 24 h", evaluated when the body is BUILT, so two builds a few
/// milliseconds apart differ there (and a snapshot's value is up to one TTL old).
fn without_build_time(mut v: Value) -> Value {
    for c in v["collections"].as_array_mut().unwrap() {
        if c["id"] == "glm-lightning" {
            c["extent"]["temporal"]["interval"][0][0] = Value::Null;
        }
    }
    v
}

#[tokio::test]
#[ignore]
async fn the_snapshot_is_exactly_what_the_live_builder_produces() {
    // The cache must not change the response, only when it is computed.
    let e = env().await;
    let (_, _, served) = list(&e).await;
    let live: Value = serde_json::from_str(
        &handlers::collections::build_collections_list_json(&e.state)
            .await
            .unwrap(),
    )
    .unwrap();
    let (a, b) = (without_build_time(served), without_build_time(live));
    // `assert!` (not assert_eq!) so a failure doesn't dump ~20 KB of JSON.
    assert!(
        a == b,
        "cached and live listings differ beyond the build-time field"
    );
    assert!(!a["collections"].as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore]
async fn a_config_reload_discards_the_snapshot() {
    let e = env().await;
    list(&e).await;
    assert!(e.state.collections_snapshot.age().await.is_some());
    // reload_config reads CONFIG_DIR/edr
    std::env::set_var("CONFIG_DIR", "../../config");
    e.state.reload_config().await.unwrap();
    assert!(
        e.state.collections_snapshot.age().await.is_none(),
        "the cached body was built from the OLD config"
    );
    // and it rebuilds cleanly afterwards
    assert_eq!(list(&e).await.0, StatusCode::OK);
}

#[tokio::test]
#[ignore]
async fn a_bad_accept_header_is_still_rejected_before_the_cache() {
    let e = env().await;
    let mut h = HeaderMap::new();
    h.insert("accept", "text/html".parse().unwrap());
    let resp = handlers::collections::list_collections_handler(Extension(e.state.clone()), h).await;
    assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
}
