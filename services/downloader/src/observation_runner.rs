//! Observation data runner for fetching point observations (METAR, TAF, etc.).
//!
//! Unlike `ModelRunner` which downloads files, `ObservationRunner` fetches JSON
//! directly from APIs (like Aviation Weather Center) and sends it to the ingester
//! for storage in PostgreSQL.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Configuration for an observation source.
#[derive(Debug, Clone)]
pub struct ObservationConfig {
    /// Source identifier (e.g., "metar")
    pub id: String,
    /// Base URL for the API
    pub base_url: String,
    /// Polling interval
    pub poll_interval_secs: u64,
    /// Geographic bounding box (for future regional filtering)
    #[allow(dead_code)]
    pub bbox: BoundingBox,
    /// URL of the ingester service
    pub ingester_url: String,
}

/// Geographic bounding box.
#[allow(dead_code)] // Used in config and tests; will be used for regional filtering
#[derive(Debug, Clone)]
pub struct BoundingBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BoundingBox {
    /// Format as API parameter: minLon,minLat,maxLon,maxLat
    #[allow(dead_code)]
    pub fn to_api_param(&self) -> String {
        format!(
            "{},{},{},{}",
            self.min_lon, self.min_lat, self.max_lon, self.max_lat
        )
    }
}

/// METAR observation from Aviation Weather API.
///
/// Field mapping from API JSON to our internal representation.
/// Values are converted to SI units when stored.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Fields populated by serde deserialization
pub struct MetarObservation {
    /// ICAO airport identifier (e.g., "KJFK")
    pub icao_id: String,
    /// Time the report was received by AWC
    pub receipt_time: Option<String>,
    /// Observation time as Unix timestamp
    pub obs_time: i64,
    /// Formatted observation time
    pub report_time: Option<String>,
    /// Temperature in Celsius
    pub temp: Option<f32>,
    /// Dewpoint in Celsius
    pub dewp: Option<f32>,
    /// Wind direction in degrees (0-360, or "VRB" for variable)
    pub wdir: Option<serde_json::Value>,
    /// Wind speed in knots
    pub wspd: Option<i32>,
    /// Wind gust in knots
    pub wgst: Option<i32>,
    /// Visibility (can be "10+" for >10SM, or a number)
    pub visib: Option<serde_json::Value>,
    /// Altimeter setting in hPa
    pub altim: Option<f32>,
    /// Sea level pressure in hPa
    pub slp: Option<f32>,
    /// Quality control field
    pub qc_field: Option<i32>,
    /// METAR type (METAR or SPECI)
    pub metar_type: Option<String>,
    /// Raw observation text
    pub raw_ob: Option<String>,
    /// Station latitude
    pub lat: f64,
    /// Station longitude
    pub lon: f64,
    /// Station elevation in meters
    pub elev: Option<i32>,
    /// Station name
    pub name: Option<String>,
    /// Cloud cover category (SKC, FEW, SCT, BKN, OVC)
    pub cover: Option<String>,
    /// Cloud layers
    pub clouds: Option<Vec<CloudLayer>>,
    /// Flight category (VFR, MVFR, IFR, LIFR)
    pub flt_cat: Option<String>,
    /// Present weather (if any)
    pub wx_string: Option<String>,
}

/// Cloud layer information.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // Fields populated by serde deserialization
pub struct CloudLayer {
    /// Coverage: SKC, FEW, SCT, BKN, OVC, VV (vertical visibility)
    pub cover: String,
    /// Base height in feet AGL
    pub base: Option<i32>,
    /// Cloud type (e.g., CB, TCU) - may be null
    #[serde(rename = "type")]
    pub cloud_type: Option<String>,
}

/// Observation payload sent to the ingester.
#[derive(Debug, Clone, Serialize)]
pub struct ObservationPayload {
    /// Source identifier (e.g., "metar")
    pub source: String,
    /// List of observations
    pub observations: Vec<IngestObservation>,
}

/// Single observation for ingestion (SI units).
#[derive(Debug, Clone, Serialize)]
pub struct IngestObservation {
    /// Location identifier (ICAO code)
    pub location_id: String,
    /// Station name
    pub name: Option<String>,
    /// Longitude
    pub longitude: f64,
    /// Latitude
    pub latitude: f64,
    /// Elevation in meters
    pub elevation_m: Option<f32>,
    /// Observation time (ISO 8601)
    pub obs_time: String,
    /// Receipt time (ISO 8601)
    pub receipt_time: Option<String>,
    /// Temperature in Kelvin
    pub temperature_k: Option<f32>,
    /// Dewpoint in Kelvin
    pub dewpoint_k: Option<f32>,
    /// Wind direction in degrees
    pub wind_direction_deg: Option<i16>,
    /// Wind speed in m/s
    pub wind_speed_mps: Option<f32>,
    /// Wind gust in m/s
    pub wind_gust_mps: Option<f32>,
    /// Visibility in meters
    pub visibility_m: Option<f32>,
    /// Altimeter setting in Pascals
    pub altimeter_pa: Option<f32>,
    /// Sea level pressure in Pascals
    pub sea_level_pressure_pa: Option<f32>,
    /// Significant wave height in meters
    pub wave_height_m: Option<f32>,
    /// Dominant wave period in seconds
    pub dominant_wave_period_s: Option<f32>,
    /// Average wave period in seconds
    pub average_wave_period_s: Option<f32>,
    /// Mean wave direction in degrees
    pub mean_wave_direction_deg: Option<i16>,
    /// Water temperature in Kelvin
    pub water_temp_k: Option<f32>,
    /// Tide / water level in meters
    pub tide_m: Option<f32>,
    /// Cloud layers as JSON array
    pub cloud_layers: Option<serde_json::Value>,
    /// Flight category
    pub flight_category: Option<String>,
    /// Weather phenomena string
    pub wx_string: Option<String>,
    /// Raw observation text
    pub raw_observation: Option<String>,
}

