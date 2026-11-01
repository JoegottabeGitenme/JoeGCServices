//! P-VTEC ("Valid Time Event Code") parsing.
//!
//! NWS products carry one or more strings like
//! `/O.NEW.KTOP.WS.W.0012.261102T0600Z-261103T0000Z/`:
//!
//! | part | meaning |
//! |------|---------|
//! | `O` | product class (O operational, T test, E experimental, X experimental VTEC) |
//! | `NEW` | action: NEW CON EXT EXA EXB UPG CAN EXP COR ROU |
//! | `KTOP` | issuing office |
//! | `WS` | phenomenon (Winter Storm) |
//! | `W` | significance: W warning, A watch, Y advisory, S statement, F forecast, O outlook, N synopsis |
//! | `0012` | event tracking number (per office, phenomenon, significance, year) |
//! | `261102T0600Z-261103T0000Z` | begin and end (`000000T0000Z` = not specified) |

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vtec {
    pub raw: String,
    pub product_class: char,
    pub action: String,
    pub office: String,
    pub phenomenon: String,
    pub significance: char,
    pub etn: u32,
    pub begin: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl Vtec {
    /// Identity of the event this message belongs to; stable across NEW, CON, EXT
    /// ... CAN. Year is not included (the begin/end time disambiguates it in
    /// practice); `office.phenomenon.significance.etn`.
    pub fn event_key(&self) -> String {
        format!(
            "{}.{}.{}.{:04}",
            self.office, self.phenomenon, self.significance, self.etn
        )
    }

    /// True for actions that mean the hazard is over (cancelled or expired).
    pub fn is_terminal(&self) -> bool {
        matches!(self.action.as_str(), "CAN" | "EXP")
    }
}

/// `YYMMDDTHHMMZ`; all-zero means "not specified".
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.strip_suffix('Z')?;
    if s.starts_with("000000") {
        return None;
    }
    NaiveDateTime::parse_from_str(s, "%y%m%dT%H%M")
        .ok()
        .map(|n| n.and_utc())
}

/// Parse one VTEC string. Returns `None` for anything malformed.
pub fn parse_vtec(raw: &str) -> Option<Vtec> {
    let body = raw.trim().trim_matches('/');
    let parts: Vec<&str> = body.split('.').collect();
    if parts.len() != 7 {
        return None;
    }
    let single = |s: &str| -> Option<char> {
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    };
    let (begin, end) = parts[6].split_once('-')?;
    let action = parts[1];
    if action.len() != 3 || parts[2].len() != 4 || parts[3].len() < 2 || parts[3].len() > 3 {
        return None;
    }
    Some(Vtec {
        raw: raw.trim().to_string(),
        product_class: single(parts[0])?,
        action: action.to_string(),
        office: parts[2].to_string(),
        phenomenon: parts[3].to_string(),
        significance: single(parts[4])?,
        etn: parts[5].parse().ok()?,
        begin: parse_time(begin),
        end: parse_time(end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_a_new_winter_storm_warning() {
        let v = parse_vtec("/O.NEW.KTOP.WS.W.0012.261102T0600Z-261103T0000Z/").unwrap();
        assert_eq!(v.product_class, 'O');
        assert_eq!(v.action, "NEW");
        assert_eq!(v.office, "KTOP");
        assert_eq!(v.phenomenon, "WS");
        assert_eq!(v.significance, 'W');
        assert_eq!(v.etn, 12);
        assert_eq!(
            v.begin,
            Some(Utc.with_ymd_and_hms(2026, 11, 2, 6, 0, 0).unwrap())
        );
        assert_eq!(
            v.end,
            Some(Utc.with_ymd_and_hms(2026, 11, 3, 0, 0, 0).unwrap())
        );
        assert_eq!(v.event_key(), "KTOP.WS.W.0012");
        assert!(!v.is_terminal());
    }

    #[test]
    fn zero_begin_means_unspecified() {
        let v = parse_vtec("/O.CON.KEKA.SC.Y.0099.000000T0000Z-261102T0500Z/").unwrap();
        assert_eq!(v.begin, None);
        assert_eq!(
            v.end,
            Some(Utc.with_ymd_and_hms(2026, 11, 2, 5, 0, 0).unwrap())
        );
        assert_eq!(v.significance, 'Y');
    }

    #[test]
    fn zero_end_means_until_further_notice() {
        let v = parse_vtec("/O.EXP.KFGZ.FF.W.0209.000000T0000Z-000000T0000Z/").unwrap();
        assert_eq!((v.begin, v.end), (None, None));
        assert!(v.is_terminal());
    }

    #[test]
    fn cancel_is_terminal() {
        let v = parse_vtec("/O.CAN.KABQ.SV.W.0428.000000T0000Z-261102T0000Z/").unwrap();
        assert!(v.is_terminal());
    }

    #[test]
    fn tolerates_whitespace_and_missing_slashes() {
        assert!(parse_vtec("  /O.CON.PAJK.GL.W.0123.000000T0000Z-261102T0300Z/ ").is_some());
        assert!(parse_vtec("O.CON.PAJK.GL.W.0123.000000T0000Z-261102T0300Z").is_some());
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_vtec("").is_none());
        assert!(parse_vtec("/O.NEW.KTOP.WS.W.0012/").is_none());
        assert!(parse_vtec("/O.NEW.KTOP.WS.W.XXXX.261102T0600Z-261103T0000Z/").is_none());
        assert!(parse_vtec("/OO.NEW.KTOP.WS.W.0012.261102T0600Z-261103T0000Z/").is_none());
        assert!(parse_vtec("/O.NEW.KTOP.WS.W.0012.badtime-261103T0000Z/")
            .is_some_and(|v| v.begin.is_none()));
    }
}
