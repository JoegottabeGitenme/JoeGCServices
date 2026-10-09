//! Integration tests for the storage crate.
//!
//! These tests require Docker and are run with `cargo test -- --ignored`.

use bytes::Bytes;
use chrono::{DateTime, Duration, SubsecRound, Utc};

use storage::{
    Catalog, CatalogEntry, ObjectStorage, ObjectStorageConfig, SegmentConditionsCatalog,
};
use test_utils::containers::TestInfrastructure;
use wms_common::BoundingBox;

/// `Utc::now()` truncated to microseconds, which is all Postgres `timestamptz`
/// stores. Comparing a round-tripped value against a nanosecond `Utc::now()`
/// fails (left `...941725Z`, right `...941725767Z`) on any clock finer than 1 us.
fn now_micros() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Helper to create a test catalog entry.
fn test_entry(model: &str, parameter: &str, forecast_hour: u32) -> CatalogEntry {
    CatalogEntry {
        model: model.to_string(),
        parameter: parameter.to_string(),
        level: "surface".to_string(),
        reference_time: Utc::now() - Duration::hours(1),
        forecast_hour,
        bbox: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
        storage_path: format!("{}/{}/f{:03}.zarr", model, parameter, forecast_hour),
        file_size: 1024,
        zarr_metadata: None,
    }
}

// ============================================================================
// Catalog Integration Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_connect_and_migrate() {
    let infra = TestInfrastructure::start().await;

    // Connect to PostgreSQL
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect to catalog");

    // Run migrations
    catalog.migrate().await.expect("Failed to run migrations");

    // Verify we can query (should be empty)
    let models = catalog.list_models().await.expect("Failed to list models");
    assert!(models.is_empty(), "Expected no models in fresh database");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_register_and_query_dataset() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register a dataset
    let entry = test_entry("gfs", "temperature", 0);
    let id = catalog
        .register_dataset(&entry)
        .await
        .expect("Failed to register dataset");
    assert!(!id.is_nil());

    // List models - should now have "gfs"
    let models = catalog.list_models().await.expect("Failed to list models");
    assert_eq!(models, vec!["gfs"]);

    // List parameters for model
    let params = catalog
        .list_parameters("gfs")
        .await
        .expect("Failed to list parameters");
    assert_eq!(params, vec!["temperature"]);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_find_by_time() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register multiple forecast hours
    for hour in [0, 3, 6, 12] {
        let entry = test_entry("gfs", "wind", hour);
        catalog
            .register_dataset(&entry)
            .await
            .expect("Failed to register");
    }

    // Find by valid time - should get closest
    let result = catalog
        .find_by_time("gfs", "wind", Utc::now())
        .await
        .expect("Failed to find by time");

    assert!(result.is_some());
    let entry = result.unwrap();
    assert_eq!(entry.model, "gfs");
    assert_eq!(entry.parameter, "wind");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_get_latest() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register datasets with different valid times
    for hour in [0, 6, 12] {
        let entry = test_entry("nam", "precipitation", hour);
        catalog
            .register_dataset(&entry)
            .await
            .expect("Failed to register");
    }

    // Get latest
    let result = catalog
        .get_latest("nam", "precipitation")
        .await
        .expect("Failed to get latest");

    assert!(result.is_some());
    let entry = result.unwrap();
    // Latest by valid_time = reference_time + forecast_hour
    assert_eq!(entry.forecast_hour, 12);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_model_stats() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register multiple models and parameters
    for param in ["temp", "wind", "precip"] {
        let entry = test_entry("gfs", param, 0);
        catalog.register_dataset(&entry).await.unwrap();
    }
    for param in ["temp", "wind"] {
        let entry = test_entry("nam", param, 0);
        catalog.register_dataset(&entry).await.unwrap();
    }

    // Get model stats
    let stats = catalog
        .get_model_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.len(), 2);

    let gfs_stats = stats.iter().find(|s| s.model == "gfs").unwrap();
    assert_eq!(gfs_stats.parameter_count, 3);
    assert_eq!(gfs_stats.dataset_count, 3);

    let nam_stats = stats.iter().find(|s| s.model == "nam").unwrap();
    assert_eq!(nam_stats.parameter_count, 2);
    assert_eq!(nam_stats.dataset_count, 2);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_mark_expired_and_delete() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register old and new datasets
    let mut old_entry = test_entry("gfs", "temp", 0);
    old_entry.reference_time = Utc::now() - Duration::days(30);
    catalog.register_dataset(&old_entry).await.unwrap();

    let new_entry = test_entry("gfs", "temp", 6);
    catalog.register_dataset(&new_entry).await.unwrap();

    // Mark entries older than 7 days as expired
    let cutoff = Utc::now() - Duration::days(7);
    let marked = catalog
        .mark_model_expired("gfs", cutoff)
        .await
        .expect("Failed to mark expired");
    assert_eq!(marked, 1);

    // Count expired
    let expired_count = catalog.count_expired().await.expect("Failed to count");
    assert_eq!(expired_count, 1);

    // Get expired paths
    let paths = catalog
        .get_expired_storage_paths()
        .await
        .expect("Failed to get paths");
    assert_eq!(paths.len(), 1);
    assert!(paths[0].contains("f000"));

    // Delete expired
    let deleted = catalog.delete_expired().await.expect("Failed to delete");
    assert_eq!(deleted, 1);

    // Verify only new entry remains
    let count = catalog.count_available().await.expect("Failed to count");
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_get_available_times() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register datasets
    for hour in [0, 3, 6, 9, 12] {
        let entry = test_entry("hrrr", "reflectivity", hour);
        catalog.register_dataset(&entry).await.unwrap();
    }

    // Get available times
    let times = catalog
        .get_available_times("hrrr", "reflectivity")
        .await
        .expect("Failed to get times");

    assert_eq!(times.len(), 5);
}

