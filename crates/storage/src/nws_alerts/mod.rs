//! NWS alerts (api.weather.gov `/alerts/active`): parsing, storage, lifecycle.
//!
//! Phase A of NWS alert support: the downloader polls NWS and POSTs the raw
//! GeoJSON features to the ingester, which calls [`parse_feature`] and
//! [`NwsAlertCatalog::apply_feed`]. Everything an app needs to *show* an alert
//! is computed once at ingest and stored (VTEC, sections, colour, effective
//! geometry), so the read side (EDR) is plain indexed SQL.
//!
//! * [`vtec`], [`sections`], [`colors`], [`lifecycle`]: pure, unit-tested.
//! * [`catalog`]: the PostGIS side (`nws_alerts`, `nws_zones`, `nws_alerts_status`).

pub mod catalog;
pub mod colors;
pub mod lifecycle;
pub mod sections;
pub mod vtec;

pub use catalog::{
    AlertRegion, AlertSearch, ApplyResult, FeedStatus, NwsAlertCatalog, StoredAlert, ZoneRecord,
    ZoneStatus,
};
pub use lifecycle::GeometrySource;
pub use vtec::{parse_vtec, Vtec};

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Marine UGC prefixes (first two letters of a `XXZnnn` code).
const MARINE_PREFIXES: &[&str] = &[
    "AM", "AN", "GM", "LC", "LE", "LH", "LM", "LO", "LS", "PH", "PK", "PM", "PS", "PZ", "SL",
];

/// Parameters that are bulky or only meaningful to broadcast systems.
const DROPPED_PARAMETERS: &[&str] = &["BLOCKCHANNEL", "EAS-ORG", "CMAMtext", "CMAMlongtext"];

/// One alert, parsed and pre-digested, ready to store.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedAlert {
    /// NWS id (URN), e.g. `urn:oid:2.49.0.1.840.0.<hash>.001.1`.
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
    pub sender: Option<String>,
    pub sender_name: Option<String>,
    pub web: Option<String>,
    pub sent: Option<DateTime<Utc>>,
    pub effective: Option<DateTime<Utc>>,
    pub onset: Option<DateTime<Utc>>,
    pub expires: Option<DateTime<Utc>>,
    pub ends: Option<DateTime<Utc>>,
    pub ugc: Vec<String>,
    pub same: Vec<String>,
    /// `forecast/KSZ001`-style keys of `affectedZones`, the join key to `nws_zones`.
    pub zone_keys: Vec<String>,
    pub is_marine: bool,
    pub vtec: Vec<Vtec>,
    pub sections: Value,
    pub parameters: Value,
    pub color: String,
    /// Ids of earlier messages this one replaces.
    pub references: Vec<String>,
    /// The alert's own polygon, as a GeoJSON geometry string.
    pub geometry_json: Option<String>,
    /// Already over on arrival (Cancel, or all VTEC actions CAN). VTEC EXP is not: it
    /// stays in force until its end time.
    pub ended_on_arrival: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("feature has no properties.id")]
    MissingId,
    #[error("alert {0} has no event")]
    MissingEvent(String),
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

