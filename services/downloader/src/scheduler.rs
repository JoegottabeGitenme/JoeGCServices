//! Download scheduler with per-model polling and ingestion triggers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Timelike, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, error, info, instrument, warn};

use crate::config::{self, ModelConfig};
use crate::download::DownloadManager;
use crate::state::DownloadState;

/// Model schedule info for API display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSchedule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Model cycles (e.g., [0, 6, 12, 18] for GFS)
    pub cycles: Vec<u32>,
    /// Hours after cycle that data becomes available
    pub delay_hours: u32,
    /// Seconds between polls
    pub poll_interval_secs: u64,
    /// S3 bucket
    pub bucket: String,
    /// Prefix template with {date}, {cycle} placeholders
    pub prefix_template: String,
    /// File pattern (e.g., "pgrb2.0p25.f{forecast:03}")
    pub file_pattern: String,
    /// Forecast hours to download
    pub forecast_hours: Vec<u32>,
    /// Whether this is observation data (vs forecast)
    pub is_observation: bool,
}

impl From<&ModelConfig> for ModelSchedule {
    fn from(config: &ModelConfig) -> Self {
        Self {
            id: config.model.id.clone(),
            name: config.model.name.clone(),
            enabled: config.model.enabled,
            cycles: config.schedule.cycles.clone(),
            delay_hours: config.schedule.delay_hours,
            poll_interval_secs: config.schedule.poll_interval_secs,
            bucket: config.source.bucket.clone(),
            prefix_template: config.source.prefix_template.clone(),
            file_pattern: config.source.file_pattern.clone(),
            forecast_hours: config.forecast_hours(),
            is_observation: config.is_observation(),
        }
    }
}

/// Download scheduler coordinating multiple models.
#[allow(dead_code)]
pub struct Scheduler {
    download_manager: Arc<DownloadManager>,
    state: Arc<DownloadState>,
    max_concurrent: usize,
    ingester_url: Option<String>,
    client: Client,
    config_dir: PathBuf,
    /// Cached model configs
    model_configs: Vec<ModelConfig>,
    /// AWS S3 client for listing files
    s3_client: Option<aws_sdk_s3::Client>,
}

impl Scheduler {
    pub async fn new(
        download_manager: Arc<DownloadManager>,
        state: Arc<DownloadState>,
        max_concurrent: usize,
        ingester_url: Option<String>,
        config_dir: PathBuf,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        // Load configs at startup
        let model_configs = config::load_model_configs(&config_dir).unwrap_or_else(|e| {
            warn!(error = %e, "Failed to load model configs, using defaults");
            Self::default_configs()
        });

        // Initialize AWS SDK for S3 listing
        // For NOAA public buckets, we need to explicitly allow anonymous access
        // by providing credentials (they won't be used but SDK requires them)
        let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .no_credentials() // Use unsigned requests for public buckets
            .load()
            .await;
        let s3_client = Some(aws_sdk_s3::Client::new(&aws_config));

        Self {
            download_manager,
            state,
            max_concurrent,
            ingester_url,
            client,
            config_dir,
            model_configs,
            s3_client,
        }
    }

    /// Get the model schedules for status display.
    pub fn get_model_schedules(&self) -> Vec<ModelSchedule> {
        self.model_configs.iter().map(ModelSchedule::from).collect()
    }

    /// Run a single download cycle for all models.
    pub async fn run_all(&self) -> Result<()> {
        for model in &self.model_configs {
            if model.model.enabled {
                if let Err(e) = self.run_model(&model.model.id).await {
                    error!(model = %model.model.id, error = %e, "Model download failed");
                }
            }
        }

        // Trigger ingestion for completed downloads
        self.trigger_ingestion().await?;

        Ok(())
    }

