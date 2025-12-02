//! Download state persistence using SQLite with sqlx.
//!
//! Tracks download progress, resume offsets, and completion status
//! to survive service restarts.

use std::path::Path;


use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use tracing::{debug, info};

use crate::download::DownloadProgress;

/// Download status for tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    InProgress,
    Retrying,
    Completed,
    Failed,
}

impl DownloadStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Retrying => "retrying",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "retrying" => Self::Retrying,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// Persistent download state stored in SQLite.
#[derive(Debug, Clone)]
pub struct DownloadRecord {
    pub url: String,
    pub filename: String,
    pub model: Option<String>,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub status: DownloadStatus,
    pub retry_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

/// Manages download state persistence.
pub struct DownloadState {
    pool: SqlitePool,
}

impl DownloadState {
    /// Open or create the state database at the given path.
    pub async fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("Failed to open SQLite database")?;

        // Create tables
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS downloads (
                url TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                model TEXT,
                total_bytes INTEGER,
                downloaded_bytes INTEGER DEFAULT 0,
                status TEXT DEFAULT 'pending',
                retry_count INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                error_message TEXT
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_downloads_status ON downloads(status)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_downloads_model ON downloads(model)")
            .execute(&pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS completed_downloads (
                url TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                model TEXT,
                total_bytes INTEGER,
                completed_at TEXT NOT NULL,
                ingested BOOLEAN DEFAULT 0
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_completed_model ON completed_downloads(model)",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_completed_ingested ON completed_downloads(ingested)",
        )
        .execute(&pool)
        .await?;

        info!(path = %path.display(), "Opened download state database");

        Ok(Self { pool })
    }

    /// Open an in-memory database (for testing).
    pub async fn open_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE downloads (
                url TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                model TEXT,
                total_bytes INTEGER,
                downloaded_bytes INTEGER DEFAULT 0,
                status TEXT DEFAULT 'pending',
                retry_count INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                error_message TEXT
            )
            "#,
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE completed_downloads (
                url TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                model TEXT,
                total_bytes INTEGER,
                completed_at TEXT NOT NULL,
                ingested BOOLEAN DEFAULT 0
            )
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    /// Get or create a download progress record.
    pub async fn get_or_create(&self, url: &str, filename: &str) -> Result<DownloadProgress> {
        let now = Utc::now().to_rfc3339();

        // Try to get existing record
        let existing: Option<(Option<i64>, i64, i32)> = sqlx::query_as(
            "SELECT total_bytes, downloaded_bytes, retry_count FROM downloads WHERE url = ?",
        )
        .bind(url)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((total_bytes, downloaded_bytes, retry_count)) = existing {
            debug!(url = %url, "Found existing download record");
            return Ok(DownloadProgress {
                url: url.to_string(),
                total_bytes: total_bytes.map(|b| b as u64),
                downloaded_bytes: downloaded_bytes as u64,
                started_at: Utc::now(),
                last_update: Utc::now(),
                retry_count: retry_count as u32,
            });
        }

        // Create new record
        sqlx::query(
            r#"
            INSERT INTO downloads (url, filename, status, created_at, updated_at)
            VALUES (?, ?, 'pending', ?, ?)
            "#,
        )
        .bind(url)
        .bind(filename)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        debug!(url = %url, filename = %filename, "Created new download record");

        Ok(DownloadProgress {
            url: url.to_string(),
            total_bytes: None,
            downloaded_bytes: 0,
            started_at: Utc::now(),
            last_update: Utc::now(),
            retry_count: 0,
        })
    }

    /// Update download progress.
    pub async fn update_progress(&self, url: &str, progress: &DownloadProgress) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            UPDATE downloads
            SET total_bytes = ?, downloaded_bytes = ?, retry_count = ?, 
                status = 'in_progress', updated_at = ?
            WHERE url = ?
            "#,
        )
        .bind(progress.total_bytes.map(|b| b as i64))
        .bind(progress.downloaded_bytes as i64)
        .bind(progress.retry_count as i32)
        .bind(&now)
        .bind(url)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update download status.
    pub async fn update_status(&self, url: &str, status: DownloadStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query("UPDATE downloads SET status = ?, updated_at = ? WHERE url = ?")
            .bind(status.as_str())
            .bind(&now)
            .bind(url)
            .execute(&self.pool)
            .await?;

        // If completed, move to completed_downloads table
        if status == DownloadStatus::Completed {
            sqlx::query(
                r#"
                INSERT OR REPLACE INTO completed_downloads (url, filename, model, total_bytes, completed_at)
                SELECT url, filename, model, total_bytes, ?
                FROM downloads WHERE url = ?
                "#,
            )
            .bind(&now)
            .bind(url)
            .execute(&self.pool)
            .await?;

            sqlx::query("DELETE FROM downloads WHERE url = ?")
                .bind(url)
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    /// Set error message for a download.
    pub async fn set_error(&self, url: &str, error: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query("UPDATE downloads SET error_message = ?, updated_at = ? WHERE url = ?")
            .bind(error)
            .bind(&now)
            .bind(url)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get all in-progress downloads (for resuming after restart).
    pub async fn get_in_progress(&self) -> Result<Vec<DownloadRecord>> {
        let rows: Vec<(
            String,
            String,
            Option<String>,
            Option<i64>,
            i64,
            String,
            i32,
            String,
            String,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT url, filename, model, total_bytes, downloaded_bytes, 
                   status, retry_count, created_at, updated_at, error_message
            FROM downloads 
            WHERE status IN ('pending', 'in_progress', 'retrying')
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let records = rows
            .into_iter()
            .map(|row| DownloadRecord {
                url: row.0,
                filename: row.1,
                model: row.2,
                total_bytes: row.3.map(|b| b as u64),
                downloaded_bytes: row.4 as u64,
                status: DownloadStatus::from_str(&row.5),
                retry_count: row.6 as u32,
                created_at: DateTime::parse_from_rfc3339(&row.7)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.8)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                error_message: row.9,
            })
            .collect();

        Ok(records)
    }

    /// Check if a URL has already been downloaded.
    pub async fn is_already_downloaded(&self, url: &str) -> Result<bool> {
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM completed_downloads WHERE url = ?")
                .bind(url)
                .fetch_one(&self.pool)
                .await?;

        Ok(count.0 > 0)
    }

    /// Get completed downloads that haven't been ingested yet.
    pub async fn get_pending_ingestion(&self) -> Result<Vec<(String, String)>> {
        let records: Vec<(String, String)> = sqlx::query_as(
            "SELECT url, filename FROM completed_downloads WHERE ingested = 0 ORDER BY completed_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    /// Mark a download as ingested.
    pub async fn mark_ingested(&self, url: &str) -> Result<()> {
        sqlx::query("UPDATE completed_downloads SET ingested = 1 WHERE url = ?")
            .bind(url)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Add a download to the queue with model metadata.
    pub async fn queue_download(&self, url: &str, filename: &str, model: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"
            INSERT OR IGNORE INTO downloads (url, filename, model, status, created_at, updated_at)
            VALUES (?, ?, ?, 'pending', ?, ?)
            "#,
        )
        .bind(url)
        .bind(filename)
        .bind(model)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        debug!(url = %url, model = %model, "Queued download");
        Ok(())
    }

    /// Get download statistics.
    pub async fn get_stats(&self) -> Result<DownloadStats> {
        let pending: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM downloads WHERE status = 'pending'")
                .fetch_one(&self.pool)
                .await?;

        let in_progress: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM downloads WHERE status = 'in_progress'")
                .fetch_one(&self.pool)
                .await?;

        let failed: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM downloads WHERE status = 'failed'")
                .fetch_one(&self.pool)
                .await?;

        let completed: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM completed_downloads")
            .fetch_one(&self.pool)
            .await?;

        let total_bytes: (i64,) =
            sqlx::query_as("SELECT COALESCE(SUM(total_bytes), 0) FROM completed_downloads")
                .fetch_one(&self.pool)
                .await
                .unwrap_or((0,));

        Ok(DownloadStats {
            pending: pending.0 as u64,
            in_progress: in_progress.0 as u64,
            failed: failed.0 as u64,
            completed: completed.0 as u64,
            total_bytes_downloaded: total_bytes.0 as u64,
        })
    }

    /// Clean up old completed downloads (older than retention_days).
    pub async fn cleanup_old_records(&self, retention_days: u32) -> Result<u64> {
        let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();

        let result = sqlx::query(
            "DELETE FROM completed_downloads WHERE completed_at < ? AND ingested = 1",
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await?;

        let deleted = result.rows_affected();

        info!(deleted = deleted, retention_days = retention_days, "Cleaned up old download records");
        Ok(deleted)
    }

    /// Get recent completed downloads for display.
    pub async fn get_recent_completed(&self, limit: usize) -> Result<Vec<CompletedDownload>> {
        let rows: Vec<(String, String, Option<String>, Option<i64>, String, bool)> = sqlx::query_as(
            r#"
            SELECT url, filename, model, total_bytes, completed_at, ingested
            FROM completed_downloads
            ORDER BY completed_at DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let records = rows
            .into_iter()
            .map(|row| CompletedDownload {
                url: row.0,
                filename: row.1,
                model: row.2,
                total_bytes: row.3.map(|b| b as u64),
                completed_at: DateTime::parse_from_rfc3339(&row.4)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                ingested: row.5,
            })
            .collect();

        Ok(records)
    }

    /// Get all pending downloads.
    pub async fn get_pending(&self) -> Result<Vec<DownloadRecord>> {
        let rows: Vec<(
            String,
            String,
            Option<String>,
            Option<i64>,
            i64,
            String,
            i32,
            String,
            String,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT url, filename, model, total_bytes, downloaded_bytes, 
                   status, retry_count, created_at, updated_at, error_message
            FROM downloads 
            WHERE status = 'pending'
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let records = rows
            .into_iter()
            .map(|row| DownloadRecord {
                url: row.0,
                filename: row.1,
                model: row.2,
                total_bytes: row.3.map(|b| b as u64),
                downloaded_bytes: row.4 as u64,
                status: DownloadStatus::from_str(&row.5),
                retry_count: row.6 as u32,
                created_at: DateTime::parse_from_rfc3339(&row.7)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.8)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                error_message: row.9,
            })
            .collect();

        Ok(records)
    }

    /// Get failed downloads.
    pub async fn get_failed(&self) -> Result<Vec<DownloadRecord>> {
        let rows: Vec<(
            String,
            String,
            Option<String>,
            Option<i64>,
            i64,
            String,
            i32,
            String,
            String,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT url, filename, model, total_bytes, downloaded_bytes, 
                   status, retry_count, created_at, updated_at, error_message
            FROM downloads 
            WHERE status = 'failed'
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let records = rows
            .into_iter()
            .map(|row| DownloadRecord {
                url: row.0,
                filename: row.1,
                model: row.2,
                total_bytes: row.3.map(|b| b as u64),
                downloaded_bytes: row.4 as u64,
                status: DownloadStatus::from_str(&row.5),
                retry_count: row.6 as u32,
                created_at: DateTime::parse_from_rfc3339(&row.7)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&row.8)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                error_message: row.9,
            })
            .collect();

        Ok(records)
    }

    /// Get download time series data (hourly aggregates for charts).
    pub async fn get_hourly_stats(&self, hours: u32) -> Result<Vec<HourlyStats>> {
        let cutoff = (Utc::now() - chrono::Duration::hours(hours as i64)).to_rfc3339();

        // SQLite date functions to group by hour
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            r#"
            SELECT 
                strftime('%Y-%m-%d %H:00', completed_at) as hour,
                COUNT(*) as count,
                COALESCE(SUM(total_bytes), 0) as bytes
            FROM completed_downloads
            WHERE completed_at >= ?
            GROUP BY strftime('%Y-%m-%d %H:00', completed_at)
            ORDER BY hour ASC
            "#,
        )
        .bind(&cutoff)
        .fetch_all(&self.pool)
        .await?;

        let stats = rows
            .into_iter()
            .map(|(hour, count, bytes)| HourlyStats {
                hour,
                download_count: count as u64,
                bytes_downloaded: bytes as u64,
            })
            .collect();

        Ok(stats)
    }

    /// Retry a failed download.
    pub async fn retry_download(&self, url: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();

        let result = sqlx::query(
            "UPDATE downloads SET status = 'pending', retry_count = 0, error_message = NULL, updated_at = ? WHERE url = ? AND status = 'failed'"
        )
        .bind(&now)
        .bind(url)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

/// Completed download record.
#[derive(Debug, Clone)]
pub struct CompletedDownload {
    pub url: String,
    pub filename: String,
    pub model: Option<String>,
    pub total_bytes: Option<u64>,
    pub completed_at: DateTime<Utc>,
    pub ingested: bool,
}

/// Hourly download statistics for charts.
#[derive(Debug, Clone)]
pub struct HourlyStats {
    pub hour: String,
    pub download_count: u64,
    pub bytes_downloaded: u64,
}

/// Statistics about download state.
#[derive(Debug, Clone)]
pub struct DownloadStats {
    pub pending: u64,
    pub in_progress: u64,
    pub failed: u64,
    pub completed: u64,
    pub total_bytes_downloaded: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_state_create_and_update() {
        let state = DownloadState::open_memory().await.unwrap();

        // Create a new download
        let progress = state
            .get_or_create("http://example.com/file.grib2", "file.grib2")
            .await
            .unwrap();

        assert_eq!(progress.downloaded_bytes, 0);
        assert_eq!(progress.total_bytes, None);

        // Update progress
        let mut updated = progress.clone();
        updated.total_bytes = Some(1000);
        updated.downloaded_bytes = 500;

        state
            .update_progress("http://example.com/file.grib2", &updated)
            .await
            .unwrap();

        // Mark complete
        state
            .update_status("http://example.com/file.grib2", DownloadStatus::Completed)
            .await
            .unwrap();

        // Check it's in completed
        assert!(state
            .is_already_downloaded("http://example.com/file.grib2")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_resume_downloads() {
        let state = DownloadState::open_memory().await.unwrap();

        // Queue some downloads
        state
            .queue_download("http://example.com/file1.grib2", "file1.grib2", "gfs")
            .await
            .unwrap();
        state
            .queue_download("http://example.com/file2.grib2", "file2.grib2", "gfs")
            .await
            .unwrap();

        // Get in-progress
        let in_progress = state.get_in_progress().await.unwrap();
        assert_eq!(in_progress.len(), 2);
    }
}
