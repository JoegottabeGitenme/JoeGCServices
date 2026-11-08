//! The `nws-alerts` feature collection: National Weather Service watches,
//! warnings and advisories, served from the `nws_alerts` PostGIS tables that the
//! downloader/ingester keep current (see `storage::nws_alerts`).
//!
//! The question the app asks is "what alerts are active at my location right
//! now?", so the primary query is `/position`. It answers with a compact
//! FeatureCollection, worst alert first, each feature carrying just what a small
//! card needs (event, severity, colour, headline, when, what/impacts/instruction).
//! `/items/{id}` returns one alert in full; `/items`, `/area` and `/radius` serve
//! maps and nearby-alert lists.
//!
//! The wire shape is built by pure functions at the top of this file
//! ([`compact_feature`], [`full_feature`], [`feed_meta`], ...) so it is unit
//! tested without a database.
//!
//! Every list response carries a top-level `meta` with the age of the data
//! (`stale` once the last successful poll of api.weather.gov is over
//! [`STALE_AFTER_SECS`] old) so a client can say so rather than present an
//! outage as "no alerts".

use axum::{
    extract::{
        rejection::{PathRejection, QueryRejection},
        Extension, Path, Query, RawQuery,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use edr_protocol::collections::geojson_output_formats;
use edr_protocol::responses::ExceptionResponse;
use edr_protocol::{Collection, DataQueries, Extent, Parameter};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Map, Value};
use std::sync::Arc;

use storage::nws_alerts::catalog::radius_boxes;
use storage::nws_alerts::sections::{derived_what, ForecastClock};
use storage::{AlertRegion, AlertSearch, FeedStatus, StoredAlert};
use wms_common::WmsError;

use crate::config::LimitsConfig;
use crate::content_negotiation::check_accept_header;
use crate::state::AppState;

/// `observation_source` value in `config/edr/nws-alerts.yaml`.
pub const OBSERVATION_SOURCE: &str = "nws_alerts";

/// Data older than this (since the last successful poll) is flagged `stale`.
pub const STALE_AFTER_SECS: i64 = 300;

/// `Cache-Control` of every successful `nws-alerts` response, the collection document included.
pub const CACHE_CONTROL: &str = "public, max-age=60";
const DEFAULT_ITEMS_LIMIT: i64 = 500;
const MAX_ITEMS_LIMIT: i64 = 2000;
/// Upper bound for position/area/radius answers (a point is never under this many alerts).
const NEARBY_LIMIT: i64 = 500;
const MAX_RADIUS_M: f64 = 500_000.0;
/// Most vertices an `/area` ring may have (the closing repeat of the first one not counted). The
/// ring is checked for self-intersection pairwise, and PostGIS repairs an invalid one at a cost
/// that explodes with the crossings, so the size is capped well below what a URL could carry.
const MAX_POLYGON_VERTICES: usize = 500;
/// Default for the `NWS_ALERTS_QUERY_TIMEOUT_MS` environment variable.
pub const DEFAULT_QUERY_TIMEOUT_MS: u64 = 10_000;
/// Compact-mode text caps.
const INSTRUCTION_MAX: usize = 600;
const SUMMARY_MAX: usize = 500;
const SUMMARY_KEYS: [&str; 7] = [
    "what", "where", "when", "impacts", "hazard", "source", "impact",
];

// =============================================================================
// Pure mapping
// =============================================================================

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn opt_ts(t: Option<DateTime<Utc>>) -> Value {
    t.map(|t| json!(ts(t))).unwrap_or(Value::Null)
}

/// Percent-encode an alert id for use as one URL path segment (`:` becomes `%3A`).
pub fn encode_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 8);
    for b in id.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Trim whitespace and cap at `max` characters, cutting at a word boundary and
/// ending with an ellipsis when something was dropped.
pub fn trim_text(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        // keep a word boundary unless that would throw away most of the budget
        Some(i) if i > max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!(
        "{}…",
        cut.trim_end_matches(|c: char| c.is_whitespace() || ",;:-".contains(c))
    )
}

/// `active` | `upcoming` | `ended` | `superseded`, as of `now`.
///
/// NWS lists an alert as active from the moment it is issued, even when the
/// hazard starts hours later, so `upcoming` is "onset (else effective) is still
/// ahead". Replaced and ended count only once they had happened by `now`, so an
/// alert read at a past instant shows what it was then, not what became of it.
pub fn alert_status(a: &StoredAlert, now: DateTime<Utc>) -> &'static str {
    if a.superseded_by.is_some() && a.superseded_at.is_none_or(|s| s <= now) {
        "superseded"
    } else if a.ended_at.is_some_and(|e| e <= now) || a.end_time.is_some_and(|e| e <= now) {
        "ended"
    } else if a.onset.or(a.effective).is_some_and(|s| s > now) {
        "upcoming"
    } else {
        "active"
    }
}

fn first_vtec(a: &StoredAlert) -> Option<&Value> {
    a.vtec.as_array().and_then(|v| v.first())
}

/// `warning` | `watch` | `advisory` | `statement` | `other`.
///
/// From the first VTEC code's significance letter (W/A/Y/S), else from the
/// event name (`... Warning`).
pub fn significance(a: &StoredAlert) -> &'static str {
    let letter = first_vtec(a)
        .and_then(|v| v["significance"].as_str())
        .and_then(|s| s.chars().next());
    match letter {
        Some('W') => return "warning",
        Some('A') => return "watch",
        Some('Y') => return "advisory",
        Some('S') => return "statement",
        _ => {}
    }
    let e = a.event.to_lowercase();
    if e.ends_with("warning") {
        "warning"
    } else if e.ends_with("watch") {
        "watch"
    } else if e.ends_with("advisory") {
        "advisory"
    } else if e.ends_with("statement") {
        "statement"
    } else {
        "other"
    }
}

/// The card headline: NWS' broadcast text (`NWSheadline`, e.g. "FLOOD WARNING IN EFFECT UNTIL 730 PM
/// MST MONDAY") when it gave one, else its formal headline ("Flood Warning issued November 1 at
/// 7:26PM MST until November 2 at 7:30PM MST by NWS Pueblo CO"). NWS leaves the broadcast text out
/// of some messages (new short-fused warnings, but also some Small Craft Advisories, Avalanche
/// Advisories and Special Weather Statements), so the style can change between the messages of one
/// event. The broadcast text says "TODAY" / "TONIGHT" for the local day it was sent, and the
/// formal headline is dated: both go out as `headline` and `formal_headline`.
fn headline(a: &StoredAlert) -> Option<&str> {
    a.nws_headline
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(a.headline.as_deref())
}

fn api_url(id: &str) -> String {
    format!("https://api.weather.gov/alerts/{id}")
}

fn links(a: &StoredAlert, base_url: &str) -> Value {
    let mut l = vec![
        json!({"rel": "self", "type": "application/geo+json",
               "href": format!("{}/collections/nws-alerts/items/{}", base_url.trim_end_matches('/'), encode_id(&a.id))}),
        json!({"rel": "alternate", "type": "application/geo+json", "href": api_url(&a.id)}),
    ];
    if let Some(w) = a.web.as_deref().filter(|w| !w.is_empty()) {
        l.push(json!({"rel": "related", "type": "text/html", "href": w}));
    }
    Value::Array(l)
}

/// What / where / when / impacts ... as plain strings, only the keys present.
///
/// An alert whose text has neither `WHAT...` nor `HAZARD...` gets a `what` all the same when
/// there is something to say (see [`derived_what`]): the forecast period(s) its headline names for
/// the Alaska coastal-water alerts, the first sentence or two of a free-text statement. The
/// forecast's `Today` and `Tonight` are read against the day the alert was sent, or the day its
/// periods put them on (`now` is when it is read), so a forecast read a day late says `Sun night`,
/// and leaves out the periods that are over.
fn summary(a: &StoredAlert, now: DateTime<Utc>, cap: Option<usize>) -> Value {
    let clock = a
        .sent
        .zip(a.headline.as_deref())
        .and_then(|(sent, formal)| ForecastClock::new(sent, now, formal));
    let derived = derived_what(&a.sections, headline(a), clock.as_ref());
    let mut m = Map::new();
    for k in SUMMARY_KEYS {
        let text = match a.sections.get(k).and_then(|v| v.as_str()).map(str::trim) {
            Some(t) if !t.is_empty() => Some(t),
            _ if k == "what" => derived.as_deref(),
            _ => None,
        };
        if let Some(t) = text {
            m.insert(
                k.to_string(),
                json!(match cap {
                    Some(n) => trim_text(t, n),
                    None => t.to_string(),
                }),
            );
        }
    }
    Value::Object(m)
}

fn instruction_text(a: &StoredAlert) -> Option<&str> {
    a.instruction_text
        .as_deref()
        .or(a.instruction.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn common_properties(
    a: &StoredAlert,
    now: DateTime<Utc>,
    base_url: &str,
    compact: bool,
) -> Map<String, Value> {
    let vtec0 = first_vtec(a);
    let mut p = Map::new();
    p.insert("event".into(), json!(a.event));
    p.insert("severity".into(), json!(a.severity));
    p.insert("urgency".into(), json!(a.urgency));
    p.insert("certainty".into(), json!(a.certainty));
    p.insert("color".into(), json!(a.color));
    p.insert("headline".into(), json!(headline(a)));
    // NWS' own `headline` property, which `headline` above replaces with the broadcast text: dated
    // ("... issued November 1 at 3:09AM AKST until ..."), where the broadcast text says "TONIGHT"
    // for the day it was sent
    p.insert("formal_headline".into(), json!(a.headline));
    p.insert("area".into(), json!(a.area_desc));
    p.insert("sender".into(), json!(a.sender_name));
    p.insert("sent".into(), opt_ts(a.sent));
    p.insert("effective".into(), opt_ts(a.effective));
    p.insert("onset".into(), opt_ts(a.onset));
    // NWS' own: null when NWS gave none ("until further notice"). The lifecycle uses `end_time`
    // (`ends`, else `expires`), so such an alert is over at `expires`.
    p.insert("ends".into(), opt_ts(a.ends));
    p.insert("expires".into(), opt_ts(a.expires));
    p.insert("status".into(), json!(alert_status(a, now)));
    // The CAP status: Actual, or Test / Exercise / System / Draft (only returned on request).
    p.insert("cap_status".into(), json!(a.status));
    p.insert("significance".into(), json!(significance(a)));
    p.insert(
        "action".into(),
        vtec0
            .and_then(|v| v["action"].as_str())
            .map(|s| json!(s))
            .unwrap_or(Value::Null),
    );
    p.insert("vtec_event".into(), json!(a.vtec_event_key));
    p.insert(
        "summary".into(),
        summary(a, now, compact.then_some(SUMMARY_MAX)),
    );
    let instr = instruction_text(a).map(|t| {
        if compact {
            trim_text(t, INSTRUCTION_MAX)
        } else {
            t.to_string()
        }
    });
    p.insert("instruction".into(), json!(instr));
    p.insert("geometry_source".into(), json!(a.geometry_source));
    p.insert("links".into(), links(a, base_url));
    p
}

fn geometry_value(a: &StoredAlert) -> Value {
    a.geometry_geojson
        .as_deref()
        .and_then(|g| serde_json::from_str(g).ok())
        .unwrap_or(Value::Null)
}

/// A feature of a list answer: the compact one used by position / items / area / radius, with the
/// geometry kept as the database's own GeoJSON text and written through as it is. Parsed into a
/// `Value` tree that text takes about nine times its size in memory, and a full-resolution answer
/// is tens of MB of it.
#[derive(Serialize)]
struct ListFeature<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
    geometry: Option<&'a RawValue>,
    properties: Map<String, Value>,
}

fn list_feature<'a>(a: &'a StoredAlert, now: DateTime<Utc>, base_url: &str) -> ListFeature<'a> {
    ListFeature {
        kind: "Feature",
        id: &a.id,
        // (text that is not JSON is as good as no geometry)
        geometry: a
            .geometry_geojson
            .as_deref()
            .and_then(|g| serde_json::from_str(g).ok()),
        properties: common_properties(a, now, base_url, true),
    }
}

/// The compact feature used by position / items / area / radius.
pub fn compact_feature(a: &StoredAlert, now: DateTime<Utc>, base_url: &str) -> Value {
    serde_json::to_value(list_feature(a, now, base_url)).unwrap_or(Value::Null)
}

