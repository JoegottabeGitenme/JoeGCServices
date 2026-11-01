//! NWS hazard colours, as used on the national hazards map
//! (<https://www.weather.gov/help-map>).
//!
//! The table below is embedded statically: NWS publishes it as a web page, not an
//! API, and it changes a few times a decade. Events that are not listed (new
//! products, `Test Message`, ...) fall back to a colour by significance
//! (warning / watch / advisory / statement, taken from the event name or VTEC)
//! and then to a neutral grey, so every alert always has a colour.

/// `(event name, "#RRGGBB")`, exact match on the NWS event name.
const EVENT_COLORS: &[(&str, &str)] = &[
    ("911 Telephone Outage Emergency", "#C0C0C0"),
    ("Administrative Message", "#C0C0C0"),
    ("Air Quality Alert", "#808080"),
    ("Air Stagnation Advisory", "#808080"),
    ("Arroyo And Small Stream Flood Advisory", "#00FF7F"),
    ("Ashfall Advisory", "#696969"),
    ("Ashfall Warning", "#A9A9A9"),
    ("Avalanche Advisory", "#CD853F"),
    ("Avalanche Warning", "#1E90FF"),
    ("Avalanche Watch", "#F4A460"),
    ("Beach Hazards Statement", "#40E0D0"),
    ("Blizzard Warning", "#FF4500"),
    ("Blizzard Watch", "#ADFF2F"),
    ("Blowing Dust Advisory", "#BDB76B"),
    ("Blowing Dust Warning", "#FFE4C4"),
    ("Brisk Wind Advisory", "#D8BFD8"),
    ("Child Abduction Emergency", "#FFD700"),
    ("Civil Danger Warning", "#FFB6C1"),
    ("Civil Emergency Message", "#FFB6C1"),
    ("Coastal Flood Advisory", "#7CFC00"),
    ("Coastal Flood Statement", "#6B8E23"),
    ("Coastal Flood Warning", "#228B22"),
    ("Coastal Flood Watch", "#66CDAA"),
    ("Cold Weather Advisory", "#AFEEEE"),
    ("Dense Fog Advisory", "#708090"),
    ("Dense Smoke Advisory", "#F0E68C"),
    ("Dust Advisory", "#BDB76B"),
    ("Dust Storm Warning", "#FFE4C4"),
    ("Earthquake Warning", "#8B4513"),
    ("Evacuation - Immediate", "#7FFF00"),
    ("Excessive Heat Warning", "#C71585"),
    ("Excessive Heat Watch", "#800000"),
    ("Extreme Cold Warning", "#0000FF"),
    ("Extreme Cold Watch", "#5F9EA0"),
    ("Extreme Fire Danger", "#E9967A"),
    ("Extreme Wind Warning", "#FF8C00"),
    ("Fire Warning", "#A0522D"),
    ("Fire Weather Watch", "#FFDEAD"),
    ("Flash Flood Statement", "#8B0000"),
    ("Flash Flood Warning", "#8B0000"),
    ("Flash Flood Watch", "#2E8B57"),
    ("Flood Advisory", "#00FF7F"),
    ("Flood Statement", "#00FF00"),
    ("Flood Warning", "#00FF00"),
    ("Flood Watch", "#2E8B57"),
    ("Freeze Warning", "#483D8B"),
    ("Freeze Watch", "#00FFFF"),
    ("Freezing Fog Advisory", "#008080"),
    ("Freezing Rain Advisory", "#DA70D6"),
    ("Freezing Spray Advisory", "#00BFFF"),
    ("Frost Advisory", "#6495ED"),
    ("Gale Warning", "#DDA0DD"),
    ("Gale Watch", "#FFC0CB"),
    ("Hard Freeze Warning", "#9400D3"),
    ("Hard Freeze Watch", "#4169E1"),
    ("Hazardous Materials Warning", "#4B0082"),
    ("Hazardous Seas Warning", "#D8BFD8"),
    ("Hazardous Seas Watch", "#483D8B"),
    ("Hazardous Weather Outlook", "#EEE8AA"),
    ("Heat Advisory", "#FF7F50"),
    ("Heavy Freezing Spray Warning", "#00BFFF"),
    ("Heavy Freezing Spray Watch", "#BC8F8F"),
    ("High Surf Advisory", "#BA55D3"),
    ("High Surf Warning", "#228B22"),
    ("High Wind Warning", "#DAA520"),
    ("High Wind Watch", "#B8860B"),
    ("Hurricane Force Wind Warning", "#CD5C5C"),
    ("Hurricane Force Wind Watch", "#9932CC"),
    ("Hurricane Local Statement", "#FFE4B5"),
    ("Hurricane Warning", "#DC143C"),
    ("Hurricane Watch", "#FF00FF"),
    ("Hydrologic Advisory", "#00FF7F"),
    ("Hydrologic Outlook", "#90EE90"),
    ("Ice Storm Warning", "#8B008B"),
    ("Lake Effect Snow Advisory", "#48D1CC"),
    ("Lake Effect Snow Warning", "#008B8B"),
    ("Lake Effect Snow Watch", "#87CEFA"),
    ("Lake Wind Advisory", "#D2B48C"),
    ("Lakeshore Flood Advisory", "#7CFC00"),
    ("Lakeshore Flood Statement", "#6B8E23"),
    ("Lakeshore Flood Warning", "#228B22"),
    ("Lakeshore Flood Watch", "#66CDAA"),
    ("Law Enforcement Warning", "#C0C0C0"),
    ("Local Area Emergency", "#C0C0C0"),
    ("Low Water Advisory", "#A52A2A"),
    ("Marine Weather Statement", "#FFDAB9"),
    ("Nuclear Power Plant Warning", "#4B0082"),
    ("Radiological Hazard Warning", "#4B0082"),
    ("Red Flag Warning", "#FF1493"),
    ("Rip Current Statement", "#40E0D0"),
    ("Severe Thunderstorm Warning", "#FFA500"),
    ("Severe Thunderstorm Watch", "#DB7093"),
    ("Severe Weather Statement", "#00FFFF"),
    ("Shelter In Place Warning", "#FA8072"),
    ("Short Term Forecast", "#98FB98"),
    ("Small Craft Advisory", "#D8BFD8"),
    ("Small Craft Advisory For Hazardous Seas", "#D8BFD8"),
    ("Small Craft Advisory For Rough Bar", "#D8BFD8"),
    ("Small Craft Advisory For Winds", "#D8BFD8"),
    ("Small Stream Flood Advisory", "#00FF7F"),
    ("Snow Squall Warning", "#C71585"),
    ("Special Marine Warning", "#FFA500"),
    ("Special Weather Statement", "#FFE4B5"),
    ("Storm Surge Warning", "#B524F7"),
    ("Storm Surge Watch", "#DB7FF7"),
    ("Storm Warning", "#9400D3"),
    ("Storm Watch", "#FFE4B5"),
    ("Tornado Warning", "#FF0000"),
    ("Tornado Watch", "#FFFF00"),
    ("Tropical Depression Local Statement", "#FFE4B5"),
    ("Tropical Storm Local Statement", "#FFE4B5"),
    ("Tropical Storm Warning", "#B22222"),
    ("Tropical Storm Watch", "#F08080"),
    ("Tsunami Advisory", "#D2691E"),
    ("Tsunami Warning", "#FD6347"),
    ("Tsunami Watch", "#FF00FF"),
    ("Typhoon Local Statement", "#FFE4B5"),
    ("Typhoon Warning", "#DC143C"),
    ("Typhoon Watch", "#FF00FF"),
    ("Urban And Small Stream Flood Advisory", "#00FF7F"),
    ("Volcano Warning", "#2F4F4F"),
    ("Wind Advisory", "#D2B48C"),
    ("Wind Chill Advisory", "#AFEEEE"),
    ("Wind Chill Warning", "#B0C4DE"),
    ("Wind Chill Watch", "#5F9EA0"),
    ("Winter Storm Warning", "#FF69B4"),
    ("Winter Storm Watch", "#4682B4"),
    ("Winter Weather Advisory", "#7B68EE"),
];

