//! Per-model download runner that operates independently with its own polling loop.
//!
//! Each model gets its own `ModelRunner` instance that:
//! - Runs on its own polling schedule
//! - Has guaranteed access to at least 1 download slot
//! - Downloads newest files first (priority ordering)
//! - Triggers ingestion immediately after each download

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Timelike, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::cleanup::delete_ingested_file;
use crate::concurrency::ModelDownloadPermit;
use crate::config::ModelConfig;
use crate::download::DownloadManager;
use crate::state::DownloadState;

/// File to download with optional timestamp for priority sorting.
#[derive(Debug, Clone)]
pub struct DownloadFile {
    pub url: String,
    pub filename: String,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Per-model download runner that operates independently.
pub struct ModelRunner {
    model: ModelConfig,
    download_manager: Arc<DownloadManager>,
    state: Arc<DownloadState>,
    permit: ModelDownloadPermit,
    ingester_url: Option<String>,
    client: Client,
    s3_client: Option<aws_sdk_s3::Client>,
    output_dir: PathBuf,
}

impl ModelRunner {
    /// Create a new model runner.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: ModelConfig,
        download_manager: Arc<DownloadManager>,
        state: Arc<DownloadState>,
        permit: ModelDownloadPermit,
        ingester_url: Option<String>,
        client: Client,
        s3_client: Option<aws_sdk_s3::Client>,
        output_dir: PathBuf,
    ) -> Self {
        Self {
            model,
            download_manager,
            state,
            permit,
            ingester_url,
            client,
            s3_client,
            output_dir,
        }
    }

    /// Get the model ID
    #[allow(dead_code)]
    pub fn model_id(&self) -> &str {
        &self.model.model.id
    }

    /// Run the model download loop forever until shutdown.
    pub async fn run_forever(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.model.schedule.poll_interval_secs);
        let model_id = &self.model.model.id;

        info!(
            model = %model_id,
            poll_interval_secs = self.model.schedule.poll_interval_secs,
            max_concurrent = self.permit.max_concurrent(),
            "Starting model runner"
        );

