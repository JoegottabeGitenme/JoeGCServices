//! Storm Events runner.
//!
//! Ingests severe-convective reports (Hail, Thunderstorm Wind, Tornado) from the
//! NOAA Storm Events Database. The NCEI bulk CSV directory publishes one
//! gzip-compressed `details` file per year:
//!
//!   StormEvents_details-ftp_v1.0_dYYYY_cYYYYMMDD.csv.gz
//!
//! The trailing `cYYYYMMDD` creation stamp changes when NOAA revises a year, so
//! the exact filename is resolved by scraping the directory listing.
//!
//! ## Cadence
//!
//! - Download: daily (poll_interval_secs).
//! - First run: one-time backfill from `backfill_start_year` to present, gated
//!   by per-year markers in the downloader's SQLite state.
//! - Steady state: re-fetch the current and previous year (to pick up NOAA
//!   revisions).
//! - County aggregate (materialized view): refreshed monthly — triggered when
//!   the calendar month rolls over since the last refresh.

use anyhow::{Context, Result};
use chrono::{Datelike, Utc};
use flate2::read::GzDecoder;
use reqwest::Client;
use serde::Serialize;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::state::DownloadState;

/// Default NCEI bulk CSV directory.
const DEFAULT_BASE_URL: &str = "https://www.ncei.noaa.gov/pub/data/swdi/stormevents/csvfiles/";

/// Configuration for the Storm Events runner.
#[derive(Debug, Clone)]
pub struct StormEventsConfig {
    /// Source identifier.
    pub id: String,
    /// Base URL of the NCEI csvfiles directory (with trailing slash).
    pub base_url: String,
    /// First year to backfill.
    pub backfill_start_year: i32,
    /// Polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Ingester service base URL.
    pub ingester_url: String,
}

/// Runner for fetching and ingesting Storm Events CSVs.
pub struct StormEventsRunner {
    config: StormEventsConfig,
    client: Client,
    state: Arc<DownloadState>,
}