/// The full feature served by `/items/{id}`: compact fields (untrimmed) plus all
/// text, sections, VTEC, parameters, zones and lifecycle.
pub fn full_feature(a: &StoredAlert, now: DateTime<Utc>, base_url: &str) -> Value {
    let mut p = common_properties(a, now, base_url, false);
    p.insert("message_type".into(), json!(a.message_type));
    p.insert("category".into(), json!(a.category));
    p.insert("response".into(), json!(a.response));
    p.insert(
        "description".into(),
        json!(a.description_text.as_ref().or(a.description.as_ref())),
    );
    p.insert("description_raw".into(), json!(a.description));
    p.insert("instruction_raw".into(), json!(a.instruction));
    p.insert("sections".into(), a.sections.clone());
    p.insert("vtec".into(), a.vtec.clone());
    p.insert("parameters".into(), a.parameters.clone());
    p.insert("ugc".into(), json!(a.ugc));
    p.insert("zones".into(), json!(a.zone_keys));
    p.insert("is_marine".into(), json!(a.is_marine));
    p.insert("references".into(), json!(a.references_ids));
    p.insert("superseded_by".into(), json!(a.superseded_by));
    p.insert("superseded_at".into(), opt_ts(a.superseded_at));
    p.insert("ended_at".into(), opt_ts(a.ended_at));
    p.insert("first_seen_at".into(), json!(ts(a.first_seen_at)));
    p.insert("last_seen_at".into(), json!(ts(a.last_seen_at)));
    p.insert("updated_at".into(), json!(ts(a.updated_at)));
    json!({
        "type": "Feature",
        "id": a.id,
        "geometry": geometry_value(a),
        "properties": p,
    })
}

/// Freshness block. `stale` when the feed has never been polled successfully or
/// the last success is over [`STALE_AFTER_SECS`] old.
pub fn feed_meta(status: Option<&FeedStatus>, now: DateTime<Utc>) -> Value {
    let last = status.and_then(|s| s.last_success_at);
    let age = last.map(|t| (now - t).num_seconds().max(0));
    let mut m = json!({
        "source": "api.weather.gov",
        "last_success_at": opt_ts(last),
        "data_age_seconds": age,
        "stale": age.is_none_or(|a| a > STALE_AFTER_SECS),
    });
    if let Some(s) = status {
        // A poll failing right now is worth knowing even before the data goes stale.
        if s.last_error_at.is_some_and(|e| last.is_none_or(|l| e > l)) {
            m["last_error"] = json!(s.last_error);
            m["last_error_at"] = opt_ts(s.last_error_at);
        }
        m["feed_updated_at"] = opt_ts(s.feed_updated_at);
    }
    m
}

/// The answer to a list query: `extra` carries `limit` and `truncated`.
#[derive(Serialize)]
struct ListCollection<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "numberReturned")]
    number_returned: usize,
    features: Vec<ListFeature<'a>>,
    #[serde(rename = "timeStamp")]
    time_stamp: String,
    meta: Value,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn collection(
    features: Vec<ListFeature<'_>>,
    now: DateTime<Utc>,
    meta: Value,
) -> ListCollection<'_> {
    ListCollection {
        kind: "FeatureCollection",
        number_returned: features.len(),
        features,
        time_stamp: ts(now),
        meta,
        extra: Map::new(),
    }
}

/// How many of the leading `rows` an answer may carry so that their geometry text stays within
/// `budget` bytes (the first row always, however large), and whether the budget cut it short.
fn within_budget(rows: &[StoredAlert], budget: usize) -> (usize, bool) {
    let mut used = 0usize;
    for (i, a) in rows.iter().enumerate() {
        used = used.saturating_add(a.geometry_geojson.as_ref().map_or(0, String::len));
        if used > budget {
            return (i.max(1), true);
        }
    }
    (rows.len(), false)
}

// =============================================================================
// Request parsing (pure)
// =============================================================================

/// Query parameters shared by every `nws-alerts` query. Each handler reads the
/// subset it documents.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct NwsQuery {
    pub coords: Option<String>,
    pub bbox: Option<String>,
    pub datetime: Option<String>,
    /// Comma list, case-insensitive: `Severe,Extreme`.
    pub severity: Option<String>,
    /// Comma list of event names, case-insensitive.
    pub event: Option<String>,
    /// items only: `active` (default) or `all`.
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub geometry: Option<String>,
    /// Geometry simplification tolerance in degrees; `0` disables.
    pub simplify: Option<f64>,
    pub within: Option<String>,
    #[serde(rename = "within-units")]
    pub within_units: Option<String>,
    pub include_superseded: Option<String>,
    /// `true` also returns CAP status Test / Exercise / System / Draft alerts (default: Actual only).
    pub include_non_actual: Option<String>,
    /// Output format: GeoJSON, the only one there is (see [`check_format`]).
    pub f: Option<String>,
}

fn nums(s: &str) -> Result<Vec<f64>, String> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().map_err(|_| format!("Invalid number: {t}")))
        .collect()
}

fn check_lonlat(lon: f64, lat: f64) -> Result<(), String> {
    if lon.is_finite()
        && lat.is_finite()
        && (-180.0..=180.0).contains(&lon)
        && (-90.0..=90.0).contains(&lat)
    {
        Ok(())
    } else {
        Err("Coordinates out of range (lon -180..180, lat -90..90)".into())
    }
}

/// `POINT(lon lat)` or `lon,lat`.
pub fn parse_point(coords: &str) -> Result<(f64, f64), String> {
    let c = coords.trim();
    // `get` (not `c[..5]`): the user's text may have a multi-byte character across the cut.
    let inner = if c.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("POINT")) {
        c[5..].trim().trim_start_matches('(').trim_end_matches(')')
    } else {
        c
    };
    let n = nums(inner)?;
    if n.len() != 2 {
        return Err("Invalid coords. Use POINT(lon lat) or lon,lat".into());
    }
    check_lonlat(n[0], n[1])?;
    Ok((n[0], n[1]))
}

/// `POLYGON((lon lat, ...))` (one ring, either winding), returned as validated WKT.
///
/// The ring is closed for you and must be simple: at least 3 vertices (not all on one line), at
/// most `MAX_POLYGON_VERTICES`, no edge spanning more than 180 degrees of longitude (so no
/// crossing the antimeridian), no inner rings, and no two edges crossing, touching or
/// overlapping. Anything else PostGIS would have to repair, at a cost the request should not get
/// to choose.
pub fn parse_polygon(coords: &str) -> Result<String, String> {
    parse_polygon_with_bbox(coords).map(|(wkt, _)| wkt)
}

/// [`parse_polygon`], and the ring's `[west, south, east, north]` box as well.
pub fn parse_polygon_with_bbox(coords: &str) -> Result<(String, [f64; 4]), String> {
    const SHAPE: &str = "Invalid coords. Use POLYGON((lon lat, lon lat, ...))";
    let c = coords.trim();
    if !c
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("POLYGON"))
    {
        return Err(SHAPE.into());
    }
    let body = c[7..]
        .trim()
        .strip_prefix('(')
        .and_then(|b| b.strip_suffix(')'))
        .ok_or(SHAPE)?;
    let (ring, rest) = body
        .trim()
        .strip_prefix('(')
        .and_then(|b| b.split_once(')'))
        .ok_or(SHAPE)?;
    if ring.contains('(') {
        return Err(SHAPE.into());
    }
    match rest.trim() {
        "" => {}
        r if r.starts_with(',') => {
            return Err(
                "Polygon holes are not supported (inner rings); send the outer ring only".into(),
            )
        }
        _ => return Err(SHAPE.into()),
    }
    let mut pts: Vec<Pt> = Vec::new();
    for pair in ring.split(',') {
        let n = nums(pair)?;
        if n.len() != 2 {
            return Err("Invalid polygon vertex".into());
        }
        check_lonlat(n[0], n[1])?;
        // A repeated vertex adds nothing; dropping it keeps zero-length edges out of the checks.
        if pts.last() != Some(&(n[0], n[1])) {
            pts.push((n[0], n[1]));
        }
        if pts.len() > MAX_POLYGON_VERTICES + 1 {
            break; // already too many; no need to read the rest
        }
    }
    if pts.first() != pts.last() {
        pts.push(pts[0]);
    }
    if pts.len() - 1 > MAX_POLYGON_VERTICES {
        return Err(format!(
            "Polygon has more than {MAX_POLYGON_VERTICES} vertices"
        ));
    }
    if pts.len() < 4 {
        return Err("A polygon needs at least 3 distinct vertices".into());
    }
    if pts.windows(2).any(|e| (e[1].0 - e[0].0).abs() > 180.0) {
        return Err(
            "Polygon must not cross the antimeridian (an edge spans more than 180 \
                    degrees of longitude); split it into two requests, one on each side of 180"
                .into(),
        );
    }
    if pts
        .iter()
        .all(|&p| turn(pts[0], pts[1], p) == Turn::Collinear)
    {
        return Err("Polygon has no area (all its vertices are on one line)".into());
    }
    if let Some((i, j)) = find_self_contact(&pts) {
        return Err(format!(
            "Polygon must be a simple ring: its edges {} and {} cross, touch or overlap",
            i + 1,
            j + 1
        ));
    }
    let body: Vec<String> = pts.iter().map(|(x, y)| format!("{x} {y}")).collect();
    let bbox = pts
        .iter()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, &(x, y)| {
            [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
        });
    Ok((format!("POLYGON(({}))", body.join(", ")), bbox))
}

type Pt = (f64, f64);

#[derive(Clone, Copy, PartialEq)]
enum Turn {
    Left,
    Right,
    /// On the line, or too close to it for f64 to say which side.
    Collinear,
}

/// Which side of the directed line `a`-`b` the point `c` is on. The determinant is only trusted
/// outside Shewchuk's bound on its rounding error (his `ccwerrboundA`); inside it the answer is
/// `Collinear`, so decimal coordinates that are collinear on paper but not bit for bit still
/// count as collinear.
fn turn(a: Pt, b: Pt, c: Pt) -> Turn {
    const EPS: f64 = f64::EPSILON / 2.0;
    const ERR_BOUND: f64 = (3.0 + 16.0 * EPS) * EPS;
    let left = (a.0 - c.0) * (b.1 - c.1);
    let right = (a.1 - c.1) * (b.0 - c.0);
    let det = left - right;
    let bound = ERR_BOUND * (left.abs() + right.abs());
    if det > bound {
        Turn::Left
    } else if -det > bound {
        Turn::Right
    } else {
        Turn::Collinear
    }
}

/// `p` lies in the bounding box of `a` and `b`.
fn in_box(a: Pt, b: Pt, p: Pt) -> bool {
    (a.0.min(b.0)..=a.0.max(b.0)).contains(&p.0) && (a.1.min(b.1)..=a.1.max(b.1)).contains(&p.1)
}

/// True when the closed segments `p1p2` and `p3p4` have a point in common: they cross, touch at
/// an end, or overlap along a line.
fn segments_meet(p1: Pt, p2: Pt, p3: Pt, p4: Pt) -> bool {
    let (d1, d2) = (turn(p3, p4, p1), turn(p3, p4, p2));
    let (d3, d4) = (turn(p1, p2, p3), turn(p1, p2, p4));
    let apart = |a, b| {
        matches!(
            (a, b),
            (Turn::Left, Turn::Right) | (Turn::Right, Turn::Left)
        )
    };
    (apart(d1, d2) && apart(d3, d4))
        || (d1 == Turn::Collinear && in_box(p3, p4, p1))
        || (d2 == Turn::Collinear && in_box(p3, p4, p2))
        || (d3 == Turn::Collinear && in_box(p1, p2, p3))
        || (d4 == Turn::Collinear && in_box(p1, p2, p4))
}

/// The first pair of edges (0-based) of a closed ring that meet anywhere but at the vertex they
/// share: non-adjacent edges that cross or touch, and adjacent edges that double back over each
/// other. `None` for a simple ring. Quadratic, which is fine at [`MAX_POLYGON_VERTICES`].
fn find_self_contact(ring: &[Pt]) -> Option<(usize, usize)> {
    let m = ring.len() - 1;
    let edge = |i: usize| (ring[i], ring[i + 1]);
    for i in 0..m {
        // The next edge (the first, after the last) starts where this one ends.
        let j = (i + 1) % m;
        let (a, b) = edge(i);
        let c = edge(j).1;
        let dot = (b.0 - a.0) * (c.0 - b.0) + (b.1 - a.1) * (c.1 - b.1);
        if turn(a, b, c) == Turn::Collinear && dot < 0.0 {
            return Some((i.min(j), i.max(j)));
        }
    }
    for i in 0..m {
        let (a, b) = edge(i);
        for j in i + 2..m {
            if i == 0 && j == m - 1 {
                continue; // adjacent through the closing vertex
            }
            let (c, d) = edge(j);
            // Segments whose boxes do not overlap cannot meet: settles most pairs cheaply.
            if a.0.max(b.0) < c.0.min(d.0)
                || c.0.max(d.0) < a.0.min(b.0)
                || a.1.max(b.1) < c.1.min(d.1)
                || c.1.max(d.1) < a.1.min(b.1)
            {
                continue;
            }
            if segments_meet(a, b, c, d) {
                return Some((i, j));
            }
        }
    }
    None
}

/// `west,south,east,north`. West greater than east is a box across the antimeridian (as in OGC API
/// Features): `170,50,-170,60` is 20 degrees wide. A box with no width or no height is an error.
pub fn parse_bbox(s: &str) -> Result<[f64; 4], String> {
    let n = nums(s)?;
    if n.len() != 4 {
        return Err("Invalid bbox. Use west,south,east,north".into());
    }
    check_lonlat(n[0], n[1])?;
    check_lonlat(n[2], n[3])?;
    if n[0] == n[2] || n[1] >= n[3] {
        return Err(
            "Invalid bbox: south must be < north and west must not equal east \
             (west > east is a box across the antimeridian)"
                .into(),
        );
    }
    Ok([n[0], n[1], n[2], n[3]])
}

