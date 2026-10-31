//! Version token, cache-key and cache-lifetime rules for WMTS tiles that depend
//! on a time the server picks ("latest").
//!
//! # Why
//!
//! A tile requested without a time resolves to the newest dataset the catalog
//! holds: the newest radar scan or satellite image, or for a model layer the
//! newest run (the RUN dimension is parsed but never honoured by GetTile, so
//! even a tile with an explicit FORECAST hour comes from the newest run that
//! has that hour). The tile cache key used to carry no trace of that choice, so
//! a request without a time was keyed with the literal `current` and the entry
//! lived for the full L1 (5 min) / L2 Redis (1 h) TTL. After new data arrived,
//! the server kept answering with the old tile ("x-cache: L2-HIT") for up to
//! an hour, however short the HTTP `max-age` was.
//!
//! The fix is to resolve the choice to a concrete *version token* (run and
//! forecast hour of the dataset that would be rendered) BEFORE building the
//! key. New data then means a new key, and old entries are simply never read
//! again; they age out through the normal TTL / LRU.
//!
//! [`TileRequestDims::cache_plan`] is the one place that turns "what the
//! request said + what the catalog resolved" into the three things every
//! caller needs: the `Cache-Control` selection, the key suffix and the cache
//! entry lifetime. GetTile, prefetch and cache warming all go through it, so
//! the rule cannot drift between them.
//!
//! # Cost, and what happens when the catalog is slow or down
//!
//! The lookup is one indexed catalog query (the same one the renderer makes on
//! a miss). Doing it for every request would add a database round trip to
//! every L1 hit, so [`LatestVersionMemo`] holds each answer for
//! [`VERSION_MEMO_TTL`]. That bounds how long a new dataset can go unnoticed
//! by the tile cache to that TTL.
//!
//! The memo is deliberately not a plain TTL map, because the tile caches sit in
//! front of Postgres and must keep working while Postgres does not:
//!
//! - **Single flight.** When an answer expires, one task re-queries; the rest
//!   do not each run their own query (a map shows 20-40 tiles at once).
//! - **Serve stale.** The others get the last good answer immediately instead
//!   of queueing behind the refresh, and a failed or slow refresh (it is cut
//!   off after [`LOOKUP_TIMEOUT`]) keeps the last good answer. Cached latest
//!   tiles therefore stay reachable through a database blip.
//! - **Unversioned only as a last resort.** The key falls back to
//!   [`UNRESOLVED_VERSION`] with a short TTL only for a layer that has never
//!   been resolved (no dataset, or the catalog was never reachable).
//!
//! # Wind barbs
//!
//! `WIND_BARBS` is a composite layer: it has no dataset of its own, the
//! renderer draws it from U/V (or speed/direction) components. Its version is
//! built from the components the renderer actually reads, see
//! [`wind_barb_components`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use storage::{Catalog, CatalogEntry};
use tokio::sync::Mutex as AsyncMutex;
use tracing::debug;
use wms_common::cache_control::{TimeSelection, LATEST_MAX_AGE_SECS};
use wms_common::WmsResult;

/// How long a resolved version is reused before the catalog is asked again.
///
/// Short enough that new data reaches the tile cache within seconds, long
/// enough that a burst of tile requests for one layer costs one query.
pub const VERSION_MEMO_TTL: Duration = Duration::from_secs(10);

/// Longest a request waits for the catalog when refreshing a version.
///
/// A version lookup is an indexed point query (milliseconds). If it takes
/// longer than this the database is struggling, and the request is better
/// served from the last known version (or the short-lived unversioned key)
/// than held up for the pool's 30 s acquire timeout.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_millis(250);

/// How long after a failed or timed-out refresh the memo leaves the catalog
/// alone before trying again, so an outage costs one query per layer every few
/// seconds instead of one per request.
pub const FAILURE_RETRY: Duration = Duration::from_secs(2);

/// Upper bound on memoised lookups. Keys are bounded by configured
/// layers x forecast hours x levels in practice; the cap only guards against a
/// client sweeping arbitrary `elevation=` values.
const VERSION_MEMO_MAX_ENTRIES: usize = 4096;

/// Key suffix for a latest tile whose version could not be resolved.
///
/// It is not `current` on purpose. Entries under `current` were written by
/// older pods (and by startup validation in older builds) with the full 1 h
/// TTL, and old and new pods share one Redis during a rolling deploy. A
/// separate marker means the only entries under it are the ones written here,
/// with [`unresolved_ttl`], so "an unversioned latest entry never outlives its
/// `max-age`" holds by construction.
pub const UNRESOLVED_VERSION: &str = "unresolved";

/// The parameter name of the composite wind barb layers.
pub const WIND_BARBS_PARAMETER: &str = "WIND_BARBS";

/// The catalog parameters the wind barb renderer reads for `model`.
///
/// Mirrors `render_wind_barbs_from_speed_direction_tile` (NBM models carry
/// speed as `WIND`, NDFD as `WSPD`, both with `WDIR`) and
/// `render_wind_barbs_tile_with_level` (U/V components for everything else).
/// Keep it in step with them: the version token must name the datasets that
/// are actually drawn.
pub fn wind_barb_components(model: &str) -> [&'static str; 2] {
    if model.starts_with("nbm") {
        ["WIND", "WDIR"]
    } else if model == "ndfd" {
        ["WSPD", "WDIR"]
    } else {
        ["UGRD", "VGRD"]
    }
}

