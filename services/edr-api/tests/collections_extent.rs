//! A collection's temporal extent comes from its own parameters' valid times, not the model's
//! (SBT124 was added after TMP, so older runs lack it): the REAL handler, the REAL `config/edr`
//! and a real PostGIS. `#[ignore]`d (Docker).
//!
//! ```text
//! cargo test -p edr-api --test collections_extent -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::Extension;
use serde_json::Value;
use tokio::sync::RwLock;

use edr_api::{
    availability::AvailabilityCache, config::EdrConfig, handlers, location_cache::LocationCache,
    metrics::MetricsCollector, snapshot_cache::SnapshotCache, state::AppState,
};
use grid_processor::{GridDataService, MinioConfig};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog,
};

struct Env {
    state: Arc<AppState>,
    pool: sqlx::PgPool,
    _infra: Option<test_utils::containers::TestInfrastructure>,
}

async fn env() -> Env {
    let (url, minio, infra) = match std::env::var("LIGHTNING_TEST_DATABASE_URL") {
        Ok(u) => (u, "http://127.0.0.1:1".to_string(), None),
        Err(_) => {
            let infra = test_utils::containers::TestInfrastructure::start().await;
            (infra.postgres_url(), infra.minio_url(), Some(infra))
        }
    };
    let catalog = Arc::new(Catalog::connect(&url).await.unwrap());
    catalog.migrate().await.unwrap();
    catalog.migrate_observations().await.unwrap();
    catalog.migrate_lightning().await.unwrap();
    catalog.migrate_linear_features().await.unwrap();
    catalog.migrate_segment_conditions().await.unwrap();
    let pool = catalog.pool_clone();

    let grid_data_service = GridDataService::new(
        Arc::clone(&catalog),
        MinioConfig {
            endpoint: minio,
            bucket: "unused".to_string(),
            access_key_id: "x".to_string(),
            secret_access_key: "x".to_string(),
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
        nws_alert_catalog: Arc::new(storage::NwsAlertCatalog::new(catalog.pool_clone())),
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
    Env {
        state,
        pool,
        _infra: infra,
    }
}

async fn collection(e: &Env, id: &str) -> (StatusCode, Value) {
    let resp = handlers::collections::get_collection_handler(
        Extension(e.state.clone()),
        Path(id.to_string()),
        HeaderMap::new(),
    )
    .await;
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 64 << 20).await.unwrap();
    (
        parts.status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Register `parameter` of `hrrr` at each lead of the 2026-11-04 00z run.
async fn register(pool: &sqlx::PgPool, parameter: &str, level: &str, leads: &[i32]) {
    for lead in leads {
        sqlx::query(
            "INSERT INTO datasets (id, model, parameter, level, reference_time, forecast_hour, \
               valid_time, bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, storage_path, \
               file_size, ingested_at, status, forecast_minute) \
             VALUES (gen_random_uuid(), 'hrrr', $1, $2, '2026-11-04T00:00:00Z', $3, \
               '2026-11-04T00:00:00Z'::timestamptz + $3 * interval '1 hour', \
               -130, 20, -60, 55, $4, 1024, now(), 'available', 0)",
        )
        .bind(parameter)
        .bind(level)
        .bind(lead)
        .bind(format!("grids/hrrr/{parameter}/f{lead:03}.zarr"))
        .execute(pool)
        .await
        .unwrap();
    }
}

fn temporal(body: &Value) -> (Vec<String>, Value) {
    let t = &body["extent"]["temporal"];
    let values = t["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no temporal values: {body}"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    (values, t["interval"].clone())
}

#[tokio::test]
#[ignore]
async fn a_collection_lists_only_the_valid_times_of_its_own_parameters() {
    let e = env().await;
    // PRES (hrrr-surface) at every hour 1..=6, SBT124 (hrrr-satellite) at hour 4 only
    register(&e.pool, "PRES", "surface", &[1, 2, 3, 4, 5, 6]).await;
    register(&e.pool, "SBT124", "top of atmosphere", &[4]).await;

    let (status, satellite) = collection(&e, "hrrr-satellite").await;
    assert_eq!(status, StatusCode::OK, "{satellite}");
    let (values, interval) = temporal(&satellite);
    assert_eq!(values, vec!["2026-11-04T04:00:00Z"]);
    assert_eq!(
        interval,
        serde_json::json!([["2026-11-04T04:00:00Z", "2026-11-04T04:00:00Z"]])
    );

    let (status, surface) = collection(&e, "hrrr-surface").await;
    assert_eq!(status, StatusCode::OK, "{surface}");
    let (values, interval) = temporal(&surface);
    assert_eq!(values.len(), 6, "{values:?}");
    assert_eq!(values[0], "2026-11-04T01:00:00Z");
    assert_eq!(
        interval,
        serde_json::json!([["2026-11-04T01:00:00Z", "2026-11-04T06:00:00Z"]])
    );
}
