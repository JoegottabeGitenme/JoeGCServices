//! End-to-end: a point read on a regular lat/lon grid answers the value of the grid point it asks
//! for. REAL GRIB2 files through the REAL ingester, Postgres, MinIO and the REAL EDR position
//! handler. `#[ignore]`d (Docker):
//!
//! ```text
//! cargo test -p edr-api --test latlon_point_reads -- --ignored --nocapture
//! ```
//!
//! The grid's stored bbox runs from its first point to its last (GFS-Wave 0..359.75 x 90..-90
//! over 1440 x 721; MRMS -129.995..-60.005 x 54.995..20.005 over 7000 x 3500), but the reader
//! divided that extent by the number of points instead of the number of gaps. Every read landed
//! up to a cell toward the far (east and south) edge of the grid: ~0.8 cell east at 300E on the
//! 0.25 degree global grids (GFS, AIGFS, GFS-Wave), ~0.7 cell at 79W on MRMS.
//!
//! Each point is an exact grid point whose neighbours differ, so a bilinear read there must
//! return the point's own value and anything shifted shows. Ground truth is `wgrib2 -lon` on the
//! same files.

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

fn fixture(dir: &str, name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures")
        .join(dir)
        .join(name)
        .to_string_lossy()
        .to_string()
}

async fn value_at(app: &Router, collection: &str, param: &str, lon: f64, lat: f64, t: &str) -> f64 {
    let uri = format!(
        "/edr/collections/{collection}/position?coords=POINT({lon}%20{lat})\
         &parameter-name={param}&datetime={t}"
    );
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 << 20)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, 200, "{uri}: {body}");
    body["ranges"][param]["values"][0]
        .as_f64()
        .unwrap_or_else(|| panic!("{uri}: no value in {body}"))
}

async fn setup() -> (Router, TestInfrastructure) {
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
    for (dir, file, model) in [
        ("gfswave", "gfswave_20261025_12z_f006.grib2", "gfswave"),
        (
            "mrms",
            "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz",
            "mrms-qpe",
        ),
    ] {
        let r = ingester
            .ingest_file(&fixture(dir, file), IngestOptions::default())
            .await
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(r.model, model, "{file}");
        assert!(r.datasets_registered > 0, "{file}: nothing registered");
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
    (app, infra)
}

// The grid reader blocks internally (block_in_place), which needs a multi-threaded runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn a_point_read_on_a_lat_lon_grid_returns_that_grid_points_value() {
    let (app, _infra) = setup().await;

    // GFS-Wave significant wave height, 12Z + 6 h. Neighbours one cell east: 2.04 and 2.67 m.
    let t = "2026-10-25T18:00:00Z";
    for (lon, lat, want) in [(-140.0, 30.0, 2.11), (-60.0, 40.0, 2.81)] {
        let got = value_at(&app, "gfswave-surface", "HTSGW", lon, lat, t).await;
        assert!(
            (got - want).abs() < 0.005,
            "gfswave HTSGW at ({lon}, {lat}): got {got}, wgrib2 says {want}"
        );
    }

    // MRMS hourly QPE (Pass2) ending 15Z. Neighbours of the first point: 18.5 (E), 12.7 (W),
    // 21.0 (N), 11.8 (S) mm.
    let t = "2026-10-09T15:00:00Z";
    for (lon, lat, want) in [(-79.345, 29.955, 16.0), (-79.985, 30.275, 11.8)] {
        let got = value_at(&app, "mrms-qpe", "QPE_01H", lon, lat, t).await;
        assert!(
            (got - want).abs() < 0.05,
            "mrms-qpe QPE_01H at ({lon}, {lat}): got {got}, wgrib2 says {want}"
        );
    }
}
