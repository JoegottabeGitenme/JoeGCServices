//! GLM lightning ingest: turns one uploaded GLM L2 LCFA granule into stored
//! flashes. Everything here is database-free so it can be unit tested; the HTTP
//! handler in `server.rs` and the retention loop below do the I/O.

use std::time::Duration;

use chrono::{DateTime, Utc};
use netcdf_parser::glm::{read_glm_flashes_from_bytes, GlmGranule};
use storage::{LightningCatalog, NewFlash};
use tracing::{info, warn};

/// Lightning is kept for the CONUS box only. Longitude/latitude, degrees.
/// Mirrored (as documentation) by `conditions`/extent in `config/edr/glm.yaml`.
pub const CONUS_MIN_LON: f64 = -125.0;
pub const CONUS_MIN_LAT: f64 = 24.0;
pub const CONUS_MAX_LON: f64 = -66.0;
pub const CONUS_MAX_LAT: f64 = 50.0;

/// Default retention if `LIGHTNING_RETENTION_HOURS` is unset.
pub const DEFAULT_RETENTION_HOURS: u64 = 24;

/// How often the retention loop deletes expired flashes.
const RETENTION_INTERVAL: Duration = Duration::from_secs(600);

/// The role a spacecraft currently fills. The API speaks in roles because the
/// East/West slots are re-assigned over time (G16 -> G19 in 2025), while the
/// platform id is what the file says.
pub fn satellite_role(platform: &str) -> Option<&'static str> {
    match platform {
        "G16" | "G19" => Some("goes-east"),
        "G17" | "G18" => Some("goes-west"),
        _ => None,
    }
}

/// What processing a granule produced.
#[derive(Debug)]
pub struct ProcessedGranule {
    pub satellite: &'static str,
    pub platform: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    /// Flashes decoded from the file (before clipping).
    pub flashes_in_file: usize,
    /// Dropped for an unusable position.
    pub skipped_invalid: usize,
    /// Kept after the CONUS clip.
    pub flashes: Vec<NewFlash>,
}

#[derive(Debug, thiserror::Error)]
pub enum LightningError {
    /// The upload is not a usable GLM granule. Retrying will not help.
    #[error("not a usable GLM granule: {0}")]
    BadGranule(String),
}

/// Parse and clip one granule. CPU-bound (libnetcdf): call from `spawn_blocking`.
pub fn process_granule(data: &[u8]) -> Result<ProcessedGranule, LightningError> {
    let mut granule: GlmGranule =
        read_glm_flashes_from_bytes(data).map_err(|e| LightningError::BadGranule(e.to_string()))?;

    let satellite = satellite_role(&granule.platform).ok_or_else(|| {
        LightningError::BadGranule(format!(
            "unknown GLM platform {:?} (expected G16/G17/G18/G19)",
            granule.platform
        ))
    })?;

    // The window end is the freshness marker. Fall back to start + 20 s (the
    // product's fixed length) for the rare file without product_time_bounds.
    let window_end = granule
        .window_end
        .unwrap_or_else(|| granule.window_start + chrono::Duration::seconds(20));

    let flashes_in_file = granule.flashes.len() + granule.skipped_invalid;
    granule.retain_in_bbox(CONUS_MIN_LON, CONUS_MIN_LAT, CONUS_MAX_LON, CONUS_MAX_LAT);

    let flashes = granule
        .flashes
        .iter()
        .map(|f| NewFlash {
            satellite: satellite.to_string(),
            flash_time: f.time,
            flash_id: i32::from(f.flash_id),
            lon: f.lon,
            lat: f.lat,
            energy_j: f.energy_j.map(|e| e as f32),
            quality: i16::from(f.quality),
        })
        .collect();

    Ok(ProcessedGranule {
        satellite,
        platform: granule.platform,
        window_start: granule.window_start,
        window_end,
        flashes_in_file,
        skipped_invalid: granule.skipped_invalid,
        flashes,
    })
}

/// Retention horizon from `LIGHTNING_RETENTION_HOURS` (default 24). A value that
/// is not a positive integer falls back to the default *and says so* rather than
/// silently deleting everything (0) or nothing.
pub fn retention_hours_from_env() -> u64 {
    match std::env::var("LIGHTNING_RETENTION_HOURS") {
        Err(_) => DEFAULT_RETENTION_HOURS,
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(h) if h > 0 => h,
            _ => {
                warn!(value = %v, default = DEFAULT_RETENTION_HOURS, "Invalid LIGHTNING_RETENTION_HOURS; using default");
                DEFAULT_RETENTION_HOURS
            }
        },
    }
}

