//! Overpass API client and response parsing.
//!
//! A single Overpass query per region returns both trail/track ways (with
//! inline geometry via `out geom;`) and trailhead nodes in one flat
//! `elements` array, distinguished by `type`. Nodes always carry `lat`/`lon`
//! directly in Overpass JSON regardless of output mode; ways only carry a
//! `geometry` array when `out geom;` is used (as opposed to `out body;`,
//! which would require a second pass resolving member node ids ourselves).

use serde::Deserialize;
use std::time::Duration;

/// A bounding box in (min_lon, min_lat, max_lon, max_lat) order — the same
/// convention used throughout the rest of this codebase (EDR bbox params,
/// populated-places queries, etc.). Converted to Overpass's
/// (south,west,north,east) order only at query-build time.
#[derive(Debug, Clone, Copy)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

/// Raw Overpass JSON response shape (only the fields we use).
#[derive(Debug, Deserialize)]
struct OverpassResponse {
    #[serde(default)]
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Deserialize)]
struct OverpassElement {
    #[serde(rename = "type")]
    kind: String,
    id: i64,
    #[serde(default)]
    tags: serde_json::Map<String, serde_json::Value>,
    /// Present on nodes (always) — trailhead points.
    lat: Option<f64>,
    lon: Option<f64>,
    /// Present on ways when queried with `out geom;` — ordered vertices.
    #[serde(default)]
    geometry: Vec<GeomPoint>,
}

#[derive(Debug, Deserialize)]
struct GeomPoint {
    lat: f64,
    lon: f64,
}

/// A parsed OSM way (trail/track/bridleway candidate).
#[derive(Debug, Clone)]
pub struct OsmWay {
    pub id: i64,
    pub tags: serde_json::Map<String, serde_json::Value>,
    /// Ordered (lon, lat) vertices.
    pub coordinates: Vec<(f64, f64)>,
}

/// A parsed OSM trailhead node.
#[derive(Debug, Clone)]
pub struct OsmTrailhead {
    pub id: i64,
    pub tags: serde_json::Map<String, serde_json::Value>,
    pub lon: f64,
    pub lat: f64,
}

/// Result of parsing one region's Overpass response.
#[derive(Debug, Default)]
pub struct OverpassResult {
    pub ways: Vec<OsmWay>,
    pub trailheads: Vec<OsmTrailhead>,
}

/// Split a bounding box into a grid of smaller tiles.
///
/// Exists because a single statewide Overpass request (confirmed live: all
/// of Colorado's path/track/bridleway ways in one `out geom;` response) is
/// ~300 MB of JSON, which parses into a Rust structure large enough to push
/// the ingester over its memory limit and crash it mid-sync -- confirmed
/// during the trail-conditions session's deploy verification (repeated
/// restarts, memory at 99.97% of a 4 GiB limit). Tiling bounds peak memory
/// to roughly one tile's worth of parsed data regardless of region size.
pub fn subdivide_bbox(bbox: BBox, tile_deg: f64) -> Vec<BBox> {
    let tile_deg = tile_deg.max(0.1); // guard against a pathological near-zero config value
    let mut tiles = Vec::new();
    let mut lat = bbox.min_lat;
    while lat < bbox.max_lat {
        let next_lat = (lat + tile_deg).min(bbox.max_lat);
        let mut lon = bbox.min_lon;
        while lon < bbox.max_lon {
            let next_lon = (lon + tile_deg).min(bbox.max_lon);
            tiles.push(BBox {
                min_lon: lon,
                min_lat: lat,
                max_lon: next_lon,
                max_lat: next_lat,
            });
            lon = next_lon;
        }
        lat = next_lat;
    }
    tiles
}

/// Build the Overpass QL query for a bounding box.
///
/// Matches `highway=path|track|bridleway` ways (generic unpaved linear
/// features — filtering into mtb-specific vs hiking-specific happens later
/// via tag inspection, not here, so this same sync can back other
/// linear-feature audiences later) plus `highway=trailhead` nodes.
fn build_query(bbox: BBox) -> String {
    format!(
        "[out:json][timeout:180];\n\
         (\n\
         \x20\x20way[\"highway\"~\"^(path|track|bridleway)$\"]({south},{west},{north},{east});\n\
         \x20\x20node[\"highway\"=\"trailhead\"]({south},{west},{north},{east});\n\
         );\n\
         out geom;",
        south = bbox.min_lat,
        west = bbox.min_lon,
        north = bbox.max_lat,
        east = bbox.max_lon,
    )
}