// ============================================================================
// Object Storage Integration Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_object_storage_put_get() {
    let infra = TestInfrastructure::start().await;

    // Create bucket first (MinIO requires this)
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("Failed to create bucket");

    // Configure object storage
    let config = ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "test-bucket".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    // Create storage client
    let storage = ObjectStorage::new(&config).expect("Failed to create storage");

    // Write data
    let data = Bytes::from("test weather data content");
    storage
        .put("gfs/temp/f000.zarr", data.clone())
        .await
        .expect("Failed to put object");

    // Read back
    let result = storage
        .get("gfs/temp/f000.zarr")
        .await
        .expect("Failed to get object");

    assert_eq!(result, data);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_object_storage_list() {
    let infra = TestInfrastructure::start().await;

    // Create bucket first
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("Failed to create bucket");

    let config = ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "test-bucket".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    let storage = ObjectStorage::new(&config).expect("Failed to create storage");

    // Write multiple files
    for hour in [0, 3, 6] {
        let path = format!("gfs/temp/f{:03}.zarr", hour);
        storage
            .put(&path, Bytes::from("data"))
            .await
            .expect("Failed to put");
    }

    // List files under prefix
    let files = storage.list("gfs/temp/").await.expect("Failed to list");

    assert_eq!(files.len(), 3);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_object_storage_delete() {
    let infra = TestInfrastructure::start().await;

    // Create bucket first
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("Failed to create bucket");

    let config = ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "test-bucket".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    let storage = ObjectStorage::new(&config).expect("Failed to create storage");

    // Write then delete
    storage
        .put("to-delete.txt", Bytes::from("data"))
        .await
        .expect("Failed to put");

    storage
        .delete("to-delete.txt")
        .await
        .expect("Failed to delete");

    // Should fail to read
    let result = storage.get("to-delete.txt").await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_object_storage_exists() {
    let infra = TestInfrastructure::start().await;

    // Create bucket first
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("Failed to create bucket");

    let config = ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "test-bucket".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    let storage = ObjectStorage::new(&config).expect("Failed to create storage");

    // Should not exist
    let exists = storage
        .exists("nonexistent.txt")
        .await
        .expect("Failed to check exists");
    assert!(!exists);

    // Write and check exists
    storage
        .put("exists.txt", Bytes::from("data"))
        .await
        .expect("Failed to put");

    let exists = storage
        .exists("exists.txt")
        .await
        .expect("Failed to check exists");
    assert!(exists);
}

// ============================================================================
// SegmentConditionsCatalog Integration Tests (Session 14)
//
// The Python trail-physics service writes this table directly via psycopg,
// not through this crate -- these tests seed rows with raw sqlx::query
// against the migrated schema, exactly what a real ingest would produce.
// ============================================================================

/// Helper: insert one segment_conditions row with an explicit valid_time
/// (and run_time = valid_time, forecast_hour = 0, unless the caller cares
/// otherwise -- these tests only exercise the "latest" query's ordering,
/// not forecast-hour semantics).
async fn insert_condition_row(
    pool: &sqlx::PgPool,
    feature_id: i64,
    run_time: chrono::DateTime<Utc>,
    valid_time: chrono::DateTime<Utc>,
    soil_moisture: f32,
    model_version: &str,
) {
    sqlx::query(
        r#"INSERT INTO segment_conditions
           (feature_id, run_time, valid_time, forecast_hour, soil_moisture, model_version)
           VALUES ($1, $2, $3, 0, $4, $5)"#,
    )
    .bind(feature_id)
    .bind(run_time)
    .bind(valid_time)
    .bind(soil_moisture)
    .bind(model_version)
    .execute(pool)
    .await
    .expect("Failed to insert segment_conditions row");
}

async fn connected_catalog(infra: &TestInfrastructure) -> Catalog {
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");
    catalog
        .migrate_linear_features()
        .await
        .expect("Failed to migrate linear_features");
    catalog
        .migrate_segment_conditions()
        .await
        .expect("Failed to migrate segment_conditions");
    catalog
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_segment_conditions_latest_orders_by_valid_time_not_insertion_order() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let feature_id: i64 = 999_001;
    let now = now_micros();
    let newer_valid_time = now - Duration::hours(1);
    let older_valid_time = now - Duration::hours(3);

    // The NEWER valid_time row is inserted FIRST (earlier ingested_at),
    // the OLDER valid_time row SECOND (later ingested_at) -- deliberately
    // backwards from what "order by ingested_at" would assume. This is
    // the direct regression test for the real Session 13 bug: the old
    // query (`ORDER BY ingested_at DESC`) would have returned the OLDER
    // row here, since it was written last.
    insert_condition_row(
        &pool,
        feature_id,
        newer_valid_time,
        newer_valid_time,
        0.20,
        "trail-physics-v1",
    )
    .await;
    insert_condition_row(
        &pool,
        feature_id,
        older_valid_time,
        older_valid_time,
        0.10,
        "trail-physics-v1",
    )
    .await;

    let conditions = SegmentConditionsCatalog::new(pool);
    let result = conditions
        .get_latest_for_feature(feature_id)
        .await
        .expect("query failed")
        .expect("expected a row");

    assert_eq!(result.valid_time, newer_valid_time);
    assert_eq!(result.soil_moisture, Some(0.20));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_segment_conditions_latest_excludes_future_valid_times() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let feature_id: i64 = 999_002;
    let now = now_micros();
    let past_valid_time = now - Duration::hours(1);
    // A genuine forecast-hour row for an upcoming hour of the same run --
    // real data, not a bug, but not "current conditions" either.
    let future_valid_time = now + Duration::hours(5);

    insert_condition_row(
        &pool,
        feature_id,
        past_valid_time,
        past_valid_time,
        0.15,
        "trail-physics-v1",
    )
    .await;
    insert_condition_row(
        &pool,
        feature_id,
        past_valid_time,
        future_valid_time,
        0.99,
        "trail-physics-v1",
    )
    .await;

    let conditions = SegmentConditionsCatalog::new(pool);
    let result = conditions
        .get_latest_for_feature(feature_id)
        .await
        .expect("query failed")
        .expect("expected a row");

    assert_eq!(result.valid_time, past_valid_time);
    assert_eq!(result.soil_moisture, Some(0.15));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_segment_conditions_latest_tie_breaks_by_newest_run_time() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let feature_id: i64 = 999_003;
    let now = now_micros();
    let valid_time = now - Duration::hours(1);

    // Same valid_time, two different model_versions -- the only way two
    // rows can share a valid_time given the table's own
    // UNIQUE(feature_id, valid_time, model_version) constraint. Simulates
    // an older initialization's forecast for this hour versus a fresher
    // run's row for the identical hour.
    insert_condition_row(
        &pool,
        feature_id,
        valid_time - Duration::hours(2),
        valid_time,
        0.11,
        "trail-physics-v1",
    )
    .await;
    insert_condition_row(
        &pool,
        feature_id,
        valid_time,
        valid_time,
        0.22,
        "trail-physics-v2",
    )
    .await;

    let conditions = SegmentConditionsCatalog::new(pool);
    let result = conditions
        .get_latest_for_feature(feature_id)
        .await
        .expect("query failed")
        .expect("expected a row");

    assert_eq!(result.soil_moisture, Some(0.22)); // the fresher run_time wins
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_segment_conditions_latest_for_features_batch_applies_same_semantics() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let now = Utc::now();

    let feature_a: i64 = 999_010; // has a real past row -- should appear
    let feature_b: i64 = 999_011; // only has a future row -- should be omitted entirely
    let past_valid_time = now - Duration::hours(2);
    let future_valid_time = now + Duration::hours(3);

    insert_condition_row(
        &pool,
        feature_a,
        past_valid_time,
        past_valid_time,
        0.33,
        "trail-physics-v1",
    )
    .await;
    insert_condition_row(
        &pool,
        feature_b,
        past_valid_time,
        future_valid_time,
        0.77,
        "trail-physics-v1",
    )
    .await;

    let conditions = SegmentConditionsCatalog::new(pool);
    let results = conditions
        .get_latest_for_features(&[feature_a, feature_b])
        .await
        .expect("query failed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].feature_id, feature_a);
    assert_eq!(results[0].soil_moisture, Some(0.33));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_segment_conditions_timeseries_is_one_row_per_hour_newest_run_wins() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let feature_id: i64 = 999_020;
    let now = Utc::now();
    let hour = |h: i64| now + Duration::hours(h);

    // Same valid hour (now+2h), two model versions with DIFFERENT run
    // times -- the only way two rows can share a valid_time under the
    // table's UNIQUE(feature_id, valid_time, model_version). The series must
    // keep exactly one, from the newer run.
    insert_condition_row(
        &pool,
        feature_id,
        hour(-5),
        hour(2),
        0.11,
        "trail-physics-v1",
    )
    .await;
    insert_condition_row(
        &pool,
        feature_id,
        hour(-1),
        hour(2),
        0.22,
        "trail-physics-v2",
    )
    .await;
    // An hour only the older run reaches must still appear (stable horizon).
    insert_condition_row(
        &pool,
        feature_id,
        hour(-5),
        hour(30),
        0.33,
        "trail-physics-v1",
    )
    .await;
    // A different feature must never leak in.
    insert_condition_row(
        &pool,
        feature_id + 1,
        hour(-1),
        hour(2),
        0.99,
        "trail-physics-v1",
    )
    .await;

    let series = SegmentConditionsCatalog::new(pool)
        .get_timeseries_for_feature(feature_id)
        .await
        .expect("query failed");

    assert_eq!(series.len(), 2);
    assert!(series.iter().all(|c| c.feature_id == feature_id));
    assert!(
        series[0].valid_time < series[1].valid_time,
        "ascending valid_time"
    );
    assert_eq!(series[0].soil_moisture, Some(0.22)); // newer run wins the shared hour
    assert_eq!(series[1].soil_moisture, Some(0.33)); // horizon extends to the older run's last hour
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_segment_conditions_timeseries_excludes_history_older_than_the_window() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let feature_id: i64 = 999_021;
    let now = Utc::now();

    insert_condition_row(
        &pool,
        feature_id,
        now - Duration::hours(40),
        now - Duration::hours(30),
        0.10,
        "trail-physics-v1",
    )
    .await;
    insert_condition_row(
        &pool,
        feature_id,
        now - Duration::hours(3),
        now - Duration::hours(2),
        0.20,
        "trail-physics-v1",
    )
    .await;

    let series = SegmentConditionsCatalog::new(pool)
        .get_timeseries_for_feature(feature_id)
        .await
        .expect("query failed");
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].soil_moisture, Some(0.20));
}

// ============================================================================
// get_parameter_availability: single-query rewrite == the old five queries
// ============================================================================

/// The previous implementation, verbatim in SQL: COUNT, then four separate
/// scans. Kept here as an independent oracle for the single-query version.
async fn availability_five_queries(
    pool: &sqlx::PgPool,
    model: &str,
    parameter: &str,
) -> Option<(Vec<String>, Vec<i32>, Vec<String>, (f64, f64, f64, f64))> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM datasets WHERE model = $1 AND parameter = $2 AND status = 'available'",
    )
    .bind(model)
    .bind(parameter)
    .fetch_one(pool)
    .await
    .unwrap();
    if count == 0 {
        return None;
    }
    let times: Vec<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT DISTINCT DATE_TRUNC('minute', reference_time) as ref_time FROM datasets \
         WHERE model = $1 AND parameter = $2 AND status = 'available' ORDER BY ref_time DESC",
    )
    .bind(model)
    .bind(parameter)
    .fetch_all(pool)
    .await
    .unwrap();
    let hours: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT forecast_hour FROM datasets \
         WHERE model = $1 AND parameter = $2 AND status = 'available' ORDER BY forecast_hour ASC",
    )
    .bind(model)
    .bind(parameter)
    .fetch_all(pool)
    .await
    .unwrap();
    let levels: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT level FROM datasets \
         WHERE model = $1 AND parameter = $2 AND status = 'available' ORDER BY level ASC",
    )
    .bind(model)
    .bind(parameter)
    .fetch_all(pool)
    .await
    .unwrap();
    let bbox: (f64, f64, f64, f64) = sqlx::query_as(
        "SELECT MIN(bbox_min_x), MIN(bbox_min_y), MAX(bbox_max_x), MAX(bbox_max_y) FROM datasets \
         WHERE model = $1 AND parameter = $2 AND status = 'available'",
    )
    .bind(model)
    .bind(parameter)
    .fetch_one(pool)
    .await
    .unwrap();
    Some((
        times
            .into_iter()
            .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .collect(),
        hours,
        levels,
        bbox,
    ))
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_parameter_availability_matches_the_five_query_oracle() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("connect");
    catalog.migrate().await.expect("migrate");
    let pool = catalog.pool_clone();

    // Anchor on a whole minute so "+20 s" stays inside the same minute bucket.
    let six_hours_ago = (Utc::now() - Duration::hours(6)).timestamp();
    let base = DateTime::<Utc>::from_timestamp(six_hours_ago - six_hours_ago % 60, 0).unwrap();

    // "multi": several runs (two inside the same minute -> must collapse to
    // one), forecast hours inserted out of order, mixed-case / numeric levels
    // (ordering is collation-sensitive), differing bboxes.
    let levels = [
        "surface",
        "2 m above ground",
        "500 mb",
        "10 m above ground",
        "Entire atmosphere",
    ];
    let mut n = 0u32;
    for (run_offset_secs, hours) in [
        (0i64, vec![6u32, 0, 3]),
        (20, vec![0, 1]), // same minute as the first run
        (3600, vec![12, 0]),
        (7200, vec![2]),
    ] {
        for h in hours {
            for (li, level) in levels.iter().enumerate() {
                n += 1;
                let mut e = test_entry("multi", "TMP", h);
                e.reference_time = base + Duration::seconds(run_offset_secs);
                e.level = level.to_string();
                e.bbox = BoundingBox::new(
                    -130.0 + li as f64 + (n % 3) as f64,
                    20.0 - (n % 4) as f64,
                    -60.0 - li as f64,
                    55.0 + (n % 5) as f64,
                );
                catalog.register_dataset(&e).await.expect("register");
            }
        }
    }
    // A single-row parameter, and one with rows that are NOT available.
    let mut one = test_entry("multi", "ONE", 0);
    one.reference_time = base;
    catalog.register_dataset(&one).await.expect("register");

    let mut hidden = test_entry("multi", "HIDDEN", 0);
    hidden.reference_time = base;
    catalog.register_dataset(&hidden).await.expect("register");
    sqlx::query("UPDATE datasets SET status = 'deleted' WHERE parameter = 'HIDDEN'")
        .execute(&pool)
        .await
        .unwrap();

    // Rows with status != available must not influence the multi parameter
    // either (extreme bbox / extra hour / extra level on a hidden row).
    let mut stray = test_entry("multi", "TMP", 99);
    stray.reference_time = base - Duration::days(9);
    stray.level = "zzz hidden level".to_string();
    stray.bbox = BoundingBox::new(-999.0, -999.0, 999.0, 999.0);
    catalog.register_dataset(&stray).await.expect("register");
    sqlx::query("UPDATE datasets SET status = 'deleted' WHERE forecast_hour = 99")
        .execute(&pool)
        .await
        .unwrap();

    // Another model with the same parameter must not leak in.
    let mut other = test_entry("othermodel", "TMP", 7);
    other.bbox = BoundingBox::new(-1.0, -1.0, 1.0, 1.0);
    catalog.register_dataset(&other).await.expect("register");

    for (model, param) in [
        ("multi", "TMP"),
        ("multi", "ONE"),
        ("multi", "HIDDEN"),
        ("multi", "NOPE"),
        ("nomodel", "TMP"),
        ("othermodel", "TMP"),
    ] {
        let got = catalog
            .get_parameter_availability(model, param)
            .await
            .expect("query");
        let want = availability_five_queries(&pool, model, param).await;
        match (got, want) {
            (None, None) => {}
            (Some(g), Some((times, hours, lv, bb))) => {
                assert_eq!(g.times, times, "{model}/{param} times");
                assert_eq!(g.forecast_hours, hours, "{model}/{param} hours");
                assert_eq!(g.levels, lv, "{model}/{param} levels");
                assert_eq!(
                    (g.bbox.min_x, g.bbox.min_y, g.bbox.max_x, g.bbox.max_y),
                    bb,
                    "{model}/{param} bbox"
                );
            }
            (g, w) => panic!(
                "{model}/{param}: got {:?}, oracle {:?}",
                g.is_some(),
                w.is_some()
            ),
        }
    }

    // Sanity on the seeded data so the comparison above can't pass vacuously.
    let multi = catalog
        .get_parameter_availability("multi", "TMP")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        multi.times.len(),
        3,
        "20 s-apart runs collapse to one minute bucket"
    );
    assert!(multi.times[0] > multi.times[1], "times newest first");
    assert_eq!(multi.forecast_hours, vec![0, 1, 2, 3, 6, 12]);
    assert_eq!(multi.levels.len(), 5);
    assert!(!multi.levels.iter().any(|l| l.contains("hidden")));
    assert!(multi.bbox.min_x > -999.0, "non-available rows are excluded");
    assert!(catalog
        .get_parameter_availability("multi", "HIDDEN")
        .await
        .unwrap()
        .is_none());
    assert!(catalog
        .get_parameter_availability("multi", "NOPE")
        .await
        .unwrap()
        .is_none());
}