fn parse_instant(s: &str, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, String> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("now") {
        return Ok(Some(now));
    }
    if s == ".." {
        return Ok(None);
    }
    DateTime::parse_from_rfc3339(s)
        .map(|d| Some(d.with_timezone(&Utc)))
        .map_err(|_| format!("Invalid datetime: {s} (use RFC 3339, e.g. 2026-11-01T18:00:00Z)"))
}

/// A parsed `datetime`: nothing, one instant, or an interval (open ends allowed).
#[derive(Debug, PartialEq)]
pub enum When {
    None,
    Instant(DateTime<Utc>),
    Interval(Option<DateTime<Utc>>, Option<DateTime<Utc>>),
}

pub fn parse_when(dt: Option<&str>, now: DateTime<Utc>) -> Result<When, String> {
    let Some(dt) = dt.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(When::None);
    };
    if let Some((a, b)) = dt.split_once('/') {
        let (from, to) = (parse_instant(a, now)?, parse_instant(b, now)?);
        if let (Some(from), Some(to)) = (from, to) {
            if from > to {
                return Err(format!(
                    "Invalid datetime interval: it starts at {} but ends at {} (use from/to)",
                    ts(from),
                    ts(to)
                ));
            }
        }
        return Ok(When::Interval(from, to));
    }
    Ok(match parse_instant(dt, now)? {
        Some(t) => When::Instant(t),
        None => When::None,
    })
}

pub fn csv(s: Option<&str>) -> Vec<String> {
    s.map(|s| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// A `severity=` / `event=` list: [`csv`], but a control character in an item is an error (a NUL
/// cannot be sent to Postgres at all, and none belongs in an event or severity name).
pub fn filter_list(name: &str, s: Option<&str>) -> Result<Vec<String>, String> {
    let items = csv(s);
    if items.iter().any(|t| t.chars().any(char::is_control)) {
        return Err(format!("{name}: control characters are not allowed"));
    }
    Ok(items)
}

/// `limit=`: 1 up to `MAX_ITEMS_LIMIT` (more is cut to it, as documented), default `default`.
/// Zero or less is a mistake, not a request for one alert.
pub fn parse_limit(limit: Option<i64>, default: i64) -> Result<i64, String> {
    match limit {
        None => Ok(default),
        Some(l) if l < 1 => Err(format!("limit must be at least 1 (got {l})")),
        Some(l) => Ok(l.min(MAX_ITEMS_LIMIT)),
    }
}

/// `simplify=` in degrees: `0` (no simplification) or more. Negative and non-finite values are
/// errors rather than another way to say `0`.
pub fn parse_simplify(simplify: Option<f64>) -> Result<Option<f64>, String> {
    match simplify {
        Some(t) if !t.is_finite() || t < 0.0 => Err(format!(
            "simplify must be a number of degrees, 0 or more (got {t})"
        )),
        other => Ok(other),
    }
}

pub fn parse_bool(s: Option<&str>, default: bool) -> Result<bool, String> {
    match s.map(|s| s.trim().to_lowercase()).as_deref() {
        None | Some("") => Ok(default),
        Some("true") | Some("1") | Some("yes") => Ok(true),
        Some("false") | Some("0") | Some("no") => Ok(false),
        Some(other) => Err(format!("Invalid boolean: {other}")),
    }
}

/// Radius in metres from `within` (`50`, `50km`, `30mi`) and `within-units`
/// (default km).
pub fn parse_within(within: Option<&str>, units: Option<&str>) -> Result<f64, String> {
    let w = within
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .ok_or("Missing parameter: within")?;
    let (num, unit) = match w.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => (w[..i].trim().to_string(), w[i..].trim().to_string()),
        None => (
            w.clone(),
            units
                .map(|u| u.trim().to_lowercase())
                .unwrap_or_else(|| "km".into()),
        ),
    };
    let n: f64 = num
        .parse()
        .map_err(|_| "Invalid radius value".to_string())?;
    let meters = match unit.as_str() {
        "km" => n * 1000.0,
        "mi" | "mile" | "miles" => n * 1609.344,
        "m" => n,
        "nm" => n * 1852.0,
        other => return Err(format!("Unsupported within-units: {other} (km, mi, m, nm)")),
    };
    if !meters.is_finite() || meters <= 0.0 || meters > MAX_RADIUS_M {
        return Err(format!(
            "Radius must be between 0 and {} km",
            MAX_RADIUS_M / 1000.0
        ));
    }
    Ok(meters)
}

/// Default simplification tolerance (degrees) for returned geometry: about one pixel of a 1500 px
/// wide view of the bbox (of `/items`, the polygon of `/area`, the circle of `/radius`), capped at
/// ~5 km.
pub fn default_simplify(bbox: &[f64; 4]) -> f64 {
    // west > east is a box across the antimeridian
    let width = match bbox[2] - bbox[0] {
        w if w < 0.0 => w + 360.0,
        w => w,
    };
    (width.max(bbox[3] - bbox[1]) / 1500.0).clamp(0.0, 0.05)
}

// =============================================================================
// Responses
// =============================================================================

fn error(status: StatusCode, exc: ExceptionResponse) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_string(&exc).unwrap_or_default().into())
        .unwrap()
}

fn bad_request(msg: impl Into<String>) -> Response {
    error(
        StatusCode::BAD_REQUEST,
        ExceptionResponse::bad_request(msg.into()),
    )
}

fn internal(msg: impl std::fmt::Display) -> Response {
    tracing::error!("nws-alerts: {msg}");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        ExceptionResponse::internal_error("Internal error"),
    )
}

/// A catalog failure as a response. A read that did not finish within its time limit
/// (`NWS_ALERTS_QUERY_TIMEOUT_MS`) is a 503 and only a warning: the query may have been too
/// expensive, or the database busy (the limit counts the time spent waiting for a lock too), so
/// the answer does not say a retry is pointless. Anything else is a fault.
fn catalog_failure(e: WmsError) -> Response {
    match e {
        WmsError::ServiceUnavailable(msg) => {
            tracing::warn!("nws-alerts: {msg}");
            let mut r = error(
                StatusCode::SERVICE_UNAVAILABLE,
                ExceptionResponse::service_unavailable(
                    "The alerts query did not finish within the server's time limit (the \
                     database was busy, or the query was too expensive); try again shortly, and \
                     for a large /area, /radius or /items ask for a smaller area",
                ),
            );
            r.headers_mut()
                .insert(header::RETRY_AFTER, header::HeaderValue::from_static("5"));
            r
        }
        other => internal(other),
    }
}

/// The alerts read time limit from `NWS_ALERTS_QUERY_TIMEOUT_MS`: a positive number of
/// milliseconds; unset, zero or unreadable means [`DEFAULT_QUERY_TIMEOUT_MS`].
pub fn query_timeout(env_value: Option<&str>) -> std::time::Duration {
    let ms = match env_value.map(str::trim).filter(|v| !v.is_empty()) {
        None => DEFAULT_QUERY_TIMEOUT_MS,
        Some(v) => match v.parse::<u64>() {
            Ok(ms) if ms > 0 => ms,
            _ => {
                tracing::warn!(
                    "NWS_ALERTS_QUERY_TIMEOUT_MS={v:?} is not a positive number of milliseconds; \
                     using {DEFAULT_QUERY_TIMEOUT_MS}"
                );
                DEFAULT_QUERY_TIMEOUT_MS
            }
        },
    };
    std::time::Duration::from_millis(ms)
}

fn geojson<T: Serialize>(body: &T) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/geo+json")
        .header(header::CACHE_CONTROL, CACHE_CONTROL)
        .body(serde_json::to_string(body).unwrap_or_default().into())
        .unwrap()
}

/// Parse the raw query string of a generic position/area/radius request.
#[allow(clippy::result_large_err)]
pub fn query_from_raw(raw: Option<&str>) -> Result<NwsQuery, Response> {
    let uri: axum::http::Uri = format!("/?{}", raw.unwrap_or(""))
        .parse()
        .map_err(|_| bad_request("Malformed query string"))?;
    Query::<NwsQuery>::try_from_uri(&uri)
        .map(|q| q.0)
        .map_err(|e| bad_request(e.body_text()))
}

/// The alerts are GeoJSON and nothing else: `f=` may name GeoJSON (or plain `json`, which GeoJSON
/// is), and without `f` the `Accept` header must admit `application/geo+json` or `application/json`
/// (`*/*` does). Asking for CoverageJSON, PNG or anything else is refused rather than answered
/// with GeoJSON the client then parses as the format it asked for: an unknown `f` is a 400, as on
/// the grid collections, and an `Accept` that excludes GeoJSON a 406.
#[allow(clippy::result_large_err)]
pub fn check_format(headers: &HeaderMap, f: Option<&str>) -> Result<(), Response> {
    let Some(f) = f.map(str::trim).filter(|f| !f.is_empty()) else {
        return check_accept_header(headers, &["application/geo+json", "application/json"]);
    };
    // (an unencoded `+` in `application/geo+json` arrives as a space)
    let wanted = f.to_ascii_lowercase().replace(' ', "+");
    match wanted.as_str() {
        "geojson" | "geo+json" | "application/geo+json" | "json" | "application/json" => Ok(()),
        _ => Err(bad_request(format!(
            "Unsupported output format '{f}': nws-alerts is served as GeoJSON \
             (application/geo+json) only"
        ))),
    }
}

/// The query string of an `nws-alerts` request, read as an [`NwsQuery`], and its output format
/// checked: the response to send instead when either is wrong.
#[allow(clippy::result_large_err)]
fn request_query(raw: Option<&str>, headers: &HeaderMap) -> Result<NwsQuery, Response> {
    let q = query_from_raw(raw)?;
    check_format(headers, q.f.as_deref())?;
    Ok(q)
}

/// `GET /collections/:collection_id/position`: alerts at the point for
/// `nws-alerts`, otherwise the grid position query.
///
/// The generic parameters are taken as a `Result` so that a query string `nws-alerts` can read
/// but the grid struct cannot (`limit=abc`, a duplicated `coords`) reaches [`query_from_raw`] and
/// its JSON error, instead of axum's plain-text rejection. Other collections still get that.
pub async fn position_route(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    params: Result<Query<super::position::PositionQueryParams>, QueryRejection>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    if is_nws_collection(&state, &collection_id).await {
        return match request_query(raw.as_deref(), &headers) {
            Ok(q) => position(state, q).await,
            Err(r) => r,
        };
    }
    match params {
        Ok(params) => {
            super::position::position_handler(
                Extension(state),
                Path(collection_id),
                params,
                headers,
            )
            .await
        }
        Err(rejection) => rejection.into_response(),
    }
}

/// `GET /collections/:collection_id/area`, `nws-alerts` aware.
pub async fn area_route(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    params: Result<Query<super::area::AreaQueryParams>, QueryRejection>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    if is_nws_collection(&state, &collection_id).await {
        return match request_query(raw.as_deref(), &headers) {
            Ok(q) => area(state, q).await,
            Err(r) => r,
        };
    }
    match params {
        Ok(params) => {
            super::area::area_handler(Extension(state), Path(collection_id), params, headers).await
        }
        Err(rejection) => rejection.into_response(),
    }
}

/// `GET /collections/:collection_id/radius`, `nws-alerts` aware.
pub async fn radius_route(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    params: Result<Query<super::radius::RadiusQueryParams>, QueryRejection>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    if is_nws_collection(&state, &collection_id).await {
        return match request_query(raw.as_deref(), &headers) {
            Ok(q) => radius(state, q).await,
            Err(r) => r,
        };
    }
    match params {
        Ok(params) => {
            super::radius::radius_handler(Extension(state), Path(collection_id), params, headers)
                .await
        }
        Err(rejection) => rejection.into_response(),
    }
}

/// `GET /collections/:collection_id/items`, `nws-alerts` aware (see [`position_route`]).
pub async fn items_route(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    params: Result<Query<super::items::ItemsQueryParams>, QueryRejection>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    if is_nws_collection(&state, &collection_id).await {
        return match request_query(raw.as_deref(), &headers) {
            Ok(q) => items(state, q).await,
            Err(r) => r,
        };
    }
    match params {
        Ok(params) => {
            super::items::items_handler(Extension(state), Path(collection_id), params).await
        }
        Err(rejection) => rejection.into_response(),
    }
}

/// True when `collection_id` is configured with `observation_source: nws_alerts`.
pub async fn is_nws_collection(state: &AppState, collection_id: &str) -> bool {
    let config = state.edr_config.read().await;
    config
        .find_collection(collection_id)
        .is_some_and(|(m, _)| m.observation_source.as_deref() == Some(OBSERVATION_SOURCE))
}

async fn meta(state: &AppState, now: DateTime<Utc>) -> Value {
    let status = state.nws_alert_catalog.status().await.ok().flatten();
    feed_meta(status.as_ref(), now)
}