/// Query Overpass for a region and parse the response into ways + trailheads.
/// Query Overpass for a region, retrying transient failures.
///
/// Confirmed live during the trail-conditions session: a run that hammered
/// Overpass across many tiles in quick succession (during an OOM-restart
/// loop, before the memory fix) got itself network-refused for most of a
/// statewide pass ("Connection refused" / "Network is unreachable"), and
/// since a failed tile makes the caller skip the soft-delete pass entirely
/// (see `run_once`), a transient blip shouldn't cost the whole tile. Retries
/// up to `MAX_ATTEMPTS` times with a fixed backoff -- deliberately simple,
/// not exponential, since Overpass rate-limiting responds to *any* patience
/// at all, not to a particular curve.
pub async fn fetch_region(
    client: &reqwest::Client,
    overpass_url: &str,
    bbox: BBox,
) -> anyhow::Result<OverpassResult> {
    const MAX_ATTEMPTS: u32 = 3;
    const BACKOFF: Duration = Duration::from_secs(15);

    let mut last_err = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match fetch_region_once(client, overpass_url, bbox).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                tracing::warn!(
                    attempt,
                    max_attempts = MAX_ATTEMPTS,
                    error = %e,
                    "Overpass tile fetch failed, will retry"
                );
                last_err = Some(e);
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(BACKOFF).await;
                }
            }
        }
    }
    Err(last_err.unwrap())
}