fn ts(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|x| DateTime::parse_from_rfc3339(x).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `https://api.weather.gov/zones/forecast/KSZ001` -> `("forecast", "KSZ001")`.
pub fn zone_from_url(url: &str) -> Option<(String, String)> {
    let mut it = url.trim_end_matches('/').rsplit('/');
    let id = it.next()?;
    let ty = it.next()?;
    if id.is_empty() || ty.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((ty.to_string(), id.to_string()))
}

/// `"forecast/KSZ001"`.
pub fn zone_key(zone_type: &str, zone_id: &str) -> String {
    format!("{zone_type}/{zone_id}")
}

pub fn is_marine_ugc(code: &str) -> bool {
    code.len() == 6 && code.as_bytes()[2] == b'Z' && MARINE_PREFIXES.contains(&&code[..2])
}

/// Parse one GeoJSON feature from `/alerts/active`.
pub fn parse_feature(feature: &Value) -> Result<ParsedAlert, ParseError> {
    let p = feature.get("properties").unwrap_or(&Value::Null);
    let id = s(p, "id")
        .or_else(|| s(feature, "id").map(|u| u.rsplit('/').next().unwrap_or("").to_string()))
        .filter(|x| !x.is_empty())
        .ok_or(ParseError::MissingId)?;
    let event = s(p, "event").ok_or_else(|| ParseError::MissingEvent(id.clone()))?;
    let message_type = s(p, "messageType").unwrap_or_else(|| "Alert".into());

    let params = p.get("parameters").cloned().unwrap_or(Value::Null);
    let first_param = |k: &str| -> Option<String> {
        params
            .get(k)
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let vtec: Vec<Vtec> = strings(params.get("VTEC"))
        .iter()
        .filter_map(|v| parse_vtec(v))
        .collect();
    let parameters = match &params {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !DROPPED_PARAMETERS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        _ => Value::Object(Default::default()),
    };

    let geocode = p.get("geocode").unwrap_or(&Value::Null);
    let ugc = strings(geocode.get("UGC"));
    let same = strings(geocode.get("SAME"));
    let zone_keys: Vec<String> = {
        let mut keys: Vec<String> = Vec::new();
        for u in strings(p.get("affectedZones")) {
            if let Some((ty, zid)) = zone_from_url(&u) {
                let k = zone_key(&ty, &zid);
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    };
    let is_marine = !ugc.is_empty() && ugc.iter().all(|c| is_marine_ugc(c));

    let description = s(p, "description");
    let instruction = s(p, "instruction");
    let sections = description
        .as_deref()
        .map(sections::parse_sections)
        .unwrap_or_default()
        .to_json();

    let references: Vec<String> = p
        .get("references")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    s(r, "identifier").or_else(|| {
                        s(r, "@id").map(|u| u.rsplit('/').next().unwrap_or("").to_string())
                    })
                })
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let geometry_json = feature
        .get("geometry")
        .filter(|g| g.is_object())
        .map(|g| g.to_string());

    let significance = vtec.first().map(|v| v.significance);
    let color = colors::hazard_color(&event, significance).to_string();
    let ended_on_arrival = lifecycle::arrives_ended(&message_type, &vtec);

    Ok(ParsedAlert {
        description_text: description.as_deref().map(sections::normalize_text),
        instruction_text: instruction.as_deref().map(sections::normalize_text),
        id,
        event,
        severity: s(p, "severity"),
        urgency: s(p, "urgency"),
        certainty: s(p, "certainty"),
        category: s(p, "category"),
        status: s(p, "status"),
        message_type,
        response: s(p, "response"),
        headline: s(p, "headline"),
        nws_headline: first_param("NWSheadline"),
        description,
        instruction,
        area_desc: s(p, "areaDesc"),
        sender: s(p, "sender"),
        sender_name: s(p, "senderName"),
        web: s(p, "web"),
        sent: ts(p, "sent"),
        effective: ts(p, "effective"),
        onset: ts(p, "onset"),
        expires: ts(p, "expires"),
        ends: ts(p, "ends"),
        ugc,
        same,
        zone_keys,
        is_marine,
        vtec,
        sections,
        parameters,
        color,
        references,
        geometry_json,
        ended_on_arrival,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str =
        include_str!("../../../../services/downloader/tests/fixtures/nws_alerts_active.json");

    fn features() -> Vec<Value> {
        let v: Value = serde_json::from_str(FIXTURE).unwrap();
        v["features"].as_array().unwrap().clone()
    }

    fn by_event<'a>(all: &'a [ParsedAlert], ev: &str) -> Vec<&'a ParsedAlert> {
        all.iter().filter(|a| a.event == ev).collect()
    }

    fn parsed() -> Vec<ParsedAlert> {
        features()
            .iter()
            .map(|f| parse_feature(f).unwrap())
            .collect()
    }

    #[test]
    fn every_fixture_feature_parses() {
        let all = parsed();
        assert_eq!(all.len(), features().len());
        for a in &all {
            assert!(
                a.id.starts_with("urn:oid:") || a.id.contains("PALIVE"),
                "{}",
                a.id
            );
            assert!(a.color.starts_with('#'));
        }
    }

    #[test]
    fn polygon_alert_with_vtec_and_hazard_blocks() {
        let all = parsed();
        let svr = by_event(&all, "Severe Thunderstorm Warning")[0];
        assert!(svr.geometry_json.as_deref().unwrap().contains("Polygon"));
        assert_eq!(svr.vtec.len(), 1);
        assert_eq!(svr.vtec[0].event_key(), "KABQ.SV.W.0428");
        assert_eq!(svr.vtec[0].action, "NEW");
        assert_eq!(svr.color, "#FFA500");
        assert_eq!(svr.sections["hazard"], "60 mph wind gusts.");
        assert_eq!(svr.sections["source"], "Radar indicated.");
        assert!(svr.sections["impact"]
            .as_str()
            .unwrap()
            .starts_with("Expect damage"));
        assert!(!svr.ended_on_arrival);
        assert_eq!(svr.zone_keys[0], "county/NMC001");
        assert!(!svr.is_marine);
        assert_eq!(svr.sent.unwrap().to_rfc3339(), "2026-11-01T23:10:00+00:00");
    }

    #[test]
    fn zone_only_alert_with_what_where_when() {
        let all = parsed();
        let w = by_event(&all, "Winter Storm Warning")[0];
        assert!(w.geometry_json.is_none());
        assert_eq!(w.zone_keys, vec!["forecast/COZ068"]);
        assert_eq!(w.ugc, vec!["COZ068"]);
        assert_eq!(w.references.len(), 1);
        assert_eq!(w.message_type, "Update");
        assert!(w.sections["what"]
            .as_str()
            .unwrap()
            .starts_with("Heavy snow expected."));
        assert!(w.sections["where"].as_str().unwrap().contains("San Juan"));
        assert!(w.sections["when"].is_string() && w.sections["impacts"].is_string());
        assert_eq!(w.color, "#FF69B4");
        assert!(w.nws_headline.is_some());
        assert!(w.parameters.get("VTEC").is_some());
        assert!(w.parameters.get("BLOCKCHANNEL").is_none());
    }

    #[test]
    fn county_ugc_alert() {
        let all = parsed();
        let t = by_event(&all, "Test Message")[0];
        assert!(t.geometry_json.is_none());
        assert_eq!(t.zone_keys, vec!["county/MDC031"]);
        assert_eq!(t.ugc, vec!["MDC031"]);
        assert_eq!(t.status.as_deref(), Some("Test"));
        assert!(t.vtec.is_empty());
        assert!(
            t.sections["narrative"].is_array(),
            "plain text is all narrative"
        );
    }

    #[test]
    fn marine_alert() {
        let all = parsed();
        let m = by_event(&all, "Small Craft Advisory");
        assert!(m.iter().all(|a| a.is_marine));
        let two = m.iter().find(|a| a.zone_keys.len() == 2).unwrap();
        assert_eq!(two.zone_keys, vec!["forecast/PZZ455", "forecast/PZZ475"]);
        assert_eq!(two.vtec[0].significance, 'Y');
        assert_eq!(two.color, "#D8BFD8");
    }

    #[test]
    fn update_with_references_and_multiple_back_references() {
        let all = parsed();
        let many = all.iter().find(|a| a.references.len() >= 3).unwrap();
        assert!(many.references.iter().all(|r| r.starts_with("urn:oid:")));
        assert_eq!(many.message_type, "Update");
    }

    #[test]
    fn expiring_vtec_is_not_ended_on_arrival() {
        let all = parsed();
        let ff = by_event(&all, "Flash Flood Warning")[0];
        assert_eq!(ff.vtec[0].action, "EXP");
        assert!(
            !ff.ended_on_arrival,
            "EXP = will expire at the end time; its `ends` decides"
        );
        assert_eq!(ff.ends.unwrap().to_rfc3339(), "2026-11-01T23:30:00+00:00");
        assert_eq!(ff.color, "#8B0000");
        assert!(!by_event(&all, "Winter Storm Warning")[0].ended_on_arrival);
    }

    #[test]
    fn can_vtec_and_cancel_arrive_ended() {
        let mut f = features()
            .into_iter()
            .find(|f| f["properties"]["event"] == "Flash Flood Warning")
            .unwrap();
        f["properties"]["parameters"]["VTEC"] =
            serde_json::json!(["/O.CAN.KFGZ.FF.W.0209.000000T0000Z-261101T2330Z/"]);
        assert!(parse_feature(&f).unwrap().ended_on_arrival);
    }

    #[test]
    fn missing_pieces() {
        assert!(matches!(
            parse_feature(&serde_json::json!({})),
            Err(ParseError::MissingId)
        ));
        assert!(matches!(
            parse_feature(&serde_json::json!({"properties": {"id": "x"}})),
            Err(ParseError::MissingEvent(_))
        ));
        let a = parse_feature(&serde_json::json!({"properties": {"id": "x", "event": "Odd Warning", "messageType": "Cancel"}})).unwrap();
        assert!(
            a.ended_on_arrival && a.zone_keys.is_empty() && a.color == colors::FALLBACK_WARNING
        );
    }

    #[test]
    fn zone_urls() {
        assert_eq!(
            zone_from_url("https://api.weather.gov/zones/forecast/KSZ001"),
            Some(("forecast".into(), "KSZ001".into()))
        );
        assert_eq!(
            zone_from_url("https://api.weather.gov/zones/fire/WYZ300/"),
            Some(("fire".into(), "WYZ300".into()))
        );
        assert_eq!(zone_from_url("nonsense"), None);
        assert!(is_marine_ugc("ANZ330") && is_marine_ugc("PZZ571") && is_marine_ugc("GMZ650"));
        assert!(!is_marine_ugc("KSZ001") && !is_marine_ugc("KSC045"));
    }
}