/// The catalog parameter(s) a layer's tile is drawn from: the parameter itself,
/// or both components for wind barbs.
fn drawn_parameters<'a>(model: &str, parameter: &'a str) -> (&'a str, Option<&'a str>) {
    if parameter == WIND_BARBS_PARAMETER {
        let [first, second] = wind_barb_components(model);
        (first, Some(second))
    } else {
        (parameter, None)
    }
}

/// The dataset a latest request resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestVersion {
    /// Identifies the dataset (run, forecast hour, minute); goes into the key.
    pub token: String,
    /// The model run (`reference_time`) the dataset belongs to, when every
    /// dataset behind the token is from the same run. `None` for a composite
    /// whose components disagree, which can then never satisfy a RUN pin.
    pub run: Option<DateTime<Utc>>,
}

impl LatestVersion {
    fn from_entry(entry: &CatalogEntry) -> Self {
        Self {
            token: version_token(entry),
            run: Some(entry.reference_time),
        }
    }

    /// Two datasets drawn together into one tile (wind U and V). Either one
    /// missing means the tile cannot be versioned.
    fn combine(first: Option<Self>, second: Option<Self>) -> Option<Self> {
        let (first, second) = (first?, second?);
        if first == second {
            return Some(first);
        }
        // Normally U and V arrive together and agree. When one lags, the key
        // must still change when either does.
        Some(Self {
            token: format!("{}-{}", first.token, second.token),
            run: if first.run == second.run {
                first.run
            } else {
                None
            },
        })
    }
}

/// Identifies the dataset a latest request resolves to: run, forecast hour and
/// minute. Contains no `:` or glob characters, so it is safe inside both the
/// `:`-separated L1 key and a Redis key.
pub fn version_token(entry: &CatalogEntry) -> String {
    format!(
        "r{}f{}m{}",
        entry.reference_time.timestamp(),
        entry.forecast_hour,
        entry.forecast_minute
    )
}

/// The dimension part of a tile cache key.
///
/// Keys for explicit requests keep their original shape (`t6`, `obs1700000000`,
/// `t6_2_m_above_ground`, an elevation alone, or none), so nothing already
/// cached or written by other code is invalidated. A latest request that
/// resolved to a `version` gets `@<version>` appended, which is what makes new
/// data a new key.
pub fn tile_dimension_suffix(
    forecast_hour: Option<u32>,
    observation_time: Option<DateTime<Utc>>,
    elevation: Option<&str>,
    version: Option<&str>,
) -> Option<String> {
    let time_key = forecast_hour
        .map(|h| format!("t{}", h))
        .or_else(|| observation_time.map(|t| format!("obs{}", t.timestamp())));
    let elevation_key = elevation.map(|e| e.replace(' ', "_"));
    let base = match (time_key, elevation_key) {
        (Some(t), Some(e)) => Some(format!("{}_{}", t, e)),
        (Some(t), None) => Some(t),
        (None, Some(e)) => Some(e),
        (None, None) => None,
    };
    match version {
        Some(v) => Some(format!("{}@{}", base.unwrap_or_default(), v)),
        None => base,
    }
}

/// The L1 (in-memory) key. Includes the TileMatrixSet so the two projections
/// never collide; the L2 key gets that from its CRS code instead.
pub fn l1_cache_key(
    tile_matrix_set: &str,
    layer: &str,
    style: &str,
    z: u32,
    x: u32,
    y: u32,
    dimension_suffix: Option<&str>,
) -> String {
    format!(
        "{}:{}:{}:{}_{}_{}:{}",
        tile_matrix_set,
        layer,
        style,
        z,
        x,
        y,
        dimension_suffix.unwrap_or("current")
    )
}

/// Lifetime of a tile stored under [`UNRESOLVED_VERSION`]: the latest
/// `max-age`, so such an entry never outlives what the response told the CDN.
pub fn unresolved_ttl() -> Duration {
    Duration::from_secs(LATEST_MAX_AGE_SECS as u64)
}

/// What a tile request said about its time and dimensions.
#[derive(Debug, Clone, Copy)]
pub struct TileRequestDims<'a> {
    /// The layer's model is an observation model (radar, satellite) whose
    /// "time" is a scan time, as opposed to a forecast model's run + hour.
    pub is_observation: bool,
    /// Explicit `TIME`, observation models only.
    pub observation_time: Option<DateTime<Utc>>,
    /// Explicit `RUN`, forecast models only.
    pub run: Option<DateTime<Utc>>,
    pub forecast_hour: Option<u32>,
    pub elevation: Option<&'a str>,
}

/// How a tile is cached, from the request plus what the catalog resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileCachePlan {
    /// Drives `Cache-Control`: explicit keeps its TTL, latest is short.
    pub selection: TimeSelection,
    /// Dimension part of the L1 and L2 keys.
    pub dimension_suffix: Option<String>,
    /// Lifetime override for the L1 and L2 entries; `None` is the cache's own.
    pub entry_ttl: Option<Duration>,
}