pub const FALLBACK_WARNING: &str = "#FF6347";
pub const FALLBACK_WATCH: &str = "#FFD700";
pub const FALLBACK_ADVISORY: &str = "#EEE8AA";
pub const FALLBACK_STATEMENT: &str = "#D3D3D3";
pub const FALLBACK_OTHER: &str = "#A9A9A9";

/// Colour for an alert: the NWS table by event name, else by significance
/// (`W` warning, `A` watch, `Y` advisory, `S` statement; from VTEC when known,
/// else from the event name's last word), else grey.
pub fn hazard_color(event: &str, significance: Option<char>) -> &'static str {
    if let Some((_, c)) = EVENT_COLORS.iter().find(|(e, _)| *e == event) {
        return c;
    }
    let sig = significance.or_else(|| match event.rsplit(' ').next().unwrap_or("") {
        "Warning" => Some('W'),
        "Watch" => Some('A'),
        "Advisory" => Some('Y'),
        "Statement" | "Message" | "Outlook" => Some('S'),
        _ => None,
    });
    match sig {
        Some('W') => FALLBACK_WARNING,
        Some('A') => FALLBACK_WATCH,
        Some('Y') => FALLBACK_ADVISORY,
        Some('S') => FALLBACK_STATEMENT,
        _ => FALLBACK_OTHER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_events() {
        assert_eq!(hazard_color("Tornado Warning", Some('W')), "#FF0000");
        assert_eq!(hazard_color("Winter Storm Warning", None), "#FF69B4");
        assert_eq!(hazard_color("Small Craft Advisory", Some('Y')), "#D8BFD8");
        assert_eq!(hazard_color("Gale Warning", None), "#DDA0DD");
    }

    #[test]
    fn unknown_event_falls_back_by_significance() {
        assert_eq!(hazard_color("Brand New Warning", None), FALLBACK_WARNING);
        assert_eq!(hazard_color("Brand New Watch", None), FALLBACK_WATCH);
        assert_eq!(hazard_color("Brand New Advisory", None), FALLBACK_ADVISORY);
        // VTEC significance wins over the name.
        assert_eq!(hazard_color("Odd Thing", Some('A')), FALLBACK_WATCH);
        assert_eq!(hazard_color("Test Message", None), FALLBACK_STATEMENT);
        assert_eq!(hazard_color("Mystery", None), FALLBACK_OTHER);
    }

    #[test]
    fn table_is_well_formed_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (e, c) in EVENT_COLORS {
            assert!(seen.insert(*e), "duplicate event {e}");
            assert!(
                c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit()),
                "bad colour {c} for {e}"
            );
        }
    }
}
