//! Alert lifecycle rules, kept free of I/O so they can be unit tested.
//!
//! An alert row is **active** at time `t` when all of:
//! 1. it has not been superseded by a later message (`superseded_by IS NULL`);
//! 2. it has not ended (`ended_at IS NULL`) -- set when a poll no longer lists
//!    it, when it arrives as a `Cancel`, or when its VTEC says CAN/EXP;
//! 3. its end time has not passed: `end_time = COALESCE(ends, expires)`.
//!    NWS `ends` is when the *hazard* ends; `expires` is when the *message*
//!    expires (often earlier, e.g. the next scheduled update), so `ends` wins
//!    and `expires` is only the fallback. No end time at all means "until the
//!    feed drops it".
//!
//! Each successful poll is authoritative for which alerts are current. A failed
//! poll changes nothing (see `NwsAlertCatalog::record_poll_failure`).

use chrono::{DateTime, Utc};

use super::vtec::Vtec;

/// Where an alert's effective geometry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometrySource {
    /// The alert carried its own polygon.
    Polygon,
    /// Union of the alert's zone geometries; every zone has been resolved.
    Zones,
    /// Some zones have not been fetched yet; `geom` holds whatever is known.
    Pending,
    /// Zones were all resolved but none has geometry (404s, empty zone list).
    None,
}

impl GeometrySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Polygon => "polygon",
            Self::Zones => "zones",
            Self::Pending => "pending",
            Self::None => "none",
        }
    }
}

/// Decide the geometry source of a zone-based alert.
pub fn zone_geometry_source(unresolved_zones: usize, zones_with_geometry: usize) -> GeometrySource {
    if unresolved_zones > 0 {
        GeometrySource::Pending
    } else if zones_with_geometry > 0 {
        GeometrySource::Zones
    } else {
        GeometrySource::None
    }
}

/// Whether a message is already over when we first see it: a `Cancel`, or one
/// whose every VTEC action is CAN/EXP.
pub fn arrives_ended(message_type: &str, vtec: &[Vtec]) -> bool {
    message_type.eq_ignore_ascii_case("cancel")
        || (!vtec.is_empty() && vtec.iter().all(Vtec::is_terminal))
}

/// Ids an incoming message supersedes: its `references`, minus itself, deduplicated.
pub fn supersede_targets(self_id: &str, references: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in references {
        if r != self_id && !out.contains(r) {
            out.push(r.clone());
        }
    }
    out
}

/// The alert's state columns, for [`is_active`].
#[derive(Debug, Clone, Copy)]
pub struct AlertTimes {
    pub superseded: bool,
    pub ended: bool,
    pub ends: Option<DateTime<Utc>>,
    pub expires: Option<DateTime<Utc>>,
}

pub fn end_time(
    ends: Option<DateTime<Utc>>,
    expires: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    ends.or(expires)
}

/// The "active" predicate; the SQL in `catalog.rs` mirrors this exactly.
pub fn is_active(a: &AlertTimes, now: DateTime<Utc>) -> bool {
    !a.superseded && !a.ended && end_time(a.ends, a.expires).is_none_or(|t| t > now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nws_alerts::vtec::parse_vtec;
    use chrono::{Duration, TimeZone};

    fn t(h: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap() + Duration::hours(h)
    }
    fn a(superseded: bool, ended: bool, ends: Option<i64>, expires: Option<i64>) -> AlertTimes {
        AlertTimes {
            superseded,
            ended,
            ends: ends.map(t),
            expires: expires.map(t),
        }
    }

    #[test]
    fn active_until_end_time() {
        let now = t(0);
        assert!(is_active(&a(false, false, Some(2), None), now));
        assert!(!is_active(&a(false, false, Some(-1), None), now));
        assert!(
            !is_active(&a(false, false, Some(0), None), now),
            "end is exclusive"
        );
    }

    #[test]
    fn ends_wins_over_expires() {
        let now = t(0);
        // message expired an hour ago, hazard runs two more hours: still active
        assert!(is_active(&a(false, false, Some(2), Some(-1)), now));
        // hazard over, message not yet expired: not active
        assert!(!is_active(&a(false, false, Some(-1), Some(3)), now));
        // no ends: expires decides
        assert!(is_active(&a(false, false, None, Some(1)), now));
        assert!(!is_active(&a(false, false, None, Some(-1)), now));
    }

    #[test]
    fn no_end_time_is_active_until_dropped() {
        assert!(is_active(&a(false, false, None, None), t(0)));
    }

    #[test]
    fn superseded_or_ended_is_inactive() {
        assert!(!is_active(&a(true, false, Some(5), None), t(0)));
        assert!(!is_active(&a(false, true, Some(5), None), t(0)));
    }

    #[test]
    fn cancel_and_terminal_vtec_arrive_ended() {
        let can = parse_vtec("/O.CAN.KABQ.SV.W.0428.000000T0000Z-261102T0000Z/").unwrap();
        let exp = parse_vtec("/O.EXP.KABQ.SV.W.0428.000000T0000Z-261102T0000Z/").unwrap();
        let con = parse_vtec("/O.CON.KABQ.SV.W.0428.000000T0000Z-261102T0000Z/").unwrap();
        assert!(arrives_ended("Cancel", &[]));
        assert!(arrives_ended("Update", std::slice::from_ref(&exp)));
        assert!(arrives_ended("Update", &[can.clone(), exp.clone()]));
        assert!(!arrives_ended("Update", std::slice::from_ref(&con)));
        // one live event in a multi-VTEC message keeps the message alive
        assert!(!arrives_ended("Update", &[can, con]));
        assert!(!arrives_ended("Alert", &[]));
    }

    #[test]
    fn supersede_targets_drop_self_and_dups() {
        let refs = vec![
            "a".to_string(),
            "me".to_string(),
            "a".to_string(),
            "b".to_string(),
        ];
        assert_eq!(supersede_targets("me", &refs), vec!["a", "b"]);
        assert!(supersede_targets("me", &[]).is_empty());
    }

    #[test]
    fn geometry_source_transitions() {
        assert_eq!(zone_geometry_source(2, 0), GeometrySource::Pending);
        assert_eq!(zone_geometry_source(1, 3), GeometrySource::Pending);
        assert_eq!(zone_geometry_source(0, 3), GeometrySource::Zones);
        assert_eq!(zone_geometry_source(0, 0), GeometrySource::None);
        assert_eq!(GeometrySource::Polygon.as_str(), "polygon");
    }
}