impl TileRequestDims<'_> {
    /// Whether the dataset must be looked up to build the key: the request left
    /// it to the server (no `TIME` on an observation layer; any forecast-model
    /// layer, whose run GetTile never pins).
    pub fn needs_latest_lookup(&self) -> bool {
        if self.is_observation {
            self.observation_time.is_none()
        } else {
            true
        }
    }

    /// Cache selection, key suffix and entry lifetime for this request.
    ///
    /// `latest` is what [`resolve_latest_version`] returned; it is ignored when
    /// [`Self::needs_latest_lookup`] is false.
    ///
    /// - Observation layer: explicit exactly when `TIME` was given.
    /// - Forecast layer: GetTile discards RUN (the catalog picks the newest run
    ///   that has the hour), so a tile is latest by default. It is explicit
    ///   when RUN was given AND the dataset resolved is that very run: then the
    ///   URL names what it returns, and a long CDN lifetime is as correct as
    ///   for a pinned scan. When a newer run has landed the tile shows that
    ///   newer run, so it is latest again. (The memo can lag the catalog by
    ///   [`VERSION_MEMO_TTL`], so for that long after a run lands a request for
    ///   the previous run may still be classified explicit while rendering the
    ///   new one; before this change such a URL was cached for an hour
    ///   regardless.)
    ///
    /// The version token is appended to the key whenever a lookup was made,
    /// even for an explicit RUN, so a RUN-pinned request and the plain latest
    /// request share one entry only while they really are the same tile.
    pub fn cache_plan(&self, latest: Option<&LatestVersion>) -> TileCachePlan {
        let looked_up = self.needs_latest_lookup();

        let selection = if self.is_observation {
            TimeSelection::from_explicit(self.observation_time.is_some())
        } else {
            let run_matches = match (self.run, latest.and_then(|v| v.run)) {
                (Some(requested), Some(resolved)) => requested == resolved,
                _ => false,
            };
            TimeSelection::from_explicit(run_matches)
        };

        let version = if looked_up {
            Some(latest.map_or(UNRESOLVED_VERSION, |v| v.token.as_str()))
        } else {
            None
        };
        let unresolved = looked_up && latest.is_none();

        TileCachePlan {
            selection,
            dimension_suffix: tile_dimension_suffix(
                self.forecast_hour,
                self.observation_time,
                self.elevation,
                version,
            ),
            entry_ttl: unresolved.then(unresolved_ttl),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MemoKey {
    model: String,
    parameter: String,
    forecast_hour: Option<u32>,
    level: Option<String>,
}

/// One memoised lookup.
struct Slot {
    /// The last answer the catalog gave. Kept when a refresh fails, so a
    /// database blip does not take the key back to the unversioned fallback.
    /// `None` is also a real answer ("no dataset"), once `fresh_until` is set.
    answer: Option<LatestVersion>,
    /// When `answer` stops being fresh; `None` before the first attempt.
    fresh_until: Option<Instant>,
    /// Held by the one task refreshing this slot.
    refresh: Arc<AsyncMutex<()>>,
}

impl Slot {
    fn new() -> Self {
        Self {
            answer: None,
            fresh_until: None,
            refresh: Arc::new(AsyncMutex::new(())),
        }
    }

    fn is_fresh(&self, now: Instant) -> bool {
        self.fresh_until.is_some_and(|until| now < until)
    }
}

/// A look at a slot: what it holds and whether that is still fresh.
struct Peek {
    answer: Option<LatestVersion>,
    fresh: bool,
    refresh: Arc<AsyncMutex<()>>,
}

/// Short-lived memo of "which dataset is latest" per layer, so the catalog is
/// not queried on every tile request. See the module docs for the single-flight
/// and stale-answer behaviour.
pub struct LatestVersionMemo {
    ttl: Duration,
    lookup_timeout: Duration,
    failure_retry: Duration,
    slots: Mutex<HashMap<MemoKey, Slot>>,
}

impl LatestVersionMemo {
    pub fn new(ttl: Duration) -> Self {
        Self::with_timing(ttl, LOOKUP_TIMEOUT, FAILURE_RETRY)
    }

    /// Like [`Self::new`] with the lookup timeout and failure back-off spelled
    /// out, so tests do not have to wait for the real ones.
    pub fn with_timing(ttl: Duration, lookup_timeout: Duration, failure_retry: Duration) -> Self {
        Self {
            ttl,
            lookup_timeout,
            failure_retry,
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn lock_slots(&self) -> std::sync::MutexGuard<'_, HashMap<MemoKey, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn peek(&self, key: &MemoKey, now: Instant) -> Peek {
        let mut slots = self.lock_slots();
        if let Some(slot) = slots.get(key) {
            return Peek {
                answer: slot.answer.clone(),
                fresh: slot.is_fresh(now),
                refresh: slot.refresh.clone(),
            };
        }
        if slots.len() >= VERSION_MEMO_MAX_ENTRIES {
            // Drop stale answers first; if the map is still full the keys are
            // not repeating (a sweep), so remembering them buys nothing.
            slots.retain(|_, slot| slot.is_fresh(now));
            if slots.len() >= VERSION_MEMO_MAX_ENTRIES {
                slots.clear();
            }
        }
        let slot = slots.entry(key.clone()).or_insert_with(Slot::new);
        Peek {
            answer: None,
            fresh: false,
            refresh: slot.refresh.clone(),
        }
    }

    /// Record the catalog's answer (including "no dataset").
    fn record_answer(&self, key: &MemoKey, answer: Option<LatestVersion>, now: Instant) {
        let mut slots = self.lock_slots();
        let slot = slots.entry(key.clone()).or_insert_with(Slot::new);
        slot.answer = answer;
        slot.fresh_until = Some(now + self.ttl);
    }

    /// A refresh failed or timed out: keep the last answer, retry shortly.
    fn record_failure(&self, key: &MemoKey, now: Instant) {
        let mut slots = self.lock_slots();
        let slot = slots.entry(key.clone()).or_insert_with(Slot::new);
        slot.fresh_until = Some(now + self.failure_retry);
    }

    /// The dataset `lookup` resolves `key` to, from the memo while fresh.
    ///
    /// `lookup` runs for at most one caller at a time per key and is cut off
    /// after the lookup timeout. Every other caller gets the last good answer
    /// (or waits for the in-flight refresh when there is none yet).
    async fn resolve<F, Fut>(&self, key: &MemoKey, lookup: F) -> Option<LatestVersion>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = WmsResult<Option<CatalogEntry>>>,
    {
        let seen = self.peek(key, Instant::now());
        if seen.fresh {
            return seen.answer;
        }

        let _refreshing = match seen.refresh.clone().try_lock_owned() {
            Ok(guard) => guard,
            // Someone else is already asking the catalog.
            Err(_) if seen.answer.is_some() => return seen.answer,
            Err(_) => {
                // Nothing to serve yet: wait for that refresh rather than
                // starting a second identical query.
                let _wait = seen.refresh.lock().await;
                return self.peek(key, Instant::now()).answer;
            }
        };

        // The refresh we raced with may have finished between peek and lock.
        let seen = self.peek(key, Instant::now());
        if seen.fresh {
            return seen.answer;
        }

        match tokio::time::timeout(self.lookup_timeout, lookup()).await {
            Ok(Ok(entry)) => {
                let answer = entry.as_ref().map(LatestVersion::from_entry);
                self.record_answer(key, answer.clone(), Instant::now());
                answer
            }
            Ok(Err(e)) => {
                // Not fatal: the tile is still served, from the last known
                // version if there is one.
                debug!(error = %e, model = %key.model, parameter = %key.parameter, "latest-version lookup failed");
                self.record_failure(key, Instant::now());
                seen.answer
            }
            Err(_) => {
                debug!(model = %key.model, parameter = %key.parameter, timeout_ms = self.lookup_timeout.as_millis() as u64, "latest-version lookup timed out");
                self.record_failure(key, Instant::now());
                seen.answer
            }
        }
    }
}

/// Resolve the dataset a latest tile request will be rendered from into a
/// version, or `None` when it cannot be resolved (see module docs).
///
/// `forecast_hour` and `level` are the values GetTile passes to the renderer,
/// and the lookup mirrors the renderer's no-observation-time branch
/// (`rendering::render_weather_data`, `rendering::wind::get_wind_entry`) so the
/// token names the dataset that will actually be drawn. Keep them in step.
pub async fn resolve_latest_version(
    catalog: &Catalog,
    memo: &LatestVersionMemo,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> Option<LatestVersion> {
    resolve_with(memo, model, parameter, forecast_hour, level, |parameter| {
        lookup_default_entry(catalog, model, parameter, forecast_hour, level)
    })
    .await
}

/// [`resolve_latest_version`] with the catalog query passed in, so the choice of
/// WHICH datasets to ask about (a composite layer asks about its components,
/// never about its own name) can be tested without a database.
async fn resolve_with<'a, F, Fut>(
    memo: &LatestVersionMemo,
    model: &str,
    parameter: &'a str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
    lookup: F,
) -> Option<LatestVersion>
where
    F: Fn(&'a str) -> Fut,
    Fut: Future<Output = WmsResult<Option<CatalogEntry>>>,
{
    let one = |parameter: &'a str| {
        let key = MemoKey {
            model: model.to_string(),
            parameter: parameter.to_string(),
            forecast_hour,
            level: level.map(str::to_string),
        };
        let lookup = &lookup;
        async move { memo.resolve(&key, || lookup(parameter)).await }
    };

    match drawn_parameters(model, parameter) {
        (only, None) => one(only).await,
        (first, Some(second)) => {
            let (first, second) = tokio::join!(one(first), one(second));
            LatestVersion::combine(first, second)
        }
    }
}

/// The catalog's default dataset for a request with no observation time.
async fn lookup_default_entry(
    catalog: &Catalog,
    model: &str,
    parameter: &str,
    forecast_hour: Option<u32>,
    level: Option<&str>,
) -> WmsResult<Option<CatalogEntry>> {
    match (forecast_hour, level) {
        (Some(hour), Some(lev)) => {
            catalog
                .find_by_forecast_hour_and_level(model, parameter, hour, lev)
                .await
        }
        (Some(hour), None) => catalog.find_by_forecast_hour(model, parameter, hour).await,
        (None, Some(lev)) => {
            catalog
                .get_latest_run_earliest_forecast_at_level(model, parameter, lev)
                .await
        }
        (None, None) => {
            catalog
                .get_latest_run_earliest_forecast(model, parameter)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use storage::CacheKey;
    use wms_common::{BoundingBox, CrsCode, WmsError};

    fn entry(reference: DateTime<Utc>, hour: u32, minute: u32) -> CatalogEntry {
        CatalogEntry {
            model: "mrms".to_string(),
            parameter: "REFL".to_string(),
            level: "0".to_string(),
            reference_time: reference,
            forecast_hour: hour,
            forecast_minute: minute,
            bbox: BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
            storage_path: "mrms/REFL/x.zarr".to_string(),
            file_size: 1,
            zarr_metadata: None,
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn version(secs: i64, hour: u32) -> LatestVersion {
        LatestVersion::from_entry(&entry(at(secs), hour, 0))
    }

    fn dims<'a>(is_observation: bool) -> TileRequestDims<'a> {
        TileRequestDims {
            is_observation,
            observation_time: None,
            run: None,
            forecast_hour: None,
            elevation: None,
        }
    }

    // ---- the cache plan: one case per kind of request ----

    #[test]
    fn explicit_observation_time_is_explicit_with_its_original_key_and_ttl() {
        let d = TileRequestDims {
            observation_time: Some(at(1_700_000_000)),
            ..dims(true)
        };
        assert!(!d.needs_latest_lookup());
        // No lookup was made, so whatever `latest` holds must not leak in.
        for latest in [None, Some(&version(1_700_000_000, 0))] {
            assert_eq!(
                d.cache_plan(latest),
                TileCachePlan {
                    selection: TimeSelection::Explicit,
                    dimension_suffix: Some("obs1700000000".to_string()),
                    entry_ttl: None,
                }
            );
        }
    }

    #[test]
    fn latest_observation_with_a_version_is_latest_keyed_by_the_scan() {
        let d = dims(true);
        assert!(d.needs_latest_lookup());
        let v = version(1_700_000_000, 0);
        assert_eq!(
            d.cache_plan(Some(&v)),
            TileCachePlan {
                selection: TimeSelection::Latest,
                dimension_suffix: Some(format!("@{}", v.token)),
                // The version in the key is what keeps it from going stale,
                // so the cache's normal lifetime is fine.
                entry_ttl: None,
            }
        );
    }

    #[test]
    fn latest_observation_without_a_version_gets_the_unresolved_key_and_a_short_ttl() {
        let plan = dims(true).cache_plan(None);
        assert_eq!(plan.selection, TimeSelection::Latest);
        assert_eq!(
            plan.dimension_suffix.as_deref(),
            Some("@unresolved"),
            "must not be the legacy `current` key"
        );
        let ttl = plan.entry_ttl.expect("an unversioned entry needs a TTL");
        assert_eq!(ttl, Duration::from_secs(LATEST_MAX_AGE_SECS as u64));
        // Never longer than what the response tells the CDN.
        assert!(ttl.as_secs() as u32 <= plan.selection.max_age_secs(3600));
    }

    #[test]
    fn model_tile_keeps_its_hour_and_level_and_is_keyed_by_the_run() {
        let d = TileRequestDims {
            forecast_hour: Some(6),
            elevation: Some("2 m above ground"),
            ..dims(false)
        };
        let v = version(1_700_000_000, 6);
        let plan = d.cache_plan(Some(&v));
        assert_eq!(plan.selection, TimeSelection::Latest);
        assert_eq!(
            plan.dimension_suffix,
            Some(format!("t6_2_m_above_ground@{}", v.token))
        );
        assert_eq!(plan.entry_ttl, None);
    }

    #[test]
    fn model_tile_without_a_version_is_unresolved_and_short_lived() {
        let d = TileRequestDims {
            forecast_hour: Some(6),
            ..dims(false)
        };
        let plan = d.cache_plan(None);
        assert_eq!(plan.dimension_suffix.as_deref(), Some("t6@unresolved"));
        assert_eq!(plan.entry_ttl, Some(unresolved_ttl()));
        assert_eq!(plan.selection, TimeSelection::Latest);
    }

    #[test]
    fn model_tile_with_a_run_is_explicit_only_when_that_run_is_what_resolved() {
        let run = at(1_700_000_000);
        let d = TileRequestDims {
            run: Some(run),
            forecast_hour: Some(6),
            ..dims(false)
        };

        // The newest run is the requested one: the URL names the tile.
        let same = version(1_700_000_000, 6);
        let plan = d.cache_plan(Some(&same));
        assert_eq!(plan.selection, TimeSelection::Explicit);
        // ...and it still shares the entry with the plain latest request.
        assert_eq!(plan.dimension_suffix, Some(format!("t6@{}", same.token)));

        // A newer run has landed: GetTile renders that one, so it is latest.
        let newer = version(1_700_021_600, 6);
        assert_eq!(d.cache_plan(Some(&newer)).selection, TimeSelection::Latest);
        // Nothing resolved: cannot claim the run.
        assert_eq!(d.cache_plan(None).selection, TimeSelection::Latest);
    }

    #[test]
    fn a_pinned_run_never_shares_a_key_with_a_different_run() {
        // Regression for keying an explicit RUN without the version: `t6` would
        // then hold whichever run was newest when it was first written.
        let d = TileRequestDims {
            run: Some(at(1_700_000_000)),
            forecast_hour: Some(6),
            ..dims(false)
        };
        let old = d.cache_plan(Some(&version(1_700_000_000, 6)));
        let new_run = TileRequestDims {
            run: Some(at(1_700_021_600)),
            ..d
        };
        let new = new_run.cache_plan(Some(&version(1_700_021_600, 6)));
        assert_ne!(old.dimension_suffix, new.dimension_suffix);
    }

    #[test]
    fn composite_with_disagreeing_runs_never_satisfies_a_run_pin() {
        let u = version(1_700_000_000, 6);
        let v = version(1_700_021_600, 6);
        let combined = LatestVersion::combine(Some(u), Some(v)).unwrap();
        assert_eq!(combined.run, None);
        let d = TileRequestDims {
            run: Some(at(1_700_000_000)),
            forecast_hour: Some(6),
            ..dims(false)
        };
        assert_eq!(
            d.cache_plan(Some(&combined)).selection,
            TimeSelection::Latest
        );
    }

    // ---- wind barbs ----

    #[test]
    fn wind_barbs_are_versioned_from_the_components_the_renderer_reads() {
        assert_eq!(wind_barb_components("hrrr"), ["UGRD", "VGRD"]);
        assert_eq!(wind_barb_components("gfs"), ["UGRD", "VGRD"]);
        assert_eq!(wind_barb_components("gfswave"), ["UGRD", "VGRD"]);
        assert_eq!(wind_barb_components("aigfs"), ["UGRD", "VGRD"]);
        assert_eq!(wind_barb_components("nbm_conus"), ["WIND", "WDIR"]);
        assert_eq!(wind_barb_components("nbm"), ["WIND", "WDIR"]);
        assert_eq!(wind_barb_components("ndfd"), ["WSPD", "WDIR"]);

        assert_eq!(
            drawn_parameters("hrrr", "WIND_BARBS"),
            ("UGRD", Some("VGRD"))
        );
        assert_eq!(drawn_parameters("hrrr", "TMP"), ("TMP", None));
    }

    #[test]
    fn wind_barb_version_changes_when_either_component_changes() {
        let u = version(1_700_000_000, 6);
        let v = version(1_700_000_000, 6);
        let same = LatestVersion::combine(Some(u.clone()), Some(v)).unwrap();
        assert_eq!(same, u, "agreeing components keep the plain token");

        let v_newer = version(1_700_021_600, 6);
        let lagging = LatestVersion::combine(Some(u.clone()), Some(v_newer.clone())).unwrap();
        assert_ne!(lagging.token, u.token);
        assert!(lagging.token.contains(&u.token) && lagging.token.contains(&v_newer.token));
        assert!(!lagging.token.contains(':') && !lagging.token.contains('*'));

        // A component with no dataset means no version at all.
        assert_eq!(LatestVersion::combine(Some(u), None), None);
    }

    // ---- cache key shape ----

    #[test]
    fn explicit_keys_keep_their_original_shape() {
        assert_eq!(tile_dimension_suffix(None, None, None, None), None);
        assert_eq!(
            tile_dimension_suffix(Some(6), None, None, None).as_deref(),
            Some("t6")
        );
        assert_eq!(
            tile_dimension_suffix(None, Some(at(1_700_000_000)), None, None).as_deref(),
            Some("obs1700000000")
        );
        assert_eq!(
            tile_dimension_suffix(Some(6), None, Some("2 m above ground"), None).as_deref(),
            Some("t6_2_m_above_ground")
        );
        assert_eq!(
            tile_dimension_suffix(None, None, Some("500 mb"), None).as_deref(),
            Some("500_mb")
        );
    }

    #[test]
    fn version_is_appended_after_the_other_dimensions() {
        assert_eq!(
            tile_dimension_suffix(None, None, None, Some("r1f0m0")).as_deref(),
            Some("@r1f0m0")
        );
        assert_eq!(
            tile_dimension_suffix(Some(3), None, Some("500 mb"), Some("r1f3m0")).as_deref(),
            Some("t3_500_mb@r1f3m0")
        );
    }

    #[test]
    fn version_token_changes_with_run_hour_and_minute() {
        let base = version_token(&entry(at(1_700_000_000), 0, 0));
        assert_ne!(base, version_token(&entry(at(1_700_000_120), 0, 0)));
        assert_ne!(base, version_token(&entry(at(1_700_000_000), 1, 0)));
        assert_ne!(base, version_token(&entry(at(1_700_000_000), 0, 15)));
        assert!(!base.contains(':') && !base.contains('*'));
    }

    /// The bug being fixed: a no-time request used to be keyed "current"
    /// whatever data it resolved to. Both the L1 and the L2 key must now
    /// differ once the latest dataset changes.
    #[test]
    fn cache_keys_differ_once_the_latest_time_changes() {
        let suffix = |secs| {
            dims(true)
                .cache_plan(Some(&version(secs, 0)))
                .dimension_suffix
        };
        let suffix_old = suffix(1_700_000_000);
        let suffix_new = suffix(1_700_000_120);
        assert_ne!(suffix_old, suffix_new);

        let l1 = |s: &Option<String>| {
            l1_cache_key(
                "WebMercatorQuad",
                "mrms_REFL",
                "default",
                5,
                8,
                12,
                s.as_deref(),
            )
        };
        assert_ne!(l1(&suffix_old), l1(&suffix_new));

        let l2 = |s: &Option<String>| {
            CacheKey::new(
                "mrms_REFL",
                "default",
                CrsCode::Epsg3857,
                BoundingBox::new(8.0, 12.0, 5.0, 0.0),
                256,
                256,
                s.clone(),
                "png",
            )
            .to_string()
        };
        assert_ne!(l2(&suffix_old), l2(&suffix_new));
        // ...and neither is the legacy unversioned key any more.
        let legacy = l2(&None);
        assert!(legacy.contains(":current:"));
        assert_ne!(l2(&suffix_new), legacy);
        assert_ne!(l2(&dims(true).cache_plan(None).dimension_suffix), legacy);
    }

    #[test]
    fn same_data_keeps_the_same_key() {
        let plan = |secs| dims(true).cache_plan(Some(&version(secs, 0)));
        assert_eq!(plan(1_700_000_000), plan(1_700_000_000));
    }

    // ---- memo ----

    fn memo_key(model: &str, hour: Option<u32>) -> MemoKey {
        MemoKey {
            model: model.to_string(),
            parameter: "REFL".to_string(),
            forecast_hour: hour,
            level: None,
        }
    }

    fn fast_memo(ttl: Duration) -> LatestVersionMemo {
        LatestVersionMemo::with_timing(ttl, Duration::from_millis(50), Duration::from_secs(60))
    }

    /// A lookup that counts its calls and answers with `reference` seconds.
    fn counting_lookup(
        calls: &Arc<AtomicUsize>,
        reference: i64,
    ) -> impl FnOnce() -> std::future::Ready<WmsResult<Option<CatalogEntry>>> {
        let calls = calls.clone();
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(Some(entry(at(reference), 0, 0))))
        }
    }

    #[tokio::test]
    async fn memo_serves_a_fresh_answer_without_asking_the_catalog_again() {
        let memo = fast_memo(Duration::from_secs(60));
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);

        let first = memo.resolve(&key, counting_lookup(&calls, 100)).await;
        let second = memo.resolve(&key, counting_lookup(&calls, 200)).await;

        assert_eq!(first, second);
        assert_eq!(first.unwrap().token, version(100, 0).token);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn memo_notices_new_data_once_the_ttl_has_passed() {
        let memo = fast_memo(Duration::ZERO);
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);

        let old = memo.resolve(&key, counting_lookup(&calls, 100)).await;
        let new = memo.resolve(&key, counting_lookup(&calls, 220)).await;

        assert_ne!(old, new, "a new scan must yield a new version");
        assert_eq!(new.unwrap().token, version(220, 0).token);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn memo_remembers_an_unresolvable_layer_too() {
        let memo = fast_memo(Duration::from_secs(60));
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);
        let none_lookup = |calls: &Arc<AtomicUsize>| {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(None))
            }
        };

        assert_eq!(memo.resolve(&key, none_lookup(&calls)).await, None);
        assert_eq!(memo.resolve(&key, none_lookup(&calls)).await, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn memo_keys_are_independent() {
        let memo = fast_memo(Duration::from_secs(60));
        let calls = Arc::new(AtomicUsize::new(0));
        memo.resolve(&memo_key("mrms", None), counting_lookup(&calls, 100))
            .await;
        memo.resolve(&memo_key("mrms", Some(3)), counting_lookup(&calls, 100))
            .await;
        memo.resolve(&memo_key("goes19", None), counting_lookup(&calls, 100))
            .await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    /// The finding being fixed: 20-40 tiles arriving just after expiry each ran
    /// their own catalog query.
    #[tokio::test]
    async fn concurrent_requests_on_a_cold_key_share_one_query() {
        let memo = Arc::new(fast_memo(Duration::from_secs(60)));
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);

        let slow_lookup = |calls: &Arc<AtomicUsize>| {
            let calls = calls.clone();
            move || async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(Some(entry(at(100), 0, 0)))
            }
        };

        let mut tasks = Vec::new();
        for _ in 0..30 {
            let (memo, key, lookup) = (memo.clone(), key.clone(), slow_lookup(&calls));
            tasks.push(tokio::spawn(
                async move { memo.resolve(&key, lookup).await },
            ));
        }
        for task in tasks {
            let answer = task.await.unwrap();
            assert_eq!(answer.unwrap().token, version(100, 0).token);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one query for 30 requests");
    }

    #[tokio::test]
    async fn while_one_task_refreshes_the_others_get_the_last_good_answer() {
        let memo = fast_memo(Duration::ZERO);
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);
        let old = memo
            .resolve(&key, counting_lookup(&calls, 100))
            .await
            .unwrap();

        let slow_refresh = {
            let calls = calls.clone();
            move || async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(Some(entry(at(220), 0, 0)))
            }
        };
        let second_caller = {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(Some(entry(at(999), 0, 0))))
            }
        };

        // `join!` polls in order: the first caller takes the refresh and
        // suspends in its slow query, the second then finds it taken.
        let (refreshed, served_stale) = tokio::join!(
            memo.resolve(&key, slow_refresh),
            memo.resolve(&key, second_caller)
        );

        assert_eq!(refreshed.unwrap().token, version(220, 0).token);
        assert_eq!(
            served_stale.unwrap(),
            old,
            "must not queue behind the refresh, nor start a second query"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2, "initial + one refresh");
    }

    /// The finding being fixed: a lookup error cached `None` and moved the
    /// request onto the unversioned key, making cached latest tiles
    /// unreachable for the whole outage.
    #[tokio::test]
    async fn a_failed_refresh_keeps_the_last_good_version() {
        let memo = fast_memo(Duration::ZERO);
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);
        let good = memo
            .resolve(&key, counting_lookup(&calls, 100))
            .await
            .unwrap();

        let failing = {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err(WmsError::DatabaseError("db down".to_string())))
            }
        };
        let answer = memo.resolve(&key, failing).await;
        assert_eq!(answer, Some(good.clone()));

        // ...and within the back-off the catalog is left alone.
        let again = memo.resolve(&key, counting_lookup(&calls, 300)).await;
        assert_eq!(again, Some(good));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "initial + the failed one");
    }

    #[tokio::test]
    async fn a_slow_refresh_is_cut_off_and_keeps_the_last_good_version() {
        let memo = fast_memo(Duration::ZERO);
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);
        let good = memo
            .resolve(&key, counting_lookup(&calls, 100))
            .await
            .unwrap();

        let hanging = || async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(Some(entry(at(220), 0, 0)))
        };
        let started = Instant::now();
        let answer = memo.resolve(&key, hanging).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must not wait for a hanging catalog"
        );
        assert_eq!(answer, Some(good));
    }

    #[tokio::test]
    async fn a_never_resolved_key_falls_back_to_none_on_failure() {
        let memo = fast_memo(Duration::from_secs(60));
        let key = memo_key("mrms", None);
        let failing = || std::future::ready(Err(WmsError::DatabaseError("db down".to_string())));
        assert_eq!(memo.resolve(&key, failing).await, None);
    }

    #[tokio::test]
    async fn an_authoritative_no_dataset_replaces_an_earlier_version() {
        // Retention removed the data: the answer is "none", not "keep the old".
        let memo = fast_memo(Duration::ZERO);
        let calls = Arc::new(AtomicUsize::new(0));
        let key = memo_key("mrms", None);
        memo.resolve(&key, counting_lookup(&calls, 100)).await;
        let gone = memo.resolve(&key, || std::future::ready(Ok(None))).await;
        assert_eq!(gone, None);
    }

    /// The finding being fixed: WIND_BARBS was looked up under its own name,
    /// which has no dataset, so every wind barb tile fell back to the
    /// short-lived unversioned key and was re-rendered every minute.
    #[tokio::test]
    async fn wind_barbs_are_resolved_from_their_components_never_by_their_own_name() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let memo = fast_memo(Duration::from_secs(60));

        let lookup = |asked: &Arc<Mutex<Vec<String>>>| {
            let asked = asked.clone();
            move |parameter: &str| {
                asked.lock().unwrap().push(parameter.to_string());
                // Only the components exist in the catalog.
                let found = parameter != WIND_BARBS_PARAMETER;
                std::future::ready(Ok(found.then(|| entry(at(1_700_000_000), 6, 0))))
            }
        };

        let version = resolve_with(&memo, "hrrr", "WIND_BARBS", Some(6), None, lookup(&asked))
            .await
            .expect("wind barbs must get a version from U/V");
        let mut queried = asked.lock().unwrap().clone();
        queried.sort();
        assert_eq!(queried, ["UGRD", "VGRD"]);
        assert_eq!(
            version.token,
            version_token(&entry(at(1_700_000_000), 6, 0))
        );

        // NBM and NDFD are drawn from speed/direction.
        asked.lock().unwrap().clear();
        let memo = fast_memo(Duration::from_secs(60));
        resolve_with(&memo, "nbm_conus", "WIND_BARBS", None, None, lookup(&asked)).await;
        resolve_with(&memo, "ndfd", "WIND_BARBS", None, None, lookup(&asked)).await;
        let mut queried = asked.lock().unwrap().clone();
        queried.sort();
        assert_eq!(queried, ["WDIR", "WDIR", "WIND", "WSPD"]);
    }

    #[tokio::test]
    async fn wind_barb_version_follows_whichever_component_moves() {
        let memo = fast_memo(Duration::ZERO);
        // U is already on the new run, V still on the old one.
        let lookup = |parameter: &str| {
            let run = if parameter == "UGRD" {
                1_700_021_600
            } else {
                1_700_000_000
            };
            std::future::ready(Ok(Some(entry(at(run), 6, 0))))
        };
        let lagging = resolve_with(&memo, "hrrr", "WIND_BARBS", Some(6), None, lookup)
            .await
            .unwrap();

        // V catches up: the key must change even though U did not.
        let caught_up = |_: &str| std::future::ready(Ok(Some(entry(at(1_700_021_600), 6, 0))));
        let current = resolve_with(&memo, "hrrr", "WIND_BARBS", Some(6), None, caught_up)
            .await
            .unwrap();
        assert_ne!(lagging.token, current.token);
        assert_eq!(current.run, Some(at(1_700_021_600)));
        assert_eq!(
            lagging.run, None,
            "components on different runs pin nothing"
        );
    }

    #[tokio::test]
    async fn an_ordinary_layer_is_resolved_under_its_own_name_once() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let memo = fast_memo(Duration::from_secs(60));
        let lookup = {
            let asked = asked.clone();
            move |parameter: &str| {
                asked.lock().unwrap().push(parameter.to_string());
                std::future::ready(Ok(Some(entry(at(1_700_000_000), 0, 0))))
            }
        };
        resolve_with(&memo, "mrms", "REFL", None, Some("0"), &lookup).await;
        resolve_with(&memo, "mrms", "REFL", None, Some("0"), &lookup).await;
        assert_eq!(*asked.lock().unwrap(), ["REFL"]);
    }

    #[test]
    fn memo_is_bounded() {
        let memo = fast_memo(Duration::from_secs(10));
        let now = Instant::now();
        for i in 0..(VERSION_MEMO_MAX_ENTRIES as u32 + 10) {
            memo.peek(&memo_key("m", Some(i)), now);
        }
        let len = memo.slots.lock().unwrap().len();
        assert!(len <= VERSION_MEMO_MAX_ENTRIES, "memo grew to {len}");
    }
}
