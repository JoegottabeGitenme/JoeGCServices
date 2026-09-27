//! Integration tests for the storage crate.
//!
//! These tests require Docker and are run with `cargo test -- --ignored`.

use bytes::Bytes;
use chrono::{Duration, Utc};

use storage::{
    Catalog, CatalogEntry, ObjectStorage, ObjectStorageConfig, SegmentConditionsCatalog,
};
use test_utils::containers::TestInfrastructure;
use wms_common::BoundingBox;

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
    let now = Utc::now();
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
    let now = Utc::now();
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
    let now = Utc::now();
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
