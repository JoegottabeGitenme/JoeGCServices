//! PostGIS side of NWS alerts. See the module docs in `mod.rs`, and
//! `NWS_ALERTS_SCHEMA_SQL` in `catalog.rs` (the crate-level one) for the tables.
//!
//! ## Geometry
//!
//! `nws_alerts.geom` is the alert's *effective* geometry, stored, not computed
//! per query: its own polygon when NWS sent one, otherwise the union of its
//! zones from `nws_zones`. Point-in-polygon ("alerts at my location") is the hot
//! read, and unioning a dozen county/marine polygons per request would cost far
//! more than keeping one row's geometry up to date: the union is recomputed only
//! when an alert's zone list changes or one of its zones arrives/refreshes.
//!
//! Zones load lazily. [`NwsAlertCatalog::apply_feed`] stores alerts immediately
//! and returns the zone keys it has no (fresh) geometry for; the caller fetches
//! those and hands them to [`NwsAlertCatalog::upsert_zones`], which fills in the
//! alerts' geometry. Until then `geometry_source = 'pending'` (and `geom` holds
//! the union of whatever zones are known, possibly NULL).
//!
//! ## Quiet polls
//!
//! NWS' feed carries a new `updated` stamp every minute while nearly every alert in it is the
//! message it was the minute before. Every alert stores a hash of the feature it was derived from
//! (`content_hash`, see `feature_hash` and `DERIVATION_VERSION` in `mod.rs`), and
//! [`NwsAlertCatalog::apply_feed`] rewrites an alert only when it is new, its hash differs (NWS
//! changed it, or the parsers did), or its ended state must change (it came back after a poll
//! missed it). The rest get a fresh `last_seen_at` in one statement; nothing is parsed or sent
//! again for them.
//!
//! ## Concurrency and bad rows
//!
//! Two writers touch alert geometry: feed polls ([`NwsAlertCatalog::apply_feed`]) and zone
//! batches ([`NwsAlertCatalog::upsert_zones`]). Both take [`NWS_WRITE_LOCK_KEY`] (a
//! transaction-scoped advisory lock) first, so each sees what the other committed. Inside their
//! transaction every alert, every zone and every geometry recomputation runs under its own
//! savepoint: a row the database rejects (a polygon PostGIS cannot parse, a union GEOS cannot
//! compute) is stored without the offending geometry or skipped and counted, never allowed to
//! abort the poll or the batch (see `ApplyResult` / `ZonesResult`).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use tracing::warn;

use wms_common::{WmsError, WmsResult};

use super::lifecycle::{self, AlertTimes};
use super::ParsedAlert;

/// Key of the transaction-scoped advisory lock that serialises feed polls and zone batches (the
/// two writers of `nws_alerts` geometry). The bytes spell "NWSWRITE".
pub const NWS_WRITE_LOCK_KEY: i64 = 0x4E57_5357_5249_5445;

/// A fetched zone is re-fetched after this long.
pub const ZONE_REFRESH_DAYS: i64 = 30;
/// A zone that 404'd / had no geometry is retried after this long.
pub const ZONE_RETRY_HOURS: i64 = 24;
/// A zone whose fetch failed transiently (timeout, 5xx, rate limit) is retried after this long.
pub const ZONE_ERROR_RETRY_MINUTES: i64 = 15;

fn db(what: &'static str) -> impl Fn(sqlx::Error) -> WmsError {
    move |e| WmsError::DatabaseError(format!("NWS alerts {what} failed: {e}"))
}

/// [`db`] for the reads, which run under a `statement_timeout`: Postgres' "query canceled"
/// (SQLSTATE 57014) is the read outlasting its time limit, reported as `ServiceUnavailable` so the
/// caller can answer "too expensive" (503) instead of a fault (500).
fn read_db(what: &'static str) -> impl Fn(sqlx::Error) -> WmsError {
    move |e| match &e {
        sqlx::Error::Database(d) if d.code().as_deref() == Some("57014") => {
            WmsError::ServiceUnavailable(format!(
                "NWS alerts {what} cancelled: it exceeded its time limit"
            ))
        }
        _ => WmsError::DatabaseError(format!("NWS alerts {what} failed: {e}")),
    }
}

/// Outcome of fetching one zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneStatus {
    Ok,
    /// The zone URL returned 404 (retired/unknown zone id).
    NotFound,
    /// Fetched, but the zone has no usable polygon.
    NoGeometry,
    /// The fetch failed transiently (timeout, 5xx, rate limit). Recorded so the zone is not
    /// asked for again every poll; it is retried after [`ZONE_ERROR_RETRY_MINUTES`]. Never
    /// replaces a geometry we already hold.
    Error,
    /// Fetched, but PostGIS rejected the polygon. Set by the catalog when it stores a zone
    /// whose geometry raised an error, never by a caller.
    InvalidGeometry,
}

impl ZoneStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::NotFound => "not_found",
            Self::NoGeometry => "no_geometry",
            Self::Error => "error",
            Self::InvalidGeometry => "invalid_geometry",
        }
    }
}

/// A zone as fetched from `/zones/<type>/<id>`.
#[derive(Debug, Clone)]
pub struct ZoneRecord {
    /// `forecast/KSZ001`.
    pub zone_key: String,
    pub name: Option<String>,
    pub state: Option<String>,
    /// NWS' own `properties.type` (public, coastal, offshore, county, fire ...).
    pub kind: Option<String>,
    pub status: ZoneStatus,
    /// GeoJSON geometry string (Polygon or MultiPolygon).
    pub geometry_json: Option<String>,
}

/// What [`NwsAlertCatalog::apply_feed`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ApplyResult {
    pub received: usize,
    /// Rows inserted or rewritten: alerts that are new, changed, came back after being marked
    /// ended, or whose stored hash is unknown or from another derivation version.
    pub upserted: usize,
    /// Alerts NWS has not changed since they were stored: only `last_seen_at` moved.
    pub unchanged: usize,
    /// Older messages marked superseded by this feed's Updates/Cancels.
    pub superseded: u64,
    /// Rows no longer in the feed, now marked ended.
    pub ended: u64,
    /// Zone keys that need fetching (never fetched, or stale/failed past retry).
    pub missing_zones: Vec<String>,
    /// Alerts whose own polygon PostGIS rejected: stored without it (geometry from their
    /// zones, else none).
    pub geometry_rejected: usize,
    /// Alerts that could not be stored at all (skipped; an older stored version stays as is).
    pub failed: usize,
    /// Alerts whose geometry could not be recomputed from their zones (parked with none).
    pub geometry_failed: usize,
    /// The feed's `updated` stamp was older than the one already applied (a lagging cache node):
    /// recorded as a successful poll, but nothing in it was applied.
    pub stale_ignored: bool,
}

/// What [`NwsAlertCatalog::upsert_zones`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ZonesResult {
    /// Zone rows written (those stored as `invalid_geometry` included).
    pub stored: usize,
    /// Zones whose geometry PostGIS rejected: stored with no geometry, `invalid_geometry`.
    pub invalid_geometry: usize,
    /// Zones that could not be stored at all (skipped; asked for again later).
    pub failed: usize,
    /// Alerts whose geometry was recomputed.
    pub alerts_updated: u64,
    /// Alerts whose geometry could not be recomputed (parked with none).
    pub alerts_failed: usize,
}

/// Poll health, for a data-age indicator.
#[derive(Debug, Clone, Default, Serialize, sqlx::FromRow)]
pub struct FeedStatus {
    pub last_attempt_at: Option<DateTime<Utc>>,
    /// Last poll that returned a usable answer (200 applied, or 304).
    pub last_success_at: Option<DateTime<Utc>>,
    /// Last time the feed content actually changed (a 200, not a 304).
    pub last_change_at: Option<DateTime<Utc>>,
    /// NWS' own `updated` stamp from the last 200.
    pub feed_updated_at: Option<DateTime<Utc>>,
    pub last_alert_count: Option<i32>,
    pub last_error: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
}