async fn fetch_region_once(
    client: &reqwest::Client,
    overpass_url: &str,
    bbox: BBox,
) -> anyhow::Result<OverpassResult> {
    let query = build_query(bbox);

    // Overpass API (and the Apache instance in front of it) reject requests
    // with no User-Agent / a generic one with a 406 -- confirmed live during
    // the trail-conditions session. A descriptive UA is effectively required,
    // not optional, for this API.
    let response = client
        .post(overpass_url)
        .timeout(Duration::from_secs(200))
        .header(
            reqwest::header::USER_AGENT,
            "weather-wms-trailsync/0.1 (https://folkweather.com)",
        )
        .form(&[("data", query.as_str())])
        .send()
        .await?
        .error_for_status()?;

    let body: OverpassResponse = response.json().await?;

    let mut result = OverpassResult::default();
    for el in body.elements {
        match el.kind.as_str() {
            "way" if el.geometry.len() >= 2 => {
                result.ways.push(OsmWay {
                    id: el.id,
                    tags: el.tags,
                    coordinates: el.geometry.iter().map(|g| (g.lon, g.lat)).collect(),
                });
            }
            "node" => {
                if let (Some(lon), Some(lat)) = (el.lon, el.lat) {
                    if el.tags.get("highway").and_then(|v| v.as_str()) == Some("trailhead") {
                        result.trailheads.push(OsmTrailhead {
                            id: el.id,
                            tags: el.tags,
                            lon,
                            lat,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    Ok(result)
}

/// Classify an OSM way into our `feature_class` discriminator.
///
/// Best-effort tag inspection, not a restructuring of OSM's own taxonomy:
/// `highway=track`/`bridleway` map directly; `highway=path` is split into
/// `mtb_trail` vs `hiking_trail` based on bicycle access tags (falling back
/// to `hiking_trail` since untagged paths are far more often foot-only than
/// not). Unrecognized `highway` values (shouldn't occur given the query
/// filter, but defensive) fall back to `"unknown"`.
pub fn classify_way(tags: &serde_json::Map<String, serde_json::Value>) -> String {
    let highway = tags.get("highway").and_then(|v| v.as_str());
    match highway {
        Some("track") => "track".to_string(),
        Some("bridleway") => "bridleway".to_string(),
        Some("path") => {
            let bicycle = tags.get("bicycle").and_then(|v| v.as_str());
            let has_mtb_scale =
                tags.contains_key("mtb:scale") || tags.contains_key("mtb:scale:uphill");
            if has_mtb_scale
                || matches!(
                    bicycle,
                    Some("yes") | Some("designated") | Some("permissive")
                )
            {
                "mtb_trail".to_string()
            } else {
                "hiking_trail".to_string()
            }
        }
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bbox() -> BBox {
        BBox {
            min_lon: -109.06,
            min_lat: 36.99,
            max_lon: -102.04,
            max_lat: 41.00,
        }
    }

    #[test]
    fn test_subdivide_bbox_covers_whole_area_with_no_gaps() {
        let tiles = subdivide_bbox(bbox(), 1.0);
        // Colorado is ~7.02 deg wide, ~4.01 deg tall -> 8 x 5 = 40 tiles
        assert_eq!(tiles.len(), 40);
        // Every tile must stay within the original bbox.
        for t in &tiles {
            assert!(t.min_lon >= bbox().min_lon - 1e-9);
            assert!(t.max_lon <= bbox().max_lon + 1e-9);
            assert!(t.min_lat >= bbox().min_lat - 1e-9);
            assert!(t.max_lat <= bbox().max_lat + 1e-9);
        }
        // First tile starts exactly at the region's corner.
        assert_eq!(tiles[0].min_lon, bbox().min_lon);
        assert_eq!(tiles[0].min_lat, bbox().min_lat);
        // Last tile ends exactly at the region's far corner.
        let last = tiles.last().unwrap();
        assert_eq!(last.max_lon, bbox().max_lon);
        assert_eq!(last.max_lat, bbox().max_lat);
    }

    #[test]
    fn test_subdivide_bbox_small_region_single_tile() {
        let small = BBox {
            min_lon: -105.3,
            min_lat: 39.6,
            max_lon: -105.1,
            max_lat: 39.8,
        };
        let tiles = subdivide_bbox(small, 1.0);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].min_lon, small.min_lon);
        assert_eq!(tiles[0].max_lon, small.max_lon);
    }

    #[test]
    fn test_subdivide_bbox_guards_against_tiny_tile_size() {
        // A pathological config value shouldn't create an unbounded number of tiles.
        let tiles = subdivide_bbox(bbox(), 0.0);
        assert!(tiles.len() < 10_000);
    }

    #[test]
    fn test_build_query_orders_bbox_as_overpass_south_west_north_east() {
        let q = build_query(bbox());
        assert!(q.contains("(36.99,-109.06,41,-102.04)"));
        assert!(q.contains("highway"));
        assert!(q.contains("trailhead"));
        assert!(q.contains("out geom;"));
    }

    #[test]
    fn test_classify_track() {
        let mut tags = serde_json::Map::new();
        tags.insert("highway".to_string(), json!("track"));
        assert_eq!(classify_way(&tags), "track");
    }

    #[test]
    fn test_classify_bridleway() {
        let mut tags = serde_json::Map::new();
        tags.insert("highway".to_string(), json!("bridleway"));
        assert_eq!(classify_way(&tags), "bridleway");
    }

    #[test]
    fn test_classify_path_with_mtb_scale_is_mtb_trail() {
        let mut tags = serde_json::Map::new();
        tags.insert("highway".to_string(), json!("path"));
        tags.insert("mtb:scale".to_string(), json!("2"));
        assert_eq!(classify_way(&tags), "mtb_trail");
    }

    #[test]
    fn test_classify_path_with_bicycle_designated_is_mtb_trail() {
        let mut tags = serde_json::Map::new();
        tags.insert("highway".to_string(), json!("path"));
        tags.insert("bicycle".to_string(), json!("designated"));
        assert_eq!(classify_way(&tags), "mtb_trail");
    }

    #[test]
    fn test_classify_bare_path_defaults_to_hiking_trail() {
        let mut tags = serde_json::Map::new();
        tags.insert("highway".to_string(), json!("path"));
        assert_eq!(classify_way(&tags), "hiking_trail");
    }

    #[test]
    fn test_classify_path_bicycle_no_is_hiking_trail() {
        let mut tags = serde_json::Map::new();
        tags.insert("highway".to_string(), json!("path"));
        tags.insert("bicycle".to_string(), json!("no"));
        assert_eq!(classify_way(&tags), "hiking_trail");
    }

    #[test]
    fn test_parse_response_separates_ways_and_trailheads() {
        let raw = json!({
            "elements": [
                {
                    "type": "way",
                    "id": 123,
                    "tags": {"highway": "track", "name": "Test Track"},
                    "geometry": [
                        {"lat": 39.7, "lon": -105.2},
                        {"lat": 39.71, "lon": -105.21}
                    ]
                },
                {
                    "type": "node",
                    "id": 456,
                    "lat": 39.72,
                    "lon": -105.22,
                    "tags": {"highway": "trailhead", "name": "Test Trailhead"}
                },
                {
                    "type": "way",
                    "id": 789,
                    "tags": {"highway": "track"},
                    "geometry": [{"lat": 1.0, "lon": 1.0}]
                }
            ]
        });
        let parsed: OverpassResponse = serde_json::from_value(raw).unwrap();
        let mut result = OverpassResult::default();
        for el in parsed.elements {
            match el.kind.as_str() {
                "way" if el.geometry.len() >= 2 => {
                    result.ways.push(OsmWay {
                        id: el.id,
                        tags: el.tags,
                        coordinates: el.geometry.iter().map(|g| (g.lon, g.lat)).collect(),
                    });
                }
                "node" => {
                    if let (Some(lon), Some(lat)) = (el.lon, el.lat) {
                        result.trailheads.push(OsmTrailhead {
                            id: el.id,
                            tags: el.tags,
                            lon,
                            lat,
                        });
                    }
                }
                _ => {}
            }
        }
        // way 789 has only 1 geometry point, must be dropped
        assert_eq!(result.ways.len(), 1);
        assert_eq!(result.ways[0].id, 123);
        assert_eq!(
            result.ways[0].coordinates,
            vec![(-105.2, 39.7), (-105.21, 39.71)]
        );
        assert_eq!(result.trailheads.len(), 1);
        assert_eq!(result.trailheads[0].id, 456);
    }
}
