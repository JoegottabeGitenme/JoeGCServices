//! End-to-end ingest of real MRMS QPE granules through the real `Ingester`, the real
//! `config/models` directory, Postgres and MinIO. `#[ignore]`d like the other Docker tests:
//!
//! ```text
//! cargo test -p ingestion --test mrms_qpe_ingest -- --ignored --nocapture
//! ```
//!
//! What this pins down (each was a real way for `mrms-qpe` to silently ingest nothing):
//! - a `mrms-qpe_`-prefixed file becomes model `mrms-qpe`, not `mrms`;
//! - Pass2 (GRIB2 number 37) and Pass1 (number 30) hourly QPE both become `QPE_01H`;
//! - an hour's Pass1 grid and Pass2 grid share one dataset key and one storage path, so a
//!   late Pass2 file replaces the Pass1 fallback (and `source_file` says which is there).

use std::path::Path;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ingestion::{IngestOptions, Ingester};
use storage::{Catalog, ObjectStorage, ObjectStorageConfig};
use test_utils::containers::TestInfrastructure;

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mrms")
        .join(name)
        .to_string_lossy()
        .to_string()
}

const PASS1: &str = "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass1_00.00_20261009-150000.grib2.gz";
const PASS2: &str = "mrms-qpe_MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz";

#[derive(Debug)]
struct Row {
    model: String,
    parameter: String,
    reference_time: chrono::DateTime<Utc>,
    storage_path: String,
    status: String,
    source_file: Option<String>,
}

async fn rows(pool: &sqlx::PgPool) -> Vec<Row> {
    use sqlx::Row as _;
    sqlx::query(
        "SELECT model, parameter, reference_time, storage_path, status, \
                zarr_metadata->>'source_file' AS source_file \
         FROM datasets ORDER BY model, parameter, reference_time",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| Row {
        model: r.get("model"),
        parameter: r.get("parameter"),
        reference_time: r.get("reference_time"),
        storage_path: r.get("storage_path"),
        status: r.get("status"),
        source_file: r.get("source_file"),
    })
    .collect()
}

#[tokio::test]
#[ignore] // Requires Docker
async fn mrms_qpe_pass1_and_pass2_ingest_as_one_hourly_parameter() {
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
    let hour = Utc.with_ymd_and_hms(2026, 10, 9, 15, 0, 0).unwrap();

    // --- Pass1 alone (the fallback case: Pass2 for this hour never arrived) ----
    let r1 = ingester
        .ingest_file(&fixture(PASS1), IngestOptions::default())
        .await
        .expect("Pass1 ingest");
    assert_eq!(
        r1.model, "mrms-qpe",
        "the mrms-qpe_ prefix selects the model"
    );
    assert_eq!(
        r1.datasets_registered, 1,
        "Pass1 (GRIB2 number 30) must map to a parameter, not be ingested as nothing"
    );
    assert_eq!(r1.parameters, ["QPE_01H"]);
    assert_eq!(r1.reference_time, hour);

    let after_pass1 = rows(&pool).await;
    assert_eq!(after_pass1.len(), 1, "{after_pass1:?}");
    let a = &after_pass1[0];
    assert_eq!(
        (a.model.as_str(), a.parameter.as_str(), a.status.as_str()),
        ("mrms-qpe", "QPE_01H", "available")
    );
    assert_eq!(a.reference_time, hour);
    assert_eq!(
        a.source_file.as_deref(),
        Some(PASS1),
        "provenance names the Pass1 file"
    );
    let path_after_pass1 = a.storage_path.clone();
    assert!(
        path_after_pass1.starts_with("grids/mrms-qpe/20261009_1500z/"),
        "{path_after_pass1}"
    );

    // --- Pass2 for the same hour arrives later: it replaces, it does not add --------
    let r2 = ingester
        .ingest_file(&fixture(PASS2), IngestOptions::default())
        .await
        .expect("Pass2 ingest");
    assert_eq!((r2.model.as_str(), r2.datasets_registered), ("mrms-qpe", 1));
    assert_eq!(r2.parameters, ["QPE_01H"]);

    let after_pass2 = rows(&pool).await;
    assert_eq!(
        after_pass2.len(),
        1,
        "same hour = same dataset key; a second row would double-count the hour: {after_pass2:?}"
    );
    let b = &after_pass2[0];
    assert_eq!(
        b.source_file.as_deref(),
        Some(PASS2),
        "the better Pass2 grid now backs the hour"
    );
    assert_eq!(
        b.storage_path, path_after_pass1,
        "same storage path, overwritten in place"
    );

    // --- Nothing leaked into the radar model ----------------------------------------
    assert!(
        after_pass2.iter().all(|r| r.model != "mrms"),
        "QPE must not be catalogued under the 2-hour radar model"
    );

    // --- The grid really is in object storage ----------------------------------------
    let listed = storage
        .list(&format!("{}/", path_after_pass1.trim_end_matches('/')))
        .await
        .expect("list zarr");
    assert!(!listed.is_empty(), "no objects under {path_after_pass1}");
}
