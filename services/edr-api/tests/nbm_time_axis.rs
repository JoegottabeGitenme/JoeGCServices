//! A forecast series over a model whose runs do not all step alike (NBM), through the REAL
//! position handler. `#[ignore]`d (Docker).
//!
//! ```text
//! cargo test -p edr-api --test nbm_time_axis -- --ignored --nocapture
//! ```
//!
//! NBM runs are hourly for two days and then step 3 h, but the tail's valid times are multiples
//! of 3 for most runs and shifted by the cycle hour for the long hourly-extended ones (19z here:
//! hourly to f60, then f63 = 10z). The model's time axis was the union of every run's instants,
//! so the tail alternated 1 h and 2 h, and each extra instant was answered with the nearest
//! grid, a neighbour's value repeated under another time. The axis is now one run's cadence at a
//! time and each instant is read from the newest run that holds it.
//!
//! The catalog is synthesised from the one real HRRR fixture: its 4 grids are registered as
//! `nbm-conus` TMP at 2 m, one grid per run, so the value of an instant says which run answered.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use tokio::sync::RwLock;
use tower::ServiceExt;

use edr_api::{
    availability::AvailabilityCache, config::EdrConfig, handlers, location_cache::LocationCache,
    metrics::MetricsCollector, snapshot_cache::SnapshotCache, state::AppState,
};
use grid_processor::{DatasetQuery, GridDataService, MinioConfig};
use ingestion::{IngestOptions, Ingester};
use storage::{
    linear_features::LinearFeatureCatalog, observations::ObservationCatalog,
    segment_conditions::SegmentConditionsCatalog, storm_events::StormEventCatalog, Catalog,
    LightningCatalog, ObjectStorage, ObjectStorageConfig,
};
use test_utils::containers::TestInfrastructure;

const FIXTURE: &str = "hrrr/hrrr_20261009_23z_f002.grib2";
const POINT: (f64, f64) = (-104.99, 39.74);

/// The fixture's parameters, one per synthetic run (and one spare).
const TEMPLATES: [&str; 4] = ["PWAT", "REFC", "TCDC", "MSLMA"];

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
}

/// A synthetic run: its reference time, the template grid it points at and its lead hours.
struct Run {
    reference: DateTime<Utc>,
    template: usize,
    leads: Vec<i64>,
}

/// The runs, newest first.
fn runs() -> Vec<Run> {
    vec![
        // 00z: hourly to f48, then every 3 h from f51 (valid times that are multiples of 3)
        Run {
            reference: at(4, 0),
            template: 0,
            leads: (1..=48).chain((51..=120).step_by(3)).collect(),
        },
        // 23z, an hourly short run: hourly to f48, a few 3-hourly steps after
        Run {
            reference: at(3, 23),
            template: 1,
            leads: (1..=48).chain((49..=58).step_by(3)).collect(),
        },
        // 19z, a long hourly-extended run: hourly to f60, then every 3 h on its own lead hours
        Run {
            reference: at(3, 19),
            template: 2,
            leads: (1..=60).chain((63..=120).step_by(3)).collect(),
        },
    ]
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

fn series(body: &Value, param: &str) -> (Vec<DateTime<Utc>>, Vec<Option<f32>>) {
    let t = body["domain"]["axes"]["t"]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no t axis: {body}"))
        .iter()
        .map(|v| {
            DateTime::parse_from_rfc3339(v.as_str().unwrap())
                .unwrap()
                .with_timezone(&Utc)
        })
        .collect();
    let v = body["ranges"][param]["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values for {param}: {body}"))
        .iter()
        .map(|v| v.as_f64().map(|x| x as f32))
        .collect();
    (t, v)
}

struct Env {
    app: Router,
    catalog: Arc<Catalog>,
    /// the value at `POINT` of each template grid
    template_values: Vec<f32>,
    _infra: TestInfrastructure,
}