/// Delete expired flashes forever, every [`RETENTION_INTERVAL`]. The first sweep
/// runs immediately so a restart after downtime cleans up straight away.
pub async fn run_retention(catalog: LightningCatalog, retention_hours: u64) {
    let mut tick = tokio::time::interval(RETENTION_INTERVAL);
    loop {
        tick.tick().await;
        let cutoff = Utc::now() - chrono::Duration::hours(retention_hours as i64);
        match catalog.delete_before(cutoff).await {
            Ok(0) => {}
            Ok(n) => info!(deleted = n, %cutoff, "Lightning retention sweep"),
            Err(e) => warn!(error = %e, "Lightning retention sweep failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // Real granules, shared with the parser crate's tests.
    const G18: &[u8] = include_bytes!(
        "../../../crates/netcdf-parser/tests/fixtures/OR_GLM-L2-LCFA_G18_s20262802033000_e20262802033200_c20262802033222.nc"
    );
    const G19: &[u8] = include_bytes!(
        "../../../crates/netcdf-parser/tests/fixtures/OR_GLM-L2-LCFA_G19_s20262802033000_e20262802033200_c20262802033219.nc"
    );
    const EMPTY: &[u8] =
        include_bytes!("../../../crates/netcdf-parser/tests/fixtures/glm_empty_no_flashes.nc");

    #[test]
    fn platform_to_role_mapping() {
        assert_eq!(satellite_role("G19"), Some("goes-east"));
        assert_eq!(satellite_role("G16"), Some("goes-east"));
        assert_eq!(satellite_role("G18"), Some("goes-west"));
        assert_eq!(satellite_role("G17"), Some("goes-west"));
        assert_eq!(
            satellite_role("G20"),
            None,
            "a new spacecraft must be rejected, not guessed"
        );
        assert_eq!(satellite_role(""), None);
    }

    #[test]
    fn goes19_is_clipped_to_conus_with_reference_counts() {
        let p = process_granule(G19).unwrap();
        assert_eq!(p.satellite, "goes-east");
        assert_eq!(p.platform, "G19");
        assert_eq!(p.flashes_in_file, 899);
        assert_eq!(
            p.flashes.len(),
            21,
            "reference: 21 of 899 are inside the CONUS box"
        );
        assert_eq!(p.skipped_invalid, 0);
        assert!(p.flashes.iter().all(|f| f.satellite == "goes-east"));
        assert!(p.flashes.iter().all(|f| {
            f.lon >= CONUS_MIN_LON
                && f.lon <= CONUS_MAX_LON
                && f.lat >= CONUS_MIN_LAT
                && f.lat <= CONUS_MAX_LAT
        }));
    }

    #[test]
    fn goes18_maps_to_goes_west() {
        let p = process_granule(G18).unwrap();
        assert_eq!(p.satellite, "goes-west");
        assert_eq!((p.flashes_in_file, p.flashes.len()), (57, 9));
    }

    #[test]
    fn window_end_is_start_plus_20_seconds() {
        let p = process_granule(G19).unwrap();
        assert_eq!(
            p.window_start,
            Utc.with_ymd_and_hms(2026, 10, 7, 20, 33, 0).unwrap()
        );
        assert_eq!(
            p.window_end,
            Utc.with_ymd_and_hms(2026, 10, 7, 20, 33, 20).unwrap()
        );
    }

    #[test]
    fn converted_values_survive_the_type_narrowing() {
        let p = process_granule(G19).unwrap();
        for f in &p.flashes {
            assert!(
                (0..=65535).contains(&f.flash_id),
                "flash_id {} is a u16 counter",
                f.flash_id
            );
            assert!(
                f.flash_id > 32_767,
                "unsigned decoding must reach the DB intact"
            );
            assert!(matches!(f.quality, 0 | 1 | 3 | 5));
            let e = f.energy_j.expect("energy present in this granule");
            assert!(
                e > 1e-16 && e < 1e-9,
                "energy {e} J out of the plausible range"
            );
        }
    }

    #[test]
    fn an_empty_granule_is_valid_and_still_reports_its_window() {
        // The monitoring marker must advance even when the sky is quiet.
        let p = process_granule(EMPTY).unwrap();
        assert!(p.flashes.is_empty());
        assert_eq!(p.flashes_in_file, 0);
        assert_eq!(
            p.window_end,
            Utc.with_ymd_and_hms(2026, 10, 7, 20, 35, 20).unwrap()
        );
    }

    #[test]
    fn garbage_is_a_bad_granule_not_a_panic() {
        assert!(matches!(
            process_granule(b"definitely not netcdf"),
            Err(LightningError::BadGranule(_))
        ));
        assert!(matches!(
            process_granule(&[]),
            Err(LightningError::BadGranule(_))
        ));
    }

    #[test]
    fn retention_env_parsing() {
        // Serialized in one test: process env is global.
        std::env::remove_var("LIGHTNING_RETENTION_HOURS");
        assert_eq!(retention_hours_from_env(), 24);
        std::env::set_var("LIGHTNING_RETENTION_HOURS", "72");
        assert_eq!(retention_hours_from_env(), 72);
        for bad in ["0", "-5", "abc", "", "1.5"] {
            std::env::set_var("LIGHTNING_RETENTION_HOURS", bad);
            assert_eq!(
                retention_hours_from_env(),
                24,
                "{bad:?} must fall back, never mean 'delete everything'"
            );
        }
        std::env::remove_var("LIGHTNING_RETENTION_HOURS");
    }
}

/// End-to-end tests of `POST /ingest/lightning` against a real PostGIS: the
/// router, the body-limit layer, parsing, clipping, the DB write, and the
/// progress marker -- the whole path except the downloader.
///
/// `#[ignore]`d (CI runs them with Docker). Locally:
/// `LIGHTNING_TEST_DATABASE_URL=postgres://postgres:t@localhost:15434/postgres \
///    cargo test -p ingester -- --ignored http_tests`
#[cfg(test)]
mod http_tests {
    use super::*;
    use crate::server::{build_router, IngestionTracker, LightningIngestResponse, ServerState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::TimeZone;
    use ingestion::Ingester;
    use std::sync::Arc;
    use storage::{Catalog, FlashArea, FlashQuery, ObjectStorage, ObjectStorageConfig};
    use tower::ServiceExt;

    const G18: &[u8] = include_bytes!(
        "../../../crates/netcdf-parser/tests/fixtures/OR_GLM-L2-LCFA_G18_s20262802033000_e20262802033200_c20262802033222.nc"
    );
    const G19: &[u8] = include_bytes!(
        "../../../crates/netcdf-parser/tests/fixtures/OR_GLM-L2-LCFA_G19_s20262802033000_e20262802033200_c20262802033219.nc"
    );
    const EMPTY: &[u8] =
        include_bytes!("../../../crates/netcdf-parser/tests/fixtures/glm_empty_no_flashes.nc");

    struct Harness {
        router: axum::Router,
        lightning: LightningCatalog,
        _infra: Option<test_utils::containers::TestInfrastructure>,
    }

    async fn harness() -> Harness {
        let (url, infra) = match std::env::var("LIGHTNING_TEST_DATABASE_URL") {
            Ok(u) => (u, None),
            Err(_) => {
                let infra = test_utils::containers::TestInfrastructure::start().await;
                (infra.postgres_url(), Some(infra))
            }
        };
        let catalog = Catalog::connect(&url).await.unwrap();
        catalog.migrate().await.unwrap();
        catalog.migrate_observations().await.unwrap();
        catalog.migrate_lightning().await.unwrap();
        sqlx_truncate(&catalog).await;

        // Never contacted by the lightning route.
        let storage = Arc::new(
            ObjectStorage::new(&ObjectStorageConfig {
                endpoint: "http://127.0.0.1:1".to_string(),
                bucket: "unused".to_string(),
                access_key_id: "x".to_string(),
                secret_access_key: "x".to_string(),
                region: "us-east-1".to_string(),
                allow_http: true,
            })
            .unwrap(),
        );
        let lightning = LightningCatalog::new(catalog.pool_clone());
        let state = Arc::new(ServerState {
            ingester: Ingester::new(storage, catalog.clone()),
            observation_catalog: None,
            storm_event_catalog: None,
            trail_report_catalog: None,
            lightning_catalog: Some(lightning.clone()),
            tracker: IngestionTracker::new(),
        });
        Harness {
            router: build_router(state),
            lightning,
            _infra: infra,
        }
    }

    async fn sqlx_truncate(catalog: &Catalog) {
        sqlx::query("TRUNCATE lightning_flashes, lightning_ingest_progress RESTART IDENTITY")
            .execute(catalog.pool())
            .await
            .unwrap();
    }

    async fn post(h: &Harness, body: Vec<u8>) -> (StatusCode, LightningIngestResponse2) {
        let resp = h
            .router
            .clone()
            .oneshot(
                Request::post("/ingest/lightning?source_url=test")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// The response, read back as JSON (the server type is `Serialize` only).
    #[derive(serde::Deserialize, Debug)]
    struct LightningIngestResponse2 {
        success: bool,
        satellite: Option<String>,
        flashes_in_file: usize,
        flashes_in_conus: usize,
        flashes_inserted: usize,
        latency_secs: Option<f64>,
    }

    fn all() -> FlashQuery {
        FlashQuery {
            satellites: vec![],
            since: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            until: None,
            after_id: None,
            area: FlashArea::Anywhere,
            limit: 10_000,
        }
    }

    // LightningIngestResponse is imported only to prove the server type still exists.
    #[allow(dead_code)]
    fn _type_check(_: LightningIngestResponse) {}

    #[tokio::test]
    #[ignore]
    async fn a_real_granule_lands_in_the_database_with_unsigned_ids_intact() {
        let h = harness().await;
        let (status, r) = post(&h, G19.to_vec()).await;
        assert_eq!(status, StatusCode::OK, "{r:?}");
        assert!(r.success);
        assert_eq!(r.satellite.as_deref(), Some("goes-east"));
        assert_eq!(
            (r.flashes_in_file, r.flashes_in_conus, r.flashes_inserted),
            (899, 21, 21)
        );
        assert!(r.latency_secs.is_some());

        let rows = h.lightning.query_flashes(&all()).await.unwrap();
        assert_eq!(rows.len(), 21);
        assert!(rows.iter().all(|f| f.satellite == "goes-east"));
        // Every flash_id in this granule is > 32,767: they only reach the DB
        // intact if the unsigned decoding survived the whole path.
        assert!(
            rows.iter().all(|f| f.flash_id > 32_767),
            "{:?}",
            rows.iter().map(|f| f.flash_id).collect::<Vec<_>>()
        );
        assert!(rows
            .iter()
            .all(|f| f.lon >= -125.0 && f.lon <= -66.0 && f.lat >= 24.0 && f.lat <= 50.0));
        assert!(rows.iter().all(|f| f.energy_j.is_some()));

        let progress = h.lightning.ingest_progress().await.unwrap();
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0].0, "goes-east");
        assert_eq!(
            progress[0].1,
            Utc.with_ymd_and_hms(2026, 10, 7, 20, 33, 20).unwrap()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn resending_a_granule_inserts_nothing() {
        let h = harness().await;
        assert_eq!(post(&h, G19.to_vec()).await.1.flashes_inserted, 21);
        let (status, r) = post(&h, G19.to_vec()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (r.flashes_in_conus, r.flashes_inserted),
            (21, 0),
            "re-ingest must be a no-op"
        );
        assert_eq!(h.lightning.count().await.unwrap(), 21);
    }

    #[tokio::test]
    #[ignore]
    async fn both_satellites_are_stored_under_their_roles() {
        let h = harness().await;
        post(&h, G19.to_vec()).await;
        let (_, w) = post(&h, G18.to_vec()).await;
        assert_eq!(
            (w.satellite.as_deref(), w.flashes_inserted),
            (Some("goes-west"), 9)
        );
        let rows = h.lightning.query_flashes(&all()).await.unwrap();
        assert_eq!(
            rows.iter().filter(|f| f.satellite == "goes-east").count(),
            21
        );
        assert_eq!(
            rows.iter().filter(|f| f.satellite == "goes-west").count(),
            9
        );
        assert_eq!(h.lightning.ingest_progress().await.unwrap().len(), 2);
    }

    #[tokio::test]
    #[ignore]
    async fn an_empty_granule_succeeds_and_still_advances_the_progress_marker() {
        // A quiet sky is normal; it must not look like an outage.
        let h = harness().await;
        let (status, r) = post(&h, EMPTY.to_vec()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((r.flashes_in_file, r.flashes_inserted), (0, 0));
        assert_eq!(h.lightning.count().await.unwrap(), 0);
        let progress = h.lightning.ingest_progress().await.unwrap();
        assert_eq!(progress[0].0, "goes-west");
        assert_eq!(
            progress[0].1,
            Utc.with_ymd_and_hms(2026, 10, 7, 20, 35, 20).unwrap()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn garbage_is_rejected_with_422_writes_nothing_and_does_not_poison_later_uploads() {
        let h = harness().await;
        let (status, r) = post(&h, b"this is not a netcdf file".to_vec()).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "422 tells the downloader not to retry"
        );
        assert!(!r.success);
        assert_eq!(h.lightning.count().await.unwrap(), 0);
        assert!(h.lightning.ingest_progress().await.unwrap().is_empty());

        let (status, r) = post(&h, G19.to_vec()).await;
        assert_eq!(
            (status, r.flashes_inserted),
            (StatusCode::OK, 21),
            "a good upload after a bad one still works"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn bodies_over_axums_2mb_default_are_not_refused_by_the_transport() {
        // 3 MB of junk must reach the handler (-> 422 "not a granule"), not be cut
        // off by axum's default body limit (-> 413). Proves the raised limit is wired.
        let h = harness().await;
        let (status, _) = post(&h, vec![7u8; 3 * 1024 * 1024]).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
