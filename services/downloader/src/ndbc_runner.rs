//! NDBC (National Data Buoy Center) observation runner.
//!
//! Fetches latest observations from all NDBC buoy/CMAN stations via the
//! `latest_obs.txt` file, which contains a single row per station with
//! the most recent observation (less than 2 hours old).
//!
//! Reference: docs/ocean_data.md
//! Data source: https://www.ndbc.noaa.gov/data/latest_obs/latest_obs.txt

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use reqwest::Client;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{debug, error, info};

use crate::observation_runner::{IngestObservation, ObservationPayload};

/// Configuration for the NDBC runner.
#[derive(Debug, Clone)]
pub struct NdbcConfig {
    /// Source identifier
    pub id: String,
    /// URL for latest_obs.txt
    pub latest_obs_url: String,
    /// URL for activestations.xml (station metadata bootstrap)
    #[allow(dead_code)]
    pub stations_url: String,
    /// Polling interval in seconds
    pub poll_interval_secs: u64,
    /// Ingester service URL
    pub ingester_url: String,
}

/// Runner for fetching and ingesting NDBC buoy observations.
pub struct NdbcRunner {
    config: NdbcConfig,
    client: Client,
}

impl NdbcRunner {
    /// Create a new NDBC runner.
    pub fn new(config: NdbcConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { config, client })
    }

    /// Run the NDBC fetcher forever until shutdown signal.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.config.poll_interval_secs);

        info!(
            source = "ndbc",
            poll_interval_secs = self.config.poll_interval_secs,
            url = %self.config.latest_obs_url,
            "Starting NDBC observation runner"
        );

        // Initial fetch
        if let Err(e) = self.fetch_and_ingest().await {
            error!(source = "ndbc", error = %e, "Initial NDBC fetch failed");
        }

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    if let Err(e) = self.fetch_and_ingest().await {
                        error!(source = "ndbc", error = %e, "NDBC fetch failed");
                    }
                }
                _ = shutdown.recv() => {
                    info!(source = "ndbc", "NDBC runner shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Fetch latest_obs.txt and send observations to ingester.
    async fn fetch_and_ingest(&self) -> Result<()> {
        let text = self.fetch_latest_obs().await?;
        let observations = self.parse_latest_obs(&text)?;

        if observations.is_empty() {
            debug!(source = "ndbc", "No NDBC observations parsed");
            return Ok(());
        }

        info!(
            source = "ndbc",
            count = observations.len(),
            "Parsed NDBC observations"
        );

        self.send_to_ingester(observations).await
    }

    /// Fetch the latest_obs.txt file from NDBC.
    async fn fetch_latest_obs(&self) -> Result<String> {
        let response = self
            .client
            .get(&self.config.latest_obs_url)
            .send()
            .await
            .context("Failed to fetch NDBC latest_obs.txt")?;

        if !response.status().is_success() {
            anyhow::bail!("NDBC latest_obs.txt returned status {}", response.status());
        }

        response
            .text()
            .await
            .context("Failed to read NDBC response body")
    }

    /// Parse the NDBC latest_obs.txt fixed-width format.
    ///
    /// Format (header + data rows):
    /// ```text
    /// #STN     LAT      LON  YYYY MM DD hh mm WDIR WSPD  GST  WVHT   DPD   APD  MWD  PRES  PTDY  ATMP  WTMP  DEWP  VIS  TIDE
    /// #text    deg      deg   yr  mo da hr mn degT  m/s  m/s    m   sec   sec degT   hPa   hPa  degC  degC  degC   nmi    ft
    /// 13001  12.000  -23.000 2024 06 15 12 00  90   5.0  6.0  1.2   8.0   5.5  90 1015.0   MM  26.5  27.2  24.1   MM    MM
    /// ```
    ///
    /// Missing values are reported as `MM`.
    fn parse_latest_obs(&self, text: &str) -> Result<Vec<IngestObservation>> {
        let mut observations = Vec::new();

        for line in text.lines() {
            // Skip header/comment lines
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }

            match self.parse_obs_line(line) {
                Ok(Some(obs)) => observations.push(obs),
                Ok(None) => {} // Skipped (e.g., no valid data)
                Err(e) => {
                    debug!(line = %line.chars().take(40).collect::<String>(), error = %e, "Failed to parse NDBC obs line");
                }
            }
        }

        Ok(observations)
    }

    /// Parse a single observation line from latest_obs.txt.
    ///
    /// Columns are whitespace-separated:
    /// STN LAT LON YYYY MM DD hh mm WDIR WSPD GST WVHT DPD APD MWD PRES PTDY ATMP WTMP DEWP VIS TIDE
    fn parse_obs_line(&self, line: &str) -> Result<Option<IngestObservation>> {
        let fields: Vec<&str> = line.split_whitespace().collect();

        if fields.len() < 22 {
            anyhow::bail!("Expected at least 22 fields, got {}", fields.len());
        }

        let station_id = fields[0].to_string();
        let lat = parse_f64(fields[1])?;
        let lon = parse_f64(fields[2])?;

        // Parse observation time: YYYY MM DD hh mm
        let year: i32 = fields[3].parse().context("Invalid year")?;
        let month: u32 = fields[4].parse().context("Invalid month")?;
        let day: u32 = fields[5].parse().context("Invalid day")?;
        let hour: u32 = fields[6].parse().context("Invalid hour")?;
        let minute: u32 = fields[7].parse().context("Invalid minute")?;

        let obs_time = Utc
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .context("Invalid datetime")?;

        // Parse meteorological fields (MM = missing)
        let wind_dir = parse_optional_i16(fields[8]); // WDIR (degrees true)
        let wind_speed = parse_optional_f32(fields[9]); // WSPD (m/s)
        let wind_gust = parse_optional_f32(fields[10]); // GST (m/s)
        let wave_height = parse_optional_f32(fields[11]); // WVHT (m)
        let dom_wave_period = parse_optional_f32(fields[12]); // DPD (s)
        let avg_wave_period = parse_optional_f32(fields[13]); // APD (s)
        let mean_wave_dir = parse_optional_i16(fields[14]); // MWD (degrees true)
        let pressure = parse_optional_f32(fields[15]); // PRES (hPa)
                                                       // fields[16] = PTDY (pressure tendency, skip)
        let air_temp = parse_optional_f32(fields[17]); // ATMP (°C)
        let water_temp = parse_optional_f32(fields[18]); // WTMP (°C)
        let dewpoint = parse_optional_f32(fields[19]); // DEWP (°C)
        let visibility = parse_optional_f32(fields[20]); // VIS (nautical miles)
        let tide = parse_optional_f32(fields[21]); // TIDE (feet)

        // Check if there's any actual data (not just station metadata)
        let has_data = wind_dir.is_some()
            || wind_speed.is_some()
            || wave_height.is_some()
            || pressure.is_some()
            || air_temp.is_some()
            || water_temp.is_some();

        if !has_data {
            return Ok(None);
        }

        // Unit conversions to SI
        let temperature_k = air_temp.map(|c| c + 273.15);
        let dewpoint_k = dewpoint.map(|c| c + 273.15);
        let water_temp_k = water_temp.map(|c| c + 273.15);
        let sea_level_pressure_pa = pressure.map(|hpa| hpa * 100.0);
        let visibility_m = visibility.map(|nm| nm * 1852.0); // nautical miles to meters
        let tide_m = tide.map(|ft| ft * 0.3048); // feet to meters

        Ok(Some(IngestObservation {
            location_id: station_id,
            name: None, // Will be filled from station metadata
            longitude: lon,
            latitude: lat,
            elevation_m: None,
            obs_time: obs_time.to_rfc3339(),
            receipt_time: Some(Utc::now().to_rfc3339()),
            temperature_k,
            dewpoint_k,
            wind_direction_deg: wind_dir,
            wind_speed_mps: wind_speed, // Already in m/s
            wind_gust_mps: wind_gust,   // Already in m/s
            visibility_m,
            altimeter_pa: None, // Not in NDBC data
            sea_level_pressure_pa,
            wave_height_m: wave_height,
            dominant_wave_period_s: dom_wave_period,
            average_wave_period_s: avg_wave_period,
            mean_wave_direction_deg: mean_wave_dir,
            water_temp_k,
            tide_m,
            cloud_layers: None,
            flight_category: None,
            wx_string: None,
            raw_observation: Some(line.to_string()),
        }))
    }

    /// Send observations to the ingester service.
    async fn send_to_ingester(&self, observations: Vec<IngestObservation>) -> Result<()> {
        let payload = ObservationPayload {
            source: "ndbc".to_string(),
            observations,
        };

        let url = format!("{}/ingest/observations", self.config.ingester_url);

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send NDBC observations to ingester")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Ingester returned {} for NDBC observations: {}",
                status,
                body
            );
        }

        let result: serde_json::Value = response.json().await.unwrap_or_default();
        info!(
            source = "ndbc",
            result = %result,
            "NDBC observations ingested"
        );

        Ok(())
    }
}

