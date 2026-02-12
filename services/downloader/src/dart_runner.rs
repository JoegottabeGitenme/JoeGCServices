//! DART (Deep-ocean Assessment and Reporting of Tsunamis) buoy runner.
//!
//! Fetches water column height data from DART tsunami buoy stations.
//! Station discovery uses NDBC activestations.xml (filter type="dart" dart="y").
//! Per-station data files are at `https://www.ndbc.noaa.gov/data/realtime2/{ID}.dart`
//!
//! Data format: 8-column space-delimited, 2-line `#` header:
//!   #YY  MM DD hh mm ss T   HEIGHT
//!   #yr  mo dy hr mn  s -        m
//!   2026 02 10 06 00 00 1 4265.970
//!
//! Columns: year, month, day, hour, minute, second, measurement_type, height_meters
//! T values: 1=15min (standard), 2=1min (event), 3=15sec (event)
//!
//! Each station file contains ~45 days of history (~4,300 readings).
//! Dedup relies on the existing ON CONFLICT (location_id, source, obs_time) DO NOTHING.

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::observation_runner::{IngestObservation, ObservationPayload};

/// Configuration for the DART runner.
#[derive(Debug, Clone)]
pub struct DartConfig {
    /// Source identifier
    pub id: String,
    /// URL for activestations.xml (station discovery)
    pub stations_url: String,
    /// Base URL for per-station .dart data files
    pub data_base_url: String,
    /// Polling interval in seconds
    pub poll_interval_secs: u64,
    /// Ingester service URL
    pub ingester_url: String,
}

/// A discovered DART station.
#[derive(Debug, Clone)]
struct DartStation {
    /// Station ID (e.g., "21413")
    id: String,
    /// Latitude
    lat: f64,
    /// Longitude
    lon: f64,
    /// Station name/description
    name: String,
}

/// Runner for fetching and ingesting DART tsunami buoy data.
pub struct DartRunner {
    config: DartConfig,
    client: Client,
}