/// Runner for fetching and ingesting observation data.
pub struct ObservationRunner {
    config: ObservationConfig,
    client: Client,
}

impl ObservationRunner {
    /// Create a new observation runner.
    pub fn new(config: ObservationConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("JoeGCServices-Downloader/1.0")
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { config, client })
    }

    /// Run the observation polling loop forever until shutdown.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.config.poll_interval_secs);
        let source_id = &self.config.id;

        info!(
            source = %source_id,
            poll_interval_secs = self.config.poll_interval_secs,
            "Starting observation runner"
        );

        // Run first cycle immediately
        if let Err(e) = self.run_cycle().await {
            error!(source = %source_id, error = %e, "Initial observation fetch failed");
        }

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!(source = %source_id, "Shutting down observation runner");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    debug!(source = %source_id, "Running scheduled observation fetch");
                    if let Err(e) = self.run_cycle().await {
                        error!(source = %source_id, error = %e, "Observation fetch failed");
                    }
                }
            }
        }

        Ok(())
    }

    /// Run a single fetch cycle.
    pub async fn run_cycle(&self) -> Result<()> {
        let source_id = &self.config.id;

        // 1. Fetch observations from API
        let observations = self.fetch_observations().await?;

        if observations.is_empty() {
            debug!(source = %source_id, "No observations received");
            return Ok(());
        }

        info!(
            source = %source_id,
            count = observations.len(),
            "Fetched observations"
        );

        // 2. Convert to SI units and prepare payload
        let payload = self.prepare_payload(observations);

        // 3. Send to ingester
        self.send_to_ingester(payload).await?;

        Ok(())
    }

    /// Fetch observations from the Aviation Weather API.
    ///
    /// Uses station IDs to query because the bbox parameter requires additional
    /// zoom/density params that don't return comprehensive results.
    async fn fetch_observations(&self) -> Result<Vec<MetarObservation>> {
        // Major US airports - the API works reliably with station IDs
        // These cover major metro areas across CONUS
        const STATION_IDS: &str = "KJFK,KLAX,KORD,KDFW,KDEN,KATL,KSFO,KLAS,KSEA,KMCO,\
            KEWR,KBOS,KMSP,KPHX,KDTW,KLGA,KFLL,KIAH,KPHL,KCLT,\
            KMIA,KDCA,KBWI,KSLC,KSAN,KTPA,KAUS,KPDX,KSTL,KMKE,\
            KMCI,KSAT,KOAK,KSMF,KRDU,KBNA,KCLE,KIND,KCMH,KPVD,\
            KPIT,KCVG,KABQ,KOKC,KTUL,KOMA,KBOI,KELP,KCOS,KICT,\
            KBTV,KPWM,KBDL,KALB,KBUF,KSYR,KROC,KRIC,KORF,KGSO,\
            KCHS,KJAX,KPBI,KRSW,KHOU,KMDW,KGEG,KFAT,KSJC,KONT,\
            KBURB,KLGB,KSNA,KABQ,KTUS,KHNL,PANC,PAFA,PAKT,PAJN";

        let url = format!("{}?format=json&ids={}", self.config.base_url, STATION_IDS);

        debug!(url = %url, "Fetching observations");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch observations")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API returned error {}: {}", status, body);
        }

        // Handle 204 No Content (no data available)
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(Vec::new());
        }

        let body = response
            .text()
            .await
            .context("Failed to read response body")?;

        let observations: Vec<MetarObservation> = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "JSON parse error: {}. First 500 chars: {}",
                e,
                &body[..body.len().min(500)]
            )
        })?;

        Ok(observations)
    }

    /// Convert observations to SI units and prepare payload.
    fn prepare_payload(&self, observations: Vec<MetarObservation>) -> ObservationPayload {
        let ingest_observations: Vec<IngestObservation> = observations
            .into_iter()
            .map(|obs| self.convert_observation(obs))
            .collect();

        ObservationPayload {
            source: self.config.id.clone(),
            observations: ingest_observations,
        }
    }

    /// Convert a single observation to SI units.
    fn convert_observation(&self, obs: MetarObservation) -> IngestObservation {
        // Convert temperature: Celsius to Kelvin
        let temperature_k = obs.temp.map(|c| c + 273.15);
        let dewpoint_k = obs.dewp.map(|c| c + 273.15);

        // Convert wind speed: knots to m/s (1 kt = 0.514444 m/s)
        let wind_speed_mps = obs.wspd.map(|kt| kt as f32 * 0.514444);
        let wind_gust_mps = obs.wgst.map(|kt| kt as f32 * 0.514444);

        // Wind direction (can be number or "VRB" for variable)
        let wind_direction_deg = self.parse_wind_direction(&obs.wdir);

        // Convert visibility: parse value, convert SM to meters (1 SM = 1609.34 m)
        let visibility_m = self.parse_visibility(&obs.visib);

        // Convert pressure: hPa to Pascals (1 hPa = 100 Pa)
        let altimeter_pa = obs.altim.map(|hpa| hpa * 100.0);
        let sea_level_pressure_pa = obs.slp.map(|hpa| hpa * 100.0);

        // Convert cloud layers to JSON with heights in meters
        let cloud_layers = obs.clouds.as_ref().map(|clouds| {
            let converted: Vec<serde_json::Value> = clouds
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "cover": c.cover,
                        "base_m": c.base.map(|ft| ft as f32 * 0.3048)
                    })
                })
                .collect();
            serde_json::Value::Array(converted)
        });

        // Format observation time from Unix timestamp
        let obs_time = chrono::DateTime::from_timestamp(obs.obs_time, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();

        IngestObservation {
            location_id: obs.icao_id,
            name: obs.name,
            longitude: obs.lon,
            latitude: obs.lat,
            elevation_m: obs.elev.map(|e| e as f32),
            obs_time,
            receipt_time: obs.receipt_time,
            temperature_k,
            dewpoint_k,
            wind_direction_deg,
            wind_speed_mps,
            wind_gust_mps,
            visibility_m,
            altimeter_pa,
            sea_level_pressure_pa,
            wave_height_m: None,
            dominant_wave_period_s: None,
            average_wave_period_s: None,
            mean_wave_direction_deg: None,
            water_temp_k: None,
            tide_m: None,
            cloud_layers,
            flight_category: obs.flt_cat,
            wx_string: obs.wx_string,
            raw_observation: obs.raw_ob,
        }
    }

    /// Parse visibility value (can be number or "10+" string).
    fn parse_visibility(&self, visib: &Option<serde_json::Value>) -> Option<f32> {
        match visib {
            Some(serde_json::Value::Number(n)) => {
                // Visibility in statute miles
                n.as_f64().map(|sm| sm as f32 * 1609.34)
            }
            Some(serde_json::Value::String(s)) => {
                // Handle "10+" as 16093.4m (10 SM) or greater
                if s.contains('+') {
                    let num_str = s.trim_end_matches('+');
                    num_str.parse::<f32>().ok().map(|sm| sm * 1609.34)
                } else {
                    s.parse::<f32>().ok().map(|sm| sm * 1609.34)
                }
            }
            _ => None,
        }
    }

    /// Parse wind direction value (can be number or "VRB" string).
    /// Returns None for variable winds (VRB).
    fn parse_wind_direction(&self, wdir: &Option<serde_json::Value>) -> Option<i16> {
        match wdir {
            Some(serde_json::Value::Number(n)) => n.as_i64().map(|d| d as i16),
            Some(serde_json::Value::String(s)) => {
                // "VRB" means variable winds - we return None
                if s == "VRB" {
                    None
                } else {
                    s.parse::<i16>().ok()
                }
            }
            _ => None,
        }
    }

    /// Send observations to the ingester.
    async fn send_to_ingester(&self, payload: ObservationPayload) -> Result<()> {
        // Handle both http://host:port and http://host:port/ingest base URLs
        let base = self.config.ingester_url.trim_end_matches("/ingest");
        let url = format!("{}/ingest/observations", base);

        debug!(
            url = %url,
            count = payload.observations.len(),
            "Sending observations to ingester"
        );

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send to ingester")?;

        if response.status().is_success() {
            info!(
                source = %self.config.id,
                count = payload.observations.len(),
                "Observations ingested successfully"
            );
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(
                source = %self.config.id,
                status = %status,
                error = %body,
                "Ingestion failed"
            );
            anyhow::bail!("Ingestion failed with status {}: {}", status, body)
        }
    }
}

