//! End-to-end tests of `GET /edr/collections/:collection_id/conditions?ids=`: the REAL
//! routes and handlers, the REAL `config/edr` directory and a real PostGIS.
//! `#[ignore]`d like the other integration tests (CI runs them with Docker). Locally:
//!
//! ```text
//! docker run -d --name pg-glm -p 15434:5432 -e POSTGRES_PASSWORD=t postgis/postgis:16-3.4
//! LIGHTNING_TEST_DATABASE_URL=postgres://postgres:t@localhost:15434/postgres \
//!   cargo test -p edr-api --test trail_conditions_batch -- --ignored --test-threads=1
//! ```
//!
//! The first block is the frontend's own acceptance list for the endpoint.

use std::sync::Arc;
use std::time::Duration;

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use serde_json::Value;
use tokio::sync::RwLock;
use tower::ServiceExt;

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
    app: Router,
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
    sqlx::query("TRUNCATE linear_features, segment_conditions")
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

    // The two routes under test, with the same paths main.rs registers.
    let app = Router::new()
        .route(
            "/edr/collections/:collection_id/conditions",
            get(handlers::linear_features::trail_conditions_batch_handler),
        )
        .route(
            "/edr/collections/:collection_id/items/:feature_id/conditions",
            get(handlers::linear_features::trail_conditions_timeseries_handler),
        )
        .layer(Extension(state));
    Env {
        app,
        pool,
        _infra: infra,
    }
}

struct Reply {
    status: u16,
    cache_control: Option<String>,
    content_type: Option<String>,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or_else(|_| panic!("not JSON: {}", self.text))
    }
}

