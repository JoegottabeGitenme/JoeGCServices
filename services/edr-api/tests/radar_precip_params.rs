//! End-to-end: HRRR composite reflectivity / total cloud cover / sea-level pressure and MRMS
//! precipitation type / hail size, from REAL messages through the REAL ingester, Postgres, MinIO and
//! the REAL EDR position handler. `#[ignore]`d (Docker):
//!
//! ```text
//! cargo test -p edr-api --test radar_precip_params -- --ignored --nocapture
//! ```
//!
//! Each of these was a silent production failure or would have been:
//! - HRRR publishes REFC and TCDC at GRIB2 level type 10; the config said 200, so the ingestion
//!   filter dropped every message and HRRR had no reflectivity or cloud cover;
//! - HRRR publishes sea-level pressure as MSLMA (number 198), not PRMSL (number 1);
//! - MRMS PrecipFlag is categorical: a point read must not interpolate between codes;
//! - the code-102 level label is shared by every MRMS parameter, and MESH is a 500 m product.
//!
//! Ground truth was read with `wgrib2` straight from the same fixture files.

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

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures")
        .join(rel)
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

/// The single value of `param` in a CoverageJSON point response: `Some(v)`, or `None` for null.
fn point_value(body: &Value, param: &str) -> Option<f64> {
    let values = body["ranges"][param]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values for {param}: {body}"));
    assert_eq!(values.len(), 1, "{param}: {body}");
    values[0].as_f64()
}

async fn point(app: &Router, collection: &str, param: &str, lon: f64, lat: f64) -> Option<f64> {
    let (status, body) = get_json(
        app,
        &format!(
            "/edr/collections/{collection}/position?coords=POINT({lon}%20{lat})&parameter-name={param}"
        ),
    )
    .await;
    assert_eq!(status, 200, "{collection}/{param} at ({lon},{lat}): {body}");
    point_value(&body, param)
}