impl StormEventsRunner {
    /// Create a new Storm Events runner.
    pub fn new(config: StormEventsConfig, state: Arc<DownloadState>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300)) // yearly files can be tens of MB
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Self {
            config,
            client,
            state,
        })
    }

    /// Run forever until shutdown.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.config.poll_interval_secs);

        info!(
            source = "storm_events",
            poll_interval_secs = self.config.poll_interval_secs,
            backfill_start_year = self.config.backfill_start_year,
            "Starting Storm Events runner"
        );

        // Initial cycle (includes one-time backfill).
        if let Err(e) = self.run_cycle().await {
            error!(source = "storm_events", error = %e, "Initial Storm Events cycle failed");
        }

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    if let Err(e) = self.run_cycle().await {
                        error!(source = "storm_events", error = %e, "Storm Events cycle failed");
                    }
                }
                _ = shutdown.recv() => {
                    info!(source = "storm_events", "Storm Events runner shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run one ingest cycle.
    async fn run_cycle(&self) -> Result<()> {
        let current_year = Utc::now().year();

        // Resolve directory listing once per cycle to map year -> filename.
        let listing = self.fetch_directory_listing().await?;

        // Determine which years to process this cycle.
        // - Backfill any year not yet marked complete (first run).
        // - Always re-process current + previous year for revisions.
        let mut years: Vec<i32> = Vec::new();
        for year in self.config.backfill_start_year..=current_year {
            let is_recent = year >= current_year - 1;
            let already_done = self.year_marker_present(year).await;
            if is_recent || !already_done {
                years.push(year);
            }
        }

        if years.is_empty() {
            debug!(source = "storm_events", "No years to process this cycle");
        }

        for year in years {
            match self.process_year(year, &listing).await {
                Ok(count) => {
                    info!(source = "storm_events", year, count, "Ingested year");
                    // Mark year complete only when it is not the still-changing
                    // current year (we always re-pull recent years anyway).
                    if year < current_year {
                        self.set_year_marker(year).await;
                    }
                }
                Err(e) => {
                    warn!(source = "storm_events", year, error = %e, "Failed to process year");
                }
            }
        }

        // Monthly county-aggregate refresh.
        self.maybe_refresh_counties().await;

        Ok(())
    }

    /// Process a single year: download, parse, ingest.
    async fn process_year(&self, year: i32, listing: &str) -> Result<usize> {
        let filename = resolve_year_filename(listing, year)
            .with_context(|| format!("No details file found for year {}", year))?;
        let url = format!("{}{}", self.config.base_url, filename);

        debug!(source = "storm_events", year, url = %url, "Downloading year file");
        let bytes = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to GET {}", url))?
            .error_for_status()
            .with_context(|| format!("Bad status for {}", url))?
            .bytes()
            .await
            .context("Failed to read response body")?;

        // Decompress gzip.
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut csv_text = String::new();
        decoder
            .read_to_string(&mut csv_text)
            .context("Failed to gunzip storm events CSV")?;

        let events = parse_storm_events_csv(&csv_text)?;
        if events.is_empty() {
            return Ok(0);
        }

        // Send in batches to keep request bodies reasonable.
        let mut total = 0;
        for chunk in events.chunks(2000) {
            self.send_to_ingester(chunk).await?;
            total += chunk.len();
        }
        Ok(total)
    }

    /// POST a batch of events to the ingester.
    async fn send_to_ingester(&self, events: &[IngestStormEvent]) -> Result<()> {
        let base = self.config.ingester_url.trim_end_matches("/ingest");
        let url = format!("{}/ingest/storm-events", base);

        let payload = StormEventPayload {
            events: events.to_vec(),
        };

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send storm events to ingester")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ingester returned {} for storm events: {}", status, body);
        }
        Ok(())
    }

    /// Refresh the county aggregate if the calendar month has rolled over.
    async fn maybe_refresh_counties(&self) {
        let now = Utc::now();
        let month_key = format!("{}-{:02}", now.year(), now.month());
        let marker = format!("storm_events:counties_refreshed:{}", month_key);

        if self.state_marker_present(&marker).await {
            return;
        }

        let base = self.config.ingester_url.trim_end_matches("/ingest");
        let url = format!("{}/ingest/storm-events/refresh-counties", base);
        match self.client.post(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(source = "storm_events", month = %month_key, "County aggregate refreshed");
                self.set_state_marker(&marker).await;
            }
            Ok(resp) => {
                warn!(source = "storm_events", status = %resp.status(), "County refresh failed");
            }
            Err(e) => {
                warn!(source = "storm_events", error = %e, "County refresh request failed");
            }
        }
    }

    // --- State markers (stored in the downloader SQLite state) ---------------

    fn year_marker(year: i32) -> String {
        format!("storm_events:year_complete:{}", year)
    }

    async fn year_marker_present(&self, year: i32) -> bool {
        let key = Self::year_marker(year);
        self.state_marker_present(&key).await
    }

    async fn set_year_marker(&self, year: i32) {
        let key = Self::year_marker(year);
        self.set_state_marker(&key).await;
    }

    async fn state_marker_present(&self, key: &str) -> bool {
        self.state.has_marker(key).await.unwrap_or(false)
    }

    async fn set_state_marker(&self, key: &str) {
        if let Err(e) = self.state.set_marker(key).await {
            warn!(source = "storm_events", key = %key, error = %e, "Failed to set state marker");
        }
    }

    /// Fetch the NCEI directory listing (HTML) to resolve year filenames.
    async fn fetch_directory_listing(&self) -> Result<String> {
        let text = self
            .client
            .get(&self.config.base_url)
            .send()
            .await
            .context("Failed to fetch storm events directory listing")?
            .error_for_status()
            .context("Bad status for storm events directory listing")?
            .text()
            .await
            .context("Failed to read directory listing body")?;
        Ok(text)
    }
}

/// Resolve the `details` filename for a given year from the directory listing.
///
/// Matches `StormEvents_details-ftp_v1.0_dYYYY_c########.csv.gz` and, if
/// multiple revisions are present, returns the lexicographically greatest
/// (latest creation stamp).
fn resolve_year_filename(listing: &str, year: i32) -> Option<String> {
    let needle = format!("StormEvents_details-ftp_v1.0_d{}_c", year);
    let mut best: Option<String> = None;

    // Scan the HTML for occurrences of the needle and capture the full filename.
    let mut search_from = 0;
    while let Some(idx) = listing[search_from..].find(&needle) {
        let start = search_from + idx;
        // Filename ends at `.csv.gz`.
        if let Some(end_rel) = listing[start..].find(".csv.gz") {
            let end = start + end_rel + ".csv.gz".len();
            let candidate = listing[start..end].to_string();
            best = match best {
                Some(b) if b >= candidate => Some(b),
                _ => Some(candidate),
            };
            search_from = end;
        } else {
            search_from = start + needle.len();
        }
    }

    best
}

