//! GOES GLM lightning feature-collection handlers (`glm-lightning`).
//!
//! Backed by the `lightning_flashes` table (see `storage::lightning`), served
//! as GeoJSON points. Endpoints (dispatched from the generic items/area/radius
//! handlers on `observation_source: lightning`):
//!
//! - `items`:  flashes in a bbox (`?bbox=`), or CONUS-wide
//! - `area`:   flashes in a bbox (`?coords=minLon,minLat,maxLon,maxLat`)
//! - `radius`: flashes within a geodesic radius of a point
//!
//! ## Choosing the time range
//!
//! A phone should not have to compute timestamps from its own clock (phone
//! clocks are routinely minutes off), so the window is server-relative:
//!
//! - `window=PT10M` (ISO-8601 duration, default `PT10M`, at most 24 h): "the last
//!   10 minutes, as the server sees it".
//! - `datetime=start/end` for an explicit RFC-3339 interval (`..` = open end).
//!   A single instant is rejected: for event data it is ambiguous.
//! - `window` and `datetime` are mutually exclusive.
//!
//! ## Polling efficiently: the `after` cursor
//!
//! Every feature carries a monotonic `id`, and every response a top-level
//! `lastId`. A client that remembers `lastId` and passes `after=<lastId>` on its
//! next poll receives only new flashes, oldest first, instead of re-downloading
//! the whole window. With `after` and no explicit window, the window defaults to
//! the full 24 h retention (the cursor, not the clock, defines the position).
//!
//! ## Detecting truncation
//!
//! `numberReturned == limit` means there may be more: without `after` you got the
//! *newest* `limit` flashes; with `after` you got the *oldest* `limit` after the
//! cursor, so repeat the request with `after=lastId` to continue.

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::Response,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use storage::lightning::{FlashArea, FlashQuery, StoredFlash};

use crate::handlers::linear_features::{
    bad_request, error_response, internal_error, parse_bbox, parse_point_wkt, parse_radius,
};
use crate::state::AppState;
use crate::temporal_interpolation::parse_iso8601_duration;

/// Window used when the client says nothing about time.
pub const DEFAULT_WINDOW_MINUTES: i64 = 10;
/// Flashes are kept this long, so no window can usefully be longer.
/// Mirrors the ingester's `LIGHTNING_RETENTION_HOURS` default.
pub const MAX_WINDOW_HOURS: i64 = 24;
pub const DEFAULT_LIMIT: i64 = 1000;
pub const MAX_LIMIT: i64 = 10_000;
/// Default / largest radius for `/radius` queries.
const DEFAULT_RADIUS_M: f64 = 50_000.0;
const MAX_RADIUS_M: f64 = 500_000.0;
/// Short: new granules land every 20 s, and a polling client wants fresh data.
const CACHE_MAX_AGE_SECS: u32 = 5;

// =============================================================================
// Query parameters
// =============================================================================

