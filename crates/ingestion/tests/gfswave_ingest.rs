//! End-to-end ingest of a real GFS-Wave file through the real `Ingester`, the real
//! `config/models` directory, Postgres and MinIO. `#[ignore]`d like the other Docker tests:
//!
//! ```text
//! cargo test -p ingestion --test gfswave_ingest -- --ignored --nocapture
//! ```
//!
//! Until this existed GFS-Wave had never produced a dataset: the downloader saves files as
//! `gfswave_*.grib2`, the ingester read that name as model `gfs`, the GFS parameter table has no
//! oceanographic (discipline 10) entries, and every file "succeeded" with zero datasets.
//!
//! The fixture is messages 5-8 (HTSGW, PERPW, DIRPW, WVHGT) of
//! `gfs.20261025/12/wave/gridded/gfswave.t12z.global.0p25.f006.grib2`, cut by byte range.

use std::path::Path;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ingestion::{IngestOptions, Ingester};
use storage::{Catalog, ObjectStorage, ObjectStorageConfig};
use test_utils::containers::TestInfrastructure;

const FILE: &str = "gfswave_20261025_12z_f006.grib2";

#[tokio::test]
#[ignore] // Requires Docker
async fn a_gfswave_file_registers_its_wave_parameters_under_gfswave() {
    // Process-wide: the GRIB tables are built from this directory on first use.
    std::env::set_var(
        "CONFIG_DIR",
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"),
    );

    let infra = TestInfrastructure::start().await;
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("bucket");
    let catalog = Catalog::connect(&infra.postgres_url()).await.unwrap();
    catalog.migrate().await.unwrap();
    let pool = catalog.pool_clone();
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
    let ingester = Ingester::new(Arc::clone(&storage), catalog.clone());

    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gfswave")
        .join(FILE);
    let result = ingester
        .ingest_file(file.to_str().unwrap(), IngestOptions::default())
        .await
        .expect("ingest");

    assert_eq!(
        result.model, "gfswave",
        "the gfswave_ prefix selects the model"
    );
    assert_eq!(
        result.reference_time,
        Utc.with_ymd_and_hms(2026, 10, 25, 12, 0, 0).unwrap()
    );
    let mut params = result.parameters.clone();
    params.sort();
    assert_eq!(params, ["DIRPW", "HTSGW", "PERPW", "WVHGT"]);
    assert_eq!(result.datasets_registered, 4);

    let rows: Vec<(String, String, String, i32, String)> = sqlx::query_as(
        "SELECT model, parameter, level, forecast_hour, storage_path FROM datasets \
         ORDER BY parameter",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4, "{rows:?}");
    for (model, parameter, _level, forecast_hour, storage_path) in &rows {
        assert_eq!(model, "gfswave", "{parameter}");
        assert_eq!(*forecast_hour, 6, "{parameter}");
        assert!(
            storage_path.starts_with("grids/gfswave/20261025_12z/"),
            "{storage_path}"
        );
    }
}