/// Parse the Storm Events `details` CSV, filtering to Hail / Thunderstorm Wind /
/// Tornado and normalizing into ingest records.
fn parse_storm_events_csv(csv_text: &str) -> Result<Vec<IngestStormEvent>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(csv_text.as_bytes());

    let headers = reader.headers().context("Missing CSV headers")?.clone();
    let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));

    let i_event_id = col("EVENT_ID");
    let i_episode_id = col("EPISODE_ID");
    let i_event_type = col("EVENT_TYPE");
    let i_begin_yhm = col("BEGIN_DATE_TIME");
    let i_end_yhm = col("END_DATE_TIME");
    let i_state = col("STATE");
    let i_cz_name = col("CZ_NAME");
    let i_cz_fips = col("CZ_FIPS");
    let i_cz_type = col("CZ_TYPE");
    let i_begin_lat = col("BEGIN_LAT");
    let i_begin_lon = col("BEGIN_LON");
    let i_end_lat = col("END_LAT");
    let i_end_lon = col("END_LON");
    let i_mag = col("MAGNITUDE");
    let i_mag_type = col("MAGNITUDE_TYPE");
    let i_tor_scale = col("TOR_F_SCALE");

    let get = |rec: &csv::StringRecord, idx: Option<usize>| -> Option<String> {
        idx.and_then(|i| rec.get(i))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let mut out = Vec::new();
    for result in reader.records() {
        let rec = match result {
            Ok(r) => r,
            Err(e) => {
                debug!("Skipping malformed storm events row: {}", e);
                continue;
            }
        };

        let raw_type = match get(&rec, i_event_type) {
            Some(t) => t,
            None => continue,
        };
        let event_type = match normalize_event_type(&raw_type) {
            Some(t) => t,
            None => continue, // not one of our three types
        };

        let event_id = match get(&rec, i_event_id).and_then(|s| s.parse::<i64>().ok()) {
            Some(id) => id,
            None => continue,
        };

        let begin_time = match get(&rec, i_begin_yhm).and_then(|s| parse_storm_datetime(&s)) {
            Some(t) => t,
            None => continue,
        };
        let end_time = get(&rec, i_end_yhm).and_then(|s| parse_storm_datetime(&s));

        let begin_lat = get(&rec, i_begin_lat).and_then(|s| s.parse::<f64>().ok());
        let begin_lon = get(&rec, i_begin_lon).and_then(|s| s.parse::<f64>().ok());
        let end_lat = get(&rec, i_end_lat).and_then(|s| s.parse::<f64>().ok());
        let end_lon = get(&rec, i_end_lon).and_then(|s| s.parse::<f64>().ok());

        let raw_mag = get(&rec, i_mag).and_then(|s| s.parse::<f64>().ok());
        let mag_type = get(&rec, i_mag_type);
        let tor_scale = get(&rec, i_tor_scale).and_then(|s| parse_ef_scale(&s));

        // Canonical magnitude + unit by type.
        let (magnitude, magnitude_unit) = match event_type.as_str() {
            "hail" => (raw_mag, Some("in".to_string())),
            "wind" => (raw_mag, Some("kt".to_string())),
            "tornado" => (tor_scale.map(|s| s as f64), Some("EF".to_string())),
            _ => (raw_mag, mag_type),
        };

        out.push(IngestStormEvent {
            event_id,
            episode_id: get(&rec, i_episode_id).and_then(|s| s.parse::<i64>().ok()),
            event_type,
            begin_time: begin_time.to_rfc3339(),
            end_time: end_time.map(|t| t.to_rfc3339()),
            begin_lat,
            begin_lon,
            end_lat,
            end_lon,
            magnitude,
            magnitude_unit,
            tor_f_scale: tor_scale,
            state: get(&rec, i_state),
            cz_name: get(&rec, i_cz_name),
            cz_fips: get(&rec, i_cz_fips),
            cz_type: get(&rec, i_cz_type),
            raw: serde_json::Value::Null,
        });
    }

    Ok(out)
}

/// Normalize a Storm Events EVENT_TYPE to one of our three internal types.
fn normalize_event_type(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "hail" | "marine hail" => Some("hail".to_string()),
        "thunderstorm wind" | "marine thunderstorm wind" => Some("wind".to_string()),
        "tornado" => Some("tornado".to_string()),
        _ => None,
    }
}

/// Parse a Storm Events datetime like `28-APR-15 18:30:00` or
/// `2015-04-28 18:30:00`. Returns UTC (the CSV times are local standard time,
/// but for MVP display purposes we treat them as UTC).
fn parse_storm_datetime(s: &str) -> Option<chrono::DateTime<Utc>> {
    use chrono::NaiveDateTime;
    let s = s.trim();

    // Common Storm Events format: "DD-MON-YY HH:MM:SS"
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%d-%b-%y %H:%M:%S") {
        return Some(Utc.from_utc_datetime(&ndt));
    }
    // ISO-ish "YYYY-MM-DD HH:MM:SS"
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(Utc.from_utc_datetime(&ndt));
    }
    None
}

/// Parse an EF/F scale value like "EF3", "F2", or "3".
fn parse_ef_scale(s: &str) -> Option<i16> {
    let t = s.trim().to_ascii_uppercase();
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<i16>().ok().filter(|v| (0..=5).contains(v))
}

