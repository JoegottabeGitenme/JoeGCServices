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
    #[instrument(skip(self, state), fields(url = %url))]
    pub async fn download(
        &self,
        url: &str,
        filename: &str,
        state: &DownloadState,
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
                        if actual != expected {
                            return Err(anyhow!(
                                "Download size mismatch: expected {} bytes, got {}",
                                expected,
                                actual
                            ));
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
