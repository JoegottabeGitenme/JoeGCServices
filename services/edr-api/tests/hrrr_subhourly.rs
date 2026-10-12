//! End-to-end: HRRR 15-minute output (`wrfsubhf`), from REAL messages through the REAL ingester,
//! Postgres, MinIO and the REAL EDR position handler. `#[ignore]`d (Docker):
//!
//! ```text
//! cargo test -p edr-api --test hrrr_subhourly -- --ignored --nocapture
//! ```
//!
//! Before this existed, a file like `wrfsubhf01` (15/30/45/60 minutes of every parameter) could not
//! be ingested correctly at all:
//! - the GRIB2 parser read the forecast time without its unit, so "15 min" meant 15 hours;
//! - the ingester kept the first message of each parameter and dropped the other three;
//! - the dataset key had one slot per hour, and the hour came from the file name;
//! - one storage path per hour, so the four steps would have overwritten each other.
//!
//! The fixtures are real messages from `hrrr.t18z.wrfsubhf{00,01,02}.grib2` of 2026-10-11 (the S3
//! mirror and NOMADS publish identical files). Ground truth is `wgrib2 -ijlat` on the same bytes.

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
use grid_processor::{GridDataService, MinioConfig};
use ingestion::{IngestOptions, Ingester};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog, ObjectStorage, ObjectStorageConfig,
};
use test_utils::containers::TestInfrastructure;

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures/hrrr-subh")
        .join(name)
        .to_string_lossy()
        .to_string()
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

/// `(valid times, values)` of a position series for `param`.
async fn series(
    app: &Router,
    collection: &str,
    param: &str,
    (lon, lat): (f64, f64),
    datetime: &str,
) -> (Vec<String>, Vec<Option<f64>>) {
    let (status, body) = get_json(
        app,
        &format!(
            "/edr/collections/{collection}/position?coords=POINT({lon}%20{lat})\
             &parameter-name={param}&datetime={datetime}"
        ),
    )
    .await;
    assert_eq!(status, 200, "{collection}/{param} {datetime}: {body}");
    let t = body["domain"]["axes"]["t"]["values"]
        .as_array()
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();
    let v = body["ranges"][param]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values for {param}: {body}"))
        .iter()
        .map(|v| v.as_f64())
        .collect();
    (t, v)
}

fn assert_close(got: &[Option<f64>], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let g = g.unwrap_or_else(|| panic!("{what}[{i}] is null, want {w}"));
        assert!(
            (g - w).abs() <= tol,
            "{what}[{i}]: got {g}, want {w} (tol {tol})"
        );
    }
}

async fn setup() -> (Router, Arc<AppState>, Ingester, TestInfrastructure) {
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
    let ingester = Ingester::new(storage, (*catalog).clone());

    // Real data, real ingester. 24 + 6 + 1 messages, each its own dataset.
    for (file, want) in [
        ("hrrr-subh_20261011_18z_f001.grib2", 24),
        ("hrrr-subh_20261011_18z_f000.grib2", 6),
        ("hrrr-subh_20261011_18z_f002.grib2", 1),
    ] {
        let r = ingester
            .ingest_file(&fixture(file), IngestOptions::default())
            .await
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(
            r.model, "hrrr-subh",
            "{file}: the model comes from the file name"
        );
        assert_eq!(
            r.datasets_registered, want,
            "{file}: every message must become a dataset (the first-of-each-parameter rule kept 6 of 24)"
        );
    }

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
    let app = Router::new()
        .route(
            "/edr/collections/:collection_id/position",
            get(handlers::position::position_handler),
        )
        .layer(Extension(Arc::clone(&state)));
    (app, state, ingester, infra)
}

// Two cells of the 3 km grid (wgrib2 -ijlat, 1-based (506,788) and (1667,574)), at their exact
// centres: an Idaho/Montana cell with snow, and a cell over the Atlantic with rain. Chosen because
// the reflectivity there is different at every lead time and the surrounding 3x3 is smooth, so a
// bilinear read at the centre is the cell's own value.
const SNOW: (f64, f64) = (-112.409346, 44.582824);
const RAIN: (f64, f64) = (-71.350535, 36.725810);

// REFC (dBZ) at 0 (analysis), 15, 30, 45, 60 and 75 minutes after the 18Z cycle
const SNOW_REFC: [f64; 6] = [26.19, 25.69, 24.56, 22.81, 21.5, 20.12];
const RAIN_REFC: [f64; 6] = [27.31, 27.25, 27.5, 26.0, 23.81, 17.75];

const ALL_STEPS: [&str; 6] = [
    "2026-10-11T18:00:00Z",
    "2026-10-11T18:15:00Z",
    "2026-10-11T18:30:00Z",
    "2026-10-11T18:45:00Z",
    "2026-10-11T19:00:00Z",
    "2026-10-11T19:15:00Z",
];