// =============================================================================
// TAF (Terminal Aerodrome Forecast) Support
// =============================================================================

/// TAF from Aviation Weather API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TafApiResponse {
    /// ICAO airport identifier
    pub icao_id: String,
    /// Issue time as ISO string
    pub issue_time: Option<String>,
    /// Bulletin time as ISO string (when received)
    pub bulletin_time: Option<String>,
    /// Start of validity period (Unix timestamp)
    pub valid_time_from: i64,
    /// End of validity period (Unix timestamp)
    pub valid_time_to: i64,
    /// Raw TAF text
    #[serde(rename = "rawTAF")]
    pub raw_taf: Option<String>,
    /// Station latitude
    pub lat: f64,
    /// Station longitude
    pub lon: f64,
    /// Station elevation in meters
    pub elev: Option<i32>,
    /// Station name
    pub name: Option<String>,
    /// Forecast periods
    pub fcsts: Option<Vec<TafPeriodApi>>,
}

/// A single forecast period from the TAF API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TafPeriodApi {
    /// Start of period (Unix timestamp)
    pub time_from: i64,
    /// End of period (Unix timestamp)
    pub time_to: i64,
    /// Forecast change indicator: null, "FM", "BECMG", "TEMPO"
    pub fcst_change: Option<String>,
    /// Probability (30 or 40 for PROB groups)
    pub prob: Option<i32>,
    /// Wind direction in degrees (or "VRB" for variable)
    pub wdir: Option<serde_json::Value>,
    /// Wind speed in knots
    pub wspd: Option<i32>,
    /// Wind gust in knots
    pub wgst: Option<i32>,
    /// Visibility (can be number or "6+" string)
    pub visib: Option<serde_json::Value>,
    /// Weather phenomena string
    pub wx_string: Option<String>,
    /// Cloud layers
    pub clouds: Option<Vec<CloudLayer>>,
}

