//! End-to-end: an hourly QPE point series through the REAL position handler, with real MRMS
//! granules ingested by the REAL ingester into Postgres + MinIO. `#[ignore]`d (Docker).
//!
//! ```text
//! cargo test -p edr-api --test mrms_qpe_series -- --ignored --nocapture
//! ```
//!
//! The situation under test: the series asks every parameter for every time ANY parameter in
//! the model has. Here QPE_24H has a grid for 16:00Z but QPE_01H does not (Pass2 and Pass1
//! both missing, or simply not published yet). The catalog's `find_by_time` would answer with
//! the nearest QPE_01H grid -- 15:00Z's -- and the 16:00Z point would repeat 15:00Z's rain.

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
        .join("../../crates/ingestion/tests/fixtures/mrms")
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

/// `(times, values)` of one parameter's series from a CoverageJSON point series.
fn series(body: &Value, param: &str) -> (Vec<String>, Vec<Option<f64>>) {
    let t = body["domain"]["axes"]["t"]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no t axis: {body}"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let v = body["ranges"][param]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values for {param}: {body}"))
        .iter()
        .map(|v| v.as_f64())
        .collect();
    (t, v)
}

// The grid reader blocks internally (block_in_place), which needs a multi-threaded runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore] // Requires Docker
async fn an_hour_with_no_grid_for_a_parameter_is_null_not_a_neighbours_value() {
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

    // Real granules, through the real ingester.
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
    for f in [
        // QPE_01H valid 15:00Z
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz",
        // QPE_24H valid 16:00Z -- an hour QPE_01H has no grid for
        "mrms-qpe_MRMS_MultiSensor_QPE_24H_Pass2_00.00_20261009-160000.grib2.gz",
    ] {
        let r = ingester
            .ingest_file(&fixture(f), IngestOptions::default())
            .await
            .unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(
            (r.model.as_str(), r.datasets_registered),
            ("mrms-qpe", 1),
            "{f}"
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
        .layer(Extension(state));

    // Seattle, where it rained at 15:00Z (0.47 mm in the live series).
    let q = |param: &str| {
        format!(
            "/edr/collections/mrms-qpe/position?coords=POINT(-122.3%2047.6)&parameter-name={param}\
             &datetime=2026-10-09T15:00:00Z/2026-10-09T16:00:00Z"
        )
    };

    let (status, body) = get_json(&app, &q("QPE_01H")).await;
    assert_eq!(status, 200, "{body}");
    let (times, values) = series(&body, "QPE_01H");
    assert_eq!(times, ["2026-10-09T15:00:00Z", "2026-10-09T16:00:00Z"]);
    let at_15 = values[0].expect("QPE_01H has a real grid for 15:00Z");
    assert!(at_15 > 0.1, "Seattle was wet at 15Z, got {at_15}");
    assert_eq!(
        values[1], None,
        "QPE_01H has no grid for 16:00Z: that point must be null, not 15:00Z's {at_15} again"
    );

    // The same hour is a real value for the parameter that does have it.
    let (status, body) = get_json(&app, &q("QPE_24H")).await;
    assert_eq!(status, 200, "{body}");
    let (times, values) = series(&body, "QPE_24H");
    assert_eq!(times, ["2026-10-09T15:00:00Z", "2026-10-09T16:00:00Z"]);
    assert_eq!(values[0], None, "QPE_24H has no 15:00Z grid");
    assert!(values[1].is_some(), "QPE_24H has a real 16:00Z grid");

    // Both parameters in one request keep their own gaps.
    let both = "/edr/collections/mrms-qpe/position?coords=POINT(-122.3%2047.6)\
                &parameter-name=QPE_01H,QPE_24H&datetime=2026-10-09T15:00:00Z/2026-10-09T16:00:00Z";
    let (status, body) = get_json(&app, both).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(series(&body, "QPE_01H").1[1], None);
    assert_eq!(series(&body, "QPE_24H").1[0], None);
}
