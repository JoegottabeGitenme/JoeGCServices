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
//! ## Concurrency and bad rows
//!
//! Two writers touch alert geometry: feed polls ([`NwsAlertCatalog::apply_feed`]) and zone
//! batches ([`NwsAlertCatalog::upsert_zones`]). Both take [`NWS_WRITE_LOCK_KEY`] (a
//! transaction-scoped advisory lock) first, so each sees what the other committed. Inside their
//! transaction every alert, every zone and every geometry recomputation runs under its own
//! savepoint: a row the database rejects (a polygon PostGIS cannot parse, a union GEOS cannot
//! compute) is stored without the offending geometry or skipped and counted, never allowed to
//! abort the poll or the batch (see `ApplyResult` / `ZonesResult`).

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
    /// Rows inserted or refreshed.
    pub upserted: usize,
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
    a.superseded_by, a.ended_at, a.geometry_source, a.first_seen_at, a.updated_at, a.last_seen_at";

/// Not superseded, not ended, end time (if any) in the future. `$now` is bound by the caller.
fn active_predicate(now_param: &str) -> String {
    format!(
        "a.superseded_by IS NULL AND a.ended_at IS NULL AND (a.end_time IS NULL OR a.end_time > {now_param})"
    )
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

fn geom_col(with_geometry: bool) -> &'static str {
    if with_geometry {
        "ST_AsGeoJSON(a.geom, 5)"
    } else {
        "NULL::text"
    }
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
    /// `[west, south, east, north]`.
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
    pub now: DateTime<Utc>,
    /// `None`: alerts in force at `now` (not superseded, not ended, end time
    /// null or future; alerts that start later count). `Some((from, to))`:
    /// every alert whose life overlaps the window, ended ones included.
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

#[derive(Clone)]
pub struct NwsAlertCatalog {
    pool: PgPool,
}

impl NwsAlertCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // --- writes ---------------------------------------------------------------

    /// Apply one successful poll of the active feed (every alert NWS lists now).
    ///
    /// In one transaction: upsert each alert; mark the older messages each Update
    /// or Cancel references as superseded; mark every stored, still-live alert
    /// that this feed no longer lists as ended; fill in geometry from the zones
    /// already known; record poll success. An **empty** feed ends nothing (NWS
    /// always has alerts; an empty answer is a fault, not "all clear").
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

        for a in alerts {
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

            let targets = lifecycle::supersede_targets(&a.id, &a.references);
            if !targets.is_empty() {
                let r = sqlx::query(
                    "UPDATE nws_alerts SET superseded_by = $1, superseded_at = $3 \
                     WHERE id = ANY($2) AND superseded_by IS NULL",
                )
                .bind(&a.id)
                .bind(&targets)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(db("supersede"))?;
                res.superseded += r.rows_affected();
            }
        }

        if !alerts.is_empty() {
            let ids: Vec<&str> = alerts.iter().map(|a| a.id.as_str()).collect();
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
            "SELECT id FROM nws_alerts WHERE zone_keys && $1 AND geometry_source IN ('pending', 'zones')",
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
        sqlx::query_as::<_, FeedStatus>(
            "SELECT last_attempt_at, last_success_at, last_change_at, feed_updated_at, \
                    last_alert_count, last_error, last_error_at FROM nws_alerts_status WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(db("status read"))
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
            geom = geom_col(with_geometry),
            active = active_predicate("$3"),
            actual = actual_only(false),
        );
        sqlx::query_as(&sql)
            .bind(lon)
            .bind(lat)
            .bind(now)
            .fetch_all(&self.pool)
            .await
            .map_err(db("point query"))
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
               AND COALESCE(a.effective, a.sent) <= $6 \
               AND COALESCE(LEAST(a.end_time, a.ended_at, a.superseded_at), 'infinity'::timestamptz) >= $5 \
             ORDER BY {SEVERITY_ORDER} LIMIT $8",
            geom = geom_col(with_geometry),
            actual = actual_only(false),
        );
        sqlx::query_as(&sql)
            .bind(bbox[0])
            .bind(bbox[1])
            .bind(bbox[2])
            .bind(bbox[3])
            .bind(from)
            .bind(to)
            .bind(now)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(db("bbox query"))
    }

    /// Filtered search over a point, box, polygon or radius; worst alerts first.
    /// Only CAP `Actual` alerts unless `include_non_actual`.
    pub async fn search(&self, q: &AlertSearch) -> WmsResult<Vec<StoredAlert>> {
        let geom = match q.simplify.filter(|t| t.is_finite() && *t > 0.0) {
            Some(tol) if q.with_geometry => {
                format!("ST_AsGeoJSON(ST_SimplifyPreserveTopology(a.geom, {tol}), 5)")
            }
            _ => geom_col(q.with_geometry).to_string(),
        };
        let time = match q.window {
            None => format!(
                "{} AND $2::timestamptz IS NULL AND $3::timestamptz IS NULL",
                active_predicate("$1")
            ),
            Some(_) => format!(
                "{} AND COALESCE(a.effective, a.sent) <= $3 \
                 AND COALESCE(LEAST(a.end_time, a.ended_at, a.superseded_at), 'infinity'::timestamptz) >= $2",
                if q.include_superseded { "TRUE" } else { "a.superseded_by IS NULL" }
            ),
        };
        let region = match &q.region {
            AlertRegion::Point { .. } => {
                "a.geom && ST_SetSRID(ST_MakePoint($7, $8), 4326) \
                 AND ST_Intersects(a.geom, ST_SetSRID(ST_MakePoint($7, $8), 4326))"
            }
            AlertRegion::Bbox(_) => {
                "a.geom && ST_MakeEnvelope($7, $8, $9, $10, 4326) \
                 AND ST_Intersects(a.geom, ST_MakeEnvelope($7, $8, $9, $10, 4326))"
            }
            AlertRegion::Polygon { .. } => {
                "a.geom && ST_MakeValid(ST_GeomFromText($7, 4326)) \
                 AND ST_Intersects(a.geom, ST_MakeValid(ST_GeomFromText($7, 4326)))"
            }
            AlertRegion::Radius { .. } => {
                "a.geom && ST_Expand(ST_SetSRID(ST_MakePoint($7, $8), 4326), $10) \
                 AND ST_DWithin(a.geom::geography, ST_SetSRID(ST_MakePoint($7, $8), 4326)::geography, $9)"
            }
        };
        let sql = format!(
            "SELECT {COLS}, {geom} AS geometry_geojson FROM nws_alerts a \
             WHERE {time} AND {actual} AND {region} \
               AND (cardinality($4::text[]) = 0 OR lower(a.severity) = ANY($4)) \
               AND (cardinality($5::text[]) = 0 OR lower(a.event) = ANY($5)) \
             ORDER BY {SEVERITY_ORDER} LIMIT $6",
            actual = actual_only(q.include_non_actual),
        );
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
            AlertRegion::Radius { lon, lat, meters } => {
                // Degrees that surely cover `meters` at this latitude (index prefilter only).
                let deg = meters / (111_320.0 * lat.to_radians().cos().abs().max(0.05)) + 0.01;
                query.bind(*lon).bind(*lat).bind(*meters).bind(deg)
            }
        };
        query.fetch_all(&self.pool).await.map_err(db("search"))
    }

    /// One alert by NWS id (any state).
    pub async fn get(&self, id: &str, with_geometry: bool) -> WmsResult<Option<StoredAlert>> {
        let sql = format!(
            "SELECT {COLS}, {geom} AS geometry_geojson FROM nws_alerts a WHERE a.id = $1",
            geom = geom_col(with_geometry),
        );
        sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db("get"))
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
        .bind(a.ended_on_arrival)
        .bind(now)
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
    geom, geometry_source, ended_at, first_seen_at, last_seen_at, updated_at
) VALUES (
    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19,
    $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, $36,
    $37, $38, $39,
    nws_clean_geom(ST_SetSRID(ST_GeomFromGeoJSON($40::text), 4326)),
    CASE WHEN $40::text IS NULL THEN 'pending' ELSE 'polygon' END,
    CASE WHEN $41 THEN $42 ELSE NULL END,
    $42, $42, $42
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
    -- the feed lists it again: it is live unless the message itself is a cancel/expiry
    ended_at = CASE WHEN $41 THEN COALESCE(nws_alerts.ended_at, $42) ELSE NULL END,
    last_seen_at = $42,
    updated_at = CASE WHEN (nws_alerts.description, nws_alerts.headline, nws_alerts.severity,
                            nws_alerts.expires, nws_alerts.ends, nws_alerts.message_type)
                           IS DISTINCT FROM
                          (EXCLUDED.description, EXCLUDED.headline, EXCLUDED.severity,
                           EXCLUDED.expires, EXCLUDED.ends, EXCLUDED.message_type)
                      THEN $42 ELSE nws_alerts.updated_at END
"#;
