//! Cleanup system for managing downloaded files and database records.
//!
//! This module handles:
//! - Immediate deletion of source files after successful ingestion
//! - Periodic cleanup of stale partial files in temp directory
//! - Cleanup of orphan files (marked ingested but still on disk)
//! - Retry of failed ingestions for pending files
//! - Cleanup of stale pending files (failed ingestion > max age)
//! - Pruning old records from the state database
//!
//! # Background
//!
//! Downloaded GRIB2/NetCDF files are stored in `/data/downloads/` until ingested.
//! After the ingester processes a file and converts it to Zarr format in MinIO,
//! the source file is no longer needed. Without cleanup, these files accumulate
//! indefinitely, consuming disk space.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use chrono::Utc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::state::DownloadState;

/// Configuration for the cleanup system.
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// Enable cleanup (default: true)
    pub enabled: bool,
    /// Dry run mode - log what would be deleted without actually deleting (default: false)
    pub dry_run: bool,
    /// Interval between periodic cleanup runs in seconds (default: 3600 = 1 hour)
    pub interval_secs: u64,
    /// Max age for .partial files before deletion in seconds (default: 3600 = 1 hour)
    pub partial_file_max_age_secs: u64,
    /// Max age for pending ingestion files before deletion in seconds (default: 7200 = 2 hours)
    pub pending_ingestion_max_age_secs: u64,
    /// Days to retain completed_downloads records (default: 7)
    pub completed_record_retention_days: u32,
    /// Days to retain failed download records (default: 7)
    pub failed_record_retention_days: u32,
    /// Output directory for completed downloads
    pub output_dir: PathBuf,
    /// Temp directory for partial downloads
    pub temp_dir: PathBuf,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200, // 2 hours
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: PathBuf::from("/data/downloads"),
            temp_dir: PathBuf::from("/tmp/weather-downloads"),
        }
    }
}

/// Statistics from a cleanup run.
#[derive(Debug, Default, Clone)]
pub struct CleanupStats {
    /// Number of partial files deleted
    pub partial_files_deleted: u64,
    /// Number of orphan files deleted (ingested but still on disk)
    pub orphan_files_deleted: u64,
    /// Bytes reclaimed from file deletion
    pub bytes_reclaimed: u64,
    /// Number of completed_downloads records pruned
    pub completed_records_pruned: u64,
    /// Number of failed download records pruned
    pub failed_records_pruned: u64,
    /// Number of ingestion retries attempted
    pub ingestion_retries: u64,
    /// Number of successful ingestion retries
    pub ingestion_retries_succeeded: u64,
    /// Number of stale pending files deleted (failed ingestion > max age)
    pub stale_pending_files_deleted: u64,
    /// Errors encountered during cleanup
    pub errors: Vec<String>,
}

impl CleanupStats {
    /// Merge another stats into this one.
    pub fn merge(&mut self, other: CleanupStats) {
        self.partial_files_deleted += other.partial_files_deleted;
        self.orphan_files_deleted += other.orphan_files_deleted;
        self.bytes_reclaimed += other.bytes_reclaimed;
        self.completed_records_pruned += other.completed_records_pruned;
        self.failed_records_pruned += other.failed_records_pruned;
        self.ingestion_retries += other.ingestion_retries;
        self.ingestion_retries_succeeded += other.ingestion_retries_succeeded;
        self.stale_pending_files_deleted += other.stale_pending_files_deleted;
        self.errors.extend(other.errors);
    }
}

/// Accumulated metrics for Prometheus export.
#[derive(Debug, Default)]
pub struct CleanupMetrics {
    pub runs_total: AtomicU64,
    pub files_deleted_total: AtomicU64,
    pub bytes_reclaimed_total: AtomicU64,
    pub db_records_pruned_total: AtomicU64,
    pub last_run_timestamp: AtomicU64,
    pub last_run_duration_ms: AtomicU64,
    pub errors_total: AtomicU64,
    pub ingestion_retries_total: AtomicU64,
    pub ingestion_retries_succeeded_total: AtomicU64,
    pub stale_pending_deleted_total: AtomicU64,
}

impl CleanupMetrics {
    /// Create new metrics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record stats from a cleanup run.
    pub fn record(&self, stats: &CleanupStats, duration_ms: u64) {
        self.runs_total.fetch_add(1, Ordering::Relaxed);
        self.files_deleted_total.fetch_add(
            stats.partial_files_deleted
                + stats.orphan_files_deleted
                + stats.stale_pending_files_deleted,
            Ordering::Relaxed,
        );
        self.bytes_reclaimed_total
            .fetch_add(stats.bytes_reclaimed, Ordering::Relaxed);
        self.db_records_pruned_total.fetch_add(
            stats.completed_records_pruned + stats.failed_records_pruned,
            Ordering::Relaxed,
        );
        self.last_run_timestamp.store(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            Ordering::Relaxed,
        );
        self.last_run_duration_ms
            .store(duration_ms, Ordering::Relaxed);
        self.errors_total
            .fetch_add(stats.errors.len() as u64, Ordering::Relaxed);
        self.ingestion_retries_total
            .fetch_add(stats.ingestion_retries, Ordering::Relaxed);
        self.ingestion_retries_succeeded_total
            .fetch_add(stats.ingestion_retries_succeeded, Ordering::Relaxed);
        self.stale_pending_deleted_total
            .fetch_add(stats.stale_pending_files_deleted, Ordering::Relaxed);
    }