/// `(model, parameter, level, forecast_hour)` of every available dataset.
async fn catalog_rows(pool: &sqlx::PgPool) -> Vec<(String, String, String, i32)> {
    use sqlx::Row;
    sqlx::query(
        "SELECT model, parameter, level, forecast_hour FROM datasets \
         WHERE status = 'available' ORDER BY model, parameter",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
    .collect()
}

/// The longitude/latitude at which the point reader lands on fractional grid index `(x, y)` of
/// `model`/`param` (index space: integer = a cell centre, `y` counted from the first stored row,
/// which for these north-first grids is the northern edge).
///
/// Cells are addressed by index so the test does not depend on how the reader converts degrees to
/// indices. That conversion drifts from the true cell centres (resolution = span / cell count rather
/// than / (count - 1), up to one cell at the eastern edge); a separate issue, deliberately not part
/// of what these assertions pin down.
async fn lonlat_of_index(state: &AppState, model: &str, param: &str, x: f64, y: f64) -> (f64, f64) {
    let meta = state
        .grid_data_service
        .get_metadata(&grid_processor::DatasetQuery::observation(model, param))
        .await
        .expect("grid metadata");
    let (res_x, res_y) = meta.resolution();
    (meta.bbox.min_lon + x * res_x, meta.bbox.max_lat - y * res_y)
}

async fn setup() -> (Router, Arc<AppState>, TestInfrastructure) {
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

    // Real data, real ingester. The expected counts are the point of the first assertions.
    let hrrr = ingester
        .ingest_file(
            &fixture("hrrr/hrrr_20261009_23z_f002.grib2"),
            IngestOptions::default(),
        )
        .await
        .expect("HRRR fixture");
    assert_eq!(hrrr.model, "hrrr");
    let mut got = hrrr.parameters.clone();
    got.sort();
    assert_eq!(
        got,
        ["MSLMA", "PWAT", "REFC", "TCDC"],
        "all four messages must ingest; REFC/TCDC used to be dropped (level 10 vs 200) and MSLMA \
         was configured under the wrong name"
    );
    assert_eq!(hrrr.datasets_registered, 4);
    for f in [
        "mrms/mrms_MRMS_PrecipFlag_00.00_20261009-230000.grib2.gz",
        "mrms/mrms_MRMS_MESH_00.50_20261009-230041.grib2.gz",
    ] {
        let r = ingester
            .ingest_file(&fixture(f), IngestOptions::default())
            .await
            .unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(
            (r.model.as_str(), r.datasets_registered),
            ("mrms", 1),
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
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn hrrr_reflectivity_cloud_cover_and_sea_level_pressure_and_mrms_type_and_hail() {
    let (app, state, _infra) = setup().await;
    let pool = state.catalog.pool_clone();

    // ---- what the catalog holds ---------------------------------------------------------------
    let rows = catalog_rows(&pool).await;
    let has = |model: &str, param: &str, level: &str| {
        rows.iter()
            .any(|(m, p, l, _)| m == model && p == param && l == level)
    };
    assert!(has("hrrr", "REFC", "entire atmosphere"), "{rows:?}");
    assert!(has("hrrr", "TCDC", "entire atmosphere"), "{rows:?}");
    assert!(has("hrrr", "MSLMA", "mean sea level"), "{rows:?}");
    assert!(
        has("hrrr", "PWAT", "entire atmosphere"),
        "PWAT (level 200) must keep ingesting next to the level-10 parameters: {rows:?}"
    );
    assert!(
        !rows.iter().any(|(_, p, _, _)| p == "PRMSL"),
        "HRRR has no PRMSL: {rows:?}"
    );
    assert!(rows.iter().filter(|r| r.0 == "hrrr").all(|r| r.3 == 2));
    // The MRMS level label is shared by every MRMS parameter; MESH is a 500 m product.
    assert!(has("mrms", "PRECIP_FLAG", "0 m above MSL"), "{rows:?}");
    assert!(has("mrms", "MESH", "500 m above MSL"), "{rows:?}");

    // ---- HRRR through EDR (Denver; wgrib2 on the fixture: REFC -1, MSLMA 99907, PWAT 12.55, TCDC 100)
    let (lon, lat) = (-104.99, 39.74);
    let refc = point(&app, "hrrr-atmosphere", "REFC", lon, lat).await;
    assert!(
        refc.is_some_and(|v| (-10.0..=80.0).contains(&v)),
        "REFC (dBZ) {refc:?}"
    );
    let tcdc = point(&app, "hrrr-atmosphere", "TCDC", lon, lat)
        .await
        .expect("TCDC");
    assert!((tcdc - 100.0).abs() < 1.0, "TCDC {tcdc}");
    let mslma = point(&app, "hrrr-mean-sea-level", "MSLMA", lon, lat)
        .await
        .expect("MSLMA");
    assert!((mslma - 99907.0).abs() < 400.0, "MSLMA {mslma} Pa");
    let pwat = point(&app, "hrrr-atmosphere", "PWAT", lon, lat)
        .await
        .expect("PWAT");
    assert!((pwat - 12.55).abs() < 3.0, "PWAT {pwat}");

    // ---- MRMS precipitation type: values -----------------------------------------------------------
    // Boulder: covered, dry.            Mexico: outside radar coverage (-3).
    assert_eq!(
        point(&app, "mrms-single-level", "PRECIP_FLAG", -105.27, 40.01).await,
        Some(0.0)
    );
    assert_eq!(
        point(&app, "mrms-single-level", "PRECIP_FLAG", -100.0, 22.0).await,
        None,
        "-3 (no radar coverage) must come back as null, not as a precipitation code"
    );

    // ---- ... and it is read from the NEAREST cell, never interpolated ---------------------------
    // File-native cells (col 2918, row 2529) = code 0 and (col 2919, row 2529) = code 7, hail
    // (wgrib2 on the fixture). Bilinear would answer 2.1 at 30% of the way and 4.9 at 70%.
    let at = |fx: f64| lonlat_of_index(&state, "mrms", "PRECIP_FLAG", 2918.0 + fx, 2529.0);
    let (lon, lat) = at(0.0).await;
    assert_eq!(
        point(&app, "mrms-single-level", "PRECIP_FLAG", lon, lat).await,
        Some(0.0),
        "the 0 cell itself"
    );
    let (lon, lat) = at(1.0).await;
    assert_eq!(
        point(&app, "mrms-single-level", "PRECIP_FLAG", lon, lat).await,
        Some(7.0),
        "the 7 (hail) cell itself"
    );
    let mut seen = std::collections::BTreeSet::new();
    for tenth in 1..=9 {
        let (lon, lat) = at(tenth as f64 / 10.0).await;
        let v = point(&app, "mrms-single-level", "PRECIP_FLAG", lon, lat)
            .await
            .unwrap_or_else(|| panic!("null at {tenth}/10 of the way"));
        assert!(
            v == 0.0 || v == 7.0,
            "{tenth}/10 of the way from a 0 cell to a 7 cell the reader said {v}: that is not a code"
        );
        seen.insert(v as i32);
    }
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        [0, 7],
        "the answer must switch from one cell to the other somewhere along the segment"
    );

    // ---- MESH: values, sentinels, and that ordinary quantities are STILL interpolated -------------
    // File-native cell (col 2921, row 2527) is the 19.3 mm hail pixel; its east neighbour is 16.2 mm.
    let (lon, lat) = lonlat_of_index(&state, "mrms", "MESH", 2921.0, 2527.0).await;
    let at_cell = point(&app, "mrms-single-level", "MESH", lon, lat)
        .await
        .expect("MESH at the hail cell");
    assert!((at_cell - 19.3).abs() < 0.01, "{at_cell}");
    assert_eq!(
        point(&app, "mrms-single-level", "MESH", -105.27, 40.01).await,
        Some(-1.0),
        "-1 = covered, no hail, is kept"
    );
    assert_eq!(
        point(&app, "mrms-single-level", "MESH", -100.0, 22.0).await,
        None,
        "-3 = no coverage is null"
    );
    let (lon, lat) = lonlat_of_index(&state, "mrms", "MESH", 2921.5, 2527.0).await;
    let between = point(&app, "mrms-single-level", "MESH", lon, lat)
        .await
        .expect("MESH between two hail cells");
    assert!(
        (between - 17.75).abs() < 0.1,
        "halfway between 19.3 and 16.2 mm a quantity is interpolated: {between}"
    );
    // The same parameters answer on the latest-only collection.
    assert_eq!(
        point(
            &app,
            "mrms-single-level-latest",
            "PRECIP_FLAG",
            -105.27,
            40.01
        )
        .await,
        Some(0.0)
    );
}