/// TAF payload sent to the ingester.
#[derive(Debug, Clone, Serialize)]
pub struct TafPayload {
    /// Source identifier ("taf")
    pub source: String,
    /// List of TAF forecasts
    pub forecasts: Vec<IngestTaf>,
}

/// Single TAF for ingestion (SI units).
#[derive(Debug, Clone, Serialize)]
pub struct IngestTaf {
    /// Location identifier (ICAO code)
    pub location_id: String,
    /// Station name
    pub name: Option<String>,
    /// Longitude
    pub longitude: f64,
    /// Latitude
    pub latitude: f64,
    /// Elevation in meters
    pub elevation_m: Option<f32>,
    /// Issue time (ISO 8601)
    pub issue_time: String,
    /// Start of validity (ISO 8601)
    pub valid_from: String,
    /// End of validity (ISO 8601)
    pub valid_to: String,
    /// Raw TAF text
    pub raw_taf: Option<String>,
    /// Forecast periods
    pub periods: Vec<IngestTafPeriod>,
}

/// Single TAF period for ingestion (SI units).
#[derive(Debug, Clone, Serialize)]
pub struct IngestTafPeriod {
    /// Start of period (ISO 8601)
    pub period_from: String,
    /// End of period (ISO 8601)
    pub period_to: String,
    /// Change indicator: null, "FM", "BECMG", "TEMPO"
    pub change_indicator: Option<String>,
    /// Probability (30 or 40)
    pub probability: Option<i16>,
    /// Wind direction in degrees
    pub wind_direction_deg: Option<i16>,
    /// Wind speed in m/s
    pub wind_speed_ms: Option<f32>,
    /// Wind gust in m/s
    pub wind_gust_ms: Option<f32>,
    /// Visibility in meters
    pub visibility_m: Option<f32>,
    /// Weather phenomena string
    pub wx_string: Option<String>,
    /// Cloud layers as JSON
    pub cloud_layers: Option<serde_json::Value>,
}

/// Runner for fetching and ingesting TAF data.
pub struct TafRunner {
    config: ObservationConfig,
    client: Client,
}

