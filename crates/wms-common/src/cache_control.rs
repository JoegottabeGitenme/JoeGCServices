//! `Cache-Control` policy shared by the HTTP services (wms-api, edr-api).
//!
//! WHY this exists: folkweather.com sits behind Cloudflare, which stores any
//! response that carries a `Cache-Control: max-age` and serves it to everyone
//! until it expires. That is right for a response about an *explicit* time (a
//! given radar scan, a given model run and forecast hour never changes), but
//! wrong for a response whose time the *server* picked ("latest": the newest
//! scan, the newest run). Those change underneath an unchanged URL every few
//! minutes (MRMS every ~2 min, GOES every 5-10 min), so an hour-long `max-age`
//! shows a viewer an hour-old scan as if it were current.
//!
//! The rule lives here, once, instead of as ~30 hand-edited literals:
//!
//! - a response is **latest** when the request named no time and the server
//!   chose the default / newest time, run or forecast hour. It gets
//!   [`LATEST_MAX_AGE_SECS`].
//! - a response for an **explicit** time keeps whatever TTL its handler always
//!   used (callers pass that value in, so nothing about explicit responses
//!   changes).
//!
//! A latest response never gets a *longer* TTL than the explicit one would
//! (`min`), so a handler that already used a shorter TTL (for example 5
//! seconds for lightning) is not made slower to refresh.

/// `max-age` for a response whose time the server picked ("latest").
///
/// 60 s is below the fastest gridded update cadence we serve (MRMS ~2 min),
/// so a cached copy is never more than one cadence behind, while still letting
/// the CDN absorb a burst of identical requests. 30 s for radar-only was
/// considered and rejected: it would need per-layer cadence knowledge at every
/// call site for no visible gain, because the data itself only moves every
/// ~2 minutes.
pub const LATEST_MAX_AGE_SECS: u32 = 60;

/// Whether a request named its time or let the server choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSelection {
    /// The request pinned the time (observation time, run, forecast hour,
    /// instance, closed `datetime`). The response is stable.
    Explicit,
    /// The server picked the default / newest time. The response changes when
    /// new data arrives.
    Latest,
}

impl TimeSelection {
    /// `Explicit` when `explicit` is true, otherwise `Latest`.
    pub fn from_explicit(explicit: bool) -> Self {
        if explicit {
            Self::Explicit
        } else {
            Self::Latest
        }
    }

    /// True for a server-chosen ("latest") time.
    pub fn is_latest(self) -> bool {
        self == Self::Latest
    }

    /// The `max-age` in seconds. `explicit_secs` is the handler's existing TTL
    /// for explicit-time responses; a latest response is capped at
    /// [`LATEST_MAX_AGE_SECS`] and never raised above `explicit_secs`.
    pub fn max_age_secs(self, explicit_secs: u32) -> u32 {
        match self {
            Self::Explicit => explicit_secs,
            Self::Latest => explicit_secs.min(LATEST_MAX_AGE_SECS),
        }
    }

    /// The `Cache-Control` header value, for example `max-age=60`.
    pub fn cache_control(self, explicit_secs: u32) -> String {
        format!("max-age={}", self.max_age_secs(explicit_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_keeps_the_handlers_ttl() {
        assert_eq!(TimeSelection::Explicit.max_age_secs(300), 300);
        assert_eq!(TimeSelection::Explicit.max_age_secs(3600), 3600);
        assert_eq!(TimeSelection::Explicit.cache_control(3600), "max-age=3600");
    }

    #[test]
    fn latest_is_capped_at_the_latest_ttl() {
        assert_eq!(TimeSelection::Latest.max_age_secs(300), LATEST_MAX_AGE_SECS);
        assert_eq!(
            TimeSelection::Latest.max_age_secs(3600),
            LATEST_MAX_AGE_SECS
        );
        assert_eq!(TimeSelection::Latest.cache_control(300), "max-age=60");
    }

    #[test]
    fn latest_never_extends_a_shorter_explicit_ttl() {
        assert_eq!(TimeSelection::Latest.max_age_secs(5), 5);
        assert_eq!(TimeSelection::Latest.max_age_secs(60), 60);
        assert_eq!(TimeSelection::Latest.cache_control(5), "max-age=5");
    }

    #[test]
    fn from_explicit_maps_the_flag() {
        assert_eq!(TimeSelection::from_explicit(true), TimeSelection::Explicit);
        assert_eq!(TimeSelection::from_explicit(false), TimeSelection::Latest);
        assert!(TimeSelection::from_explicit(false).is_latest());
        assert!(!TimeSelection::from_explicit(true).is_latest());
    }
}
