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
/// Ingest the named fixtures with the real ingester and build the real position route.
/// Returns the router, the state (for direct grid reads) and the infra guard.
async fn setup(files: &[&str]) -> (Router, Arc<AppState>, TestInfrastructure) {
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
    for f in files {
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
        .layer(Extension(Arc::clone(&state)));
    (app, state, infra)
}

// The grid reader blocks internally (block_in_place), which needs a multi-threaded runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore] // Requires Docker
async fn an_hour_with_no_grid_for_a_parameter_is_null_not_a_neighbours_value() {
    let (app, _state, _infra) = setup(&[
        // QPE_01H valid 15:00Z
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz",
        // QPE_24H valid 16:00Z -- an hour QPE_01H has no grid for
        "mrms-qpe_MRMS_MultiSensor_QPE_24H_Pass2_00.00_20261009-160000.grib2.gz",
    ])
    .await;

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

/// The previous per-step behaviour, kept here as the oracle: ask the catalog for the dataset
/// NEAREST to the instant (`find_by_time[_and_level]` via `read_point`), then discard the answer
/// if the grid that came back is not that instant's (the guard the handler already had).
async fn per_step_oracle(
    state: &AppState,
    parameter: &str,
    level: Option<&str>,
    times: &[chrono::DateTime<chrono::Utc>],
    lon: f64,
    lat: f64,
) -> Vec<Option<f32>> {
    let mut out = Vec::new();
    for t in times {
        let mut q =
            grid_processor::DatasetQuery::observation("mrms-qpe", parameter).at_valid_time(*t);
        if let Some(l) = level {
            q = q.at_level(l);
        }
        out.push(
            match state.grid_data_service.read_point(&q, lon, lat).await {
                Ok(p) if (p.time - *t).num_seconds().abs() <= 1 => p.value,
                _ => None,
            },
        );
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn the_batched_concurrent_series_equals_the_per_step_path_on_real_grids() {
    use chrono::{Duration, TimeZone, Utc};
    let (_app, state, _infra) = setup(&[
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-130000.grib2.gz",
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-140000.grib2.gz",
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz",
        "mrms-qpe_MRMS_MultiSensor_QPE_24H_Pass2_00.00_20261009-160000.grib2.gz",
    ])
    .await;
    let h = |hour: u32| Utc.with_ymd_and_hms(2026, 10, 9, hour, 0, 0).unwrap();

    // Out of order, with a repeat, a missing hour (12, 16), sub-second and 30 s offsets, and the
    // far side of the data (00:00 next day).
    let times = vec![
        h(15),
        h(13),
        h(16),
        h(14),
        h(15),
        h(12),
        h(14) + Duration::milliseconds(500),
        h(14) + Duration::seconds(30),
        Utc.with_ymd_and_hms(2026, 10, 10, 0, 0, 0).unwrap(),
    ];
    // Seattle (wet), Boulder, and a point outside the MRMS domain.
    let points = [(-122.3, 47.6), (-105.27, 40.01), (10.0, 50.0)];
    // The level the datasets were stored under, a level they were not, and no level.
    let stored_level: String = {
        let cands = state
            .catalog
            .find_datasets_in_valid_time_range("mrms-qpe", "QPE_01H", None, h(13), h(13))
            .await
            .unwrap();
        cands[0].1.level.clone()
    };
    let levels: [Option<&str>; 3] = [
        None,
        Some(stored_level.as_str()),
        Some("999 m above nothing"),
    ];

    let mut nonnull_seen = 0;
    for (lon, lat) in points {
        for level in levels {
            for param in ["QPE_01H", "QPE_24H"] {
                let want = per_step_oracle(&state, param, level, &times, lon, lat).await;
                let got: Vec<Option<f32>> = state
                    .grid_data_service
                    .read_point_series("mrms-qpe", param, level, &times, lon, lat, 4)
                    .await
                    .into_iter()
                    .zip(&times)
                    .map(|(r, t)| match r {
                        Ok(p) if (p.time - *t).num_seconds().abs() <= 1 => p.value,
                        _ => None,
                    })
                    .collect();
                assert_eq!(got, want, "{param} level={level:?} at ({lon},{lat})");
                nonnull_seen += got.iter().flatten().count();
            }
        }
    }
    // The comparison must not pass vacuously on all-None.
    assert!(
        nonnull_seen > 20,
        "only {nonnull_seen} real values compared"
    );

    // Concurrency must not reorder results: the same request at concurrency 1 and 8 agree.
    let one = state
        .grid_data_service
        .read_point_series("mrms-qpe", "QPE_01H", None, &times, -122.3, 47.6, 1)
        .await;
    let eight = state
        .grid_data_service
        .read_point_series("mrms-qpe", "QPE_01H", None, &times, -122.3, 47.6, 8)
        .await;
    let vals = |v: &[Result<grid_processor::PointValue, _>]| -> Vec<Option<f32>> {
        v.iter()
            .map(|r| r.as_ref().ok().and_then(|p| p.value))
            .collect()
    };
    assert_eq!(vals(&one), vals(&eight));
    // and distinct hours really did return distinct grids' values
    let by_hour = vals(&one);
    assert_ne!(
        by_hour[0], by_hour[1],
        "15:00 and 13:00 must differ at Seattle"
    );
    assert_eq!(
        by_hour[0], by_hour[4],
        "the repeated 15:00 request returns the same value"
    );

    // One catalog lookup for the whole series, however many instants; the per-step path pays
    // one per instant.
    let before = state.grid_data_service.catalog_lookups();
    state
        .grid_data_service
        .read_point_series("mrms-qpe", "QPE_01H", None, &times, -122.3, 47.6, 8)
        .await;
    assert_eq!(state.grid_data_service.catalog_lookups() - before, 1);
    let before = state.grid_data_service.catalog_lookups();
    per_step_oracle(&state, "QPE_01H", None, &times, -122.3, 47.6).await;
    assert_eq!(
        state.grid_data_service.catalog_lookups() - before,
        times.len() as u64,
        "the oracle is the per-step path: one lookup per instant"
    );

    // No requested instants -> nothing, without querying anything.
    let before = state.grid_data_service.catalog_lookups();
    assert!(state
        .grid_data_service
        .read_point_series("mrms-qpe", "QPE_01H", None, &[], 0.0, 0.0, 8)
        .await
        .is_empty());
    assert_eq!(
        state.grid_data_service.catalog_lookups(),
        before,
        "nothing requested, nothing queried"
    );
}

/// The HTTP route must actually take the batched path: a many-hour series is ONE catalog lookup.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn the_position_route_resolves_a_whole_series_with_one_catalog_lookup() {
    let (app, state, _infra) = setup(&[
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-130000.grib2.gz",
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-140000.grib2.gz",
        "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz",
    ])
    .await;
    let before = state.grid_data_service.catalog_lookups();
    let (status, body) = get_json(
        &app,
        "/edr/collections/mrms-qpe/position?coords=POINT(-122.3%2047.6)&parameter-name=QPE_01H\
         &datetime=2026-10-09T13:00:00Z/2026-10-09T15:00:00Z",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (times, values) = series(&body, "QPE_01H");
    assert_eq!(times.len(), 3);
    assert!(values.iter().all(|v| v.is_some()), "{values:?}");
    assert_eq!(
        state.grid_data_service.catalog_lookups() - before,
        1,
        "3 hours must cost one catalog lookup, not three"
    );
}
