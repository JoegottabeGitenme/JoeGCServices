//! Resumable download manager with retry logic and progress tracking.
//!
//! Key features:
//! - HTTP Range requests for resumable downloads
//! - Exponential backoff retry on failures
//! - Progress tracking and persistence
//! - File integrity verification via Content-Length
//!
//! TODO need to test retry logic?

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use reqwest::{header, Client, Response, StatusCode};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, instrument, warn};

use crate::grib_index::{ByteRange, GribIndex, ParamFilter};
use crate::state::{DownloadState, DownloadStatus};

/// Configuration for the download manager.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial retry delay (doubles each retry)
    pub initial_retry_delay: Duration,
    /// Maximum retry delay
    pub max_retry_delay: Duration,
    /// HTTP request timeout
    pub request_timeout: Duration,
    /// Chunk size for streaming downloads (64KB default)
    pub chunk_size: usize,
    /// Directory for temporary download files
    pub temp_dir: PathBuf,
    /// Directory for completed downloads
    pub output_dir: PathBuf,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            initial_retry_delay: Duration::from_secs(2),
            max_retry_delay: Duration::from_secs(120),
            request_timeout: Duration::from_secs(600), // 10 minutes
            chunk_size: 64 * 1024,                     // 64KB
            temp_dir: PathBuf::from("/tmp/weather-downloads"),
            output_dir: PathBuf::from("/data/downloads"),
        }
    }
}

/// Download progress information.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub url: String,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub started_at: DateTime<Utc>,
    pub last_update: DateTime<Utc>,
    pub retry_count: u32,
}

impl DownloadProgress {
    pub fn percent_complete(&self) -> Option<f64> {
        self.total_bytes
            .map(|total| (self.downloaded_bytes as f64 / total as f64) * 100.0)
    }

    pub fn bytes_per_second(&self) -> f64 {
        let elapsed = (self.last_update - self.started_at).num_seconds() as f64;
        if elapsed > 0.0 {
            self.downloaded_bytes as f64 / elapsed
        } else {
            0.0
        }
    }
}

/// Manages downloads with resumption and retry support.
pub struct DownloadManager {
    client: Client,
    config: DownloadConfig,
}