impl DartRunner {
    /// Create a new DART runner.
    pub fn new(config: DartConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("JoeGCServices-Downloader/1.0")
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { config, client })
    }

    /// Run the DART fetcher forever until shutdown signal.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.config.poll_interval_secs);

        info!(
            source = "dart",
            poll_interval_secs = self.config.poll_interval_secs,
            stations_url = %self.config.stations_url,
            data_base_url = %self.config.data_base_url,
            "Starting DART tsunami buoy runner"
        );

        // Initial fetch
        if let Err(e) = self.fetch_and_ingest().await {
            error!(source = "dart", error = %e, "Initial DART fetch failed");
        }

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    if let Err(e) = self.fetch_and_ingest().await {
                        error!(source = "dart", error = %e, "DART fetch failed");
                    }
                }
                _ = shutdown.recv() => {
                    info!(source = "dart", "DART runner shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Discover DART stations, fetch their data, and ingest.
    async fn fetch_and_ingest(&self) -> Result<()> {
        // Step 1: Discover active DART stations
        let stations = self.discover_stations().await?;

        if stations.is_empty() {
            warn!(source = "dart", "No DART stations discovered");
            return Ok(());
        }

        info!(
            source = "dart",
            station_count = stations.len(),
            "Discovered DART stations"
        );

        // Step 2: Fetch data from each station in parallel (max 10 concurrent)
        let all_observations: Vec<IngestObservation> = stream::iter(stations)
            .map(|station| async move {
                match self.fetch_station_data(&station).await {
                    Ok(obs) => obs,
                    Err(e) => {
                        debug!(
                            source = "dart",
                            station = %station.id,
                            error = %e,
                            "Failed to fetch DART station data"
                        );
                        Vec::new()
                    }
                }
            })
            .buffer_unordered(10)
            .collect::<Vec<Vec<IngestObservation>>>()
            .await
            .into_iter()
            .flatten()
            .collect();

        if all_observations.is_empty() {
            debug!(source = "dart", "No DART observations parsed");
            return Ok(());
        }

        info!(
            source = "dart",
            total_observations = all_observations.len(),
            "Parsed DART observations from all stations"
        );

        // Step 3: Send to ingester in batches of 500
        for chunk in all_observations.chunks(500) {
            self.send_to_ingester(chunk.to_vec()).await?;
        }

        Ok(())
    }

    /// Discover active DART stations from activestations.xml.
    ///
    /// Parses the XML looking for elements with type="dart" or dart="y".
    async fn discover_stations(&self) -> Result<Vec<DartStation>> {
        let response = self
            .client
            .get(&self.config.stations_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("Failed to fetch activestations.xml")?;

        if !response.status().is_success() {
            anyhow::bail!("activestations.xml returned HTTP {}", response.status());
        }

        let text = response
            .text()
            .await
            .context("Failed to read activestations.xml body")?;
        let stations = Self::parse_active_stations(&text)?;
        Ok(stations)
    }

    /// Parse activestations.xml to extract DART stations.
    ///
    /// We use simple string parsing rather than a full XML parser to avoid
    /// adding a dependency. The format is stable and well-known.
    ///
    /// Each station element looks like:
    /// <station id="21413" lat="30.515" lon="-152.117" name="..." ... type="dart" ... dart="y" />
    fn parse_active_stations(xml: &str) -> Result<Vec<DartStation>> {
        let mut stations = Vec::new();

        for line in xml.lines() {
            let line = line.trim();

            // Look for <station ...> elements that are DART buoys
            if !line.starts_with("<station ") {
                continue;
            }

            // Check if this is a DART station
            let is_dart = Self::get_xml_attr(line, "type")
                .map(|t| t.eq_ignore_ascii_case("dart"))
                .unwrap_or(false)
                || Self::get_xml_attr(line, "dart")
                    .map(|d| d.eq_ignore_ascii_case("y"))
                    .unwrap_or(false);

            if !is_dart {
                continue;
            }

            // Extract station attributes
            let id = match Self::get_xml_attr(line, "id") {
                Some(id) => id.to_string(),
                None => continue,
            };

            let lat = match Self::get_xml_attr(line, "lat").and_then(|v| v.parse::<f64>().ok()) {
                Some(v) => v,
                None => continue,
            };

            let lon = match Self::get_xml_attr(line, "lon").and_then(|v| v.parse::<f64>().ok()) {
                Some(v) => v,
                None => continue,
            };

            let name = Self::get_xml_attr(line, "name").unwrap_or(&id).to_string();

            stations.push(DartStation { id, lat, lon, name });
        }

        Ok(stations)
    }

    /// Extract an attribute value from a simple XML element string.
    ///
    /// Handles both `attr="value"` and `attr='value'` forms.
    fn get_xml_attr<'a>(element: &'a str, attr: &str) -> Option<&'a str> {
        let pattern = format!("{}=\"", attr);
        if let Some(start) = element.find(&pattern) {
            let value_start = start + pattern.len();
            if let Some(end) = element[value_start..].find('"') {
                return Some(&element[value_start..value_start + end]);
            }
        }

        // Try single quotes
        let pattern = format!("{}='", attr);
        if let Some(start) = element.find(&pattern) {
            let value_start = start + pattern.len();
            if let Some(end) = element[value_start..].find('\'') {
                return Some(&element[value_start..value_start + end]);
            }
        }

        None
    }

    /// Fetch and parse data for a single DART station.
    async fn fetch_station_data(&self, station: &DartStation) -> Result<Vec<IngestObservation>> {
        let url = format!("{}/{}.dart", self.config.data_base_url, station.id);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .with_context(|| format!("Failed to fetch DART data for station {}", station.id))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "DART data for station {} returned HTTP {}",
                station.id,
                response.status()
            );
        }

        let text = response.text().await?;
        Self::parse_dart_data(&text, station)
    }

    /// Parse DART data file content into observations.
    ///
    /// Format (8 columns, space-delimited, 2-line `#` header):
    /// ```text
    /// #YY  MM DD hh mm ss T   HEIGHT
    /// #yr  mo dy hr mn  s -        m
    /// 2026 02 10 06 00 00 1 4265.970
    /// ```
    fn parse_dart_data(text: &str, station: &DartStation) -> Result<Vec<IngestObservation>> {
        let mut observations = Vec::new();

        for line in text.lines() {
            let line = line.trim();

            // Skip header lines and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            match Self::parse_dart_line(line, station) {
                Ok(Some(obs)) => observations.push(obs),
                Ok(None) => {} // Invalid data, skip
                Err(e) => {
                    debug!(
                        station = %station.id,
                        error = %e,
                        line = %line,
                        "Failed to parse DART data line"
                    );
                }
            }
        }

        Ok(observations)
    }

    /// Parse a single data line from a DART file.
    ///
    /// Returns None if the height value is missing/invalid.
    fn parse_dart_line(line: &str, station: &DartStation) -> Result<Option<IngestObservation>> {
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() < 8 {
            anyhow::bail!("Expected 8 columns, got {}", parts.len());
        }

        let year: i32 = parts[0].parse().context("Invalid year")?;
        let month: u32 = parts[1].parse().context("Invalid month")?;
        let day: u32 = parts[2].parse().context("Invalid day")?;
        let hour: u32 = parts[3].parse().context("Invalid hour")?;
        let minute: u32 = parts[4].parse().context("Invalid minute")?;
        let second: u32 = parts[5].parse().context("Invalid second")?;
        let measurement_type: &str = parts[6]; // 1=15min, 2=1min, 3=15sec
        let height: f32 = parts[7].parse().context("Invalid height")?;

        // Build timestamp
        let obs_time = Utc
            .with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .context("Invalid date/time")?;

        // Store measurement type in raw_text for frontend use
        // T=1: standard 15-min, T=2: 1-min event mode, T=3: 15-sec event mode
        let raw_text = format!("T={} HEIGHT={:.3}", measurement_type, height);

        Ok(Some(IngestObservation {
            location_id: station.id.clone(),
            name: Some(station.name.clone()),
            longitude: station.lon,
            latitude: station.lat,
            elevation_m: None,
            obs_time: obs_time.to_rfc3339(),
            receipt_time: Some(Utc::now().to_rfc3339()),
            temperature_k: None,
            dewpoint_k: None,
            wind_direction_deg: None,
            wind_speed_mps: None,
            wind_gust_mps: None,
            visibility_m: None,
            altimeter_pa: None,
            sea_level_pressure_pa: None,
            wave_height_m: None,
            dominant_wave_period_s: None,
            average_wave_period_s: None,
            mean_wave_direction_deg: None,
            water_temp_k: None,
            tide_m: None,
            water_column_height_m: Some(height),
            cloud_layers: None,
            flight_category: None,
            wx_string: None,
            raw_observation: Some(raw_text),
        }))
    }

    /// Send observations to the ingester service.
    async fn send_to_ingester(&self, observations: Vec<IngestObservation>) -> Result<()> {
        let count = observations.len();

        let payload = ObservationPayload {
            source: "dart".to_string(),
            observations,
        };

        // Handle both http://host:port and http://host:port/ingest base URLs
        let base = self.config.ingester_url.trim_end_matches("/ingest");
        let url = format!("{}/ingest/observations", base);

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .context("Failed to send DART observations to ingester")?;

        if response.status().is_success() {
            info!(
                source = "dart",
                count = count,
                "Successfully sent DART observations to ingester"
            );
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!(
                source = "dart",
                status = %status,
                body = %body,
                "Ingester returned error for DART observations"
            );
        }

        Ok(())
    }
}

