//! `Cache-Control` for EDR data queries: short for "latest", unchanged for an
//! explicit time.
//!
//! WHY: Cloudflare caches any response with a `max-age`. An EDR query that names
//! no time (`/collections/mrms-single-level/area`, any `*-latest` collection)
//! is answered from whatever the newest scan or run is, so its body changes
//! every couple of minutes under an unchanged URL. The old blanket
//! `max-age=300` kept showing the previous scan for up to five minutes after a
//! new one arrived. A query that pins its time (`datetime`, an instance, or a
//! `run` on an endpoint that honours it) cannot change and keeps its TTL.
//!
//! The decision is made here, from the request's time selectors, so every
//! handler asks the same question instead of repeating a literal. The shared
//! rule and TTL live in [`wms_common::cache_control`].
//!
//! # `run` only counts where it is honoured
//!
//! `position` selects the model run a client names. The other data endpoints
//! (area, radius, cube, corridor, trajectory, locations) parse `run` for
//! validation and then query the newest run regardless. A URL that names a run
//! but returns the newest one is *latest*: a long `max-age` would let the CDN
//! keep whichever run was newest when it was first fetched, under a URL that
//! names a different run. Those params structs implement [`TimeSelectors`]
//! with `ignores_run`, so their `run` never pins the time. If an endpoint
//! starts honouring `run`, drop the flag there.
//!
//! Only data queries use this. Metadata (collections, conformance, instance
//! lists) and non-temporal data (locations lists, trails, populated places)
//! keep their own TTLs: they are not "about latest".

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

pub use wms_common::cache_control::{TimeSelection, LATEST_MAX_AGE_SECS};

/// The parts of a request that can pin its time.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestedTime<'a> {
    /// The `datetime` query parameter.
    pub datetime: Option<&'a str>,
    /// The `run` query parameter, for endpoints that honour it. Endpoints that
    /// ignore it pass `None` (see [`TimeSelectors::run`]). `forecast-hour` is
    /// not listed because it is only valid together with `run`.
    pub run: Option<&'a str>,
    /// The `/instances/{id}` path segment.
    pub instance: Option<&'a str>,
    /// Times carried in the coordinates themselves (`LINESTRING M` values on
    /// corridor and trajectory queries), which are mutually exclusive with
    /// `datetime`.
    pub embedded_times: bool,
}

impl<'a> RequestedTime<'a> {
    /// The selectors of a query: its `datetime` and `run` parameters and the
    /// instance in its path, if any.
    pub fn new(datetime: Option<&'a str>, run: Option<&'a str>, instance: Option<&'a str>) -> Self {
        Self {
            datetime,
            run,
            instance,
            embedded_times: false,
        }
    }

    /// Mark that the coordinates carry their own times (`LINESTRING M`).
    pub fn with_embedded_times(mut self, embedded: bool) -> Self {
        self.embedded_times = embedded;
        self
    }
}

impl RequestedTime<'_> {
    /// Whether the response is for a time the client named or one the server
    /// picked.
    pub fn selection(&self) -> TimeSelection {
        self.selection_at(Utc::now())
    }

    /// [`Self::selection`] with the clock spelled out: a closed `datetime`
    /// interval ending after `now` is not pinned yet (see
    /// [`datetime_is_pinned`]).
    pub fn selection_at(&self, now: DateTime<Utc>) -> TimeSelection {
        TimeSelection::from_explicit(
            given(self.run)
                || given(self.instance)
                || self.embedded_times
                || self.datetime.is_some_and(|dt| datetime_is_pinned(dt, now)),
        )
    }

    /// `Cache-Control` value; `explicit_secs` is the TTL this endpoint used
    /// before the latest-time rule (it still applies to explicit queries).
    pub fn cache_control(&self, explicit_secs: u32) -> String {
        self.selection().cache_control(explicit_secs)
    }
}

/// Query-parameter structs of the data endpoints, so a handler asks
/// `params.requested_time(instance)` instead of repeating which fields pin the
/// time. Implement it with [`impl_time_selectors!`].
pub trait TimeSelectors {
    fn datetime(&self) -> Option<&str>;