/// Load NDBC configuration from a model config file.
/// Returns None if the source type is not "ndbc_latest_obs".
pub fn load_ndbc_config(
    config_path: &std::path::Path,
    ingester_url: &str,
) -> Result<Option<NdbcConfig>> {
    use crate::config::ModelConfig;

    let model_config = ModelConfig::load(config_path)?;

    if model_config.source.source_type != "ndbc_latest_obs" {
        return Ok(None);
    }

    let latest_obs_url = model_config
        .source
        .base_url
        .ok_or_else(|| anyhow::anyhow!("Missing base_url for ndbc_latest_obs source"))?;

    // stations_url comes from the extra config or defaults
    let stations_url = model_config
        .source
        .stations_url
        .unwrap_or_else(|| "https://www.ndbc.noaa.gov/activestations.xml".to_string());

    Ok(Some(NdbcConfig {
        id: model_config.model.id,
        latest_obs_url,
        stations_url,
        poll_interval_secs: model_config.schedule.poll_interval_secs,
        ingester_url: ingester_url.to_string(),
    }))
}

// =============================================================================
// Parsing helpers
// =============================================================================

/// Parse a string to f64, returning an error on failure.
fn parse_f64(s: &str) -> Result<f64> {
    s.parse::<f64>().context("Invalid float value")
}

