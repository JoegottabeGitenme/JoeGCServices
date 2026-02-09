//! Integration tests for the WMS API service.
//!
//! These tests require Docker and are run with `cargo test -- --ignored`.
//! They verify the database and storage layers work correctly.

use chrono::{Duration, Utc};
use std::sync::Arc;

use storage::{Catalog, CatalogEntry, ObjectStorage, ObjectStorageConfig};
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
// Catalog Integration Tests for WMS
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_connect_for_wms() {
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
async fn test_layer_discovery() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register datasets for WMS layers
    for model in ["hrrr", "gfs"] {
        for param in ["TMP", "REFC", "WIND"] {
            let entry = test_entry(model, param, 0);
            catalog.register_dataset(&entry).await.unwrap();
        }
    }

    // Verify layer discovery works
    let models = catalog.list_models().await.unwrap();
    assert_eq!(models.len(), 2);

    // Each model should have 3 parameters = 3 layers
    let hrrr_params = catalog.list_parameters("hrrr").await.unwrap();
    assert_eq!(hrrr_params.len(), 3);
    assert!(hrrr_params.contains(&"TMP".to_string()));
    assert!(hrrr_params.contains(&"REFC".to_string()));
    assert!(hrrr_params.contains(&"WIND".to_string()));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_time_dimension_discovery() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    let reference_time = Utc::now() - Duration::hours(2);

    // Register multiple forecast hours (WMS TIME dimension)
    for hour in [0, 1, 2, 3, 6, 9, 12] {
        let mut entry = test_entry("hrrr", "TMP", hour);
        entry.reference_time = reference_time;
        catalog.register_dataset(&entry).await.unwrap();
    }

    // Verify time dimension
    let times = catalog
        .get_available_times("hrrr", "TMP")
        .await
        .expect("Failed to get times");

    assert_eq!(times.len(), 7);

    // Verify forecast hours
    let fhours = catalog
        .get_available_forecast_hours("hrrr", "TMP")
        .await
        .unwrap();
    assert_eq!(fhours, vec![0, 1, 2, 3, 6, 9, 12]);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_model_runs_dimension() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register datasets from multiple model runs
    for run_offset in [0, 6, 12, 18] {
        let reference_time = Utc::now() - Duration::hours(run_offset);
        for hour in [0, 3, 6] {
            let mut entry = test_entry("gfs", "TMP", hour);
            entry.reference_time = reference_time;
            catalog.register_dataset(&entry).await.unwrap();
        }
    }

    // Verify runs can be discovered
    let runs = catalog
        .get_available_runs("gfs", "TMP")
        .await
        .expect("Failed to get runs");

    assert_eq!(runs.len(), 4); // 4 different model runs
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_latest_for_getmap() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    let now = Utc::now();

    // Register old and new entries
    let mut old_entry = test_entry("hrrr", "REFC", 0);
    old_entry.reference_time = now - Duration::hours(6);
    catalog.register_dataset(&old_entry).await.unwrap();

    let mut new_entry = test_entry("hrrr", "REFC", 0);
    new_entry.reference_time = now - Duration::hours(1);
    catalog.register_dataset(&new_entry).await.unwrap();

    // GetMap with no TIME should get latest
    let latest = catalog
        .get_latest_run_earliest_forecast("hrrr", "REFC")
        .await
        .expect("Failed to get latest");

    assert!(latest.is_some());
    let entry = latest.unwrap();
    // Should get the newest run
    assert!(
        (entry.reference_time - (now - Duration::hours(1)))
            .num_minutes()
            .abs()
            < 5
    );
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_find_by_time_for_getmap() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    let reference = Utc::now() - Duration::hours(2);

    // Register forecast hours 0, 3, 6, 12
    for hour in [0, 3, 6, 12] {
        let mut entry = test_entry("gfs", "TMP", hour);
        entry.reference_time = reference;
        catalog.register_dataset(&entry).await.unwrap();
    }

    // Query for time that's closest to T+4h
    let target_time = reference + Duration::hours(4);
    let result = catalog
        .find_by_time("gfs", "TMP", target_time)
        .await
        .expect("Failed to find by time");

    assert!(result.is_some());
    let entry = result.unwrap();
    // Closest to +4h is +3h
    assert_eq!(entry.forecast_hour, 3);
}

// ============================================================================
// Storage Integration Tests for WMS
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_storage_for_wms() {
    use bytes::Bytes;

    let infra = TestInfrastructure::start().await;

    let config = ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "wms-test".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    // Create bucket first
    infra
        .create_minio_bucket("wms-test")
        .await
        .expect("Failed to create bucket");

    let storage = ObjectStorage::new(&config).expect("Failed to create storage");

    // Write a mock Zarr metadata file
    let metadata = r#"{"zarr_format": 3, "attributes": {}}"#;
    storage
        .put("hrrr/TMP/zarr.json", Bytes::from(metadata))
        .await
        .expect("Failed to write");

    // Read it back
    let result = storage
        .get("hrrr/TMP/zarr.json")
        .await
        .expect("Failed to read");

    assert!(String::from_utf8_lossy(&result).contains("zarr_format"));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_storage_stats() {
    use bytes::Bytes;

    let infra = TestInfrastructure::start().await;

    let config = ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "stats-test".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    // Create bucket first
    infra
        .create_minio_bucket("stats-test")
        .await
        .expect("Failed to create bucket");
    let storage = Arc::new(ObjectStorage::new(&config).expect("Failed to create storage"));

    // Write some test files
    for i in 0..5 {
        storage
            .put(
                &format!("test/file_{}.bin", i),
                Bytes::from(vec![0u8; 1024]),
            )
            .await
            .expect("Failed to write");
    }

    // Get stats
    let stats = storage.stats().await.expect("Failed to get stats");

    assert_eq!(stats.object_count, 5);
    assert_eq!(stats.total_size, 5 * 1024);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_bbox_from_catalog() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url())
        .await
        .expect("Failed to connect");
    catalog.migrate().await.expect("Failed to migrate");

    // Register datasets with specific bbox (CONUS)
    let mut entry = test_entry("hrrr", "TMP", 0);
    entry.bbox = BoundingBox::new(-125.0, 21.0, -66.0, 53.0); // CONUS
    catalog.register_dataset(&entry).await.unwrap();

    // Get model bbox (used for WMS capabilities)
    let bbox = catalog
        .get_model_bbox("hrrr")
        .await
        .expect("Failed to get bbox");

    assert!((bbox.min_x - (-125.0)).abs() < 0.01);
    assert!((bbox.min_y - 21.0).abs() < 0.01);
    assert!((bbox.max_x - (-66.0)).abs() < 0.01);
    assert!((bbox.max_y - 53.0).abs() < 0.01);
}