/// How many bytes of geometry one list answer may carry: the collection's
/// `limits.max_response_size_mb` (`config/edr/nws-alerts.yaml`).
async fn geometry_budget(state: &AppState) -> usize {
    let config = state.edr_config.read().await;
    let mb = config
        .models
        .values()
        .find(|m| m.observation_source.as_deref() == Some(OBSERVATION_SOURCE))
        .map_or(LimitsConfig::default().max_response_size_mb, |m| {
            m.limits.max_response_size_mb
        });
    mb.saturating_mul(1024 * 1024)
}

/// Run a search and build the FeatureCollection response. The geometry of the answer is held to
/// [`geometry_budget`]: the alerts beyond it are left out and `truncated` says so, whatever
/// `limit` is, so no request (`simplify=0` over a year of history) can make the service build an
/// answer of any size.
async fn respond(state: &AppState, search: AlertSearch, extra: Vec<(&str, Value)>) -> Response {
    let wall = Utc::now();
    let limit = search.limit;
    let budget = geometry_budget(state).await;
    let mut rows = match state
        .nws_alert_catalog
        .search(&AlertSearch {
            limit: limit + 1,
            max_geometry_bytes: Some(budget),
            ..search.clone()
        })
        .await
    {
        Ok(r) => r,
        Err(e) => return catalog_failure(e),
    };
    let (fit, over_budget) = within_budget(&rows, budget);
    let truncated = over_budget || rows.len() as i64 > limit;
    rows.truncate(fit.min(limit as usize));
    let features = rows
        .iter()
        .map(|a| list_feature(a, search.now, &state.base_url))
        .collect();
    let mut fc = collection(features, wall, meta(state, wall).await);
    for (k, v) in extra {
        fc.extra.insert(k.to_string(), v);
    }
    if truncated {
        fc.extra.insert("truncated".to_string(), json!(true));
    }
    geojson(&fc)
}

/// `include_non_actual=true|false` (default false).
pub fn parse_include_non_actual(q: &NwsQuery) -> Result<bool, String> {
    parse_bool(q.include_non_actual.as_deref(), false)
        .map_err(|e| format!("include_non_actual: {e}"))
}

fn base_search(
    region: AlertRegion,
    q: &NwsQuery,
    now: DateTime<Utc>,
    limit: i64,
    geometry: bool,
    include_non_actual: bool,
) -> Result<AlertSearch, String> {
    Ok(AlertSearch {
        region,
        now,
        window: None,
        include_superseded: false,
        severities: filter_list("severity", q.severity.as_deref())?,
        events: filter_list("event", q.event.as_deref())?,
        limit,
        with_geometry: geometry,
        simplify: parse_simplify(q.simplify)?,
        include_non_actual,
        max_geometry_bytes: None,
    })
}

/// Scope an "active at" search to an explicit `datetime`: the alerts in force at that instant,
/// each labelled as of it. No instant leaves the live set at the wall clock, answered from the
/// lifecycle flags on the indexed fast path.
fn at_instant(search: AlertSearch, instant: Option<DateTime<Utc>>) -> AlertSearch {
    match instant {
        Some(t) => search.in_force_at(t),
        None => search,
    }
}

/// The instant of an instant-or-nothing `datetime` for the "active at" queries; `None` is the
/// live set at the wall clock.
fn active_time(q: &NwsQuery) -> Result<Option<DateTime<Utc>>, String> {
    let wall = Utc::now();
    match parse_when(q.datetime.as_deref(), wall)? {
        When::None => Ok(None),
        // `datetime=now` is the wall clock, not an instant of its own
        When::Instant(t) if t == wall => Ok(None),
        When::Instant(t) => Ok(Some(t)),
        When::Interval(..) => Err(
            "datetime must be a single instant here (alerts active at that time), not an interval"
                .into(),
        ),
    }
}

// =============================================================================
// Handlers
// =============================================================================

/// `GET /collections/nws-alerts/position?coords=POINT(lon lat)`: alerts in force at a point.
pub async fn position(state: Arc<AppState>, q: NwsQuery) -> Response {
    let (lon, lat) = match parse_point(q.coords.as_deref().unwrap_or("")) {
        Ok(p) => p,
        Err(e) => return bad_request(e),
    };
    let at = match active_time(&q) {
        Ok(t) => t,
        Err(e) => return bad_request(e),
    };
    let geometry = match parse_bool(q.geometry.as_deref(), false) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let include_non_actual = match parse_include_non_actual(&q) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let search = match base_search(
        AlertRegion::Point { lon, lat },
        &q,
        Utc::now(),
        NEARBY_LIMIT,
        geometry,
        include_non_actual,
    ) {
        Ok(s) => s,
        Err(e) => return bad_request(e),
    };
    respond(&state, at_instant(search, at), vec![]).await
}

/// `GET /collections/nws-alerts/area?coords=POLYGON((...))`.
pub async fn area(state: Arc<AppState>, q: NwsQuery) -> Response {
    let (wkt, bbox) = match parse_polygon_with_bbox(q.coords.as_deref().unwrap_or("")) {
        Ok(p) => p,
        Err(e) => return bad_request(e),
    };
    nearby(
        state,
        q,
        AlertRegion::Polygon { wkt },
        default_simplify(&bbox),
    )
    .await
}

/// `GET /collections/nws-alerts/radius?coords=POINT(lon lat)&within=50&within-units=km`.
pub async fn radius(state: Arc<AppState>, q: NwsQuery) -> Response {
    let (lon, lat) = match parse_point(q.coords.as_deref().unwrap_or("")) {
        Ok(p) => p,
        Err(e) => return bad_request(e),
    };
    let meters = match parse_within(q.within.as_deref(), q.within_units.as_deref()) {
        Ok(m) => m,
        Err(e) => return bad_request(e),
    };
    // the circle's extent: what `/items` would call its bbox
    let boxes = radius_boxes(lon, lat, meters);
    let width: f64 = boxes.iter().map(|b| b[2] - b[0]).sum();
    let extent = [0.0, boxes[0][1], width, boxes[0][3]];
    nearby(
        state,
        q,
        AlertRegion::Radius { lon, lat, meters },
        default_simplify(&extent),
    )
    .await
}

/// `/area` and `/radius`: `default_tolerance` is the geometry simplification when the request gives
/// no `simplify` (it only matters with `geometry=true`), scaled by the query's extent like `/items`.
async fn nearby(
    state: Arc<AppState>,
    q: NwsQuery,
    region: AlertRegion,
    default_tolerance: f64,
) -> Response {
    let at = match active_time(&q) {
        Ok(t) => t,
        Err(e) => return bad_request(e),
    };
    let geometry = match parse_bool(q.geometry.as_deref(), false) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let include_non_actual = match parse_include_non_actual(&q) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let limit = match parse_limit(q.limit, DEFAULT_ITEMS_LIMIT) {
        Ok(l) => l,
        Err(e) => return bad_request(e),
    };
    let mut search = match base_search(region, &q, Utc::now(), limit, geometry, include_non_actual)
    {
        Ok(s) => s,
        Err(e) => return bad_request(e),
    };
    search.simplify = Some(search.simplify.unwrap_or(default_tolerance));
    respond(
        &state,
        at_instant(search, at),
        vec![("limit", json!(limit))],
    )
    .await
}

/// `GET /collections/nws-alerts/items`: alerts in a bbox (default: everywhere).
pub async fn items(state: Arc<AppState>, q: NwsQuery) -> Response {
    let bbox = match q.bbox.as_deref().filter(|b| !b.trim().is_empty()) {
        Some(b) => match parse_bbox(b) {
            Ok(b) => b,
            Err(e) => return bad_request(e),
        },
        None => [-180.0, -90.0, 180.0, 90.0],
    };
    let geometry = match parse_bool(q.geometry.as_deref(), true) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let include_superseded = match parse_bool(q.include_superseded.as_deref(), false) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let include_non_actual = match parse_include_non_actual(&q) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let all = match q
        .status
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .as_deref()
    {
        None | Some("") | Some("active") => false,
        Some("all") => true,
        Some(other) => return bad_request(format!("Invalid status: {other} (active or all)")),
    };
    let wall = Utc::now();
    let when = match parse_when(q.datetime.as_deref(), wall) {
        Ok(w) => w,
        Err(e) => return bad_request(e),
    };
    let limit = match parse_limit(q.limit, DEFAULT_ITEMS_LIMIT) {
        Ok(l) => l,
        Err(e) => return bad_request(e),
    };

    let mut search = match base_search(
        AlertRegion::Bbox(bbox),
        &q,
        wall,
        limit,
        geometry,
        include_non_actual,
    ) {
        Ok(s) => s,
        Err(e) => return bad_request(e),
    };
    search.simplify = Some(search.simplify.unwrap_or_else(|| default_simplify(&bbox)));
    match (all, when) {
        (false, When::None) => {}
        // `datetime=now` is the wall clock, not an instant of its own
        (false, When::Instant(t)) if t == wall => {}
        // in force then, whatever became of it since (include_superseded does not apply)
        (false, When::Instant(t)) => search = search.in_force_at(t),
        (false, When::Interval(..)) => {
            return bad_request(
                "status=active takes a single instant; use status=all for an interval",
            )
        }
        (true, when) => {
            let (now, window) = match when {
                When::None => (wall, (wall - Duration::hours(24), wall)),
                When::Instant(t) => (t, (t, t)),
                When::Interval(a, b) => (
                    wall,
                    (
                        a.unwrap_or(wall - Duration::days(365)),
                        b.unwrap_or(wall + Duration::days(365)),
                    ),
                ),
            };
            search.now = now;
            search.window = Some(window);
            search.include_superseded = include_superseded;
        }
    }
    respond(&state, search, vec![("limit", json!(limit))]).await
}

/// `GET /collections/nws-alerts/items/{id}`: one alert in full, in any state.
pub async fn item(state: Arc<AppState>, id: String, q: NwsQuery) -> Response {
    // (a NUL, which no id has, cannot be sent to Postgres at all)
    if id.chars().any(char::is_control) {
        return bad_request("Invalid alert id: control characters are not allowed");
    }
    let geometry = match parse_bool(q.geometry.as_deref(), true) {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    match state.nws_alert_catalog.get(&id, geometry).await {
        Ok(Some(a)) => {
            let wall = Utc::now();
            let mut f = full_feature(&a, wall, &state.base_url);
            f["meta"] = meta(&state, wall).await;
            geojson(&f)
        }
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!("Alert not found: {id}")),
        ),
        Err(e) => catalog_failure(e),
    }
}

/// Route handler for `/collections/:collection_id/items/:feature_id`.
///
/// The path is taken as a `Result` because an id that is not UTF-8 once unescaped (`%FF`) is
/// rejected by axum before the handler runs, as a plain-text 400; here it is the JSON one.
pub async fn item_route_handler(
    Extension(state): Extension<Arc<AppState>>,
    path: Result<Path<(String, String)>, PathRejection>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Path((collection_id, feature_id)) = match path {
        Ok(p) => p,
        Err(rejection) => return bad_request(rejection.body_text()),
    };
    if !is_nws_collection(&state, &collection_id).await {
        return error(
            StatusCode::NOT_FOUND,
            ExceptionResponse::not_found(format!(
                "Collection {collection_id} has no addressable items"
            )),
        );
    }
    match request_query(raw.as_deref(), &headers) {
        Ok(q) => item(state, feature_id, q).await,
        Err(r) => r,
    }
}

/// The data-age sentence of the collection description, from [`feed_meta`]'s block.
fn data_age_sentence(m: &Value) -> String {
    match m["data_age_seconds"].as_i64() {
        Some(s) if !m["stale"].as_bool().unwrap_or(true) => format!("Data age {s} s."),
        Some(s) => format!("STALE: last successful poll of api.weather.gov was {s} s ago."),
        None => "STALE: no successful poll of api.weather.gov yet.".to_string(),
    }
}