/// Load DART configuration from a model YAML config file.
///
/// Returns None if the config is not a DART source.
pub fn load_dart_config(path: &std::path::Path, ingester_base: &str) -> Result<Option<DartConfig>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;

    let config: serde_yaml::Value = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse config: {}", path.display()))?;

    // Check source type
    let source_type = config
        .get("source")
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    if source_type != "dart_realtime" {
        return Ok(None);
    }

    // Check if enabled
    let enabled = config
        .get("model")
        .and_then(|m| m.get("enabled"))
        .and_then(|e| e.as_bool())
        .unwrap_or(true);

    if !enabled {
        return Ok(None);
    }

    let id = config
        .get("model")
        .and_then(|m| m.get("id"))
        .and_then(|i| i.as_str())
        .unwrap_or("dart")
        .to_string();

    let stations_url = config
        .get("source")
        .and_then(|s| s.get("stations_url"))
        .and_then(|u| u.as_str())
        .unwrap_or("https://www.ndbc.noaa.gov/activestations.xml")
        .to_string();

    let data_base_url = config
        .get("source")
        .and_then(|s| s.get("data_base_url"))
        .and_then(|u| u.as_str())
        .unwrap_or("https://www.ndbc.noaa.gov/data/realtime2")
        .to_string();

    let poll_interval_secs = config
        .get("schedule")
        .and_then(|s| s.get("poll_interval_secs"))
        .and_then(|p| p.as_u64())
        .unwrap_or(900);

    // Handle ingester URL - strip /ingest suffix if present
    let ingester_url = ingester_base.trim_end_matches("/ingest").to_string();

    Ok(Some(DartConfig {
        id,
        stations_url,
        data_base_url,
        poll_interval_secs,
        ingester_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dart_data_standard_mode() {
        let station = DartStation {
            id: "21413".to_string(),
            lat: 30.515,
            lon: -152.117,
            name: "NW PACIFIC".to_string(),
        };

        let data = r#"#YY  MM DD hh mm ss T   HEIGHT
#yr  mo dy hr mn  s -        m
2026 02 10 06 00 00 1 4265.970
2026 02 10 05 45 00 1 4265.950
2026 02 10 05 30 00 1 4265.930
"#;

        let obs = DartRunner::parse_dart_data(data, &station).unwrap();
        assert_eq!(obs.len(), 3);

        // Check first observation
        assert_eq!(obs[0].location_id, "21413");
        assert_eq!(obs[0].water_column_height_m, Some(4265.970));
        assert!(obs[0].raw_observation.as_ref().unwrap().contains("T=1"));
        assert_eq!(obs[0].longitude, -152.117);
        assert_eq!(obs[0].latitude, 30.515);

        // All meteorological fields should be None
        assert!(obs[0].temperature_k.is_none());
        assert!(obs[0].wave_height_m.is_none());
        assert!(obs[0].tide_m.is_none());
    }

    #[test]
    fn test_parse_dart_data_event_mode() {
        let station = DartStation {
            id: "46413".to_string(),
            lat: 48.0,
            lon: -128.0,
            name: "TEST DART".to_string(),
        };

        let data = r#"#YY  MM DD hh mm ss T   HEIGHT
#yr  mo dy hr mn  s -        m
2026 02 10 06 00 15 3 4100.123
2026 02 10 06 00 00 3 4100.100
2026 02 10 05 59 45 3 4100.080
2026 02 10 05 59 00 2 4100.050
"#;

        let obs = DartRunner::parse_dart_data(data, &station).unwrap();
        assert_eq!(obs.len(), 4);

        // Check event mode markers
        assert!(obs[0].raw_observation.as_ref().unwrap().contains("T=3"));
        assert!(obs[3].raw_observation.as_ref().unwrap().contains("T=2"));
    }

    #[test]
    fn test_parse_dart_data_skip_invalid() {
        let station = DartStation {
            id: "99999".to_string(),
            lat: 0.0,
            lon: 0.0,
            name: "TEST".to_string(),
        };

        let data = r#"#YY  MM DD hh mm ss T   HEIGHT
#yr  mo dy hr mn  s -        m
2026 02 10 06 00 00 1 4265.970
this is not valid data
2026 02 10 05 45 00 1 4265.950
"#;

        let obs = DartRunner::parse_dart_data(data, &station).unwrap();
        assert_eq!(obs.len(), 2); // Invalid line skipped
    }

    #[test]
    fn test_parse_active_stations() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<stations count="1366">
<station id="41001" lat="34.7" lon="-72.7" name="HATTERAS - 150 NM" type="buoy" met="y" currents="n" waterquality="n" dart="n" />
<station id="21413" lat="30.515" lon="-152.117" name="NW PACIFIC" type="dart" met="n" currents="n" waterquality="n" dart="y" />
<station id="46408" lat="49.6" lon="-128.8" name="WEST VANCOUVER ISLAND" type="dart" met="n" currents="n" waterquality="n" dart="y" />
<station id="KORD" lat="41.97" lon="-87.90" name="OHARE" type="fixed" met="y" currents="n" waterquality="n" dart="n" />
</stations>"#;

        let stations = DartRunner::parse_active_stations(xml).unwrap();
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].id, "21413");
        assert_eq!(stations[0].lat, 30.515);
        assert_eq!(stations[0].lon, -152.117);
        assert_eq!(stations[0].name, "NW PACIFIC");
        assert_eq!(stations[1].id, "46408");
    }

    #[test]
    fn test_get_xml_attr() {
        let elem =
            r#"<station id="21413" lat="30.515" lon="-152.117" name="NW PACIFIC" type="dart">"#;
        assert_eq!(DartRunner::get_xml_attr(elem, "id"), Some("21413"));
        assert_eq!(DartRunner::get_xml_attr(elem, "lat"), Some("30.515"));
        assert_eq!(DartRunner::get_xml_attr(elem, "type"), Some("dart"));
        assert_eq!(DartRunner::get_xml_attr(elem, "missing"), None);
    }

    #[test]
    fn test_parse_dart_line() {
        let station = DartStation {
            id: "21413".to_string(),
            lat: 30.515,
            lon: -152.117,
            name: "NW PACIFIC".to_string(),
        };

        let obs = DartRunner::parse_dart_line("2026 02 10 06 00 00 1 4265.970", &station)
            .unwrap()
            .unwrap();

        assert_eq!(obs.location_id, "21413");
        assert_eq!(obs.water_column_height_m, Some(4265.970));
        assert!(obs.obs_time.contains("2026-02-10T06:00:00"));
        assert!(obs.raw_observation.as_ref().unwrap().contains("T=1"));
        assert!(obs
            .raw_observation
            .as_ref()
            .unwrap()
            .contains("HEIGHT=4265.970"));
    }
}
