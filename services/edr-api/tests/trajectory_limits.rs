//! Trajectory and corridor queries through the REAL handlers, with a real HRRR message ingested
//! into Postgres + MinIO. `#[ignore]`d (Docker).
//!
//! ```text
//! cargo test -p edr-api --test trajectory_limits -- --ignored --nocapture
//! ```
//!
//! - a path of more than `max_trajectory_points` waypoints is a 400, one of exactly that many is not;
//! - a corridor longer than `max_corridor_length_km` is a 400;
//! - a plain LINESTRING with a datetime interval (or list) is a 400, a single instant is not;
//! - the waypoints are read concurrently and come back in path order.

use std::path::Path;
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
use grid_processor::{DatasetQuery, GridDataService, MinioConfig};
use ingestion::{IngestOptions, Ingester};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog, ObjectStorage, ObjectStorageConfig,
};
use test_utils::containers::TestInfrastructure;

const FIXTURE: &str = "hrrr/hrrr_20261009_23z_f002.grib2";

struct Env {
    app: Router,
    state: Arc<AppState>,
    _infra: TestInfrastructure,
}

async fn setup() -> Env {
    std::env::set_var(
        "CONFIG_DIR",
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"),
    );
    let infra = TestInfrastructure::start().await;
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("bucket");

    let catalog = Arc::new(Catalog::connect(&infra.postgres_url()).await.unwrap());
    catalog.migrate().await.unwrap();
    catalog.migrate_observations().await.unwrap();
    catalog.migrate_lightning().await.unwrap();
    catalog.migrate_linear_features().await.unwrap();
    catalog.migrate_segment_conditions().await.unwrap();

    let storage = Arc::new(
        ObjectStorage::new(&ObjectStorageConfig {
            endpoint: infra.minio_url(),
            bucket: "test-bucket".to_string(),
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
            allow_http: true,
        })
        .unwrap(),
    );
    let ingester = Ingester::new(Arc::clone(&storage), (*catalog).clone());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures")
        .join(FIXTURE);
    let r = ingester
        .ingest_file(&fixture.to_string_lossy(), IngestOptions::default())
        .await
        .expect("HRRR fixture");
    assert_eq!((r.model.as_str(), r.datasets_registered), ("hrrr", 4));

    let grid_data_service = GridDataService::new(
        Arc::clone(&catalog),
        MinioConfig {
            endpoint: infra.minio_url(),
            bucket: "test-bucket".to_string(),
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
            allow_http: true,
        },
        256,
    )
    .unwrap();

    let state = Arc::new(AppState {
        observation_catalog: Arc::new(ObservationCatalog::new(catalog.pool_clone())),
        storm_event_catalog: Arc::new(StormEventCatalog::new(catalog.pool_clone())),
        linear_feature_catalog: Arc::new(LinearFeatureCatalog::new(catalog.pool_clone())),
        segment_conditions_catalog: Arc::new(SegmentConditionsCatalog::new(catalog.pool_clone())),
        lightning_catalog: Arc::new(LightningCatalog::new(catalog.pool_clone())),
        nws_alert_catalog: Arc::new(storage::NwsAlertCatalog::new(catalog.pool_clone())),
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
    let app = Router::new()
        .route(
            "/edr/collections/:collection_id/trajectory",
            get(handlers::trajectory::trajectory_handler),
        )
        .route(
            "/edr/collections/:collection_id/corridor",
            get(handlers::corridor::corridor_handler),
        )
        .layer(Extension(Arc::clone(&state)));
    Env {
        app,
        state,
        _infra: infra,
    }
}

async fn get_json(app: &Router, uri: &str) -> (u16, Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `n` waypoints zigzagging over the HRRR domain, so neighbouring values differ.
fn path(n: usize) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let f = i as f64 / n.max(2) as f64;
            let lon = -120.0 + 45.0 * f;
            let lat = 28.0 + 18.0 * ((i * 7 % 10) as f64 / 10.0);
            (lon, lat)
        })
        .collect()
}

fn wkt(kind: &str, pts: &[(f64, f64)], extra: impl Fn(usize) -> String) -> String {
    let body: Vec<String> = pts
        .iter()
        .enumerate()
        .map(|(i, (lon, lat))| format!("{lon:.4}%20{lat:.4}{}", extra(i)))
        .collect();
    format!("{kind}({})", body.join(","))
}

