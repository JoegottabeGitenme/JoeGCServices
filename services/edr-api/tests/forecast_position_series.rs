//! End-to-end: a multi-time FORECAST point series (the app's 16 day outlook) through the REAL
//! position handler, with a real HRRR message ingested by the REAL ingester into Postgres +
//! MinIO, then multiplied into a catalog shaped like production's. `#[ignore]`d (Docker).
//!
//! ```text
//! cargo test -p edr-api --test forecast_position_series -- --ignored --nocapture
//! ```
//!
//! The handler used to answer such a series with one catalog query ("which dataset is nearest
//! to this instant") and one blocking grid read per step, parameter after parameter: 836 of
//! each for a 4 parameter, 16 day GFS curve. It now asks the catalog once per parameter and
//! reads the grids concurrently. What must not change is WHICH dataset answers each instant:
//! the nearest valid time, ties to the newest run then the most recent ingest, within the level
//! filter, with a neighbour's grid standing in for an instant that has no dataset of its own.
//! The oracle here is the old code path itself: `GridDataService::read_point` per instant.
//!
//! The catalog is synthesised from the one real grid: the fixture's 4 messages (PWAT, REFC, TCDC,
//! MSLMA) are copied to several storage paths and registered as 4 model runs x 121 forecast
//! hours for each parameter, every row pointing at one of the 4 real grids (chosen so that
//! picking the wrong row changes the value), with
//! - runs ingested NEWEST FIRST, so the newest run has the OLDEST `ingested_at`,
//! - a hole in the newest run (PWAT has no grid for forecast hours 10-14 of it),
//! - an instant no run has a grid for (REFC at 2026-10-12T06:00Z), whose two neighbours are
//!   equally near: the nearest-dataset tie,
//! - a 7 hour gap (TCDC) and a missing tail (TCDC ends a day and a half early; MSLMA's newest run
//!   ends 6 h early), where the nearest dataset is a neighbour hours away,
//! - a decoy PWAT dataset at another level, ingested last, for every instant,
//! - LCDC (cloud layers) at two levels, for a collection whose config yields NO level filter:
//!   the nearest dataset of any level answers, and the newest run's second level has a short tail.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// The fixture's parameters and the level each is stored under.
const TEMPLATES: [(&str, &str); 4] = [
    ("PWAT", "entire atmosphere"),
    ("REFC", "entire atmosphere"),
    ("TCDC", "entire atmosphere"),
    ("MSLMA", "mean sea level"),
];

/// Distinct storage copies of each template grid, so a cold request reads many different S3
/// objects as production does instead of the same four.
const COPIES_PER_TEMPLATE: usize = 24;

const RUNS: i64 = 4;
const HOURS_PER_RUN: i64 = 121;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures")
        .join(rel)
        .to_string_lossy()
        .to_string()
}

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
}

