//! Structured sections of an NWS alert description.
//!
//! NWS descriptions are hard-wrapped plain text. Two layouts matter:
//!
//! ```text
//! * WHAT...Heavy snow expected. Total snow accumulations 8 to 14
//! inches.
//!
//! * WHERE...Eastern San Juan Mountains Above 10000 Feet.
//! * WHEN...From 5 PM this afternoon to 11 AM MST Monday.
//! * IMPACTS...Travel could be very difficult.
//! * ADDITIONAL DETAILS...
//! - A strong weather system approaching ...
//! ```
//!
//! and the thunderstorm-warning / statement layout:
//!
//! ```text
//! SVRABQ
//!
//! The National Weather Service in Albuquerque has issued a
//!
//! * Severe Thunderstorm Warning for...
//! * At 410 PM MST, severe thunderstorms were located ...
//!
//! HAZARD...60 mph wind gusts.
//!
//! SOURCE...Radar indicated.
//!
//! IMPACT...Expect damage to roofs, siding, and trees.
//! ```
//!
//! A label may also have no space after the star (`*WHAT...`), as the Colorado avalanche
//! advisories write it.
//!
//! Recognised labels become keys (`what`, `where`, `when`, `impacts`,
//! `additional_details`, `precautionary_actions`, `hazard`, `source`, `impact`;
//! any other all-caps `* LABEL...` becomes a slug of itself). Text outside a
//! labelled section is returned, in order, as `narrative` paragraphs. Hard
//! wraps inside a paragraph are joined with a space; `- ` list items keep their
//! own line.

use std::collections::BTreeMap;

use serde_json::{json, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    /// Labelled sections by key (see module docs).
    pub fields: BTreeMap<String, String>,
    /// Unlabelled paragraphs, in order.
    pub narrative: Vec<String>,
}

impl Sections {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.narrative.is_empty()
    }

    /// `{"what": "...", ..., "narrative": ["...", ...]}`; only present keys.
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.fields {
            m.insert(k.clone(), json!(v));
        }
        if !self.narrative.is_empty() {
            m.insert("narrative".into(), json!(self.narrative));
        }
        Value::Object(m)
    }
}

/// Collapse hard-wrapped text into paragraphs: blank lines separate paragraphs
/// (joined with `\n\n`), wrapped lines inside one are joined with a space, and a
/// line starting with `- ` / `* ` / a bullet starts its own line.
pub fn normalize_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    normalize_lines(&lines)
}

fn is_item(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("\u{2022}")
        || star_header(t.trim_end()).is_some()
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn paragraphs(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        let t = cur.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
        cur.clear();
    };
    for line in lines {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut cur, &mut out);
            continue;
        }
        let t = collapse_spaces(t);
        if cur.is_empty() {
            cur = t;
        } else if is_item(line) {
            cur.push('\n');
            cur.push_str(&t);
        } else {
            cur.push(' ');
            cur.push_str(&t);
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn normalize_lines(lines: &[&str]) -> String {
    paragraphs(lines).join("\n\n")
}

/// A recognised label's output key.
fn label_key(label: &str) -> String {
    match label {
        "WHAT" => "what".into(),
        "WHERE" => "where".into(),
        "WHEN" => "when".into(),
        "IMPACTS" => "impacts".into(),
        "IMPACT" => "impact".into(),
        "HAZARD" => "hazard".into(),
        "SOURCE" => "source".into(),
        "ADDITIONAL DETAILS" => "additional_details".into(),
        // (Denver's avalanche advisories drop the "ARY")
        "PRECAUTIONARY/PREPAREDNESS ACTIONS" | "PRECAUTION/PREPAREDNESS ACTIONS" => {
            "precautionary_actions".into()
        }
        other => {
            let mut slug = String::new();
            for c in other.chars() {
                if c.is_ascii_alphanumeric() {
                    slug.push(c.to_ascii_lowercase());
                } else if !slug.ends_with('_') {
                    slug.push('_');
                }
            }
            slug.trim_matches('_').to_string()
        }
    }
}

/// `LABEL...rest`: label is upper-case letters, spaces, `/`, `&`, `-`.
fn split_label(s: &str) -> Option<(&str, &str)> {
    let (label, rest) = s.split_once("...")?;
    let label = label.trim();
    let ok = !label.is_empty()
        && label.len() <= 40
        && label.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && label
            .chars()
            .all(|c| c.is_ascii_uppercase() || matches!(c, ' ' | '/' | '&' | '-'));
    ok.then_some((label, rest))
}

/// `* LABEL...rest` or `*LABEL...rest` (some offices, e.g. the Colorado avalanche advisories,
/// leave out the space after the star): the label and the text after it.
fn star_header(s: &str) -> Option<(&str, &str)> {
    split_label(s.strip_prefix('*')?)
}

/// Labels allowed without a leading `* `.
fn bare_label(label: &str) -> bool {
    matches!(
        label,
        "HAZARD" | "SOURCE" | "IMPACT" | "IMPACTS" | "WHAT" | "WHERE" | "WHEN"
    )
}

struct Block<'a> {
    key: Option<String>,
    /// `* LABEL...` blocks run across blank lines; bare `HAZARD...` end at one.
    star: bool,
    lines: Vec<&'a str>,
}