async fn setup() -> Env {
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
    let ingester = Ingester::new(Arc::clone(&storage), (*catalog).clone());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures")
        .join(FIXTURE);
    let r = ingester
        .ingest_file(&fixture.to_string_lossy(), IngestOptions::default())
        .await
        .expect("HRRR fixture");
    assert_eq!((r.model.as_str(), r.datasets_registered), ("hrrr", 4));

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
        256,
    )
    .unwrap();

    // What each template grid says at the point, to tell which run answered an instant.
    let mut template_values = Vec::new();
    for name in TEMPLATES {
        let q = DatasetQuery::forecast("hrrr", name);
        let v = grid_data_service
            .read_point(&q, POINT.0, POINT.1)
            .await
            .unwrap()
            .value
            .unwrap_or_else(|| panic!("{name} has no value at the point"));
        template_values.push(v);
    }
    let mut distinct = template_values
        .iter()
        .map(|v| v.to_bits())
        .collect::<Vec<_>>();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), TEMPLATES.len(), "{template_values:?}");

    // Register every run's rows as nbm-conus TMP at 2 m, copying the grid of its template.
    let pool = catalog.pool_clone();
    let ingest_base = Utc::now() - chrono::Duration::hours(2);
    let (mut reference, mut hour, mut template, mut ingested) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // older runs first, as they are ingested
    for run in runs().iter().rev() {
        for lead in &run.leads {
            reference.push(run.reference);
            hour.push(*lead as i32);
            template.push(TEMPLATES[run.template].to_string());
            ingested.push(ingest_base + chrono::Duration::milliseconds(ingested.len() as i64));
        }
    }
    let inserted = sqlx::query(
        "INSERT INTO datasets (id, model, parameter, level, reference_time, forecast_hour, \
           valid_time, bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, storage_path, file_size, \
           ingested_at, status, zarr_metadata, forecast_minute) \
         SELECT gen_random_uuid(), 'nbm-conus', 'TMP', '2 m above ground', x.reference_time, \
           x.hour, x.reference_time + x.hour * interval '1 hour', \
           t.bbox_min_x, t.bbox_min_y, t.bbox_max_x, t.bbox_max_y, t.storage_path, t.file_size, \
           x.ingested_at, 'available', t.zarr_metadata, 0 \
         FROM unnest($1::timestamptz[], $2::int[], $3::text[], $4::timestamptz[]) \
              AS x(reference_time, hour, template, ingested_at) \
         JOIN datasets t ON t.model = 'hrrr' AND t.parameter = x.template \
                        AND t.reference_time = $5 AND t.forecast_hour = 2",
    )
    .bind(&reference)
    .bind(&hour)
    .bind(&template)
    .bind(&ingested)
    .bind(Utc.with_ymd_and_hms(2026, 10, 9, 23, 0, 0).unwrap())
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected() as usize;
    assert_eq!(inserted, reference.len());

    let state = Arc::new(AppState {
        observation_catalog: Arc::new(ObservationCatalog::new(catalog.pool_clone())),
        storm_event_catalog: Arc::new(StormEventCatalog::new(catalog.pool_clone())),
        linear_feature_catalog: Arc::new(LinearFeatureCatalog::new(catalog.pool_clone())),
        segment_conditions_catalog: Arc::new(SegmentConditionsCatalog::new(catalog.pool_clone())),
        lightning_catalog: Arc::new(LightningCatalog::new(catalog.pool_clone())),
        nws_alert_catalog: Arc::new(storage::NwsAlertCatalog::new(catalog.pool_clone())),
        collections_snapshot: Arc::new(SnapshotCache::new(Duration::from_secs(60))),
        catalog: Arc::clone(&catalog),
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
    Env {
        app,
        catalog,
        template_values,
        _infra: infra,
    }
}

fn uri(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    let f = |t: DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    format!(
        "/edr/collections/nbm-conus-2m-agl/position?coords=POINT({}%20{})\
         &parameter-name=TMP&z=2&datetime={}/{}",
        POINT.0,
        POINT.1,
        f(from),
        f(to)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn each_instant_is_served_once_from_the_newest_run_at_the_native_step() {
    let env = setup().await;

    // The premise: some instant exists only in the long run's shifted tail, so the old axis (every
    // instant any run holds) had it.
    let union = env
        .catalog
        .get_model_valid_times("nbm-conus")
        .await
        .unwrap();
    assert!(
        union.contains(&at(6, 7)),
        "19z f60 is an instant of the union"
    );
    assert!(!union.contains(&at(6, 8)));

    // Three days from the newest run's first hour.
    let (status, body) = get_json(&env.app, &uri(at(4, 1), at(7, 1))).await;
    assert_eq!(status, 200, "{body}");
    let (times, values) = series(&body, "TMP");

    // hourly to f48 (06T00z), then every 3 h on multiples of 3, up to 07T00z: 48 + 8 times
    let want: Vec<DateTime<Utc>> = (1..=48)
        .map(|h| at(4, 0) + chrono::Duration::hours(h))
        .chain(
            (51..=72)
                .step_by(3)
                .map(|h| at(4, 0) + chrono::Duration::hours(h)),
        )
        .collect();
    assert_eq!(
        times, want,
        "the newest run's own instants, none of the others'"
    );
    assert!(
        !times.contains(&at(6, 7)) && !times.contains(&at(6, 1)),
        "no instant of an older run's other phase"
    );

    // every instant comes from the newest run, which holds all of them: one grid, one value
    let newest = env.template_values[0];
    assert!(
        values.iter().all(|v| *v == Some(newest)),
        "not every value is the newest run's: {values:?}"
    );

    // The past: before the newest run's first hour, the instants come from the runs that hold
    // them, each from the newest of those (19z f3, f4; 23z f1 = 00z).
    let (status, body) = get_json(&env.app, &uri(at(3, 22), at(4, 2))).await;
    assert_eq!(status, 200, "{body}");
    let (times, values) = series(&body, "TMP");
    assert_eq!(
        times,
        vec![at(3, 22), at(3, 23), at(4, 0), at(4, 1), at(4, 2)]
    );
    let [p, r, t, ..] = env.template_values[..] else {
        unreachable!()
    };
    assert_eq!(
        values,
        vec![Some(t), Some(t), Some(r), Some(p), Some(p)],
        "19z, 19z, 23z, 00z, 00z"
    );

    // An instant of the union that is not on the axis is still answered when asked for by name.
    let (status, body) = get_json(
        &env.app,
        &format!(
            "/edr/collections/nbm-conus-2m-agl/position?coords=POINT({}%20{})\
             &parameter-name=TMP&z=2&datetime=2026-11-06T07:00:00Z",
            POINT.0, POINT.1
        ),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(series(&body, "TMP").1, vec![Some(t)], "19z f60");
}