/// Reference time of the synthetic run `r` (0 = newest): 18z, 12z, 06z, 00z on 2026-10-10.
fn run_time(r: i64) -> DateTime<Utc> {
    at(10, 18) - chrono::Duration::hours(6 * r)
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

/// `(times, values)` of one parameter's series from a CoverageJSON point series.
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

/// The unit symbol of one parameter in a CoverageJSON response (empty if it has none).
fn unit_of(body: &Value, param: &str) -> String {
    body["parameters"][param]["unit"]["symbol"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// One synthetic catalog row.
struct Synth {
    parameter: &'static str,
    level: &'static str,
    run: i64,
    hour: i64,
    /// which of the 4 real grids this row points at, and which copy of it
    template: usize,
    copy: usize,
}

/// The rows of the synthetic catalog, in the order they are "ingested" (their `ingested_at`
/// increases along this list): newest run first, the decoy last.
fn scenario() -> Vec<Synth> {
    let mut rows = Vec::new();
    for run in 0..RUNS {
        for hour in 0..HOURS_PER_RUN {
            let valid = run_time(run) + chrono::Duration::hours(hour);
            for (pi, (parameter, level)) in TEMPLATES.iter().enumerate() {
                let skip = match *parameter {
                    // the newest run is missing forecast hours 10-14; older runs have them
                    "PWAT" => run == 0 && (10..=14).contains(&hour),
                    // no run has this instant
                    "REFC" => valid == at(12, 6),
                    "TCDC" => {
                        (valid >= at(13, 0) && valid <= at(13, 6)) // a 7 hour gap
                            || valid >= at(14, 12) // the tail
                            || (run == 3 && hour <= 2) // the head, past the fixture's own row
                    }
                    // the newest run ends early; the previous one stops at 12:00 on the 15th
                    "MSLMA" => run == 0 && hour >= 115,
                    _ => unreachable!(),
                };
                if skip {
                    continue;
                }
                // a template that depends on (run, hour, parameter) in a way no ordering of the
                // rows follows, so choosing the wrong row changes the value
                let template = ((run * 7 + hour * 3 + pi as i64 * 5) % 4) as usize;
                let copy = ((run * 31 + hour) as usize) % COPIES_PER_TEMPLATE;
                rows.push(Synth {
                    parameter,
                    level,
                    run,
                    hour,
                    template,
                    copy,
                });
            }
        }
    }
    // LCDC at two levels, the second ingested after the first. `hrrr-cloud-layers` configures
    // `level_codes` but no `level_code`, so the handler asks for no level at all and every row
    // competes: the later-ingested second level wins where it exists, the first where the newest
    // run's second level has ended.
    for run in 0..RUNS {
        for hour in 0..HOURS_PER_RUN {
            for (level, template) in [
                ("low cloud layer", (run * 5 + hour * 7) % 4),
                ("middle cloud layer", (run * 3 + hour * 5 + 1) % 4),
            ] {
                if level == "middle cloud layer" && run == 0 && hour >= 100 {
                    continue;
                }
                rows.push(Synth {
                    parameter: "LCDC",
                    level,
                    run,
                    hour,
                    template: template as usize,
                    copy: ((run * 17 + hour) as usize) % COPIES_PER_TEMPLATE,
                });
            }
        }
    }
    // The decoy: PWAT at another level, for every instant, ingested after everything else. A
    // lookup that ignores the level would answer with these.
    for run in 0..RUNS {
        for hour in 0..HOURS_PER_RUN {
            rows.push(Synth {
                parameter: "PWAT",
                level: "2 m above ground",
                run,
                hour,
                template: 3,
                copy: 0,
            });
        }
    }
    rows
}

struct Env {
    app: Router,
    state: Arc<AppState>,
    /// number of synthetic rows registered
    rows: usize,
    _infra: TestInfrastructure,
}

/// Ingest the fixture, copy its grids, register the synthetic catalog, build the real route.
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
    let r = ingester
        .ingest_file(&fixture(FIXTURE), IngestOptions::default())
        .await
        .expect("HRRR fixture");
    assert_eq!((r.model.as_str(), r.datasets_registered), ("hrrr", 4));

    // The real rows are the templates.
    use sqlx::Row;
    let pool = catalog.pool_clone();
    let mut base_paths = Vec::new();
    for (param, _) in TEMPLATES {
        let path: String = sqlx::query("SELECT storage_path FROM datasets WHERE parameter = $1")
            .bind(param)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        base_paths.push(path);
    }

    // Copy each template grid's objects to COPIES_PER_TEMPLATE storage paths.
    let copy_started = Instant::now();
    let mut objects = 0;
    for base in &base_paths {
        let stem = base.trim_end_matches(".zarr");
        let keys = storage.list(base).await.unwrap();
        assert!(!keys.is_empty(), "no objects under {base}");
        for copy in 0..COPIES_PER_TEMPLATE {
            let new_base = format!("{stem}-copy{copy}.zarr");
            for key in &keys {
                let data = storage.get(key).await.unwrap();
                storage
                    .put(&key.replacen(base.as_str(), &new_base, 1), data)
                    .await
                    .unwrap();
                objects += 1;
            }
        }
    }
    println!(
        "copied {objects} objects ({} grids x {COPIES_PER_TEMPLATE}) in {:.1?}",
        TEMPLATES.len(),
        copy_started.elapsed()
    );

    // Register the synthetic rows with one statement: everything but the key columns comes from
    // the template row (same grid, so the same shape and projection).
    let rows = scenario();
    let ingest_base = Utc::now() - chrono::Duration::hours(2);
    let mut parameter = Vec::new();
    let mut level = Vec::new();
    let mut reference = Vec::new();
    let mut hour = Vec::new();
    let mut template = Vec::new();
    let mut path = Vec::new();
    let mut ingested = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        parameter.push(row.parameter.to_string());
        level.push(row.level.to_string());
        reference.push(run_time(row.run));
        hour.push(row.hour as i32);
        template.push(TEMPLATES[row.template].0.to_string());
        let base = &base_paths[row.template];
        path.push(format!(
            "{}-copy{}.zarr",
            base.trim_end_matches(".zarr"),
            row.copy
        ));
        ingested.push(ingest_base + chrono::Duration::milliseconds(i as i64 * 10));
    }
    let inserted = sqlx::query(
        "INSERT INTO datasets (id, model, parameter, level, reference_time, forecast_hour, \
           valid_time, bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, storage_path, file_size, \
           ingested_at, status, zarr_metadata, forecast_minute) \
         SELECT gen_random_uuid(), 'hrrr', x.parameter, x.level, x.reference_time, x.hour, \
           x.reference_time + x.hour * interval '1 hour', \
           t.bbox_min_x, t.bbox_min_y, t.bbox_max_x, t.bbox_max_y, x.path, t.file_size, \
           x.ingested_at, 'available', t.zarr_metadata, 0 \
         FROM unnest($1::text[], $2::text[], $3::timestamptz[], $4::int[], $5::text[], \
                     $6::text[], $7::timestamptz[]) \
              AS x(parameter, level, reference_time, hour, template, path, ingested_at) \
         JOIN datasets t ON t.model = 'hrrr' AND t.parameter = x.template \
                        AND t.reference_time = $8 AND t.forecast_hour = 2",
    )
    .bind(&parameter)
    .bind(&level)
    .bind(&reference)
    .bind(&hour)
    .bind(&template)
    .bind(&path)
    .bind(&ingested)
    .bind(at(9, 23))
    .execute(&pool)
    .await
    .unwrap()
    .rows_affected() as usize;
    assert_eq!(inserted, rows.len());
    sqlx::query("ANALYZE datasets")
        .execute(&pool)
        .await
        .unwrap();

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
        // Larger than the working set of one request is not wanted: production's cache is
        // 3 GiB, but a cold request is what matters here and `clear_cache` empties it anyway.
        256,
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
    Env {
        app,
        state,
        rows: rows.len(),
        _infra: infra,
    }
}

/// The old per-step code path, kept here as the oracle: for every instant ask the catalog for
/// the dataset nearest to it (`DatasetQuery::at_valid_time` -> `find_by_time[_and_level]`), read
/// the grid, one after another. This is what the handler's loop did for each step. Returns the
/// values and the unit the handler reported then: that of the first grid that answered.
async fn per_step_oracle(
    state: &AppState,
    parameter: &str,
    level: Option<&str>,
    times: &[DateTime<Utc>],
    lon: f64,
    lat: f64,
) -> (Vec<Option<f32>>, String) {
    let mut out = Vec::new();
    let mut units = String::new();
    for t in times {
        let mut q = DatasetQuery::forecast("hrrr", parameter).at_valid_time(*t);
        if let Some(l) = level {
            q = q.at_level(l);
        }
        let point = state.grid_data_service.read_point(&q, lon, lat).await.ok();
        if let (true, Some(p)) = (units.is_empty(), &point) {
            units = p.units.clone();
        }
        out.push(point.and_then(|p| p.value));
    }
    (out, units)
}

// The grid reader blocks internally (block_in_place), which needs a multi-threaded runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn the_forecast_series_equals_the_per_step_path_on_every_instant() {
    let env = setup().await;
    let (app, state) = (&env.app, &env.state);
    println!("catalog: {} synthetic rows + 4 real ones", env.rows);

    // Denver, Boston, and a point outside the HRRR domain.
    let points = [(-104.99, 39.74), (-71.06, 42.36), (10.0, 50.0)];
    let window = "datetime=2026-10-09T00:00:00Z/2026-10-17T00:00:00Z";
    let atmosphere = ["PWAT", "REFC", "TCDC"];

    let mut compared = 0;
    let mut distinct = std::collections::BTreeSet::new();
    let mut units = std::collections::BTreeSet::new();
    for (lon, lat) in points {
        let uri = format!(
            "/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})\
             &parameter-name={}&{window}",
            atmosphere.join(",")
        );
        let before = state.grid_data_service.catalog_lookups();
        let (status, body) = get_json(app, &uri).await;
        let lookups = state.grid_data_service.catalog_lookups() - before;
        assert_eq!(status, 200, "{body}");

        for param in atmosphere {
            let (times, got) = series(&body, param);
            // ascending, one per distinct valid time any parameter has
            assert!(times.windows(2).all(|w| w[0] < w[1]), "times ascending");
            assert_eq!(
                times.len(),
                139,
                "{param}: 10-10T00z .. 10-15T18z + fixture"
            );
            let (want, want_units) =
                per_step_oracle(state, param, Some("entire atmosphere"), &times, lon, lat).await;
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(g, w, "{param} at ({lon},{lat}) {}", times[i]);
            }
            assert_eq!(unit_of(&body, param), want_units, "{param} unit");
            units.insert(want_units);
            compared += got.len();
            distinct.extend(got.iter().flatten().map(|v| v.to_bits()));
        }
        // The old path: one lookup per (parameter, instant). The new one: one per parameter,
        // plus the per-instant lookup for each instant the list could not settle (ties and
        // the tail of TCDC), which is a handful.
        println!(
            "({lon},{lat}): {lookups} catalog lookups for {} parameters x 139 instants (was {})",
            atmosphere.len(),
            atmosphere.len() * 139
        );
        assert!(
            lookups > 3,
            "{lookups}: the tie and tail instants must go through the real lookup"
        );
        assert!(lookups <= 3 + 12, "{lookups}");
    }
    assert_eq!(compared, 3 * 3 * 139);
    // not vacuous: the four real grids, picked differently per row, give many distinct values
    assert!(
        distinct.len() >= 8,
        "only {} distinct values",
        distinct.len()
    );
    // ... and the parameters report different units (each is the unit of the first grid to answer)
    assert!(units.len() >= 2 && !units.contains(""), "units {units:?}");

    // The mean sea level collection is a second model-level query of its own.
    let (lon, lat) = (-104.99, 39.74);
    let (status, body) = get_json(
        app,
        &format!(
            "/edr/collections/hrrr-mean-sea-level/position?coords=POINT({lon}%20{lat})\
             &parameter-name=MSLMA&{window}"
        ),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (times, got) = series(&body, "MSLMA");
    let (want, want_units) =
        per_step_oracle(state, "MSLMA", Some("mean sea level"), &times, lon, lat).await;
    assert_eq!(got, want);
    assert_eq!(unit_of(&body, "MSLMA"), want_units);
    assert!(
        got.iter().all(|v| v.is_some()),
        "MSLMA has a value everywhere"
    );

    // No `parameter-name`: every parameter with data (the response's `ranges` is a map, its key
    // order is not meaningful), each with the same series as when asked for by name.
    let (status, all) = get_json(
        app,
        &format!("/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})&{window}"),
    )
    .await;
    assert_eq!(status, 200, "{all}");
    let mut ranges: Vec<&str> = all["ranges"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    ranges.sort();
    assert_eq!(ranges, ["PWAT", "REFC", "TCDC"]);
    let (status, named) = get_json(
        app,
        &format!(
            "/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})\
             &parameter-name=TCDC,PWAT,REFC&{window}"
        ),
    )
    .await;
    assert_eq!(status, 200, "{named}");
    for param in atmosphere {
        assert_eq!(series(&all, param), series(&named, param), "{param}");
    }

    // A repeated parameter is resolved once, in the batched way, and answers like the single one.
    let before = state.grid_data_service.catalog_lookups();
    let (status, repeated) = get_json(
        app,
        &format!(
            "/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})\
             &parameter-name=TCDC,PWAT,TCDC&{window}"
        ),
    )
    .await;
    let lookups = state.grid_data_service.catalog_lookups() - before;
    assert_eq!(status, 200, "{repeated}");
    for param in ["TCDC", "PWAT"] {
        assert_eq!(series(&repeated, param), series(&named, param), "{param}");
        assert_eq!(unit_of(&repeated, param), unit_of(&named, param), "{param}");
    }
    assert!(
        lookups <= 2 + 12,
        "{lookups}: the repeat took the per-step path"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn the_forecast_series_equals_the_per_step_path_on_narrow_windows_lists_and_interpolation() {
    let env = setup().await;
    let (app, state) = (&env.app, &env.state);
    let (lon, lat) = (-104.99, 39.74);
    let level = Some("entire atmosphere");

    // Windows that put the unsettled instants at the first/last/only positions of a series, and
    // a 2 instant series (the smallest that takes the batched path), and a list.
    let windows = [
        "2026-10-12T05:00:00Z/2026-10-12T07:00:00Z", // the REFC tie in the middle
        "2026-10-12T06:00:00Z/2026-10-12T07:00:00Z", // ... at the start
        "2026-10-12T05:00:00Z/2026-10-12T06:00:00Z", // ... at the end
        "2026-10-13T00:00:00Z/2026-10-13T06:00:00Z", // inside TCDC's 7 hour gap
        "2026-10-15T00:00:00Z/2026-10-15T18:00:00Z", // TCDC's tail, MSLMA's short run
        "2026-10-10T00:00:00Z/2026-10-10T03:00:00Z", // the head
        "2026-10-10T12:00:00Z/2026-10-10T16:00:00Z", // PWAT's hole in the newest run
        "2026-10-12T05:00:00Z,2026-10-12T06:00:00Z,2026-10-14T00:00:00Z", // a list
    ];
    for window in windows {
        let uri = format!(
            "/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})\
             &parameter-name=PWAT,REFC,TCDC&datetime={window}"
        );
        let (status, body) = get_json(app, &uri).await;
        assert_eq!(status, 200, "{window}: {body}");
        for param in ["PWAT", "REFC", "TCDC"] {
            let (times, got) = series(&body, param);
            assert!(!times.is_empty(), "{window}");
            let (want, want_units) = per_step_oracle(state, param, level, &times, lon, lat).await;
            assert_eq!(got, want, "{param} over {window}");
            assert_eq!(
                unit_of(&body, param),
                want_units,
                "{param} unit over {window}"
            );
        }
    }

    // Interpolated series: the batched read must also cover the instants a step interpolates
    // between. Hourly datasets, half-hourly steps: the :00 values are the stored ones, each :30
    // value is the mean of its neighbours (weight 0.5).
    let uri = format!(
        "/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})\
         &parameter-name=REFC&datetime=2026-10-11T00:00:00Z/2026-10-11T06:00:00Z\
         &interpolation=linear&step=PT30M"
    );
    let (status, body) = get_json(app, &uri).await;
    assert_eq!(status, 200, "{body}");
    let (times, got) = series(&body, "REFC");
    assert_eq!(times.len(), 13);
    let hourly: Vec<DateTime<Utc>> = times.iter().copied().step_by(2).collect();
    let (stored, stored_units) = per_step_oracle(state, "REFC", level, &hourly, lon, lat).await;
    assert_eq!(unit_of(&body, "REFC"), stored_units);
    for (k, expect) in stored.iter().enumerate() {
        assert_eq!(&got[2 * k], expect, "{}", hourly[k]);
    }
    for k in 0..stored.len() - 1 {
        let (a, b) = (stored[k].unwrap(), stored[k + 1].unwrap());
        let mid = got[2 * k + 1].unwrap();
        assert!(
            (mid - (a + b) / 2.0).abs() <= 1e-3 * (1.0 + a.abs() + b.abs()),
            "{}: {mid} is not the mean of {a} and {b}",
            times[2 * k + 1]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn the_forecast_series_without_a_level_filter_equals_the_per_step_path() {
    let env = setup().await;
    let (app, state) = (&env.app, &env.state);
    let (lon, lat) = (-104.99, 39.74);

    // `hrrr-cloud-layers` has no `level_code`, so `build_level_string` gives None and both
    // lookups (`find_by_time`, the series' range query) take the nearest dataset of any level.
    let uri = format!(
        "/edr/collections/hrrr-cloud-layers/position?coords=POINT({lon}%20{lat})\
         &parameter-name=LCDC&datetime=2026-10-09T00:00:00Z/2026-10-17T00:00:00Z"
    );
    let before = state.grid_data_service.catalog_lookups();
    let (status, body) = get_json(app, &uri).await;
    let lookups = state.grid_data_service.catalog_lookups() - before;
    assert_eq!(status, 200, "{body}");
    let (times, got) = series(&body, "LCDC");
    assert_eq!(times.len(), 139);

    let (want, want_units) = per_step_oracle(state, "LCDC", None, &times, lon, lat).await;
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "LCDC at {}", times[i]);
    }
    assert!(
        got.iter().all(|v| v.is_some()),
        "LCDC has a value everywhere"
    );
    assert_eq!(unit_of(&body, "LCDC"), want_units);
    assert!(!want_units.is_empty());
    // every instant has a dataset of its own: the one range query settles them all
    assert_eq!(lookups, 1);

    // not vacuous: restricting the lookup to one level answers differently
    let (low_only, _) =
        per_step_oracle(state, "LCDC", Some("low cloud layer"), &times, lon, lat).await;
    assert_ne!(
        got, low_only,
        "the level filter must matter in this scenario"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn a_failed_range_query_leaves_each_instant_to_its_own_lookup() {
    let env = setup().await;
    let state = &env.state;
    let (lon, lat) = (-104.99, 39.74);
    let level = "entire atmosphere";

    // Postgres has no timestamps before 4713 BC, so the range query for a window reaching back
    // from this instant is rejected by the server. (So is the instant's own lookup.)
    let ancient = Utc.with_ymd_and_hms(-5000, 1, 1, 0, 0, 0).unwrap();
    let real = at(11, 0);
    let before = state.grid_data_service.catalog_lookups();
    let got = state
        .grid_data_service
        .read_forecast_point_series("hrrr", "PWAT", Some(level), &[ancient, real], lon, lat)
        .await;
    let lookups = state.grid_data_service.catalog_lookups() - before;

    // the range query, then one lookup per instant it could not settle
    assert_eq!(lookups, 3);
    assert_eq!(got.len(), 2);
    let err = got[0].as_ref().expect_err("a time Postgres cannot store");
    assert!(
        err.to_string().to_lowercase().contains("out of range"),
        "{err}"
    );
    // the failed query did not take the other instant down with it
    let want = state
        .grid_data_service
        .read_point(
            &DatasetQuery::forecast("hrrr", "PWAT")
                .at_valid_time(real)
                .at_level(level),
            lon,
            lat,
        )
        .await
        .unwrap();
    let got = got[1].as_ref().expect("the real instant is answered");
    assert!(want.value.is_some());
    assert_eq!((got.value, &got.units), (want.value, &want.units));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn timing_old_per_step_loop_versus_the_new_handler() {
    let env = setup().await;
    let (app, state) = (&env.app, &env.state);
    let (lon, lat) = (-104.99, 39.74);
    let atmosphere = ["PWAT", "REFC", "TCDC"];
    let uri = format!(
        "/edr/collections/hrrr-atmosphere/position?coords=POINT({lon}%20{lat})\
         &parameter-name={}&datetime=2026-10-09T00:00:00Z/2026-10-17T00:00:00Z",
        atmosphere.join(",")
    );

    // The instants the handler answers for.
    let times = {
        let (status, body) = get_json(app, &uri).await;
        assert_eq!(status, 200, "{body}");
        series(&body, "PWAT").0
    };

    // The old loop: parameter after parameter, instant after instant.
    let old = || async {
        let started = Instant::now();
        let mut n = 0;
        for param in atmosphere {
            n += per_step_oracle(state, param, Some("entire atmosphere"), &times, lon, lat)
                .await
                .0
                .len();
        }
        (started.elapsed(), n)
    };
    // The handler, HTTP parsing and JSON included.
    let new = || async {
        let started = Instant::now();
        let (status, body) = get_json(app, &uri).await;
        assert_eq!(status, 200, "{body}");
        (started.elapsed(), series(&body, "PWAT").0.len())
    };

    let mut report = Vec::new();
    for round in 0..3 {
        state.grid_data_service.clear_cache().await;
        let l0 = state.grid_data_service.catalog_lookups();
        let (t_new_cold, n) = new().await;
        let new_lookups = state.grid_data_service.catalog_lookups() - l0;
        let (t_new_warm, _) = new().await;

        state.grid_data_service.clear_cache().await;
        let l0 = state.grid_data_service.catalog_lookups();
        let (t_old_cold, steps) = old().await;
        let old_lookups = state.grid_data_service.catalog_lookups() - l0;
        let (t_old_warm, _) = old().await;
        report.push((
            round,
            n,
            steps,
            t_old_cold,
            t_old_warm,
            t_new_cold,
            t_new_warm,
            old_lookups,
            new_lookups,
        ));
    }
    println!();
    println!(
        "3 parameters x 139 instants over a 4 run catalog of {} datasets (+ decoys), local Docker \
         Postgres and MinIO, {} distinct grids",
        env.rows,
        TEMPLATES.len() * COPIES_PER_TEMPLATE
    );
    println!("round | steps | old cold | old warm | new cold | new warm | lookups old -> new");
    for (round, _n, steps, oc, ow, nc, nw, ol, nl) in &report {
        println!(
            "{round:>5} | {steps:>5} | {oc:>8.1?} | {ow:>8.1?} | {nc:>8.1?} | {nw:>8.1?} | {ol} -> {nl}"
        );
    }
}
