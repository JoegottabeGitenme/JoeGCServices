//! End-to-end: the EDR `area` query as PNG, from REAL HRRR and MRMS messages through the REAL
//! ingester, Postgres, MinIO and the REAL area handler. `#[ignore]`d (Docker):
//!
//! ```text
//! cargo test -p edr-api --test area_png -- --ignored --nocapture
//! ```
//!
//! What it pins:
//! - `existing_requests_return_the_same_png_bytes`: the requests the deployed app and the other
//!   clients send answer with the bytes and headers they always did. The expected SHA-256s were
//!   recorded from the handler as it was BEFORE the read-once / overview / crop work (it looked the
//!   dataset up and read and reprojected it three times for a PNG; the answer is the same). A
//!   change here is a change to what clients see.
//! - `crop_true_*`: `crop=true` on HRRR's Lambert grid answers for exactly the polygon's box, with
//!   the value the file holds at cells whose longitude/latitude `wgrib2 -ijlat` gave.
//! - `an_instance_*` and `cache_control_*`: the run and datetime of an instance request are
//!   honoured, and a named time the catalog did not have is not cached as that time.
//!
//! The fixtures are real messages (see `crates/ingestion/tests/fixtures`).

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use sha2::{Digest, Sha256};
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

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ingestion/tests/fixtures")
        .join(rel)
        .to_string_lossy()
        .to_string()
}

struct Reply {
    status: u16,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }

    fn bbox(&self) -> Vec<f64> {
        self.header("x-edr-bbox")
            .split(',')
            .map(|v| v.parse().unwrap())
            .collect()
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// The PNG as physical values, `None` where it is no-data. Row 0 is the north edge.
    fn decode(&self) -> Decoded {
        assert_eq!(self.status, 200, "{}", self.text());
        let mut reader = png::Decoder::new(Cursor::new(&self.body))
            .read_info()
            .expect("a PNG");
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).expect("a frame");
        assert_eq!(info.color_type, png::ColorType::GrayscaleAlpha);
        let (min, max): (f64, f64) = (
            self.header("x-edr-min").parse().unwrap(),
            self.header("x-edr-max").parse().unwrap(),
        );
        let (full, bytes) = match info.bit_depth {
            png::BitDepth::Sixteen => (65535.0, 2),
            png::BitDepth::Eight => (255.0, 1),
            other => panic!("{other:?}"),
        };
        let sample = |i: usize| -> f64 {
            let at = i * bytes;
            if bytes == 2 {
                u16::from_be_bytes([buf[at], buf[at + 1]]) as f64
            } else {
                buf[at] as f64
            }
        };
        let values = (0..info.width as usize * info.height as usize)
            .map(|px| {
                // gray, alpha
                (sample(px * 2 + 1) > 0.0).then(|| min + sample(px * 2) / full * (max - min))
            })
            .collect();
        Decoded {
            width: info.width as usize,
            height: info.height as usize,
            values,
        }
    }
}

struct Decoded {
    width: usize,
    height: usize,
    values: Vec<Option<f64>>,
}

impl Decoded {
    fn at(&self, x: usize, y: usize) -> Option<f64> {
        self.values[y * self.width + x]
    }
}