// ============================================================================
// Trail conditions: latest (LATERAL) / batch timeseries / retention delete
// ============================================================================

/// Insert a row with every field the stitched series cares about.
#[allow(clippy::too_many_arguments)]
async fn insert_full_condition_row(
    pool: &sqlx::PgPool,
    feature_id: i64,
    run_time: DateTime<Utc>,
    valid_time: DateTime<Utc>,
    forecast_hour: i32,
    soil_moisture: f32,
    model_version: &str,
) {
    sqlx::query(
        r#"INSERT INTO segment_conditions
           (feature_id, run_time, valid_time, forecast_hour, soil_moisture, saturation,
            frozen_fraction, confidence, model_version)
           VALUES ($1, $2, $3, $4, $5, $5 * 2, 0.0, 1.0, $6)"#,
    )
    .bind(feature_id)
    .bind(run_time)
    .bind(valid_time)
    .bind(forecast_hour)
    .bind(soil_moisture)
    .bind(model_version)
    .execute(pool)
    .await
    .expect("insert segment_conditions row");
}

async fn insert_trail(pool: &sqlx::PgPool, feature_id: i64, name: Option<&str>, active: bool) {
    sqlx::query(
        r#"INSERT INTO linear_features (feature_id, feature_class, name, geom, region, active)
           VALUES ($1, 'mtb_trail', $2, ST_GeomFromText('LINESTRING(-105.3 40.0, -105.29 40.01)', 4326), 'test', $3)"#,
    )
    .bind(feature_id)
    .bind(name)
    .bind(active)
    .execute(pool)
    .await
    .expect("insert linear_features row");
}

