//! Periodic OSM/Overpass sync for the EDR `trails` feature collection.
//!
//! Design goal (from the trail-conditions design session): a new trail
//! opening should appear in the app without a code change or redeploy. This
//! crate is the mechanism — it polls Overpass for configured regions on a
//! schedule, upserts trail/track/bridleway ways into `linear_features` and
//! trailhead nodes into the shared `locations` registry, and soft-deletes
//! anything that drops out of a region's latest pass.
//!
//! Mirrors the `retention` crate's shape deliberately: [`run_once`] is the
//! shared logic called both by [`TrailSyncTask::run_forever`] (the scheduled
//! loop, spawned in the ingester) and directly by an admin "refresh now"
//! endpoint (in wms-api) — same pattern as `CleanupTask`/`SyncTask` versus
//! the admin cleanup-status endpoints.

mod overpass;

pub use overpass::{subdivide_bbox, BBox, OsmTrailhead, OsmWay, OverpassResult};

use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

use storage::linear_features::{LinearFeature, LinearFeatureCatalog};
use storage::observations::{Location, ObservationCatalog};

/// One sync region, loaded from `config/trail-sync.yaml`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SyncRegion {
    /// Stable key for this region (stored in `linear_features.region`, used
    /// to scope the soft-delete pass so one region's sync never marks
    /// another region's features inactive).
    pub name: String,
    /// (min_lon, min_lat, max_lon, max_lat).
    pub bbox: (f64, f64, f64, f64),
    /// Fetch tile size in degrees. A single statewide Overpass request
    /// returns ~300 MB of JSON for Colorado, which is enough to crash the
    /// ingester on memory -- confirmed live during this crate's first
    /// deploy. Default 1.0 degree keeps each fetch small regardless of
    /// region size; override per-region if a smaller/denser area needs
    /// finer or coarser tiling.
    #[serde(default = "default_tile_deg")]
    pub tile_deg: f64,
}

fn default_tile_deg() -> f64 {
    1.0
}

/// Whether a region's `seen_ids` is complete enough to safely run the
/// soft-delete pass. Extracted as its own function specifically so this
/// safety condition has a name and a unit test, after a prior version
/// (`tile_errors < tiles.len()`) let a partial failure through and wrongly
/// deactivated ~44,500 real features -- see the call site's comment.
fn safe_to_soft_delete(tile_errors: usize) -> bool {
    tile_errors == 0
}

/// Trail sync configuration, re-read from disk at the start of every cycle
/// (see [`TrailSyncConfig::load`]) so adding a region takes effect on the
/// next scheduled run or admin-triggered refresh — no restart required.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TrailSyncConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_overpass_url")]
    pub overpass_url: String,
    #[serde(default)]
    pub regions: Vec<SyncRegion>,
}

fn default_enabled() -> bool {
    true
}
fn default_interval_secs() -> u64 {
    604_800 // weekly
}
fn default_overpass_url() -> String {
    "https://overpass-api.de/api/interpreter".to_string()
}

impl Default for TrailSyncConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            interval_secs: default_interval_secs(),
            overpass_url: default_overpass_url(),
            regions: Vec::new(),
        }
    }
}

impl TrailSyncConfig {
    /// Load from a YAML file. Returns the default (disabled-by-absence, no
    /// regions) config if the file does not exist, so a fresh checkout
    /// without the config file doesn't crash the ingester.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            warn!(path = %path.display(), "Trail sync config not found, sync disabled");
            return Ok(Self {
                enabled: false,
                ..Default::default()
            });
        }
        let contents = std::fs::read_to_string(path)?;
        let config: TrailSyncConfig = serde_yaml::from_str(&contents)?;
        Ok(config)
    }

    /// Resolve the config path from `CONFIG_DIR` (matching every other
    /// config-loading convention in this codebase) or `config/` as a
    /// fallback for local runs.
    pub fn load_from_config_dir() -> anyhow::Result<Self> {
        let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "config".to_string());
        Self::load(&Path::new(&config_dir).join("trail-sync.yaml"))
    }
}

/// Summary of one sync pass, returned to both the scheduled loop (for
/// logging) and the admin trigger (as the HTTP response body).
#[derive(Debug, Default, serde::Serialize)]
pub struct SyncSummary {
    pub regions_synced: usize,
    pub ways_upserted: usize,
    pub ways_deactivated: u64,
    pub trailheads_upserted: usize,
    pub errors: Vec<String>,
}

