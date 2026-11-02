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
//! The Alaska coastal-water alerts (Gale Warning, Small Craft Advisory) carry the zone
//! forecast instead, one line per period, each starting with a dot and the period in capitals:
//!
//! ```text
//! Coastal Waters Forecast for the Northern Gulf of Alaska Coast
//! up to 100 nm out including Kodiak Island and Cook Inlet.
//!
//! Wind forecasts reflect the predominant speed and direction
//! expected. ...
//!
//! .TODAY...E wind 25 kt. Seas 7 ft. Isolated thunderstorms.
//! .TONIGHT...SE wind 25 kt becoming E 35 kt after midnight. Seas 9
//! ft. Rain.
//! .TUE NIGHT...SE wind 50 kt. Seas 20 ft.
//! .WED THROUGH THU...NE wind 35 kt. Seas 15 ft.
//! ```
//!
//! Each period becomes an entry of `periods` (`{"name": "TUE NIGHT", "text": "SE wind 50 kt. ..."}`),
//! in order, and [`derived_what`] picks the ones that the headline's day words name for a card.
//!
//! Recognised labels become keys (`what`, `where`, `when`, `impacts`,
//! `additional_details`, `precautionary_actions`, `hazard`, `source`, `impact`;
//! any other all-caps `* LABEL...` becomes a slug of itself). Text outside a
//! labelled section is returned, in order, as `narrative` paragraphs. Hard
//! wraps inside a paragraph are joined with a space; `- ` list items and period
//! lines keep their own line.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// One period of a zone forecast: `.TUE NIGHT...SE wind 50 kt. Seas 20 ft.`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    /// As NWS wrote it: `TODAY`, `TONIGHT`, `TUE NIGHT`, `WED THROUGH THU`, `REST OF TODAY`.
    pub name: String,
    /// The forecast, wraps joined: `SE wind 50 kt. Seas 20 ft.`
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    /// Labelled sections by key (see module docs).
    pub fields: BTreeMap<String, String>,
    /// The periods of a zone forecast, in order.
    pub periods: Vec<Period>,
    /// Unlabelled paragraphs, in order.
    pub narrative: Vec<String>,
}

impl Sections {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.periods.is_empty() && self.narrative.is_empty()
    }

    /// `{"what": "...", ..., "periods": [{"name": "...", "text": "..."}], "narrative": ["...", ...]}`;
    /// only present keys.
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.fields {
            m.insert(k.clone(), json!(v));
        }
        if !self.periods.is_empty() {
            m.insert(
                "periods".into(),
                Value::Array(
                    self.periods
                        .iter()
                        .map(|p| json!({"name": p.name, "text": p.text}))
                        .collect(),
                ),
            );
        }
        if !self.narrative.is_empty() {
            m.insert("narrative".into(), json!(self.narrative));
        }
        Value::Object(m)
    }
}

/// Collapse hard-wrapped text into paragraphs: blank lines separate paragraphs
/// (joined with `\n\n`), wrapped lines inside one are joined with a space, and a
/// line starting with `- ` / `* ` / a bullet / `.PERIOD...` starts its own line.
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
        || period_header(t.trim_end()).is_some()
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
            let slug = slug.trim_matches('_');
            // (`periods` and `narrative` are the keys of the parts that are not labelled)
            match slug {
                "periods" | "narrative" => format!("{slug}_section"),
                _ => slug.to_string(),
            }
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

