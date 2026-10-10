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
use crate::download::{DownloadManager, SelectiveDownloadResult};
use crate::grib_index::ParamFilter;
use crate::lis_runner;
use crate::state::DownloadState;

/// File to download with optional timestamp for priority sorting.
#[derive(Debug, Clone)]
pub struct DownloadFile {
    pub url: String,
    pub filename: String,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Earthdata authentication context for NASA GES DISC downloads (NLDAS, GLDAS, etc.).
#[derive(Clone)]
pub struct EarthdataAuth {
    pub client: Client,
    pub username: String,
    pub password: String,
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
    /// Optional Earthdata-authenticated client for NASA GES DISC sources (NLDAS).
    earthdata_auth: Option<EarthdataAuth>,
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
            earthdata_auth: None,
        }
    }

    /// Set the Earthdata authentication context for NASA GES DISC downloads.
    pub fn with_earthdata_auth(mut self, auth: EarthdataAuth) -> Self {
        self.earthdata_auth = Some(auth);
        self
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
        let always_redownload = self.model.source.always_redownload;

        // 0. For models with always_redownload enabled, expire old download records
        // This allows re-downloading files with static URLs (like NDFD)
        if always_redownload {
            let max_age_hours = self.model.retention.hours;
            if max_age_hours > 0 {
                let expired = self
                    .state
                    .expire_completed_downloads(model_id, max_age_hours)
                    .await?;
                if expired > 0 {
                    info!(
                        model = %model_id,
                        expired = expired,
                        max_age_hours = max_age_hours,
                        "Expired stale download records for re-download"
                    );
                }
            }
        }

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
        // For always_redownload models, skip the is_already_downloaded check.
        let mut pending = Vec::new();
        for file in files {
            if !always_redownload && self.state.is_already_downloaded(&file.url).await? {
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
        // Route LIS models (NLDAS, GLDAS) through Earthdata download path
        if self.is_lis() {
            self.download_nldas_files(pending).await
        } else {
            self.download_files(pending).await
        }
    }

    /// Check if this model uses NASA GES DISC (LIS) sources (NLDAS, GLDAS, etc.).
    fn is_lis(&self) -> bool {
        self.model.model.id.starts_with("nldas") || self.model.model.id.starts_with("gldas")
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

        // Build parameter filters for selective download if enabled
        let param_filters: Option<Vec<ParamFilter>> = if self.model.source.use_index_file {
            let filters = self.model.build_param_filters();
            if filters.is_empty() {
                warn!(
                    model = %model_id,
                    "use_index_file enabled but no parameter filters could be built, using full download"
                );
                None
            } else {
                info!(
                    model = %model_id,
                    filter_count = filters.len(),
                    "Selective download enabled"
                );
                Some(
                    filters
                        .into_iter()
                        .map(|(p, l)| ParamFilter::new(p, l))
                        .collect(),
                )
            }
        } else {
            None
        };

        let index_suffix = self.model.source.index_suffix.clone();
        let skip_size_validation = self.model.source.skip_size_validation;

        let results = stream::iter(files)
            .map(|file| {
                let permit = self.permit.clone();
                let manager = self.download_manager.clone();
                let state = self.state.clone();
                let ingester_url = self.ingester_url.clone();
                let client = self.client.clone();
                let output_dir = self.output_dir.clone();
                let model_id = model_id.clone();
                let param_filters = param_filters.clone();
                let index_suffix = index_suffix.clone();
                let skip_size_validation = skip_size_validation;

                async move {
                    // Acquire a download slot (guaranteed or shared)
                    let _slot = permit.acquire().await;

                    // Perform the download (selective or full)
                    let download_result = if let Some(ref filters) = param_filters {
                        // Try selective download first
                        // Note: skip_size_validation is captured from outer scope (self.model.source)
                        match manager
                            .download_selective(
                                &file.url,
                                &file.filename,
                                &index_suffix,
                                filters,
                                &state,
                            )
                            .await
                        {
                            Ok(SelectiveDownloadResult::Success(path)) => {
                                info!(
                                    model = %model_id,
                                    url = %file.url,
                                    "Selective download complete"
                                );
                                Ok(path)
                            }
                            Ok(SelectiveDownloadResult::Fallback(reason)) => {
                                info!(
                                    model = %model_id,
                                    url = %file.url,
                                    reason = %reason,
                                    "Falling back to full download"
                                );
                                manager
                                    .download(
                                        &file.url,
                                        &file.filename,
                                        &state,
                                        skip_size_validation,
                                    )
                                    .await
                            }
                            Err(e) => Err(e),
                        }
                    } else {
                        // Full download
                        manager
                            .download(&file.url, &file.filename, &state, skip_size_validation)
                            .await
                    };

                    match download_result {
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

    /// Discover forecast files available for download (GFS, HRRR, AIGFS, etc.).
    ///
    /// Supports both S3 and HTTP sources. For HTTP sources, set `source.type: http`
    /// or provide a `source.base_url`.
    ///
    /// Also supports `file_types` expansion for models like AIGFS that have multiple
    /// file types per forecast hour (e.g., "pres" and "sfc" files).
    async fn discover_forecast_files(&self) -> Result<Vec<DownloadFile>> {
        let mut files = Vec::new();
        let model = &self.model;

        // Get the most recent available cycle
        let (date, cycle) =
            self.latest_available_cycle(&model.schedule.cycles, model.schedule.delay_hours);

        // Determine if this is an HTTP source (vs S3)
        let is_http_source = model.source.source_type == "http" || model.source.base_url.is_some();

        // Get file types to expand (default to single empty string for no expansion)
        // This allows models like AIGFS to specify ["pres", "sfc"] to download
        // multiple file types per forecast hour
        let file_types: Vec<String> = model
            .source
            .file_types
            .clone()
            .unwrap_or_else(|| vec![String::new()]);

        info!(
            model = %model.model.id,
            date = %date,
            cycle = cycle,
            is_http = is_http_source,
            file_types = ?file_types,
            "Checking for available forecast files"
        );

        // For forecast models, we want files in forecast hour order (f000 first)
        // since they represent the same model run, ordered by lead time
        for forecast_hour in model.forecast_hours() {
            // Expand file_types if configured (e.g., ["pres", "sfc"] for AIGFS)
            for file_type in &file_types {
                let filename = model
                    .source
                    .file_pattern
                    .replace("{file_type}", file_type)
                    .replace("{cycle:02}", &format!("{:02}", cycle))
                    .replace("{forecast:03}", &format!("{:03}", forecast_hour))
                    .replace("{forecast:02}", &format!("{:02}", forecast_hour));

                let prefix = model
                    .source
                    .prefix_template
                    .replace("{date}", &date)
                    .replace("{cycle:02}", &format!("{:02}", cycle));

                // Build URL based on source type
                let url = if is_http_source {
                    let base_url = model
                        .source
                        .base_url
                        .as_deref()
                        .unwrap_or("https://nomads.ncep.noaa.gov");
                    format!("{}/{}/{}", base_url, prefix, filename)
                } else {
                    format!(
                        "https://{}.s3.amazonaws.com/{}/{}",
                        model.source.bucket, prefix, filename
                    )
                };

                // Check if file exists (HEAD request)
                match self.check_file_exists(&url).await {
                    Ok(true) => {
                        // Include file_type in output filename if present
                        let output_filename = if file_type.is_empty() {
                            format!(
                                "{}_{}_{:02}z_f{:03}.grib2",
                                model.model.id, date, cycle, forecast_hour
                            )
                        } else {
                            format!(
                                "{}_{}_{}_{:02}z_f{:03}.grib2",
                                model.model.id, file_type, date, cycle, forecast_hour
                            )
                        };

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
        if model.model.id.starts_with("nldas") || model.model.id.starts_with("gldas") {
            self.discover_nldas_files().await
        } else if model.source.source_type == "http" || model.model.id == "ndfd" {
            self.discover_ndfd_files().await
        } else if is_mrms_model(&model.model.id) {
            self.discover_mrms_files(now, earliest_time, lookback).await
        } else if model.model.id.starts_with("goes") {
            self.discover_goes_files(now, earliest_time, lookback).await
        } else {
            Ok(Vec::new())
        }
    }

    /// Discover LIS files (NLDAS/GLDAS) available for download from NASA GES DISC.
    ///
    /// Uses `lis_runner::build_lis_file_list()` to construct file URLs
    /// based on the configured delay and retention window.
    ///
    /// The data window is `[now - delay - retention, now - delay]`, so we pass
    /// `delay_hours + retention_hours` as the total lookback from now.
    async fn discover_nldas_files(&self) -> Result<Vec<DownloadFile>> {
        let model = &self.model;
        let now = Utc::now();

        // Data latency: configurable via delay_hours (96h for NLDAS, 792h for GLDAS EP)
        let delay_hours = model.schedule.delay_hours;
        let retention_hours = model.retention.hours;
        // The data window is [now - delay - retention, now - delay].
        // build_lis_file_list takes (delay, total_lookback_from_now), so:
        //   total_lookback = delay + retention
        //   earliest = now - total_lookback = now - delay - retention
        //   latest   = now - delay
        //
        // NLDAS:  delay=96h,  retention=720h → window = [now-816h, now-96h]  (720h of data)
        // GLDAS:  delay=792h, retention=720h → window = [now-1512h, now-792h] (720h of data)
        let total_lookback = delay_hours + retention_hours;

        info!(
            model = %model.model.id,
            delay_hours = delay_hours,
            retention_hours = retention_hours,
            total_lookback = total_lookback,
            "Discovering LIS files"
        );

        let files =
            lis_runner::build_lis_file_list(&model.model.id, now, delay_hours, total_lookback);

        if !files.is_empty() {
            info!(
                model = %model.model.id,
                count = files.len(),
                "Found LIS files to check"
            );
        }

        Ok(files)
    }

    /// Download NLDAS files using Earthdata authentication.
    ///
    /// This is a separate download path from the standard `download_files()` because
    /// NASA GES DISC requires OAuth2 redirect-based authentication that the standard
    /// `DownloadManager::download()` doesn't support.
    async fn download_nldas_files(&self, files: Vec<DownloadFile>) -> Result<()> {
        let model_id = self.model.model.id.clone();
        let max_concurrent = self.permit.max_concurrent();

        let earthdata_auth = match &self.earthdata_auth {
            Some(auth) => auth.clone(),
            None => {
                warn!(
                    model = %model_id,
                    "No Earthdata credentials configured, skipping NLDAS downloads"
                );
                return Ok(());
            }
        };

        let results = stream::iter(files)
            .map(|file| {
                let permit = self.permit.clone();
                let state = self.state.clone();
                let ingester_url = self.ingester_url.clone();
                let http_client = self.client.clone();
                let output_dir = self.output_dir.clone();
                let model_id = model_id.clone();
                let auth = earthdata_auth.clone();

                async move {
                    // Acquire a download slot
                    let _slot = permit.acquire().await;

                    let output_path = output_dir.join(&file.filename);

                    // Skip if file already exists on disk
                    if output_path.exists() {
                        debug!(
                            model = %model_id,
                            file = %file.filename,
                            "File already exists on disk, skipping"
                        );
                        return Ok(output_path);
                    }

                    // Download via Earthdata OAuth
                    match lis_runner::download_earthdata_file(
                        &auth.client,
                        &auth.username,
                        &auth.password,
                        &file.url,
                        &output_path,
                    )
                    .await
                    {
                        Ok(file_size) => {
                            info!(
                                model = %model_id,
                                url = %file.url,
                                size = file_size,
                                path = %output_path.display(),
                                "NLDAS download complete"
                            );

                            // Mark as completed in state DB
                            let _ = state
                                .queue_download(&file.url, &file.filename, &model_id)
                                .await;
                            let _ = state
                                .update_status(&file.url, crate::state::DownloadStatus::Completed)
                                .await;

                            // Trigger ingestion
                            if let Some(ref url) = ingester_url {
                                let file_path = format!("/data/downloads/{}", file.filename);
                                match http_client
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

                            Ok(output_path)
                        }
                        Err(e) => {
                            error!(
                                model = %model_id,
                                url = %file.url,
                                error = %e,
                                "NLDAS download failed"
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

        for failure in &failures {
            if let Err(e) = failure {
                warn!(model = %model_id, error = %e, "NLDAS download failure detail");
            }
        }

        info!(
            model = %model_id,
            success = successes.len(),
            failed = failures.len(),
            "NLDAS download cycle complete"
        );

        Ok(())
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
    ///
    /// For each parameter, lists its `product` over every UTC date the window touches and,
    /// when the parameter names a `fallback_product`, lists that too and adds its files for
    /// hours the primary product is still missing (see `select_mrms_files`).
    async fn discover_mrms_files(
        &self,
        now: DateTime<Utc>,
        earliest_time: DateTime<Utc>,
        lookback: u32,
    ) -> Result<Vec<DownloadFile>> {
        let mut files = Vec::new();
        let model = &self.model;

        let dates_to_check = mrms_dates_to_check(now, earliest_time);
        info!(
            model = %model.model.id,
            dates = ?dates_to_check,
            "Checking MRMS date folders"
        );

        // S3 pages at 1000 keys regardless; this is only the cap on what we keep.
        let max_results = ((lookback / 2) as usize + 10).clamp(50, 5000);

        for param in &model.parameters {
            let Some(product) = param.product.as_ref() else {
                continue;
            };

            let primary = self
                .list_mrms_product(product, &dates_to_check, now, earliest_time, max_results)
                .await;
            let fallback = match param.fallback_product.as_ref() {
                Some(fb) => {
                    self.list_mrms_product(fb, &dates_to_check, now, earliest_time, max_results)
                        .await
                }
                None => Vec::new(),
            };

            let (primary_files, fallback_files) = select_mrms_files(primary, fallback, now);
            if !fallback_files.is_empty() {
                info!(
                    model = %model.model.id,
                    parameter = %param.name,
                    count = fallback_files.len(),
                    "Using fallback product for hours missing from the primary product"
                );
            }
            for candidate in primary_files.into_iter().chain(fallback_files) {
                let filename = candidate
                    .key
                    .split('/')
                    .next_back()
                    .unwrap_or(&candidate.key);
                files.push(DownloadFile {
                    url: format!(
                        "https://{}.s3.amazonaws.com/{}",
                        model.source.bucket, candidate.key
                    ),
                    // Prefixed with the downloader model id: the ingester derives the dataset
                    // model from the filename, and `mrms-qpe_` must not read as `mrms_`.
                    filename: mrms_output_filename(&model.model.id, filename),
                    timestamp: Some(candidate.time),
                });
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

    /// List one MRMS product's files in `[earliest_time, now]` across `dates`.
    /// A listing failure is logged and yields nothing for that date, never an error:
    /// the next poll retries.
    async fn list_mrms_product(
        &self,
        product: &str,
        dates: &[String],
        now: DateTime<Utc>,
        earliest_time: DateTime<Utc>,
        max_results: usize,
    ) -> Vec<MrmsCandidate> {
        let model = &self.model;
        let mut out = Vec::new();
        for date_str in dates {
            let prefix = format!("CONUS/{}/{}/", product, date_str);
            let start_after = mrms_start_after_key(product, date_str, earliest_time);

            match self
                .list_s3_files(
                    &model.source.bucket,
                    &prefix,
                    max_results,
                    start_after.as_deref(),
                )
                .await
            {
                Ok(keys) => {
                    for key in keys {
                        if !(key.ends_with(".grib2.gz") && key.contains(product)) {
                            continue;
                        }
                        if let Some(time) = Self::parse_mrms_timestamp(&key) {
                            if time >= earliest_time && time <= now {
                                out.push(MrmsCandidate { key, time });
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
        out
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

        let satellite_num = if model.model.id == "goes19" || model.model.id == "goes19-fulldisk" {
            "19"
        } else if model.model.id == "goes18" || model.model.id == "goes18-fulldisk" {
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

// ============================================================================
// MRMS discovery helpers (pure, so the rules are testable without S3)
// ============================================================================

/// How long after its nominal time a primary-product file may be missing before the
/// fallback product is used for that hour. The Pass2 QPE files normally land about an
/// hour after their nominal time, so 90 minutes means "late, not just on schedule";
/// waiting that long also keeps us from ingesting a Pass1 grid a moment before the
/// better Pass2 grid for the same hour appears.
pub const MRMS_FALLBACK_GRACE_MINUTES: i64 = 90;

/// A file found in an MRMS S3 listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrmsCandidate {
    pub key: String,
    pub time: DateTime<Utc>,
}

/// `mrms` and any `mrms-*` model (e.g. `mrms-qpe`) are discovered the same way.
pub fn is_mrms_model(id: &str) -> bool {
    id == "mrms" || id.starts_with("mrms-")
}

/// Local filename for a downloaded MRMS file: `{model id}_{upstream filename}`.
/// For `mrms` this is the historical `mrms_MRMS_...`; for `mrms-qpe` it is
/// `mrms-qpe_MRMS_...`, which the ingester maps to the `mrms-qpe` model.
pub fn mrms_output_filename(model_id: &str, upstream_filename: &str) -> String {
    format!("{}_{}", model_id, upstream_filename)
}

/// Every UTC date folder (`YYYYMMDD`) from the day of `earliest` through the day of
/// `now`, oldest first.
///
/// This used to be just "today and the earliest day", which is only right while the
/// window spans at most two days. A 72-hour window touches four dates and the two in
/// between were silently never listed.
pub fn mrms_dates_to_check(now: DateTime<Utc>, earliest: DateTime<Utc>) -> Vec<String> {
    let mut dates = Vec::new();
    let mut day = earliest.date_naive();
    let last = now.date_naive();
    while day <= last {
        dates.push(day.format("%Y%m%d").to_string());
        day = match day.succ_opt() {
            Some(d) => d,
            None => break,
        };
    }
    dates
}

/// S3 `StartAfter` key for one product/date folder, or `None` to list the whole folder.
///
/// Only the folder containing `earliest_time` needs a start point. `StartAfter` is
/// exclusive, so the key is built from one second BEFORE `earliest_time`: a file stamped
/// exactly `earliest_time` is kept (it is inside the window). Every other date lists the
/// full folder, which also keeps its `HH0000`-stamped midnight file; the previous code
/// started those folders at `...-000000`, which excluded exactly that file and dropped
/// the 00:00 hour of every day from an hourly product.
pub fn mrms_start_after_key(
    product: &str,
    date_str: &str,
    earliest_time: DateTime<Utc>,
) -> Option<String> {
    if earliest_time.format("%Y%m%d").to_string() != date_str {
        return None;
    }
    let just_before = earliest_time - ChronoDuration::seconds(1);
    Some(format!(
        "CONUS/{}/{}/MRMS_{}_{}",
        product,
        date_str,
        product,
        just_before.format("%Y%m%d-%H%M%S")
    ))
}

/// Choose which listed files to download for one parameter: every `primary` file, plus
/// each `fallback` file whose time has no primary file and is at least
/// `MRMS_FALLBACK_GRACE_MINUTES` old. Both lists are returned sorted oldest first with
/// duplicates (the same key listed twice) removed.
pub fn select_mrms_files(
    primary: Vec<MrmsCandidate>,
    fallback: Vec<MrmsCandidate>,
    now: DateTime<Utc>,
) -> (Vec<MrmsCandidate>, Vec<MrmsCandidate>) {
    let dedup_sorted = |mut v: Vec<MrmsCandidate>| {
        v.sort_by(|a, b| (a.time, &a.key).cmp(&(b.time, &b.key)));
        v.dedup_by(|a, b| a.key == b.key);
        v
    };
    let primary = dedup_sorted(primary);
    let have: std::collections::HashSet<DateTime<Utc>> = primary.iter().map(|c| c.time).collect();
    let cutoff = now - ChronoDuration::minutes(MRMS_FALLBACK_GRACE_MINUTES);
    let fallback = dedup_sorted(
        fallback
            .into_iter()
            .filter(|c| !have.contains(&c.time) && c.time <= cutoff)
            .collect(),
    );
    (primary, fallback)
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

    // =========================================================================
    // MRMS timestamp parsing edge cases
    // =========================================================================

    #[test]
    fn test_parse_mrms_timestamp_different_products() {
        // Test with different MRMS products
        let key1 = "CONUS/MergedReflectivityQC_00.50/20251215/MRMS_MergedReflectivityQC_00.50_20251215-120000.grib2.gz";
        let ts1 = ModelRunner::parse_mrms_timestamp(key1).unwrap();
        assert_eq!(ts1.year(), 2025);
        assert_eq!(ts1.month(), 12);
        assert_eq!(ts1.day(), 15);
        assert_eq!(ts1.hour(), 12);
        assert_eq!(ts1.minute(), 0);
        assert_eq!(ts1.second(), 0);
    }

    #[test]
    fn test_parse_mrms_timestamp_invalid() {
        // Invalid timestamp - too short
        let key = "CONUS/SeamlessHSR_00.00/20251202/MRMS_SeamlessHSR_00.00_short.grib2.gz";
        let ts = ModelRunner::parse_mrms_timestamp(key);
        assert!(ts.is_none());
    }

    #[test]
    fn test_parse_mrms_timestamp_no_underscore() {
        // Key without proper underscore pattern
        let key = "some/random/path.grib2.gz";
        let ts = ModelRunner::parse_mrms_timestamp(key);
        assert!(ts.is_none());
    }

    // =========================================================================
    // GOES timestamp parsing edge cases
    // =========================================================================

    #[test]
    fn test_parse_goes_timestamp_different_bands() {
        // Test band 02 (visible)
        let key1 = "ABI-L2-CMIPC/2025/001/12/OR_ABI-L2-CMIPC-M6C02_G18_s20250011200000_e20250011202373_c20250011202449.nc";
        let ts1 = ModelRunner::parse_goes_timestamp(key1).unwrap();
        assert_eq!(ts1.year(), 2025);
        assert_eq!(ts1.hour(), 12);
        assert_eq!(ts1.minute(), 0);
        assert_eq!(ts1.second(), 0);

        // Test band 13 (IR)
        let key2 = "ABI-L2-CMIPC/2025/365/23/OR_ABI-L2-CMIPC-M6C13_G19_s20253652359450_e20253652359599_c20253652359599.nc";
        let ts2 = ModelRunner::parse_goes_timestamp(key2).unwrap();
        assert_eq!(ts2.year(), 2025);
        assert_eq!(ts2.hour(), 23);
        assert_eq!(ts2.minute(), 59);
        assert_eq!(ts2.second(), 45);
    }

    #[test]
    fn test_parse_goes_timestamp_invalid_no_s() {
        // Missing _s marker
        let key = "ABI-L2-CMIPC/2025/001/12/OR_ABI-L2-CMIPC-M6C02_G18_20250011200000.nc";
        let ts = ModelRunner::parse_goes_timestamp(key);
        assert!(ts.is_none());
    }

    #[test]
    fn test_parse_goes_timestamp_truncated() {
        // Truncated after _s
        let key = "ABI-L2-CMIPC/2025/001/12/OR_ABI-L2-CMIPC-M6C02_G18_s2025.nc";
        let ts = ModelRunner::parse_goes_timestamp(key);
        assert!(ts.is_none());
    }

    #[test]
    fn test_parse_goes_timestamp_fulldisk() {
        // Full disk product (different prefix)
        let key = "ABI-L2-CMIPF/2025/100/06/OR_ABI-L2-CMIPF-M6C02_G18_s20251000600000_e20251000609599_c20251000609599.nc";
        let ts = ModelRunner::parse_goes_timestamp(key).unwrap();
        assert_eq!(ts.year(), 2025);
        // Day 100
        assert_eq!(ts.hour(), 6);
        assert_eq!(ts.minute(), 0);
    }

    // =========================================================================
    // DownloadFile struct tests
    // =========================================================================

    #[test]
    fn test_download_file_struct() {
        let file = DownloadFile {
            url: "https://example.com/data.grib2".to_string(),
            filename: "data.grib2".to_string(),
            timestamp: Some(Utc::now()),
        };
        assert!(file.url.starts_with("https://"));
        assert!(file.timestamp.is_some());
    }

    #[test]
    fn test_download_file_without_timestamp() {
        let file = DownloadFile {
            url: "https://example.com/data.grib2".to_string(),
            filename: "data.grib2".to_string(),
            timestamp: None,
        };
        assert!(file.timestamp.is_none());
    }
    // ------------------------------------------------------------------
    // MRMS discovery rules
    // ------------------------------------------------------------------

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    fn cand(h: u32, d: u32) -> MrmsCandidate {
        let t = utc(2026, 10, d, h, 0, 0);
        MrmsCandidate {
            key: format!("CONUS/P/2026100{d}/MRMS_P_2026100{d}-{h:02}0000.grib2.gz"),
            time: t,
        }
    }

    #[test]
    fn mrms_models_are_recognised_by_id() {
        assert!(is_mrms_model("mrms"));
        assert!(is_mrms_model("mrms-qpe"));
        assert!(!is_mrms_model("mrmsx"));
        assert!(!is_mrms_model("hrrr"));
        assert!(!is_mrms_model("nbm-conus"));
    }

    #[test]
    fn output_filename_keeps_mrms_and_distinguishes_mrms_qpe() {
        let up = "MRMS_MultiSensor_QPE_01H_Pass2_00.00_20261009-150000.grib2.gz";
        assert_eq!(mrms_output_filename("mrms", up), format!("mrms_{up}"));
        assert_eq!(
            mrms_output_filename("mrms-qpe", up),
            format!("mrms-qpe_{up}")
        );
    }

    #[test]
    fn a_two_hour_window_checks_today_and_yesterday_only_when_it_crosses_midnight() {
        let now = utc(2026, 10, 9, 15, 30, 0);
        assert_eq!(
            mrms_dates_to_check(now, now - ChronoDuration::hours(2)),
            ["20261009"]
        );
        let now = utc(2026, 10, 9, 0, 40, 0);
        assert_eq!(
            mrms_dates_to_check(now, now - ChronoDuration::hours(2)),
            ["20261008", "20261009"]
        );
    }

    #[test]
    fn a_seventy_two_hour_window_checks_every_date_in_between() {
        // The old code listed only "today" and "the earliest day": the two middle days
        // of a 72 h window were never looked at.
        let now = utc(2026, 10, 10, 0, 40, 0);
        assert_eq!(
            mrms_dates_to_check(now, now - ChronoDuration::hours(72)),
            ["20261007", "20261008", "20261009", "20261010"]
        );
        // month and year boundaries
        let now = utc(2027, 1, 2, 3, 0, 0);
        assert_eq!(
            mrms_dates_to_check(now, now - ChronoDuration::hours(72)),
            ["20261230", "20261231", "20270101", "20270102"]
        );
    }

    #[test]
    fn only_the_earliest_folder_gets_a_start_after_key() {
        let earliest = utc(2026, 10, 7, 0, 40, 0);
        let key = mrms_start_after_key("PROD", "20261007", earliest).unwrap();
        // one second before the window start, with seconds (StartAfter is exclusive)
        assert_eq!(key, "CONUS/PROD/20261007/MRMS_PROD_20261007-003959");
        for later in ["20261008", "20261009", "20261010"] {
            assert_eq!(
                mrms_start_after_key("PROD", later, earliest),
                None,
                "{later}"
            );
        }
    }

    #[test]
    fn the_start_after_key_sorts_before_a_file_stamped_exactly_at_the_window_start() {
        let earliest = utc(2026, 10, 7, 1, 0, 0);
        let key = mrms_start_after_key("PROD", "20261007", earliest).unwrap();
        let file_at_start = "CONUS/PROD/20261007/MRMS_PROD_20261007-010000.grib2.gz";
        assert!(
            key.as_str() < file_at_start,
            "{key} must precede {file_at_start}"
        );
        // and after the previous hour's file
        assert!(key.as_str() > "CONUS/PROD/20261007/MRMS_PROD_20261007-000000.grib2.gz");
    }

    #[test]
    fn midnight_files_of_full_day_folders_are_not_excluded() {
        // The previous start key for a full day was `...-000000`, which equals the
        // midnight file's own stamp and (exclusive StartAfter) skipped it.
        let earliest = utc(2026, 10, 7, 12, 0, 0);
        assert_eq!(mrms_start_after_key("PROD", "20261008", earliest), None);
    }

    #[test]
    fn fallback_fills_only_hours_the_primary_is_missing_and_old_enough() {
        let now = utc(2026, 10, 9, 20, 0, 0);
        let primary = vec![cand(10, 9), cand(12, 9), cand(18, 9)];
        // fallback has every hour 10..=19
        let fallback: Vec<_> = (10..=19).map(|h| cand(h, 9)).collect();
        let (p, f) = select_mrms_files(primary, fallback, now);
        assert_eq!(p.len(), 3);
        let hours: Vec<u32> = f.iter().map(|c| chrono::Timelike::hour(&c.time)).collect();
        // missing from primary: 11, 13..=17, 19. 19:00 is only 60 min old (< 90) -> wait.
        assert_eq!(hours, [11, 13, 14, 15, 16, 17]);
    }

    #[test]
    fn the_grace_period_is_ninety_minutes_exactly() {
        let t = utc(2026, 10, 9, 15, 0, 0);
        let fb = vec![MrmsCandidate {
            key: "k".into(),
            time: t,
        }];
        let just_inside = t + ChronoDuration::minutes(89);
        let at_limit = t + ChronoDuration::minutes(90);
        assert!(select_mrms_files(vec![], fb.clone(), just_inside)
            .1
            .is_empty());
        assert_eq!(select_mrms_files(vec![], fb, at_limit).1.len(), 1);
    }

    #[test]
    fn without_a_fallback_nothing_changes_and_output_is_sorted_and_deduplicated() {
        let now = utc(2026, 10, 9, 20, 0, 0);
        let (p, f) = select_mrms_files(
            vec![cand(12, 9), cand(10, 9), cand(12, 9), cand(11, 9)],
            vec![],
            now,
        );
        assert!(f.is_empty());
        let hours: Vec<u32> = p.iter().map(|c| chrono::Timelike::hour(&c.time)).collect();
        assert_eq!(hours, [10, 11, 12]);
    }

    #[test]
    fn a_late_primary_file_wins_over_a_fallback_for_the_same_hour() {
        // Pass2 for 15:00 shows up late (at 17:00): the fallback for 15:00 is no longer chosen.
        let now = utc(2026, 10, 9, 17, 0, 0);
        let (p, f) = select_mrms_files(vec![cand(15, 9)], vec![cand(15, 9)], now);
        assert_eq!(p.len(), 1);
        assert!(f.is_empty());
    }
}