async fn get_reply(app: &Router, uri: &str) -> Reply {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body = axum::body::to_bytes(resp.into_body(), 64 << 20)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

/// `POLYGON((w s,e s,e n,w n,w s))` as it goes in a URL.
fn rect(w: f64, s: f64, e: f64, n: f64) -> String {
    format!("POLYGON(({w}%20{s},{e}%20{s},{e}%20{n},{w}%20{n},{w}%20{s}))")
}

async fn setup() -> (Router, Arc<AppState>, TestInfrastructure) {
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
    for file in [
        "hrrr-subh/hrrr-subh_20261011_18z_f001.grib2",
        "hrrr-subh/hrrr-subh_20261011_18z_f000.grib2",
        "hrrr-subh/hrrr-subh_20261011_18z_f002.grib2",
        "hrrr/hrrr_20261009_23z_f002.grib2",
        "mrms/mrms_MRMS_PrecipFlag_00.00_20261009-230000.grib2.gz",
        "mrms/mrms_MRMS_MESH_00.50_20261009-230041.grib2.gz",
    ] {
        ingester
            .ingest_file(&fixture(file), IngestOptions::default())
            .await
            .unwrap_or_else(|e| panic!("{file}: {e}"));
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
            "/edr/collections/:collection_id/area",
            get(handlers::area::area_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/area",
            get(handlers::area::instance_area_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/position",
            get(handlers::position::instance_position_handler),
        )
        .route(
            "/edr/collections/:collection_id/radius",
            get(handlers::radius::radius_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/radius",
            get(handlers::radius::instance_radius_handler),
        )
        .layer(Extension(Arc::clone(&state)));
    (app, state, infra)
}

// ---------------------------------------------------------------------------------------------
// Existing requests: same bytes
// ---------------------------------------------------------------------------------------------

/// The polygon the app sends for every HRRR grid, so that every request is the same cacheable URL
/// and the answer is the whole model domain.
fn app_polygon() -> String {
    rect(-106.0, 38.5, -104.5, 40.9)
}

/// `(name, uri)` of the requests whose answers are pinned.
fn pinned_requests() -> Vec<(&'static str, String)> {
    let a = "/edr/collections";
    vec![
        (
            "hrrr whole domain, as the app asks (1799x1059 for a tiny polygon)",
            format!("{a}/hrrr-subh-atmosphere/area?coords={}&parameter-name=REFC&f=png&width=1799&height=1059&datetime=2026-10-11T18:15:00Z", app_polygon()),
        ),
        (
            "hrrr whole domain, 8-bit",
            format!("{a}/hrrr-subh-atmosphere/area?coords={}&parameter-name=REFC&f=png&width=1799&height=1059&datetime=2026-10-11T18:30:00Z&depth=8", app_polygon()),
        ),
        (
            "hrrr categorical flag, as the app asks",
            format!("{a}/hrrr-subh-surface/area?coords={}&parameter-name=CSNOW&f=png&width=1799&height=1059&datetime=2026-10-11T18:00:00Z", app_polygon()),
        ),
        (
            "hrrr trail polygon without crop: still the whole domain, squeezed into the image",
            format!("{a}/hrrr-subh-atmosphere/area?coords={}&parameter-name=REFC&f=png&width=250&height=150&datetime=2026-10-11T18:15:00Z", rect(-106.0, 39.0, -103.5, 40.5)),
        ),
        (
            "hrrr, no size, newest",
            format!("{a}/hrrr-subh-atmosphere/area?coords={}&parameter-name=REFC&f=png", app_polygon()),
        ),
        (
            "hrrr hourly collection",
            format!("{a}/hrrr-atmosphere/area?coords={}&parameter-name=REFC&f=png&datetime=2026-10-10T01:00:00Z", app_polygon()),
        ),
        (
            "hrrr, triangle polygon (masked)",
            format!("{a}/hrrr-subh-atmosphere/area?coords=POLYGON((-120%2030,-80%2030,-100%2048,-120%2030))&parameter-name=REFC&f=png&width=300&height=200&datetime=2026-10-11T18:00:00Z"),
        ),
        (
            "hrrr, web mercator",
            format!("{a}/hrrr-subh-atmosphere/area?coords={}&parameter-name=REFC&f=png&width=400&height=200&crs=EPSG:3857&datetime=2026-10-11T18:00:00Z", app_polygon()),
        ),
        (
            "mrms precipitation type, small image of a large area",
            format!("{a}/mrms-single-level/area?coords={}&parameter-name=PRECIP_FLAG&f=png&width=64&height=40", rect(-110.0, 30.0, -90.0, 42.0)),
        ),
        (
            "mrms hail size, no size",
            format!("{a}/mrms-single-level/area?coords={}&parameter-name=MESH&f=png", rect(-105.75, 39.5, -103.25, 41.0)),
        ),
        (
            "mrms hail size, native-ish size, dated",
            format!("{a}/mrms-single-level/area?coords={}&parameter-name=MESH&f=png&width=600&height=300&datetime=2026-10-09T23:00:41Z", rect(-100.0, 35.0, -94.0, 38.0)),
        ),
        (
            "covjson, mrms, two parameters over a triangle",
            format!("{a}/mrms-single-level/area?coords=POLYGON((-100%2035,-94%2035,-97%2038,-100%2035))&parameter-name=MESH,PRECIP_FLAG"),
        ),
        (
            "geojson, mrms",
            format!("{a}/mrms-single-level/area?coords={}&parameter-name=MESH&f=geojson", rect(-100.0, 35.0, -97.0, 37.0)),
        ),
        (
            "mrms hail size, polygon off the data",
            format!("{a}/mrms-single-level/area?coords={}&parameter-name=MESH&f=png", rect(10.0, 10.0, 12.0, 12.0)),
        ),
    ]
}

/// Recorded from the handler before this change (set `RECORD_AREA_PNG=1` to print new ones).
/// `(name, status, cache-control, sha256 of the body)`; the headers are pinned in the digest too.
/// A CoverageJSON body is hashed with its keys in sorted order (see `digest`).
const PINNED: &[(&str, u16, &str, &str)] = &[
    (
        "hrrr whole domain, as the app asks (1799x1059 for a tiny polygon)",
        200,
        "max-age=900",
        "fba7ad170d06288edc688cdb8d51ab6019eddc2a3134c476a218408c493b6c3d",
    ),
    (
        "hrrr whole domain, 8-bit",
        200,
        "max-age=900",
        "72cbb00762283465c5d0c5277dc07f3a867fc1e04c4640fcebc39a93810dcbf2",
    ),
    (
        "hrrr categorical flag, as the app asks",
        200,
        "max-age=900",
        "5e812920a0dd3fd5ba289bea7d8019a0dc716e694b8aec3b2bd6718451f40725",
    ),
    (
        "hrrr trail polygon without crop: still the whole domain, squeezed into the image",
        200,
        "max-age=900",
        "278827db04c45577cb04d7f1bbe2335a0219360996e3ca556d00f52f6f7b53ba",
    ),
    (
        "hrrr, no size, newest",
        200,
        "max-age=60",
        "24dffeb747c3a8bc7dbf9d4221ac1ad0648c0c38a462e2322bb84fbd91e4fa8c",
    ),
    (
        "hrrr hourly collection",
        200,
        "max-age=3600",
        "4c826a7a3744695a9c687bdf7be67b7789c77b03a4d7de2aba146c3c55f36632",
    ),
    (
        "hrrr, triangle polygon (masked)",
        200,
        "max-age=900",
        "39c29052c74f229658c4fbf379877e1012f5c4b99975d2d8a9887a6841db4b86",
    ),
    (
        "hrrr, web mercator",
        200,
        "max-age=900",
        "4bd136866e929470ee9c92ecc7203f8db322d597fc55c8b36c14b1e8f203d12e",
    ),
    (
        "mrms precipitation type, small image of a large area",
        200,
        "max-age=60",
        "c138a3c68d68eccaaaee5de1ae3e390241213be53c05c72d8cd66bdb283ad776",
    ),
    (
        "mrms hail size, no size",
        200,
        "max-age=60",
        "afa5e93ee87dd1723cb118de63627509fcb9a8aad0658667d926834ec857c08c",
    ),
    (
        "mrms hail size, native-ish size, dated",
        200,
        "max-age=300",
        "0e3d4421c536400c9d226c3cf14980c45204c70b6a7e0ea679fad653034f2162",
    ),
    (
        "covjson, mrms, two parameters over a triangle",
        200,
        "max-age=60",
        "22fb5026a8b8c4d38cab74fbdd244bd3943486f5020ed852fe15fe9178db5107",
    ),
    (
        "geojson, mrms",
        200,
        "max-age=60",
        "fd864f0b6c4e58cc0031a5d1f808e0f25702715e645d146db89ec6b46b9f892b",
    ),
    (
        "mrms hail size, polygon off the data",
        400,
        "",
        "adb8f5057ad3f10b5a2caffacafdcd36e5c560e851a1b47c7972fefb18668f8d",
    ),
];

/// `value` as compact JSON with the keys of every object in sorted order.
fn canonical_json(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap());
                out.push(':');
                canonical_json(&map[key], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// What a client gets, as a SHA-256. A CoverageJSON body is hashed as JSON: the order of an
/// object's keys is not part of the answer, and CoverageJSON does not keep one (its axes,
/// parameters and ranges are `HashMap`s, so the keys come out in a different order from one
/// process to the next). Numbers are read as `f64` and written back, which with the
/// `float_roundtrip` feature this workspace builds `serde_json` with gives the digits the handler
/// wrote. Every other body is hashed as it is.
fn digest(r: &Reply) -> String {
    let mut h = Sha256::new();
    if r.header("content-type").contains("cov+json") {
        let mut canonical = String::new();
        canonical_json(
            &serde_json::from_slice(&r.body).expect("a JSON body"),
            &mut canonical,
        );
        h.update(canonical.as_bytes());
    } else {
        h.update(&r.body);
    }
    for name in [
        "content-type",
        "x-edr-parameter",
        "x-edr-units",
        "x-edr-min",
        "x-edr-max",
        "x-edr-encoding",
        "x-edr-bbox",
        "x-edr-crs",
        "x-edr-width",
        "x-edr-height",
    ] {
        h.update(name.as_bytes());
        h.update(r.header(name).as_bytes());
    }
    format!("{:x}", h.finalize())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn existing_requests_return_the_same_png_bytes() {
    let (app, state, _infra) = setup().await;
    let record = std::env::var("RECORD_AREA_PNG").is_ok();

    for (name, uri) in pinned_requests() {
        let before = state.grid_data_service.catalog_lookups();
        let started = std::time::Instant::now();
        let r = get_reply(&app, &uri).await;
        let first = started.elapsed();
        let lookups = state.grid_data_service.catalog_lookups() - before;
        // (when recording: also the best of four more, with the chunks cached)
        let mut warm = first;
        for _ in 0..if record { 4 } else { 0 } {
            let started = std::time::Instant::now();
            get_reply(&app, &uri).await;
            warm = warm.min(started.elapsed());
        }
        let elapsed = format!("first {first:?}, warm {warm:?}");
        let sum = digest(&r);
        let cache = r.header("cache-control");
        if record {
            println!(
                "    ({name:?}, {}, {cache:?}, {sum:?}),   // {}x{} bbox {} -- {lookups} catalog lookups, {elapsed}",
                r.status,
                r.header("x-edr-width"),
                r.header("x-edr-height"),
                r.header("x-edr-bbox")
            );
            continue;
        }
        // The dataset is looked up once however many times it used to be read
        if uri.contains("f=png") {
            assert_eq!(lookups, 1, "{name}: catalog lookups for one PNG");
        }
        let want = PINNED
            .iter()
            .find(|p| p.0 == name)
            .unwrap_or_else(|| panic!("{name}: no recorded answer"));
        assert_eq!((r.status, cache.as_str()), (want.1, want.2), "{name}");
        assert_eq!(sum, want.3, "{name}: the PNG or its headers changed");
    }
    assert!(record || PINNED.len() == pinned_requests().len());

    // When recording, also time the requests `crop=true` is for (an older server ignores the
    // parameter and answers each with the whole domain)
    if record {
        let a = "/edr/collections/hrrr-subh-atmosphere/area";
        for (name, polygon, size) in [
            (
                "trail 2.5x1.5 deg",
                rect(-106.0, 39.0, -103.5, 40.5),
                "width=250&height=150",
            ),
            (
                "state 10x6 deg",
                rect(-110.0, 36.0, -100.0, 42.0),
                "width=500&height=300",
            ),
            (
                "region 20x12 deg",
                rect(-110.0, 33.0, -90.0, 45.0),
                "width=800&height=480",
            ),
            (
                "conus 59x26 deg",
                rect(-125.0, 24.0, -66.0, 50.0),
                "width=1024&height=512",
            ),
        ] {
            let uri = format!(
                "{a}?coords={polygon}&parameter-name=REFC&f=png&{size}&datetime=2026-10-11T18:15:00Z&crop=true"
            );
            let started = std::time::Instant::now();
            let r = get_reply(&app, &uri).await;
            let first = started.elapsed();
            let mut warm = first;
            for _ in 0..4 {
                let started = std::time::Instant::now();
                get_reply(&app, &uri).await;
                warm = warm.min(started.elapsed());
            }
            println!(
                "CROP {name} {size}: {} bytes, bbox {}, first {first:?}, warm {warm:?}",
                r.body.len(),
                r.header("x-edr-bbox")
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// crop=true
// ---------------------------------------------------------------------------------------------

// Cells of the 3 km grid where `wgrib2 -ijlat` on hrrr.t18z.wrfsubhf00 puts their centres, with
// the composite reflectivity (dBZ) the file holds there (see hrrr_subhourly.rs).
const SNOW: (f64, f64, f64) = (-112.409346, 44.582824, 26.19);
const RAIN: (f64, f64, f64) = (-71.350535, 36.725810, 27.31);

/// A box around `(lon, lat)` whose middle pixel is centred on it: `w` x `h` pixels, `w` and `h` odd.
fn around(lon: f64, lat: f64, half: (f64, f64)) -> String {
    rect(lon - half.0, lat - half.1, lon + half.0, lat + half.1)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn crop_true_answers_for_exactly_the_polygon_with_the_values_the_file_holds() {
    let (app, state, _infra) = setup().await;
    let a = "/edr/collections/hrrr-subh-atmosphere/area";
    let (w, h) = (33usize, 21usize);

    for (name, (lon, lat, want)) in [("snow", SNOW), ("rain", RAIN)] {
        let half = (0.2, 0.12);
        let uri = format!(
            "{a}?coords={}&parameter-name=REFC&f=png&width={w}&height={h}&datetime=2026-10-11T18:00:00Z&crop=true",
            around(lon, lat, half)
        );
        let (lookups, chunks) = (
            state.grid_data_service.catalog_lookups(),
            state.grid_data_service.cache_stats().await.entries,
        );
        let r = get_reply(&app, &uri).await;
        assert_eq!(r.status, 200, "{name}: {}", r.text());

        // ONE catalog lookup for the whole request (it was six), and on a cold cache one or two
        // of the grid's twelve chunks (the second box is read after the first loop whole)
        let read = state.grid_data_service.cache_stats().await.entries - chunks;
        assert_eq!(
            state.grid_data_service.catalog_lookups() - lookups,
            1,
            "{name}"
        );
        if chunks == 0 {
            assert!((1..=2).contains(&read), "{name}: {read} chunks read");
        }

        // the box, the size, nothing else
        let bbox = r.bbox();
        let want_bbox = [lon - half.0, lat - half.1, lon + half.0, lat + half.1];
        for (got, want) in bbox.iter().zip(want_bbox) {
            assert!(
                (got - want).abs() < 1e-9,
                "{name}: {bbox:?} vs {want_bbox:?}"
            );
        }
        assert_eq!(
            (r.header("x-edr-width"), r.header("x-edr-height")),
            (w.to_string(), h.to_string())
        );
        let img = r.decode();
        assert_eq!((img.width, img.height), (w, h));

        // and the middle pixel is the cell the file says is at that longitude/latitude
        let middle = img.at(w / 2, h / 2).expect("data at the cell");
        assert!(
            (middle - want).abs() < 0.02,
            "{name}: got {middle} dBZ, file has {want}"
        );
        assert!(
            img.values.iter().all(|v| v.is_some()),
            "{name}: all on the grid"
        );

        // Without crop the answer is the whole domain: a megapixel image for the same polygon.
        // (This is also what crop=true gives on a server that does not know the parameter.)
        let whole_uri = format!(
            "{a}?coords={}&parameter-name=REFC&f=png&width=1799&height=1059&datetime=2026-10-11T18:00:00Z",
            around(lon, lat, half)
        );
        let whole = get_reply(&app, &whole_uri).await;
        let wb = whole.bbox();
        assert!(wb[2] - wb[0] > 60.0, "{name}: {wb:?}");
        assert!(
            r.body.len() * 20 < whole.body.len(),
            "{name}: {} vs {}",
            r.body.len(),
            whole.body.len()
        );
        // ...and it holds the same field: the pixel of it at that longitude/latitude is within a
        // few dBZ (it is a ~4 km pixel of a bilinear reprojection, not the cell itself)
        let whole = whole.decode();
        let px = (((lon - wb[0]) / (wb[2] - wb[0])) * whole.width as f64) as usize;
        let py = (((wb[3] - lat) / (wb[3] - wb[1])) * whole.height as f64) as usize;
        let coarse = whole.at(px, py).expect("on the grid");
        assert!((coarse - want).abs() < 6.0, "{name}: {coarse} vs {want}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn crop_true_edges_sizes_and_no_data() {
    let (app, _state, _infra) = setup().await;
    let a = "/edr/collections/hrrr-subh-atmosphere/area";
    let when = "datetime=2026-10-11T18:00:00Z";

    // No size: the grid's own cell spacing (a 2 x 1.2 degree box is ~60 x 40 cells)
    let uri = format!(
        "{a}?coords={}&parameter-name=REFC&f=png&{when}&crop=true",
        rect(-113.4, 44.0, -111.4, 45.2)
    );
    let r = get_reply(&app, &uri).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let (rw, rh): (usize, usize) = (
        r.header("x-edr-width").parse().unwrap(),
        r.header("x-edr-height").parse().unwrap(),
    );
    assert!(
        (50..=80).contains(&rw) && (35..=60).contains(&rh),
        "{rw}x{rh}"
    );
    assert_eq!(r.bbox(), vec![-113.4, 44.0, -111.4, 45.2]);

    // A box over the western edge of the domain: no data outside it, data inside it
    let uri = format!(
        "{a}?coords={}&parameter-name=REFC&f=png&width=150&height=50&{when}&crop=true",
        rect(-140.0, 35.0, -125.0, 40.0)
    );
    let r = get_reply(&app, &uri).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.bbox(), vec![-140.0, 35.0, -125.0, 40.0]);
    let img = r.decode();
    assert!(
        (0..img.height).all(|y| img.at(0, y).is_none()),
        "west of the domain"
    );
    assert!(
        (0..img.height).any(|y| img.at(img.width - 1, y).is_some()),
        "on the domain"
    );

    // A MULTIPOLYGON is cropped to the box around ALL its polygons, not the first one's: the
    // second polygon's data is in the image
    let multi = "MULTIPOLYGON(((-113.4%2044,-111.4%2044,-111.4%2045.2,-113.4%2045.2,-113.4%2044)),\
                 ((-100%2038,-98%2038,-98%2039,-100%2039,-100%2038)))";
    let r = get_reply(
        &app,
        &format!(
            "{a}?coords={multi}&parameter-name=REFC&f=png&width=154&height=72&{when}&crop=true"
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.bbox(), vec![-113.4, 38.0, -98.0, 45.2]);
    let img = r.decode();
    assert_eq!((img.width, img.height), (154, 72));
    // -112.4, 44.6 in the first polygon; -99, 38.5 in the second
    assert!(img.at(10, 5).is_some(), "first polygon");
    assert!(img.at(144, 65).is_some(), "second polygon");
    // ...and a pixel in neither (-98.95, 44.65; the first polygon is a rectangle, which a
    // single-polygon request does not mask) is no-data, like the gap between the two
    assert!(img.at(144, 5).is_none(), "in neither polygon");
    assert!(img.at(77, 40).is_none(), "between the polygons");
    assert_eq!(
        img.values.iter().flatten().count(),
        20 * 12 + 20 * 10,
        "only the pixels inside a polygon"
    );

    // A polygon with nothing under it is the handler's usual no-data answer
    let uri = format!(
        "{a}?coords={}&parameter-name=REFC&f=png&width=50&height=50&{when}&crop=true",
        rect(0.0, 45.0, 10.0, 50.0)
    );
    let r = get_reply(&app, &uri).await;
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(r
        .text()
        .contains("No data available for the requested area"));

    // A big area into a small image comes from an overview level and is still the right box
    let big = rect(-110.0, 33.0, -90.0, 45.0);
    let small = get_reply(
        &app,
        &format!("{a}?coords={big}&parameter-name=REFC&f=png&width=100&height=60&{when}&crop=true"),
    )
    .await;
    assert_eq!(small.status, 200, "{}", small.text());
    assert_eq!(small.bbox(), vec![-110.0, 33.0, -90.0, 45.0]);
    assert_eq!(small.decode().values.len(), 100 * 60);
    let large = get_reply(
        &app,
        &format!(
            "{a}?coords={big}&parameter-name=REFC&f=png&width=560&height=440&{when}&crop=true"
        ),
    )
    .await;
    assert_eq!(large.status, 200);
    assert!(small.body.len() < large.body.len());

    // The flag grids are never read from an overview level: they still answer
    let flag = get_reply(
        &app,
        &format!("/edr/collections/hrrr-subh-surface/area?coords={big}&parameter-name=CSNOW&f=png&width=100&height=60&{when}&crop=true"),
    )
    .await;
    assert_eq!(flag.status, 200, "{}", flag.text());
    assert_eq!(flag.bbox(), vec![-110.0, 33.0, -90.0, 45.0]);

    // crop is a PNG option, and a boolean
    let r = get_reply(
        &app,
        &format!(
            "{a}?coords={}&parameter-name=REFC&{when}&crop=true",
            rect(-113.4, 44.0, -111.4, 45.2)
        ),
    )
    .await;
    assert_eq!(r.status, 400, "{}", r.text());
    assert!(r.text().contains("crop is only supported for PNG"));
    let r = get_reply(
        &app,
        &format!("{a}?coords={big}&parameter-name=REFC&f=png&{when}&crop=yes"),
    )
    .await;
    assert_eq!(r.status, 400);
    assert!(r.text().contains("crop"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn crop_true_on_a_lat_lon_grid_changes_nothing() {
    let (app, _state, _infra) = setup().await;
    let a = "/edr/collections/mrms-single-level/area";
    for size in ["", "&width=64&height=40", "&width=600&height=300"] {
        for param in ["MESH", "PRECIP_FLAG"] {
            let uri = format!(
                "{a}?coords={}&parameter-name={param}&f=png{size}",
                rect(-110.0, 30.0, -90.0, 42.0)
            );
            let plain = get_reply(&app, &uri).await;
            let cropped = get_reply(&app, &format!("{uri}&crop=true")).await;
            assert_eq!(plain.status, 200, "{}", plain.text());
            assert_eq!(plain.body, cropped.body, "{param}{size}");
            assert_eq!(plain.bbox(), cropped.bbox());
        }
    }
}

/// A wide polygon into a small image used to come from an overview level for hail size and from
/// the native grid for the precipitation type, which is never read from a level: the two came back
/// with different extents, and the app lays one over the other from a single extent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn a_wide_lat_lon_view_is_on_one_grid_for_every_parameter() {
    let (app, _state, _infra) = setup().await;
    let a = "/edr/collections/mrms-single-level/area";
    // 20 x 12 degrees: 2000 x 1200 cells of the 0.01 degree grid, 250 x 150 at overview level 3
    let polygon = (-110.0, 30.0, -90.0, 42.0);
    for size in ["&width=64&height=40", "&width=200&height=120", ""] {
        let mut grids = Vec::new();
        for param in ["MESH", "PRECIP_FLAG"] {
            let uri = format!(
                "{a}?coords={}&parameter-name={param}&f=png{size}",
                rect(polygon.0, polygon.1, polygon.2, polygon.3)
            );
            let r = get_reply(&app, &uri).await;
            assert_eq!(r.status, 200, "{param}{size}: {}", r.text());
            grids.push((r.bbox(), r.header("x-edr-width"), r.header("x-edr-height")));
        }
        assert_eq!(grids[0], grids[1], "MESH and PRECIP_FLAG{size}");

        // and it is the native grid's: the polygon within a few 0.01 degree cells (a level 3 cell
        // is 0.08 degrees)
        let want = [polygon.0, polygon.1, polygon.2, polygon.3];
        for (got, want) in grids[0].0.iter().zip(want) {
            assert!((got - want).abs() < 0.05, "{:?}{size}", grids[0].0);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Instances and cache lifetime
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn an_instance_is_read_at_its_run_and_datetime_and_cached_for_what_it_matched() {
    let (app, state, _infra) = setup().await;
    let a = "/edr/collections/hrrr-subh-atmosphere";
    let when = |t: &str| format!("datetime=2026-10-11T{t}Z");
    let (lon, lat, at_18_00) = SNOW;
    let box_ = around(lon, lat, (0.2, 0.12));
    let q = |t: &str| {
        format!(
            "coords={box_}&parameter-name=REFC&f=png&width=33&height=21&crop=true&{}",
            when(t)
        )
    };
    let centre = |r: &Reply| r.decode().at(16, 10).expect("data at the cell");
    // REFC at the cell for 18:15, from hrrr_subhourly.rs
    let at_18_15 = 25.69;

    // A second run that also holds 18:00, with different data: the 17Z run's 1 h forecast, which
    // is the 18Z run's 18:15 data. Asking for the 17Z instance at 18:00 must give THAT, not the
    // newest run's 18:00 (26.19).
    let catalog = &state.catalog;
    let t = |s: &str| -> chrono::DateTime<chrono::Utc> { s.parse().unwrap() };
    let mut entry = catalog
        .find_datasets_in_valid_time_range(
            "hrrr-subh",
            "REFC",
            None,
            t("2026-10-11T18:15:00Z"),
            t("2026-10-11T18:15:00Z"),
        )
        .await
        .unwrap()
        .remove(0)
        .1;
    entry.reference_time = t("2026-10-11T17:00:00Z");
    entry.forecast_hour = 1;
    entry.forecast_minute = 0;
    assert_eq!(entry.valid_time(), t("2026-10-11T18:00:00Z"));
    catalog.register_dataset(&entry).await.unwrap();

    let newest = get_reply(&app, &format!("{a}/area?{}", q("18:00:00"))).await;
    assert!(
        (centre(&newest) - at_18_00).abs() < 0.02,
        "newest run at 18:00"
    );

    let older = get_reply(
        &app,
        &format!("{a}/instances/2026-10-11T17:00:00Z/area?{}", q("18:00:00")),
    )
    .await;
    assert_eq!(older.status, 200, "{}", older.text());
    assert!(
        (centre(&older) - at_18_15).abs() < 0.02,
        "the 17Z instance at 18:00 is its own data, not the newest run's: {}",
        centre(&older)
    );

    let own = get_reply(
        &app,
        &format!("{a}/instances/2026-10-11T18:00:00Z/area?{}", q("18:00:00")),
    )
    .await;
    assert!((centre(&own) - at_18_00).abs() < 0.02);

    // ...honouring the datetime: the 18Z run at 18:15 and 18:30
    for (time, want) in [("18:15:00", at_18_15), ("18:30:00", 24.56)] {
        let r = get_reply(
            &app,
            &format!("{a}/instances/2026-10-11T18:00:00Z/area?{}", q(time)),
        )
        .await;
        assert!((centre(&r) - want).abs() < 0.02, "{time}: {}", centre(&r));
    }

    // a time the instance does not hold is not found, not another run's data
    let none = get_reply(
        &app,
        &format!("{a}/instances/2026-10-11T17:00:00Z/area?{}", q("18:30:00")),
    )
    .await;
    assert_eq!(none.status, 404, "{}", none.text());
    // ...and neither is a run that does not exist (as before)
    let none = get_reply(
        &app,
        &format!("{a}/instances/2026-10-11T03:00:00Z/area?{}", q("18:00:00")),
    )
    .await;
    assert_eq!(none.status, 404);

    // with no datetime the instance's first forecast
    let first = get_reply(
        &app,
        &format!(
            "{a}/instances/2026-10-11T18:00:00Z/area?coords={box_}&parameter-name=REFC&f=png&width=33&height=21&crop=true"
        ),
    )
    .await;
    assert!((centre(&first) - at_18_00).abs() < 0.02);

    // ...which is the run's earliest step, not "hour 0": the 17Z run has no f000 (it starts at its
    // 1 hour forecast, valid 18:00), and the same URL used to be a 404
    let first_of_older = get_reply(
        &app,
        &format!(
            "{a}/instances/2026-10-11T17:00:00Z/area?coords={box_}&parameter-name=REFC&f=png&width=33&height=21&crop=true"
        ),
    )
    .await;
    assert_eq!(first_of_older.status, 200, "{}", first_of_older.text());
    assert!((centre(&first_of_older) - at_18_15).abs() < 0.02);
    assert_eq!(first_of_older.header("cache-control"), "max-age=900");

    // An instance is a fixed run and time: the model's explicit lifetime (900 s for hrrr-subh)
    assert_eq!(older.header("cache-control"), "max-age=900");
    assert_eq!(own.header("cache-control"), "max-age=900");

    // A span of time is read at the first time THE RUN holds. Give the 17Z run an earlier step
    // (17:45) than any the 18Z run has (18:00 to 18:30): the model's earliest instant in these
    // spans is now one the 18Z instance does not hold, which used to make each a 404.
    entry.forecast_hour = 0;
    entry.forecast_minute = 45;
    assert_eq!(entry.valid_time(), t("2026-10-11T17:45:00Z"));
    catalog.register_dataset(&entry).await.unwrap();

    let span = |datetime: &str| {
        format!(
            "coords={box_}&parameter-name=REFC&f=png&width=33&height=21&crop=true&datetime={datetime}"
        )
    };
    let in_18z = format!("{a}/instances/2026-10-11T18:00:00Z/area");
    for (name, datetime, want) in [
        (
            "an interval that starts before the run",
            "2026-10-11T17:30:00Z/2026-10-11T18:30:00Z",
            at_18_00,
        ),
        ("an open start", "../2026-10-11T18:30:00Z", at_18_00),
        (
            "an interval inside the run",
            "2026-10-11T18:10:00Z/2026-10-11T18:40:00Z",
            at_18_15,
        ),
        (
            "a list that starts with a time the run does not hold",
            "2026-10-11T17:45:00Z,2026-10-11T18:00:00Z",
            at_18_00,
        ),
    ] {
        let r = get_reply(&app, &format!("{in_18z}?{}", span(datetime))).await;
        assert_eq!(r.status, 200, "{name}: {}", r.text());
        assert!(
            (centre(&r) - want).abs() < 0.02,
            "{name}: {} dBZ, wanted {want}",
            centre(&r)
        );
        assert_eq!(r.header("cache-control"), "max-age=900", "{name}");
    }

    // ...and the same span of the 17Z run is its own 17:45 (the 18:15 data, as registered above),
    // not the 18Z run's 18:00
    let r = get_reply(
        &app,
        &format!(
            "{a}/instances/2026-10-11T17:00:00Z/area?{}",
            span("2026-10-11T17:30:00Z/2026-10-11T18:30:00Z")
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(
        (centre(&r) - at_18_15).abs() < 0.02,
        "the 17Z run's 17:45 step: {}",
        centre(&r)
    );

    // CoverageJSON gets the same: 200, with the run's times on its time axis
    let r = get_reply(
        &app,
        &format!(
            "{in_18z}?coords={box_}&parameter-name=REFC&datetime=2026-10-11T17:30:00Z/2026-10-11T18:30:00Z"
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(
        body["domain"]["axes"]["t"]["values"],
        serde_json::json!([
            "2026-10-11T18:00:00Z",
            "2026-10-11T18:15:00Z",
            "2026-10-11T18:30:00Z"
        ])
    );

    // A span with nothing of the run in it is a time the run does not hold: a 404 like a single
    // instant, not the run's first forecast outside the span (and not a 900 s lifetime for it)
    for datetime in [
        "2030-01-01T00:00:00Z/2030-01-02T00:00:00Z",
        "2026-10-11T17:30:00Z,2026-10-11T17:45:00Z",
        "../2026-10-11T17:30:00Z",
        "2030-01-01T00:00:00Z/..",
    ] {
        let r = get_reply(&app, &format!("{in_18z}?{}", span(datetime))).await;
        assert_eq!(r.status, 404, "{datetime}: {}", r.text());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn cache_control_is_explicit_only_for_the_dataset_that_was_asked_for() {
    let (app, _state, _infra) = setup().await;
    let cc = |r: &Reply| r.header("cache-control");

    // hrrr-subh: png_max_age 900. 18:15 exists; 18:20 is answered with the 18:15 frame
    let sub = |datetime: &str| {
        format!(
            "/edr/collections/hrrr-subh-atmosphere/area?coords={}&parameter-name=REFC&f=png&width=40&height=30{datetime}",
            app_polygon()
        )
    };
    let exact = get_reply(&app, &sub("&datetime=2026-10-11T18:15:00Z")).await;
    assert_eq!(exact.status, 200);
    assert_eq!(cc(&exact), "max-age=900", "the frame for that instant");

    let near = get_reply(&app, &sub("&datetime=2026-10-11T18:20:00Z")).await;
    assert_eq!(near.status, 200);
    assert_eq!(cc(&near), "max-age=60", "5 minutes from the frame it got");
    assert_eq!(near.body, exact.body, "it is the 18:15 frame");

    let future = get_reply(&app, &sub("&datetime=2030-01-01T00:00:00Z")).await;
    assert_eq!(future.status, 200);
    assert_eq!(
        cc(&future),
        "max-age=60",
        "a future instant is not a fixed answer"
    );

    let expired = get_reply(&app, &sub("&datetime=2020-01-01T00:00:00Z")).await;
    assert_eq!(cc(&expired), "max-age=60");

    let newest = get_reply(&app, &sub("")).await;
    assert_eq!(cc(&newest), "max-age=60");

    // mrms: 300 s by default. The scan is at 23:00:41
    let mrms = |datetime: &str| {
        format!(
            "/edr/collections/mrms-single-level/area?coords={}&parameter-name=MESH&f=png&width=60&height=30{datetime}",
            rect(-100.0, 35.0, -94.0, 38.0)
        )
    };
    assert_eq!(
        cc(&get_reply(&app, &mrms("&datetime=2026-10-09T23:00:41Z")).await),
        "max-age=300"
    );
    assert_eq!(
        cc(&get_reply(&app, &mrms("&datetime=2026-10-09T23:00:00Z")).await),
        "max-age=60"
    );
    assert_eq!(
        cc(&get_reply(&app, &mrms("&datetime=2026-10-10T23:00:00Z")).await),
        "max-age=60"
    );
    assert_eq!(cc(&get_reply(&app, &mrms("")).await), "max-age=60");
}

// ---------------------------------------------------------------------------------------------
// Instances of the other query types (cube needs numeric vertical levels, which no fixture has)
// ---------------------------------------------------------------------------------------------

/// Every number in `ranges.REFC.values` of a CoverageJSON body, nested arrays flattened.
fn covjson_values(reply: &Reply) -> Vec<f64> {
    assert_eq!(reply.status, 200, "{}", reply.text());
    let body: serde_json::Value = serde_json::from_slice(&reply.body).expect("CoverageJSON");
    let mut out = vec![];
    let mut stack = vec![&body["ranges"]["REFC"]["values"]];
    while let Some(v) = stack.pop() {
        match v {
            serde_json::Value::Array(a) => stack.extend(a.iter().rev()),
            serde_json::Value::Number(n) => out.push(n.as_f64().unwrap()),
            _ => {}
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn an_instance_position_and_radius_are_read_at_the_requested_time() {
    let (app, _state, _infra) = setup().await;
    let a = "/edr/collections/hrrr-subh-atmosphere/instances/2026-10-11T18:00:00Z";
    let (lon, lat, at_18_00) = SNOW;
    // REFC at the cell for 18:15 and 18:30, from hrrr_subhourly.rs
    let at_18_15 = 25.69;
    let at_18_30 = 24.56;

    // The run's first forecast is 18:00. Before the fix each of these answered with it whatever the
    // datetime, because the valid time was replaced by the run.
    let mut radius_by_time = vec![];
    for (time, want) in [
        ("18:00:00", at_18_00),
        ("18:15:00", at_18_15),
        ("18:30:00", at_18_30),
    ] {
        let datetime = format!("datetime=2026-10-11T{time}Z");
        let position = get_reply(
            &app,
            &format!("{a}/position?coords=POINT({lon}%20{lat})&parameter-name=REFC&{datetime}"),
        )
        .await;
        let values = covjson_values(&position);
        assert_eq!(values.len(), 1, "{time} position: {values:?}");
        assert!(
            (values[0] - want).abs() < 0.02,
            "position at {time}: {} dBZ, wanted {want}",
            values[0]
        );

        // A radius reads the cells within it: the instance answers with what the same run gives
        // without naming it (the newest run is this one), and that differs from time to time.
        let radius_uri = |prefix: &str| {
            format!(
                "{prefix}/radius?coords=POINT({lon}%20{lat})&within=60&within-units=km&parameter-name=REFC&{datetime}"
            )
        };
        let instance = covjson_values(&get_reply(&app, &radius_uri(a)).await);
        let newest = covjson_values(
            &get_reply(&app, &radius_uri("/edr/collections/hrrr-subh-atmosphere")).await,
        );
        assert!(!instance.is_empty(), "radius at {time}");
        assert_eq!(instance, newest, "radius at {time}");
        radius_by_time.push(instance);
    }
    assert!(
        radius_by_time[0] != radius_by_time[1] && radius_by_time[1] != radius_by_time[2],
        "a radius is read at each requested time, not at the run's first hour"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires Docker
async fn the_png_says_which_valid_and_run_time_it_is() {
    let (app, _state, _infra) = setup().await;
    let valid = |r: &Reply| r.header("x-edr-valid-time");
    let run = |r: &Reply| r.header("x-edr-reference-time");

    // hrrr-subh (a forecast): its steps are 15 minutes, which `forecast_hour` alone cannot say
    let a = "/edr/collections/hrrr-subh-atmosphere";
    let sub = |datetime: &str| {
        format!(
            "{a}/area?coords={}&parameter-name=REFC&f=png&width=40&height=30{datetime}",
            app_polygon()
        )
    };
    let exact = get_reply(&app, &sub("&datetime=2026-10-11T18:15:00Z")).await;
    assert_eq!(exact.status, 200, "{}", exact.text());
    assert_eq!(valid(&exact), "2026-10-11T18:15:00Z");
    assert_eq!(run(&exact), "2026-10-11T18:00:00Z");

    // a near miss is the frame it got, not the time asked for
    let near = get_reply(&app, &sub("&datetime=2026-10-11T18:20:00Z")).await;
    assert_eq!(valid(&near), "2026-10-11T18:15:00Z");
    assert_eq!(run(&near), "2026-10-11T18:00:00Z");

    // no datetime: the newest dataset
    let newest = get_reply(&app, &sub("")).await;
    assert_eq!(newest.status, 200, "{}", newest.text());
    assert_eq!(valid(&newest), "2026-10-11T18:45:00Z");
    assert_eq!(run(&newest), "2026-10-11T18:00:00Z");

    // an instance: that run's reference time, and its own step
    let inst = get_reply(
        &app,
        &format!(
            "{a}/instances/2026-10-11T18:00:00Z/area?coords={}&parameter-name=REFC&f=png&width=40&height=30&datetime=2026-10-11T18:30:00Z",
            app_polygon()
        ),
    )
    .await;
    assert_eq!(inst.status, 200, "{}", inst.text());
    assert_eq!(valid(&inst), "2026-10-11T18:30:00Z");
    assert_eq!(run(&inst), "2026-10-11T18:00:00Z");

    // mrms (an observation): the scan's own second; no run time, it is the scan time
    let mrms = |datetime: &str| {
        format!(
            "/edr/collections/mrms-single-level/area?coords={}&parameter-name=MESH&f=png&width=60&height=30{datetime}",
            rect(-100.0, 35.0, -94.0, 38.0)
        )
    };
    for datetime in [
        "&datetime=2026-10-09T23:00:41Z",
        "&datetime=2026-10-09T23:00:00Z",
        "",
    ] {
        let r = get_reply(&app, &mrms(datetime)).await;
        assert_eq!(r.status, 200, "{}", r.text());
        assert_eq!(valid(&r), "2026-10-09T23:00:41Z", "{datetime}");
        assert!(
            r.headers.get("x-edr-reference-time").is_none(),
            "{datetime}"
        );
    }
}