impl TafRunner {
    /// Create a new TAF runner.
    pub fn new(config: ObservationConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("JoeGCServices-Downloader/1.0")
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { config, client })
    }

    /// Run the TAF polling loop forever until shutdown.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.config.poll_interval_secs);
        let source_id = &self.config.id;

        info!(
            source = %source_id,
            poll_interval_secs = self.config.poll_interval_secs,
            "Starting TAF runner"
        );

        // Run first cycle immediately
        if let Err(e) = self.run_cycle().await {
            error!(source = %source_id, error = %e, "Initial TAF fetch failed");
        }

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!(source = %source_id, "Shutting down TAF runner");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    debug!(source = %source_id, "Running scheduled TAF fetch");
                    if let Err(e) = self.run_cycle().await {
                        error!(source = %source_id, error = %e, "TAF fetch failed");
                    }
                }
            }
        }

        Ok(())
    }

    /// Run a single fetch cycle.
    pub async fn run_cycle(&self) -> Result<()> {
        let source_id = &self.config.id;

        // 1. Fetch TAFs from API
        let tafs = self.fetch_tafs().await?;

        if tafs.is_empty() {
            debug!(source = %source_id, "No TAFs received");
            return Ok(());
        }

        info!(
            source = %source_id,
            count = tafs.len(),
            "Fetched TAFs"
        );

        // 2. Convert to SI units and prepare payload
        let payload = self.prepare_payload(tafs);

        // 3. Send to ingester
        self.send_to_ingester(payload).await?;

        Ok(())
    }

    /// Fetch TAFs from the Aviation Weather API.
    ///
    /// Uses station IDs to query because the bbox parameter requires additional
    /// zoom/density params that don't return comprehensive results.
    async fn fetch_tafs(&self) -> Result<Vec<TafApiResponse>> {
        // Major US airports - same list as METAR for consistency
        const STATION_IDS: &str = "KJFK,KLAX,KORD,KDFW,KDEN,KATL,KSFO,KLAS,KSEA,KMCO,\
            KEWR,KBOS,KMSP,KPHX,KDTW,KLGA,KFLL,KIAH,KPHL,KCLT,\
            KMIA,KDCA,KBWI,KSLC,KSAN,KTPA,KAUS,KPDX,KSTL,KMKE,\
            KMCI,KSAT,KOAK,KSMF,KRDU,KBNA,KCLE,KIND,KCMH,KPVD,\
            KPIT,KCVG,KABQ,KOKC,KTUL,KOMA,KBOI,KELP,KCOS,KICT,\
            KBTV,KPWM,KBDL,KALB,KBUF,KSYR,KROC,KRIC,KORF,KGSO,\
            KCHS,KJAX,KPBI,KRSW,KHOU,KMDW,KGEG,KFAT,KSJC,KONT,\
            KBURB,KLGB,KSNA,KABQ,KTUS,KHNL,PANC,PAFA,PAKT,PAJN";

        let url = format!("{}?format=json&ids={}", self.config.base_url, STATION_IDS);

        debug!(url = %url, "Fetching TAFs");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch TAFs")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("TAF API returned error {}: {}", status, body);
        }

        // Handle 204 No Content
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(Vec::new());
        }

        let body = response
            .text()
            .await
            .context("Failed to read TAF response body")?;

        let tafs: Vec<TafApiResponse> = serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "TAF JSON parse error: {}. First 500 chars: {}",
                e,
                &body[..body.len().min(500)]
            )
        })?;

        Ok(tafs)
    }

    /// Convert TAFs to SI units and prepare payload.
    fn prepare_payload(&self, tafs: Vec<TafApiResponse>) -> TafPayload {
        let forecasts: Vec<IngestTaf> = tafs
            .into_iter()
            .filter_map(|taf| self.convert_taf(taf))
            .collect();

        TafPayload {
            source: self.config.id.clone(),
            forecasts,
        }
    }

    /// Convert a single TAF to SI units.
    fn convert_taf(&self, taf: TafApiResponse) -> Option<IngestTaf> {
        // Parse issue time - try the issueTime field first, then bulletinTime
        let issue_time = taf
            .issue_time
            .as_ref()
            .or(taf.bulletin_time.as_ref())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.to_utc())
            .or_else(|| {
                // Fallback: use validTimeFrom as issue time
                chrono::DateTime::from_timestamp(taf.valid_time_from, 0)
            })?;

        let valid_from = chrono::DateTime::from_timestamp(taf.valid_time_from, 0)?;
        let valid_to = chrono::DateTime::from_timestamp(taf.valid_time_to, 0)?;

        // Convert periods
        let periods: Vec<IngestTafPeriod> = taf
            .fcsts
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| self.convert_taf_period(p))
            .collect();

        Some(IngestTaf {
            location_id: taf.icao_id,
            name: taf.name,
            longitude: taf.lon,
            latitude: taf.lat,
            elevation_m: taf.elev.map(|e| e as f32),
            issue_time: issue_time.to_rfc3339(),
            valid_from: valid_from.to_rfc3339(),
            valid_to: valid_to.to_rfc3339(),
            raw_taf: taf.raw_taf,
            periods,
        })
    }

    /// Convert a single TAF period to SI units.
    fn convert_taf_period(&self, period: TafPeriodApi) -> Option<IngestTafPeriod> {
        let period_from = chrono::DateTime::from_timestamp(period.time_from, 0)?;
        let period_to = chrono::DateTime::from_timestamp(period.time_to, 0)?;

        // Convert wind speed: knots to m/s
        let wind_speed_ms = period.wspd.map(|kt| kt as f32 * 0.514444);
        let wind_gust_ms = period.wgst.map(|kt| kt as f32 * 0.514444);

        // Convert visibility: SM to meters
        let visibility_m = self.parse_visibility(&period.visib);

        // Convert cloud layers to SI (feet to meters)
        let cloud_layers = period.clouds.as_ref().map(|clouds| {
            let converted: Vec<serde_json::Value> = clouds
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "cover": c.cover,
                        "base_m": c.base.map(|ft| ft as f32 * 0.3048)
                    })
                })
                .collect();
            serde_json::Value::Array(converted)
        });

        Some(IngestTafPeriod {
            period_from: period_from.to_rfc3339(),
            period_to: period_to.to_rfc3339(),
            change_indicator: period.fcst_change,
            probability: period.prob.map(|p| p as i16),
            wind_direction_deg: self.parse_wind_direction(&period.wdir),
            wind_speed_ms,
            wind_gust_ms,
            visibility_m,
            wx_string: period.wx_string,
            cloud_layers,
        })
    }

    /// Parse wind direction value (can be number or "VRB" string).
    /// Returns None for variable winds (VRB).
    fn parse_wind_direction(&self, wdir: &Option<serde_json::Value>) -> Option<i16> {
        match wdir {
            Some(serde_json::Value::Number(n)) => n.as_i64().map(|d| d as i16),
            Some(serde_json::Value::String(s)) => {
                // "VRB" means variable winds - we return None
                if s == "VRB" {
                    None
                } else {
                    s.parse::<i16>().ok()
                }
            }
            _ => None,
        }
    }

    /// Parse visibility value (can be number or "6+" string).
    fn parse_visibility(&self, visib: &Option<serde_json::Value>) -> Option<f32> {
        match visib {
            Some(serde_json::Value::Number(n)) => {
                // Visibility in statute miles
                n.as_f64().map(|sm| sm as f32 * 1609.34)
            }
            Some(serde_json::Value::String(s)) => {
                // Handle "6+" as 9999m (>6 SM, unlimited visibility)
                if s.contains('+') {
                    Some(9999.0)
                } else {
                    s.parse::<f32>().ok().map(|sm| sm * 1609.34)
                }
            }
            _ => None,
        }
    }

    /// Send TAFs to the ingester.
    async fn send_to_ingester(&self, payload: TafPayload) -> Result<()> {
        // Handle both http://host:port and http://host:port/ingest base URLs
        let base = self.config.ingester_url.trim_end_matches("/ingest");
        let url = format!("{}/ingest/tafs", base);

        debug!(
            url = %url,
            count = payload.forecasts.len(),
            "Sending TAFs to ingester"
        );

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("Failed to send TAFs to ingester")?;

        if response.status().is_success() {
            info!(
                source = %self.config.id,
                count = payload.forecasts.len(),
                "TAFs ingested successfully"
            );
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            warn!(
                source = %self.config.id,
                status = %status,
                error = %body,
                "TAF ingestion failed"
            );
            anyhow::bail!("TAF ingestion failed with status {}: {}", status, body)
        }
    }
}