pub fn parse_sections(description: &str) -> Sections {
    let text = description.replace('\r', "");
    let mut lines: Vec<&str> = text.lines().collect();

    // Drop the WMO/AWIPS product id line (`SVRABQ`) some products start with.
    if let Some(first) = lines.iter().position(|l| !l.trim().is_empty()) {
        let t = lines[first].trim();
        if (4..=6).contains(&t.len())
            && t.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            lines.remove(first);
        }
    }

    let mut blocks: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;
    macro_rules! flush {
        () => {
            if let Some(b) = cur.take() {
                blocks.push(b);
            }
        };
    }

    for line in lines {
        let t = line.trim_end();
        let trimmed = t.trim();
        if trimmed == "&&" || trimmed == "$$" {
            break;
        }
        // `* LABEL...rest`, `*LABEL...rest` or bare `HAZARD...rest`
        let header = match star_header(trimmed) {
            Some((l, r)) => Some((l, r, true)),
            None => split_label(trimmed)
                .filter(|(l, _)| bare_label(l))
                .map(|(l, r)| (l, r, false)),
        };
        if let Some((label, rest, star)) = header {
            flush!();
            cur = Some(Block {
                key: Some(label_key(label)),
                star,
                lines: vec![rest],
            });
        } else if let Some(body) = trimmed.strip_prefix("* ") {
            // An unlabelled bullet closes any open section and starts narrative.
            flush!();
            cur = Some(Block {
                key: None,
                star: true,
                lines: vec![body],
            });
        } else if trimmed.is_empty() {
            let bare_open = cur.as_ref().is_some_and(|b| b.key.is_some() && !b.star);
            if bare_open {
                flush!();
            } else if let Some(b) = cur.as_mut() {
                b.lines.push("");
            }
        } else {
            match cur.as_mut() {
                Some(b) => b.lines.push(t),
                None => {
                    cur = Some(Block {
                        key: None,
                        star: false,
                        lines: vec![t],
                    })
                }
            }
        }
    }
    flush!();

    let mut out = Sections::default();
    for b in blocks {
        match b.key {
            Some(k) => {
                let body = normalize_lines(&b.lines);
                if body.is_empty() {
                    continue;
                }
                out.fields
                    .entry(k)
                    .and_modify(|e| {
                        e.push_str("\n\n");
                        e.push_str(&body);
                    })
                    .or_insert(body);
            }
            None => out.narrative.extend(paragraphs(&b.lines)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINTER: &str = "* WHAT...Heavy snow expected. Total snow accumulations 8 to 14\ninches. Winds gusting as high as 50 mph.\n\n* WHERE...Eastern San Juan Mountains Above 10000 Feet.\n\n* WHEN...From 5 PM this afternoon to 11 AM MST Monday.\n\n* IMPACTS...Travel could be very difficult, especially tonight and\nMonday morning over Wolf Creek Pass and Cumbres Pass. Strong winds\ncould produce areas of blowing and drifting snow.";

    #[test]
    fn star_sections_are_split_and_unwrapped() {
        let s = parse_sections(WINTER);
        assert_eq!(
            s.get("what"),
            Some("Heavy snow expected. Total snow accumulations 8 to 14 inches. Winds gusting as high as 50 mph.")
        );
        assert_eq!(
            s.get("where"),
            Some("Eastern San Juan Mountains Above 10000 Feet.")
        );
        assert_eq!(
            s.get("when"),
            Some("From 5 PM this afternoon to 11 AM MST Monday.")
        );
        assert!(s
            .get("impacts")
            .unwrap()
            .starts_with("Travel could be very difficult, especially tonight and Monday morning"));
        assert!(s.narrative.is_empty());
        assert!(s.get("hazard").is_none());
    }

    #[test]
    fn additional_details_keeps_list_items_on_their_own_lines() {
        let t = "* WHAT...Flooding possible.\n\n* ADDITIONAL DETAILS...\n- A strong weather system approaching the area will bring\nwidespread  showers today.\n- http://www.weather.gov/safety/flood";
        let s = parse_sections(t);
        assert_eq!(
            s.get("additional_details"),
            Some("- A strong weather system approaching the area will bring widespread showers today.\n- http://www.weather.gov/safety/flood")
        );
    }

    #[test]
    fn precautionary_actions_label() {
        let t = "* WHAT...Gales.\n\n* PRECAUTIONARY/PREPAREDNESS ACTIONS...\n\nMariners should avoid\nthe area.";
        let s = parse_sections(t);
        assert_eq!(
            s.get("precautionary_actions"),
            Some("Mariners should avoid the area.")
        );
    }

    /// The three Colorado avalanche advisories NWS Denver had out on 2026-11-01, as the feed gave
    /// them: labels written `*WHAT...` with no space after the star.
    const AVALANCHE_SAN_JUAN: &str = "SABCO\n\nTHE FOLLOWING MESSAGE IS TRANSMITTED AT THE REQUEST OF THE\nCOLORADO AVALANCHE INFORMATION CENTER.\n\n...A SPECIAL AVALANCHE ADVISORY IN EFFECT FROM 4:30 PM SUNDAY\nUNTIL 4:30 PM MONDAY...\n\n*WHAT...Avalanche conditions are dangerous above treeline.\n*WHERE...San Juan Mountains\n*WHEN... From 4:30 PM Sunday until 4:30 PM Monday.\n*IMPACTS... Backcountry travelers can easily trigger dangerous\navalanches large enough to bury a human.\n*PRECAUTION/PREPAREDNESS ACTIONS... A potent winter storm increases\navalanche danger Sunday into Monday. Rain, snow, and wind will cause\nnatural avalanches above treeline. Check the forecast at\nwww.colorado.gov/avalanche.\n\n\nBeyer";
    const AVALANCHE_WEST_ELK: &str = "SABCO\n\nTHE FOLLOWING MESSAGE IS TRANSMITTED AT THE REQUEST OF THE\nCOLORADO AVALANCHE INFORMATION CENTER.\n\n...A SPECIAL AVALANCHE ADVISORY IN EFFECT FROM 4:30 PM SATURDAY\nUNTIL 4:30 PM MONDAY...\n\n*WHAT...Avalanche conditions are dangerous above treeline.\n*WHERE...West Elk Mountains, Elk Mountains, San Juan Mountains, and\nLa Plata Mountains\n*WHEN...From 4:30 PM Saturday until 4:30 PM Monday.\n*IMPACTS...Backcountry travelers can easily trigger dangerous\navalanches large enough to bury a human.\n\n*PRECAUTION/PREPAREDNESS ACTIONS...A potent winter storm increases\navalanche danger Sunday into Monday. Rain, snow, and wind will cause\nnatural avalanches above treeline. Check the forecast at\nwww.colorado.gov/avalanche.\n\nBeyer & Pritchett";

    #[test]
    fn a_star_with_no_space_is_a_section_label_too() {
        let s = parse_sections(AVALANCHE_SAN_JUAN);
        assert_eq!(
            s.get("what"),
            Some("Avalanche conditions are dangerous above treeline.")
        );
        assert_eq!(s.get("where"), Some("San Juan Mountains"));
        assert_eq!(
            s.get("when"),
            Some("From 4:30 PM Sunday until 4:30 PM Monday.")
        );
        assert_eq!(
            s.get("impacts"),
            Some("Backcountry travelers can easily trigger dangerous avalanches large enough to bury a human.")
        );
        // the last section runs to the end of the text, the signature included
        assert!(s
            .get("precautionary_actions")
            .unwrap()
            .starts_with("A potent winter storm increases avalanche danger Sunday into Monday."));
        // what is left is the product preamble, and the product id line is dropped
        assert_eq!(
            s.narrative,
            vec![
                "THE FOLLOWING MESSAGE IS TRANSMITTED AT THE REQUEST OF THE COLORADO AVALANCHE INFORMATION CENTER.",
                "...A SPECIAL AVALANCHE ADVISORY IN EFFECT FROM 4:30 PM SUNDAY UNTIL 4:30 PM MONDAY...",
            ]
        );

        let s = parse_sections(AVALANCHE_WEST_ELK);
        assert_eq!(
            s.get("where"),
            Some("West Elk Mountains, Elk Mountains, San Juan Mountains, and La Plata Mountains")
        );
        assert_eq!(
            s.get("when"),
            Some("From 4:30 PM Saturday until 4:30 PM Monday.")
        );
        // a blank line between two labelled sections ends the first one
        assert_eq!(
            s.get("impacts"),
            Some("Backcountry travelers can easily trigger dangerous avalanches large enough to bury a human.")
        );
        assert!(s
            .get("precautionary_actions")
            .unwrap()
            .ends_with("Beyer & Pritchett"));
        assert_eq!(s.narrative.len(), 2);
    }

    #[test]
    fn star_labels_parse_the_same_with_or_without_the_space() {
        let spaced = "* WHAT...Heavy snow.\n* WHERE...The pass.\n* WHEN... Monday.\n* IMPACTS...Slow travel\nand closures.";
        let tight = "*WHAT...Heavy snow.\n*WHERE...The pass.\n*WHEN... Monday.\n*IMPACTS...Slow travel\nand closures.";
        let loose = "*  WHAT...Heavy snow.\n*  WHERE...The pass.\n*\tWHEN... Monday.\n*IMPACTS...Slow travel\nand closures.";
        let want = parse_sections(spaced);
        assert_eq!(want.get("what"), Some("Heavy snow."));
        assert_eq!(want.get("when"), Some("Monday."));
        assert_eq!(want.get("impacts"), Some("Slow travel and closures."));
        assert_eq!(parse_sections(tight), want);
        assert_eq!(parse_sections(loose), want);
        // the bare forms (no star) are unchanged
        let bare =
            parse_sections("WHAT...Heavy snow.\n\nHAZARD...60 mph gusts.\n\nSOURCE...Radar.");
        assert_eq!(bare.get("what"), Some("Heavy snow."));
        assert_eq!(bare.get("hazard"), Some("60 mph gusts."));
        assert_eq!(bare.get("source"), Some("Radar."));
        // a star with lower-case text is not a label, with or without the space
        for t in [
            "* Locations impacted include...\nTown.",
            "*Locations impacted include...\nTown.",
        ] {
            let s = parse_sections(t);
            assert!(s.fields.is_empty(), "{t:?}: {:?}", s.fields);
        }
        // and "**" or a label with a stray star inside is no label
        assert!(parse_sections("**WHAT...x").fields.is_empty());
    }

    #[test]
    fn the_normalised_description_keeps_tight_star_labels_on_their_own_lines() {
        let n = normalize_text(AVALANCHE_SAN_JUAN);
        assert!(
            n.contains("*WHAT...Avalanche conditions are dangerous above treeline.\n*WHERE...San Juan Mountains\n*WHEN..."),
            "{n}"
        );
    }

    #[test]
    fn unknown_label_becomes_a_slug() {
        let s = parse_sections("* TIDE TIMES...High at noon.");
        assert_eq!(s.get("tide_times"), Some("High at noon."));
    }

    const SVR: &str = "SVRABQ\n\nThe National Weather Service in Albuquerque has issued a\n\n* Severe Thunderstorm Warning for...\nBernalillo County in central New Mexico...\nSanta Fe County in north central New Mexico...\n\n* Until 500 PM MST.\n\n* At 410 PM MST, severe thunderstorms were located along a line from\nnear Cochiti Pueblo to over Meadow Lake, moving northeast at 55\nmph.\n\nHAZARD...60 mph wind gusts.\n\nSOURCE...Radar indicated.\n\nIMPACT...Expect damage to roofs, siding, and trees.\n\n* Locations impacted include...\nAlbuquerque, eastern Rio Rancho, Santa Fe.\n\nThis includes the following highways...\nInterstate 40 between Mile Markers 157 and 213.";

    #[test]
    fn warning_layout_hazard_source_impact() {
        let s = parse_sections(SVR);
        assert_eq!(s.get("hazard"), Some("60 mph wind gusts."));
        assert_eq!(s.get("source"), Some("Radar indicated."));
        assert_eq!(
            s.get("impact"),
            Some("Expect damage to roofs, siding, and trees.")
        );
        // The product id line is dropped; the rest is narrative, in order.
        assert_eq!(
            s.narrative[0],
            "The National Weather Service in Albuquerque has issued a"
        );
        assert!(s
            .narrative
            .iter()
            .any(|p| p.starts_with("Severe Thunderstorm Warning for...")));
        assert!(s.narrative.iter().any(|p| p == "Until 500 PM MST."));
        assert!(s
            .narrative
            .iter()
            .any(|p| p.starts_with("At 410 PM MST") && p.ends_with("mph.")));
        assert!(!s.narrative.iter().any(|p| p == "SVRABQ"));
        // "* Locations impacted include..." is lower-case: narrative, not a section.
        assert!(s
            .narrative
            .iter()
            .any(|p| p.starts_with("Locations impacted include...")));
        assert!(s
            .narrative
            .iter()
            .any(|p| p.starts_with("This includes the following highways")));
    }

    #[test]
    fn statement_without_stars() {
        let t = "At 627 PM EST, National Weather Service meteorologists were tracking\na strong thunderstorm over Riviera Beach, moving north at 5 mph.\n\nHAZARD...Wind gusts of 45 to 50 mph.\n\nSOURCE...Radar indicated.\n\nIMPACT...Gusty winds could knock down tree limbs and blow around\nunsecured objects.\n\nLocations impacted include...\nWest Palm Beach, Jupiter.";
        let s = parse_sections(t);
        assert_eq!(s.get("hazard"), Some("Wind gusts of 45 to 50 mph."));
        assert_eq!(
            s.get("impact"),
            Some("Gusty winds could knock down tree limbs and blow around unsecured objects.")
        );
        assert_eq!(s.narrative.len(), 2);
        assert!(s.narrative[0].starts_with(
            "At 627 PM EST, National Weather Service meteorologists were tracking a strong"
        ));
        assert!(s.narrative[1].starts_with("Locations impacted include... West Palm Beach"));
    }

    #[test]
    fn plain_text_is_all_narrative() {
        let s = parse_sections("A Test Message\nspanning two lines.\n\nSecond paragraph.");
        assert!(s.fields.is_empty());
        assert_eq!(
            s.narrative,
            vec!["A Test Message spanning two lines.", "Second paragraph."]
        );
    }

    #[test]
    fn terminator_stops_parsing() {
        let s = parse_sections("* WHAT...Wind.\n\n&&\n\n* WHERE...ignored");
        assert_eq!(s.get("what"), Some("Wind."));
        assert!(s.get("where").is_none());
    }

    #[test]
    fn empty_input() {
        assert!(parse_sections("").is_empty());
        assert!(parse_sections("   \n\n").is_empty());
    }

    #[test]
    fn json_has_only_present_keys() {
        let v = parse_sections(WINTER).to_json();
        assert!(v.get("what").is_some());
        assert!(v.get("hazard").is_none());
        assert!(v.get("narrative").is_none());
    }

    #[test]
    fn normalize_collapses_wraps() {
        assert_eq!(normalize_text("a\nb  c\n\nd"), "a b c\n\nd");
        assert_eq!(normalize_text(""), "");
    }
}