/// The time/selection parameters every lightning endpoint shares, as raw strings.
#[derive(Debug, Default, Clone)]
pub struct CommonParams {
    pub datetime: Option<String>,
    pub window: Option<String>,
    pub after: Option<i64>,
    pub satellite: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LightningItemsParams {
    /// bbox as minLon,minLat,maxLon,maxLat; omitted = all of CONUS.
    pub bbox: Option<String>,
    pub datetime: Option<String>,
    pub window: Option<String>,
    pub after: Option<i64>,
    pub satellite: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LightningAreaParams {
    /// bbox as minLon,minLat,maxLon,maxLat (the EDR `coords` for this collection).
    pub coords: Option<String>,
    pub datetime: Option<String>,
    pub window: Option<String>,
    pub after: Option<i64>,
    pub satellite: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LightningRadiusParams {
    pub coords: Option<String>,
    pub within: Option<String>,
    #[serde(rename = "within-units")]
    pub within_units: Option<String>,
    pub datetime: Option<String>,
    pub window: Option<String>,
    pub after: Option<i64>,
    pub satellite: Option<String>,
    pub limit: Option<i64>,
    pub f: Option<String>,
}

impl LightningItemsParams {
    fn common(&self) -> CommonParams {
        CommonParams {
            datetime: self.datetime.clone(),
            window: self.window.clone(),
            after: self.after,
            satellite: self.satellite.clone(),
            limit: self.limit,
        }
    }
}
impl LightningAreaParams {
    fn common(&self) -> CommonParams {
        CommonParams {
            datetime: self.datetime.clone(),
            window: self.window.clone(),
            after: self.after,
            satellite: self.satellite.clone(),
            limit: self.limit,
        }
    }
}
impl LightningRadiusParams {
    fn common(&self) -> CommonParams {
        CommonParams {
            datetime: self.datetime.clone(),
            window: self.window.clone(),
            after: self.after,
            satellite: self.satellite.clone(),
            limit: self.limit,
        }
    }
}

// =============================================================================
// Pure request resolution (unit tested)
// =============================================================================

/// What the common parameters resolve to.
#[derive(Debug, PartialEq)]
pub struct Resolved {
    /// Empty = both satellites.
    pub satellites: Vec<String>,
    pub since: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
    pub after_id: Option<i64>,
    pub limit: i64,
}

/// Validate and resolve the shared parameters against a fixed `now`.
/// `Err` is the 400 message.
pub fn resolve_common(now: DateTime<Utc>, p: &CommonParams) -> Result<Resolved, String> {
    let max_window = Duration::hours(MAX_WINDOW_HOURS);
    let oldest = now - max_window;

    if p.datetime.is_some() && p.window.is_some() {
        return Err("Use either `datetime` or `window`, not both".to_string());
    }
    if let Some(after) = p.after {
        if after < 0 {
            return Err("`after` must be a non-negative flash id".to_string());
        }
    }

    let (since, until) = if let Some(w) = &p.window {
        let d = parse_iso8601_duration(w).ok_or_else(|| {
            format!(
                "Invalid `window` {:?}; use an ISO-8601 duration such as PT10M",
                w
            )
        })?;
        if d <= Duration::zero() {
            return Err("`window` must be greater than zero".to_string());
        }
        if d > max_window {
            return Err(format!(
                "`window` cannot exceed PT{}H: lightning is only kept for {} hours",
                MAX_WINDOW_HOURS, MAX_WINDOW_HOURS
            ));
        }
        (now - d, None)
    } else if let Some(dt) = &p.datetime {
        parse_interval(dt, oldest)?
    } else if p.after.is_some() {
        // The cursor defines the position; look as far back as flashes exist.
        (oldest, None)
    } else {
        (now - Duration::minutes(DEFAULT_WINDOW_MINUTES), None)
    };

    let satellites = match p.satellite.as_deref() {
        None => vec!["goes-east".to_string()],
        Some("goes-east") => vec!["goes-east".to_string()],
        Some("goes-west") => vec!["goes-west".to_string()],
        Some("both") => vec![],
        Some(other) => {
            return Err(format!(
                "Invalid `satellite` {:?}; use goes-east, goes-west or both",
                other
            ))
        }
    };

    Ok(Resolved {
        satellites,
        since,
        until,
        after_id: p.after,
        limit: p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
    })
}

/// `start/end`, either side may be `..` (open). A lone instant is refused.
/// The lower bound is raised to `oldest`: nothing older exists, and an
/// unbounded scan must not be possible.
fn parse_interval(
    s: &str,
    oldest: DateTime<Utc>,
) -> Result<(DateTime<Utc>, Option<DateTime<Utc>>), String> {
    let Some((a, b)) = s.split_once('/') else {
        return Err(
            "`datetime` must be an interval `start/end` (use `..` for an open end); \
             for \"the last N minutes\" use `window=PT10M`"
                .to_string(),
        );
    };
    let parse = |x: &str| -> Result<Option<DateTime<Utc>>, String> {
        let x = x.trim();
        if x.is_empty() || x == ".." {
            return Ok(None);
        }
        DateTime::parse_from_rfc3339(x)
            .map(|t| Some(t.with_timezone(&Utc)))
            .map_err(|_| {
                format!(
                    "Invalid datetime {:?}; use RFC 3339, e.g. 2026-10-07T20:30:00Z",
                    x
                )
            })
    };
    let (start, end) = (parse(a)?, parse(b)?);
    if let (Some(s), Some(e)) = (start, end) {
        if s > e {
            return Err("`datetime` start is after its end".to_string());
        }
    }
    let since = start.map_or(oldest, |s| s.max(oldest));
    Ok((since, end))
}

/// Default radius handling: omitted -> 50 km; capped at 500 km.
pub fn resolve_radius_m(within: &Option<String>, units: Option<&str>) -> Result<f64, String> {
    let m = if within.is_some() {
        parse_radius(within, units)?
    } else {
        DEFAULT_RADIUS_M
    };
    if !m.is_finite() || m <= 0.0 {
        return Err("`within` must be a positive distance".to_string());
    }
    if m > MAX_RADIUS_M {
        return Err(format!(
            "`within` cannot exceed {} km",
            (MAX_RADIUS_M / 1000.0) as i64
        ));
    }
    Ok(m)
}

fn validate_bbox(b: (f64, f64, f64, f64)) -> Result<FlashArea, String> {
    let (min_lon, min_lat, max_lon, max_lat) = b;
    if ![min_lon, min_lat, max_lon, max_lat]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err("bbox values must be finite numbers".to_string());
    }
    if min_lon >= max_lon || min_lat >= max_lat {
        return Err("bbox must satisfy minLon < maxLon and minLat < maxLat".to_string());
    }
    Ok(FlashArea::Bbox {
        min_lon,
        min_lat,
        max_lon,
        max_lat,
    })
}

// =============================================================================
// Response building (pure)
// =============================================================================

/// Round to `places` decimal places (positions: ~11 m at 4, far finer than GLM's
/// ~8 km pixels, and it keeps the payload small).
fn round_to(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (v * k).round() / k
}

/// A `REAL` widened to f64 prints as `4.699999870296806e-14`; round to 4
/// significant figures so the JSON reads `4.7e-14`.
fn sig4(v: f32) -> f64 {
    format!("{:.3e}", v).parse().unwrap_or(v as f64)
}

pub fn flash_to_feature(f: &StoredFlash, now: DateTime<Utc>) -> Value {
    let age = (now - f.flash_time).num_milliseconds() as f64 / 1000.0;
    json!({
        "type": "Feature",
        "id": f.id,
        "geometry": { "type": "Point", "coordinates": [round_to(f.lon, 4), round_to(f.lat, 4)] },
        "properties": {
            "flash_time": f.flash_time.to_rfc3339_opts(SecondsFormat::Millis, true),
            // Seconds between the flash and `timeStamp` (the server's clock when it
            // answered). A client adds its own time since receiving the response;
            // that way it never needs a correct wall clock.
            "age_seconds": round_to(age, 1),
            "satellite": f.satellite,
            "energy_j": f.energy_j.map(sig4),
            "quality": f.quality,
        }
    })
}

pub fn flashes_to_collection(flashes: &[StoredFlash], now: DateTime<Utc>) -> Value {
    json!({
        "type": "FeatureCollection",
        "features": flashes.iter().map(|f| flash_to_feature(f, now)).collect::<Vec<_>>(),
        "numberReturned": flashes.len(),
        "timeStamp": now.to_rfc3339_opts(SecondsFormat::Millis, true),
        // Cursor for the next poll's `after`; null when nothing was returned
        // (keep the previous one).
        "lastId": flashes.iter().map(|f| f.id).max(),
    })
}

fn geojson_response(value: Value) -> Response {
    match serde_json::to_string(&value) {
        Ok(json) => Response::builder()
            .status(StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, "application/geo+json")
            .header(
                axum::http::header::CACHE_CONTROL,
                format!("max-age={}", CACHE_MAX_AGE_SECS),
            )
            .body(json.into())
            .unwrap(),
        Err(e) => internal_error(format!("Serialization failed: {}", e)),
    }
}

// =============================================================================
// Handlers
// =============================================================================

pub async fn lightning_items_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<LightningItemsParams>,
) -> Response {
    if let Err(resp) = resolve_lightning_collection(&state, &collection_id).await {
        return resp;
    }
    let area = match &params.bbox {
        None => FlashArea::Anywhere,
        Some(b) => match parse_bbox(b).and_then(validate_bbox) {
            Ok(a) => a,
            Err(e) => return bad_request(e),
        },
    };
    run(&state, area, &params.common()).await
}

pub async fn lightning_area_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<LightningAreaParams>,
) -> Response {
    if let Err(resp) = resolve_lightning_collection(&state, &collection_id).await {
        return resp;
    }
    let area =
        match parse_bbox(params.coords.as_deref().unwrap_or_default()).and_then(validate_bbox) {
            Ok(a) => a,
            Err(e) => return bad_request(e),
        };
    run(&state, area, &params.common()).await
}