/// Run one full sync pass across all configured regions.
///
/// Callable directly (admin "refresh now" trigger) or from the scheduled
/// loop. A failure fetching or parsing one region is recorded in
/// `errors` and does not stop the others.
pub async fn run_once(
    config: &TrailSyncConfig,
    linear_catalog: &LinearFeatureCatalog,
    observation_catalog: &ObservationCatalog,
) -> SyncSummary {
    let client = reqwest::Client::new();
    let mut summary = SyncSummary::default();

    for region in &config.regions {
        let region_bbox = BBox {
            min_lon: region.bbox.0,
            min_lat: region.bbox.1,
            max_lon: region.bbox.2,
            max_lat: region.bbox.3,
        };
        let tiles = subdivide_bbox(region_bbox, region.tile_deg);

        info!(
            region = %region.name,
            tiles = tiles.len(),
            tile_deg = region.tile_deg,
            "Starting trail sync for region"
        );

        // Accumulated across all tiles for the region. Deliberately small:
        // seen_ids is a Vec<i64> (~8 bytes/way) and trailhead_locations holds
        // full Location structs but trailheads are far sparser than ways --
        // neither approaches the memory cost of holding parsed way geometry
        // for a whole region at once, which is exactly what tiling avoids.
        let mut seen_ids: Vec<i64> = Vec::new();
        let mut trailhead_locations: Vec<Location> = Vec::new();
        let mut region_ways_upserted = 0usize;
        let mut tile_errors = 0usize;

        for (i, tile) in tiles.iter().enumerate() {
            let result = match overpass::fetch_region(&client, &config.overpass_url, *tile).await {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!(
                        "region {} tile {}/{}: Overpass fetch failed: {}",
                        region.name,
                        i + 1,
                        tiles.len(),
                        e
                    );
                    warn!("{}", msg);
                    summary.errors.push(msg);
                    tile_errors += 1;
                    continue;
                }
            };

            // Build and upsert this tile's features immediately, then drop
            // them -- this is the whole point of tiling. Never accumulate
            // `features`/`result` across tiles.
            let features: Vec<LinearFeature> = result
                .ways
                .iter()
                .map(|way| {
                    seen_ids.push(way.id);
                    LinearFeature {
                        feature_id: way.id,
                        feature_class: overpass::classify_way(&way.tags),
                        name: way
                            .tags
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        // Best-effort: OSM trail-system grouping generally lives
                        // on route *relations*, not way tags, and resolving
                        // relation membership needs a separate Overpass pass.
                        // `network` is the closest way-level tag and is often
                        // absent -- a known v1 limitation, not a bug (design doc).
                        system: way
                            .tags
                            .get("network")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        coordinates: way.coordinates.clone(),
                        tags: serde_json::Value::Object(way.tags.clone()),
                        region: region.name.clone(),
                    }
                })
                .collect();

            match linear_catalog.upsert_features(&features).await {
                Ok(n) => region_ways_upserted += n,
                Err(e) => {
                    let msg = format!(
                        "region {} tile {}/{}: way upsert failed: {}",
                        region.name,
                        i + 1,
                        tiles.len(),
                        e
                    );
                    warn!("{}", msg);
                    summary.errors.push(msg);
                }
            }

            trailhead_locations.extend(
                result
                    .trailheads
                    .iter()
                    .map(|th| trailhead_to_location(th, &region.name)),
            );

            // Be a considerate Overpass citizen: a courtesy pause between
            // tile requests, not just back-to-back hammering. Widened from
            // 500ms after a statewide pass got itself network-refused for
            // most of a run (see fetch_region's retry doc comment) -- this
            // is a weekly background job, an extra couple of minutes per
            // cycle costs nothing.
            if i + 1 < tiles.len() {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }

        summary.ways_upserted += region_ways_upserted;

        // Soft-delete ways no longer present in this region's pass. Skipped
        // if ANY tile errored, not just if every tile errored -- confirmed
        // live during the trail-conditions splash-page verification session:
        // a partial failure (34/40 tiles down) left `seen_ids` covering only
        // the surviving tiles, and the original `tile_errors < tiles.len()`
        // guard let the pass proceed anyway, wrongly marking ~44,500 real,
        // still-existing ways in the untouched tiles' area inactive. A
        // partial `seen_ids` is just as unsafe to soft-delete against as an
        // empty one -- only a fully successful pass has the complete
        // picture needed to know what's actually missing.
        if safe_to_soft_delete(tile_errors) {
            match linear_catalog
                .mark_inactive_except(&region.name, &seen_ids)
                .await
            {
                Ok(n) => {
                    if n > 0 {
                        info!(region = %region.name, count = n, "Marked trail ways inactive (missing from latest sync)");
                    }
                    summary.ways_deactivated += n;
                }
                Err(e) => {
                    let msg = format!("region {}: mark-inactive failed: {}", region.name, e);
                    warn!("{}", msg);
                    summary.errors.push(msg);
                }
            }
        } else {
            warn!(
                region = %region.name,
                tile_errors,
                total_tiles = tiles.len(),
                "One or more tiles failed, skipping soft-delete pass (seen_ids is incomplete, would wrongly deactivate real features in untouched tiles)"
            );
        }

        // Upsert trailheads into the shared locations registry.
        match observation_catalog
            .upsert_locations(&trailhead_locations)
            .await
        {
            Ok(n) => summary.trailheads_upserted += n,
            Err(e) => {
                let msg = format!("region {}: trailhead upsert failed: {}", region.name, e);
                warn!("{}", msg);
                summary.errors.push(msg);
            }
        }

        info!(
            region = %region.name,
            ways = region_ways_upserted,
            trailheads = trailhead_locations.len(),
            tile_errors,
            "Trail sync region complete"
        );

        summary.regions_synced += 1;
    }

    summary
}