/// An alert as stored, plus its derived `active` flag.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StoredAlert {
    pub id: String,
    pub event: String,
    pub severity: Option<String>,
    pub urgency: Option<String>,
    pub certainty: Option<String>,
    pub category: Option<String>,
    pub status: Option<String>,
    pub message_type: String,
    pub response: Option<String>,
    pub headline: Option<String>,
    pub nws_headline: Option<String>,
    pub description: Option<String>,
    pub instruction: Option<String>,
    pub description_text: Option<String>,
    pub instruction_text: Option<String>,
    pub area_desc: Option<String>,
    pub sender_name: Option<String>,
    pub web: Option<String>,
    pub sent: Option<DateTime<Utc>>,
    pub effective: Option<DateTime<Utc>>,
    pub onset: Option<DateTime<Utc>>,
    pub expires: Option<DateTime<Utc>>,
    pub ends: Option<DateTime<Utc>>,
    /// `COALESCE(ends, expires)`.
    pub end_time: Option<DateTime<Utc>>,
    pub ugc: Vec<String>,
    pub zone_keys: Vec<String>,
    pub is_marine: bool,
    pub vtec: Value,
    pub vtec_event_key: Option<String>,
    pub sections: Value,
    pub parameters: Value,
    pub color: String,
    pub references_ids: Vec<String>,
    pub superseded_by: Option<String>,
    /// When the replacement was issued (set with `superseded_by`).
    pub superseded_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub geometry_source: String,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Last poll that listed it.
    pub last_seen_at: DateTime<Utc>,
    /// GeoJSON of the effective geometry, when requested.
    pub geometry_geojson: Option<String>,
}

impl StoredAlert {
    /// Active at `now` (the SQL predicate in `ACTIVE_PREDICATE` agrees).
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        lifecycle::is_active(
            &AlertTimes {
                superseded: self.superseded_by.is_some(),
                ended: self.ended_at.is_some(),
                ends: self.ends,
                expires: self.expires,
            },
            now,
        )
    }
}

const COLS: &str = "a.id, a.event, a.severity, a.urgency, a.certainty, a.category, a.status, \
    a.message_type, a.response, a.headline, a.nws_headline, a.description, a.instruction, \
    a.description_text, a.instruction_text, a.area_desc, a.sender_name, a.web, a.sent, \
    a.effective, a.onset, a.expires, a.ends, a.end_time, a.ugc, a.zone_keys, a.is_marine, \
    a.vtec, a.vtec_event_key, a.sections, a.parameters, a.color, a.references_ids, \
    a.superseded_by, a.superseded_at, a.ended_at, a.geometry_source, a.first_seen_at, \
    a.updated_at, a.last_seen_at";

/// Not superseded, not ended, end time (if any) in the future. `$now` is bound by the caller.
fn active_predicate(now_param: &str) -> String {
    format!(
        "a.superseded_by IS NULL AND a.ended_at IS NULL AND (a.end_time IS NULL OR a.end_time > {now_param})"
    )
}

/// The message's life `[issue, end)` overlaps the window `[from, to]`: issued by `to` (`effective`,
/// else `sent`) and not over before `from`, over being its end time, its `ended_at` or the
/// moment a newer message replaced it, whichever came first. For `from == to` this is "in force
/// at that instant", an end time or replacement at that very instant already counting as over.
/// `$from` and `$to` are bound by the caller.
fn life_overlaps(from_param: &str, to_param: &str) -> String {
    format!(
        "COALESCE(a.effective, a.sent) <= {to_param} \
         AND COALESCE(LEAST(a.end_time, a.ended_at, a.superseded_at), 'infinity'::timestamptz) > {from_param}"
    )
}

/// The time part of [`NwsAlertCatalog::search`]'s `WHERE`: `$1` is `now`, `$2`/`$3` the window.
/// Without a window it is the lifecycle-flag predicate the partial index
/// `idx_nws_alerts_geom_live` serves, and must stay exactly that.
fn time_predicate(q: &AlertSearch) -> String {
    match q.window {
        None => format!(
            "{} AND $2::timestamptz IS NULL AND $3::timestamptz IS NULL",
            active_predicate("$1")
        ),
        Some(_) => format!(
            "{} AND {}",
            if q.include_superseded {
                "TRUE"
            } else {
                "a.superseded_by IS NULL"
            },
            life_overlaps("$2", "$3"),
        ),
    }
}

/// The CAP `status` filter. NWS' feed also carries `Test`, `Exercise`, `System` and `Draft`
/// messages (e.g. the "Test Message" keepalive); they are all stored, but the reads only show
/// `Actual` ones unless the caller asks for the rest. A missing status is treated as `Actual`:
/// hiding a real warning because a field was absent is the worse mistake.
fn actual_only(include_non_actual: bool) -> &'static str {
    if include_non_actual {
        "TRUE"
    } else {
        "COALESCE(lower(a.status), 'actual') = 'actual'"
    }
}

/// Returned coordinates are multiples of this many degrees (about a metre), printed with
/// [`GEOJSON_DIGITS`] decimals.
const GEOJSON_GRID: &str = "0.00001";
const GEOJSON_DIGITS: u8 = 5;

/// The SQL expression for an alert's geometry as GeoJSON text (`NULL` unless `with_geometry`),
/// simplified by `simplify` degrees when that is a positive number. The stored geometry is valid,
/// but a plain `ST_AsGeoJSON(geom, 5)` is not what a client reads back:
///
/// - rounding to a metre can push two edges of a thin zone through each other (two NWS zones,
///   AMZ364 and PZZ745, come out self-intersecting);
/// - `ST_SimplifyPreserveTopology` keeps edges from crossing but, before GEOS 3.12, lets a
///   simplified shell edge skip past a whole island or hole, which then lies outside the shell
///   ("Hole lies outside shell", nested shells), common on coastal and offshore zone unions.
///
/// So the simplified geometry is made valid first (`nws_clean_geom`), then snapped to the grid;
/// if the snapped geometry is no longer valid it is rebuilt with `ST_ReducePrecision`, which
/// rounds to the grid and repairs what the rounding broke. The snap and the validity check cost
/// a few milliseconds even for a 100 000-vertex zone; `ST_ReducePrecision` alone would be ten
/// times that for every alert. The result is always a MultiPolygon, or NULL for a shape that
/// collapses at this precision. The `OFFSET 0`s keep the planner from evaluating the inner
/// expressions once per use.
fn geometry_json_sql(with_geometry: bool, simplify: Option<f64>) -> String {
    if !with_geometry {
        return "NULL::text".to_string();
    }
    let source = match simplify.filter(|t| t.is_finite() && *t > 0.0) {
        Some(tol) => format!("nws_clean_geom(ST_SimplifyPreserveTopology(a.geom, {tol}))"),
        None => "a.geom".to_string(),
    };
    format!(
        "(SELECT ST_AsGeoJSON(CASE WHEN ST_IsEmpty(s.snapped) THEN NULL \
                WHEN ST_IsValid(s.snapped) THEN s.snapped \
                ELSE nws_clean_geom(ST_ReducePrecision(s.src, {GEOJSON_GRID})) END, {GEOJSON_DIGITS}) \
         FROM (SELECT t.src, ST_SnapToGrid(t.src, {GEOJSON_GRID}) AS snapped \
               FROM (SELECT {source} AS src OFFSET 0) t OFFSET 0) s)"
    )
}

/// `ORDER BY` that puts the worst, most recent alerts first.
const SEVERITY_ORDER: &str = "CASE a.severity WHEN 'Extreme' THEN 0 WHEN 'Severe' THEN 1 \
    WHEN 'Moderate' THEN 2 WHEN 'Minor' THEN 3 ELSE 4 END, a.effective DESC NULLS LAST, a.id";

/// Where an [`AlertSearch`] looks.
#[derive(Debug, Clone, PartialEq)]
pub enum AlertRegion {
    Point {
        lon: f64,
        lat: f64,
    },
    /// `[west, south, east, north]`. West greater than east is a box across the antimeridian:
    /// west to 180 and -180 to east.
    Bbox([f64; 4]),
    /// WKT `POLYGON((lon lat, ...))`, validated by the caller.
    Polygon {
        wkt: String,
    },
    /// Within `meters` of a point (geodesic).
    Radius {
        lon: f64,
        lat: f64,
        meters: f64,
    },
}

/// A filtered alert query: the general form behind the EDR position / area /
/// radius / items reads.
#[derive(Debug, Clone)]
pub struct AlertSearch {
    pub region: AlertRegion,
    /// The moment the caller reads the alerts at; the lifecycle status of each is as of `now`.
    pub now: DateTime<Utc>,
    /// `None`: the live set at `now` (not superseded, not ended, end time null or future;
    /// alerts that start later count), from the lifecycle flags alone: the indexed fast path
    /// for the wall clock. `Some((from, to))`: every alert whose life overlaps the window,
    /// ended ones included; see [`Self::in_force_at`] for an instant in the past.
    pub window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Window mode only: also return messages that a later one replaced.
    pub include_superseded: bool,
    /// Case-insensitive; empty means any.
    pub severities: Vec<String>,
    /// Case-insensitive event names; empty means any.
    pub events: Vec<String>,
    pub limit: i64,
    pub with_geometry: bool,
    /// Douglas-Peucker tolerance in degrees applied to returned geometry.
    pub simplify: Option<f64>,
    /// Also return CAP `status` Test / Exercise / System / Draft messages (default: only `Actual`).
    pub include_non_actual: bool,
}