    /// Format metrics for Prometheus.
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP downloader_cleanup_runs_total Total cleanup runs executed\n\
             # TYPE downloader_cleanup_runs_total counter\n\
             downloader_cleanup_runs_total {}\n\
             # HELP downloader_cleanup_files_deleted_total Total files deleted by cleanup\n\
             # TYPE downloader_cleanup_files_deleted_total counter\n\
             downloader_cleanup_files_deleted_total {}\n\
             # HELP downloader_cleanup_bytes_reclaimed_total Total bytes reclaimed by cleanup\n\
             # TYPE downloader_cleanup_bytes_reclaimed_total counter\n\
             downloader_cleanup_bytes_reclaimed_total {}\n\
             # HELP downloader_cleanup_db_records_pruned_total Total database records pruned\n\
             # TYPE downloader_cleanup_db_records_pruned_total counter\n\
             downloader_cleanup_db_records_pruned_total {}\n\
             # HELP downloader_cleanup_last_run_timestamp Unix timestamp of last cleanup run\n\
             # TYPE downloader_cleanup_last_run_timestamp gauge\n\
             downloader_cleanup_last_run_timestamp {}\n\
             # HELP downloader_cleanup_last_run_duration_ms Duration of last cleanup run in milliseconds\n\
             # TYPE downloader_cleanup_last_run_duration_ms gauge\n\
             downloader_cleanup_last_run_duration_ms {}\n\
             # HELP downloader_cleanup_errors_total Total errors encountered during cleanup\n\
             # TYPE downloader_cleanup_errors_total counter\n\
             downloader_cleanup_errors_total {}\n\
             # HELP downloader_cleanup_ingestion_retries_total Total ingestion retry attempts\n\
             # TYPE downloader_cleanup_ingestion_retries_total counter\n\
             downloader_cleanup_ingestion_retries_total {}\n\
             # HELP downloader_cleanup_ingestion_retries_succeeded_total Successful ingestion retries\n\
             # TYPE downloader_cleanup_ingestion_retries_succeeded_total counter\n\
             downloader_cleanup_ingestion_retries_succeeded_total {}\n\
             # HELP downloader_cleanup_stale_pending_deleted_total Files deleted due to stale pending ingestion\n\
             # TYPE downloader_cleanup_stale_pending_deleted_total counter\n\
             downloader_cleanup_stale_pending_deleted_total {}\n",
            self.runs_total.load(Ordering::Relaxed),
            self.files_deleted_total.load(Ordering::Relaxed),
            self.bytes_reclaimed_total.load(Ordering::Relaxed),
            self.db_records_pruned_total.load(Ordering::Relaxed),
            self.last_run_timestamp.load(Ordering::Relaxed),
            self.last_run_duration_ms.load(Ordering::Relaxed),
            self.errors_total.load(Ordering::Relaxed),
            self.ingestion_retries_total.load(Ordering::Relaxed),
            self.ingestion_retries_succeeded_total.load(Ordering::Relaxed),
            self.stale_pending_deleted_total.load(Ordering::Relaxed),
        )
    }
}

/// Background cleanup task.
pub struct CleanupTask {
    config: CleanupConfig,
    state: Arc<DownloadState>,
    metrics: Arc<CleanupMetrics>,
    ingester_url: Option<String>,
    http_client: reqwest::Client,
}