pub async fn lightning_radius_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<LightningRadiusParams>,
) -> Response {
    if let Err(resp) = resolve_lightning_collection(&state, &collection_id).await {
        return resp;
    }
    let (lon, lat) = match parse_point_wkt(params.coords.as_deref().unwrap_or_default()) {
        Ok(c) => c,
        Err(e) => return bad_request(e),
    };
    if !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) {
        return bad_request("coords out of range (lon -180..180, lat -90..90)");
    }
    let meters = match resolve_radius_m(&params.within, params.within_units.as_deref()) {
        Ok(m) => m,
        Err(e) => return bad_request(e),
    };
    run(
        &state,
        FlashArea::Radius { lon, lat, meters },
        &params.common(),
    )
    .await
}

async fn run(state: &Arc<AppState>, area: FlashArea, common: &CommonParams) -> Response {
    let now = Utc::now();
    let r = match resolve_common(now, common) {
        Ok(r) => r,
        Err(e) => return bad_request(e),
    };
    let query = FlashQuery {
        satellites: r.satellites,
        since: r.since,
        until: r.until,
        after_id: r.after_id,
        area,
        limit: r.limit,
    };
    match state.lightning_catalog.query_flashes(&query).await {
        Ok(flashes) => geojson_response(flashes_to_collection(&flashes, now)),
        Err(e) => internal_error(format!("Lightning query failed: {}", e)),
    }
}