impl AlertSearch {
    /// The messages in force at `t`: issued by then and not yet over, ended or replaced, whatever
    /// became of them since (so a message replaced a minute after `t` is returned, its
    /// replacement is not). That is the window `[t, t]` over every message, replaced ones
    /// included, read as of `t`. History reaches back only as far as retention keeps rows.
    pub fn in_force_at(mut self, t: DateTime<Utc>) -> Self {
        self.now = t;
        self.window = Some((t, t));
        self.include_superseded = true;
        self
    }
}

#[derive(Clone)]
pub struct NwsAlertCatalog {
    pool: PgPool,
    /// `statement_timeout` for every read (see [`Self::with_read_timeout`]); `None`: the
    /// server's own setting.
    read_timeout: Option<std::time::Duration>,
}

impl NwsAlertCatalog {
    /// Longest a read may run when the caller sets none of its own (the EDR API's default).
    pub const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            read_timeout: None,
        }
    }

    /// Cap every read ([`Self::status`], [`Self::active_at_point`], [`Self::in_bbox`],
    /// [`Self::search`], [`Self::get`]) at `timeout`. Postgres cancels a read that runs longer
    /// and it fails as `WmsError::ServiceUnavailable`. The limit is a `SET LOCAL` inside the
    /// read's own transaction, so it ends with the read: it never reaches the next user of the
    /// pooled connection, and the writers (which never use it) stay unlimited.
    ///
    /// Postgres checks for the cancel between operations, not inside one long GEOS call, so a
    /// single pathological geometry operation still has to be kept out by validating the input.
    pub fn with_read_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }

    /// A transaction for one read, with the read timeout set when there is one.
    async fn read_tx(&self) -> WmsResult<sqlx::Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        if let Some(timeout) = self.read_timeout {
            // `set_config(.., true)` is `SET LOCAL` that takes a bind parameter (milliseconds).
            sqlx::query("SELECT set_config('statement_timeout', $1, true)")
                .bind(timeout.as_millis().max(1).to_string())
                .execute(&mut *tx)
                .await
                .map_err(db("set statement_timeout"))?;
        }
        Ok(tx)
    }

    // --- writes ---------------------------------------------------------------

    /// Apply one successful poll of the active feed (every alert NWS lists now).
    ///
    /// In one transaction: upsert each alert that is new or changed (an alert whose stored
    /// `content_hash` equals the feed's, and whose ended state already matches, is only given a
    /// fresh `last_seen_at`, in one statement for all of them); mark the older messages each
    /// Update or Cancel references as superseded; mark every stored, still-live alert that this
    /// feed no longer lists as ended; fill in geometry from the zones already known; record poll
    /// success. An **empty** feed ends nothing (NWS always has alerts; an empty answer is a
    /// fault, not "all clear").
    pub async fn apply_feed(
        &self,
        alerts: &[ParsedAlert],
        feed_updated: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> WmsResult<ApplyResult> {
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        lock_writers(&mut tx).await?;
        let mut res = ApplyResult {
            received: alerts.len(),
            ..Default::default()
        };

        // NWS answers from a CDN; a lagging node can serve a feed older than the one already
        // applied. Applying it would resurrect nothing but could end every alert issued since
        // (they are "no longer listed") and revert updates. The answer is still a valid reply, so
        // the poll counts as a success (data age keeps moving), but it changes no alert. Equal
        // stamps are processed normally; a feed without a (parseable) stamp always is.
        if let Some(incoming) = feed_updated.filter(|_| !alerts.is_empty()) {
            let held: Option<DateTime<Utc>> =
                sqlx::query_scalar("SELECT feed_updated_at FROM nws_alerts_status WHERE id = 1")
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(db("status read"))?
                    .flatten();
            if let Some(held) = held.filter(|h| incoming < *h) {
                warn!(%incoming, %held, "NWS alerts feed is older than the one already applied; ignoring it");
                sqlx::query(
                    "INSERT INTO nws_alerts_status (id, last_attempt_at, last_success_at) VALUES (1, $1, $1) \
                     ON CONFLICT (id) DO UPDATE SET last_attempt_at = $1, last_success_at = $1",
                )
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(db("status"))?;
                tx.commit().await.map_err(db("commit"))?;
                res.stale_ignored = true;
                res.missing_zones = self.missing_zones(now).await?;
                return Ok(res);
            }
        }

        // What the database holds for the feed's alerts, in one read. NWS' `updated` stamp moves
        // every minute while nearly every alert in the feed is the message it was the minute
        // before, and rewriting all of them (their text, their polygons parsed again, every index
        // entry) is the cost an unchanged feed must not have.
        let ids: Vec<&str> = alerts.iter().map(|a| a.id.as_str()).collect();
        let held: HashMap<String, HeldAlert> = sqlx::query_as::<_, HeldAlert>(
            "SELECT id, content_hash, ended_at IS NOT NULL AS ended FROM nws_alerts WHERE id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(db("stored alerts"))?
        .into_iter()
        .map(|h| (h.id.clone(), h))
        .collect();

        // The unchanged ones only need their `last_seen_at` moved, all in one statement. (An id
        // listed twice is written both times, the last copy winning.)
        let mut seen: HashSet<&str> = HashSet::new();
        let mut unchanged: Vec<bool> = alerts
            .iter()
            .map(|a| seen.insert(a.id.as_str()) && held.get(&a.id).is_some_and(|h| h.is_current(a)))
            .collect();
        let touch: Vec<&str> = alerts
            .iter()
            .zip(&unchanged)
            .filter_map(|(a, same)| same.then_some(a.id.as_str()))
            .collect();
        if !touch.is_empty() && !touch_alerts(&mut tx, &touch, now).await? {
            // The database refused one of them: store them one at a time, where it is skipped alone.
            unchanged.fill(false);
        }
        res.unchanged = unchanged.iter().filter(|same| **same).count();

        // The alerts of the feed that are in the table as they should be, in feed order.
        let mut stored_alerts: Vec<&ParsedAlert> = Vec::with_capacity(alerts.len());
        for (a, same) in alerts.iter().zip(&unchanged) {
            if *same {
                stored_alerts.push(a);
                continue;
            }
            // Each alert is stored under its own savepoint: one the database rejects (a polygon
            // PostGIS cannot parse, text it cannot hold) must not abort the rest of the feed.
            let stored = match upsert_alert_isolated(&mut tx, a, a.geometry_json.as_deref(), now)
                .await
            {
                Ok(()) => Stored::Full,
                Err(e) if !is_row_error(&e) => return Err(db("upsert")(e)),
                Err(e) if a.geometry_json.is_some() => {
                    warn!(alert = %a.id, error = %e, "NWS alert rejected by the database; retrying without its own polygon");
                    match upsert_alert_isolated(&mut tx, a, None, now).await {
                        Ok(()) => Stored::WithoutPolygon,
                        Err(e) if !is_row_error(&e) => return Err(db("upsert")(e)),
                        Err(e) => {
                            warn!(alert = %a.id, error = %e, "NWS alert skipped: the database rejects it even without its polygon");
                            Stored::Skipped
                        }
                    }
                }
                Err(e) => {
                    warn!(alert = %a.id, error = %e, "NWS alert skipped: the database rejected it");
                    Stored::Skipped
                }
            };
            match stored {
                Stored::Full => res.upserted += 1,
                Stored::WithoutPolygon => {
                    res.upserted += 1;
                    res.geometry_rejected += 1;
                }
                Stored::Skipped => {
                    // Not stored, so it supersedes nothing; an older stored version of it stays
                    // as it was (its id is in the feed, so the sweep below does not end it).
                    res.failed += 1;
                    continue;
                }
            }

            stored_alerts.push(a);
        }

        res.superseded = supersede_replaced(&mut tx, &stored_alerts, now).await?;

        if !alerts.is_empty() {
            let r = sqlx::query(
                "UPDATE nws_alerts SET ended_at = $2 \
                 WHERE ended_at IS NULL AND superseded_by IS NULL AND id <> ALL($1)",
            )
            .bind(&ids)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db("end disappeared"))?;
            res.ended = r.rows_affected();

            // A polygon that cleaned to nothing falls back to its zones.
            sqlx::query(
                "UPDATE nws_alerts SET geometry_source = 'pending' \
                 WHERE id = ANY($1) AND geometry_source = 'polygon' AND geom IS NULL",
            )
            .bind(&ids)
            .execute(&mut *tx)
            .await
            .map_err(db("polygon fallback"))?;
            // Every alert still waiting on zones, not just this feed's with no geometry yet: one
            // left half-resolved (partial geometry) is finished here once its zones all exist.
            let pending: Vec<String> =
                sqlx::query_scalar("SELECT id FROM nws_alerts WHERE geometry_source = 'pending'")
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(db("pending alerts"))?;
            res.geometry_failed = recompute_alerts(&mut tx, &pending).await?.1;

            sqlx::query(
                "INSERT INTO nws_alerts_status (id, last_attempt_at, last_success_at, last_change_at, \
                    feed_updated_at, last_alert_count) VALUES (1, $1, $1, $1, $2, $3) \
                 ON CONFLICT (id) DO UPDATE SET last_attempt_at = $1, last_success_at = $1, \
                    last_change_at = $1, feed_updated_at = COALESCE($2, nws_alerts_status.feed_updated_at), \
                    last_alert_count = $3",
            )
            .bind(now)
            .bind(feed_updated)
            .bind(alerts.len() as i32)
            .execute(&mut *tx)
            .await
            .map_err(db("status"))?;
        }

        tx.commit().await.map_err(db("commit"))?;
        res.missing_zones = self.missing_zones(now).await?;
        Ok(res)
    }

    /// The feed was unchanged (HTTP 304): still a success. Returns the zones still to fetch.
    pub async fn record_not_modified(&self, now: DateTime<Utc>) -> WmsResult<Vec<String>> {
        sqlx::query(
            "INSERT INTO nws_alerts_status (id, last_attempt_at, last_success_at) VALUES (1, $1, $1) \
             ON CONFLICT (id) DO UPDATE SET last_attempt_at = $1, last_success_at = $1",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db("status"))?;
        self.missing_zones(now).await
    }

    /// A poll failed (timeout, 5xx, garbage). Stored alerts are left exactly as
    /// they were; only the status row changes.
    pub async fn record_poll_failure(&self, error: &str, now: DateTime<Utc>) -> WmsResult<()> {
        sqlx::query(
            "INSERT INTO nws_alerts_status (id, last_attempt_at, last_error, last_error_at) \
             VALUES (1, $1, $2, $1) \
             ON CONFLICT (id) DO UPDATE SET last_attempt_at = $1, last_error = $2, last_error_at = $1",
        )
        .bind(now)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(db("status"))?;
        Ok(())
    }

    /// Zones that live alerts (not carrying their own polygon) reference but that
    /// have no fresh row: never fetched, `ok` older than [`ZONE_REFRESH_DAYS`], a
    /// transient `error` older than [`ZONE_ERROR_RETRY_MINUTES`], or any other
    /// outcome without geometry (404, none, invalid) older than [`ZONE_RETRY_HOURS`].
    pub async fn missing_zones(&self, now: DateTime<Utc>) -> WmsResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT DISTINCT k FROM nws_alerts a \
               CROSS JOIN LATERAL unnest(a.zone_keys) AS k \
               LEFT JOIN nws_zones z ON z.zone_key = k \
             WHERE a.ended_at IS NULL AND a.superseded_by IS NULL AND a.geometry_source <> 'polygon' \
               AND (z.zone_key IS NULL \
                    OR (z.fetch_status = 'ok' AND z.fetched_at < $1) \
                    OR (z.fetch_status = 'error' AND z.fetched_at < $3) \
                    OR (z.fetch_status NOT IN ('ok', 'error') AND z.fetched_at < $2)) \
             ORDER BY k",
        )
        .bind(now - Duration::days(ZONE_REFRESH_DAYS))
        .bind(now - Duration::hours(ZONE_RETRY_HOURS))
        .bind(now - Duration::minutes(ZONE_ERROR_RETRY_MINUTES))
        .fetch_all(&self.pool)
        .await
        .map_err(db("missing zones"))
    }

    /// Store fetched zones and recompute the geometry of every alert that uses them.
    ///
    /// Each zone and each alert recomputation runs under its own savepoint, so one zone
    /// PostGIS rejects (stored as `invalid_geometry`, no geometry) or one union that fails
    /// cannot take the rest of the batch down with it.
    pub async fn upsert_zones(
        &self,
        zones: &[ZoneRecord],
        now: DateTime<Utc>,
    ) -> WmsResult<ZonesResult> {
        let mut res = ZonesResult::default();
        if zones.is_empty() {
            return Ok(res);
        }
        let mut tx = self.pool.begin().await.map_err(db("begin"))?;
        lock_writers(&mut tx).await?;
        for z in zones {
            let stored = match upsert_zone_isolated(
                &mut tx,
                z,
                z.geometry_json.as_deref(),
                z.status,
                now,
            )
            .await
            {
                Ok(()) => Some(false),
                Err(e) if !is_row_error(&e) => return Err(db("zone upsert")(e)),
                Err(e) if z.geometry_json.is_some() => {
                    warn!(zone = %z.zone_key, error = %e, "NWS zone geometry rejected by the database; storing the zone without it");
                    match upsert_zone_isolated(&mut tx, z, None, ZoneStatus::InvalidGeometry, now)
                        .await
                    {
                        Ok(()) => Some(true),
                        Err(e) if !is_row_error(&e) => return Err(db("zone upsert")(e)),
                        Err(e) => {
                            warn!(zone = %z.zone_key, error = %e, "NWS zone skipped: the database rejects it even without geometry");
                            None
                        }
                    }
                }
                Err(e) => {
                    warn!(zone = %z.zone_key, error = %e, "NWS zone skipped: the database rejected it");
                    None
                }
            };
            match stored {
                Some(invalid) => {
                    res.stored += 1;
                    res.invalid_geometry += usize::from(invalid);
                }
                None => res.failed += 1,
            }
        }
        let keys: Vec<&str> = zones.iter().map(|z| z.zone_key.as_str()).collect();
        let affected: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM nws_alerts WHERE zone_keys && $1 AND geometry_source IN ('pending', 'zones', 'none')",
        )
        .bind(&keys)
        .fetch_all(&mut *tx)
        .await
        .map_err(db("affected alerts"))?;
        let (updated, failed) = recompute_alerts(&mut tx, &affected).await?;
        res.alerts_updated = updated;
        res.alerts_failed = failed;
        tx.commit().await.map_err(db("commit"))?;
        Ok(res)
    }

    /// Delete alerts that ended (or expired, or were superseded) before `cutoff` and
    /// have not been listed by NWS since. Zones are kept.
    pub async fn delete_retired_before(&self, cutoff: DateTime<Utc>) -> WmsResult<u64> {
        let r = sqlx::query(
            "DELETE FROM nws_alerts WHERE last_seen_at < $1 \
               AND COALESCE(LEAST(end_time, ended_at, superseded_at), last_seen_at) < $1",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .map_err(db("retention"))?;
        Ok(r.rows_affected())
    }

    // --- reads (what the EDR layer will use) ----------------------------------

    pub async fn status(&self) -> WmsResult<Option<FeedStatus>> {
        let mut tx = self.read_tx().await?;
        let status = sqlx::query_as::<_, FeedStatus>(
            "SELECT last_attempt_at, last_success_at, last_change_at, feed_updated_at, \
                    last_alert_count, last_error, last_error_at FROM nws_alerts_status WHERE id = 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(read_db("status read"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(status)
    }

    /// `Actual` alerts in force at a point, worst first.
    pub async fn active_at_point(
        &self,
        lon: f64,
        lat: f64,
        now: DateTime<Utc>,
        with_geometry: bool,
    ) -> WmsResult<Vec<StoredAlert>> {
        let sql = format!(
            "SELECT {COLS}, {geom} AS geometry_geojson FROM nws_alerts a \
             WHERE {active} AND {actual} \
               AND a.geom && ST_SetSRID(ST_MakePoint($1, $2), 4326) \
               AND ST_Intersects(a.geom, ST_SetSRID(ST_MakePoint($1, $2), 4326)) \
             ORDER BY {SEVERITY_ORDER}",
            geom = geometry_json_sql(with_geometry, None),
            active = active_predicate("$3"),
            actual = actual_only(false),
        );
        let mut tx = self.read_tx().await?;
        let rows = sqlx::query_as(&sql)
            .bind(lon)
            .bind(lat)
            .bind(now)
            .fetch_all(&mut *tx)
            .await
            .map_err(read_db("point query"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(rows)
    }

    /// `Actual` alerts intersecting a lon/lat box whose life overlaps `[from, to]`.
    /// `active_only` restricts to those in force at `now`; otherwise ended
    /// alerts are included, and superseded messages only if `include_superseded`.
    #[allow(clippy::too_many_arguments)]
    pub async fn in_bbox(
        &self,
        bbox: [f64; 4],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        active_only: bool,
        include_superseded: bool,
        now: DateTime<Utc>,
        limit: i64,
        with_geometry: bool,
    ) -> WmsResult<Vec<StoredAlert>> {
        let filter = if active_only {
            active_predicate("$7")
        } else if include_superseded {
            "TRUE".to_string()
        } else {
            "a.superseded_by IS NULL".to_string()
        };
        let sql = format!(
            "SELECT {COLS}, {geom} AS geometry_geojson FROM nws_alerts a \
             WHERE {filter} AND {actual} \
               AND a.geom && ST_MakeEnvelope($1, $2, $3, $4, 4326) \
               AND ST_Intersects(a.geom, ST_MakeEnvelope($1, $2, $3, $4, 4326)) \
               AND {overlap} \
             ORDER BY {SEVERITY_ORDER} LIMIT $8",
            geom = geometry_json_sql(with_geometry, None),
            actual = actual_only(false),
            overlap = life_overlaps("$5", "$6"),
        );
        let mut tx = self.read_tx().await?;
        let rows = sqlx::query_as(&sql)
            .bind(bbox[0])
            .bind(bbox[1])
            .bind(bbox[2])
            .bind(bbox[3])
            .bind(from)
            .bind(to)
            .bind(now)
            .bind(limit)
            .fetch_all(&mut *tx)
            .await
            .map_err(read_db("bbox query"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(rows)
    }

    /// Filtered search over a point, box, polygon or radius; worst alerts first.
    /// Only CAP `Actual` alerts unless `include_non_actual`.
    pub async fn search(&self, q: &AlertSearch) -> WmsResult<Vec<StoredAlert>> {
        let sql = search_sql(q);
        let lower = |v: &[String]| v.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>();
        let (from, to) = match q.window {
            Some((f, t)) => (Some(f), Some(t)),
            None => (None, None),
        };
        let query = sqlx::query_as(&sql)
            .bind(q.now)
            .bind(from)
            .bind(to)
            .bind(lower(&q.severities))
            .bind(lower(&q.events))
            .bind(q.limit);
        let query = match &q.region {
            AlertRegion::Point { lon, lat } => query.bind(*lon).bind(*lat),
            AlertRegion::Bbox(b) => query.bind(b[0]).bind(b[1]).bind(b[2]).bind(b[3]),
            AlertRegion::Polygon { wkt } => query.bind(wkt.clone()),
            AlertRegion::Radius { lon, lat, meters } => radius_boxes(*lon, *lat, *meters)
                .iter()
                .flatten()
                .fold(query.bind(*lon).bind(*lat).bind(*meters), |query, edge| {
                    query.bind(*edge)
                }),
        };
        let mut tx = self.read_tx().await?;
        let rows = query.fetch_all(&mut *tx).await.map_err(read_db("search"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(rows)
    }

    /// One alert by NWS id (any state).
    pub async fn get(&self, id: &str, with_geometry: bool) -> WmsResult<Option<StoredAlert>> {
        let sql = format!(
            "SELECT {COLS}, {geom} AS geometry_geojson FROM nws_alerts a WHERE a.id = $1",
            geom = geometry_json_sql(with_geometry, None),
        );
        let mut tx = self.read_tx().await?;
        let alert = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(read_db("get"))?;
        tx.commit().await.map_err(db("commit"))?;
        Ok(alert)
    }
}

/// The statement [`NwsAlertCatalog::search`] runs, its `$n` placeholders unbound (`$1` is `now`,
/// `$2`/`$3` the window, `$4` severities, `$5` events, `$6` the limit, `$7..` the region). Public
/// so plan checks can ask Postgres how it would run it.
pub fn search_sql(q: &AlertSearch) -> String {
    let geom = geometry_json_sql(q.with_geometry, q.simplify);
    let time = time_predicate(q);
    // A polygon is parsed and made valid once, in a CTE the planner may not fold back into
    // the predicates, not once per candidate row in each of them.
    let (with, from, region) = match &q.region {
        AlertRegion::Point { .. } => (
            "",
            "nws_alerts a",
            "a.geom && ST_SetSRID(ST_MakePoint($7, $8), 4326) \
             AND ST_Intersects(a.geom, ST_SetSRID(ST_MakePoint($7, $8), 4326))"
                .to_string(),
        ),
        // A box across the antimeridian is two boxes: the index finds what meets either (one
        // bitmap-OR), then the exact test.
        AlertRegion::Bbox(b) if b[0] > b[2] => {
            let (east, west) = (
                "ST_MakeEnvelope($7, $8, 180, $10, 4326)",
                "ST_MakeEnvelope(-180, $8, $9, $10, 4326)",
            );
            (
                "",
                "nws_alerts a",
                format!(
                    "(a.geom && {east} OR a.geom && {west}) \
                     AND (ST_Intersects(a.geom, {east}) OR ST_Intersects(a.geom, {west}))"
                ),
            )
        }
        AlertRegion::Bbox(_) => (
            "",
            "nws_alerts a",
            "a.geom && ST_MakeEnvelope($7, $8, $9, $10, 4326) \
             AND ST_Intersects(a.geom, ST_MakeEnvelope($7, $8, $9, $10, 4326))"
                .to_string(),
        ),
        AlertRegion::Polygon { .. } => (
            "WITH q AS MATERIALIZED (SELECT ST_MakeValid(ST_GeomFromText($7, 4326)) AS g) ",
            "nws_alerts a, q",
            "a.geom && q.g AND ST_Intersects(a.geom, q.g)".to_string(),
        ),
        // The planar boxes (see `radius_boxes`) only let the index discard what is far away; the
        // distance itself is geodetic, so it is right across the antimeridian and at the poles.
        AlertRegion::Radius { lon, lat, meters } => {
            let boxes = (0..radius_boxes(*lon, *lat, *meters).len())
                .map(|i| {
                    let p = 10 + 4 * i;
                    format!(
                        "a.geom && ST_MakeEnvelope(${p}, ${}, ${}, ${}, 4326)",
                        p + 1,
                        p + 2,
                        p + 3
                    )
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            (
                "",
                "nws_alerts a",
                format!(
                    "({boxes}) AND ST_DWithin(a.geom::geography, \
                     ST_SetSRID(ST_MakePoint($7, $8), 4326)::geography, $9)"
                ),
            )
        }
    };
    format!(
        "{with}SELECT {COLS}, {geom} AS geometry_geojson FROM {from} \
         WHERE {time} AND {actual} AND {region} \
           AND (cardinality($4::text[]) = 0 OR lower(a.severity) = ANY($4)) \
           AND (cardinality($5::text[]) = 0 OR lower(a.event) = ANY($5)) \
         ORDER BY {SEVERITY_ORDER} LIMIT $6",
        actual = actual_only(q.include_non_actual),
    )
}

/// Longitude/latitude boxes `[west, south, east, north]` that together cover every point within
/// `meters` of `(lon, lat)`: the planar prefilter the GiST index answers before the geodetic
/// `ST_DWithin` runs. It must never drop a match, so it is generous: an arc degree is taken at its
/// shortest (110.57 km, on the spheroid) and the width gets a margin.
///
/// One box, or two when the circle crosses the antimeridian (the part beyond 180 comes back in
/// from -180, and the alerts NWS splits at the line are on either side of it), or every longitude
/// when the circle reaches a pole. A single box at `lon +- deg` would go past +-180 and never meet
/// a geometry on the far side.
pub fn radius_boxes(lon: f64, lat: f64, meters: f64) -> Vec<[f64; 4]> {
    let r = meters / 110_574.0;
    let (south, north) = ((lat - r).max(-90.0), (lat + r).min(90.0));
    // Half the width of a circle of angular radius r centred at lat (which has no pole inside it).
    let half = if lat.abs() + r >= 90.0 {
        180.0
    } else {
        (r.to_radians().sin() / lat.to_radians().cos())
            .asin()
            .to_degrees()
            + 0.01
    };
    let (west, east) = (lon - half, lon + half);
    if half >= 180.0 {
        vec![[-180.0, south, 180.0, north]]
    } else if west < -180.0 {
        vec![
            [-180.0, south, east, north],
            [west + 360.0, south, 180.0, north],
        ]
    } else if east > 180.0 {
        vec![
            [west, south, 180.0, north],
            [-180.0, south, east - 360.0, north],
        ]
    } else {
        vec![[west, south, east, north]]
    }
}

/// Take the writers' lock for the rest of the transaction. Feed polls and zone batches both
/// write alert geometry from what the other writes (a poll stores alerts that need zones, a batch
/// stores zones that alerts need); run side by side, each can miss the other's uncommitted rows
/// and leave an alert half-resolved, and their row locks can be taken in opposite orders. One at
/// a time, each sees everything the other committed.
async fn lock_writers(conn: &mut PgConnection) -> WmsResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(NWS_WRITE_LOCK_KEY)
        .execute(conn)
        .await
        .map_err(db("writers lock"))?;
    Ok(())
}

/// What a poll needs of a stored alert to tell whether upserting the feed's copy would change it.
#[derive(sqlx::FromRow)]
struct HeldAlert {
    id: String,
    content_hash: Option<String>,
    /// `ended_at` is set.
    ended: bool,
}

impl HeldAlert {
    /// Upserting `a` over this row would change nothing but `last_seen_at`: the row was derived
    /// from the very feature `a` was (same hash, which includes the derivation version), and it
    /// is ended exactly when the upsert would leave it ended: a message that is not over on
    /// arrival and was marked ended because a poll missed it is revived by the upsert. Geometry
    /// is not compared: it is a function of the feature and the zones, and the zone writers
    /// (`upsert_zones`, the pending recompute) keep it current for alerts nobody rewrites.
    fn is_current(&self, a: &ParsedAlert) -> bool {
        a.content_hash.is_some()
            && self.content_hash == a.content_hash
            && self.ended == a.ended_on_arrival
    }
}

/// Give the alerts `ids` a fresh `last_seen_at`, all in one statement under a savepoint. `false`
/// when the database refused one of them (a row error): nothing was changed, and the caller
/// stores them one at a time instead.
async fn touch_alerts(
    conn: &mut PgConnection,
    ids: &[&str],
    now: DateTime<Utc>,
) -> WmsResult<bool> {
    savepoint(conn).await.map_err(db("last seen"))?;
    let r = sqlx::query("UPDATE nws_alerts SET last_seen_at = $2 WHERE id = ANY($1)")
        .bind(ids)
        .bind(now)
        .execute(&mut *conn)
        .await;
    match finish_savepoint(conn, r).await {
        Ok(_) => Ok(true),
        Err(e) if is_row_error(&e) => {
            warn!(alerts = ids.len(), error = %e, "NWS alerts: the database refused to update the last-seen time of one; storing them one at a time");
            Ok(false)
        }
        Err(e) => Err(db("last seen")(e)),
    }
}

/// The moment a message takes effect as far as other messages are concerned: when it was issued
/// (`effective`, else `sent`), never later than this poll (an issue time still ahead is not waited
/// for). A replaced message is over from then, and so is a cancellation, which is never in force.
fn issued_by(a: &ParsedAlert, now: DateTime<Utc>) -> DateTime<Utc> {
    a.effective.or(a.sent).map_or(now, |t| t.min(now))
}

/// Mark as superseded the older messages that `alerts` (in feed order) replace, in one statement.
/// A message that several of them reference (each update names the whole chain before it) is
/// replaced by the one issued first, the feed order breaking a tie: it is the first to end the old
/// message, and NWS lists newest first, so the first listed is usually the wrong one. One that
/// arrives in the same feed as its replacement is superseded by it in this very poll whichever
/// comes first. Returns how many messages were superseded.
async fn supersede_replaced(
    conn: &mut PgConnection,
    alerts: &[&ParsedAlert],
    now: DateTime<Utc>,
) -> WmsResult<u64> {
    let (mut targets, mut by, mut at) = (Vec::new(), Vec::new(), Vec::new());
    for a in alerts {
        // The old message is over from the moment the new one was issued, not from the poll that
        // first showed it, so no instant in between has both in force.
        let replaced_at = issued_by(a, now);
        for t in lifecycle::supersede_targets(&a.id, &a.references) {
            targets.push(t);
            by.push(a.id.as_str());
            at.push(replaced_at);
        }
    }
    if targets.is_empty() {
        return Ok(0);
    }
    let r = sqlx::query(
        "UPDATE nws_alerts o SET superseded_by = s.by_id, superseded_at = s.replaced_at \
         FROM (SELECT DISTINCT ON (target) target, by_id, replaced_at \
               FROM unnest($1::text[], $2::text[], $3::timestamptz[]) \
                    WITH ORDINALITY AS u(target, by_id, replaced_at, n) \
               ORDER BY target, replaced_at, n) s \
         WHERE o.id = s.target AND o.superseded_by IS NULL",
    )
    .bind(&targets)
    .bind(&by)
    .bind(&at)
    .execute(conn)
    .await
    .map_err(db("supersede"))?;
    Ok(r.rows_affected())
}

/// How one alert of a feed was stored.
enum Stored {
    Full,
    /// Its own polygon was rejected; stored without it.
    WithoutPolygon,
    Skipped,
}

/// Errors that belong to one row's data (data exceptions, constraint violations, PostGIS/GEOS
/// "internal" errors on bad shapes), as opposed to the connection or the transaction being in
/// trouble. Only these are isolated and skipped; anything else fails the call.
fn is_row_error(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c.starts_with("22") || c.starts_with("23") || c.starts_with("XX"))
}

async fn savepoint(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SAVEPOINT nws_row").execute(conn).await?;
    Ok(())
}

/// Release the savepoint when `r` is Ok; roll back to it (and pop it) when it is not, so the
/// surrounding transaction is usable again. If the rollback itself fails the connection is in
/// trouble and that error is the one returned.
async fn finish_savepoint<T>(
    conn: &mut PgConnection,
    r: Result<T, sqlx::Error>,
) -> Result<T, sqlx::Error> {
    match r {
        Ok(v) => {
            sqlx::query("RELEASE SAVEPOINT nws_row")
                .execute(conn)
                .await?;
            Ok(v)
        }
        Err(e) => {
            sqlx::query("ROLLBACK TO SAVEPOINT nws_row")
                .execute(&mut *conn)
                .await?;
            sqlx::query("RELEASE SAVEPOINT nws_row")
                .execute(conn)
                .await?;
            Err(e)
        }
    }
}

async fn upsert_alert_isolated(
    conn: &mut PgConnection,
    a: &ParsedAlert,
    polygon: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    savepoint(conn).await?;
    let r = upsert_alert(conn, a, polygon, now).await;
    finish_savepoint(conn, r).await
}

/// `polygon` is the alert's own GeoJSON geometry (passed separately so a rejected one can be
/// retried as `None`).
async fn upsert_alert(
    conn: &mut PgConnection,
    a: &ParsedAlert,
    polygon: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    // Stored without the polygon NWS sent, the row is not what the feature says, so it gets no
    // hash: the next poll tries the polygon again, as it did before alerts were hashed.
    let content_hash = a
        .content_hash
        .as_deref()
        .filter(|_| polygon.is_some() == a.geometry_json.is_some());
    sqlx::query(UPSERT_ALERT_SQL)
        .bind(&a.id)
        .bind(&a.event)
        .bind(&a.severity)
        .bind(&a.urgency)
        .bind(&a.certainty)
        .bind(&a.category)
        .bind(&a.status)
        .bind(&a.message_type)
        .bind(&a.response)
        .bind(&a.headline)
        .bind(&a.nws_headline)
        .bind(&a.description)
        .bind(&a.instruction)
        .bind(&a.description_text)
        .bind(&a.instruction_text)
        .bind(&a.area_desc)
        .bind(&a.sender)
        .bind(&a.sender_name)
        .bind(&a.web)
        .bind(a.sent)
        .bind(a.effective)
        .bind(a.onset)
        .bind(a.expires)
        .bind(a.ends)
        .bind(&a.ugc)
        .bind(&a.same)
        .bind(&a.zone_keys)
        .bind(a.is_marine)
        .bind(serde_json::to_value(&a.vtec).unwrap_or(Value::Null))
        .bind(a.vtec.first().map(|v| v.event_key()))
        .bind(a.vtec.first().map(|v| v.office.clone()))
        .bind(a.vtec.first().map(|v| v.phenomenon.clone()))
        .bind(a.vtec.first().map(|v| v.significance.to_string()))
        .bind(a.vtec.first().map(|v| v.action.clone()))
        .bind(a.vtec.first().map(|v| v.etn as i32))
        .bind(&a.sections)
        .bind(&a.parameters)
        .bind(&a.color)
        .bind(&a.references)
        .bind(polygon)
        // A message that is over on arrival never had a life: it ends the moment it was issued,
        // not at the poll that first showed it, or it would be "in force" in between.
        .bind(a.ended_on_arrival.then(|| issued_by(a, now)))
        .bind(now)
        .bind(content_hash)
        .execute(conn)
        .await?;
    Ok(())
}

async fn upsert_zone_isolated(
    conn: &mut PgConnection,
    z: &ZoneRecord,
    geometry: Option<&str>,
    status: ZoneStatus,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    savepoint(conn).await?;
    let r = upsert_zone(conn, z, geometry, status, now).await;
    finish_savepoint(conn, r).await
}

async fn upsert_zone(
    conn: &mut PgConnection,
    z: &ZoneRecord,
    geometry: Option<&str>,
    status: ZoneStatus,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let (ty, id) = z.zone_key.split_once('/').unwrap_or(("", &z.zone_key));
    sqlx::query(
        "INSERT INTO nws_zones (zone_key, zone_type, zone_id, name, state, kind, geom, fetch_status, fetched_at) \
         VALUES ($1, $2, $3, $4, $5, $6, \
            nws_clean_geom(ST_SetSRID(ST_GeomFromGeoJSON($7::text), 4326)), $8, $9) \
         ON CONFLICT (zone_key) DO UPDATE SET \
            name = COALESCE(EXCLUDED.name, nws_zones.name), \
            state = COALESCE(EXCLUDED.state, nws_zones.state), \
            kind = COALESCE(EXCLUDED.kind, nws_zones.kind), \
            geom = COALESCE(EXCLUDED.geom, nws_zones.geom), \
            fetch_status = CASE WHEN EXCLUDED.geom IS NULL AND nws_zones.geom IS NOT NULL \
                                     AND EXCLUDED.fetch_status <> 'error' \
                                THEN nws_zones.fetch_status ELSE EXCLUDED.fetch_status END, \
            fetched_at = EXCLUDED.fetched_at",
    )
    .bind(&z.zone_key)
    .bind(ty)
    .bind(id)
    .bind(&z.name)
    .bind(&z.state)
    .bind(&z.kind)
    .bind(geometry)
    .bind(status.as_str())
    .bind(now)
    .execute(conn)
    .await?;
    Ok(())
}

/// Recompute the geometry of these zone-based alerts. One statement under a savepoint in the
/// normal case; if the database rejects it (a union GEOS cannot compute), redo it alert by alert
/// so the rest still get theirs, and park the culprit with no geometry. Returns
/// `(recomputed, failed)`.
async fn recompute_alerts(conn: &mut PgConnection, ids: &[String]) -> WmsResult<(u64, usize)> {
    if ids.is_empty() {
        return Ok((0, 0));
    }
    match recompute_isolated(&mut *conn, ids).await {
        Ok(n) => return Ok((n, 0)),
        Err(e) if !is_row_error(&e) => return Err(db("geometry")(e)),
        Err(e) => {
            warn!(alerts = ids.len(), error = %e, "NWS alert geometry recompute failed; redoing it one alert at a time")
        }
    }
    let (mut done, mut failed) = (0u64, 0usize);
    for id in ids {
        match recompute_isolated(&mut *conn, std::slice::from_ref(id)).await {
            Ok(n) => done += n,
            Err(e) if !is_row_error(&e) => return Err(db("geometry")(e)),
            Err(e) => {
                warn!(alert = %id, error = %e, "NWS alert geometry could not be computed from its zones; leaving it without");
                sqlx::query(
                    "UPDATE nws_alerts SET geom = NULL, geometry_source = 'none' WHERE id = $1",
                )
                .bind(id)
                .execute(&mut *conn)
                .await
                .map_err(db("geometry"))?;
                failed += 1;
            }
        }
    }
    Ok((done, failed))
}

async fn recompute_isolated(conn: &mut PgConnection, ids: &[String]) -> Result<u64, sqlx::Error> {
    savepoint(conn).await?;
    let r = sqlx::query(&recompute_geometry_sql("a2.id = ANY($1)"))
        .bind(ids)
        .execute(&mut *conn)
        .await
        .map(|r| r.rows_affected());
    finish_savepoint(conn, r).await
}

/// Recompute `geom`/`geometry_source` for the zone-based alerts selected by `cond`
/// (a predicate over alias `a2`; its only parameter is `$1`).
fn recompute_geometry_sql(cond: &str) -> String {
    format!(
        "UPDATE nws_alerts a SET geom = r.g, geometry_source = r.src FROM ( \
           SELECT x.id, x.g, \
                  CASE WHEN x.unresolved > 0 THEN 'pending' WHEN x.g IS NULL THEN 'none' ELSE 'zones' END AS src \
           FROM ( \
             SELECT a2.id, \
                    nws_clean_geom((SELECT ST_UnaryUnion(ST_Collect(z.geom)) FROM nws_zones z \
                                    WHERE z.zone_key = ANY(a2.zone_keys))) AS g, \
                    (SELECT count(*) FROM unnest(a2.zone_keys) AS k \
                      WHERE NOT EXISTS (SELECT 1 FROM nws_zones z WHERE z.zone_key = k \
                                        AND NOT (z.fetch_status = 'error' AND z.geom IS NULL))) AS unresolved \
             FROM nws_alerts a2 WHERE {cond} \
           ) x \
         ) r WHERE a.id = r.id"
    )
}

const UPSERT_ALERT_SQL: &str = r#"
INSERT INTO nws_alerts (
    id, event, severity, urgency, certainty, category, status, message_type, response,
    headline, nws_headline, description, instruction, description_text, instruction_text,
    area_desc, sender, sender_name, web, sent, effective, onset, expires, ends,
    ugc, same, zone_keys, is_marine, vtec, vtec_event_key, vtec_office, vtec_phenomenon,
    vtec_significance, vtec_action, vtec_etn, sections, parameters, color, references_ids,
    geom, geometry_source, ended_at, first_seen_at, last_seen_at, updated_at, content_hash
) VALUES (
    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19,
    $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, $36,
    $37, $38, $39,
    nws_clean_geom(ST_SetSRID(ST_GeomFromGeoJSON($40::text), 4326)),
    CASE WHEN $40::text IS NULL THEN 'pending' ELSE 'polygon' END,
    $41,
    $42, $42, $42, $43
)
ON CONFLICT (id) DO UPDATE SET
    event = EXCLUDED.event, severity = EXCLUDED.severity, urgency = EXCLUDED.urgency,
    certainty = EXCLUDED.certainty, category = EXCLUDED.category, status = EXCLUDED.status,
    message_type = EXCLUDED.message_type, response = EXCLUDED.response,
    headline = EXCLUDED.headline, nws_headline = EXCLUDED.nws_headline,
    description = EXCLUDED.description, instruction = EXCLUDED.instruction,
    description_text = EXCLUDED.description_text, instruction_text = EXCLUDED.instruction_text,
    area_desc = EXCLUDED.area_desc, sender = EXCLUDED.sender, sender_name = EXCLUDED.sender_name,
    web = EXCLUDED.web, sent = EXCLUDED.sent, effective = EXCLUDED.effective,
    onset = EXCLUDED.onset, expires = EXCLUDED.expires, ends = EXCLUDED.ends,
    ugc = EXCLUDED.ugc, same = EXCLUDED.same, is_marine = EXCLUDED.is_marine,
    vtec = EXCLUDED.vtec, vtec_event_key = EXCLUDED.vtec_event_key,
    vtec_office = EXCLUDED.vtec_office, vtec_phenomenon = EXCLUDED.vtec_phenomenon,
    vtec_significance = EXCLUDED.vtec_significance, vtec_action = EXCLUDED.vtec_action,
    vtec_etn = EXCLUDED.vtec_etn, sections = EXCLUDED.sections, parameters = EXCLUDED.parameters,
    color = EXCLUDED.color, references_ids = EXCLUDED.references_ids,
    geom = CASE
        WHEN EXCLUDED.geometry_source = 'polygon' THEN EXCLUDED.geom
        WHEN nws_alerts.geometry_source = 'polygon' OR nws_alerts.zone_keys IS DISTINCT FROM EXCLUDED.zone_keys THEN NULL
        ELSE nws_alerts.geom END,
    geometry_source = CASE
        WHEN EXCLUDED.geometry_source = 'polygon' THEN 'polygon'
        WHEN nws_alerts.geometry_source = 'polygon' OR nws_alerts.zone_keys IS DISTINCT FROM EXCLUDED.zone_keys THEN 'pending'
        ELSE nws_alerts.geometry_source END,
    zone_keys = EXCLUDED.zone_keys,
    content_hash = EXCLUDED.content_hash,
    -- the feed lists it again: it is live unless the message itself is a cancel/expiry, which is
    -- over from its issue time ($41), or from the earlier moment an older write of it recorded
    ended_at = CASE WHEN $41::timestamptz IS NOT NULL THEN LEAST(nws_alerts.ended_at, $41) ELSE NULL END,
    last_seen_at = $42,
    updated_at = CASE WHEN (nws_alerts.description, nws_alerts.headline, nws_alerts.severity,
                            nws_alerts.expires, nws_alerts.ends, nws_alerts.message_type)
                           IS DISTINCT FROM
                          (EXCLUDED.description, EXCLUDED.headline, EXCLUDED.severity,
                           EXCLUDED.expires, EXCLUDED.ends, EXCLUDED.message_type)
                      THEN $42 ELSE nws_alerts.updated_at END
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap()
    }

    fn point_search() -> AlertSearch {
        AlertSearch {
            region: AlertRegion::Point {
                lon: -107.0,
                lat: 37.5,
            },
            now: t(),
            window: None,
            include_superseded: false,
            severities: vec![],
            events: vec![],
            limit: 10,
            with_geometry: false,
            simplify: None,
            include_non_actual: false,
        }
    }

    /// The common app read (`/position`, no `datetime`) is the lifecycle-flag predicate the
    /// partial index `idx_nws_alerts_geom_live` is built for. This pins the statement; a change
    /// here changes the plan of the hottest query (the Docker test `a_search_without_a_datetime...`
    /// asks Postgres for it).
    #[test]
    fn no_window_keeps_the_lifecycle_flag_predicate_verbatim() {
        assert_eq!(
            time_predicate(&point_search()),
            "a.superseded_by IS NULL AND a.ended_at IS NULL AND (a.end_time IS NULL OR a.end_time > $1) \
             AND $2::timestamptz IS NULL AND $3::timestamptz IS NULL"
        );
        let sql = search_sql(&point_search());
        assert!(sql.contains("WHERE a.superseded_by IS NULL AND a.ended_at IS NULL AND"));
        assert!(!sql.contains("COALESCE(a.effective"), "{sql}");
    }

    fn covered(boxes: &[[f64; 4]], lon: f64, lat: f64) -> bool {
        boxes
            .iter()
            .any(|b| (b[0]..=b[2]).contains(&lon) && (b[1]..=b[3]).contains(&lat))
    }

    #[test]
    fn a_radius_near_the_antimeridian_is_two_boxes_one_on_each_side() {
        // 50 km at 57.5N is 0.85 degrees of longitude: 179.95 + 0.85 goes over the line
        let b = radius_boxes(179.95, 57.5, 50_000.0);
        assert_eq!(b.len(), 2, "{b:?}");
        assert_eq!((b[0][2], b[1][0]), (180.0, -180.0));
        assert!(b[0][0] > 179.0 && b[1][2] < -179.0 && b[1][2] > -180.0);
        assert_eq!((b[0][1], b[0][3]), (b[1][1], b[1][3]), "the same latitudes");
        // and the mirror image
        let m = radius_boxes(-179.95, 57.5, 50_000.0);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0][0], m[1][2]), (-180.0, 180.0));
        // a circle centred on the line itself
        assert_eq!(radius_boxes(180.0, 0.0, 10_000.0).len(), 2);
        assert_eq!(radius_boxes(-180.0, 0.0, 10_000.0).len(), 2);
        // far from the line it is one ordinary box, around the circle
        let one = radius_boxes(-107.0, 37.5, 50_000.0);
        assert_eq!(one.len(), 1);
        assert!(one[0][0] < -107.5 && one[0][2] > -106.5 && one[0][1] < 37.1 && one[0][3] > 37.9);
        assert!(one[0][0] >= -180.0 && one[0][2] <= 180.0);
        // reaching a pole is every longitude
        let polar = radius_boxes(10.0, 88.0, 300_000.0);
        assert_eq!(polar.len(), 1);
        assert_eq!((polar[0][0], polar[0][2]), (-180.0, 180.0));
        assert_eq!(polar[0][3], 90.0);
        // never outside the world
        for (lon, lat, m) in [
            (179.9, 89.9, 500_000.0),
            (-179.9, -89.0, 500_000.0),
            (0.0, 0.0, 500_000.0),
        ] {
            for b in radius_boxes(lon, lat, m) {
                assert!(b[0] >= -180.0 && b[2] <= 180.0 && b[0] <= b[2], "{b:?}");
                assert!(b[1] >= -90.0 && b[3] <= 90.0 && b[1] <= b[3], "{b:?}");
            }
        }
    }

    /// The boxes are only an index prefilter, so what matters is that no point of the circle is
    /// outside them: walk a circle (on a sphere a touch smaller than the spheroid's, so every
    /// point is really within `meters`) around many centres and radii, the dateline and the poles
    /// among them.
    #[test]
    fn the_boxes_cover_every_point_within_the_radius() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |lo: f64, hi: f64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            lo + (state >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
        };
        let earth = 6_356_752.0; // polar radius: the smallest, so the widest angle per metre
        let mut crossing = 0;
        for i in 0..3000 {
            let lon = match i % 3 {
                0 => next(170.0, 180.0),
                1 => next(-180.0, -170.0),
                _ => next(-180.0, 180.0),
            };
            let lat = if i % 5 == 0 {
                next(80.0, 90.0) * if i % 2 == 0 { 1.0 } else { -1.0 }
            } else {
                next(-75.0, 75.0)
            };
            let meters = next(1_000.0, 500_000.0);
            let boxes = radius_boxes(lon, lat, meters);
            crossing += (boxes.len() == 2) as i32;
            let (phi1, lam1, delta) = (lat.to_radians(), lon.to_radians(), meters / earth);
            for k in 0..72 {
                let brg = (k as f64 * 5.0).to_radians();
                let phi2 = (phi1.sin() * delta.cos() + phi1.cos() * delta.sin() * brg.cos()).asin();
                let lam2 = lam1
                    + (brg.sin() * delta.sin() * phi1.cos())
                        .atan2(delta.cos() - phi1.sin() * phi2.sin());
                let lon2 = (lam2.to_degrees() + 540.0).rem_euclid(360.0) - 180.0;
                assert!(
                    covered(&boxes, lon2, phi2.to_degrees()),
                    "({lon}, {lat}) {meters} m: ({lon2}, {}) is outside {boxes:?}",
                    phi2.to_degrees()
                );
            }
        }
        assert!(
            crossing > 500,
            "the sweep should cross the antimeridian a lot: {crossing}"
        );
    }

    #[test]
    fn a_box_across_the_antimeridian_is_two_envelopes_in_the_sql() {
        let mut q = point_search();
        q.region = AlertRegion::Bbox([170.0, 50.0, -170.0, 60.0]);
        let sql = search_sql(&q);
        assert!(
            sql.contains("ST_MakeEnvelope($7, $8, 180, $10, 4326)"),
            "{sql}"
        );
        assert!(
            sql.contains("ST_MakeEnvelope(-180, $8, $9, $10, 4326)"),
            "{sql}"
        );
        assert!(
            sql.contains("4326) OR a.geom && ST_MakeEnvelope(-180,"),
            "{sql}"
        );
        assert!(
            sql.contains("OR ST_Intersects(a.geom, ST_MakeEnvelope(-180,"),
            "{sql}"
        );
        // an ordinary box is the single envelope, as before
        q.region = AlertRegion::Bbox([-170.0, 50.0, 170.0, 60.0]);
        let sql = search_sql(&q);
        assert!(
            sql.contains("ST_MakeEnvelope($7, $8, $9, $10, 4326)") && !sql.contains(" OR a.geom"),
            "{sql}"
        );
        // a radius across the line has one prefilter arm per box
        q.region = AlertRegion::Radius {
            lon: 179.95,
            lat: 57.5,
            meters: 50_000.0,
        };
        let sql = search_sql(&q);
        assert!(sql.contains("a.geom && ST_MakeEnvelope($10, $11, $12, $13, 4326) OR a.geom && ST_MakeEnvelope($14, $15, $16, $17, 4326)"), "{sql}");
        q.region = AlertRegion::Radius {
            lon: -107.0,
            lat: 37.5,
            meters: 50_000.0,
        };
        assert!(!search_sql(&q).contains("$14"));
    }

    #[test]
    fn an_instant_reads_each_messages_own_life() {
        let at = point_search().in_force_at(t() - chrono::Duration::hours(3));
        assert_eq!(at.window, Some((at.now, at.now)));
        assert_eq!(at.now, t() - chrono::Duration::hours(3));
        let p = time_predicate(&at);
        // replaced and ended messages are not hidden by today's flags...
        assert!(p.starts_with("TRUE AND "), "{p}");
        // ...but by when it happened: issued by the instant, not over at it (end exclusive)
        assert_eq!(
            p,
            "TRUE AND COALESCE(a.effective, a.sent) <= $3 AND \
             COALESCE(LEAST(a.end_time, a.ended_at, a.superseded_at), 'infinity'::timestamptz) > $2"
        );
        // the history window rule is the same predicate with `to` and `from` apart
        let mut win = point_search();
        win.window = Some((t() - chrono::Duration::hours(1), t()));
        assert_eq!(
            time_predicate(&win),
            format!("a.superseded_by IS NULL AND {}", life_overlaps("$2", "$3"))
        );
    }
}