/// Payload posted to the ingester.
#[derive(Debug, Serialize)]
struct StormEventPayload {
    events: Vec<IngestStormEvent>,
}

/// A single storm event in the ingest payload (mirrors the ingester's
/// `StormEventData`).
#[derive(Debug, Clone, Serialize)]
pub struct IngestStormEvent {
    pub event_id: i64,
    pub episode_id: Option<i64>,
    pub event_type: String,
    pub begin_time: String,
    pub end_time: Option<String>,
    pub begin_lat: Option<f64>,
    pub begin_lon: Option<f64>,
    pub end_lat: Option<f64>,
    pub end_lon: Option<f64>,
    pub magnitude: Option<f64>,
    pub magnitude_unit: Option<String>,
    pub tor_f_scale: Option<i16>,
    pub state: Option<String>,
    pub cz_name: Option<String>,
    pub cz_fips: Option<String>,
    pub cz_type: Option<String>,
    pub raw: serde_json::Value,
}

/// Load Storm Events configuration from a model config file.
/// Returns None if the source type is not "storm_events_csv".
pub fn load_storm_events_config(
    config_path: &std::path::Path,
    ingester_url: &str,
) -> Result<Option<StormEventsConfig>> {
    use crate::config::ModelConfig;

    let model_config = ModelConfig::load(config_path)?;

    if model_config.source.source_type != "storm_events_csv" {
        return Ok(None);
    }

    let base_url = model_config
        .source
        .base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let base_url = if base_url.ends_with('/') {
        base_url
    } else {
        format!("{}/", base_url)
    };

    Ok(Some(StormEventsConfig {
        id: model_config.model.id,
        base_url,
        backfill_start_year: model_config.source.backfill_start_year.unwrap_or(1995),
        poll_interval_secs: model_config.schedule.poll_interval_secs,
        ingester_url: ingester_url.to_string(),
    }))
}

// Bring TimeZone into scope for from_utc_datetime / with_ymd_and_hms.
use chrono::TimeZone;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_event_type() {
        assert_eq!(normalize_event_type("Hail").as_deref(), Some("hail"));
        assert_eq!(
            normalize_event_type("Thunderstorm Wind").as_deref(),
            Some("wind")
        );
        assert_eq!(normalize_event_type("Tornado").as_deref(), Some("tornado"));
        assert_eq!(normalize_event_type("Flood"), None);
    }

    #[test]
    fn test_parse_ef_scale() {
        assert_eq!(parse_ef_scale("EF3"), Some(3));
        assert_eq!(parse_ef_scale("F2"), Some(2));
        assert_eq!(parse_ef_scale("5"), Some(5));
        assert_eq!(parse_ef_scale("EF6"), None);
        assert_eq!(parse_ef_scale(""), None);
    }

    #[test]
    fn test_parse_storm_datetime() {
        assert!(parse_storm_datetime("28-APR-15 18:30:00").is_some());
        assert!(parse_storm_datetime("2015-04-28 18:30:00").is_some());
        assert!(parse_storm_datetime("garbage").is_none());
    }

    #[test]
    fn test_resolve_year_filename() {
        let listing = r#"
            <a href="StormEvents_details-ftp_v1.0_d2014_c20220425.csv.gz">f</a>
            <a href="StormEvents_details-ftp_v1.0_d2015_c20220425.csv.gz">f</a>
            <a href="StormEvents_details-ftp_v1.0_d2015_c20230118.csv.gz">f</a>
        "#;
        assert_eq!(
            resolve_year_filename(listing, 2015).as_deref(),
            Some("StormEvents_details-ftp_v1.0_d2015_c20230118.csv.gz")
        );
        assert_eq!(
            resolve_year_filename(listing, 2014).as_deref(),
            Some("StormEvents_details-ftp_v1.0_d2014_c20220425.csv.gz")
        );
        assert_eq!(resolve_year_filename(listing, 1999), None);
    }

    #[test]
    fn test_parse_csv_filters_types() {
        let csv = "EVENT_ID,EVENT_TYPE,BEGIN_DATE_TIME,BEGIN_LAT,BEGIN_LON,MAGNITUDE\n\
                   1,Hail,28-APR-15 18:30:00,35.5,-97.5,1.75\n\
                   2,Flood,28-APR-15 18:30:00,35.5,-97.5,0\n\
                   3,Tornado,28-APR-15 19:00:00,36.0,-98.0,\n";
        let events = parse_storm_events_csv(csv).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "hail");
        assert_eq!(events[0].magnitude, Some(1.75));
        assert_eq!(events[1].event_type, "tornado");
    }
}