    /// The `run` that pins this query's time. `None` for an endpoint that
    /// parses `run` but queries the newest run anyway, because such a query is
    /// latest whatever the URL says (see the module docs).
    fn run(&self) -> Option<&str>;

    /// The selectors of this query. `instance` is the `/instances/{id}` path
    /// segment, if the request came in through one.
    fn requested_time<'a>(&'a self, instance: Option<&'a str>) -> RequestedTime<'a> {
        RequestedTime::new(self.datetime(), self.run(), instance)
    }
}

/// Implement [`TimeSelectors`] for a params struct with `datetime` and `run`
/// fields of type `Option<String>`.
///
/// `impl_time_selectors!(P)` for an endpoint that selects the run a client
/// names; `impl_time_selectors!(P, ignores_run)` for one that parses `run` but
/// always queries the newest run, so its `run` must not pin the time.
macro_rules! impl_time_selectors {
    ($params:ty) => {
        impl $crate::cache_policy::TimeSelectors for $params {
            fn datetime(&self) -> Option<&str> {
                self.datetime.as_deref()
            }
            fn run(&self) -> Option<&str> {
                self.run.as_deref()
            }
        }
    };
    ($params:ty, ignores_run) => {
        impl $crate::cache_policy::TimeSelectors for $params {
            fn datetime(&self) -> Option<&str> {
                self.datetime.as_deref()
            }
            fn run(&self) -> Option<&str> {
                None
            }
        }
    };
}
pub(crate) use impl_time_selectors;

/// Lifetime cap for the server's own cached copy of a response, so it cannot
/// outlive the `max-age` sent with it.
///
/// `Some(LATEST_MAX_AGE_SECS)` for a latest response: a cache HIT would
/// otherwise hand out a body older than the header admits (our cache's default
/// is minutes), and the CDN would keep it for the header's lifetime on top.
/// `None` for an explicit one: the cache's own TTL applies.
pub fn server_cache_ttl(selection: TimeSelection) -> Option<Duration> {
    selection
        .is_latest()
        .then(|| Duration::from_secs(LATEST_MAX_AGE_SECS as u64))
}