/// The `nws-alerts` collection document: position/area/radius queries, an `items` link, extent
/// and, with `live_age`, a data-age sentence in the description. `items` and `items/{id}` are
/// served by dedicated routes and described in the description, like the other feature
/// collections. Everything is GeoJSON, and the document says so.
///
/// `live_age` is for the document served per request. The `/collections` list is cached, and a
/// relative age ("Data age 64 s") in a cached body is wrong by the time anyone reads it, in either
/// direction (it can claim fresh data through an outage), so the list passes `false`: the age lives
/// in this document and in every response's `meta`.
pub async fn collection_metadata(
    state: &AppState,
    def_id: &str,
    title: &str,
    description: &str,
    live_age: bool,
) -> Collection {
    let mut description = format!(
        "{description} Also /items?bbox=&datetime=&status=active|all and /items/{{id}} \
         (URN, URL-encoded or raw). Filters: severity=, event= (comma lists), geometry=true|false, \
         include_non_actual=true (also Test/Exercise/System/Draft messages; default Actual only)."
    );
    if live_age {
        let status = state.nws_alert_catalog.status().await.ok().flatten();
        let m = feed_meta(status.as_ref(), Utc::now());
        description = format!("{description} {}", data_age_sentence(&m));
    }
    let mut c = Collection::new(def_id)
        .with_title(title)
        .with_description(&description)
        .with_output_formats(geojson_output_formats());
    c.build_items_links(&state.base_url);
    c = c.with_data_queries(
        DataQueries::with_position(&state.base_url, def_id)
            .with_area(&state.base_url, def_id)
            .with_radius(&state.base_url, def_id)
            .geojson_only(),
    );
    // US-centred but includes the Pacific/Alaska/territory zones NWS covers.
    c = c.with_extent(Extent::with_spatial([-180.0, -15.0, 180.0, 72.0], None));
    let mut params = std::collections::HashMap::new();
    for (k, d) in [
        (
            "severity",
            "Alert severity: Extreme, Severe, Moderate, Minor, Unknown",
        ),
        ("event", "NWS event name, e.g. Tornado Warning"),
    ] {
        params.insert(k.to_string(), Parameter::new(k, d));
    }
    c.with_parameters(params)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use storage::nws_alerts::{parse_feature, ParsedAlert};

    const FIXTURE: &str = include_str!("../../../downloader/tests/fixtures/nws_alerts_active.json");

    fn parsed() -> Vec<ParsedAlert> {
        let v: Value = serde_json::from_str(FIXTURE).unwrap();
        v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| parse_feature(f).unwrap())
            .collect()
    }

    /// Alerts saved from the live feed whose text has no WHAT or HAZARD: Alaska coastal-water
    /// forecasts and free-text statements.
    const NO_LABELS: &str =
        include_str!("../../../downloader/tests/fixtures/nws_alerts_no_labels.json");

    fn no_labels(id_ends_with: &str) -> StoredAlert {
        let v: Value = serde_json::from_str(NO_LABELS).unwrap();
        let f = v["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| {
                f["properties"]["id"]
                    .as_str()
                    .unwrap()
                    .ends_with(id_ends_with)
            })
            .unwrap();
        stored(&parse_feature(f).unwrap())
    }

    /// Build the row the catalog would return for a parsed alert.
    fn stored(p: &ParsedAlert) -> StoredAlert {
        let now = Utc.with_ymd_and_hms(2026, 11, 1, 23, 30, 0).unwrap();
        StoredAlert {
            id: p.id.clone(),
            event: p.event.clone(),
            severity: p.severity.clone(),
            urgency: p.urgency.clone(),
            certainty: p.certainty.clone(),
            category: p.category.clone(),
            status: p.status.clone(),
            message_type: p.message_type.clone(),
            response: p.response.clone(),
            headline: p.headline.clone(),
            nws_headline: p.nws_headline.clone(),
            description: p.description.clone(),
            instruction: p.instruction.clone(),
            description_text: p.description_text.clone(),
            instruction_text: p.instruction_text.clone(),
            area_desc: p.area_desc.clone(),
            sender_name: p.sender_name.clone(),
            web: p.web.clone(),
            sent: p.sent,
            effective: p.effective,
            onset: p.onset,
            expires: p.expires,
            ends: p.ends,
            end_time: p.ends.or(p.expires),
            ugc: p.ugc.clone(),
            zone_keys: p.zone_keys.clone(),
            is_marine: p.is_marine,
            vtec: serde_json::to_value(&p.vtec).unwrap(),
            vtec_event_key: p.vtec.first().map(|v| v.event_key()),
            sections: p.sections.clone(),
            parameters: p.parameters.clone(),
            color: p.color.clone(),
            references_ids: p.references.clone(),
            superseded_by: None,
            superseded_at: None,
            ended_at: None,
            geometry_source: "polygon".into(),
            first_seen_at: now,
            updated_at: now,
            last_seen_at: now,
            geometry_geojson: p.geometry_json.clone(),
        }
    }

    fn by_event(event: &str) -> StoredAlert {
        stored(parsed().iter().find(|a| a.event == event).unwrap())
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, 23, 30, 0).unwrap()
    }

    #[test]
    fn ids_are_percent_encoded_as_one_segment() {
        assert_eq!(
            encode_id("urn:oid:2.49.0.1.840.0.abc.001.1"),
            "urn%3Aoid%3A2.49.0.1.840.0.abc.001.1"
        );
        assert_eq!(encode_id("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn trimming_caps_at_a_word_and_marks_it() {
        assert_eq!(trim_text("  short  ", 20), "short");
        let t = trim_text("alpha beta gamma delta epsilon", 17);
        assert_eq!(t, "alpha beta gamma…");
        assert!(trim_text(&"x".repeat(50), 10).chars().count() <= 11);
        // multi-byte text never panics
        assert!(trim_text(&"é".repeat(30), 10).ends_with('…'));
    }

    #[test]
    fn compact_properties_carry_what_the_card_needs() {
        let a = by_event("Severe Thunderstorm Warning");
        let f = compact_feature(&a, t0(), "https://x.test/edr");
        assert_eq!(f["id"], a.id);
        assert!(
            f["geometry"].is_object(),
            "fixture alert carries its polygon"
        );
        let p = &f["properties"];
        assert_eq!(p["event"], "Severe Thunderstorm Warning");
        assert_eq!(p["severity"], "Severe");
        assert_eq!(p["color"], "#FFA500");
        assert_eq!(p["significance"], "warning");
        assert_eq!(p["action"], "NEW");
        assert_eq!(p["vtec_event"], "KABQ.SV.W.0428");
        assert_eq!(p["status"], "active");
        assert_eq!(p["summary"]["hazard"], "60 mph wind gusts.");
        assert_eq!(
            p["formal_headline"], p["headline"],
            "this one has no broadcast text: both are NWS' own"
        );
        assert_eq!(p["ends"], "2026-11-02T00:00:00Z");
        assert_eq!(p["expires"], "2026-11-02T00:00:00Z");
        assert!(p["instruction"].as_str().unwrap().len() <= INSTRUCTION_MAX + 3);
        assert_eq!(
            p["links"][0]["href"],
            format!(
                "https://x.test/edr/collections/nws-alerts/items/{}",
                encode_id(&a.id)
            )
        );
        assert_eq!(p["links"][1]["rel"], "alternate");
        // compact leaves the heavy fields out
        for k in [
            "description",
            "sections",
            "vtec",
            "parameters",
            "ugc",
            "references",
        ] {
            assert!(p.get(k).is_none(), "{k} must not be in the compact shape");
        }
    }

    #[test]
    fn ends_is_what_nws_gave_and_null_when_it_gave_none() {
        // The avalanche advisories have no `ends`, only an `expires`.
        let a = by_event("Avalanche Advisory");
        assert!(a.ends.is_none());
        let expires = a.expires.unwrap();
        for f in [compact_feature(&a, t0(), ""), full_feature(&a, t0(), "")] {
            let p = &f["properties"];
            assert!(p["ends"].is_null(), "not the message expiry: {}", p["ends"]);
            assert_eq!(p["expires"], ts(expires));
        }
        // the lifecycle still ends it at `expires` ("ends, else expires")
        assert_eq!(alert_status(&a, expires - Duration::seconds(1)), "active");
        assert_eq!(alert_status(&a, expires), "ended");
        // and an alert that has an `ends` reports it, whatever `expires` says
        let mut b = by_event("Severe Thunderstorm Warning");
        b.expires = Some(b.ends.unwrap() + Duration::hours(1));
        let p = compact_feature(&b, t0(), "")["properties"].clone();
        assert_eq!(p["ends"], ts(b.ends.unwrap()));
        assert_eq!(p["expires"], ts(b.expires.unwrap()));
    }

    #[test]
    fn cap_status_is_exposed_next_to_the_lifecycle_status() {
        let a = by_event("Severe Thunderstorm Warning");
        let f = compact_feature(&a, t0(), "");
        assert_eq!(f["properties"]["cap_status"], "Actual");
        assert_eq!(
            f["properties"]["status"], "active",
            "lifecycle status is unchanged"
        );
        let t = by_event("Test Message");
        assert_eq!(t.status.as_deref(), Some("Test"));
        for f in [compact_feature(&t, t0(), ""), full_feature(&t, t0(), "")] {
            assert_eq!(f["properties"]["cap_status"], "Test");
        }
        let mut none = by_event("Flood Watch");
        none.status = None;
        assert!(compact_feature(&none, t0(), "")["properties"]["cap_status"].is_null());
    }

    #[test]
    fn include_non_actual_parses_as_a_boolean_defaulting_to_off() {
        let q = |v: Option<&str>| NwsQuery {
            include_non_actual: v.map(String::from),
            ..Default::default()
        };
        assert!(!parse_include_non_actual(&q(None)).unwrap());
        assert!(!parse_include_non_actual(&q(Some("false"))).unwrap());
        assert!(parse_include_non_actual(&q(Some("true"))).unwrap());
        assert!(parse_include_non_actual(&q(Some("TRUE"))).unwrap());
        assert!(parse_include_non_actual(&q(Some("maybe"))).is_err());
        // and it comes through the raw query string the position/area/radius routes parse
        let parsed = query_from_raw(Some("coords=1,2&include_non_actual=true")).unwrap();
        assert!(parse_include_non_actual(&parsed).unwrap());
    }

    #[test]
    fn headline_prefers_the_nws_headline() {
        let mut a = by_event("Severe Thunderstorm Warning");
        a.nws_headline = Some("NWS says".into());
        a.headline = Some("Long headline".into());
        assert_eq!(headline(&a), Some("NWS says"));
        a.nws_headline = Some("  ".into());
        assert_eq!(headline(&a), Some("Long headline"));
        a.nws_headline = None;
        a.headline = None;
        assert_eq!(headline(&a), None);
    }

    #[test]
    fn the_formal_headline_comes_with_the_card_headline_in_every_answer() {
        let mut a = by_event("Flash Flood Warning");
        a.headline = Some("Flood Warning issued November 1 at 7:26PM MST by NWS Pueblo CO".into());
        a.nws_headline = Some("FLOOD WARNING IN EFFECT UNTIL 730 PM MST MONDAY".into());
        let p = |f: Value| f["properties"].clone();
        let (compact, full) = (
            p(compact_feature(&a, t0(), "")),
            p(full_feature(&a, t0(), "")),
        );
        assert_eq!(
            compact["headline"],
            "FLOOD WARNING IN EFFECT UNTIL 730 PM MST MONDAY"
        );
        assert_eq!(full["headline"], compact["headline"]);
        for card in [&compact, &full] {
            assert_eq!(
                card["formal_headline"],
                "Flood Warning issued November 1 at 7:26PM MST by NWS Pueblo CO",
                "in the list as well as the full record"
            );
        }
        // without broadcast text the two are the same
        a.nws_headline = None;
        let full = p(full_feature(&a, t0(), ""));
        assert_eq!(full["headline"], full["formal_headline"]);
        a.headline = None;
        assert!(p(full_feature(&a, t0(), ""))["formal_headline"].is_null());
    }

    #[test]
    fn an_alaska_forecast_alert_says_the_period_its_headline_names() {
        // GALE WARNING TONIGHT AND TUESDAY
        let a = no_labels("4ed4e123.003.1");
        assert_eq!(headline(&a), Some("GALE WARNING TONIGHT AND TUESDAY"));
        let want = "Tonight: SE wind 25 kt becoming E 35 kt after midnight. Seas 9 ft. Rain. Tue: E wind 45 kt. Seas 18 ft. Rain.";
        let c = compact_feature(&a, t0(), "");
        assert_eq!(c["properties"]["summary"], json!({ "what": want }));
        assert!(c["properties"].get("sections").is_none());
        let f = full_feature(&a, t0(), "");
        assert_eq!(f["properties"]["summary"], json!({ "what": want }));
        // the periods are in the full record, one per line in the description
        let periods = f["properties"]["sections"]["periods"].as_array().unwrap();
        assert_eq!(periods.len(), 7);
        assert_eq!(periods[1]["name"], "TONIGHT");
        assert_eq!(
            f["properties"]["description"]
                .as_str()
                .unwrap()
                .lines()
                .filter(|l| l.starts_with('.'))
                .count(),
            7
        );

        // no broadcast text (NWS Juneau): the formal headline names no day, so the first period
        let a = no_labels("4da2086d58a5449608c1.002.1");
        assert!(a.nws_headline.is_none());
        let c = compact_feature(&a, t0(), "");
        assert_eq!(
            c["properties"]["summary"],
            json!({ "what": "Rest of today: SW wind 25 kt. Seas 16 ft." })
        );

        // The same Juneau advisory read on Monday morning (07:07 AKST), a day after it was sent
        // (Sunday 12:12 AKST): `Rest of today` and `Tonight` were Sunday's and are over, what is
        // in force is Monday's.
        let monday = Utc.with_ymd_and_hms(2026, 11, 2, 16, 7, 0).unwrap();
        for f in [
            compact_feature(&a, monday, ""),
            full_feature(&a, monday, ""),
        ] {
            assert_eq!(
                f["properties"]["summary"],
                json!({ "what": "Mon: S wind 10 kt. Seas 9 ft. SW swell." })
            );
        }
        // and with the zone the office is in not to be had (no formal headline), as NWS wrote it
        let mut b = a.clone();
        b.headline = None;
        assert_eq!(
            compact_feature(&b, monday, "")["properties"]["summary"],
            json!({ "what": "Rest of today: SW wind 25 kt. Seas 16 ft." })
        );

        // a long run of periods is trimmed like any other summary text in the list, not in full
        let mut a = no_labels("4ed4e123.003.1");
        a.nws_headline = Some("GALE WARNING TODAY AND TONIGHT AND TUESDAY AND WEDNESDAY".into());
        a.sections["periods"] = json!((0..30)
            .map(|i| json!({"name": if i % 2 == 0 { "TODAY" } else { "TONIGHT" }, "text": format!("Wind {i} kt.")}))
            .collect::<Vec<_>>());
        let what = |f: Value| {
            f["properties"]["summary"]["what"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(what(compact_feature(&a, t0(), "")).ends_with('…'));
        assert!(what(full_feature(&a, t0(), "")).ends_with("Tonight: Wind 29 kt."));
    }

    #[test]
    fn an_alaska_forecast_sent_just_after_midnight_dates_its_tonight_to_the_night_before() {
        // NWS Juneau sent Small Craft Advisories on Wednesday 2026-10-28 at 00:28 AKDT with TONIGHT
        // first, the night that began on Tuesday evening, and WED after it
        let mut a = no_labels("4da2086d58a5449608c1.002.1");
        a.sent = Some(Utc.with_ymd_and_hms(2026, 10, 28, 8, 28, 0).unwrap());
        a.headline = Some("Small Craft Advisory issued October 28 at 12:28AM AKDT until October 28 at 12:00PM AKDT by NWS Juneau AK".into());
        a.nws_headline = Some("SMALL CRAFT ADVISORY THROUGH WEDNESDAY MORNING".into());
        a.sections = json!({"periods": [
            {"name": "TONIGHT", "text": "N wind 25 kt. Seas 11 ft."},
            {"name": "WED", "text": "NE wind 20 kt. Seas 8 ft."},
            {"name": "WED NIGHT", "text": "NE wind 15 kt. Seas 6 ft."},
            {"name": "THU", "text": "E wind 10 kt. Seas 5 ft."},
        ]});
        let what = |now: DateTime<Utc>| {
            [compact_feature(&a, now, ""), full_feature(&a, now, "")].map(|f| {
                f["properties"]["summary"]["what"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
        };
        // Wednesday 00:30 AKDT: Tuesday night is in force, with Wednesday after it
        let want = "Tue night: N wind 25 kt. Seas 11 ft. Wed: NE wind 20 kt. Seas 8 ft.";
        assert_eq!(
            what(Utc.with_ymd_and_hms(2026, 10, 28, 8, 30, 0).unwrap()),
            [want, want]
        );
        // Wednesday 19:00 and Thursday 01:00 AKDT: Wednesday night, never TONIGHT as Wednesday's
        for now in [(10, 29, 3), (10, 29, 9)] {
            let now = Utc
                .with_ymd_and_hms(2026, now.0, now.1, now.2, 0, 0)
                .unwrap();
            let want = "Wed night: NE wind 15 kt. Seas 6 ft.";
            assert_eq!(what(now), [want, want]);
        }
    }

    #[test]
    fn a_free_text_statement_says_its_first_sentences_and_labels_win() {
        let a = no_labels("6e4557.001.1"); // NWS Pendleton, Special Weather Statement
        let c = compact_feature(&a, t0(), "");
        let what = c["properties"]["summary"]["what"].as_str().unwrap();
        assert!(what.starts_with("Patchy dense fog has been observed around the Tri Cities"));
        assert!(what.ends_with("persist through mid morning."));
        assert!(what.chars().count() <= 200, "{what}");
        assert_eq!(c["properties"]["summary"].as_object().unwrap().len(), 1);

        // NWS' own WHAT or HAZARD is never replaced, and an alert with other labels gets nothing
        let mut a = by_event("Winter Storm Warning");
        let own = a.sections["what"].clone();
        a.sections["periods"] = json!([{"name": "TODAY", "text": "Rain."}]);
        let c = compact_feature(&a, t0(), "");
        assert_eq!(c["properties"]["summary"]["what"], own);
        let mut a = by_event("Winter Storm Warning");
        a.sections = json!({"where": "Here.", "narrative": ["A sentence."]});
        let c = compact_feature(&a, t0(), "");
        assert_eq!(c["properties"]["summary"], json!({ "where": "Here." }));
        let a = by_event("Severe Thunderstorm Warning");
        let c = compact_feature(&a, t0(), "");
        assert!(c["properties"]["summary"].get("what").is_none());
    }

    #[test]
    fn long_text_is_trimmed_in_compact_and_kept_in_full() {
        let mut a = by_event("Winter Storm Warning");
        a.instruction_text = Some("word ".repeat(400));
        a.sections = json!({"what": "w ".repeat(600), "where": "Here."});
        let c = compact_feature(&a, t0(), "");
        let i = c["properties"]["instruction"].as_str().unwrap();
        assert!(i.ends_with('…') && i.chars().count() <= INSTRUCTION_MAX + 1);
        assert!(c["properties"]["summary"]["what"]
            .as_str()
            .unwrap()
            .ends_with('…'));
        assert_eq!(c["properties"]["summary"]["where"], "Here.");
        assert!(c["properties"]["summary"].get("when").is_none());
        let f = full_feature(&a, t0(), "");
        assert_eq!(
            f["properties"]["instruction"]
                .as_str()
                .unwrap()
                .trim()
                .len(),
            4 * 400 + 399
        );
        assert!(!f["properties"]["summary"]["what"]
            .as_str()
            .unwrap()
            .ends_with('…'));
    }

    #[test]
    fn full_adds_everything_and_lifecycle() {
        let mut a = by_event("Flash Flood Warning");
        a.superseded_by = Some("urn:later".into());
        let f = full_feature(&a, t0(), "");
        let p = &f["properties"];
        assert_eq!(p["status"], "superseded");
        assert_eq!(p["superseded_by"], "urn:later");
        assert!(p["description_raw"].is_string());
        assert!(p["sections"].is_object());
        assert!(p["vtec"].as_array().is_some_and(|v| !v.is_empty()));
        assert!(p["parameters"].is_object());
        assert!(p["ugc"].is_array() && p["zones"].is_array());
        assert!(p["references"].is_array());
        assert!(p["first_seen_at"].is_string() && p["last_seen_at"].is_string());
        assert!(p["ended_at"].is_null());
    }

    #[test]
    fn status_derivation() {
        let mut a = by_event("Winter Storm Warning"); // onset 2026-11-02T00:00Z, ends 18:00Z
        assert_eq!(alert_status(&a, t0()), "upcoming");
        let during = Utc.with_ymd_and_hms(2026, 11, 2, 3, 0, 0).unwrap();
        assert_eq!(alert_status(&a, during), "active");
        let after = a.end_time.unwrap() + Duration::seconds(1);
        assert_eq!(alert_status(&a, after), "ended");
        a.ended_at = Some(t0());
        assert_eq!(alert_status(&a, t0()), "ended");
        a.superseded_by = Some("x".into());
        assert_eq!(alert_status(&a, t0()), "superseded");
        // read at a past instant, replaced / ended later is not yet replaced / ended
        a.superseded_at = Some(t0());
        a.ended_at = Some(t0() + Duration::minutes(5));
        assert_eq!(alert_status(&a, t0() - Duration::seconds(1)), "upcoming");
        assert_eq!(
            alert_status(&a, t0()),
            "superseded",
            "from the instant it was replaced"
        );
        a.superseded_by = None;
        a.superseded_at = None;
        assert_eq!(alert_status(&a, t0() + Duration::minutes(4)), "upcoming");
        assert_eq!(alert_status(&a, t0() + Duration::minutes(5)), "ended");
        // no onset: effective decides
        let mut b = by_event("Severe Thunderstorm Warning");
        b.onset = None;
        b.effective = Some(t0() + Duration::hours(1));
        assert_eq!(alert_status(&b, t0()), "upcoming");
    }

    #[test]
    fn significance_from_vtec_then_name() {
        let expect = [
            ("Severe Thunderstorm Warning", "warning"),
            ("Flood Watch", "watch"),
            ("Small Craft Advisory", "advisory"),
            ("Special Weather Statement", "statement"),
            ("Test Message", "other"),
        ];
        for (event, want) in expect {
            let mut a = by_event(event);
            assert_eq!(significance(&a), want, "{event} (with VTEC if any)");
            a.vtec = json!([]);
            assert_eq!(significance(&a), want, "{event} (name only)");
        }
        let mut a = by_event("Flood Watch");
        a.vtec = json!([{"significance": "Y"}]);
        assert_eq!(significance(&a), "advisory", "VTEC wins over the name");
    }

    #[test]
    fn meta_flags_staleness() {
        let now = t0();
        let st = |ago: i64| FeedStatus {
            last_success_at: Some(now - Duration::seconds(ago)),
            ..Default::default()
        };
        let fresh = feed_meta(Some(&st(40)), now);
        assert_eq!(fresh["data_age_seconds"], 40);
        assert_eq!(fresh["stale"], false);
        assert_eq!(fresh["source"], "api.weather.gov");
        assert_eq!(feed_meta(Some(&st(301)), now)["stale"], true);
        assert_eq!(feed_meta(Some(&st(300)), now)["stale"], false);
        let never = feed_meta(None, now);
        assert_eq!(
            (never["stale"].clone(), never["last_success_at"].clone()),
            (json!(true), Value::Null)
        );
        // a failing poll after the last success is surfaced
        let mut failing = st(100);
        failing.last_error = Some("timeout".into());
        failing.last_error_at = Some(now - Duration::seconds(10));
        assert_eq!(feed_meta(Some(&failing), now)["last_error"], "timeout");
        assert!(feed_meta(Some(&st(100)), now).get("last_error").is_none());
    }

    #[test]
    fn the_collection_description_says_how_old_the_data_is() {
        let now = t0();
        let st = |ago: i64| FeedStatus {
            last_success_at: Some(now - Duration::seconds(ago)),
            ..Default::default()
        };
        assert_eq!(
            data_age_sentence(&feed_meta(Some(&st(40)), now)),
            "Data age 40 s."
        );
        assert_eq!(
            data_age_sentence(&feed_meta(Some(&st(900)), now)),
            "STALE: last successful poll of api.weather.gov was 900 s ago."
        );
        assert_eq!(
            data_age_sentence(&feed_meta(None, now)),
            "STALE: no successful poll of api.weather.gov yet."
        );
    }

    /// The geometry of a list answer goes through as the text the database made, not as a parsed
    /// tree, and the answer is the same JSON as the `Value` one.
    #[test]
    fn list_answers_write_the_geometry_text_through() {
        let g = r#"{"type":"MultiPolygon","coordinates":[[[[-107.5,37],[-106.5,37],[-106.5,38],[-107.5,37]]]]}"#;
        let mut a = by_event("Flood Watch");
        a.geometry_geojson = Some(g.to_string());
        let mut c = collection(
            vec![list_feature(&a, t0(), "")],
            t0(),
            json!({"stale": false}),
        );
        c.extra.insert("truncated".into(), json!(true));
        let text = serde_json::to_string(&c).unwrap();
        assert!(
            text.contains(&format!(r#""geometry":{g},"properties":"#)),
            "{text}"
        );
        assert!(text.starts_with(
            r#"{"type":"FeatureCollection","numberReturned":1,"features":[{"type":"Feature","id":"#
        ));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["truncated"], true);
        assert_eq!(v["timeStamp"], ts(t0()));
        assert_eq!(v["meta"]["stale"], false);
        // the same feature as the Value form tests and `full_feature` use
        assert_eq!(v["features"][0], compact_feature(&a, t0(), ""));
        assert_eq!(
            v["features"][0]["geometry"],
            serde_json::from_str::<Value>(g).unwrap()
        );
        for none in [None, Some("not json".to_string()), Some(String::new())] {
            a.geometry_geojson = none;
            assert!(compact_feature(&a, t0(), "")["geometry"].is_null());
            let text = serde_json::to_string(&list_feature(&a, t0(), "")).unwrap();
            assert!(text.contains(r#""geometry":null"#), "{text}");
        }
    }

    #[test]
    fn the_geometry_budget_cuts_an_answer_after_the_row_that_crosses_it() {
        let rows: Vec<StoredAlert> = [Some(100), Some(100), None, Some(100), Some(100)]
            .into_iter()
            .map(|n: Option<usize>| {
                let mut a = by_event("Flood Watch");
                a.geometry_geojson = n.map(|n| "x".repeat(n));
                a
            })
            .collect();
        // sizes 100, 100, 0, 100, 100: the row that would take the total over the budget is not kept
        // (budget, rows kept, cut short)
        for (budget, kept, cut) in [
            (1_000, 5, false),
            (400, 5, false),
            (399, 4, true),
            (300, 4, true),
            (299, 3, true),
            (200, 3, true), // the empty one fits after the second
            (199, 1, true),
            (0, 1, true), // never less than the first row
        ] {
            assert_eq!(within_budget(&rows, budget), (kept, cut), "budget {budget}");
        }
        assert_eq!(within_budget(&[], 10), (0, false));
        assert_eq!(
            within_budget(&rows[2..3], 0),
            (1, false),
            "no geometry costs nothing"
        );
    }

    #[test]
    fn collection_counts_features() {
        let a = by_event("Flood Watch");
        let c = serde_json::to_value(collection(
            vec![list_feature(&a, t0(), "")],
            t0(),
            json!({}),
        ))
        .unwrap();
        assert_eq!(c["type"], "FeatureCollection");
        assert_eq!(c["numberReturned"], 1);
    }

    #[test]
    fn point_and_polygon_parsing() {
        assert_eq!(parse_point("POINT(-105.2 40.0)").unwrap(), (-105.2, 40.0));
        assert_eq!(parse_point("point( -105.2   40 )").unwrap(), (-105.2, 40.0));
        assert_eq!(parse_point("-105.2,40").unwrap(), (-105.2, 40.0));
        assert!(parse_point("POINT(200 40)").is_err());
        assert!(parse_point("POINT(1)").is_err());
        assert!(parse_point("").is_err());
        let w = parse_polygon("POLYGON((-106 39,-105 39,-105 40,-106 40))").unwrap();
        assert_eq!(w, "POLYGON((-106 39, -105 39, -105 40, -106 40, -106 39))"); // closed for you
        assert!(parse_polygon("POLYGON((-106 39,-105 39))").is_err());
        assert!(parse_polygon("POINT(1 2)").is_err());
        assert_eq!(
            parse_bbox("-106,39,-105,40").unwrap(),
            [-106.0, 39.0, -105.0, 40.0]
        );
        assert!(parse_bbox("1,2,3").is_err());
        // no height, no width, south above north
        for bad in ["-106,40,-105,40", "-106,41,-105,40", "-106,39,-106,40"] {
            assert!(parse_bbox(bad).is_err(), "{bad}");
        }
    }

    /// `POLYGON((x y, ...))` from points, written as the request would carry it (not closed).
    fn wkt(pts: &[Pt]) -> String {
        let body: Vec<String> = pts.iter().map(|(x, y)| format!("{x} {y}")).collect();
        format!("POLYGON(({}))", body.join(","))
    }

    /// `n` points on a circle, counter-clockwise (or clockwise).
    fn circle(n: usize, centre: Pt, radius: f64, ccw: bool) -> Vec<Pt> {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64 * if ccw { 1.0 } else { -1.0 };
                (centre.0 + radius * a.cos(), centre.1 + radius * a.sin())
            })
            .collect()
    }

    /// `n` random points, 1 decimal, in lon -120..-70 and lat 25..48, from a fixed seed. Joined in
    /// order they make the ring that took the live service minutes to answer.
    fn scribble(n: usize, seed: u64) -> Vec<Pt> {
        let mut state = seed;
        let mut next = |lo: f64, hi: f64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            ((lo + unit * (hi - lo)) * 10.0).round() / 10.0
        };
        (0..n)
            .map(|_| (next(-120.0, -70.0), next(25.0, 48.0)))
            .collect()
    }

    #[test]
    fn polygon_rings_that_are_simple_are_accepted_in_either_winding() {
        for ccw in [true, false] {
            let w = parse_polygon(&wkt(&circle(300, (-100.0, 40.0), 2.0, ccw))).unwrap();
            assert_eq!(
                w.matches(',').count(),
                300,
                "300 vertices plus the closing one"
            );
        }
        let square = |pts: &[Pt]| parse_polygon(&wkt(pts)).is_ok();
        assert!(square(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]));
        assert!(square(&[(0.0, 0.0), (0.0, 4.0), (4.0, 4.0), (4.0, 0.0)]));
        // collinear but apart: the two y=4 edges of a U shape are no contact
        assert!(square(&[
            (0.0, 0.0),
            (6.0, 0.0),
            (6.0, 4.0),
            (4.0, 4.0),
            (4.0, 2.0),
            (2.0, 2.0),
            (2.0, 4.0),
            (0.0, 4.0),
        ]));
        // a redundant vertex on a straight edge, and a repeated vertex, change nothing
        assert!(square(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (0.0, 4.0)
        ]));
        assert_eq!(
            parse_polygon("POLYGON((0 0,4 0,4 0,4 4,0 4,0 0,0 0))").unwrap(),
            "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))"
        );
        // exactly 180 degrees of longitude in one edge is still allowed
        assert!(square(&[
            (-90.0, 0.0),
            (90.0, 0.0),
            (90.0, 10.0),
            (-90.0, 10.0)
        ]));
    }

    #[test]
    fn polygon_rejects_a_self_intersecting_scribble() {
        let ring = scribble(200, 1);
        let e = parse_polygon(&wkt(&ring)).unwrap_err();
        assert!(e.contains("simple ring"), "{e}");
        // its closed form, as the verifier sent it, too
        let mut closed = ring.clone();
        closed.push(ring[0]);
        assert!(parse_polygon(&wkt(&closed))
            .unwrap_err()
            .contains("simple ring"));
    }

    #[test]
    fn polygon_rejects_edges_that_meet() {
        let simple = |pts: &[Pt]| {
            parse_polygon(&wkt(pts))
                .err()
                .is_some_and(|e| e.contains("simple ring"))
        };
        // bowtie: two edges cross
        assert!(simple(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]));
        // a vertex on another edge (T junction)
        assert!(simple(&[
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (2.0, 0.0),
            (0.0, 4.0)
        ]));
        // two lobes that share a vertex
        assert!(simple(&[
            (0.0, 0.0),
            (2.0, 0.0),
            (1.0, 1.0),
            (2.0, 2.0),
            (0.0, 2.0),
            (1.0, 1.0),
        ]));
        // non-adjacent edges overlapping along y=0
        assert!(simple(&[
            (0.0, 0.0),
            (4.0, 0.0),
            (5.0, 3.0),
            (3.0, 0.0),
            (1.0, 0.0),
            (1.0, -3.0)
        ]));
        // adjacent edges doubling back over each other (a spike)
        assert!(simple(&[(0.0, 0.0), (4.0, 0.0), (2.0, 0.0), (2.0, 3.0)]));
        // the same spike across the closing vertex
        assert!(simple(&[(2.0, 0.0), (2.0, 3.0), (0.0, 0.0), (4.0, 0.0)]));
        // collinear decimals that are not bit for bit on the line still touch
        assert!(simple(&[
            (0.1, 0.1),
            (0.3, 0.3),
            (0.5, 0.1),
            (0.2, 0.2),
            (0.2, 0.5),
        ]));
    }

    #[test]
    fn a_bbox_with_west_above_east_crosses_the_antimeridian() {
        assert_eq!(
            parse_bbox("170,50,-170,60").unwrap(),
            [170.0, 50.0, -170.0, 60.0]
        );
        assert_eq!(
            parse_bbox("180,50,-179,60").unwrap(),
            [180.0, 50.0, -179.0, 60.0]
        );
        assert_eq!(
            parse_bbox("-180,50,180,60").unwrap(),
            [-180.0, 50.0, 180.0, 60.0]
        );
        // longitudes beyond +-180 are still out of range
        assert!(parse_bbox("170,50,190,60").is_err());
        let e = parse_bbox("-100,50,-100,60").unwrap_err();
        assert!(e.contains("antimeridian"), "{e}");
    }

    #[test]
    fn a_polygon_comes_with_its_bbox() {
        let (wkt, bbox) = parse_polygon_with_bbox("POLYGON((-106 39,-105 39,-105.5 41))").unwrap();
        assert_eq!(wkt, "POLYGON((-106 39, -105 39, -105.5 41, -106 39))");
        assert_eq!(bbox, [-106.0, 39.0, -105.0, 41.0]);
        assert_eq!(
            parse_polygon("POLYGON((-106 39,-105 39,-105.5 41))").unwrap(),
            wkt
        );
        assert!(parse_polygon_with_bbox("POLYGON((0 0,1 1,2 2,0 0))").is_err());
    }

    #[test]
    fn polygon_rejects_degenerate_rings() {
        for (c, want) in [
            ("POLYGON((0 0,1 1,2 2,0 0))", "no area"),
            ("POLYGON((0 0,0 0,0 0,0 0))", "3 distinct vertices"),
            ("POLYGON((0 0,1 0,0 0))", "3 distinct vertices"),
            ("POLYGON((0 0,1 0))", "3 distinct vertices"),
            ("POLYGON(())", "Invalid polygon vertex"),
        ] {
            let e = parse_polygon(c).unwrap_err();
            assert!(e.contains(want), "{c}: {e}");
        }
    }

    #[test]
    fn polygon_rejects_holes_and_trailing_text() {
        let hole = "POLYGON((0 0,10 0,10 10,0 10,0 0),(2 2,4 2,4 4,2 4,2 2))";
        let e = parse_polygon(hole).unwrap_err();
        assert!(e.contains("holes are not supported"), "{e}");
        // a second polygon part is not smuggled through either
        assert!(parse_polygon("POLYGON((0 0,1 0,1 1,0 0)),((5 5,6 5,6 6,5 5))").is_err());
        for c in [
            "POLYGON((0 0,4 0,4 4,0 4,0 0)) junk",
            "POLYGON((0 0,4 0,4 4,0 4,0 0)",
            "POLYGON(((0 0,4 0,4 4,0 4,0 0)))",
            "POLYGON(0 0,4 0,4 4,0 4,0 0)",
        ] {
            assert!(parse_polygon(c).is_err(), "{c}");
        }
    }

    #[test]
    fn polygon_rejects_the_antimeridian() {
        let e = parse_polygon("POLYGON((179 56,-179 56,-179 58,179 58,179 56))").unwrap_err();
        assert!(e.contains("antimeridian"), "{e}");
        // one long edge is enough, and 180.5 degrees is over the line
        let e = parse_polygon("POLYGON((-90.5 0,90 0,90 10,-90.5 10))").unwrap_err();
        assert!(e.contains("antimeridian"), "{e}");
        // each half on its own side is fine
        assert!(parse_polygon("POLYGON((179 56,180 56,180 58,179 58,179 56))").is_ok());
        assert!(parse_polygon("POLYGON((-180 56,-179 56,-179 58,-180 58))").is_ok());
    }

    #[test]
    fn polygon_vertex_cap_counts_vertices_not_the_closing_repeat() {
        let ring = circle(MAX_POLYGON_VERTICES, (-100.0, 40.0), 5.0, true);
        assert!(parse_polygon(&wkt(&ring)).is_ok(), "500 vertices");
        let mut closed = ring.clone();
        closed.push(ring[0]);
        assert!(
            parse_polygon(&wkt(&closed)).is_ok(),
            "500 vertices, closed by the caller"
        );
        let e = parse_polygon(&wkt(&circle(
            MAX_POLYGON_VERTICES + 1,
            (-100.0, 40.0),
            5.0,
            true,
        )))
        .unwrap_err();
        assert!(e.contains("more than 500 vertices"), "{e}");
        // far past the cap it stops reading rather than building a huge ring
        let e = parse_polygon(&wkt(&circle(5000, (-100.0, 40.0), 5.0, true))).unwrap_err();
        assert!(e.contains("more than 500 vertices"), "{e}");
    }

    /// The pairwise check is quadratic: the biggest ring, and the one with the most overlapping
    /// boxes (a spiky star, whose long edges all cross the middle), still validate quickly. The
    /// bound is far above the few milliseconds it takes, so a slow runner cannot fail it.
    #[test]
    fn polygon_validation_of_the_biggest_ring_is_fast() {
        let star: Vec<Pt> = (0..MAX_POLYGON_VERTICES)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / MAX_POLYGON_VERTICES as f64;
                let r = if i % 2 == 0 { 5.0 } else { 0.5 };
                (-100.0 + r * a.cos(), 40.0 + r * a.sin())
            })
            .collect();
        for ring in [
            star,
            circle(MAX_POLYGON_VERTICES, (-100.0, 40.0), 5.0, false),
        ] {
            let start = std::time::Instant::now();
            parse_polygon(&wkt(&ring)).unwrap();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "took {:?}",
                start.elapsed()
            );
        }
    }

    #[test]
    fn a_timed_out_read_is_a_503_and_other_failures_a_500() {
        let r = catalog_failure(WmsError::ServiceUnavailable("search cancelled".into()));
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "application/json");
        // a lock wait counts toward the limit, so a retry may well work
        assert_eq!(r.headers()[header::RETRY_AFTER], "5");
        let r = catalog_failure(WmsError::DatabaseError("connection reset".into()));
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn query_timeout_defaults_to_ten_seconds() {
        use std::time::Duration;
        assert_eq!(query_timeout(None), Duration::from_secs(10));
        assert_eq!(query_timeout(Some("2500")), Duration::from_millis(2500));
        assert_eq!(query_timeout(Some(" 750 ")), Duration::from_millis(750));
        for bad in ["", "0", "-5", "10s", "1.5", "soon"] {
            assert_eq!(query_timeout(Some(bad)), Duration::from_secs(10), "{bad:?}");
        }
    }

    #[test]
    fn non_ascii_coords_are_a_400_not_a_panic() {
        // The prefix checks used to slice bytes (`c[..5]`, `c[..7]`), which panics when
        // a multi-byte character straddles the cut.
        for c in [
            "1234€",       // byte 5 is inside the euro sign
            "€€€€€€€",     // every cut is inside a character
            "POIN€(1 2)",  // 'POIN' + a 3-byte char
            "POINT€(1 2)", // prefix is fine, the rest is not a number
            "ÄÄÄ,ÖÖÖ",     // "lon,lat" made of non-ASCII
            "   é   ",
        ] {
            assert!(parse_point(c).is_err(), "{c:?} must be rejected, not panic");
        }
        for c in [
            "POLYGÖN((1 1,2 1,2 2,1 1))",
            "POLYG€((1 1,2 1,2 2,1 1))", // byte 7 is inside the euro sign
            "PÖLYGON((1 1,2 1,2 2,1 1))",
            "€€€€€€€€",
            "POLYGON€((1 1,2 1,2 2,1 1))",
        ] {
            assert!(
                parse_polygon(c).is_err(),
                "{c:?} must be rejected, not panic"
            );
        }
        // plain ASCII still works, in any case
        assert_eq!(parse_point("Point(1 2)").unwrap(), (1.0, 2.0));
        assert!(parse_polygon("polygon((0 0,1 0,1 1,0 0))").is_ok());
        // `within` units are sliced at an ASCII letter, so non-ASCII text is just an error
        assert!(parse_within(Some("5€km"), None).is_err());
        assert!(parse_within(Some("5ñ"), None).is_err());
    }

    #[test]
    fn datetime_and_option_parsing() {
        let now = t0();
        assert_eq!(parse_when(None, now).unwrap(), When::None);
        assert_eq!(parse_when(Some("now"), now).unwrap(), When::Instant(now));
        let t = Utc.with_ymd_and_hms(2026, 11, 1, 18, 0, 0).unwrap();
        assert_eq!(
            parse_when(Some("2026-11-01T18:00:00Z"), now).unwrap(),
            When::Instant(t)
        );
        assert_eq!(
            parse_when(Some("2026-11-01T18:00:00Z/.."), now).unwrap(),
            When::Interval(Some(t), None)
        );
        assert!(parse_when(Some("yesterday"), now).is_err());
        assert_eq!(csv(Some("Severe, extreme,,")), vec!["Severe", "extreme"]);
        assert!(csv(None).is_empty());
        assert!(parse_bool(Some("maybe"), true).is_err());
        assert!(!parse_bool(Some("FALSE"), true).unwrap());
        assert_eq!(parse_within(Some("50"), None).unwrap(), 50_000.0);
        assert_eq!(parse_within(Some("10"), Some("mi")).unwrap(), 16093.44);
        assert_eq!(parse_within(Some("5km"), Some("mi")).unwrap(), 5000.0);
        assert!(parse_within(Some("9999"), None).is_err());
        assert!(parse_within(None, None).is_err());
        assert!(default_simplify(&[-125.0, 24.0, -66.0, 50.0]) <= 0.05);
        // a box across the antimeridian is as wide as it looks on the map, not 340 degrees
        assert_eq!(
            default_simplify(&[179.0, 50.0, -179.0, 51.0]),
            default_simplify(&[0.0, 50.0, 2.0, 51.0])
        );
        assert!(default_simplify(&[179.0, 50.0, -179.0, 51.0]) < 0.002);
        // the whole width of the world is 360 wide, even when the box is thin
        assert_eq!(default_simplify(&[-180.0, 10.0, 180.0, 11.0]), 0.05);
        assert!(default_simplify(&[-105.3, 39.9, -105.2, 40.0]) < 0.0001);
    }

    #[test]
    fn filters_reject_control_characters_but_not_ordinary_names() {
        assert_eq!(
            filter_list("event", Some("Tornado Warning, Flash Flood Warning,,")).unwrap(),
            vec!["Tornado Warning", "Flash Flood Warning"]
        );
        assert!(filter_list("event", None).unwrap().is_empty());
        for bad in [
            "a\0b",
            "\0",
            "Severe,a\0",
            "x\u{7f}",
            "a\tb",
            "a\nb",
            "\u{1b}[0m",
        ] {
            let e = filter_list("severity", Some(bad)).unwrap_err();
            assert!(
                e.starts_with("severity: control characters"),
                "{bad:?}: {e}"
            );
        }
        // padding is trimmed away like any other, and non-ASCII names are fine
        assert_eq!(
            filter_list("event", Some("\tTornado Warning\n")).unwrap(),
            vec!["Tornado Warning"]
        );
        assert_eq!(filter_list("event", Some("Ouragan, ñ")).unwrap().len(), 2);
    }

    #[test]
    fn limit_is_one_or_more_and_cut_to_the_maximum() {
        assert_eq!(parse_limit(None, 500), Ok(500));
        assert_eq!(parse_limit(Some(1), 500), Ok(1));
        assert_eq!(parse_limit(Some(2000), 500), Ok(2000));
        assert_eq!(parse_limit(Some(2001), 500), Ok(2000));
        assert_eq!(parse_limit(Some(i64::MAX), 500), Ok(2000));
        for bad in [0, -1, -5, i64::MIN] {
            let e = parse_limit(Some(bad), 500).unwrap_err();
            assert!(e.contains("at least 1"), "{bad}: {e}");
        }
    }

    #[test]
    fn simplify_is_zero_or_a_positive_finite_number_of_degrees() {
        assert_eq!(parse_simplify(None), Ok(None));
        assert_eq!(parse_simplify(Some(0.0)), Ok(Some(0.0)));
        assert_eq!(parse_simplify(Some(0.01)), Ok(Some(0.01)));
        for bad in [-1.0, -0.000001, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(parse_simplify(Some(bad)).is_err(), "{bad}");
        }
        // "1e999" is infinity once parsed, so it is refused like `inf`
        let q = query_from_raw(Some("simplify=1e999")).unwrap();
        assert!(parse_simplify(q.simplify).is_err());
    }

    #[test]
    fn a_reversed_datetime_interval_is_an_error() {
        let now = t0();
        let e = parse_when(Some("2026-11-02T00:00:00Z/2026-11-01T00:00:00Z"), now).unwrap_err();
        assert!(
            e.contains("starts at 2026-11-02T00:00:00Z but ends at 2026-11-01T00:00:00Z"),
            "{e}"
        );
        assert!(parse_when(Some("now/2026-11-01T00:00:00Z"), now).is_err());
        // equal ends are an instant-long window; open ends cannot be reversed
        let t = Utc.with_ymd_and_hms(2026, 11, 1, 18, 0, 0).unwrap();
        assert_eq!(
            parse_when(Some("2026-11-01T18:00:00Z/2026-11-01T18:00:00Z"), now).unwrap(),
            When::Interval(Some(t), Some(t))
        );
        assert!(parse_when(Some("../2026-11-01T00:00:00Z"), now).is_ok());
        assert!(parse_when(Some("2026-11-01T00:00:00Z/.."), now).is_ok());
    }

    /// A rejected query string is the handler's JSON exception, with the reason, not axum's text.
    #[tokio::test]
    async fn a_query_string_that_does_not_parse_is_a_json_400_with_the_reason() {
        for qs in [
            "limit=abc",
            "limit=",
            "limit=99999999999999999999",
            "simplify=abc",
            "simplify=",
            "coords=1,2&coords=3,4",
        ] {
            let r = query_from_raw(Some(qs)).unwrap_err();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{qs}");
            assert_eq!(
                r.headers()[header::CONTENT_TYPE],
                "application/json",
                "{qs}"
            );
            let body = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
            let v: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["status"], 400, "{qs}");
            let detail = v["detail"].as_str().unwrap();
            assert!(
                detail.starts_with("Failed to deserialize query string:"),
                "{qs}: {detail}"
            );
            assert!(
                detail.len() > "Failed to deserialize query string:".len() + 5,
                "reason kept: {detail}"
            );
        }
        assert!(query_from_raw(Some("limit=5&simplify=0.1&event=Flood%20Watch")).is_ok());
    }

    fn accept(a: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(a) = a {
            h.insert(header::ACCEPT, a.parse().unwrap());
        }
        h
    }

    #[test]
    fn only_geojson_can_be_asked_for() {
        // the formats that are GeoJSON, in the spellings clients use
        for f in [
            "GeoJSON",
            "geojson",
            "geo+json",
            "application/geo+json",
            "application/geo json", // an unencoded `+`
            "json",
            "application/json",
            " GeoJSON ",
            "",
        ] {
            assert!(check_format(&accept(None), Some(f)).is_ok(), "{f:?}");
        }
        assert!(check_format(&accept(None), None).is_ok());
        // everything else is a 400 that says what is served
        for f in [
            "CoverageJSON",
            "covjson",
            "application/vnd.cov+json",
            "png",
            "html",
            "zzz",
            "CSV",
        ] {
            let r = check_format(&accept(None), Some(f)).unwrap_err();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{f}");
            assert_eq!(r.headers()[header::CONTENT_TYPE], "application/json", "{f}");
        }
        // an explicit f wins over the Accept header, as on the grid collections
        assert!(check_format(&accept(Some("image/png")), Some("GeoJSON")).is_ok());
        assert!(check_format(&accept(Some("*/*")), Some("CoverageJSON")).is_err());
    }

    #[test]
    fn accept_must_admit_geojson() {
        for ok in [
            "application/geo+json",
            "application/json",
            "*/*",
            "application/*",
            "application/vnd.cov+json, application/geo+json;q=0.5",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "application/vnd.cov+json, */*;q=0.1",
        ] {
            assert!(check_format(&accept(Some(ok)), None).is_ok(), "{ok}");
        }
        for bad in [
            "application/vnd.cov+json",
            "application/prs.coverage+json",
            "image/png",
            "text/html",
            "application/vnd.cov+json, image/png, text/html",
        ] {
            let r = check_format(&accept(Some(bad)), None).unwrap_err();
            assert_eq!(r.status(), StatusCode::NOT_ACCEPTABLE, "{bad}");
            assert_eq!(
                r.headers()[header::CONTENT_TYPE],
                "application/json",
                "{bad}"
            );
        }
    }

    #[test]
    fn active_time_rejects_intervals() {
        let q = NwsQuery {
            datetime: Some("2026-11-01T18:00:00Z/2026-11-01T19:00:00Z".into()),
            ..Default::default()
        };
        assert!(active_time(&q).is_err());
    }

    /// Not run by default: what a saved copy of the live feed (`curl -A "<agent>"
    /// https://api.weather.gov/alerts/active > feed.json`) makes of every alert, to compare two
    /// builds of the parsers. Writes `[{id, event, headline, summary, description, sections}]`
    /// (the compact `summary` and `headline` as a list answer carries them, the rest from the
    /// full record) to `NWS_SUMMARY_OUT`:
    /// `NWS_FEED_JSON=feed.json NWS_SUMMARY_OUT=out.json cargo test -p edr-api --lib
    /// live_feed_summaries -- --ignored`
    /// The answers are made as of `NWS_NOW` (RFC 3339) when it is set, else of 2026-11-01T23:30Z.
    #[test]
    #[ignore = "reads NWS_FEED_JSON, a saved copy of the live feed"]
    fn live_feed_summaries() {
        let feed = std::env::var("NWS_FEED_JSON").expect("NWS_FEED_JSON: path of the saved feed");
        let out = std::env::var("NWS_SUMMARY_OUT").expect("NWS_SUMMARY_OUT: where to write");
        let now = std::env::var("NWS_NOW")
            .ok()
            .map(|n| {
                DateTime::parse_from_rfc3339(&n)
                    .expect("NWS_NOW: RFC 3339")
                    .to_utc()
            })
            .unwrap_or_else(t0);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(feed).unwrap()).unwrap();
        let rows: Vec<Value> = v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let a = stored(&parse_feature(f).unwrap());
                let list = compact_feature(&a, now, "https://x.test/edr");
                let full = full_feature(&a, now, "https://x.test/edr");
                json!({
                    "id": a.id,
                    "event": a.event,
                    "headline": list["properties"]["headline"],
                    "summary": list["properties"]["summary"],
                    "description": full["properties"]["description"],
                    "sections": full["properties"]["sections"],
                })
            })
            .collect();
        eprintln!("{} alerts", rows.len());
        std::fs::write(out, serde_json::to_string_pretty(&rows).unwrap()).unwrap();
    }

    #[test]
    fn only_an_explicit_instant_leaves_the_live_set() {
        let q = |dt: Option<&str>| NwsQuery {
            datetime: dt.map(str::to_string),
            ..Default::default()
        };
        // no datetime, an empty one and `now` are all the wall clock: the live, indexed read
        for live in [None, Some(""), Some("now"), Some(" NOW ")] {
            assert_eq!(active_time(&q(live)), Ok(None), "{live:?}");
        }
        assert_eq!(
            active_time(&q(Some("2026-11-01T18:00:00-06:00"))),
            Ok(Some(Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap()))
        );
        assert!(active_time(&q(Some("yesterday"))).is_err());
    }
}