    /// Run a single download cycle for a specific model.
    #[instrument(skip(self))]
    pub async fn run_model(&self, model_id: &str) -> Result<()> {
        let model = self
            .model_configs
            .iter()
            .find(|m| m.model.id == model_id)
            .context("Model not found")?;

        if !model.model.enabled {
            info!(model = %model_id, "Model is disabled, skipping");
            return Ok(());
        }

        info!(model = %model_id, "Starting download cycle");

        // Discover available files based on model type
        let files = if model.is_observation() {
            self.discover_observation_files(model).await?
        } else {
            self.discover_forecast_files(model).await?
        };

        if files.is_empty() {
            info!(model = %model_id, "No new files to download");
            return Ok(());
        }

        info!(model = %model_id, count = files.len(), "Found files to download");

        // Queue downloads
        for (url, filename) in &files {
            // Skip if already downloaded
            if self.state.is_already_downloaded(url).await? {
                debug!(url = %url, "Already downloaded, skipping");
                continue;
            }

            self.state.queue_download(url, filename, model_id).await?;
        }

        // Process download queue with concurrency limit
        // Each download triggers ingestion immediately upon completion
        let pending = self.state.get_in_progress().await?;
        let ingester_url = self.ingester_url.clone();
        let client = self.client.clone();

        let results = stream::iter(pending)
            .map(|record| {
                let manager = self.download_manager.clone();
                let state = self.state.clone();
                let ingester_url = ingester_url.clone();
                let client = client.clone();
                async move {
                    match manager.download(&record.url, &record.filename, &state).await {
                        Ok(path) => {
                            info!(url = %record.url, path = %path.display(), "Download complete");

                            // Trigger ingestion immediately for this file
                            if let Some(ref url) = ingester_url {
                                let file_path = format!("/data/downloads/{}", record.filename);
                                match client
                                    .post(url)
                                    .json(&serde_json::json!({
                                        "file_path": file_path,
                                        "source_url": record.url
                                    }))
                                    .send()
                                    .await
                                {
                                    Ok(response) if response.status().is_success() => {
                                        info!(file = %record.filename, "Ingestion triggered successfully");
                                        let _ = state.mark_ingested(&record.url).await;
                                    }
                                    Ok(response) => {
                                        warn!(
                                            file = %record.filename,
                                            status = %response.status(),
                                            "Ingestion request failed"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(file = %record.filename, error = %e, "Failed to trigger ingestion");
                                    }
                                }
                            }

                            Ok(path)
                        }
                        Err(e) => {
                            error!(url = %record.url, error = %e, "Download failed");
                            Err(e)
                        }
                    }
                }
            })
            .buffer_unordered(self.max_concurrent)
            .collect::<Vec<_>>()
            .await;

        let (successes, failures): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);

        info!(
            model = %model_id,
            success = successes.len(),
            failed = failures.len(),
            "Download cycle complete"
        );

        Ok(())
    }

    /// Run continuously, polling for new data.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        // Track last poll time per model
        let mut last_poll: HashMap<String, std::time::Instant> = HashMap::new();

