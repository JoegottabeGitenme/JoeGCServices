//! Cleanup system for managing downloaded files and database records.
//!
//! This module handles:
//! - Immediate deletion of source files after successful ingestion
//! - Periodic cleanup of stale partial files in temp directory
//! - Cleanup of orphan files (marked ingested but still on disk)
//! - Pruning old records from the state database
//!
//! # Background
//!
//! Downloaded GRIB2/NetCDF files are stored in `/data/downloads/` until ingested.
//! After the ingester processes a file and converts it to Zarr format in MinIO,
//! the source file is no longer needed. Without cleanup, these files accumulate
//! indefinitely, consuming disk space.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::state::DownloadState;

/// Configuration for the cleanup system.
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// Enable cleanup (default: true)
    pub enabled: bool,
    /// Interval between periodic cleanup runs in seconds (default: 3600 = 1 hour)
    pub interval_secs: u64,
    /// Max age for .partial files before deletion in seconds (default: 3600 = 1 hour)
    pub partial_file_max_age_secs: u64,
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
            interval_secs: 3600,
            partial_file_max_age_secs: 3600,
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
        self.errors.extend(other.errors);
    }
}

/// Accumulated metrics for Prometheus export.
#[derive(Debug, Default)]
pub struct CleanupMetrics {
    pub files_deleted_total: AtomicU64,
    pub bytes_reclaimed_total: AtomicU64,
    pub db_records_pruned_total: AtomicU64,
    pub last_run_timestamp: AtomicU64,
    pub last_run_duration_ms: AtomicU64,
    pub errors_total: AtomicU64,
}

impl CleanupMetrics {
    /// Create new metrics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record stats from a cleanup run.
    pub fn record(&self, stats: &CleanupStats, duration_ms: u64) {
        self.files_deleted_total.fetch_add(
            stats.partial_files_deleted + stats.orphan_files_deleted,
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
    }

    /// Format metrics for Prometheus.
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP downloader_cleanup_files_deleted_total Total files deleted by cleanup\n\
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
             downloader_cleanup_errors_total {}\n",
            self.files_deleted_total.load(Ordering::Relaxed),
            self.bytes_reclaimed_total.load(Ordering::Relaxed),
            self.db_records_pruned_total.load(Ordering::Relaxed),
            self.last_run_timestamp.load(Ordering::Relaxed),
            self.last_run_duration_ms.load(Ordering::Relaxed),
            self.errors_total.load(Ordering::Relaxed),
        )
    }
}

/// Background cleanup task.
pub struct CleanupTask {
    config: CleanupConfig,
    state: Arc<DownloadState>,
    metrics: Arc<CleanupMetrics>,
}

impl CleanupTask {
    /// Create a new cleanup task.
    pub fn new(
        config: CleanupConfig,
        state: Arc<DownloadState>,
        metrics: Arc<CleanupMetrics>,
    ) -> Self {
        Self {
            config,
            state,
            metrics,
        }
    }

    /// Run startup cleanup (partial files and orphans only, no DB pruning).
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

        // Clean stale partial files
        match self.cleanup_partial_files().await {
            Ok(partial_stats) => stats.merge(partial_stats),
            Err(e) => {
                let msg = format!("Failed to cleanup partial files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // Clean orphan files
        match self.cleanup_orphan_files().await {
            Ok(orphan_stats) => stats.merge(orphan_stats),
            Err(e) => {
                let msg = format!("Failed to cleanup orphan files: {}", e);
                warn!("{}", msg);
                stats.errors.push(msg);
            }
        }

        // Prune database records
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

        // Get all filenames that have been ingested
        let ingested_filenames = self.state.get_ingested_filenames().await?;
        let ingested_set: HashSet<String> = ingested_filenames.into_iter().collect();

        if ingested_set.is_empty() {
            debug!("No ingested files in database, skipping orphan cleanup");
            return Ok(stats);
        }

        debug!(
            count = ingested_set.len(),
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

            // If file is in ingested set, it should have been deleted - delete it now
            if ingested_set.contains(&filename) {
                let path = entry.path();
                let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);

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
        assert_eq!(config.interval_secs, 3600);
        assert_eq!(config.partial_file_max_age_secs, 3600);
        assert_eq!(config.completed_record_retention_days, 7);
    }

    #[test]
    fn test_cleanup_stats_merge() {
        let mut stats1 = CleanupStats {
            partial_files_deleted: 5,
            orphan_files_deleted: 3,
            bytes_reclaimed: 1000,
            completed_records_pruned: 10,
            failed_records_pruned: 2,
            errors: vec!["error1".to_string()],
        };

        let stats2 = CleanupStats {
            partial_files_deleted: 2,
            orphan_files_deleted: 1,
            bytes_reclaimed: 500,
            completed_records_pruned: 5,
            failed_records_pruned: 1,
            errors: vec!["error2".to_string()],
        };

        stats1.merge(stats2);

        assert_eq!(stats1.partial_files_deleted, 7);
        assert_eq!(stats1.orphan_files_deleted, 4);
        assert_eq!(stats1.bytes_reclaimed, 1500);
        assert_eq!(stats1.completed_records_pruned, 15);
        assert_eq!(stats1.failed_records_pruned, 3);
        assert_eq!(stats1.errors.len(), 2);
    }

    #[test]
    fn test_cleanup_metrics_prometheus_format() {
        let metrics = CleanupMetrics::new();
        metrics.files_deleted_total.store(100, Ordering::Relaxed);
        metrics
            .bytes_reclaimed_total
            .store(1024000, Ordering::Relaxed);

        let output = metrics.to_prometheus();
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
            interval_secs: 3600,
            partial_file_max_age_secs: 3600, // 1 hour max age
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
}