fn row_key(r: &storage::segment_conditions::SegmentCondition) -> String {
    format!(
        "{}|{}|{}|{}|{:?}|{}",
        r.feature_id,
        r.valid_time.to_rfc3339(),
        r.run_time.to_rfc3339(),
        r.forecast_hour,
        r.soil_moisture,
        r.model_version
    )
}

/// The previous `get_latest_for_features` SQL as the oracle (DISTINCT ON + global
/// sort), plus the explicit `model_version DESC` tie-break the old ORDER BY lacked.
async fn latest_distinct_on_oracle(
    pool: &sqlx::PgPool,
    ids: &[i64],
) -> Vec<storage::segment_conditions::SegmentCondition> {
    sqlx::query_as(
        r#"
        SELECT DISTINCT ON (feature_id)
               feature_id, run_time, valid_time, forecast_hour,
               soil_moisture, saturation, frozen_fraction, frost_depth_m, swe_mm,
               softness_index, confidence, model_version
        FROM segment_conditions
        WHERE feature_id = ANY($1) AND valid_time <= NOW()
        ORDER BY feature_id, valid_time DESC, run_time DESC, model_version DESC
        "#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Seed several trails with different shapes of history. Returns the ids.
async fn seed_conditions(pool: &sqlx::PgPool) -> Vec<i64> {
    let now = now_micros();
    let hour = |h: i64| now.trunc_subsecs(0) + Duration::hours(h);
    let (a, b, c, d, e) = (11_i64, 12_i64, 13_i64, 14_i64, 15_i64);

    // a: many past hours, runs overlapping (newer run wins per valid hour),
    //    plus future hours; two model versions on one hour.
    for h in -20..=6 {
        // old run covers everything, newer run covers only -3..=3
        insert_full_condition_row(
            pool,
            a,
            hour(-24),
            hour(h),
            (h + 24) as i32,
            0.10,
            "trail-physics-v1",
        )
        .await;
        if (-3..=3).contains(&h) {
            insert_full_condition_row(
                pool,
                a,
                hour(-4),
                hour(h),
                (h + 4) as i32,
                0.30,
                "trail-physics-v1.1",
            )
            .await;
        }
    }
    // b: only the future (no row at or before now) -> no "latest", still a series
    for h in 2..=5 {
        insert_full_condition_row(
            pool,
            b,
            hour(0),
            hour(h),
            h as i32,
            0.20,
            "trail-physics-v1",
        )
        .await;
    }
    // c: a single past row
    insert_full_condition_row(pool, c, hour(-2), hour(-1), 1, 0.40, "trail-physics-v1").await;
    // d: same valid_time, same run_time, different versions (tie)
    insert_full_condition_row(pool, d, hour(-3), hour(-1), 2, 0.11, "trail-physics-v1").await;
    insert_full_condition_row(pool, d, hour(-3), hour(-1), 2, 0.22, "trail-physics-v2").await;
    // e: known trail with no rows at all (inserted as a trail below, none here)
    vec![a, b, c, d, e]
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_latest_for_features_lateral_matches_the_distinct_on_oracle() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let ids = seed_conditions(&pool).await;
    let conditions = storage::segment_conditions::SegmentConditionsCatalog::new(pool.clone());

    // every id, a subset, duplicates, ids with no rows, and nothing at all
    for query in [
        ids.clone(),
        vec![11, 13],
        vec![13, 13, 11, 11],
        vec![12, 15, 99_999],
        vec![99_999],
        vec![],
    ] {
        let got = conditions.get_latest_for_features(&query).await.unwrap();
        let want = latest_distinct_on_oracle(&pool, &query).await;
        assert_eq!(
            got.iter().map(row_key).collect::<Vec<_>>(),
            want.iter().map(row_key).collect::<Vec<_>>(),
            "ids {query:?}"
        );
    }

    // sanity on the seed, so the comparison cannot pass vacuously
    let all = conditions.get_latest_for_features(&ids).await.unwrap();
    let by_id = |id: i64| all.iter().find(|r| r.feature_id == id);
    assert_eq!(
        all.len(),
        3,
        "a, c, d have a past row; b (future only) and e (none) do not"
    );
    assert!(by_id(12).is_none() && by_id(15).is_none());
    // a: the newest run wins at the latest hour (valid_time = now truncated to the hour)
    let a = by_id(11).unwrap();
    assert_eq!(a.model_version, "trail-physics-v1.1");
    assert_eq!(a.soil_moisture, Some(0.30));
    // d: valid_time and run_time tie -> the higher model_version, deterministically
    assert_eq!(by_id(14).unwrap().model_version, "trail-physics-v2");
    assert!(
        all.windows(2).all(|w| w[0].feature_id < w[1].feature_id),
        "ordered by feature_id"
    );
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_timeseries_for_features_matches_per_feature_calls() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let ids = seed_conditions(&pool).await;
    let conditions = storage::segment_conditions::SegmentConditionsCatalog::new(pool.clone());

    for query in [
        ids.clone(),
        vec![11],
        vec![14, 11],
        vec![15],
        vec![99_999, 13],
        vec![],
    ] {
        let batch = conditions
            .get_timeseries_for_features(&query)
            .await
            .unwrap();

        // grouped by feature, ascending valid_time within each, ascending feature_id
        assert!(
            batch
                .windows(2)
                .all(|w| (w[0].feature_id, w[0].valid_time) < (w[1].feature_id, w[1].valid_time)),
            "ordering for {query:?}"
        );

        let mut sorted_query = query.clone();
        sorted_query.sort_unstable();
        sorted_query.dedup();
        let mut want = Vec::new();
        for id in &sorted_query {
            want.extend(conditions.get_timeseries_for_feature(*id).await.unwrap());
        }
        assert_eq!(
            batch.iter().map(row_key).collect::<Vec<_>>(),
            want.iter().map(row_key).collect::<Vec<_>>(),
            "batch != per-feature calls for {query:?}"
        );
    }

    // sanity: the series are non-trivial, stitched and windowed
    let a = conditions.get_timeseries_for_features(&[11]).await.unwrap();
    assert!(a.len() > 6, "past 6 h + future hours: {}", a.len());
    assert!(
        a.iter()
            .all(|r| r.valid_time >= Utc::now() - Duration::hours(7)),
        "history window"
    );
    let at_zero = a
        .iter()
        .find(|r| r.soil_moisture == Some(0.30))
        .expect("newer run wins its hours");
    assert_eq!(at_zero.model_version, "trail-physics-v1.1");
    assert!(
        a.iter()
            .filter(|r| r.valid_time == at_zero.valid_time)
            .count()
            == 1,
        "one row per valid_time"
    );
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_get_names_by_ids_reports_known_trails_only() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    insert_trail(&pool, 21, Some("Forsythe Canyon Trail"), true).await;
    insert_trail(&pool, 22, None, true).await;
    insert_trail(&pool, 23, Some("Retired Way"), false).await; // inactive is still "known"
    let features = storage::LinearFeatureCatalog::new(pool.clone());

    let mut got = features
        .get_names_by_ids(&[23, 21, 22, 99_999])
        .await
        .unwrap();
    got.sort();
    assert_eq!(
        got,
        vec![
            (21, Some("Forsythe Canyon Trail".to_string())),
            (22, None),
            (23, Some("Retired Way".to_string())),
        ]
    );
    // exactly what the single-trail lookup reports
    for id in [21, 22, 23, 99_999] {
        let single = features.get_feature_by_id(id).await.unwrap();
        let batch = got.iter().find(|(i, _)| *i == id);
        assert_eq!(single.is_some(), batch.is_some(), "id {id}");
        if let (Some(s), Some((_, n))) = (single, batch) {
            assert_eq!(&s.name, n);
        }
    }
    assert!(features.get_names_by_ids(&[]).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_delete_valid_before_batch_removes_only_old_rows_in_batches() {
    let infra = TestInfrastructure::start().await;
    let catalog = connected_catalog(&infra).await;
    let pool = catalog.pool_clone();
    let conditions = storage::segment_conditions::SegmentConditionsCatalog::new(pool.clone());
    let now = now_micros().trunc_subsecs(0);

    // 10 old rows (13..22 h ago), 5 recent/future rows (-2..+2 h)
    for h in 13..23 {
        insert_full_condition_row(
            &pool,
            31,
            now - Duration::hours(30),
            now - Duration::hours(h),
            0,
            0.1,
            "v",
        )
        .await;
    }
    for h in -2..=2 {
        insert_full_condition_row(
            &pool,
            31,
            now - Duration::hours(30),
            now + Duration::hours(h),
            0,
            0.1,
            "v",
        )
        .await;
    }
    let cutoff = now - Duration::hours(12);

    // batches of 4: 4, 4, 2, then nothing
    assert_eq!(
        conditions
            .delete_valid_before_batch(cutoff, 4)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        conditions
            .delete_valid_before_batch(cutoff, 4)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        conditions
            .delete_valid_before_batch(cutoff, 4)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        conditions
            .delete_valid_before_batch(cutoff, 4)
            .await
            .unwrap(),
        0
    );

    let left: Vec<DateTime<Utc>> =
        sqlx::query_scalar("SELECT valid_time FROM segment_conditions ORDER BY valid_time")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(left.len(), 5, "every recent and future row survives");
    assert!(left.iter().all(|t| *t >= cutoff));
    // a row exactly at the cutoff is kept (strictly older is deleted)
    insert_full_condition_row(&pool, 32, now, cutoff, 0, 0.1, "v").await;
    assert_eq!(
        conditions
            .delete_valid_before_batch(cutoff, 100)
            .await
            .unwrap(),
        0
    );
    // a non-positive batch size is treated as 1, never as "everything"
    insert_full_condition_row(&pool, 33, now, cutoff - Duration::hours(1), 0, 0.1, "v").await;
    insert_full_condition_row(&pool, 34, now, cutoff - Duration::hours(2), 0, 0.1, "v").await;
    assert_eq!(
        conditions
            .delete_valid_before_batch(cutoff, 0)
            .await
            .unwrap(),
        1
    );
}