/// Load observation configuration from a model config file.
/// Returns None if the source type is not "aviation_weather_api" (METAR).
pub fn load_observation_config(
    config_path: &std::path::Path,
    ingester_url: &str,
) -> Result<Option<ObservationConfig>> {
    use crate::config::ModelConfig;

    let model_config = ModelConfig::load(config_path)?;

    // Only process if source type is aviation_weather_api (METAR)
    if model_config.source.source_type != "aviation_weather_api" {
        return Ok(None);
    }

    let base_url = model_config
        .source
        .base_url
        .ok_or_else(|| anyhow::anyhow!("Missing base_url for aviation_weather_api source"))?;

    let bbox = BoundingBox {
        min_lon: model_config.grid.bbox.min_lon,
        min_lat: model_config.grid.bbox.min_lat,
        max_lon: model_config.grid.bbox.max_lon,
        max_lat: model_config.grid.bbox.max_lat,
    };

    Ok(Some(ObservationConfig {
        id: model_config.model.id,
        base_url,
        poll_interval_secs: model_config.schedule.poll_interval_secs,
        bbox,
        ingester_url: ingester_url.to_string(),
    }))
}

/// Load TAF configuration from a model config file.
/// Returns None if the source type is not "aviation_weather_api_taf".
pub fn load_taf_config(
    config_path: &std::path::Path,
    ingester_url: &str,
) -> Result<Option<ObservationConfig>> {
    use crate::config::ModelConfig;

    let model_config = ModelConfig::load(config_path)?;

    // Only process if source type is aviation_weather_api_taf
    if model_config.source.source_type != "aviation_weather_api_taf" {
        return Ok(None);
    }

    let base_url = model_config
        .source
        .base_url
        .ok_or_else(|| anyhow::anyhow!("Missing base_url for aviation_weather_api_taf source"))?;

    let bbox = BoundingBox {
        min_lon: model_config.grid.bbox.min_lon,
        min_lat: model_config.grid.bbox.min_lat,
        max_lon: model_config.grid.bbox.max_lon,
        max_lat: model_config.grid.bbox.max_lat,
    };

    Ok(Some(ObservationConfig {
        id: model_config.model.id,
        base_url,
        poll_interval_secs: model_config.schedule.poll_interval_secs,
        bbox,
        ingester_url: ingester_url.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbox_to_api_param() {
        let bbox = BoundingBox {
            min_lon: -130.0,
            min_lat: 20.0,
            max_lon: -60.0,
            max_lat: 55.0,
        };
        assert_eq!(bbox.to_api_param(), "-130,20,-60,55");
    }

    #[test]
    fn test_parse_metar_json() {
        let json = r#"{
            "icaoId": "KJFK",
            "receiptTime": "2026-01-19T18:58:16.068Z",
            "obsTime": 1768848660,
            "reportTime": "2026-01-19T19:00:00.000Z",
            "temp": 0,
            "dewp": -6.1,
            "wdir": 240,
            "wspd": 19,
            "wgst": 24,
            "visib": "10+",
            "altim": 1014,
            "slp": 1013.7,
            "qcField": 12,
            "metarType": "METAR",
            "rawOb": "METAR KJFK 191851Z 24019G24KT 10SM SCT039 SCT090 00/M06 A2994 RMK AO2 PK WND 24026/1820 SLP137 T00001061 $",
            "lat": 40.6392,
            "lon": -73.7639,
            "elev": 3,
            "name": "New York/JF Kennedy Intl, NY, US",
            "cover": "SCT",
            "clouds": [{"cover": "SCT", "base": 3900}, {"cover": "SCT", "base": 9000}],
            "fltCat": "VFR"
        }"#;

        let obs: MetarObservation = serde_json::from_str(json).unwrap();
        assert_eq!(obs.icao_id, "KJFK");
        assert_eq!(obs.temp, Some(0.0));
        assert_eq!(obs.dewp, Some(-6.1));
        assert_eq!(obs.wdir, Some(serde_json::json!(240)));
        assert_eq!(obs.wspd, Some(19));
        assert_eq!(obs.lat, 40.6392);
        assert_eq!(obs.lon, -73.7639);
        assert_eq!(obs.flt_cat, Some("VFR".to_string()));
    }

    #[test]
    fn test_unit_conversions() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test temperature conversion: 20C -> 293.15K
        let obs = MetarObservation {
            icao_id: "TEST".to_string(),
            receipt_time: None,
            obs_time: 1700000000,
            report_time: None,
            temp: Some(20.0),
            dewp: Some(10.0),
            wdir: Some(serde_json::json!(180)),
            wspd: Some(10), // 10 knots
            wgst: None,
            visib: Some(serde_json::json!(10)), // 10 SM
            altim: Some(1013.25),               // hPa
            slp: Some(1013.25),
            qc_field: None,
            metar_type: None,
            raw_ob: None,
            lat: 40.0,
            lon: -74.0,
            elev: Some(10),
            name: None,
            cover: None,
            clouds: Some(vec![CloudLayer {
                cover: "SCT".to_string(),
                base: Some(3000), // 3000 ft
                cloud_type: None,
            }]),
            flt_cat: None,
            wx_string: None,
        };

        let converted = runner.convert_observation(obs);

        // Temperature: 20C = 293.15K
        assert!((converted.temperature_k.unwrap() - 293.15).abs() < 0.01);
        // Dewpoint: 10C = 283.15K
        assert!((converted.dewpoint_k.unwrap() - 283.15).abs() < 0.01);
        // Wind: 10 kt = 5.14444 m/s
        assert!((converted.wind_speed_mps.unwrap() - 5.14444).abs() < 0.01);
        // Visibility: 10 SM = 16093.4 m
        assert!((converted.visibility_m.unwrap() - 16093.4).abs() < 1.0);
        // Altimeter: 1013.25 hPa = 101325 Pa
        assert!((converted.altimeter_pa.unwrap() - 101325.0).abs() < 1.0);
        // Cloud layers: should have one layer with SCT at ~914.4m
        let layers = converted.cloud_layers.unwrap();
        let first_layer = layers.as_array().unwrap().first().unwrap();
        assert_eq!(first_layer["cover"], "SCT");
        let base_m = first_layer["base_m"].as_f64().unwrap();
        assert!((base_m - 914.4).abs() < 0.1);
    }

    #[test]
    fn test_parse_taf_json() {
        let json = r#"{
            "icaoId": "KJFK",
            "issueTime": "2026-01-19T20:30:00.000Z",
            "bulletinTime": "2026-01-19T20:30:00.000Z",
            "validTimeFrom": 1768856400,
            "validTimeTo": 1768953600,
            "rawTAF": "TAF KJFK 192030Z 1921/2024 25018G27KT P6SM SCT040",
            "lat": 40.6392,
            "lon": -73.7639,
            "elev": 3,
            "name": "New York/JF Kennedy Intl, NY, US",
            "fcsts": [
                {
                    "timeFrom": 1768856400,
                    "timeTo": 1768888800,
                    "fcstChange": null,
                    "wdir": 250,
                    "wspd": 18,
                    "wgst": 27,
                    "visib": "6+",
                    "wxString": null,
                    "clouds": [{"cover": "SCT", "base": 4000}]
                },
                {
                    "timeFrom": 1768888800,
                    "timeTo": 1768917600,
                    "fcstChange": "FM",
                    "wdir": 270,
                    "wspd": 15,
                    "wgst": null,
                    "visib": 6,
                    "wxString": null,
                    "clouds": [{"cover": "FEW", "base": 5000}]
                }
            ]
        }"#;

        let taf: TafApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(taf.icao_id, "KJFK");
        assert_eq!(taf.valid_time_from, 1768856400);
        assert_eq!(taf.valid_time_to, 1768953600);
        assert_eq!(taf.lat, 40.6392);
        assert_eq!(taf.lon, -73.7639);

        let fcsts = taf.fcsts.unwrap();
        assert_eq!(fcsts.len(), 2);
        assert_eq!(fcsts[0].wdir, Some(serde_json::json!(250)));
        assert_eq!(fcsts[0].wspd, Some(18));
        assert_eq!(fcsts[0].wgst, Some(27));
        assert!(fcsts[0].fcst_change.is_none());
        assert_eq!(fcsts[1].fcst_change, Some("FM".to_string()));
    }

    #[test]
    fn test_taf_unit_conversions() {
        let config = ObservationConfig {
            id: "taf".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 600,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = TafRunner::new(config).unwrap();

        let period = TafPeriodApi {
            time_from: 1768856400,
            time_to: 1768888800,
            fcst_change: Some("FM".to_string()),
            prob: Some(30),
            wdir: Some(serde_json::json!(250)),
            wspd: Some(20),                    // 20 knots
            wgst: Some(30),                    // 30 knots
            visib: Some(serde_json::json!(3)), // 3 SM
            wx_string: Some("RA".to_string()),
            clouds: Some(vec![CloudLayer {
                cover: "BKN".to_string(),
                base: Some(2000), // 2000 ft
                cloud_type: None,
            }]),
        };

        let converted = runner.convert_taf_period(period).unwrap();

        // Wind speed: 20 kt = 10.29 m/s
        assert!((converted.wind_speed_ms.unwrap() - 10.2889).abs() < 0.01);
        // Wind gust: 30 kt = 15.43 m/s
        assert!((converted.wind_gust_ms.unwrap() - 15.4333).abs() < 0.01);
        // Visibility: 3 SM = 4828 m
        assert!((converted.visibility_m.unwrap() - 4828.0).abs() < 1.0);
        // Change indicator
        assert_eq!(converted.change_indicator, Some("FM".to_string()));
        // Probability
        assert_eq!(converted.probability, Some(30));
        // Cloud layers
        let layers = converted.cloud_layers.unwrap();
        let first_layer = layers.as_array().unwrap().first().unwrap();
        assert_eq!(first_layer["cover"], "BKN");
        let base_m = first_layer["base_m"].as_f64().unwrap();
        assert!((base_m - 609.6).abs() < 0.1); // 2000 ft = 609.6 m
    }

    // =========================================================================
    // Visibility parsing edge cases
    // =========================================================================

    #[test]
    fn test_parse_visibility_number() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test numeric visibility (5 SM)
        let vis = runner.parse_visibility(&Some(serde_json::json!(5)));
        assert!(vis.is_some());
        let vis_m = vis.unwrap();
        assert!((vis_m - 8046.7).abs() < 1.0); // 5 SM = 8046.7 m
    }

    #[test]
    fn test_parse_visibility_plus_string() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test "10+" visibility (common for clear conditions)
        let vis = runner.parse_visibility(&Some(serde_json::json!("10+")));
        assert!(vis.is_some());
        let vis_m = vis.unwrap();
        assert!((vis_m - 16093.4).abs() < 1.0); // 10 SM = 16093.4 m
    }

    #[test]
    fn test_parse_visibility_string_number() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test string number without plus
        let vis = runner.parse_visibility(&Some(serde_json::json!("3")));
        assert!(vis.is_some());
        let vis_m = vis.unwrap();
        assert!((vis_m - 4828.0).abs() < 1.0); // 3 SM = 4828 m
    }

    #[test]
    fn test_parse_visibility_none() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test None visibility
        let vis = runner.parse_visibility(&None);
        assert!(vis.is_none());
    }

    // =========================================================================
    // Wind direction parsing edge cases
    // =========================================================================

    #[test]
    fn test_parse_wind_direction_number() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test numeric wind direction
        let wdir = runner.parse_wind_direction(&Some(serde_json::json!(270)));
        assert_eq!(wdir, Some(270));
    }

    #[test]
    fn test_parse_wind_direction_vrb() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // VRB (variable) should return None
        let wdir = runner.parse_wind_direction(&Some(serde_json::json!("VRB")));
        assert!(wdir.is_none());
    }

    #[test]
    fn test_parse_wind_direction_string_number() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test string number
        let wdir = runner.parse_wind_direction(&Some(serde_json::json!("180")));
        assert_eq!(wdir, Some(180));
    }

    #[test]
    fn test_parse_wind_direction_none() {
        let config = ObservationConfig {
            id: "test".to_string(),
            base_url: "http://test".to_string(),
            poll_interval_secs: 300,
            bbox: BoundingBox {
                min_lon: -130.0,
                min_lat: 20.0,
                max_lon: -60.0,
                max_lat: 55.0,
            },
            ingester_url: "http://test".to_string(),
        };
        let runner = ObservationRunner::new(config).unwrap();

        // Test None
        let wdir = runner.parse_wind_direction(&None);
        assert!(wdir.is_none());
    }
}
