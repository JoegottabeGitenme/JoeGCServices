//! Integration tests for the EDR API service.
//!
//! These tests require Docker and are run with `cargo test -- --ignored`.
//! They verify the API endpoints work correctly with real databases.

use std::sync::Arc;
use tokio::sync::RwLock;

use axum::http::HeaderMap;

use edr_api::{
    availability::AvailabilityCache, config::EdrConfig, handlers, location_cache::LocationCache,
    metrics::MetricsCollector, state::AppState,
};
use grid_processor::{GridDataService, MinioConfig};
use storage::{observations::ObservationCatalog, Catalog};
use test_utils::containers::TestInfrastructure;

/// Create test AppState from infrastructure.
async fn create_test_state(infra: &TestInfrastructure) -> Arc<AppState> {
    // Connect to catalog and run migrations
    let catalog = Arc::new(
        Catalog::connect(&infra.postgres_url())
            .await
            .expect("Failed to connect to catalog"),
    );
    catalog.migrate().await.expect("Failed to migrate");
    catalog
        .migrate_observations()
        .await
        .expect("Failed to migrate observations");

    // Create MinIO config
    let minio_config = MinioConfig {
        endpoint: infra.minio_url(),
        bucket: "test-bucket".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    // Create grid data service
    let grid_data_service = GridDataService::new(Arc::clone(&catalog), minio_config, 64)
        .expect("Failed to create GridDataService");

    // Create observation catalog
    let observation_catalog = Arc::new(ObservationCatalog::new(catalog.pool_clone()));

    // Create empty EDR config (no YAML files in test)
    let edr_config = EdrConfig::default();

    // Create caches and metrics
    let location_cache = Arc::new(LocationCache::new(16, 60));
    let availability_cache = Arc::new(AvailabilityCache::new(60));
    let metrics = Arc::new(MetricsCollector::new());

    Arc::new(AppState {
        catalog,
        grid_data_service,
        observation_catalog,
        edr_config: Arc::new(RwLock::new(edr_config)),
        base_url: "http://localhost:8083/edr".to_string(),
        location_cache,
        availability_cache,
        metrics,
    })
}

// ============================================================================
// Health Endpoint Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_health_handler() {
    // Health handler doesn't need state, just test it works
    let response = handlers::health::health_handler().await;
    assert_eq!(response.status, "ok");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_ready_handler_database_connected() {
    use axum::Extension;

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    // Call ready handler with extension
    let response = handlers::health::ready_handler(Extension(state)).await;

    // Parse the response body
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("Failed to read body");

    let json: serde_json::Value = serde_json::from_slice(&body).expect("Failed to parse response");

    assert_eq!(json["ready"], true);
    assert_eq!(json["database"], "ok");
}

// ============================================================================
// Landing Page Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_landing_handler() {
    use axum::Extension;

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    // Call landing handler with empty headers
    let headers = HeaderMap::new();
    let response = handlers::landing::landing_handler(Extension(state), headers).await;

    // Parse the response
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .expect("Failed to read body");

    let json: serde_json::Value = serde_json::from_slice(&body).expect("Failed to parse response");

    // Verify OGC API landing page structure
    assert!(json["title"].is_string(), "Missing title");
    assert!(json["description"].is_string(), "Missing description");
    assert!(json["links"].is_array(), "Missing links array");

    // Check required links
    let links = json["links"].as_array().unwrap();
    let has_self = links.iter().any(|l| l["rel"] == "self");
    let has_collections = links.iter().any(|l| l["rel"] == "data");
    let has_conformance = links.iter().any(|l| l["rel"] == "conformance");

    assert!(has_self, "Missing self link");
    assert!(has_collections, "Missing data link");
    assert!(has_conformance, "Missing conformance link");
}

// ============================================================================
// Conformance Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_conformance_handler() {
    // Conformance handler needs HeaderMap
    let headers = HeaderMap::new();
    let response = handlers::conformance::conformance_handler(headers).await;

    // Parse the response body
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .expect("Failed to read body");

    let json: serde_json::Value = serde_json::from_slice(&body).expect("Failed to parse response");

    // Verify conformance structure
    assert!(json["conformsTo"].is_array());
    let conforms_to = json["conformsTo"].as_array().unwrap();
    assert!(!conforms_to.is_empty());

    // Should conform to OGC API EDR core
    let has_edr_core = conforms_to
        .iter()
        .any(|c| c.as_str().map(|s| s.contains("edr")).unwrap_or(false));
    assert!(has_edr_core, "Should conform to EDR API");
}