/// Convert an OSM trailhead node into a `locations` row.
///
/// Id convention `TH<osm_node_id>` mirrors the `PP<GEOID>`/`ZIP<code>`
/// convention already used for populated places and ZIP codes.
fn trailhead_to_location(node: &OsmTrailhead, region: &str) -> Location {
    let name = node
        .tags
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("Trailhead {}", node.id));

    Location {
        id: format!("TH{}", node.id),
        name,
        description: None,
        lon: node.lon,
        lat: node.lat,
        elevation_m: None,
        location_type: Some("trailhead".to_string()),
        country: Some("US".to_string()),
        region: Some(region.to_string()),
        properties: serde_json::Value::Object(node.tags.clone()),
    }
}

/// Scheduled background task, spawned in the ingester alongside the
/// retention loops. Re-reads config at the top of every cycle.
pub struct TrailSyncTask;

impl TrailSyncTask {
    /// Run the sync loop forever. The first pass runs immediately (matching
    /// the retention tasks' `run_forever` convention) rather than waiting a
    /// full interval after startup.
    pub async fn run_forever(
        linear_catalog: LinearFeatureCatalog,
        observation_catalog: ObservationCatalog,
    ) {
        loop {
            let config = match TrailSyncConfig::load_from_config_dir() {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "Failed to load trail sync config, skipping cycle");
                    tokio::time::sleep(Duration::from_secs(default_interval_secs())).await;
                    continue;
                }
            };

            if !config.enabled || config.regions.is_empty() {
                info!("Trail sync disabled or no regions configured, sleeping");
                tokio::time::sleep(Duration::from_secs(config.interval_secs.max(60))).await;
                continue;
            }

            let interval_secs = config.interval_secs;
            let summary = run_once(&config, &linear_catalog, &observation_catalog).await;
            info!(
                regions = summary.regions_synced,
                ways_upserted = summary.ways_upserted,
                ways_deactivated = summary.ways_deactivated,
                trailheads_upserted = summary.trailheads_upserted,
                errors = summary.errors.len(),
                "Trail sync cycle complete"
            );

            tokio::time::sleep(Duration::from_secs(interval_secs.max(60))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the wrongly-deactivated-44,500-ways incident
    /// (see `safe_to_soft_delete`'s doc comment). The bug was `tile_errors
    /// < tiles.len()`, which is true for ANY partial failure short of a
    /// total wipeout -- these cases must all be false.
    #[test]
    fn test_safe_to_soft_delete_only_when_zero_errors() {
        assert!(safe_to_soft_delete(0));
        assert!(!safe_to_soft_delete(1));
        assert!(!safe_to_soft_delete(34)); // the exact incident: 34/40 tiles failed
        assert!(!safe_to_soft_delete(39)); // the old buggy condition's edge case
        assert!(!safe_to_soft_delete(40)); // total failure -- also unsafe
    }

    #[test]
    fn test_config_defaults_when_file_missing() {
        let config = TrailSyncConfig::load(Path::new("/nonexistent/trail-sync.yaml")).unwrap();
        assert!(!config.enabled);
        assert!(config.regions.is_empty());
    }

    #[test]
    fn test_config_parses_regions() {
        let yaml = r#"
enabled: true
interval_secs: 604800
overpass_url: "https://overpass-api.de/api/interpreter"
regions:
  - name: colorado
    bbox: [-109.06, 36.99, -102.04, 41.00]
"#;
        let config: TrailSyncConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.regions.len(), 1);
        assert_eq!(config.regions[0].name, "colorado");
        assert_eq!(config.regions[0].bbox, (-109.06, 36.99, -102.04, 41.00));
    }

    #[test]
    fn test_trailhead_to_location_id_convention() {
        let mut tags = serde_json::Map::new();
        tags.insert("name".to_string(), serde_json::json!("Apex Trailhead"));
        let node = OsmTrailhead {
            id: 42,
            tags,
            lon: -105.2,
            lat: 39.7,
        };
        let loc = trailhead_to_location(&node, "colorado");
        assert_eq!(loc.id, "TH42");
        assert_eq!(loc.name, "Apex Trailhead");
        assert_eq!(loc.location_type.as_deref(), Some("trailhead"));
        assert_eq!(loc.region.as_deref(), Some("colorado"));
    }

    #[test]
    fn test_trailhead_to_location_falls_back_to_generated_name() {
        let node = OsmTrailhead {
            id: 99,
            tags: serde_json::Map::new(),
            lon: -105.0,
            lat: 39.0,
        };
        let loc = trailhead_to_location(&node, "colorado");
        assert_eq!(loc.name, "Trailhead 99");
    }
}