/// A parameter counts as given only when it has content: the handlers treat an
/// empty or whitespace-only `run` as absent.
fn given(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

/// Whether a `datetime` value fixes the response.
///
/// An instant, a list and an interval that has already ended do. An empty value
/// does not. Neither does an interval that is still open at the end: it is
/// expanded against the times available *now*, so it grows as new data
/// arrives, which is exactly what "latest" means for caching. That holds both
/// for an open end (`2026-10-07T20:00:00Z/..`) and for a closed end that lies
/// after `now` (`<2h ago>/<in 1h>`, or `../<in 1h>`): scans that land inside
/// the window change the answer just the same. An open start with an end that
/// has passed (`../2026-10-07T20:00:00Z`) is bounded above and stable.
fn datetime_is_pinned(datetime: &str, now: DateTime<Utc>) -> bool {
    let datetime = datetime.trim();
    if datetime.is_empty() {
        return false;
    }
    match datetime.split_once('/') {
        Some((_, end)) => {
            let end = end.trim();
            if end.is_empty() || end == ".." {
                return false;
            }
            // An end we cannot read is the handler's 400 to give; it is not
            // "ahead of now", so it counts as closed.
            interval_end(end).is_none_or(|end| end <= now)
        }
        None => true,
    }
}

/// The instant a closed interval's end names, in the formats the `datetime`
/// parser accepts (`edr_protocol::DateTimeQuery`): RFC 3339, a zone-less
/// date-time (UTC) and a bare date. A bare date covers its whole day, so it
/// ends when the next day starts.
fn interval_end(end: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(end) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(end, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc());
    }
    NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.succ_opt())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(rt: RequestedTime<'_>) -> TimeSelection {
        rt.selection()
    }

    #[test]
    fn no_time_at_all_is_latest() {
        assert_eq!(selection(RequestedTime::default()), TimeSelection::Latest);
        assert_eq!(RequestedTime::default().cache_control(300), "max-age=60");
    }

    /// A fixed "now" for the interval cases, so they do not depend on the
    /// clock of the machine running the tests.
    fn now() -> DateTime<Utc> {
        "2026-10-31T12:00:00Z".parse().unwrap()
    }

    fn datetime_selection(dt: &str) -> TimeSelection {
        RequestedTime {
            datetime: Some(dt),
            ..Default::default()
        }
        .selection_at(now())
    }

    #[test]
    fn explicit_datetime_keeps_the_existing_ttl() {
        for dt in [
            "2026-10-07T20:00:00Z",
            "2026-10-07T20:00:00Z,2026-10-07T21:00:00Z",
            "2026-10-07T20:00:00Z/2026-10-07T22:00:00Z",
            "../2026-10-07T22:00:00Z",
            // zone-less and date-only forms the datetime parser also accepts
            "2026-10-07T20:00:00/2026-10-07T22:00:00",
            "2026-10-01/2026-10-07",
        ] {
            assert_eq!(
                datetime_selection(dt),
                TimeSelection::Explicit,
                "datetime={dt}"
            );
        }
    }

    #[test]
    fn open_ended_or_empty_datetime_is_still_latest() {
        for dt in [
            "2026-10-07T20:00:00Z/..",
            "2026-10-07T20:00:00Z/",
            "../..",
            "",
            "  ",
        ] {
            assert_eq!(
                datetime_selection(dt),
                TimeSelection::Latest,
                "datetime={dt:?}"
            );
        }
        // ...with the short TTL on the wire.
        let rt = RequestedTime {
            datetime: Some("2026-10-07T20:00:00Z/.."),
            ..Default::default()
        };
        assert_eq!(rt.cache_control(300), "max-age=60");
    }

    /// The finding being fixed: `T/..` was latest but `<now-2h>/<now+1h>` was
    /// pinned, although both grow as scans arrive inside the window.
    #[test]
    fn closed_interval_ending_in_the_future_is_latest_like_an_open_one() {
        for dt in [
            "2026-10-31T10:00:00Z/2026-10-31T13:00:00Z",
            "../2026-10-31T13:00:00Z",
            "2026-10-31T10:00:00/2026-10-31T13:00:00", // zone-less, UTC
            "2026-10-30/2026-10-31",                   // the whole of today
            "2026-10-31T10:00:00-05:00/2026-10-31T09:00:00-05:00", // 14:00Z end
        ] {
            assert_eq!(
                datetime_selection(dt),
                TimeSelection::Latest,
                "datetime={dt}"
            );
        }
        let rt = RequestedTime {
            datetime: Some("2026-10-31T10:00:00Z/2026-10-31T13:00:00Z"),
            ..Default::default()
        };
        assert_eq!(rt.selection_at(now()).cache_control(300), "max-age=60");
    }

    #[test]
    fn interval_that_has_already_ended_stays_pinned() {
        for dt in [
            "2026-10-31T10:00:00Z/2026-10-31T11:59:59Z",
            "../2026-10-31T11:00:00Z",
            "2026-10-29/2026-10-30", // ended at the start of the 31st
        ] {
            assert_eq!(
                datetime_selection(dt),
                TimeSelection::Explicit,
                "datetime={dt}"
            );
        }
        // An unreadable end is the handler's 400, not "ahead of now".
        assert_eq!(
            datetime_selection("2026-10-07T20:00:00Z/not-a-time"),
            TimeSelection::Explicit
        );
    }

    #[test]
    fn instants_and_lists_in_the_future_are_not_treated_as_growing() {
        // Nothing exists there yet, but the answer for that exact time is what
        // it is; only a window can fill up with new scans.
        assert_eq!(
            datetime_selection("2027-01-01T00:00:00Z"),
            TimeSelection::Explicit
        );
        assert_eq!(
            datetime_selection("2027-01-01T00:00:00Z,2027-01-01T01:00:00Z"),
            TimeSelection::Explicit
        );
    }

    #[test]
    fn explicit_run_instance_and_embedded_times_are_explicit() {
        let run = RequestedTime {
            run: Some("2026-10-07T12:00:00Z"),
            ..Default::default()
        };
        assert_eq!(run.cache_control(300), "max-age=300");

        let instance = RequestedTime {
            instance: Some("2026-10-07T12:00:00Z"),
            ..Default::default()
        };
        assert_eq!(instance.cache_control(300), "max-age=300");

        let embedded = RequestedTime {
            embedded_times: true,
            ..Default::default()
        };
        assert_eq!(embedded.cache_control(300), "max-age=300");
    }

    #[test]
    fn blank_run_or_instance_does_not_pin_the_time() {
        let rt = RequestedTime {
            run: Some(" "),
            instance: Some(""),
            ..Default::default()
        };
        assert_eq!(rt.selection(), TimeSelection::Latest);
    }

    #[test]
    fn any_one_selector_is_enough_to_pin_the_time() {
        let rt = RequestedTime {
            datetime: Some("2026-10-07T20:00:00Z/.."),
            run: Some("2026-10-07T12:00:00Z"),
            ..Default::default()
        };
        // The open interval alone would be latest, the run pins it.
        assert_eq!(rt.selection(), TimeSelection::Explicit);
    }

    // ---- the real query-parameter structs of the data endpoints ----
    //
    // The handlers need an AppState (Postgres + MinIO), so they can only run in
    // the Docker integration tests. These parse real query strings into the
    // very structs the handlers take and ask for the same `requested_time` the
    // handlers use, so a renamed or unwired `datetime` / `run` field is caught.

    use crate::handlers::area::AreaQueryParams;
    use crate::handlers::corridor::CorridorQueryParams;
    use crate::handlers::cube::CubeQueryParams;
    use crate::handlers::locations::LocationQueryParams;
    use crate::handlers::position::PositionQueryParams;
    use crate::handlers::radius::RadiusQueryParams;
    use crate::handlers::trajectory::TrajectoryQueryParams;
    use axum::extract::Query;

    fn parse<T: serde::de::DeserializeOwned>(query: &str) -> T {
        let uri: axum::http::Uri = format!("/q?{query}").parse().unwrap();
        Query::<T>::try_from_uri(&uri).unwrap().0
    }

    /// Latest vs every way of pinning the time, for one params struct.
    /// `honours_run`: whether the endpoint selects the run a client names
    /// (only `position` does; the rest parse `run` and query the newest).
    fn assert_latest_vs_explicit<T: TimeSelectors + serde::de::DeserializeOwned>(
        base: &str,
        honours_run: bool,
    ) {
        let cc = |extra: &str, instance: Option<&str>| {
            parse::<T>(&format!("{base}{extra}"))
                .requested_time(instance)
                .cache_control(300)
        };
        let pinned = "max-age=300";
        let latest = "max-age=60";

        // No time named: the server picks the newest scan / run.
        assert_eq!(cc("", None), latest, "no time");
        // Named time: unchanged.
        assert_eq!(
            cc("&datetime=2020-10-07T20:00:00Z", None),
            pinned,
            "instant"
        );
        assert_eq!(
            cc("&datetime=2020-10-07T20:00:00Z/2020-10-07T22:00:00Z", None),
            pinned,
            "closed interval"
        );
        assert_eq!(cc("", Some("2020-10-07T12:00:00Z")), pinned, "instance");
        // A run only pins the time where the endpoint honours it. Elsewhere
        // the newest run is queried whatever the URL says, so it is latest.
        assert_eq!(
            cc("&run=2020-10-07T12:00:00Z&forecast-hour=0/6", None),
            if honours_run { pinned } else { latest },
            "run + forecast-hour (honours_run={honours_run})"
        );
        // An open end grows with new data.
        assert_eq!(
            cc("&datetime=2020-10-07T20:00:00Z/..", None),
            latest,
            "open-ended interval"
        );
        // ...and so does a window that closes in the future.
        assert_eq!(
            cc("&datetime=2020-10-07T20:00:00Z/2999-01-01T00:00:00Z", None),
            latest,
            "interval ending in the future"
        );
    }

    #[test]
    fn position_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<PositionQueryParams>("coords=POINT(-97.5%2035.5)", true);
    }

    #[test]
    fn area_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<AreaQueryParams>(
            "coords=POLYGON((-100%2030,-90%2030,-90%2040,-100%2040,-100%2030))&f=image/png",
            false,
        );
    }

    #[test]
    fn radius_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<RadiusQueryParams>(
            "coords=POINT(-97.5%2035.5)&within=50",
            false,
        );
    }

    #[test]
    fn cube_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<CubeQueryParams>("bbox=-100,30,-90,40", false);
    }

    #[test]
    fn corridor_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<CorridorQueryParams>(
            "coords=LINESTRING(-100%2030,-90%2040)&corridor-width=10",
            false,
        );
    }

    #[test]
    fn trajectory_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<TrajectoryQueryParams>(
            "coords=LINESTRING(-100%2030,-90%2040)",
            false,
        );
    }

    #[test]
    fn location_query_latest_vs_explicit() {
        assert_latest_vs_explicit::<LocationQueryParams>("parameter-name=TMP", false);
    }

    /// The finding being fixed: `/area?...&run=<older run>` renders the newest
    /// run, so it must not get the long PNG max-age that would let the CDN keep
    /// whichever run was newest for an hour under a URL naming another run.
    #[test]
    fn a_run_on_an_endpoint_that_ignores_it_does_not_pin_the_time() {
        let area = parse::<AreaQueryParams>(
            "coords=POLYGON((-100%2030,-90%2030,-90%2040,-100%2040,-100%2030))&f=image/png&run=2020-10-07T12:00:00Z",
        );
        assert_eq!(area.requested_time(None).cache_control(3600), "max-age=60");
        // Position selects the run, so there it does pin it.
        let position =
            parse::<PositionQueryParams>("coords=POINT(-97.5%2035.5)&run=2020-10-07T12:00:00Z");
        assert_eq!(
            position.requested_time(None).cache_control(3600),
            "max-age=3600"
        );
    }

    #[test]
    fn linestring_with_m_values_is_explicit_even_without_datetime() {
        let p = parse::<TrajectoryQueryParams>("coords=LINESTRINGM(-100%2030%201,-90%2040%202)");
        // `datetime` is rejected alongside M values, so the times are in the
        // coordinates themselves.
        assert_eq!(
            p.requested_time(None)
                .with_embedded_times(true)
                .cache_control(300),
            "max-age=300"
        );
        assert_eq!(
            p.requested_time(None)
                .with_embedded_times(false)
                .cache_control(300),
            "max-age=60"
        );
    }

    #[test]
    fn area_png_with_no_datetime_is_short_and_a_pinned_one_keeps_the_model_ttl() {
        // The PWA's "newest scan": mrms-single-level/area with no datetime.
        let latest = parse::<AreaQueryParams>(
            "coords=POLYGON((-100%2030,-90%2030,-90%2040,-100%2040,-100%2030))&f=image/png",
        );
        let pinned = parse::<AreaQueryParams>(
            "coords=POLYGON((-100%2030,-90%2030,-90%2040,-100%2040,-100%2030))&f=image/png&datetime=2026-10-07T20:00:00Z",
        );
        for configured in [300, 900, 86_400] {
            assert_eq!(
                latest.requested_time(None).cache_control(configured),
                "max-age=60"
            );
            assert_eq!(
                pinned.requested_time(None).cache_control(configured),
                format!("max-age={configured}")
            );
        }
    }

    #[test]
    fn server_side_cache_entries_for_latest_never_outlive_the_max_age() {
        let latest = server_cache_ttl(TimeSelection::Latest).expect("latest needs a cap");
        assert_eq!(latest, Duration::from_secs(LATEST_MAX_AGE_SECS as u64));
        // The cap is the very value sent as max-age.
        assert_eq!(
            latest.as_secs() as u32,
            TimeSelection::Latest.max_age_secs(300)
        );
        assert_eq!(server_cache_ttl(TimeSelection::Explicit), None);
    }

    #[test]
    fn png_ttl_from_model_config_is_capped_for_latest_and_kept_for_explicit() {
        // area.rs passes the model's configured png_max_age (default 300, up to
        // 86400 for historical models) as the explicit TTL.
        let latest = RequestedTime::default();
        assert_eq!(latest.cache_control(300), "max-age=60");
        assert_eq!(latest.cache_control(3600), "max-age=60");
        // A model configured shorter than the latest TTL is never lengthened.
        assert_eq!(latest.cache_control(30), "max-age=30");

        let pinned = RequestedTime {
            datetime: Some("2026-10-07T20:00:00Z"),
            ..Default::default()
        };
        assert_eq!(pinned.cache_control(3600), "max-age=3600");
    }
}