// The grid reader blocks internally (block_in_place), which needs a multi-threaded runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn fifteen_minute_steps_are_distinct_datasets_with_the_right_times_and_values() {
    use sqlx::Row;
    let (app, state, ingester, _infra) = setup().await;
    let pool = state.catalog.pool_clone();

    // ---- the catalog: one dataset per message, keyed by hour AND minute ---------------------------
    let rows = sqlx::query(
        "SELECT forecast_hour, forecast_minute, to_char(valid_time AT TIME ZONE 'UTC', \
         'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), storage_path FROM datasets \
         WHERE model = 'hrrr-subh' AND parameter = 'REFC' ORDER BY valid_time",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let got: Vec<(i32, i16, String)> = rows
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();
    let want: Vec<(i32, i16, String)> = [(0, 0), (0, 15), (0, 30), (0, 45), (1, 0), (1, 15)]
        .iter()
        .zip(ALL_STEPS)
        .map(|((h, m), t)| (*h, *m, t.to_string()))
        .collect();
    assert_eq!(
        got, want,
        "hour, minute and valid time of each REFC step (the old parser read 15 minutes as 15 hours)"
    );
    let paths: std::collections::BTreeSet<String> = rows.iter().map(|r| r.get(3)).collect();
    assert_eq!(
        paths.len(),
        6,
        "each step needs its own Zarr path: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("_f000m15.zarr")),
        "{paths:?}"
    );
    assert!(paths.iter().any(|p| p.ends_with("_f001.zarr")), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.ends_with("_f001m15.zarr")),
        "{paths:?}"
    );
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM datasets WHERE model = 'hrrr-subh'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 31);
    // the zarr metadata carries the minute the writer does not
    let meta: Value = sqlx::query_scalar(
        "SELECT zarr_metadata FROM datasets WHERE parameter = 'REFC' AND forecast_hour = 1 \
         AND forecast_minute = 15",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(meta["forecast_minute"], 15, "{meta}");

    // ---- ingesting the same file again changes nothing (the upsert key includes the minute) ------
    ingester
        .ingest_file(
            &fixture("hrrr-subh_20261011_18z_f001.grib2"),
            IngestOptions::default(),
        )
        .await
        .expect("re-ingest");
    let again: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM datasets WHERE model = 'hrrr-subh'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(again, 31, "re-ingesting must upsert, not duplicate or drop");

    // ---- hour-keyed lookups mean the whole hour, not "some step inside it" ------------------------
    let one_hour = state
        .catalog
        .find_by_run_and_forecast_hour(
            "hrrr-subh",
            "REFC",
            chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 11, 18, 0, 0).unwrap(),
            1,
            None,
        )
        .await
        .expect("hour 1");
    assert_eq!(
        (one_hour.forecast_hour, one_hour.forecast_minute),
        (1, 0),
        "forecast_hour=1 is the +60 minute grid, never +75"
    );
    assert_eq!(
        one_hour.valid_time().to_rfc3339(),
        "2026-10-11T19:00:00+00:00"
    );
    let hours = state
        .catalog
        .get_run_forecast_hours(
            "hrrr-subh",
            chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 11, 18, 0, 0).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hours, [0, 1], "whole hours only");

    // ---- EDR: a 15-minute series, values straight from the file ------------------------------------
    let range = "2026-10-11T18:00:00Z/2026-10-11T19:15:00Z";
    for (cell, want, name) in [
        (SNOW, SNOW_REFC, "snow cell"),
        (RAIN, RAIN_REFC, "rain cell"),
    ] {
        let (times, values) = series(&app, "hrrr-subh-atmosphere", "REFC", cell, range).await;
        assert_eq!(
            times, ALL_STEPS,
            "{name}: the time axis is every 15 minutes"
        );
        assert_close(&values, &want, 0.15, &format!("{name} REFC"));
    }

    // one instant: 18:30 is the 30 minute grid, not the 18:00 or 19:00 one
    let (t, v) = series(
        &app,
        "hrrr-subh-atmosphere",
        "REFC",
        SNOW,
        "2026-10-11T18:30:00Z",
    )
    .await;
    assert_eq!(t, ["2026-10-11T18:30:00Z"]);
    assert_close(&v, &[24.56], 0.15, "snow cell REFC at 18:30");

    // precipitation rate (kg/m^2/s; wgrib2 prints 4 decimals) and type
    let rate = "2026-10-11T18:15:00Z/2026-10-11T19:00:00Z";
    let (_, prate) = series(&app, "hrrr-subh-surface", "PRATE", SNOW, rate).await;
    assert_close(
        &prate,
        &[0.0008, 0.0007, 0.0005, 0.0004],
        6e-5,
        "snow cell PRATE",
    );
    for (param, snow, rain) in [
        ("CSNOW", 1.0, 0.0),
        ("CRAIN", 0.0, 1.0),
        ("CFRZR", 0.0, 0.0),
        ("CICEP", 0.0, 0.0),
    ] {
        let (_, v) = series(&app, "hrrr-subh-surface", param, SNOW, rate).await;
        assert_close(&v, &[snow; 4], 0.0, &format!("snow cell {param}"));
        let (_, v) = series(&app, "hrrr-subh-surface", param, RAIN, rate).await;
        assert_close(&v, &[rain; 4], 0.0, &format!("rain cell {param}"));
    }
}