// ============================================================================
// Database Integration Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Docker
async fn test_catalog_operations_through_state() {
    use chrono::{Duration, Utc};
    use storage::CatalogEntry;
    use wms_common::BoundingBox;

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    // Insert some test data into the catalog
    let entry = CatalogEntry {
        model: "test-model".to_string(),
        parameter: "temperature".to_string(),
        level: "surface".to_string(),
        reference_time: Utc::now() - Duration::hours(1),
        forecast_hour: 0,
        bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
        storage_path: "test-model/temperature/f000.zarr".to_string(),
        file_size: 1024,
        zarr_metadata: None,
    };

    state
        .catalog
        .register_dataset(&entry)
        .await
        .expect("Failed to register dataset");

    // Verify data is available through catalog
    let models = state.catalog.list_models().await.expect("Failed to list");
    assert!(models.contains(&"test-model".to_string()));

    // Verify parameters
    let params = state
        .catalog
        .list_parameters("test-model")
        .await
        .expect("Failed to list params");
    assert!(params.contains(&"temperature".to_string()));

    // Verify stats
    let stats = state.catalog.get_model_stats().await.expect("Failed");
    let test_model_stats = stats.iter().find(|s| s.model == "test-model");
    assert!(test_model_stats.is_some());
    assert_eq!(test_model_stats.unwrap().dataset_count, 1);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_observation_catalog_locations() {
    use storage::observations::Location;

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    // Insert a test location using the Location struct's builder pattern
    let location = Location::new(
        "KJFK",
        "John F. Kennedy International Airport",
        -73.7781,
        40.6413,
    )
    .with_elevation(4.0)
    .with_type("airport")
    .with_country("US")
    .with_region("NY");

    state
        .observation_catalog
        .upsert_location(&location)
        .await
        .expect("Failed to insert location");

    // Query back the location
    let found = state
        .observation_catalog
        .get_location("KJFK")
        .await
        .expect("Failed to get location");

    assert!(found.is_some());
    let loc = found.unwrap();
    assert_eq!(loc.id, "KJFK");
    assert_eq!(loc.name, "John F. Kennedy International Airport");
    assert!((loc.lat - 40.6413).abs() < 0.0001);
    assert!((loc.lon - (-73.7781)).abs() < 0.0001);
    assert_eq!(loc.location_type.as_deref(), Some("airport"));
    assert_eq!(loc.country.as_deref(), Some("US"));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_observation_catalog_observations() {
    use chrono::Utc;
    use storage::observations::{Location, Observation, ObservationQuery};

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    // Insert a test location first
    let location = Location::new(
        "KORD",
        "Chicago O'Hare International Airport",
        -87.9048,
        41.9742,
    )
    .with_type("airport");

    state
        .observation_catalog
        .upsert_location(&location)
        .await
        .expect("Failed to insert location");

    // Insert a test observation with the proper field names
    let now = Utc::now();
    let observation = Observation {
        id: None,
        location_id: "KORD".to_string(),
        source: "metar".to_string(),
        obs_time: now,
        receipt_time: Some(now),
        temperature_k: Some(268.15), // -5C in Kelvin
        dewpoint_k: Some(258.15),    // -15C in Kelvin
        wind_direction_deg: Some(310),
        wind_speed_ms: Some(8.0),
        wind_gust_ms: None,
        altimeter_pa: Some(102950.0),
        sea_level_pressure_pa: None,
        visibility_m: Some(16093.0),
        precip_1hr_mm: None,
        relative_humidity_pct: None,
        wave_height_m: None,
        dominant_wave_period_s: None,
        average_wave_period_s: None,
        mean_wave_direction_deg: None,
        water_temp_k: None,
        tide_m: None,
        water_column_height_m: None,
        raw_text: Some("METAR KORD 061756Z 31016KT 10SM FEW250 M05/M15 A3040".to_string()),
        flight_category: Some("VFR".to_string()),
        wx_string: None,
        cloud_layers: None,
        temperature_qc: None,
        dewpoint_qc: None,
        wind_qc: None,
        pressure_qc: None,
    };

    state
        .observation_catalog
        .insert_observation(&observation)
        .await
        .expect("Failed to insert observation");

    // Query observations using ObservationQuery
    let query = ObservationQuery {
        location_id: Some("KORD".to_string()),
        source: Some("metar".to_string()),
        start_time: Some(now - chrono::Duration::hours(1)),
        end_time: Some(now + chrono::Duration::hours(1)),
        ..Default::default()
    };

    let obs = state
        .observation_catalog
        .get_observations(&query)
        .await
        .expect("Failed to get observations");

    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].source, "metar");
    assert_eq!(obs[0].location_id, "KORD");
    assert!(obs[0].temperature_k.is_some());
    assert!((obs[0].temperature_k.unwrap() - 268.15).abs() < 0.1);
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_multiple_models_and_parameters() {
    use chrono::{Duration, Utc};
    use storage::CatalogEntry;
    use wms_common::BoundingBox;

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    let reference_time = Utc::now() - Duration::hours(1);

    // Insert datasets for multiple models
    let datasets = vec![
        ("gfs", "temperature", 0),
        ("gfs", "temperature", 3),
        ("gfs", "temperature", 6),
        ("gfs", "wind", 0),
        ("gfs", "wind", 3),
        ("hrrr", "reflectivity", 0),
        ("hrrr", "reflectivity", 1),
        ("nam", "precipitation", 0),
    ];

    for (model, param, fhour) in datasets {
        let entry = CatalogEntry {
            model: model.to_string(),
            parameter: param.to_string(),
            level: "surface".to_string(),
            reference_time,
            forecast_hour: fhour,
            bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
            storage_path: format!("{}/{}/f{:03}.zarr", model, param, fhour),
            file_size: 1024,
            zarr_metadata: None,
        };
        state.catalog.register_dataset(&entry).await.unwrap();
    }

    // Verify models
    let models = state.catalog.list_models().await.unwrap();
    assert_eq!(models.len(), 3);
    assert!(models.contains(&"gfs".to_string()));
    assert!(models.contains(&"hrrr".to_string()));
    assert!(models.contains(&"nam".to_string()));

    // Verify parameters per model
    let gfs_params = state.catalog.list_parameters("gfs").await.unwrap();
    assert_eq!(gfs_params.len(), 2);
    assert!(gfs_params.contains(&"temperature".to_string()));
    assert!(gfs_params.contains(&"wind".to_string()));

    // Verify forecast hours
    let temp_fhours = state
        .catalog
        .get_available_forecast_hours("gfs", "temperature")
        .await
        .unwrap();
    assert_eq!(temp_fhours, vec![0, 3, 6]);

    // Verify get_latest
    let latest = state
        .catalog
        .get_latest("gfs", "temperature")
        .await
        .unwrap();
    assert!(latest.is_some());
    assert_eq!(latest.unwrap().forecast_hour, 6); // Latest by valid_time
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_availability_cache_integration() {
    use chrono::{Duration, Utc};
    use storage::CatalogEntry;
    use wms_common::BoundingBox;

    let infra = TestInfrastructure::start().await;
    let state = create_test_state(&infra).await;

    // Insert a dataset
    let entry = CatalogEntry {
        model: "cache-test".to_string(),
        parameter: "temp".to_string(),
        level: "surface".to_string(),
        reference_time: Utc::now() - Duration::hours(1),
        forecast_hour: 0,
        bbox: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
        storage_path: "cache-test/temp/f000.zarr".to_string(),
        file_size: 1024,
        zarr_metadata: None,
    };
    state.catalog.register_dataset(&entry).await.unwrap();

    // First access should populate cache
    let availability = state
        .availability_cache
        .get_model_availability(&state.catalog, "cache-test")
        .await;

    assert!(availability.is_some());
    let avail = availability.unwrap();
    assert!(avail.has_parameter("temp"));
    assert!(avail.is_level_available("temp", "surface"));

    // Second access should hit cache
    let availability2 = state
        .availability_cache
        .get_model_availability(&state.catalog, "cache-test")
        .await;

    assert!(availability2.is_some());
    assert!(availability2.unwrap().has_parameter("temp"));
}