impl DownloadManager {
    /// Create a new download manager with the given configuration.
    pub fn new(config: DownloadConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(4)
            .tcp_nodelay(true)
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, config })
    }

    /// Download a file with automatic retry and resumption.
    ///
    /// Returns the path to the completed download.
    ///
    /// # Arguments
    /// * `skip_size_validation` - If true, skip Content-Length validation after download.
    ///   Useful for sources like NDFD where files are updated in-place during download.
    #[instrument(skip(self, state), fields(url = %url))]
    pub async fn download(
        &self,
        url: &str,
        filename: &str,
        state: &DownloadState,
        skip_size_validation: bool,
    ) -> Result<PathBuf> {
        // Ensure directories exist
        fs::create_dir_all(&self.config.temp_dir).await?;
        fs::create_dir_all(&self.config.output_dir).await?;

        let temp_path = self.config.temp_dir.join(format!("{}.partial", filename));
        let final_path = self.config.output_dir.join(filename);

        // Check if already completed
        if final_path.exists() {
            info!(path = %final_path.display(), "File already exists, skipping download");
            // If not already in completed_downloads, add it
            if !state.is_already_downloaded(url).await? {
                // Get or create download state
                state.get_or_create(url, filename).await?;
                // Mark as completed in database so it gets ingested
                state.update_status(url, DownloadStatus::Completed).await?;
            }
            return Ok(final_path);
        }

        // Get or create download state
        let mut progress = state.get_or_create(url, filename).await?;

        // Check for partial download
        let resume_from = if temp_path.exists() {
            let metadata = fs::metadata(&temp_path).await?;
            metadata.len()
        } else {
            0
        };

        progress.downloaded_bytes = resume_from;
        progress.started_at = Utc::now();

        info!(
            url = %url,
            filename = %filename,
            resume_from = resume_from,
            "Starting download"
        );

        let mut retry_count = 0;
        let mut delay = self.config.initial_retry_delay;

        loop {
            match self
                .download_with_resume(url, &temp_path, &mut progress, state)
                .await
            {
                Ok(()) => {
                    // Verify and move to final location
                    if let Some(expected) = progress.total_bytes {
                        let actual = fs::metadata(&temp_path).await?.len();
                        if !skip_size_validation && actual != expected {
                            return Err(anyhow!(
                                "Download size mismatch: expected {} bytes, got {}",
                                expected,
                                actual
                            ));
                        }
                        // Log if sizes don't match but we're skipping validation
                        if skip_size_validation && actual != expected {
                            info!(
                                expected = expected,
                                actual = actual,
                                "Size mismatch ignored (skip_size_validation enabled)"
                            );
                        }
                    }

                    // Move to final location (use copy+delete for cross-filesystem support)
                    if let Err(_) = fs::rename(&temp_path, &final_path).await {
                        // rename failed (likely cross-device), fall back to copy+delete
                        fs::copy(&temp_path, &final_path).await?;
                        fs::remove_file(&temp_path).await?;
                    }

                    // Update state
                    state.update_status(url, DownloadStatus::Completed).await?;

                    info!(
                        path = %final_path.display(),
                        bytes = progress.downloaded_bytes,
                        "Download completed"
                    );

                    return Ok(final_path);
                }
                Err(e) => {
                    retry_count += 1;
                    progress.retry_count = retry_count;

                    if retry_count > self.config.max_retries {
                        state.update_status(url, DownloadStatus::Failed).await?;
                        return Err(anyhow!(
                            "Download failed after {} retries: {}",
                            retry_count,
                            e
                        ));
                    }

                    warn!(
                        error = %e,
                        retry = retry_count,
                        max_retries = self.config.max_retries,
                        delay_secs = delay.as_secs(),
                        "Download failed, retrying"
                    );

                    // Update state
                    state.update_status(url, DownloadStatus::Retrying).await?;
                    state.update_progress(url, &progress).await?;

                    // Wait before retry
                    tokio::time::sleep(delay).await;

                    // Exponential backoff
                    delay = std::cmp::min(delay * 2, self.config.max_retry_delay);
                }
            }
        }
    }

    /// Download with HTTP Range support for resumption.
    async fn download_with_resume(
        &self,
        url: &str,
        temp_path: &Path,
        progress: &mut DownloadProgress,
        state: &DownloadState,
    ) -> Result<()> {
        // First, get file size with HEAD request (if we don't have it)
        if progress.total_bytes.is_none() {
            progress.total_bytes = self.get_content_length(url).await?;
        }

        // Check if server supports range requests
        let supports_range = self.check_range_support(url).await.unwrap_or(false);

        // Loop to handle RANGE_NOT_SATISFIABLE retry without recursion
        loop {
            let resume_from = if temp_path.exists() {
                fs::metadata(temp_path).await?.len()
            } else {
                0
            };

            // If we already have all the bytes, we're done
            if let Some(total) = progress.total_bytes {
                if resume_from >= total {
                    progress.downloaded_bytes = total;
                    return Ok(());
                }
            }

            // Build request
            let mut request = self.client.get(url);

            if resume_from > 0 && supports_range {
                info!(
                    resume_from = resume_from,
                    total = ?progress.total_bytes,
                    "Resuming download"
                );
                request = request.header(header::RANGE, format!("bytes={}-", resume_from));
                progress.downloaded_bytes = resume_from;
            } else if resume_from > 0 {
                warn!("Server does not support range requests, restarting download");
                fs::remove_file(temp_path).await.ok();
                progress.downloaded_bytes = 0;
            }

            let response = request.send().await.context("HTTP request failed")?;

            // Check status
            match response.status() {
                StatusCode::OK => {
                    // Full content, start from scratch
                    if resume_from > 0 {
                        fs::remove_file(temp_path).await.ok();
                        progress.downloaded_bytes = 0;
                    }
                }
                StatusCode::PARTIAL_CONTENT => {
                    // Partial content, resuming
                    debug!("Received partial content, resuming download");
                }
                StatusCode::RANGE_NOT_SATISFIABLE => {
                    // We have more than what's available (possibly complete)
                    if let Some(total) = progress.total_bytes {
                        if resume_from >= total {
                            return Ok(());
                        }
                    }
                    // Otherwise, start over and retry
                    fs::remove_file(temp_path).await.ok();
                    progress.downloaded_bytes = 0;
                    continue; // Retry the loop
                }
                status => {
                    return Err(anyhow!("HTTP error: {}", status));
                }
            }

            // Update total size if not set
            if progress.total_bytes.is_none() {
                progress.total_bytes = response
                    .headers()
                    .get(header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse().ok());
            }

            // Stream to file
            return self
                .stream_to_file(response, temp_path, progress, state)
                .await;
        }
    }

    /// Stream response body to file with progress updates.
    async fn stream_to_file(
        &self,
        response: Response,
        path: &Path,
        progress: &mut DownloadProgress,
        state: &DownloadState,
    ) -> Result<()> {
        // Open file for appending
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .context("Failed to open output file")?;

        let mut stream = response.bytes_stream();
        let mut bytes_since_update = 0u64;
        let update_interval = 1_000_000; // Update state every 1MB

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Error reading response chunk")?;

            file.write_all(&chunk)
                .await
                .context("Error writing to file")?;

            progress.downloaded_bytes += chunk.len() as u64;
            progress.last_update = Utc::now();
            bytes_since_update += chunk.len() as u64;

            // Periodic state update
            if bytes_since_update >= update_interval {
                state.update_progress(&progress.url, progress).await?;
                bytes_since_update = 0;

                if let Some(percent) = progress.percent_complete() {
                    debug!(
                        downloaded = progress.downloaded_bytes,
                        total = ?progress.total_bytes,
                        percent = format!("{:.1}%", percent),
                        speed = format!("{:.1} KB/s", progress.bytes_per_second() / 1024.0),
                        "Download progress"
                    );
                }
            }
        }

        // Flush and sync
        file.flush().await?;
        file.sync_all().await?;

        Ok(())
    }

    /// Get content length with HEAD request.
    async fn get_content_length(&self, url: &str) -> Result<Option<u64>> {
        let response = self
            .client
            .head(url)
            .send()
            .await
            .context("HEAD request failed")?;

        if !response.status().is_success() {
            return Ok(None);
        }

        Ok(response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok()))
    }

    /// Check if server supports HTTP Range requests.
    async fn check_range_support(&self, url: &str) -> Result<bool> {
        let response = self
            .client
            .head(url)
            .send()
            .await
            .context("HEAD request failed")?;

        if !response.status().is_success() {
            return Ok(false);
        }

        // Check Accept-Ranges header
        if let Some(accept_ranges) = response.headers().get(header::ACCEPT_RANGES) {
            if let Ok(value) = accept_ranges.to_str() {
                return Ok(value != "none");
            }
        }

        // Assume support if header is missing (many servers don't send it)
        Ok(true)
    }

    // =========================================================================
    // Selective Download Methods (using .idx files)
    // =========================================================================

    /// Download only specific GRIB messages using an index file.
    ///
    /// This method:
    /// 1. Fetches the .idx index file
    /// 2. Parses it to find byte offsets for requested parameters
    /// 3. Downloads only those byte ranges
    /// 4. Concatenates them into a valid GRIB file
    ///
    /// Falls back to full download on any error.
    ///
    /// # Arguments
    /// * `url` - URL of the GRIB file
    /// * `filename` - Output filename
    /// * `index_suffix` - Suffix for index file (usually ".idx")
    /// * `filters` - Parameter/level filters to match
    /// * `state` - Download state tracker
    ///
    /// # Returns
    /// * `Ok(SelectiveDownloadResult::Success(path))` - Selective download succeeded
    /// * `Ok(SelectiveDownloadResult::Fallback(reason))` - Should fall back to full download
    /// * `Err(e)` - Fatal error (should not fall back)
    #[instrument(skip(self, state, filters), fields(url = %url))]
    pub async fn download_selective(
        &self,
        url: &str,
        filename: &str,
        index_suffix: &str,
        filters: &[ParamFilter],
        state: &DownloadState,
    ) -> Result<SelectiveDownloadResult> {
        // Ensure directories exist
        fs::create_dir_all(&self.config.temp_dir).await?;
        fs::create_dir_all(&self.config.output_dir).await?;

        let final_path = self.config.output_dir.join(filename);

        // Check if already completed
        if final_path.exists() {
            info!(path = %final_path.display(), "File already exists, skipping download");
            if !state.is_already_downloaded(url).await? {
                state.get_or_create(url, filename).await?;
                state.update_status(url, DownloadStatus::Completed).await?;
            }
            return Ok(SelectiveDownloadResult::Success(final_path));
        }

        // Build index URL
        let index_url = format!("{}{}", url, index_suffix);

        // Fetch index file
        let index_content = match self.fetch_index_file(&index_url).await {
            Ok(content) => content,
            Err(e) => {
                warn!(
                    index_url = %index_url,
                    error = %e,
                    "Failed to fetch index file, falling back to full download"
                );
                return Ok(SelectiveDownloadResult::Fallback(format!(
                    "Index fetch failed: {}",
                    e
                )));
            }
        };

        // Get file size for calculating last message range
        let file_size = self.get_content_length(url).await.unwrap_or(None);

        // Parse index
        let index = match GribIndex::parse(&index_content, file_size) {
            Ok(idx) => idx,
            Err(e) => {
                warn!(
                    error = %e,
                    "Failed to parse index file, falling back to full download"
                );
                return Ok(SelectiveDownloadResult::Fallback(format!(
                    "Index parse failed: {}",
                    e
                )));
            }
        };

        info!(
            index_entries = index.len(),
            filter_count = filters.len(),
            "Parsed index file"
        );

        // Find matching entries
        let matching = index.find_matching_entries(filters);

        if matching.is_empty() {
            warn!(
                filters = ?filters.iter().map(|f| format!("{}:{}", f.parameter, f.level)).collect::<Vec<_>>(),
                "No matching parameters found in index, falling back to full download"
            );
            return Ok(SelectiveDownloadResult::Fallback(
                "No matching parameters in index".to_string(),
            ));
        }

        // Check for missing filters and warn
        let mut found_filters = std::collections::HashSet::new();
        for entry in &matching {
            for filter in filters {
                if filter.matches(entry) {
                    found_filters.insert(format!("{}:{}", filter.parameter, filter.level));
                }
            }
        }

        let requested: std::collections::HashSet<_> = filters
            .iter()
            .map(|f| format!("{}:{}", f.parameter, f.level))
            .collect();

        let missing: Vec<_> = requested.difference(&found_filters).collect();
        if !missing.is_empty() {
            warn!(
                missing_params = ?missing,
                "Some requested parameters not found in index"
            );
        }

        // Calculate byte ranges
        let ranges = index.get_byte_ranges(filters);

        // Optionally merge nearby ranges to reduce HTTP requests
        // Using 0 threshold means only truly adjacent ranges are merged
        let merged_ranges = GribIndex::merge_ranges(ranges, 0);

        let total_selective_bytes: u64 = merged_ranges.iter().map(|r| r.size()).sum();

        info!(
            matched_messages = matching.len(),
            byte_ranges = merged_ranges.len(),
            selective_bytes = total_selective_bytes,
            full_file_bytes = ?file_size,
            savings_percent = file_size.map(|fs| format!("{:.1}%", (1.0 - total_selective_bytes as f64 / fs as f64) * 100.0)),
            "Calculated selective download ranges"
        );

        // Download byte ranges
        let temp_path = self.config.temp_dir.join(format!("{}.partial", filename));

        match self
            .download_byte_ranges(url, &merged_ranges, &temp_path)
            .await
        {
            Ok(bytes_downloaded) => {
                // Move to final location (use copy+delete for cross-filesystem support)
                if let Err(rename_err) = fs::rename(&temp_path, &final_path).await {
                    debug!(
                        error = %rename_err,
                        "rename failed (likely cross-device), falling back to copy"
                    );
                    fs::copy(&temp_path, &final_path).await?;
                    fs::remove_file(&temp_path).await?;
                }

                // Update state
                state.get_or_create(url, filename).await?;
                state.update_status(url, DownloadStatus::Completed).await?;

                info!(
                    path = %final_path.display(),
                    bytes = bytes_downloaded,
                    ranges = merged_ranges.len(),
                    "Selective download completed"
                );

                Ok(SelectiveDownloadResult::Success(final_path))
            }
            Err(e) => {
                // Clean up partial file
                fs::remove_file(&temp_path).await.ok();

                warn!(
                    error = %e,
                    "Byte range download failed, falling back to full download"
                );
                Ok(SelectiveDownloadResult::Fallback(format!(
                    "Range download failed: {}",
                    e
                )))
            }
        }
    }

    /// Fetch an index file (small text file, no resume needed).
    async fn fetch_index_file(&self, url: &str) -> Result<String> {
        let response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("Index file request failed")?;

        if !response.status().is_success() {
            return Err(anyhow!("Index file not found: HTTP {}", response.status()));
        }

        let content = response.text().await.context("Failed to read index file")?;

        if content.is_empty() {
            return Err(anyhow!("Index file is empty"));
        }

        debug!(
            url = %url,
            size = content.len(),
            "Fetched index file"
        );

        Ok(content)
    }

    /// Download multiple byte ranges and concatenate them.
    ///
    /// Downloads each range sequentially (S3 doesn't support multi-range requests).
    async fn download_byte_ranges(
        &self,
        url: &str,
        ranges: &[ByteRange],
        output_path: &Path,
    ) -> Result<u64> {
        // Create/truncate output file
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(output_path)
            .await
            .context("Failed to create output file")?;

        let mut total_bytes = 0u64;

        for (i, range) in ranges.iter().enumerate() {
            debug!(
                range_index = i,
                start = range.start,
                end = range.end,
                size = range.size(),
                "Downloading byte range"
            );

            let response = self
                .client
                .get(url)
                .header(header::RANGE, range.to_http_range())
                .send()
                .await
                .with_context(|| format!("Range request failed for range {}", i))?;

            match response.status() {
                StatusCode::PARTIAL_CONTENT => {
                    // Expected for range requests
                }
                StatusCode::OK => {
                    // Server ignored Range header, returned full content
                    // This is unexpected but we'll handle it
                    warn!(
                        range_index = i,
                        "Server returned full content instead of range, aborting selective download"
                    );
                    return Err(anyhow!("Server does not support range requests"));
                }
                status => {
                    return Err(anyhow!("HTTP error for range {}: {}", i, status));
                }
            }

            // Stream this range to file
            let bytes = response
                .bytes()
                .await
                .context("Failed to read range data")?;

            // Verify we got approximately the right amount
            // Allow 10% variance for potential Content-Range header differences
            let expected = range.size();
            let actual = bytes.len() as u64;
            let tolerance = expected / 10; // 10% tolerance
            let min_expected = expected.saturating_sub(tolerance);
            let max_expected = expected.saturating_add(tolerance);

            if actual < min_expected || actual > max_expected {
                // More than 10% deviation - this is suspicious
                if actual < expected / 2 || actual > expected * 2 {
                    // More than 2x deviation - likely corrupted or wrong data
                    return Err(anyhow!(
                        "Range {} size mismatch: expected {} bytes, got {} (>2x deviation)",
                        i,
                        expected,
                        actual
                    ));
                }
                // Between 10% and 2x - warn but continue
                warn!(
                    range_index = i,
                    expected = expected,
                    actual = actual,
                    deviation_percent = format!(
                        "{:.1}%",
                        ((actual as f64 / expected as f64) - 1.0).abs() * 100.0
                    ),
                    "Range size outside expected tolerance"
                );
            }

            file.write_all(&bytes)
                .await
                .context("Failed to write range data")?;

            total_bytes += actual;
        }

        // Flush and sync
        file.flush().await?;
        file.sync_all().await?;

        Ok(total_bytes)
    }
}

/// Result of a selective download attempt.
#[derive(Debug)]
#[must_use = "SelectiveDownloadResult must be handled - check for Fallback case"]
pub enum SelectiveDownloadResult {
    /// Download succeeded, contains path to downloaded file
    Success(PathBuf),
    /// Should fall back to full download, contains reason
    Fallback(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_percent() {
        let progress = DownloadProgress {
            url: "http://example.com/file".to_string(),
            total_bytes: Some(1000),
            downloaded_bytes: 500,
            started_at: Utc::now(),
            last_update: Utc::now(),
            retry_count: 0,
        };

        assert_eq!(progress.percent_complete(), Some(50.0));
    }

    #[test]
    fn test_progress_no_total() {
        let progress = DownloadProgress {
            url: "http://example.com/file".to_string(),
            total_bytes: None,
            downloaded_bytes: 500,
            started_at: Utc::now(),
            last_update: Utc::now(),
            retry_count: 0,
        };

        assert_eq!(progress.percent_complete(), None);
    }
}