        // Run first cycle immediately
        if let Err(e) = self.run_cycle().await {
            error!(model = %model_id, error = %e, "Initial download cycle failed");
        }

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!(model = %model_id, "Shutting down model runner");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    debug!(model = %model_id, "Running scheduled download cycle");
                    if let Err(e) = self.run_cycle().await {
                        error!(model = %model_id, error = %e, "Download cycle failed");
                    }
                }
            }
        }

        Ok(())
    }

    /// Run a single download cycle.
    pub async fn run_cycle(&self) -> Result<()> {
        let model_id = &self.model.model.id;

        // 1. Discover available files
        let mut files = if self.model.is_observation() {
            self.discover_observation_files().await?
        } else {
            self.discover_forecast_files().await?
        };

        if files.is_empty() {
            debug!(model = %model_id, "No new files to download");
            return Ok(());
        }

        // 2. Sort by priority (newest first)
        self.sort_by_priority(&mut files);

        info!(
            model = %model_id,
            count = files.len(),
            "Found files to download (sorted by priority)"
        );

        // 3. Queue downloads and filter already downloaded
        // Note: queue_download uses INSERT OR IGNORE, making this idempotent.
        // If another model runner queued the same URL between our check and insert,
        // the duplicate insert is safely ignored.
        let mut pending = Vec::new();
        for file in files {
            if self.state.is_already_downloaded(&file.url).await? {
                debug!(url = %file.url, "Already downloaded, skipping");
                continue;
            }

            self.state
                .queue_download(&file.url, &file.filename, model_id)
                .await?;
            pending.push(file);
        }

        if pending.is_empty() {
            debug!(model = %model_id, "All files already downloaded");
            return Ok(());
        }

        info!(
            model = %model_id,
            count = pending.len(),
            "Downloading files"
        );

        // 4. Download with permit-based concurrency
        self.download_files(pending).await
    }

    /// Sort files by priority (newest first).
    /// Files with timestamps come before files without timestamps.
    /// Among files with timestamps, newer files come first.
    fn sort_by_priority(&self, files: &mut [DownloadFile]) {
        files.sort_by(|a, b| {
            match (&a.timestamp, &b.timestamp) {
                // Both have timestamps: newest first (reverse chronological)
                (Some(a_time), Some(b_time)) => b_time.cmp(a_time),
                // a has timestamp, b doesn't: a comes first
                (Some(_), None) => std::cmp::Ordering::Less,
                // b has timestamp, a doesn't: b comes first
                (None, Some(_)) => std::cmp::Ordering::Greater,
                // Neither has timestamp: maintain order
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    }

    /// Download files with permit-based concurrency control.
    async fn download_files(&self, files: Vec<DownloadFile>) -> Result<()> {
        let model_id = self.model.model.id.clone();
        let max_concurrent = self.permit.max_concurrent();

        let results = stream::iter(files)
            .map(|file| {
                let permit = self.permit.clone();
                let manager = self.download_manager.clone();
                let state = self.state.clone();
                let ingester_url = self.ingester_url.clone();
                let client = self.client.clone();
                let output_dir = self.output_dir.clone();
                let model_id = model_id.clone();

                async move {
                    // Acquire a download slot (guaranteed or shared)
                    let _slot = permit.acquire().await;

                    // Perform the download
                    match manager.download(&file.url, &file.filename, &state).await {
                        Ok(path) => {
                            info!(
                                model = %model_id,
                                url = %file.url,
                                path = %path.display(),
                                "Download complete"
                            );

                            // Trigger ingestion immediately
                            if let Some(ref url) = ingester_url {
                                let file_path = format!("/data/downloads/{}", file.filename);
                                match client
                                    .post(url)
                                    .json(&serde_json::json!({
                                        "file_path": file_path,
                                        "source_url": file.url
                                    }))
                                    .send()
                                    .await
                                {
                                    Ok(response) if response.status().is_success() => {
                                        info!(
                                            model = %model_id,
                                            file = %file.filename,
                                            "Ingestion triggered successfully"
                                        );
                                        let _ = state.mark_ingested(&file.url).await;
                                        // Delete source file after successful ingestion
                                        delete_ingested_file(&output_dir, &file.filename).await;
                                    }
                                    Ok(response) => {
                                        warn!(
                                            model = %model_id,
                                            file = %file.filename,
                                            status = %response.status(),
                                            "Ingestion request failed"
                                        );
                                    }
                                    Err(e) => {
                                        warn!(
                                            model = %model_id,
                                            file = %file.filename,
                                            error = %e,
                                            "Failed to trigger ingestion"
                                        );
                                    }
                                }
                            }

                            Ok(path)
                        }
                        Err(e) => {
                            error!(
                                model = %model_id,
                                url = %file.url,
                                error = %e,
                                "Download failed"
                            );
                            Err(e)
                        }
                    }
                }
            })
            .buffer_unordered(max_concurrent)
            .collect::<Vec<_>>()
            .await;

        let (successes, failures): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);

        // Log individual failure details
        for failure in &failures {
            if let Err(e) = failure {
                warn!(model = %model_id, error = %e, "Download failure detail");
            }
        }

        info!(
            model = %model_id,
            success = successes.len(),
            failed = failures.len(),
            "Download cycle complete"
        );

        Ok(())
    }

    // ========================================================================
    // File Discovery Methods
    // ========================================================================

    /// Discover forecast files available for download (GFS, HRRR, etc.).
    async fn discover_forecast_files(&self) -> Result<Vec<DownloadFile>> {
        let mut files = Vec::new();
        let model = &self.model;

        // Get the most recent available cycle
        let (date, cycle) =
            self.latest_available_cycle(&model.schedule.cycles, model.schedule.delay_hours);

        info!(
            model = %model.model.id,
            date = %date,
            cycle = cycle,
            "Checking for available forecast files"
        );

        // For forecast models, we want files in forecast hour order (f000 first)
        // since they represent the same model run, ordered by lead time
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

                    // For forecast files, we don't use timestamps for priority sorting.
                    // Files are discovered in forecast hour order (f000, f001, f002, ...)
                    // which is the desired download order (earliest forecasts first).
                    // The sort_by_priority function preserves order for files without timestamps.
                    files.push(DownloadFile {
                        url,
                        filename: output_filename,
                        timestamp: None,
                    });
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

    /// Discover observation files available for download (MRMS, GOES, NDFD, etc.).
    async fn discover_observation_files(&self) -> Result<Vec<DownloadFile>> {
        let model = &self.model;
        let lookback = model.lookback_minutes();
        let now = Utc::now();
        let earliest_time = now - ChronoDuration::minutes(lookback as i64);

        info!(
            model = %model.model.id,
            lookback_minutes = lookback,
            retention_hours = model.retention.hours,
            earliest_time = %earliest_time,
            "Checking for available observation files"
        );

        // Route to appropriate discovery method
        if model.source.source_type == "http" || model.model.id == "ndfd" {
            self.discover_ndfd_files().await
        } else if model.model.id == "mrms" {
            self.discover_mrms_files(now, earliest_time, lookback).await
        } else if model.model.id.starts_with("goes") {
            self.discover_goes_files(now, earliest_time, lookback).await
        } else {
            Ok(Vec::new())
        }
    }

    /// Discover NDFD files available for download.
    async fn discover_ndfd_files(&self) -> Result<Vec<DownloadFile>> {
        let mut files = Vec::new();
        let model = &self.model;

        let base_url = model
            .source
            .base_url
            .as_deref()
            .unwrap_or("https://tgftp.nws.noaa.gov");

        let prefix = &model.source.prefix_template;

        info!(
            model = %model.model.id,
            base_url = base_url,
            prefix = prefix,
            "Checking for available NDFD files"
        );

        for param in &model.parameters {
            let file_id = param
                .file
                .clone()
                .unwrap_or_else(|| param.name.to_lowercase());

            let url = format!("{}/{}/ds.{}.bin", base_url, prefix, file_id);

            match self.check_file_exists(&url).await {
                Ok(true) => {
                    let output_filename = format!("ndfd_{}.bin", file_id);
                    debug!(
                        model = %model.model.id,
                        parameter = %param.name,
                        url = %url,
                        output = %output_filename,
                        "Found NDFD file"
                    );
                    // NDFD files don't have timestamps in filenames
                    files.push(DownloadFile {
                        url,
                        filename: output_filename,
                        timestamp: None,
                    });
                }
                Ok(false) => {
                    debug!(
                        model = %model.model.id,
                        parameter = %param.name,
                        url = %url,
                        "NDFD file not available"
                    );
                }
                Err(e) => {
                    debug!(
                        model = %model.model.id,
                        parameter = %param.name,
                        url = %url,
                        error = %e,
                        "Error checking NDFD file"
                    );
                }
            }
        }

        if !files.is_empty() {
            info!(
                model = %model.model.id,
                count = files.len(),
                "Found NDFD files to download"
            );
        }

        Ok(files)
    }

    /// Discover MRMS files within the lookback period.
    async fn discover_mrms_files(
        &self,
        now: DateTime<Utc>,
        earliest_time: DateTime<Utc>,
        lookback: u32,
    ) -> Result<Vec<DownloadFile>> {
        let mut files = Vec::new();
        let model = &self.model;

        // Determine which dates we need to check
        let mut dates_to_check = Vec::new();
        let today = now.format("%Y%m%d").to_string();
        dates_to_check.push(today.clone());

        let earliest_date = earliest_time.format("%Y%m%d").to_string();
        if earliest_date != today {
            dates_to_check.push(earliest_date.clone());
        }

        info!(
            model = %model.model.id,
            dates = ?dates_to_check,
            "Checking MRMS date folders"
        );

        for param in &model.parameters {
            if let Some(product) = param.product.as_ref() {
                for date_str in &dates_to_check {
                    let prefix = format!("CONUS/{}/{}/", product, date_str);

                    let start_time = if date_str == &earliest_date {
                        earliest_time
                    } else {
                        now.date_naive()
                            .and_hms_opt(0, 0, 0)
                            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
                            .unwrap_or(earliest_time)
                    };

                    let start_after_key = format!(
                        "CONUS/{}/{}/MRMS_{}_{}",
                        product,
                        date_str,
                        product,
                        start_time.format("%Y%m%d-%H%M00")
                    );

                    let max_results = ((lookback / 2) as usize + 10).max(50);

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
                            for key in keys {
                                if key.ends_with(".grib2.gz") && key.contains(product) {
                                    if let Some(file_time) = Self::parse_mrms_timestamp(&key) {
                                        if file_time >= earliest_time && file_time <= now {
                                            let url = format!(
                                                "https://{}.s3.amazonaws.com/{}",
                                                model.source.bucket, key
                                            );

                                            let filename =
                                                key.split('/').next_back().unwrap_or(&key);
                                            let output_filename = format!("mrms_{}", filename);

                                            files.push(DownloadFile {
                                                url,
                                                filename: output_filename,
                                                timestamp: Some(file_time),
                                            });
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
        }

        if !files.is_empty() {
            info!(
                model = %model.model.id,
                count = files.len(),
                "Found MRMS files to download"
            );
        }

        Ok(files)
    }

    /// Parse timestamp from MRMS filename.
    /// Filename format: MRMS_{product}_{YYYYMMDD-HHMMSS}.grib2.gz
    fn parse_mrms_timestamp(key: &str) -> Option<DateTime<Utc>> {
        let filename = key.split('/').next_back()?;
        let timestamp_part = filename.split('_').next_back()?;
        let timestamp_str = timestamp_part.replace(".grib2.gz", "");
        let timestamp_clean = timestamp_str.replace('-', "");

        if timestamp_clean.len() >= 14 {
            let year: i32 = timestamp_clean[0..4].parse().ok()?;
            let month: u32 = timestamp_clean[4..6].parse().ok()?;
            let day: u32 = timestamp_clean[6..8].parse().ok()?;
            let hour: u32 = timestamp_clean[8..10].parse().ok()?;
            let minute: u32 = timestamp_clean[10..12].parse().ok()?;
            let second: u32 = timestamp_clean[12..14].parse().ok()?;

            let naive_dt = chrono::NaiveDate::from_ymd_opt(year, month, day)?
                .and_hms_opt(hour, minute, second)?;
            Some(DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc))
        } else {
            None
        }
    }

    /// Discover GOES files within the lookback period.
    async fn discover_goes_files(
        &self,
        now: DateTime<Utc>,
        earliest_time: DateTime<Utc>,
        lookback: u32,
    ) -> Result<Vec<DownloadFile>> {
        let mut files = Vec::new();
        let model = &self.model;

        let satellite_num = if model.model.id == "goes19" {
            "19"
        } else if model.model.id == "goes18" {
            "18"
        } else {
            "18" // Default to GOES-West
        };

        let hours_to_check = (lookback / 60) + 1;
        let files_per_hour = 12;
        let max_results = (files_per_hour * 2).max(24);

        let bands = model.source.bands.clone().unwrap_or_else(|| vec![2, 8, 13]);

        info!(
            model = %model.model.id,
            hours_to_check = hours_to_check,
            bands = ?bands,
            earliest_time = %earliest_time,
            "Checking GOES hour folders"
        );

        for hours_ago in 0..hours_to_check {
            let check_time = now - ChronoDuration::hours(hours_ago as i64);
            let hour = check_time.hour();
            let check_doy = check_time.ordinal();
            let check_year = check_time.year();

            for band in &bands {
                let product = model.source.product.as_deref().unwrap_or("ABI-L2-CMIPC");
                let prefix = format!("{}/{}/{:03}/{:02}/", product, check_year, check_doy, hour);

                let start_after_key = format!(
                    "{}OR_{}-M6C{:02}_G{}_",
                    prefix, product, band, satellite_num
                );

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
                        let band_str = format!("C{:02}", band);
                        let sat_str = format!("_G{}_", satellite_num);

                        for key in keys {
                            if key.contains(&band_str)
                                && key.contains(&sat_str)
                                && key.ends_with(".nc")
                            {
                                let file_time = Self::parse_goes_timestamp(&key);

                                // Only include files within the lookback window
                                let include = match file_time {
                                    Some(t) => t >= earliest_time && t <= now,
                                    None => true, // Include if we can't parse timestamp
                                };

                                if include {
                                    let url = format!(
                                        "https://{}.s3.amazonaws.com/{}",
                                        model.source.bucket, key
                                    );

                                    let filename = key.split('/').next_back().unwrap_or(&key);
                                    let output_filename =
                                        format!("goes{}_{}", satellite_num, filename);

                                    files.push(DownloadFile {
                                        url,
                                        filename: output_filename,
                                        timestamp: file_time,
                                    });
                                }
                            }
                        }
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

        if !files.is_empty() {
            info!(
                model = %model.model.id,
                count = files.len(),
                "Found GOES files to download"
            );
        }

        Ok(files)
    }

    /// Parse timestamp from GOES filename.
    /// Filename format: OR_ABI-L2-CMIPC-M6C{band}_G{sat}_s{start}_e{end}_c{created}.nc
    fn parse_goes_timestamp(key: &str) -> Option<DateTime<Utc>> {
        let filename = key.split('/').next_back()?;
        let s_idx = filename.find("_s")?;
        let timestamp_start = s_idx + 2;

        if filename.len() < timestamp_start + 14 {
            return None;
        }
        let timestamp_str = &filename[timestamp_start..timestamp_start + 13];

        let year: i32 = timestamp_str[0..4].parse().ok()?;
        let doy: u32 = timestamp_str[4..7].parse().ok()?;
        let hour: u32 = timestamp_str[7..9].parse().ok()?;
        let minute: u32 = timestamp_str[9..11].parse().ok()?;
        let second: u32 = timestamp_str[11..13].parse().ok()?;

        let naive_date = chrono::NaiveDate::from_yo_opt(year, doy)?;
        let naive_dt = naive_date.and_hms_opt(hour, minute, second)?;
        Some(DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc))
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Calculate the most recent available model cycle.
    fn latest_available_cycle(&self, cycles: &[u32], delay_hours: u32) -> (String, u32) {
        let now = Utc::now() - ChronoDuration::hours(delay_hours as i64);
        let date = now.format("%Y%m%d").to_string();
        let current_hour = now.hour();

        let cycle = cycles
            .iter()
            .filter(|&&c| c <= current_hour)
            .max()
            .copied()
            .unwrap_or_else(|| *cycles.last().unwrap_or(&0));

        (date, cycle)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mrms_timestamp() {
        let key =
            "CONUS/SeamlessHSR_00.00/20251202/MRMS_SeamlessHSR_00.00_20251202-175037.grib2.gz";
        let timestamp = ModelRunner::parse_mrms_timestamp(key);
        assert!(timestamp.is_some());

        let ts = timestamp.unwrap();
        assert_eq!(ts.year(), 2025);
        assert_eq!(ts.month(), 12);
        assert_eq!(ts.day(), 2);
        assert_eq!(ts.hour(), 17);
        assert_eq!(ts.minute(), 50);
        assert_eq!(ts.second(), 37);
    }

    #[test]
    fn test_parse_goes_timestamp() {
        let key = "ABI-L2-CMIPC/2025/357/01/OR_ABI-L2-CMIPC-M6C13_G18_s20253570101170_e20253570103543_c20253570104039.nc";
        let timestamp = ModelRunner::parse_goes_timestamp(key);
        assert!(timestamp.is_some());

        let ts = timestamp.unwrap();
        assert_eq!(ts.year(), 2025);
        // Day 357 of 2025
        assert_eq!(ts.hour(), 1);
        assert_eq!(ts.minute(), 1);
        assert_eq!(ts.second(), 17);
    }

    #[test]
    fn test_sort_by_priority() {
        let now = Utc::now();
        let mut files = vec![
            DownloadFile {
                url: "url1".to_string(),
                filename: "old.grib2".to_string(),
                timestamp: Some(now - ChronoDuration::hours(2)),
            },
            DownloadFile {
                url: "url2".to_string(),
                filename: "newest.grib2".to_string(),
                timestamp: Some(now),
            },
            DownloadFile {
                url: "url3".to_string(),
                filename: "middle.grib2".to_string(),
                timestamp: Some(now - ChronoDuration::hours(1)),
            },
            DownloadFile {
                url: "url4".to_string(),
                filename: "no_timestamp.grib2".to_string(),
                timestamp: None,
            },
        ];

        // Sort by priority (newest first)
        files.sort_by(|a, b| match (&a.timestamp, &b.timestamp) {
            (Some(a_time), Some(b_time)) => b_time.cmp(a_time),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        assert_eq!(files[0].filename, "newest.grib2");
        assert_eq!(files[1].filename, "middle.grib2");
        assert_eq!(files[2].filename, "old.grib2");
        assert_eq!(files[3].filename, "no_timestamp.grib2");
    }
}