/// `.PERIOD...rest`, a period of a zone forecast: a dot, 2 to 30 capital letters and spaces
/// (`TODAY`, `TUE NIGHT`, `WED THROUGH THU`, `REST OF TODAY`), three dots. (`...A SPECIAL
/// AVALANCHE ADVISORY...`, a headline, has dots where the label would start.)
fn period_header(s: &str) -> Option<(&str, &str)> {
    let (label, rest) = s.trim().strip_prefix('.')?.split_once("...")?;
    let ok = (2..=30).contains(&label.len())
        && label.starts_with(|c: char| c.is_ascii_uppercase())
        && !label.ends_with(' ')
        && label.chars().all(|c| c.is_ascii_uppercase() || c == ' ');
    ok.then_some((label, rest))
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
    /// A period of a zone forecast (`.TUE...`): ends at the next header or a blank line.
    period: Option<String>,
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
                period: None,
                star,
                lines: vec![rest],
            });
        } else if let Some((name, rest)) = period_header(trimmed) {
            flush!();
            cur = Some(Block {
                key: None,
                period: Some(name.to_string()),
                star: false,
                lines: vec![rest],
            });
        } else if let Some(body) = trimmed.strip_prefix("* ") {
            // An unlabelled bullet closes any open section and starts narrative.
            flush!();
            cur = Some(Block {
                key: None,
                period: None,
                star: true,
                lines: vec![body],
            });
        } else if trimmed.is_empty() {
            let bare_open = cur
                .as_ref()
                .is_some_and(|b| b.period.is_some() || (b.key.is_some() && !b.star));
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
                        period: None,
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
        if let Some(name) = b.period {
            let text = normalize_lines(&b.lines);
            if !text.is_empty() {
                out.periods.push(Period { name, text });
            }
            continue;
        }
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

// -----------------------------------------------------------------------------
// The summary of an alert with no WHAT / HAZARD
// -----------------------------------------------------------------------------

/// Most characters of the sentences [`derived_what`] takes from free text.
pub const FREE_TEXT_MAX: usize = 200;

/// What a card says about an alert whose text has no `WHAT...` or `HAZARD...`, from the stored
/// `sections` (see [`Sections::to_json`]) and the alert's headline. `None` when NWS gave one of
/// the two, or there is nothing to say.
///
/// * Zone forecast periods (Alaska coastal waters): the periods that the headline's day words
///   name, each labelled with its period (`Tonight: SE wind 25 kt ... Tue: E wind 45 kt ...`).
///   `GALE WARNING TONIGHT AND TUESDAY` names TONIGHT and TUE, `SMALL CRAFT ADVISORY THROUGH
///   MONDAY NIGHT` names MON NIGHT, `TODAY` and `REST OF TODAY` stand for themselves, a period
///   such as `WED THROUGH THU` or `TUE AND TUE NIGHT` is named by any of the days it spans.
///   With no day words, or none that a period carries, the first period.
/// * Free text with neither labels nor periods (a Special Weather Statement): its first sentence,
///   or two when both fit in [`FREE_TEXT_MAX`] characters. A paragraph that is not a sentence
///   (a title such as `Grand River near Chillicothe`) is passed over.
///
/// A derived value, not a stored one: the rules can change without rewriting any alert.
pub fn derived_what(sections: &Value, headline: Option<&str>) -> Option<String> {
    let has = |k: &str| {
        sections
            .get(k)
            .and_then(Value::as_str)
            .is_some_and(|t| !t.trim().is_empty())
    };
    if has("what") || has("hazard") {
        return None;
    }
    let periods: Vec<(&str, &str)> = sections
        .get("periods")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| Some((p.get("name")?.as_str()?, p.get("text")?.as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    if !periods.is_empty() {
        return Some(period_summary(&periods, headline));
    }
    let labelled = sections.as_object().is_some_and(|m| {
        m.iter().any(|(k, v)| {
            k != "narrative" && k != "periods" && v.as_str().is_some_and(|t| !t.trim().is_empty())
        })
    });
    if labelled {
        return None;
    }
    let narrative: Vec<&str> = sections
        .get("narrative")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    first_sentences(&narrative, FREE_TEXT_MAX)
}

/// A point of a forecast's timeline: the day (`None` is the day the forecast was written) and
/// whether it is the night.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    weekday: Option<u8>,
    night: bool,
}

/// 0 for MON to 6 for SUN, from `MON` or `MONDAY` (capitals).
fn weekday(word: &str) -> Option<u8> {
    Some(match word {
        "MON" | "MONDAY" => 0,
        "TUE" | "TUES" | "TUESDAY" => 1,
        "WED" | "WEDNESDAY" => 2,
        "THU" | "THUR" | "THURS" | "THURSDAY" => 3,
        "FRI" | "FRIDAY" => 4,
        "SAT" | "SATURDAY" => 5,
        "SUN" | "SUNDAY" => 6,
        _ => return None,
    })
}

/// The slots a period's name covers: `TODAY`, `REST OF TODAY`, `TONIGHT`, `TUE`, `TUE NIGHT`,
/// `TUE AND TUE NIGHT`, and `WED THROUGH THU` (every day and night from the one to the other).
fn period_slots(name: &str) -> Vec<Slot> {
    let mut out = Vec::new();
    for part in name.split(" AND ") {
        let words: Vec<&str> = part.split_whitespace().collect();
        if let Some(i) = words.iter().position(|w| *w == "THROUGH") {
            if let (Some(from), Some(to)) = (
                words[..i].first().copied().and_then(weekday),
                words.get(i + 1).copied().and_then(weekday),
            ) {
                let mut d = from;
                loop {
                    for night in [false, true] {
                        out.push(Slot {
                            weekday: Some(d),
                            night,
                        });
                    }
                    if d == to {
                        break;
                    }
                    d = (d + 1) % 7;
                }
            }
            continue;
        }
        let slot = match words.as_slice() {
            ["TODAY"] | ["REST", "OF", "TODAY"] => Some((None, false)),
            ["TONIGHT"] => Some((None, true)),
            [d] => weekday(d).map(|d| (Some(d), false)),
            [d, "NIGHT"] => weekday(d).map(|d| (Some(d), true)),
            _ => None,
        };
        out.extend(slot.map(|(weekday, night)| Slot { weekday, night }));
    }
    out
}

/// The slots a headline's day words name: `TODAY`, `TONIGHT` (also `LATE TONIGHT`), `THIS
/// MORNING` / `AFTERNOON` (today) and `THIS EVENING` (tonight), a weekday (`TUESDAY`, `TUESDAY
/// MORNING`), and a weekday's night (`TUESDAY NIGHT`, `TUESDAY EVENING`). The rest, `THROUGH`
/// and `AND` among it, only joins them.
fn headline_slots(headline: &str) -> Vec<Slot> {
    let upper = headline.to_ascii_uppercase();
    let words: Vec<&str> = upper
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let next = words.get(i + 1).copied();
        let slot = match *w {
            "TODAY" => Some((None, false)),
            "TONIGHT" => Some((None, true)),
            "THIS" => match next {
                Some("MORNING" | "AFTERNOON") => Some((None, false)),
                Some("EVENING") => Some((None, true)),
                _ => None,
            },
            w => weekday(w).map(|d| (Some(d), matches!(next, Some("NIGHT" | "EVENING")))),
        };
        out.extend(slot.map(|(weekday, night)| Slot { weekday, night }));
    }
    out
}

/// `TUE NIGHT` as `Tue night`, `REST OF TODAY` as `Rest of today`.
fn period_label(name: &str) -> String {
    let words: Vec<String> = name
        .split_whitespace()
        .map(|w| {
            let lower = w.to_ascii_lowercase();
            if weekday(w).is_some() {
                capitalize(&lower)
            } else {
                lower
            }
        })
        .collect();
    capitalize(&words.join(" "))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn period_summary(periods: &[(&str, &str)], headline: Option<&str>) -> String {
    let want = headline.map(headline_slots).unwrap_or_default();
    let mut picked: Vec<&(&str, &str)> = periods
        .iter()
        .filter(|(name, _)| period_slots(name).iter().any(|s| want.contains(s)))
        .collect();
    if picked.is_empty() {
        picked.push(&periods[0]);
    }
    picked
        .iter()
        .map(|(name, text)| format!("{}: {}", period_label(name), text))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words that end in a full stop without ending the sentence.
const ABBREVIATIONS: [&str; 10] = [
    "mt", "mts", "st", "dr", "mr", "mrs", "ms", "vs", "approx", "hwy",
];

/// The sentences of `text`: split after `.`, `!` or `?` and a space when a capital, a digit or
/// the end follows, but not after an abbreviation (`Mt.`, `p.m.`) or an ellipsis (`...`).
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let after = i + 1;
        if bytes.get(after) == Some(&b'.') || (i > 0 && bytes[i - 1] == b'.') {
            continue; // part of an ellipsis
        }
        let rest = &text[after..];
        let boundary = rest.is_empty()
            || (rest.starts_with(char::is_whitespace)
                && rest
                    .trim_start()
                    .chars()
                    .next()
                    .is_none_or(|n| !n.is_lowercase()));
        if !boundary {
            continue;
        }
        let word = text[start..i]
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if c == '.' && (word.contains('.') || ABBREVIATIONS.contains(&word.as_str())) {
            continue;
        }
        out.push(text[start..after].trim());
        start = after;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// The first sentence of `paragraphs`, and the second when both fit in `max` characters; a first
/// sentence over `max` is cut at a word. Paragraphs that do not end like a sentence are passed
/// over, unless none does.
fn first_sentences(paragraphs: &[&str], max: usize) -> Option<String> {
    let ends_sentence = |p: &&str| p.ends_with(['.', '!', '?']);
    let prose: Vec<&str> = if paragraphs.iter().any(ends_sentence) {
        paragraphs.iter().copied().filter(ends_sentence).collect()
    } else {
        paragraphs.to_vec()
    };
    let all: Vec<&str> = prose.iter().flat_map(|p| sentences(p)).collect();
    let first = *all.first()?;
    if first.chars().count() > max {
        let cut: String = first.chars().take(max).collect();
        let cut = match cut.rfind(char::is_whitespace) {
            Some(i) if i > 0 => &cut[..i],
            _ => &cut,
        };
        return Some(format!("{}…", cut.trim_end_matches([',', ';', ':', ' '])));
    }
    match all.get(1) {
        Some(second) if first.chars().count() + 1 + second.chars().count() <= max => {
            Some(format!("{first} {second}"))
        }
        _ => Some(first.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NWS Anchorage, Gale Warning, headline `GALE WARNING TONIGHT AND TUESDAY`, as the feed gave it on 2026-11-02.
    const ANCHORAGE_GALE: &str = "Coastal Waters Forecast for the Northern Gulf of Alaska Coast\nup to 100 nm out including Kodiak Island and Cook Inlet.\n\nWind forecasts reflect the predominant speed and direction\nexpected. Sea forecasts represent an average of the highest\none-third of the combined wind wave and swell height.\n\n.TODAY...E wind 25 kt. Seas 7 ft. Isolated thunderstorms.\n.TONIGHT...SE wind 25 kt becoming E 35 kt after midnight. Seas 9\nft. Rain.\n.TUE...E wind 45 kt. Seas 18 ft. Rain.\n.TUE NIGHT...SE wind 50 kt. Seas 20 ft.\n.WED...E wind 25 kt. Seas 11 ft.\n.THU...NE wind 35 kt. Seas 15 ft.\n.FRI...E wind 30 kt. Seas 15 ft.";

    /// NWS Anchorage, Small Craft Advisory, `SMALL CRAFT ADVISORY MONDAY NIGHT`, as the feed gave it on 2026-11-02.
    const ANCHORAGE_SCA: &str = "Coastal Waters Forecast for the Northern Gulf of Alaska Coast\nup to 100 nm out including Kodiak Island and Cook Inlet.\n\nWind forecasts reflect the predominant speed and direction\nexpected. Sea forecasts represent an average of the highest\none-third of the combined wind wave and swell height.\n\n.TONIGHT...NE wind 15 kt. Seas 3 ft. Widespread rain showers.\nIsolated thunderstorms.\n.MON...NE wind 20 kt. Seas 4 ft. Widespread rain showers. Isolated\nthunderstorms.\n.MON NIGHT...E wind 25 kt. Seas 4 ft building to 6 ft after\nmidnight. Rain.\n.TUE AND TUE NIGHT...NE wind 25 kt. Seas 5 ft.\n.WED...NE wind 20 kt. Seas 4 ft.\n.THU...N wind 20 kt. Seas 4 ft.\n.FRI...NE wind 15 kt. Seas 3 ft.";

    /// NWS Fairbanks, Small Craft Advisory, `SMALL CRAFT ADVISORY IN EFFECT THROUGH LATE TONIGHT`, as the feed gave it on 2026-11-02.
    const FAIRBANKS_SCA: &str = "Northwestern Alaska Coastal Waters out 100 NM\n\nWind forecasts reflect the predominant speed and direction\nexpected. Sea forecasts represent the average of the highest one-\nthird of the combined wind-wave and swell height.\n\n.TODAY...E winds 25 kt. Seas 7 ft.\n.TONIGHT...E winds 30 kt. Seas 9 ft.\n.TUE...E winds 25 kt. Seas 9 ft.\n.TUE NIGHT...E winds 20 kt. Seas 5 ft.\n.WED...E winds 20 kt. Seas 5 ft.\n.WED NIGHT...E winds 15 kt. Seas 5 ft.\n.THU...E winds 15 kt. Seas 4 ft.\n.FRI...N winds 20 kt. Seas 4 ft.";

    /// NWS Juneau, Small Craft Advisory, no NWSheadline (its formal headline names no day), as the feed gave it on 2026-11-02.
    const JUNEAU_SCA: &str = "Southeast Alaska Coastal Waters from Dixon Entrance to\nCape Suckling out 100 NM\n\nWind forecasts reflect the predominant speed and direction\nexpected. Seas forecasts represent the average of the highest\none-third of the combined windwave and swell height.\n\nUPDATED\n\n...SMALL CRAFT ADVISORY THROUGH MONDAY...\n\n.REST OF TODAY...SW wind 25 kt. Seas 16 ft.\n.TONIGHT...SE wind 10 kt. Seas 11 ft. SW swell.\n.MON...S wind 10 kt. Seas 9 ft. SW swell.\n.MON NIGHT...SE wind 20 kt. Seas 8 ft. SW swell.\n.TUE...SE gale to 35 kt. Seas 14 ft.\n.WED...SE gale to 35 kt. Seas 17 ft.\n.THU...SE gale to 45 kt. Seas 24 ft.";

    /// NWS Anchorage, offshore Gale Warning, `GALE WARNING TODAY AND MONDAY`, as the feed gave it on 2026-11-02.
    const BERING_GALE: &str = "Offshore Waters Forecast for the Bering Sea\n\nWind forecasts reflect the predominant speed and direction\nexpected. Sea forecasts represent an average of the highest\none-third of the combined wind wave and swell height.\n\n.TODAY...S wind 25 to 40 kt. Seas 12 to 18 ft. Rain.\n.TONIGHT...S wind 15 to 30 kt. Seas 13 to 20 ft.\n.MON...N wind 10 to 25 kt becoming NW 25 to 40 kt in the afternoon.\nSeas 12 to 17 ft.\n.MON NIGHT...NW wind 25 to 40 kt. Seas 13 to 21 ft.\n.TUE THROUGH THU...NW wind 20 to 35 kt. Seas 12 to 20 ft.";

    /// NWS Pendleton, Special Weather Statement: free text, no NWSheadline, as the feed gave it on 2026-11-02.
    const PENDLETON_SPS: &str = "Patchy dense fog has been observed around the Tri Cities this morning.\nWithin fog, visibilities as low as one-quarter mile are expected\nto persist through mid morning.  Use extra caution while traveling as\nvisibility can change quickly. If fog is encountered, use low- beam headlights\nand keep plenty of space between you and other vehicles.";

    /// NWS Kansas City, Hydrologic Outlook: free text that starts with the river's name and holds a table, as the feed gave it on 2026-11-02.
    const HYDRO_OUTLOOK: &str = "ESFEAX\n\nGrand River near Chillicothe\n\nThis hydrologic outlook is based on the forecast rainfall for the\nnext 48 hours and estimated runoff from earlier rainfall. Crests may\nvary if actual rainfall or runoff is greater or less than\nanticipated.\n\nLocation: Grand River near Chillicothe\nFlood stage: 24.0 feet\nLatest stage: 5.8 feet at 8 AM Monday\nMaximum Forecast Stage: 27.7 feet at 12 AM Wednesday Nov 04\n\n\nFld   Obs             Forecasts\nLocation        Stg   Stg   Day/Time  Mon   Tue   Wed\n12pm  12pm  12pm\nGrand River\nChillicothe     24.0  5.8   Mon 8am   5.7   19.3  18.8\n\n\nLater statements, possibly warnings, may be issued as additional\ninformation becomes available.";

    /// NWS Grand Forks, Special Weather Statement: free text, as the feed gave it on 2026-11-02.
    const GRAND_FORKS_SPS: &str = "Localized areas of fog reducing visibility to around one-quarter\nmile have developed across portions of northeast ND, especially\nnear the Pembina Escarpment. Travelers should be prepared for\nrapidly deteriorating visibility, and to use extra caution at\nintersections, railroad crossings, and school bus stops. The fog\nis expected to start improving early this afternoon.";

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

    fn period_names(s: &Sections) -> Vec<&str> {
        s.periods.iter().map(|p| p.name.as_str()).collect()
    }

    fn what(description: &str, headline: &str) -> Option<String> {
        derived_what(&parse_sections(description).to_json(), Some(headline))
    }

    #[test]
    fn alaska_periods_are_split_in_order_and_unwrapped() {
        let s = parse_sections(ANCHORAGE_GALE);
        assert_eq!(
            period_names(&s),
            ["TODAY", "TONIGHT", "TUE", "TUE NIGHT", "WED", "THU", "FRI"]
        );
        assert_eq!(
            s.periods[0],
            Period {
                name: "TODAY".into(),
                text: "E wind 25 kt. Seas 7 ft. Isolated thunderstorms.".into()
            }
        );
        // a period that NWS wrapped over two lines
        assert_eq!(
            s.periods[1].text,
            "SE wind 25 kt becoming E 35 kt after midnight. Seas 9 ft. Rain."
        );
        // what is left is the zone name and the boilerplate, with no labelled section
        assert!(s.fields.is_empty());
        assert_eq!(
            s.narrative,
            vec![
                "Coastal Waters Forecast for the Northern Gulf of Alaska Coast up to 100 nm out including Kodiak Island and Cook Inlet.",
                "Wind forecasts reflect the predominant speed and direction expected. Sea forecasts represent an average of the highest one-third of the combined wind wave and swell height.",
            ]
        );
        let j = s.to_json();
        assert_eq!(j["periods"][3]["name"], "TUE NIGHT");
        assert_eq!(j["periods"][3]["text"], "SE wind 50 kt. Seas 20 ft.");
        assert_eq!(j["periods"].as_array().unwrap().len(), 7);
        assert_eq!(j["narrative"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn range_and_compound_period_names_are_periods() {
        let s = parse_sections(ANCHORAGE_SCA);
        assert_eq!(
            period_names(&s),
            [
                "TONIGHT",
                "MON",
                "MON NIGHT",
                "TUE AND TUE NIGHT",
                "WED",
                "THU",
                "FRI"
            ]
        );
        let s = parse_sections(BERING_GALE);
        assert_eq!(
            period_names(&s),
            ["TODAY", "TONIGHT", "MON", "MON NIGHT", "TUE THROUGH THU"]
        );
        assert_eq!(
            s.periods[2].text,
            "N wind 10 to 25 kt becoming NW 25 to 40 kt in the afternoon. Seas 12 to 17 ft."
        );
        let s = parse_sections(JUNEAU_SCA);
        assert_eq!(s.periods[0].name, "REST OF TODAY");
        // the headline line and the update mark are narrative, not periods
        assert!(s.narrative.iter().any(|p| p == "UPDATED"));
        assert!(s
            .narrative
            .iter()
            .any(|p| p == "...SMALL CRAFT ADVISORY THROUGH MONDAY..."));
    }

    #[test]
    fn period_headers_are_dot_capitals_dots() {
        for (line, name, rest) in [
            (".TODAY...E wind 25 kt.", "TODAY", "E wind 25 kt."),
            (".TUE NIGHT...SE wind 50 kt.", "TUE NIGHT", "SE wind 50 kt."),
            (".WED THROUGH THU...NE wind.", "WED THROUGH THU", "NE wind."),
            (".REST OF TODAY...", "REST OF TODAY", ""),
            ("  .TONIGHT...Rain.  ", "TONIGHT", "Rain."),
        ] {
            assert_eq!(period_header(line), Some((name, rest)), "{line:?}");
        }
        for line in [
            "...A SPECIAL AVALANCHE ADVISORY IN EFFECT FROM 4:30 PM SUNDAY UNTIL 4:30 PM MONDAY...",
            "...SMALL CRAFT ADVISORY THROUGH MONDAY...",
            "TODAY...E wind",
            ".Today...E wind",
            ".TODAY",
            ". TODAY...x",
            ".A...x",
            ".TODAY ...x",
            "*WHAT...Snow",
            ".TUE 5...x",
            ".ABCDEFGHIJKLMNOPQRSTUVWXYZ ABCDE...x",
        ] {
            assert_eq!(period_header(line), None, "{line:?}");
        }
    }

    #[test]
    fn the_description_keeps_one_period_per_line() {
        let n = normalize_text(ANCHORAGE_GALE);
        let want = "Wind forecasts reflect the predominant speed and direction expected. Sea forecasts represent an average of the highest one-third of the combined wind wave and swell height.\n\n.TODAY...E wind 25 kt. Seas 7 ft. Isolated thunderstorms.\n.TONIGHT...SE wind 25 kt becoming E 35 kt after midnight. Seas 9 ft. Rain.\n.TUE...E wind 45 kt. Seas 18 ft. Rain.\n.TUE NIGHT...SE wind 50 kt. Seas 20 ft.\n.WED...E wind 25 kt. Seas 11 ft.\n.THU...NE wind 35 kt. Seas 15 ft.\n.FRI...E wind 30 kt. Seas 15 ft.";
        assert!(n.ends_with(want), "{n}");
        // the hard wraps inside a period are still joined, and the headline-style line is not split
        let n = normalize_text(JUNEAU_SCA);
        assert!(n.contains("\n\n...SMALL CRAFT ADVISORY THROUGH MONDAY...\n\n.REST OF TODAY...SW wind 25 kt. Seas 16 ft.\n.TONIGHT..."));
    }

    #[test]
    fn periods_do_not_disturb_labelled_sections() {
        // a period straight after a labelled section ends it, and a blank line ends the period
        let t = "* WHAT...Gales.\n.TONIGHT...SE wind 30 kt.\n.TUE...E wind 45 kt.\n\nCloser.";
        let s = parse_sections(t);
        assert_eq!(s.get("what"), Some("Gales."));
        assert_eq!(period_names(&s), ["TONIGHT", "TUE"]);
        assert_eq!(s.periods[1].text, "E wind 45 kt.");
        assert_eq!(s.narrative, vec!["Closer."]);
        // `periods` and `narrative` stay the keys of the parts that are not labelled
        let s = parse_sections("* PERIODS...x\n* NARRATIVE...y\n.TODAY...Rain.");
        let j = s.to_json();
        assert_eq!(j["periods_section"], "x");
        assert_eq!(j["narrative_section"], "y");
        assert_eq!(j["periods"][0]["name"], "TODAY");
        // none of it without periods
        assert!(parse_sections(WINTER).to_json().get("periods").is_none());
        assert!(!parse_sections(ANCHORAGE_GALE).is_empty());
    }

    #[test]
    fn the_summary_of_a_forecast_is_the_period_the_headline_names() {
        // GALE WARNING TUESDAY names TUE, not TUE NIGHT; TONIGHT AND TUESDAY names two
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING TUESDAY").unwrap(),
            "Tue: E wind 45 kt. Seas 18 ft. Rain."
        );
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING TONIGHT AND TUESDAY").unwrap(),
            "Tonight: SE wind 25 kt becoming E 35 kt after midnight. Seas 9 ft. Rain. Tue: E wind 45 kt. Seas 18 ft. Rain."
        );
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING TUESDAY NIGHT").unwrap(),
            "Tue night: SE wind 50 kt. Seas 20 ft."
        );
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING TODAY").unwrap(),
            "Today: E wind 25 kt. Seas 7 ft. Isolated thunderstorms."
        );
        assert_eq!(
            what(ANCHORAGE_GALE, "STORM WARNING LATE TONIGHT").unwrap(),
            "Tonight: SE wind 25 kt becoming E 35 kt after midnight. Seas 9 ft. Rain."
        );
        // "through" and "and" only join the days; "morning" is the day, "evening" the night
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING THROUGH WEDNESDAY MORNING").unwrap(),
            "Wed: E wind 25 kt. Seas 11 ft."
        );
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING THIS EVENING THROUGH TUESDAY EVENING").unwrap(),
            "Tonight: SE wind 25 kt becoming E 35 kt after midnight. Seas 9 ft. Rain. Tue night: SE wind 50 kt. Seas 20 ft."
        );
        assert_eq!(
            what(ANCHORAGE_GALE, "GALE WARNING THIS AFTERNOON").unwrap(),
            "Today: E wind 25 kt. Seas 7 ft. Isolated thunderstorms."
        );
        assert_eq!(
            what(
                FAIRBANKS_SCA,
                "SMALL CRAFT ADVISORY IN EFFECT THROUGH LATE TONIGHT"
            )
            .unwrap(),
            "Tonight: E winds 30 kt. Seas 9 ft."
        );
        // case does not matter
        assert_eq!(
            what(ANCHORAGE_GALE, "Gale Warning Tuesday").unwrap(),
            "Tue: E wind 45 kt. Seas 18 ft. Rain."
        );
    }

    #[test]
    fn a_period_that_spans_days_is_named_by_any_of_them() {
        // TUE AND TUE NIGHT
        let tue = "Tue and Tue night: NE wind 25 kt. Seas 5 ft.";
        assert_eq!(
            what(ANCHORAGE_SCA, "SMALL CRAFT ADVISORY TUESDAY").unwrap(),
            tue
        );
        assert_eq!(
            what(ANCHORAGE_SCA, "SMALL CRAFT ADVISORY TUESDAY NIGHT").unwrap(),
            tue
        );
        assert_eq!(
            what(ANCHORAGE_SCA, "SMALL CRAFT ADVISORY MONDAY NIGHT").unwrap(),
            "Mon night: E wind 25 kt. Seas 4 ft building to 6 ft after midnight. Rain."
        );
        // TUE THROUGH THU
        let range = "Tue through Thu: NW wind 20 to 35 kt. Seas 12 to 20 ft.";
        for h in [
            "GALE WARNING WEDNESDAY",
            "GALE WARNING THURSDAY NIGHT",
            "GALE WARNING THROUGH TUESDAY",
        ] {
            assert_eq!(what(BERING_GALE, h).unwrap(), range, "{h}");
        }
        assert_eq!(
            what(BERING_GALE, "GALE WARNING TODAY AND MONDAY").unwrap(),
            "Today: S wind 25 to 40 kt. Seas 12 to 18 ft. Rain. Mon: N wind 10 to 25 kt becoming NW 25 to 40 kt in the afternoon. Seas 12 to 17 ft."
        );
        // a range wraps over the end of the week
        assert!(period_slots("SAT THROUGH MON").contains(&Slot {
            weekday: Some(6),
            night: true
        }));
        assert_eq!(period_slots("SAT THROUGH MON").len(), 6);
    }

    #[test]
    fn a_forecast_with_no_day_words_to_match_says_its_first_period() {
        // the formal headline has no day words
        let formal = "Gale Warning issued November 2 at 3:20AM AKST until November 3 at 5:00PM AKST by NWS Anchorage AK";
        let first = "Today: E wind 25 kt. Seas 7 ft. Isolated thunderstorms.";
        assert_eq!(what(ANCHORAGE_GALE, formal).unwrap(), first);
        assert_eq!(
            derived_what(&parse_sections(ANCHORAGE_GALE).to_json(), None).unwrap(),
            first
        );
        // a day that no period carries (SUNDAY; the forecast starts at TODAY)
        assert_eq!(what(ANCHORAGE_GALE, "GALE WARNING SUNDAY").unwrap(), first);
        // REST OF TODAY is the first period, and the one TODAY names
        let rest = "Rest of today: SW wind 25 kt. Seas 16 ft.";
        assert_eq!(
            what(JUNEAU_SCA, "Small Craft Advisory issued November 1 at 12:12PM AKST until November 2 at 5:00PM AKST by NWS Juneau AK").unwrap(),
            rest
        );
        assert_eq!(
            what(JUNEAU_SCA, "SMALL CRAFT ADVISORY TODAY").unwrap(),
            rest
        );
        assert_eq!(
            what(JUNEAU_SCA, "SMALL CRAFT ADVISORY THROUGH MONDAY").unwrap(),
            "Mon: S wind 10 kt. Seas 9 ft. SW swell."
        );
    }

    #[test]
    fn nothing_is_derived_when_nws_wrote_a_what_or_a_hazard() {
        assert_eq!(
            derived_what(&parse_sections(WINTER).to_json(), Some("x")),
            None
        );
        assert_eq!(derived_what(&parse_sections(SVR).to_json(), None), None);
        // WHAT wins over the periods that follow it
        let t = "* WHAT...Gales.\n\n.TONIGHT...SE wind 30 kt.";
        assert_eq!(
            derived_what(&parse_sections(t).to_json(), Some("TONIGHT")),
            None
        );
        // and so does a labelled section of any other kind: the alert has labels, just not those
        let t = "* WHERE...The pass.\n\nSome text.";
        assert_eq!(derived_what(&parse_sections(t).to_json(), None), None);
        // an empty WHAT counts as none
        let j = json!({"what": "  ", "narrative": ["A sentence."]});
        assert_eq!(derived_what(&j, None).unwrap(), "A sentence.");
        assert_eq!(derived_what(&json!({}), None), None);
        assert_eq!(derived_what(&Value::Null, None), None);
        assert_eq!(derived_what(&parse_sections("  ").to_json(), None), None);
    }

    #[test]
    fn free_text_is_summed_up_by_its_first_sentence_or_two() {
        // both sentences fit in 200 characters
        assert_eq!(
            derived_what(&parse_sections(PENDLETON_SPS).to_json(), None).unwrap(),
            "Patchy dense fog has been observed around the Tri Cities this morning. Within fog, visibilities as low as one-quarter mile are expected to persist through mid morning."
        );
        // they do not: the first one
        let w = derived_what(&parse_sections(GRAND_FORKS_SPS).to_json(), None).unwrap();
        assert_eq!(
            w,
            "Localized areas of fog reducing visibility to around one-quarter mile have developed across portions of northeast ND, especially near the Pembina Escarpment."
        );
        assert!(w.chars().count() <= FREE_TEXT_MAX);
        // a line that is only a name is passed over, and so is the table below it
        let w = derived_what(&parse_sections(HYDRO_OUTLOOK).to_json(), None).unwrap();
        assert_eq!(
            w,
            "This hydrologic outlook is based on the forecast rainfall for the next 48 hours and estimated runoff from earlier rainfall."
        );
        // text with no sentence at all is taken as it is
        let j = parse_sections("Monitoring message\nonly").to_json();
        assert_eq!(derived_what(&j, None).unwrap(), "Monitoring message only");
        let j = parse_sections("Monitoring message only. Please disregard.").to_json();
        assert_eq!(
            derived_what(&j, None).unwrap(),
            "Monitoring message only. Please disregard."
        );
    }

    #[test]
    fn a_long_first_sentence_is_cut_at_a_word() {
        let long = format!("{} end.", "word ".repeat(80));
        let w = derived_what(&parse_sections(&long).to_json(), None).unwrap();
        assert!(w.ends_with("word…"), "{w}");
        assert!(w.chars().count() <= FREE_TEXT_MAX + 1);
        // multi-byte text never panics
        let long = format!("{}.", "é".repeat(300));
        let w = derived_what(&parse_sections(&long).to_json(), None).unwrap();
        assert!(w.ends_with('…') && w.chars().count() <= FREE_TEXT_MAX + 1);
    }

    #[test]
    fn sentences_split_after_a_stop_and_a_capital() {
        assert_eq!(
            sentences("Rain. Seas 4 ft. Mt. Hood is closed. Use the 5 p.m. route! Is it? yes. U.S. 40 is open."),
            vec![
                "Rain.",
                "Seas 4 ft.",
                "Mt. Hood is closed.",
                "Use the 5 p.m. route!",
                "Is it? yes.",
                "U.S. 40 is open.",
            ]
        );
        // an ellipsis does not end one; the end of the text does
        assert_eq!(
            sentences("Locations include... Albuquerque. Santa Fe..."),
            vec!["Locations include... Albuquerque.", "Santa Fe..."]
        );
        assert_eq!(sentences("No stop"), vec!["No stop"]);
        assert!(sentences("").is_empty());
    }

    #[test]
    fn period_labels_are_short_and_readable() {
        for (name, label) in [
            ("TODAY", "Today"),
            ("TONIGHT", "Tonight"),
            ("REST OF TODAY", "Rest of today"),
            ("TUE NIGHT", "Tue night"),
            ("WED THROUGH THU", "Wed through Thu"),
            ("TUE AND TUE NIGHT", "Tue and Tue night"),
        ] {
            assert_eq!(period_label(name), label);
        }
    }
}