async fn get_path(app: &Router, uri: &str) -> Reply {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let header = |n: &str| {
        resp.headers()
            .get(n)
            .map(|v| v.to_str().unwrap().to_string())
    };
    let cache_control = header("cache-control");
    let content_type = header("content-type");
    let bytes = axum::body::to_bytes(resp.into_body(), 64 << 20)
        .await
        .unwrap();
    Reply {
        status,
        cache_control,
        content_type,
        text: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

async fn insert_trail(pool: &sqlx::PgPool, id: i64, name: Option<&str>) {
    sqlx::query(
        "INSERT INTO linear_features (feature_id, feature_class, name, geom, region) \
         VALUES ($1, 'mtb_trail', $2, ST_GeomFromText('LINESTRING(-105.3 40.0, -105.29 40.01)', 4326), 'test')",
    )
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

/// Hourly rows from `back` hours ago to `ahead` hours from now, from the run that
/// started `run_age_h` hours ago, under `version`. Overlapping calls with different
/// versions/runs exercise the "newest run wins per valid hour" stitching.
async fn insert_series(
    pool: &sqlx::PgPool,
    id: i64,
    back: i32,
    ahead: i32,
    run_age_h: i32,
    version: &str,
    soil: f32,
) {
    sqlx::query(
        "INSERT INTO segment_conditions \
           (feature_id, run_time, valid_time, forecast_hour, soil_moisture, saturation, \
            frozen_fraction, confidence, model_version) \
         SELECT $1, date_trunc('hour', now()) - make_interval(hours => $4), \
                date_trunc('hour', now()) + make_interval(hours => g), g + $4, $6, $6 * 2, 0, 1, $5 \
         FROM generate_series(-$2, $3) g",
    )
    .bind(id)
    .bind(back)
    .bind(ahead)
    .bind(run_age_h)
    .bind(version)
    .bind(soil)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed(pool: &sqlx::PgPool) {
    insert_trail(pool, 101, Some("Forsythe Canyon Trail")).await;
    insert_trail(pool, 102, None).await; // unnamed
    insert_trail(pool, 103, Some("Out Of Coverage")).await; // known, no rows
    insert_trail(pool, 104, Some("Gamma")).await;
    // 101: an older run covering -8h..+20h, a newer run (different version) covering
    // -3h..+6h on top of it -> stitched series
    insert_series(pool, 101, 8, 20, 10, "trail-physics-v1", 0.10).await;
    insert_series(pool, 101, 3, 6, 2, "trail-physics-v1.1", 0.30).await;
    insert_series(pool, 102, 6, 12, 1, "trail-physics-v1", 0.20).await;
    insert_series(pool, 104, 2, 3, 1, "trail-physics-v1", 0.40).await;
}

const BASE: &str = "/edr/collections/trails";

fn csv(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

#[tokio::test]
#[ignore]
async fn batch_endpoint_end_to_end() {
    let e = env().await;
    seed(&e.pool).await;

    // --- acceptance 1: a batch of N known ids == N single calls --------------
    let known = [101_i64, 102, 103, 104];
    let batch = get_path(&e.app, &format!("{BASE}/conditions?ids={}", csv(&known))).await;
    assert_eq!(batch.status, 200, "{}", batch.text);
    assert_eq!(batch.content_type.as_deref(), Some("application/json"));
    assert_eq!(batch.cache_control.as_deref(), Some("max-age=300"));
    let body = batch.json();
    let series = body["series"].as_array().unwrap();
    assert_eq!(series.len(), known.len());
    assert_eq!(body["unknown_ids"], serde_json::json!([]), "always present");

    for (i, id) in known.iter().enumerate() {
        let single = get_path(&e.app, &format!("{BASE}/items/{id}/conditions")).await;
        assert_eq!(single.status, 200);
        assert_eq!(single.cache_control.as_deref(), Some("max-age=300"));
        assert_eq!(
            serde_json::to_string(&series[i]).unwrap(),
            single.text,
            "series[{i}] (id {id}) is not byte-for-byte the single-trail body"
        );
    }

    // the seed is non-trivial: stitched across runs, ascending, own run_time per point
    let s101 = &series[0];
    assert_eq!(s101["name"], "Forsythe Canyon Trail");
    let pts = s101["conditions"].as_array().unwrap();
    assert!(pts.len() >= 20, "{} points", pts.len());
    let times: Vec<&str> = pts
        .iter()
        .map(|p| p["valid_time"].as_str().unwrap())
        .collect();
    let mut sorted = times.clone();
    sorted.sort();
    assert_eq!(times, sorted, "ascending valid_time");
    let versions: std::collections::HashSet<&str> = pts
        .iter()
        .map(|p| p["model_version"].as_str().unwrap())
        .collect();
    assert_eq!(versions.len(), 2, "both runs contribute: {versions:?}");
    assert_eq!(
        s101["model_version"], "trail-physics-v1.1",
        "newest contributing run"
    );
    assert!(series[1]["name"].is_null(), "unnamed trail -> name: null");

    // --- acceptance 2: mixed batch (known + unknown + out of coverage) -------
    let mixed = get_path(
        &e.app,
        &format!("{BASE}/conditions?ids=777,103,101,888,102"),
    )
    .await;
    assert_eq!(mixed.status, 200, "{}", mixed.text);
    let m = mixed.json();
    let order: Vec<i64> = m["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["feature_id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        order,
        [103, 101, 102],
        "request order, unknown ids omitted from series"
    );
    assert_eq!(m["unknown_ids"], serde_json::json!([777, 888]));
    let out_of_coverage = &m["series"][0];
    assert_eq!(
        out_of_coverage["conditions"],
        serde_json::json!([]),
        "present, not omitted"
    );
    assert!(out_of_coverage["run_time"].is_null());
    assert!(out_of_coverage["model_version"].is_null());
    assert_eq!(out_of_coverage["name"], "Out Of Coverage");

    // all unknown -> still 200
    let none = get_path(&e.app, &format!("{BASE}/conditions?ids=777,888")).await;
    assert_eq!(none.status, 200);
    assert_eq!(none.json()["series"], serde_json::json!([]));
    assert_eq!(none.json()["unknown_ids"], serde_json::json!([777, 888]));

    // --- acceptance 3: the 501-id cap ---------------------------------------
    let ids_501: Vec<i64> = (1..=501).collect();
    let over = get_path(&e.app, &format!("{BASE}/conditions?ids={}", csv(&ids_501))).await;
    assert_eq!(over.status, 400);
    let err = over.json();
    assert_eq!(err["status"], 400, "OGC JSON error body: {}", over.text);
    assert!(
        err["detail"].as_str().unwrap().contains("501"),
        "{}",
        over.text
    );
    let at_cap: Vec<i64> = (1..=500).collect();
    let ok = get_path(&e.app, &format!("{BASE}/conditions?ids={}", csv(&at_cap))).await;
    assert_eq!(ok.status, 200, "exactly 500 is allowed");
    assert_eq!(
        ok.json()["unknown_ids"].as_array().unwrap().len(),
        496,
        "ids 101-104 exist, the other 496 do not"
    );

    // --- malformed ids -> 400 JSON ------------------------------------------
    for q in [
        "ids=abc",
        "ids=101,x,102",
        "ids=-5",
        "ids=1.5",
        "ids=99999999999999999999",
        "ids=",
        "ids=,,",
        "",
        "other=1",
    ] {
        let r = get_path(&e.app, &format!("{BASE}/conditions?{q}")).await;
        assert_eq!(r.status, 400, "{q:?}: {}", r.text);
        assert_eq!(r.content_type.as_deref(), Some("application/json"), "{q:?}");
        assert_eq!(r.json()["status"], 400, "{q:?}");
    }

    // --- request-shape details ----------------------------------------------
    // duplicates collapse (first wins), trailing comma tolerated
    let dup = get_path(&e.app, &format!("{BASE}/conditions?ids=104,101,104,"))
        .await
        .json();
    let order: Vec<i64> = dup["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["feature_id"].as_i64().unwrap())
        .collect();
    assert_eq!(order, [104, 101]);
    // key is case-insensitive; an encoded comma is a separator
    let enc = get_path(&e.app, &format!("{BASE}/conditions?IDS=101%2C102")).await;
    assert_eq!(enc.status, 200, "{}", enc.text);
    assert_eq!(enc.json()["series"].as_array().unwrap().len(), 2);

    // --- collection checks match the single-trail endpoint -------------------
    let missing = get_path(&e.app, "/edr/collections/nope/conditions?ids=1").await;
    assert_eq!(missing.status, 404);
    let wrong_kind = get_path(&e.app, "/edr/collections/hrrr-soil/conditions?ids=1").await;
    assert_eq!(wrong_kind.status, 400);
    assert!(wrong_kind.json()["detail"]
        .as_str()
        .unwrap()
        .contains("not a linear-features"));

    // --- a database failure is a 500 that leaks nothing ----------------------
    e.pool.close().await;
    let dead = get_path(&e.app, &format!("{BASE}/conditions?ids=101")).await;
    assert_eq!(dead.status, 500);
    assert_eq!(dead.json()["detail"], "Internal error");
}