/// Parse an optional f32 field. Returns None for "MM" (missing marker).
fn parse_optional_f32(s: &str) -> Option<f32> {
    if s == "MM" {
        return None;
    }
    s.parse::<f32>().ok()
}

/// Parse an optional i16 field. Returns None for "MM" (missing marker).
fn parse_optional_i16(s: &str) -> Option<i16> {
    if s == "MM" {
        return None;
    }
    s.parse::<i16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LATEST_OBS: &str = r#"#STN     LAT      LON  YYYY MM DD hh mm WDIR WSPD  GST  WVHT   DPD   APD  MWD  PRES  PTDY  ATMP  WTMP  DEWP  VIS  TIDE
#text    deg      deg   yr  mo da hr mn degT  m/s  m/s    m   sec   sec degT   hPa   hPa  degC  degC  degC   nmi    ft
41001  34.700  -72.700 2024 06 15 12 00 180   5.1  6.3  1.5   8.0   5.5  180 1015.2  -0.3  23.5  25.1  20.1   MM    MM
41002  32.300  -75.200 2024 06 15 12 00 200   3.6   MM  0.8   7.0   4.8  200 1016.5   0.1  24.8  26.3  22.0   MM    MM
FPSN7  35.200  -75.600 2024 06 15 12 00 190   4.5  5.8   MM    MM    MM   MM 1015.8   MM  25.0   MM  21.5   MM   2.3
46087  48.500 -124.700 2024 06 15 11 00  MM    MM   MM   MM    MM    MM   MM    MM    MM    MM    MM    MM    MM    MM
"#;

    #[test]
    fn test_parse_latest_obs() {
        let runner = NdbcRunner {
            config: NdbcConfig {
                id: "ndbc".to_string(),
                latest_obs_url: "https://example.com".to_string(),
                stations_url: "https://example.com".to_string(),
                poll_interval_secs: 600,
                ingester_url: "http://localhost:8082".to_string(),
            },
            client: Client::new(),
        };

        let observations = runner.parse_latest_obs(SAMPLE_LATEST_OBS).unwrap();

        // Station 46087 has all MM values, should be skipped
        assert_eq!(
            observations.len(),
            3,
            "Expected 3 observations (46087 has no data)"
        );

        // Check station 41001
        let obs = &observations[0];
        assert_eq!(obs.location_id, "41001");
        assert_eq!(obs.latitude, 34.7);
        assert_eq!(obs.longitude, -72.7);
        assert!((obs.temperature_k.unwrap() - 296.65).abs() < 0.1); // 23.5°C → K
        assert!((obs.water_temp_k.unwrap() - 298.25).abs() < 0.1); // 25.1°C → K
        assert_eq!(obs.wind_direction_deg, Some(180));
        assert!((obs.wind_speed_mps.unwrap() - 5.1).abs() < 0.01);
        assert!((obs.wind_gust_mps.unwrap() - 6.3).abs() < 0.01);
        assert!((obs.wave_height_m.unwrap() - 1.5).abs() < 0.01);
        assert!((obs.dominant_wave_period_s.unwrap() - 8.0).abs() < 0.01);
        assert!((obs.sea_level_pressure_pa.unwrap() - 101520.0).abs() < 1.0);

        // Check station 41002 (no wind gust)
        let obs = &observations[1];
        assert_eq!(obs.location_id, "41002");
        assert!(obs.wind_gust_mps.is_none());

        // Check CMAN station FPSN7 (no wave data, has tide)
        let obs = &observations[2];
        assert_eq!(obs.location_id, "FPSN7");
        assert!(obs.wave_height_m.is_none());
        assert!(obs.dominant_wave_period_s.is_none());
        assert!((obs.tide_m.unwrap() - 0.70104).abs() < 0.001); // 2.3 ft → m
    }

    #[test]
    fn test_parse_missing_values() {
        assert_eq!(parse_optional_f32("MM"), None);
        assert_eq!(parse_optional_f32("5.5"), Some(5.5));
        assert_eq!(parse_optional_f32("invalid"), None);
        assert_eq!(parse_optional_i16("MM"), None);
        assert_eq!(parse_optional_i16("180"), Some(180));
    }
}