fn trajectory_uri(coords: &str, extra: &str) -> String {
    format!(
        "/edr/collections/hrrr-atmosphere/trajectory?coords={coords}&parameter-name=PWAT{extra}"
    )
}

// The grid reader blocks internally (block_in_place), which needs a multi-threaded runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn a_trajectory_is_limited_in_points_and_time_and_read_in_order() {
    let env = setup().await;
    let app = &env.app;

    // ---- points: the configured maximum (100 unless the model sets one) ----
    let (status, body) = get_json(
        app,
        &trajectory_uri(&wkt("LINESTRING", &path(100), |_| String::new()), ""),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = get_json(
        app,
        &trajectory_uri(&wkt("LINESTRING", &path(101), |_| String::new()), ""),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    let detail = body.to_string();
    assert!(
        detail.contains("101") && detail.contains("100"),
        "the message says what was given and what is allowed: {detail}"
    );

    // ---- time: a plain LINESTRING is sampled at one time ----
    let line = wkt("LINESTRING", &path(5), |_| String::new());
    let instant = "&datetime=2026-10-10T01:00:00Z";
    let (status, body) = get_json(app, &trajectory_uri(&line, instant)).await;
    assert_eq!(status, 200, "a single instant is fine: {body}");
    for datetime in [
        "2026-10-10T00:00:00Z/2026-10-10T03:00:00Z",
        "2026-10-10T00:00:00Z/..",
        "2026-10-10T01:00:00Z,2026-10-10T02:00:00Z",
    ] {
        let (status, body) = get_json(
            app,
            &trajectory_uri(&line, &format!("&datetime={datetime}")),
        )
        .await;
        assert_eq!(status, 400, "{datetime}: {body}");
        assert!(
            body.to_string().contains("LINESTRINGM"),
            "the message says what to do instead: {body}"
        );
    }
    // with M, the times are in the coordinates and the interval is not allowed (as before)
    let epoch = 1_791_594_000; // 2026-10-10T01:00:00Z
    let timed = wkt("LINESTRINGM", &path(5), |_| format!("%20{epoch}"));
    let (status, body) = get_json(app, &trajectory_uri(&timed, "")).await;
    assert_eq!(status, 200, "{body}");

    // ---- order: 80 waypoints, each read on its own task, come back in path order ----
    let pts = path(80);
    let timed = wkt("LINESTRINGM", &pts, |_| format!("%20{epoch}"));
    let (status, body) = get_json(app, &trajectory_uri(&timed, "")).await;
    assert_eq!(status, 200, "{body}");
    let got: Vec<Option<f64>> = body["ranges"]["PWAT"]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values: {body}"))
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(got.len(), pts.len());
    for (i, (lon, lat)) in pts.iter().enumerate() {
        // the coordinates are sent with 4 decimals
        let (lon, lat) = (
            format!("{lon:.4}").parse::<f64>().unwrap(),
            format!("{lat:.4}").parse::<f64>().unwrap(),
        );
        let q = DatasetQuery::forecast("hrrr", "PWAT").at_level("entire atmosphere");
        let want = env
            .state
            .grid_data_service
            .read_point(&q, lon, lat)
            .await
            .ok()
            .and_then(|p| p.value)
            .map(f64::from);
        assert_eq!(
            got[i].map(|v| v as f32),
            want.map(|v| v as f32),
            "waypoint {i}"
        );
    }
    let mut distinct: Vec<u64> = got.iter().flatten().map(|v| v.to_bits()).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() > 10,
        "not vacuous: the waypoints see {} different values",
        distinct.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn a_corridor_is_limited_in_length() {
    let env = setup().await;
    let corridor = |pts: &[(f64, f64)]| {
        format!(
            "/edr/collections/hrrr-atmosphere/corridor?coords={}&corridor-width=2&width-units=km\
             &parameter-name=PWAT",
            wkt("LINESTRING", pts, |_| String::new())
        )
    };
    // about 1,100 km
    let (status, body) = get_json(&env.app, &corridor(&[(-105.0, 36.0), (-105.0, 46.0)])).await;
    assert_eq!(status, 200, "{body}");
    // about 3,300 km
    let (status, body) = get_json(
        &env.app,
        &corridor(&[(-120.0, 35.0), (-100.0, 35.0), (-80.0, 35.0)]),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("2000"), "{body}");
}