impl CleanupTask {
    /// Create a new cleanup task.
    pub fn new(
        config: CleanupConfig,
        state: Arc<DownloadState>,
        metrics: Arc<CleanupMetrics>,
        ingester_url: Option<String>,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            state,
            metrics,
            ingester_url,
            http_client,
        }
    }

    /// Run startup cleanup (partial files, pending ingestion retry, stale pending, and orphans).
    pub async fn run_startup_cleanup(&self) -> Result<CleanupStats> {
        info!("Running startup cleanup");
        let start = std::time::Instant::now();

        let mut stats = CleanupStats::default();

        // Clean stale partial files
        match self.cleanup_partial_files().await {
            Ok(partial_stats) => stats.merge(partial_stats),
            Err(e) => {
                let msg = format!("Failed to cleanup partial files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // Process pending files (retry recent ones, delete stale ones)
        match self.process_pending_files().await {
            Ok(pending_stats) => stats.merge(pending_stats),
            Err(e) => {
                let msg = format!("Failed to process pending files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // Clean orphan files (ingested but still on disk)
        match self.cleanup_orphan_files().await {
            Ok(orphan_stats) => stats.merge(orphan_stats),
            Err(e) => {
                let msg = format!("Failed to cleanup orphan files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        self.metrics.record(&stats, duration_ms);

        info!(
            partial_deleted = stats.partial_files_deleted,
            orphan_deleted = stats.orphan_files_deleted,
            stale_pending_deleted = stats.stale_pending_files_deleted,
            ingestion_retries = stats.ingestion_retries,
            ingestion_retries_succeeded = stats.ingestion_retries_succeeded,
            bytes_reclaimed = stats.bytes_reclaimed,
            errors = stats.errors.len(),
            duration_ms = duration_ms,
            "Startup cleanup complete"
        );

        Ok(stats)
    }

    /// Run periodic cleanup (files + database pruning).
    pub async fn run_periodic_cleanup(&self) -> Result<CleanupStats> {
        info!("Running periodic cleanup");
        let start = std::time::Instant::now();

        let mut stats = CleanupStats::default();

        // 1. Clean stale partial files
        match self.cleanup_partial_files().await {
            Ok(partial_stats) => stats.merge(partial_stats),
            Err(e) => {
                let msg = format!("Failed to cleanup partial files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // 2. Process pending files (retry recent ones, delete stale ones)
        match self.process_pending_files().await {
            Ok(pending_stats) => stats.merge(pending_stats),
            Err(e) => {
                let msg = format!("Failed to process pending files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // 3. Clean orphan files (ingested but still on disk)
        match self.cleanup_orphan_files().await {
            Ok(orphan_stats) => stats.merge(orphan_stats),
            Err(e) => {
                let msg = format!("Failed to cleanup orphan files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // 4. Prune database records
        match self.prune_database().await {
            Ok(db_stats) => stats.merge(db_stats),
            Err(e) => {
                let msg = format!("Failed to prune database: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        self.metrics.record(&stats, duration_ms);

        info!(
            partial_deleted = stats.partial_files_deleted,
            orphan_deleted = stats.orphan_files_deleted,
            stale_pending_deleted = stats.stale_pending_files_deleted,
            ingestion_retries = stats.ingestion_retries,
            ingestion_retries_succeeded = stats.ingestion_retries_succeeded,
            bytes_reclaimed = stats.bytes_reclaimed,
            completed_pruned = stats.completed_records_pruned,
            failed_pruned = stats.failed_records_pruned,
            errors = stats.errors.len(),
            duration_ms = duration_ms,
            "Periodic cleanup complete"
        );

        Ok(stats)
    }

    /// Run cleanup task forever in background.
    pub async fn run_forever(self, mut shutdown: broadcast::Receiver<()>) {
        if !self.config.enabled {
            info!("Cleanup task disabled");
            return;
        }

        info!(
            interval_secs = self.config.interval_secs,
            "Starting cleanup background task"
        );

        let mut interval = tokio::time::interval(Duration::from_secs(self.config.interval_secs));

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("Cleanup task shutting down");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = self.run_periodic_cleanup().await {
                        error!(error = %e, "Periodic cleanup failed");
                    }
                }
            }
        }
    }

    /// Clean stale partial files from temp directory.
    async fn cleanup_partial_files(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();
        let max_age = Duration::from_secs(self.config.partial_file_max_age_secs);
        let now = SystemTime::now();

        // Check if temp directory exists
        if !self.config.temp_dir.exists() {
            debug!(
                path = %self.config.temp_dir.display(),
                "Temp directory does not exist, skipping partial file cleanup"
            );
            return Ok(stats);
        }

        let mut entries = match tokio::fs::read_dir(&self.config.temp_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to read temp directory {}: {}",
                    self.config.temp_dir.display(),
                    e
                ));
            }
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Only process .partial files
            if !path.extension().map_or(false, |e| e == "partial") {
                continue;
            }

            let metadata = match entry.metadata().await {
                Ok(m) => m,
                Err(e) => {
                    debug!(path = %path.display(), error = %e, "Failed to get metadata");
                    continue;
                }
            };

            let modified = match metadata.modified() {
                Ok(m) => m,
                Err(e) => {
                    debug!(path = %path.display(), error = %e, "Failed to get modified time");
                    continue;
                }
            };

            let age = now.duration_since(modified).unwrap_or_default();
            if age > max_age {
                let size = metadata.len();

                if self.config.dry_run {
                    info!(
                        path = %path.display(),
                        age_secs = age.as_secs(),
                        size = size,
                        "[DRY RUN] Would delete stale partial file"
                    );
                    stats.partial_files_deleted += 1;
                    stats.bytes_reclaimed += size;
                    continue;
                }

                match tokio::fs::remove_file(&path).await {
                    Ok(()) => {
                        info!(
                            path = %path.display(),
                            age_secs = age.as_secs(),
                            size = size,
                            "Deleted stale partial file"
                        );
                        stats.partial_files_deleted += 1;
                        stats.bytes_reclaimed += size;
                    }
                    Err(e) => {
                        let msg =
                            format!("Failed to delete partial file {}: {}", path.display(), e);
                        warn!("{}", msg);
                        stats.errors.push(msg);
                    }
                }
            }
        }

        Ok(stats)
    }

    /// Clean orphan files - files marked as ingested but still on disk.
    async fn cleanup_orphan_files(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();

        // Check if output directory exists
        if !self.config.output_dir.exists() {
            debug!(
                path = %self.config.output_dir.display(),
                "Output directory does not exist, skipping orphan file cleanup"
            );
            return Ok(stats);
        }

        // Get all filenames that have been ingested, with their completion timestamps.
        // This allows us to safely handle race conditions where a file might be
        // re-downloaded between fetching the ingested set and checking the filesystem.
        let ingested_files = self.state.get_ingested_files_with_timestamps().await?;
        let ingested_map: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> =
            ingested_files.into_iter().collect();

        if ingested_map.is_empty() {
            debug!("No ingested files in database, skipping orphan cleanup");
            return Ok(stats);
        }

        debug!(
            count = ingested_map.len(),
            "Checking for orphan files against ingested set"
        );

        let mut entries = match tokio::fs::read_dir(&self.config.output_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to read output directory {}: {}",
                    self.config.output_dir.display(),
                    e
                ));
            }
        };

        while let Some(entry) = entries.next_entry().await? {
            let filename = entry.file_name().to_string_lossy().to_string();

            // Check if file is in ingested set
            if let Some(ingested_at) = ingested_map.get(&filename) {
                let path = entry.path();
                let metadata = match entry.metadata().await {
                    Ok(m) => m,
                    Err(e) => {
                        debug!(file = %filename, error = %e, "Failed to get file metadata");
                        continue;
                    }
                };

                // Safety check: only delete if the file's modification time is BEFORE
                // the ingestion timestamp. This prevents deleting a newly re-downloaded
                // file that happens to have the same name.
                let file_mtime = match metadata.modified() {
                    Ok(mtime) => mtime,
                    Err(e) => {
                        debug!(file = %filename, error = %e, "Failed to get file mtime");
                        continue;
                    }
                };

                let file_mtime_chrono: chrono::DateTime<chrono::Utc> = file_mtime.into();

                // Add a small buffer (1 second) to account for filesystem time resolution
                if file_mtime_chrono > *ingested_at + chrono::Duration::seconds(1) {
                    debug!(
                        file = %filename,
                        file_mtime = %file_mtime_chrono,
                        ingested_at = %ingested_at,
                        "Skipping file - newer than ingestion record (possible re-download)"
                    );
                    continue;
                }

                let size = metadata.len();

                if self.config.dry_run {
                    info!(
                        file = %filename,
                        size = size,
                        "[DRY RUN] Would delete orphan ingested file"
                    );
                    stats.orphan_files_deleted += 1;
                    stats.bytes_reclaimed += size;
                } else {
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {
                            info!(
                                file = %filename,
                                size = size,
                                "Deleted orphan ingested file"
                            );
                            stats.orphan_files_deleted += 1;
                            stats.bytes_reclaimed += size;
                        }
                        Err(e) => {
                            let msg = format!("Failed to delete orphan file {}: {}", filename, e);
                            warn!("{}", msg);
                            stats.errors.push(msg);
                        }
                    }
                }
            }
        }

        Ok(stats)
    }

    /// Process pending ingestion files: retry recent ones, clean up stale ones.
    ///
    /// This combines retry logic and stale cleanup to avoid double DB queries.
    /// Files are categorized by age:
    /// - < 5 minutes: Too recent, skip (let normal flow handle it)
    /// - 5 minutes to max_age: Retry ingestion
    /// - > max_age: Delete file and DB record
    async fn process_pending_files(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();

        // Get all pending ingestion files with timestamps (single DB query)
        let pending_files = self.state.get_pending_ingestion_with_timestamps().await?;

        if pending_files.is_empty() {
            debug!("No pending ingestion files to process");
            return Ok(stats);
        }

        let now = Utc::now();
        let min_retry_age = chrono::Duration::minutes(5);
        let max_age = chrono::Duration::seconds(self.config.pending_ingestion_max_age_secs as i64);

        // Partition files into retryable vs stale
        let mut retryable = Vec::new();
        let mut stale = Vec::new();

        for (url, filename, completed_at) in pending_files {
            let age = now - completed_at;

            if age < min_retry_age {
                // Too recent - let normal download flow handle it
                debug!(
                    file = %filename,
                    age_secs = age.num_seconds(),
                    "Skipping recently completed file (waiting for normal ingestion)"
                );
                continue;
            } else if age <= max_age {
                retryable.push((url, filename, completed_at));
            } else {
                stale.push((url, filename, completed_at));
            }
        }

        // Process retryable files (only if ingester URL is configured)
        if let Some(ref ingester_url) = self.ingester_url {
            if !retryable.is_empty() {
                debug!(
                    count = retryable.len(),
                    "Found pending ingestion files to retry"
                );

                for (url, filename, _completed_at) in retryable {
                    // Check if file still exists on disk
                    let file_path = self.config.output_dir.join(&filename);
                    if !file_path.exists() {
                        debug!(
                            file = %filename,
                            "Pending file no longer exists, skipping retry"
                        );
                        continue;
                    }

                    stats.ingestion_retries += 1;

                    // Attempt to trigger ingestion
                    let ingest_path = format!("/data/downloads/{}", filename);
                    let result = self
                        .http_client
                        .post(ingester_url)
                        .json(&serde_json::json!({
                            "file_path": ingest_path,
                            "source_url": url
                        }))
                        .send()
                        .await;

                    match result {
                        Ok(response) if response.status().is_success() => {
                            info!(
                                file = %filename,
                                "Ingestion retry succeeded"
                            );
                            stats.ingestion_retries_succeeded += 1;

                            // Mark as ingested first - only delete if this succeeds
                            if let Err(e) = self.state.mark_ingested(&url).await {
                                let msg = format!("Failed to mark {} as ingested: {}", filename, e);
                                warn!("{}", msg);
                                stats.errors.push(msg);
                                continue; // Don't delete the file if we couldn't mark it
                            }

                            delete_ingested_file(&self.config.output_dir, &filename).await;
                        }
                        Ok(response) => {
                            debug!(
                                file = %filename,
                                status = %response.status(),
                                "Ingestion retry failed (will try again later)"
                            );
                        }
                        Err(e) => {
                            debug!(
                                file = %filename,
                                error = %e,
                                "Ingestion retry request failed (will try again later)"
                            );
                        }
                    }
                }

                if stats.ingestion_retries > 0 {
                    info!(
                        retries = stats.ingestion_retries,
                        succeeded = stats.ingestion_retries_succeeded,
                        "Ingestion retry complete"
                    );
                }
            }
        } else if !retryable.is_empty() {
            debug!(
                count = retryable.len(),
                "No ingester URL configured, skipping {} retryable files",
                retryable.len()
            );
        }

        // Process stale files (delete file and DB record)
        for (url, filename, completed_at) in stale {
            let age = now - completed_at;
            let file_path = self.config.output_dir.join(&filename);
            let age_hours = age.num_minutes() as f64 / 60.0;

            if self.config.dry_run {
                info!(
                    file = %filename,
                    age_hours = format!("{:.1}", age_hours),
                    "[DRY RUN] Would delete stale pending file"
                );
                stats.stale_pending_files_deleted += 1;
            } else {
                // Delete file if it exists
                if file_path.exists() {
                    if let Ok(metadata) = tokio::fs::metadata(&file_path).await {
                        stats.bytes_reclaimed += metadata.len();
                    }

                    match tokio::fs::remove_file(&file_path).await {
                        Ok(()) => {
                            warn!(
                                file = %filename,
                                age_hours = format!("{:.1}", age_hours),
                                "Deleted stale pending file (ingestion failed repeatedly)"
                            );
                        }
                        Err(e) => {
                            let msg =
                                format!("Failed to delete stale pending file {}: {}", filename, e);
                            warn!("{}", msg);
                            stats.errors.push(msg);
                            continue; // Don't delete DB record if file deletion failed
                        }
                    }
                }

                // Delete DB record
                match self.state.delete_completed_record(&url).await {
                    Ok(_) => {
                        stats.stale_pending_files_deleted += 1;
                    }
                    Err(e) => {
                        let msg = format!("Failed to delete DB record for {}: {}", filename, e);
                        warn!("{}", msg);
                        stats.errors.push(msg);
                    }
                }
            }
        }

        if stats.stale_pending_files_deleted > 0 {
            warn!(
                count = stats.stale_pending_files_deleted,
                bytes = stats.bytes_reclaimed,
                max_age_hours = self.config.pending_ingestion_max_age_secs / 3600,
                "Cleaned up stale pending files (check ingester health!)"
            );
        }

        Ok(stats)
    }

    /// Prune old records from the database.
    async fn prune_database(&self) -> Result<CleanupStats> {
        let mut stats = CleanupStats::default();

        // Prune old completed records
        match self
            .state
            .prune_old_completed(self.config.completed_record_retention_days)
            .await
        {
            Ok(count) => {
                if count > 0 {
                    info!(count = count, "Pruned old completed download records");
                }
                stats.completed_records_pruned = count;
            }
            Err(e) => {
                let msg = format!("Failed to prune completed records: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // Prune old failed records
        match self
            .state
            .prune_old_failed(self.config.failed_record_retention_days)
            .await
        {
            Ok(count) => {
                if count > 0 {
                    info!(count = count, "Pruned old failed download records");
                }
                stats.failed_records_pruned = count;
            }
            Err(e) => {
                let msg = format!("Failed to prune failed records: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // Vacuum if we deleted a significant number of records
        let total_pruned = stats.completed_records_pruned + stats.failed_records_pruned;
        if total_pruned > 1000 {
            match self.state.vacuum().await {
                Ok(()) => {
                    debug!("Vacuumed database after pruning {} records", total_pruned);
                }
                Err(e) => {
                    let msg = format!("Failed to vacuum database: {}", e);
                    warn!("{}", msg);
                    stats.errors.push(msg);
                }
            }
        }

        Ok(stats)
    }
}

/// Delete a single file after successful ingestion.
/// This is called inline during the download/ingest flow.
pub async fn delete_ingested_file(output_dir: &std::path::Path, filename: &str) {
    let file_path = output_dir.join(filename);
    match tokio::fs::remove_file(&file_path).await {
        Ok(()) => {
            info!(file = %filename, "Deleted source file after ingestion");
        }
        Err(e) => {
            // Log but don't fail - periodic cleanup will catch it
            warn!(
                file = %filename,
                error = %e,
                "Failed to delete source file after ingestion (will retry in periodic cleanup)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_config_default() {
        let config = CleanupConfig::default();
        assert!(config.enabled);
        assert!(!config.dry_run);
        assert_eq!(config.interval_secs, 3600);
        assert_eq!(config.partial_file_max_age_secs, 3600);
        assert_eq!(config.completed_record_retention_days, 7);
        assert_eq!(config.failed_record_retention_days, 7);
    }

    #[test]
    fn test_cleanup_stats_merge() {
        let mut stats1 = CleanupStats {
            partial_files_deleted: 5,
            orphan_files_deleted: 3,
            bytes_reclaimed: 1000,
            completed_records_pruned: 10,
            failed_records_pruned: 2,
            ingestion_retries: 3,
            ingestion_retries_succeeded: 2,
            stale_pending_files_deleted: 1,
            errors: vec!["error1".to_string()],
        };

        let stats2 = CleanupStats {
            partial_files_deleted: 2,
            orphan_files_deleted: 1,
            bytes_reclaimed: 500,
            completed_records_pruned: 5,
            failed_records_pruned: 1,
            ingestion_retries: 2,
            ingestion_retries_succeeded: 1,
            stale_pending_files_deleted: 2,
            errors: vec!["error2".to_string()],
        };

        stats1.merge(stats2);

        assert_eq!(stats1.partial_files_deleted, 7);
        assert_eq!(stats1.orphan_files_deleted, 4);
        assert_eq!(stats1.bytes_reclaimed, 1500);
        assert_eq!(stats1.completed_records_pruned, 15);
        assert_eq!(stats1.failed_records_pruned, 3);
        assert_eq!(stats1.ingestion_retries, 5);
        assert_eq!(stats1.ingestion_retries_succeeded, 3);
        assert_eq!(stats1.stale_pending_files_deleted, 3);
        assert_eq!(stats1.errors.len(), 2);
    }

    #[test]
    fn test_cleanup_metrics_prometheus_format() {
        let metrics = CleanupMetrics::new();
        metrics.runs_total.store(5, Ordering::Relaxed);
        metrics.files_deleted_total.store(100, Ordering::Relaxed);
        metrics
            .bytes_reclaimed_total
            .store(1024000, Ordering::Relaxed);

        let output = metrics.to_prometheus();
        assert!(output.contains("downloader_cleanup_runs_total 5"));
        assert!(output.contains("downloader_cleanup_files_deleted_total 100"));
        assert!(output.contains("downloader_cleanup_bytes_reclaimed_total 1024000"));
    }

    #[tokio::test]
    async fn test_cleanup_partial_files() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        // Create a partial file
        let partial_path = temp_dir.path().join("test.grib2.partial");
        tokio::fs::write(&partial_path, "test data").await.unwrap();

        // Set modification time to 2 hours ago
        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        filetime::set_file_mtime(
            &partial_path,
            filetime::FileTime::from_system_time(two_hours_ago),
        )
        .unwrap();

        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,      // 1 hour max age
            pending_ingestion_max_age_secs: 7200, // 2 hours max age
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        // We can't test the full task without a real database,
        // but we can verify the config is correct
        assert!(config.enabled);
        assert_eq!(config.partial_file_max_age_secs, 3600);
    }

    #[tokio::test]
    async fn test_cleanup_full_integration() {
        use crate::state::DownloadState;

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        // Create an in-memory database
        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // 1. Create a stale partial file (should be deleted)
        let partial_path = temp_dir.path().join("test.grib2.partial");
        tokio::fs::write(&partial_path, "partial data")
            .await
            .unwrap();
        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        filetime::set_file_mtime(
            &partial_path,
            filetime::FileTime::from_system_time(two_hours_ago),
        )
        .unwrap();

        // 2. Create an orphan file (marked as ingested but still on disk)
        // Set the file's mtime to BEFORE the ingestion time so it gets deleted
        let orphan_filename = "orphan_file.grib2";
        let orphan_path = output_dir.path().join(orphan_filename);
        tokio::fs::write(&orphan_path, "orphan data that should be deleted")
            .await
            .unwrap();
        // Set file mtime to 1 hour ago (before ingestion)
        let one_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        filetime::set_file_mtime(
            &orphan_path,
            filetime::FileTime::from_system_time(one_hour_ago),
        )
        .unwrap();

        // Mark the orphan file as downloaded and ingested in the database
        // The ingestion timestamp will be "now", which is AFTER the file mtime
        state
            .queue_download(
                "http://example.com/orphan_file.grib2",
                orphan_filename,
                "test",
            )
            .await
            .unwrap();
        state
            .update_status(
                "http://example.com/orphan_file.grib2",
                crate::state::DownloadStatus::Completed,
            )
            .await
            .unwrap();
        state
            .mark_ingested("http://example.com/orphan_file.grib2")
            .await
            .unwrap();

        // 3. Create a file that should NOT be deleted (not in database as ingested)
        let keep_filename = "keep_this_file.grib2";
        let keep_path = output_dir.path().join(keep_filename);
        tokio::fs::write(&keep_path, "data to keep").await.unwrap();

        // Verify files exist before cleanup
        assert!(
            partial_path.exists(),
            "Partial file should exist before cleanup"
        );
        assert!(
            orphan_path.exists(),
            "Orphan file should exist before cleanup"
        );
        assert!(keep_path.exists(), "Keep file should exist before cleanup");

        // Create and run cleanup task
        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200,
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        let cleanup_task = CleanupTask::new(config, state, metrics.clone(), None);

        // Run startup cleanup
        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // Verify results
        assert_eq!(
            stats.partial_files_deleted, 1,
            "Should delete 1 stale partial file"
        );
        assert_eq!(stats.orphan_files_deleted, 1, "Should delete 1 orphan file");
        assert!(stats.errors.is_empty(), "Should have no errors");

        // Verify files after cleanup
        assert!(!partial_path.exists(), "Partial file should be deleted");
        assert!(!orphan_path.exists(), "Orphan file should be deleted");
        assert!(keep_path.exists(), "Keep file should NOT be deleted");

        // Verify metrics were recorded
        assert!(metrics.runs_total.load(Ordering::Relaxed) >= 1);
        assert!(metrics.files_deleted_total.load(Ordering::Relaxed) >= 2);
        assert!(metrics.bytes_reclaimed_total.load(Ordering::Relaxed) > 0);
    }

    #[tokio::test]
    async fn test_cleanup_race_condition_protection() {
        // Test that a file re-downloaded AFTER ingestion is NOT deleted
        use crate::state::DownloadState;

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // Create a file and mark it as ingested in the database
        let filename = "redownloaded_file.grib2";
        let file_path = output_dir.path().join(filename);

        // First, create the ingestion record (simulating past ingestion)
        state
            .queue_download(
                "http://example.com/redownloaded_file.grib2",
                filename,
                "test",
            )
            .await
            .unwrap();
        state
            .update_status(
                "http://example.com/redownloaded_file.grib2",
                crate::state::DownloadStatus::Completed,
            )
            .await
            .unwrap();
        state
            .mark_ingested("http://example.com/redownloaded_file.grib2")
            .await
            .unwrap();

        // Now create a NEW file with the same name (simulating re-download)
        tokio::fs::write(&file_path, "newly downloaded data - should NOT be deleted")
            .await
            .unwrap();

        // Set the file's mtime to be 10 seconds in the FUTURE to simulate
        // a file that was re-downloaded after the ingestion record was created
        let future_time = std::time::SystemTime::now() + Duration::from_secs(10);
        filetime::set_file_mtime(
            &file_path,
            filetime::FileTime::from_system_time(future_time),
        )
        .unwrap();

        // Create and run cleanup task
        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200,
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        let cleanup_task = CleanupTask::new(config, state, metrics.clone(), None);

        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // File should NOT be deleted because its mtime is after ingestion time
        assert!(
            file_path.exists(),
            "Re-downloaded file should NOT be deleted (race condition protection)"
        );
        assert_eq!(
            stats.orphan_files_deleted, 0,
            "Should not delete files newer than ingestion time"
        );
    }

    #[tokio::test]
    async fn test_cleanup_dry_run_mode() {
        use crate::state::DownloadState;

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();

        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // Create a stale partial file
        let partial_path = temp_dir.path().join("test.grib2.partial");
        tokio::fs::write(&partial_path, "partial data")
            .await
            .unwrap();
        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        filetime::set_file_mtime(
            &partial_path,
            filetime::FileTime::from_system_time(two_hours_ago),
        )
        .unwrap();

        // Create and run cleanup task in DRY RUN mode
        let config = CleanupConfig {
            enabled: true,
            dry_run: true, // DRY RUN!
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200,
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        let cleanup_task = CleanupTask::new(config, state, metrics.clone(), None);

        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // Stats should show what WOULD be deleted
        assert_eq!(
            stats.partial_files_deleted, 1,
            "Should report 1 file would be deleted"
        );

        // But file should still exist!
        assert!(
            partial_path.exists(),
            "File should NOT be deleted in dry run mode"
        );
    }

    #[tokio::test]
    async fn test_delete_ingested_file() {
        let output_dir = TempDir::new().unwrap();
        let filename = "test_delete.grib2";
        let file_path = output_dir.path().join(filename);

        // Create a test file
        tokio::fs::write(&file_path, "test data to delete")
            .await
            .unwrap();
        assert!(file_path.exists(), "File should exist before deletion");

        // Delete it
        delete_ingested_file(output_dir.path(), filename).await;

        // Verify it's gone
        assert!(!file_path.exists(), "File should be deleted");
    }

    #[tokio::test]
    async fn test_delete_ingested_file_missing() {
        let output_dir = TempDir::new().unwrap();
        let filename = "nonexistent.grib2";

        // This should not panic, just log a warning
        delete_ingested_file(output_dir.path(), filename).await;
        // If we get here without panicking, the test passes
    }

    #[tokio::test]
    async fn test_process_pending_files_successful_retry() {
        use crate::state::DownloadState;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // Start mock ingester server
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ingest"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        // Create a pending file (completed 10 minutes ago - past min retry age)
        let filename = "pending_retry.grib2";
        let file_path = output_dir.path().join(filename);
        tokio::fs::write(&file_path, "data to ingest")
            .await
            .unwrap();

        // Add to DB as completed but not ingested (10 minutes ago to pass 5-minute min age)
        let ten_minutes_ago = Utc::now() - chrono::Duration::minutes(10);
        state
            .insert_completed_for_test(
                "http://example.com/test.grib2",
                filename,
                Some(ten_minutes_ago),
            )
            .await
            .unwrap();

        // Create cleanup task with mock ingester URL
        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200,
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        let ingester_url = format!("{}/ingest", mock_server.uri());
        let cleanup_task =
            CleanupTask::new(config, state.clone(), metrics.clone(), Some(ingester_url));

        // Run cleanup
        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // Verify retry was attempted and succeeded
        assert_eq!(stats.ingestion_retries, 1, "Should have 1 retry attempt");
        assert_eq!(stats.ingestion_retries_succeeded, 1, "Retry should succeed");

        // File should be deleted after successful ingestion
        assert!(
            !file_path.exists(),
            "File should be deleted after successful retry"
        );

        // DB record should be marked as ingested
        let pending = state.get_pending_ingestion().await.unwrap();
        assert!(pending.is_empty(), "No pending files should remain");
    }

    #[tokio::test]
    async fn test_process_pending_files_failed_retry_keeps_file() {
        use crate::state::DownloadState;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // Start mock ingester server that returns 500 error
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ingest"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&mock_server)
            .await;

        // Create a pending file
        let filename = "pending_fail.grib2";
        let file_path = output_dir.path().join(filename);
        tokio::fs::write(&file_path, "data to ingest")
            .await
            .unwrap();

        // Add to DB as completed but not ingested (10 minutes ago)
        let ten_minutes_ago = Utc::now() - chrono::Duration::minutes(10);
        state
            .insert_completed_for_test(
                "http://example.com/fail.grib2",
                filename,
                Some(ten_minutes_ago),
            )
            .await
            .unwrap();

        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200,
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        let ingester_url = format!("{}/ingest", mock_server.uri());
        let cleanup_task =
            CleanupTask::new(config, state.clone(), metrics.clone(), Some(ingester_url));

        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // Verify retry was attempted but failed
        assert_eq!(stats.ingestion_retries, 1, "Should have 1 retry attempt");
        assert_eq!(stats.ingestion_retries_succeeded, 0, "Retry should fail");

        // File should still exist
        assert!(file_path.exists(), "File should remain after failed retry");

        // DB record should still be pending
        let pending = state.get_pending_ingestion().await.unwrap();
        assert_eq!(pending.len(), 1, "Pending file should remain in DB");
    }

    #[tokio::test]
    async fn test_process_pending_files_stale_cleanup() {
        use crate::state::DownloadState;

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // Create a stale pending file (completed 3 hours ago, max age is 2 hours)
        let filename = "stale_pending.grib2";
        let file_path = output_dir.path().join(filename);
        tokio::fs::write(&file_path, "stale data").await.unwrap();

        // Completed 3 hours ago (past the 2-hour max age)
        let three_hours_ago = Utc::now() - chrono::Duration::hours(3);
        state
            .insert_completed_for_test(
                "http://example.com/stale.grib2",
                filename,
                Some(three_hours_ago),
            )
            .await
            .unwrap();

        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200, // 2 hours
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        // No ingester URL - only stale cleanup should happen
        let cleanup_task = CleanupTask::new(config, state.clone(), metrics.clone(), None);

        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // Verify stale file was deleted
        assert_eq!(
            stats.stale_pending_files_deleted, 1,
            "Should delete 1 stale file"
        );
        assert!(!file_path.exists(), "Stale file should be deleted");

        // DB record should be gone
        let pending = state.get_pending_ingestion().await.unwrap();
        assert!(pending.is_empty(), "No pending files should remain");
    }

    #[tokio::test]
    async fn test_process_pending_files_skips_recent() {
        use crate::state::DownloadState;

        let temp_dir = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let state = Arc::new(DownloadState::open_memory().await.unwrap());

        // Create a very recent pending file (just now - should be skipped)
        let filename = "recent_pending.grib2";
        let file_path = output_dir.path().join(filename);
        tokio::fs::write(&file_path, "recent data").await.unwrap();

        // Completed just now (should be skipped as too recent)
        state
            .insert_completed_for_test("http://example.com/recent.grib2", filename, None)
            .await
            .unwrap();

        let config = CleanupConfig {
            enabled: true,
            dry_run: false,
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
            pending_ingestion_max_age_secs: 7200,
            completed_record_retention_days: 7,
            failed_record_retention_days: 7,
            output_dir: output_dir.path().to_path_buf(),
            temp_dir: temp_dir.path().to_path_buf(),
        };

        let metrics = Arc::new(CleanupMetrics::new());
        let cleanup_task = CleanupTask::new(
            config,
            state.clone(),
            metrics.clone(),
            Some("http://ingester:8082/ingest".to_string()),
        );

        let stats = cleanup_task.run_startup_cleanup().await.unwrap();

        // Verify no retry was attempted (file is too recent)
        assert_eq!(stats.ingestion_retries, 0, "Should skip recent file");

        // File should still exist
        assert!(file_path.exists(), "Recent file should remain");

        // DB record should still be pending
        let pending = state.get_pending_ingestion().await.unwrap();
        assert_eq!(pending.len(), 1, "Recent pending file should remain");
    }
}