        loop {
            for model in &self.model_configs {
                if !model.model.enabled {
                    continue;
                }

                let interval = Duration::from_secs(model.schedule.poll_interval_secs);
                let last = last_poll.get(&model.model.id).copied();

                let should_poll = match last {
                    None => true,
                    Some(t) => t.elapsed() >= interval,
                };

                if should_poll {
                    info!(model = %model.model.id, "Running scheduled download");

                    if let Err(e) = self.run_model(&model.model.id).await {
                        error!(model = %model.model.id, error = %e, "Scheduled download failed");
                    }

                    last_poll.insert(model.model.id.clone(), std::time::Instant::now());
                }
            }

            // Trigger ingestion for any completed downloads
            if let Err(e) = self.trigger_ingestion().await {
                warn!(error = %e, "Failed to trigger ingestion");
            }

            // Check for shutdown
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("Shutting down scheduler");
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    // Continue polling
                }
            }
        }

        Ok(())
    }

    /// Discover forecast files available for download (GFS, HRRR, etc.).
    async fn discover_forecast_files(&self, model: &ModelConfig) -> Result<Vec<(String, String)>> {
        let mut files = Vec::new();

        // Get the most recent available cycle
        let (date, cycle) =
            self.latest_available_cycle(&model.schedule.cycles, model.schedule.delay_hours);

        info!(
            model = %model.model.id,
            date = %date,
            cycle = cycle,
            "Checking for available forecast files"
        );

        for forecast_hour in model.forecast_hours() {
            let filename = model
                .source
                .file_pattern
                .replace("{cycle:02}", &format!("{:02}", cycle))
                .replace("{forecast:03}", &format!("{:03}", forecast_hour))
                .replace("{forecast:02}", &format!("{:02}", forecast_hour));

            let prefix = model
                .source
                .prefix_template
                .replace("{date}", &date)
                .replace("{cycle:02}", &format!("{:02}", cycle));

            let url = format!(
                "https://{}.s3.amazonaws.com/{}/{}",
                model.source.bucket, prefix, filename
            );

            // Check if file exists (HEAD request)
            match self.check_file_exists(&url).await {
                Ok(true) => {
                    let output_filename = format!(
                        "{}_{}_{:02}z_f{:03}.grib2",
                        model.model.id, date, cycle, forecast_hour
                    );
                    files.push((url, output_filename));
                }
                Ok(false) => {
                    debug!(url = %url, "File not yet available");
                }
                Err(e) => {
                    debug!(url = %url, error = %e, "Error checking file");
                }
            }
        }

        Ok(files)
    }

    /// Discover observation files available for download (MRMS, GOES, etc.).
    async fn discover_observation_files(
        &self,
        model: &ModelConfig,
    ) -> Result<Vec<(String, String)>> {
        let mut files = Vec::new();

        // For observation data, we look for recent timestamps
        let lookback = model.schedule.lookback_minutes;
        let now = Utc::now();

        info!(
            model = %model.model.id,
            lookback_minutes = lookback,
            "Checking for available observation files"
        );

        // MRMS uses S3 listing to discover recent files
        if model.model.id == "mrms" {
            // MRMS files are organized by date: CONUS/{product}/{YYYYMMDD}/MRMS_{product}_{timestamp}.grib2.gz
            // Files are available every 2 minutes
            let date_str = now.format("%Y%m%d").to_string();

            // Check each MRMS product
            for param in &model.parameters {
                if let Some(product) = param.product.as_ref() {
                    let prefix = format!("CONUS/{}/{}/", product, date_str);

                    // Calculate start_after to skip to files within lookback period
                    // MRMS filename format: MRMS_{product}_{YYYYMMDD-HHMMSS}.grib2.gz
                    let earliest_time = now - ChronoDuration::minutes(lookback as i64);
                    let start_after_key = format!(
                        "CONUS/{}/{}/MRMS_{}_{}",
                        product,
                        date_str,
                        product,
                        earliest_time.format("%Y%m%d-%H%M00")
                    );

                    debug!(
                        model = %model.model.id,
                        prefix = %prefix,
                        product = product,
                        start_after = %start_after_key,
                        "Listing MRMS files from S3"
                    );

                    // List recent files from S3 using start_after to skip older files
                    match self
                        .list_s3_files(&model.source.bucket, &prefix, 50, Some(&start_after_key))
                        .await
                    {
                        Ok(keys) => {
                            info!(
                                model = %model.model.id,
                                prefix = %prefix,
                                count = keys.len(),
                                "Listed files from S3"
                            );
                            // Filter for files within lookback period
                            // Filename format: MRMS_{product}_{YYYYMMDD-HHMMSS}.grib2.gz
                            let earliest_time = now - ChronoDuration::minutes(lookback as i64);

                            for key in keys {
                                if key.ends_with(".grib2.gz") && key.contains(product) {
                                    // Extract timestamp from filename
                                    // Example: CONUS/MergedReflectivityQC_00.50/20251202/MRMS_MergedReflectivityQC_00.50_20251202-005440.grib2.gz
                                    if let Some(filename) = key.split('/').next_back() {
                                        // Parse timestamp from filename (YYYYMMDD-HHMMSS)
                                        if let Some(timestamp_part) =
                                            filename.split('_').next_back()
                                        {
                                            let timestamp_str =
                                                timestamp_part.replace(".grib2.gz", "");
                                            // Format: 20251202-175037 (YYYYMMDD-HHMMSS, total 15 chars including hyphen)
                                            // Remove hyphen for easier parsing
                                            let timestamp_clean = timestamp_str.replace("-", "");
                                            // Now format is: 20251202175037 (14 chars)
                                            if timestamp_clean.len() >= 14 {
                                                let year: i32 =
                                                    timestamp_clean[0..4].parse().unwrap_or(0);
                                                let month: u32 =
                                                    timestamp_clean[4..6].parse().unwrap_or(0);
                                                let day: u32 =
                                                    timestamp_clean[6..8].parse().unwrap_or(0);
                                                let hour: u32 =
                                                    timestamp_clean[8..10].parse().unwrap_or(0);
                                                let minute: u32 =
                                                    timestamp_clean[10..12].parse().unwrap_or(0);
                                                let second: u32 =
                                                    timestamp_clean[12..14].parse().unwrap_or(0);

                                                if let Some(naive_dt) =
                                                    chrono::NaiveDate::from_ymd_opt(
                                                        year, month, day,
                                                    )
                                                    .and_then(|d| {
                                                        d.and_hms_opt(hour, minute, second)
                                                    })
                                                {
                                                    let file_time =
                                                        DateTime::<Utc>::from_naive_utc_and_offset(
                                                            naive_dt, Utc,
                                                        );

                                                    // Only include files within lookback period
                                                    if file_time >= earliest_time
                                                        && file_time <= now
                                                    {
                                                        let url = format!(
                                                            "https://{}.s3.amazonaws.com/{}",
                                                            model.source.bucket, key
                                                        );

                                                        let output_filename =
                                                            format!("mrms_{}", filename);

                                                        info!(
                                                            model = %model.model.id,
                                                            url = %url,
                                                            output = %output_filename,
                                                            timestamp = %file_time,
                                                            "Found MRMS file"
                                                        );

                                                        files.push((url, output_filename));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                model = %model.model.id,
                                prefix = %prefix,
                                error = %e,
                                "Failed to list MRMS files from S3"
                            );
                        }
                    }
                }
            }

            if files.is_empty() {
                debug!(
                    model = %model.model.id,
                    lookback_minutes = lookback,
                    "No new MRMS files found"
                );
            } else {
                info!(
                    model = %model.model.id,
                    count = files.len(),
                    "Found MRMS files to download"
                );
            }
        } else if model.model.id.starts_with("goes") {
            // GOES-16 and GOES-18 satellite imagery
            // Files are available every 5-10 minutes
            // Format: OR_ABI-L2-CMIPC-M{mode}C{band:02}_G{satellite}_s{start}_e{end}_c{created}.nc

            let satellite_num = if model.model.id == "goes16" {
                "16"
            } else {
                "18"
            };

            // Check recent hours (lookback is in minutes, convert to hours)
            let hours_to_check = (lookback / 60).max(1);

            // Calculate max_results based on lookback period
            // GOES files are available every 5 minutes, so ~12 files per hour per band
            // We want to fetch all files within the lookback period
            let files_per_hour = 12; // ~5 minute intervals
            let max_results = (files_per_hour * hours_to_check as usize).max(20);

            // Get configured bands from source
            let bands = model.source.bands.clone().unwrap_or_else(|| vec![2, 8, 13]); // Default: Red, WV, IR

            for hours_ago in 0..hours_to_check {
                let check_time = now - ChronoDuration::hours(hours_ago as i64);
                let hour = check_time.hour();
                let check_doy = check_time.ordinal(); // Handle day boundary
                let check_year = check_time.year();

                for band in &bands {
                    // S3 path pattern: ABI-L2-CMIPC/{year}/{day_of_year:03}/{hour:02}/
                    let product = model.source.product.as_deref().unwrap_or("ABI-L2-CMIPC");

                    let prefix =
                        format!("{}/{}/{:03}/{:02}/", product, check_year, check_doy, hour);

                    // Use start_after to efficiently skip to files matching this band and satellite
                    // GOES filenames are lexicographically sorted by band (C01, C02, ..., C13, C14, C15, C16)
                    // and then by satellite (G16, G18) and timestamp
                    // Example: OR_ABI-L2-CMIPC-M6C13_G18_s20253570101170...
                    // We want to start just before our target band+satellite combo
                    let start_after_key = format!(
                        "{}OR_{}-M6C{:02}_G{}_",
                        prefix, product, band, satellite_num
                    );

                    info!(
                        model = %model.model.id,
                        prefix = %prefix,
                        band = band,
                        satellite = satellite_num,
                        start_after = %start_after_key,
                        max_results = max_results,
                        "Listing GOES files from S3 with start_after"
                    );

                    // List files from S3 using start_after to skip directly to target band
                    match self
                        .list_s3_files(
                            &model.source.bucket,
                            &prefix,
                            max_results,
                            Some(&start_after_key),
                        )
                        .await
                    {
                        Ok(keys) => {
                            // Filter for the specific band we want (start_after gets us close but we still need to verify)
                            // Filename format: OR_ABI-L2-CMIPC-M6C{band:02}_G{sat}_s{start}_e{end}_c{created}.nc
                            let band_str = format!("C{:02}", band);
                            let sat_str = format!("_G{}_", satellite_num);

                            let mut found_count = 0;
                            for key in keys {
                                if key.contains(&band_str)
                                    && key.contains(&sat_str)
                                    && key.ends_with(".nc")
                                {
                                    // Construct the full S3 URL
                                    let url = format!(
                                        "https://{}.s3.amazonaws.com/{}",
                                        model.source.bucket, key
                                    );

                                    // Extract timestamp from filename for output filename
                                    // Example: OR_ABI-L2-CMIPC-M6C02_G16_s20242350000000_e20242350009308_c20242350009356.nc
                                    let filename = key.split('/').next_back().unwrap_or(&key);
                                    let output_filename =
                                        format!("goes{}_{}", satellite_num, filename);

                                    debug!(
                                        model = %model.model.id,
                                        url = %url,
                                        output = %output_filename,
                                        "Found GOES file"
                                    );

                                    files.push((url, output_filename));
                                    found_count += 1;
                                }
                            }

                            info!(
                                model = %model.model.id,
                                band = band,
                                hour = hour,
                                found = found_count,
                                "GOES files found for band/hour"
                            );
                        }
                        Err(e) => {
                            warn!(
                                model = %model.model.id,
                                prefix = %prefix,
                                error = %e,
                                "Failed to list GOES files from S3"
                            );
                        }
                    }
                }
            }

            if files.is_empty() {
                debug!(
                    model = %model.model.id,
                    lookback_minutes = lookback,
                    "No new GOES files found"
                );
            } else {
                info!(
                    model = %model.model.id,
                    count = files.len(),
                    "Found GOES files to download"
                );
            }
        }

        Ok(files)
    }

    /// Check if a file exists via HEAD request.
    async fn check_file_exists(&self, url: &str) -> Result<bool> {
        let response = self
            .client
            .head(url)
            .send()
            .await
            .context("HEAD request failed")?;

        Ok(response.status().is_success())
    }

    /// List files from S3 bucket matching a prefix.
    /// Optionally uses start_after to efficiently skip to a specific lexicographic position.
    async fn list_s3_files(
        &self,
        bucket: &str,
        prefix: &str,
        max_results: usize,
        start_after: Option<&str>,
    ) -> Result<Vec<String>> {
        let s3_client = match &self.s3_client {
            Some(client) => client,
            None => {
                debug!("S3 client not initialized, skipping S3 listing");
                return Ok(Vec::new());
            }
        };

        let mut files = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let mut request = s3_client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(prefix)
                .max_keys(100);

            if let Some(ref token) = continuation_token {
                request = request.continuation_token(token.clone());
            }

            if let Some(start) = start_after {
                if continuation_token.is_none() {
                    // Only use start_after on the first request
                    request = request.start_after(start);
                }
            }

            let response = request.send().await.context("S3 list_objects_v2 failed")?;

            for object in response.contents() {
                if let Some(key) = object.key() {
                    files.push(key.to_string());
                    if files.len() >= max_results {
                        return Ok(files);
                    }
                }
            }

            if response.is_truncated() == Some(true) {
                continuation_token = response.next_continuation_token().map(|s| s.to_string());
            } else {
                break;
            }
        }

        Ok(files)
    }

    /// Calculate the most recent available model cycle.
    fn latest_available_cycle(&self, cycles: &[u32], delay_hours: u32) -> (String, u32) {
        let now = Utc::now() - ChronoDuration::hours(delay_hours as i64);
        let date = now.format("%Y%m%d").to_string();
        let current_hour = now.hour();

        // Find the most recent cycle that's available
        let cycle = cycles
            .iter()
            .filter(|&&c| c <= current_hour)
            .max()
            .copied()
            .unwrap_or_else(|| {
                // Use previous day's last cycle
                *cycles.last().unwrap_or(&0)
            });

        (date, cycle)
    }

    /// Trigger ingestion for completed downloads.
    async fn trigger_ingestion(&self) -> Result<()> {
        let pending = self.state.get_pending_ingestion().await?;

        if pending.is_empty() {
            return Ok(());
        }

        let ingester_url = match &self.ingester_url {
            Some(url) => url,
            None => {
                // No ingester URL configured, mark as ingested anyway
                for (url, _) in &pending {
                    self.state.mark_ingested(url).await?;
                }
                return Ok(());
            }
        };

        info!(
            count = pending.len(),
            "Triggering ingestion for completed downloads"
        );

        for (url, filename) in pending {
            // Call ingester API (INGESTER_URL should be the full endpoint, e.g., http://wms-api:8080/admin/ingest)
            let file_path = format!("/data/downloads/{}", filename);

            match self
                .client
                .post(ingester_url)
                .json(&serde_json::json!({
                    "file_path": file_path,
                    "source_url": url
                }))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    info!(file = %filename, "Ingestion triggered successfully");
                    self.state.mark_ingested(&url).await?;
                }
                Ok(response) => {
                    warn!(
                        file = %filename,
                        status = %response.status(),
                        "Ingestion failed"
                    );
                }
                Err(e) => {
                    error!(file = %filename, error = %e, "Failed to call ingester");
                }
            }
        }

        Ok(())
    }

    /// Default model configurations when YAML files aren't available.
    fn default_configs() -> Vec<ModelConfig> {
        // Return empty - configs should come from YAML files
        // In production, this would fail loudly if configs are missing
        warn!("Using default (empty) model configurations - no models will be downloaded");
        Vec::new()
    }
}