/// The collection must exist and be wired to the lightning backend.
async fn resolve_lightning_collection(
    state: &Arc<AppState>,
    collection_id: &str,
) -> Result<(), Response> {
    let config = state.edr_config.read().await;
    let Some((model_config, _)) = config.find_collection(collection_id) else {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            edr_protocol::responses::ExceptionResponse::not_found(format!(
                "Collection not found: {}",
                collection_id
            )),
        ));
    };
    if !model_config.data_type.is_feature_data()
        || model_config.observation_source.as_deref() != Some("lightning")
    {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            edr_protocol::responses::ExceptionResponse::bad_request(format!(
                "Collection {} is not a lightning collection",
                collection_id
            )),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 7, 21, 0, 0).unwrap()
    }

    fn p() -> CommonParams {
        CommonParams::default()
    }

    // ---- time resolution ----

    #[test]
    fn default_is_the_last_ten_minutes_of_goes_east() {
        let r = resolve_common(now(), &p()).unwrap();
        assert_eq!(r.since, now() - Duration::minutes(10));
        assert_eq!(r.until, None);
        assert_eq!(r.satellites, vec!["goes-east".to_string()]);
        assert_eq!((r.after_id, r.limit), (None, DEFAULT_LIMIT));
    }

    #[test]
    fn window_is_relative_to_the_server_clock() {
        for (w, mins) in [("PT10M", 10), ("PT1H", 60), ("PT90M", 90), ("P1D", 1440)] {
            let r = resolve_common(
                now(),
                &CommonParams {
                    window: Some(w.into()),
                    ..p()
                },
            )
            .unwrap();
            assert_eq!(r.since, now() - Duration::minutes(mins), "{w}");
        }
    }

    #[test]
    fn bad_windows_are_rejected_with_a_helpful_message() {
        for w in ["10", "10m", "PT0M", "PT-5M", "P2D", "PT25H", "banana", ""] {
            let e = resolve_common(
                now(),
                &CommonParams {
                    window: Some(w.into()),
                    ..p()
                },
            )
            .expect_err(&format!("{w:?} must be rejected"));
            assert!(e.contains("window"), "{w:?} -> {e}");
        }
    }

    #[test]
    fn datetime_and_window_together_are_ambiguous() {
        let e = resolve_common(
            now(),
            &CommonParams {
                window: Some("PT10M".into()),
                datetime: Some("2026-10-07T20:00:00Z/..".into()),
                ..p()
            },
        )
        .unwrap_err();
        assert!(e.contains("not both"));
    }

    #[test]
    fn a_single_instant_is_refused_with_advice() {
        let e = resolve_common(
            now(),
            &CommonParams {
                datetime: Some("2026-10-07T20:00:00Z".into()),
                ..p()
            },
        )
        .unwrap_err();
        assert!(e.contains("interval") && e.contains("window"), "{e}");
    }

    #[test]
    fn intervals_support_open_ends() {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let r = resolve_common(
            now(),
            &CommonParams {
                datetime: Some("2026-10-07T20:30:00Z/..".into()),
                ..p()
            },
        )
        .unwrap();
        assert_eq!((r.since, r.until), (t("2026-10-07T20:30:00Z"), None));

        let r = resolve_common(
            now(),
            &CommonParams {
                datetime: Some("2026-10-07T20:30:00Z/2026-10-07T20:40:00Z".into()),
                ..p()
            },
        )
        .unwrap();
        assert_eq!(r.until, Some(t("2026-10-07T20:40:00Z")));

        // open start -> as far back as flashes exist
        let r = resolve_common(
            now(),
            &CommonParams {
                datetime: Some("../2026-10-07T20:40:00Z".into()),
                ..p()
            },
        )
        .unwrap();
        assert_eq!(r.since, now() - Duration::hours(24));
    }

    #[test]
    fn an_interval_older_than_retention_is_clamped_so_no_unbounded_scan_is_possible() {
        let r = resolve_common(
            now(),
            &CommonParams {
                datetime: Some("1999-01-01T00:00:00Z/..".into()),
                ..p()
            },
        )
        .unwrap();
        assert_eq!(r.since, now() - Duration::hours(24));
    }

    #[test]
    fn malformed_and_inverted_intervals_are_rejected() {
        for bad in [
            "not/a-date",
            "2026-10-07T21:00:00Z/2026-10-07T20:00:00Z",
            "2026-10-07/2026-10-08",
            "/",
        ] {
            // "/" is two open ends: valid. The rest must fail.
            let r = resolve_common(
                now(),
                &CommonParams {
                    datetime: Some(bad.into()),
                    ..p()
                },
            );
            if bad == "/" {
                assert!(r.is_ok());
            } else {
                assert!(r.is_err(), "{bad:?} must be rejected");
            }
        }
    }

    #[test]
    fn a_cursor_without_a_window_looks_back_over_all_retained_data() {
        let r = resolve_common(
            now(),
            &CommonParams {
                after: Some(500),
                ..p()
            },
        )
        .unwrap();
        assert_eq!(
            (r.after_id, r.since),
            (Some(500), now() - Duration::hours(24))
        );
        // ...but an explicit window still wins.
        let r = resolve_common(
            now(),
            &CommonParams {
                after: Some(500),
                window: Some("PT30M".into()),
                ..p()
            },
        )
        .unwrap();
        assert_eq!(r.since, now() - Duration::minutes(30));
    }

    #[test]
    fn a_negative_cursor_is_rejected() {
        assert!(resolve_common(
            now(),
            &CommonParams {
                after: Some(-1),
                ..p()
            }
        )
        .is_err());
        assert!(resolve_common(
            now(),
            &CommonParams {
                after: Some(0),
                ..p()
            }
        )
        .is_ok());
    }

    // ---- satellite / limit ----

    #[test]
    fn satellite_selection() {
        let sat = |s: Option<&str>| {
            resolve_common(
                now(),
                &CommonParams {
                    satellite: s.map(String::from),
                    ..p()
                },
            )
            .map(|r| r.satellites)
        };
        assert_eq!(sat(None).unwrap(), vec!["goes-east"]);
        assert_eq!(sat(Some("goes-east")).unwrap(), vec!["goes-east"]);
        assert_eq!(sat(Some("goes-west")).unwrap(), vec!["goes-west"]);
        assert_eq!(
            sat(Some("both")).unwrap(),
            Vec::<String>::new(),
            "empty = no satellite filter"
        );
        for bad in ["east", "G19", "GOES-EAST", ""] {
            assert!(sat(Some(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn limit_is_clamped() {
        let lim = |l| {
            resolve_common(now(), &CommonParams { limit: l, ..p() })
                .unwrap()
                .limit
        };
        assert_eq!(
            (
                lim(None),
                lim(Some(5)),
                lim(Some(0)),
                lim(Some(-9)),
                lim(Some(10_000_000))
            ),
            (1000, 5, 1, 1, 10_000)
        );
    }

    // ---- geometry parameters ----

    #[test]
    fn radius_defaults_and_caps() {
        assert_eq!(resolve_radius_m(&None, None).unwrap(), 50_000.0);
        assert_eq!(
            resolve_radius_m(&Some("25".into()), Some("km")).unwrap(),
            25_000.0
        );
        assert!(resolve_radius_m(&Some("501".into()), Some("km")).is_err());
        assert!(resolve_radius_m(&Some("0".into()), Some("km")).is_err());
        assert!(resolve_radius_m(&Some("-3".into()), Some("km")).is_err());
        assert!(resolve_radius_m(&Some("abc".into()), Some("km")).is_err());
    }

    #[test]
    fn bbox_validation() {
        assert!(validate_bbox((-106.0, 39.0, -105.0, 40.0)).is_ok());
        assert!(
            validate_bbox((-105.0, 39.0, -106.0, 40.0)).is_err(),
            "inverted lon"
        );
        assert!(
            validate_bbox((-106.0, 40.0, -105.0, 39.0)).is_err(),
            "inverted lat"
        );
        assert!(
            validate_bbox((-106.0, 39.0, -106.0, 40.0)).is_err(),
            "zero width"
        );
        assert!(validate_bbox((f64::NAN, 39.0, -105.0, 40.0)).is_err());
    }

    #[test]
    fn params_deserialize_from_a_real_query_string() {
        let uri: axum::http::Uri =
            "/x?bbox=-106,39,-105,40&window=PT5M&after=42&satellite=both&limit=7"
                .parse()
                .unwrap();
        let p = Query::<LightningItemsParams>::try_from_uri(&uri).unwrap().0;
        assert_eq!(
            (
                p.window.as_deref(),
                p.after,
                p.satellite.as_deref(),
                p.limit
            ),
            (Some("PT5M"), Some(42), Some("both"), Some(7))
        );
        let c = p.common();
        assert_eq!((c.after, c.limit), (Some(42), Some(7)));

        let uri: axum::http::Uri =
            "/x?coords=POINT(-105.2%2039.7)&within=30&within-units=km&after=9"
                .parse()
                .unwrap();
        let r = Query::<LightningRadiusParams>::try_from_uri(&uri)
            .unwrap()
            .0;
        assert_eq!(
            (r.within.as_deref(), r.within_units.as_deref(), r.after),
            (Some("30"), Some("km"), Some(9))
        );
    }

    // ---- response shape ----

    fn flash(id: i64, secs_ago: i64, energy: Option<f32>) -> StoredFlash {
        StoredFlash {
            id,
            satellite: "goes-east".into(),
            flash_time: now() - Duration::seconds(secs_ago) + Duration::milliseconds(234),
            flash_id: 40000,
            lon: -105.123456789,
            lat: 40.987654321,
            energy_j: energy,
            quality: 0,
        }
    }

    #[test]
    fn a_feature_has_the_documented_shape() {
        let f = flash_to_feature(&flash(77, 12, Some(4.7e-14)), now());
        assert_eq!(f["type"], "Feature");
        assert_eq!(f["id"], 77);
        assert_eq!(f["geometry"]["type"], "Point");
        assert_eq!(
            f["geometry"]["coordinates"],
            json!([-105.1235, 40.9877]),
            "lon,lat, 4 dp"
        );
        let props = &f["properties"];
        assert_eq!(
            props["flash_time"], "2026-10-07T20:59:48.234Z",
            "ms precision, Z suffix"
        );
        assert_eq!(props["satellite"], "goes-east");
        assert_eq!(props["quality"], 0);
        assert_eq!(
            props["age_seconds"], 11.8,
            "12 s ago minus the 234 ms offset, 1 dp"
        );
        assert_eq!(
            props["energy_j"], 4.7e-14,
            "no f32 widening noise like 4.699999870296806e-14"
        );
    }

    #[test]
    fn missing_energy_is_null_not_zero() {
        let f = flash_to_feature(&flash(1, 5, None), now());
        assert!(f["properties"]["energy_j"].is_null());
    }

    #[test]
    fn energy_survives_the_float_widening_cleanly_across_magnitudes() {
        for e in [2.3e-15_f32, 4.728e-14, 2.078e-12, 1.0e-9] {
            let v = sig4(e);
            assert!((v - e as f64).abs() / (e as f64) < 1e-3, "{e} -> {v}");
            assert!(
                format!("{v:e}").len() <= 10,
                "{e} -> {v:e} should print short"
            );
        }
    }

    #[test]
    fn the_collection_reports_count_timestamp_and_cursor() {
        let c = flashes_to_collection(
            &[
                flash(10, 30, None),
                flash(12, 20, None),
                flash(11, 25, None),
            ],
            now(),
        );
        assert_eq!(c["type"], "FeatureCollection");
        assert_eq!(c["numberReturned"], 3);
        assert_eq!(c["lastId"], 12, "the max id, whatever the order");
        assert_eq!(c["timeStamp"], "2026-10-07T21:00:00.000Z");
        assert_eq!(c["features"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn an_empty_result_has_a_null_cursor_so_the_client_keeps_its_old_one() {
        let c = flashes_to_collection(&[], now());
        assert_eq!(c["numberReturned"], 0);
        assert!(c["lastId"].is_null());
        assert_eq!(c["features"], json!([]));
    }

    #[test]
    fn responses_are_cached_only_briefly() {
        let r = geojson_response(json!({"type": "FeatureCollection", "features": []}));
        assert_eq!(
            r.headers().get(axum::http::header::CACHE_CONTROL).unwrap(),
            "max-age=5"
        );
        assert_eq!(
            r.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
            "application/geo+json"
        );
    }
}
