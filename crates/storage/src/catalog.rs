//! Metadata catalog using PostgreSQL.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, FromRow, PgPool};
use uuid::Uuid;

use wms_common::{BoundingBox, LayerId, WmsError, WmsResult};

/// Database connection pool and catalog operations.
pub struct Catalog {
    pool: PgPool,
}

impl Catalog {
    /// Create a new catalog connection from database URL with default pool size.
    pub async fn connect(database_url: &str) -> WmsResult<Self> {
        Self::connect_with_pool_size(database_url, 10).await
    }

    /// Create a new catalog connection from database URL with custom pool size.
    pub async fn connect_with_pool_size(database_url: &str, max_connections: u32) -> WmsResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Connection failed: {}", e)))?;

        Ok(Self { pool })
    }

    /// Run database migrations.
    pub async fn migrate(&self) -> WmsResult<()> {
        // Split SQL statements and execute them individually
        for statement in SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| WmsError::DatabaseError(format!("Migration failed: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Register a new ingested dataset.
    pub async fn register_dataset(&self, entry: &CatalogEntry) -> WmsResult<Uuid> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO datasets (
                id, model, parameter, level, 
                reference_time, forecast_hour, valid_time,
                bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y,
                storage_path, file_size, ingested_at, status
            ) VALUES (
                $1, $2, $3, $4,
                $5, $6, $7,
                $8, $9, $10, $11,
                $12, $13, $14, $15
            )
            ON CONFLICT (model, parameter, level, reference_time, forecast_hour)
            DO UPDATE SET
                storage_path = EXCLUDED.storage_path,
                file_size = EXCLUDED.file_size,
                ingested_at = EXCLUDED.ingested_at,
                status = EXCLUDED.status
            "#,
        )
        .bind(id)
        .bind(&entry.model)
        .bind(&entry.parameter)
        .bind(&entry.level)
        .bind(entry.reference_time)
        .bind(entry.forecast_hour as i32)
        .bind(entry.valid_time())
        .bind(entry.bbox.min_x)
        .bind(entry.bbox.min_y)
        .bind(entry.bbox.max_x)
        .bind(entry.bbox.max_y)
        .bind(&entry.storage_path)
        .bind(entry.file_size as i64)
        .bind(Utc::now())
        .bind("available")
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Insert failed: {}", e)))?;

        Ok(id)
    }

    /// Find datasets matching query criteria.
    pub async fn find_datasets(&self, query: &DatasetQuery) -> WmsResult<Vec<CatalogEntry>> {
        // TODO: implement dynamic query building using these variables
        let mut _sql = String::from(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets WHERE status = 'available'",
        );

        let mut _params: Vec<String> = Vec::new();
        let mut _param_idx = 1;

        if let Some(model) = &query.model {
            _sql.push_str(&format!(" AND model = ${}", _param_idx));
            _params.push(model.clone());
            _param_idx += 1;
        }

        if let Some(parameter) = &query.parameter {
            _sql.push_str(&format!(" AND parameter = ${}", _param_idx));
            _params.push(parameter.clone());
            _param_idx += 1;
        }

        // For now, use a simpler approach - full query building would need runtime SQL
        let rows = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets WHERE status = 'available' \
             ORDER BY valid_time DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get the most recent dataset for a layer.
    pub async fn get_latest(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY valid_time DESC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Find dataset closest to requested valid time.
    pub async fn find_by_time(
        &self,
        model: &str,
        parameter: &str,
        valid_time: DateTime<Utc>,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY ABS(EXTRACT(EPOCH FROM (valid_time - $3))) ASC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .bind(valid_time)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Find dataset by forecast hour.
    pub async fn find_by_forecast_hour(
        &self,
        model: &str,
        parameter: &str,
        forecast_hour: u32,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND forecast_hour = $3 AND status = 'available' \
             ORDER BY reference_time DESC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .bind(forecast_hour as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Get available time steps for a layer.
    pub async fn get_available_times(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<DateTime<Utc>>> {
        let rows = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT DISTINCT valid_time FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY valid_time DESC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get list of available models.
    pub async fn list_models(&self) -> WmsResult<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT model FROM datasets WHERE status = 'available' ORDER BY model",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get list of parameters for a model.
    pub async fn list_parameters(&self, model: &str) -> WmsResult<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT parameter FROM datasets WHERE model = $1 AND status = 'available' ORDER BY parameter"
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get recently ingested datasets (last N minutes).
    pub async fn get_recent_ingestions(&self, minutes: i64) -> WmsResult<Vec<CatalogEntry>> {
        let cutoff = Utc::now() - chrono::Duration::minutes(minutes);
        let rows = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE ingested_at > $1 AND status = 'available' \
             ORDER BY ingested_at DESC LIMIT 50",
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Mark old datasets for cleanup.
    pub async fn mark_expired(&self, older_than: DateTime<Utc>) -> WmsResult<u64> {
        let result = sqlx::query(
            "UPDATE datasets SET status = 'expired' WHERE valid_time < $1 AND status = 'available'",
        )
        .bind(older_than)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Update failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Mark old datasets for a specific model as expired based on retention hours.
    pub async fn mark_model_expired(&self, model: &str, older_than: DateTime<Utc>) -> WmsResult<u64> {
        let result = sqlx::query(
            "UPDATE datasets SET status = 'expired' WHERE model = $1 AND valid_time < $2 AND status = 'available'",
        )
        .bind(model)
        .bind(older_than)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Update failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Get storage paths for expired datasets (for deletion from object storage).
    pub async fn get_expired_storage_paths(&self) -> WmsResult<Vec<String>> {
        let paths = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT storage_path FROM datasets WHERE status = 'expired'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(paths)
    }

    /// Delete expired dataset records from the database.
    /// Call this AFTER deleting files from object storage.
    pub async fn delete_expired(&self) -> WmsResult<u64> {
        let result = sqlx::query("DELETE FROM datasets WHERE status = 'expired'")
            .execute(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Delete failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Get count of expired datasets.
    pub async fn count_expired(&self) -> WmsResult<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM datasets WHERE status = 'expired'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(count)
    }

    /// Preview what datasets would be expired for a specific model based on retention.
    /// Returns count and total size of datasets that would be purged.
    pub async fn preview_model_expiration(
        &self,
        model: &str,
        older_than: DateTime<Utc>,
    ) -> WmsResult<PurgePreview> {
        let row = sqlx::query_as::<_, (i64, Option<i64>)>(
            "SELECT COUNT(*), COALESCE(SUM(file_size), 0) FROM datasets \
             WHERE model = $1 AND valid_time < $2 AND status = 'available'",
        )
        .bind(model)
        .bind(older_than)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(PurgePreview {
            dataset_count: row.0 as u64,
            total_size_bytes: row.1.unwrap_or(0) as u64,
        })
    }

    /// Get the oldest dataset time for a model (for calculating when next purge will happen).
    pub async fn get_oldest_dataset_time(&self, model: &str) -> WmsResult<Option<DateTime<Utc>>> {
        // MIN returns NULL when no rows match, so we use Option<Option<DateTime>>
        let oldest = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT MIN(valid_time) FROM datasets WHERE model = $1 AND status = 'available'",
        )
        .bind(model)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(oldest)
    }

    /// Get available model run times (reference_time) for a model/parameter.
    pub async fn get_available_runs(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<DateTime<Utc>>> {
        let rows = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT DISTINCT reference_time FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY reference_time DESC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get available forecast hours for a model/parameter.
    pub async fn get_available_forecast_hours(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<i32>> {
        let rows = sqlx::query_scalar::<_, i32>(
            "SELECT DISTINCT forecast_hour FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY forecast_hour ASC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get available levels for a model/parameter.
    pub async fn get_available_levels(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT level FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY level ASC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Find dataset by forecast hour and level.
    pub async fn find_by_forecast_hour_and_level(
        &self,
        model: &str,
        parameter: &str,
        forecast_hour: u32,
        level: &str,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND forecast_hour = $3 AND level = $4 AND status = 'available' \
             ORDER BY reference_time DESC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .bind(forecast_hour as i32)
        .bind(level)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Get the most recent dataset for a layer at a specific level.
    pub async fn get_latest_at_level(
        &self,
        model: &str,
        parameter: &str,
        level: &str,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND level = $3 AND status = 'available' \
             ORDER BY valid_time DESC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .bind(level)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Get dataset from latest run with earliest forecast hour.
    /// This is the preferred default: most recent model run, but showing analysis/F00.
    pub async fn get_latest_run_earliest_forecast(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY reference_time DESC, forecast_hour ASC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Get dataset from latest run with earliest forecast hour at a specific level.
    pub async fn get_latest_run_earliest_forecast_at_level(
        &self,
        model: &str,
        parameter: &str,
        level: &str,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND level = $3 AND status = 'available' \
             ORDER BY reference_time DESC, forecast_hour ASC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .bind(level)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(row.map(|r| r.into()))
    }

    /// Get available runs and forecast hours for all layers of a model.
    /// Returns (runs, forecast_hours) where runs are ISO8601 strings and forecast_hours are integers.
    pub async fn get_model_dimensions(&self, model: &str) -> WmsResult<(Vec<String>, Vec<i32>)> {
        // Get distinct reference times, truncated to nearest minute to group similar ingestion times
        let runs = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT DISTINCT DATE_TRUNC('minute', reference_time) as ref_time FROM datasets \
             WHERE model = $1 AND status = 'available' \
             ORDER BY ref_time DESC",
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        let forecast_hours = sqlx::query_scalar::<_, i32>(
            "SELECT DISTINCT forecast_hour FROM datasets \
             WHERE model = $1 AND status = 'available' \
             ORDER BY forecast_hour ASC",
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Format runs as ISO8601 strings
        let run_strings: Vec<String> = runs
            .into_iter()
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .collect();

        Ok((run_strings, forecast_hours))
    }
    
    /// Get the geographic bounding box for a model
    /// Returns the union of all dataset bounding boxes for the model
    pub async fn get_model_bbox(&self, model: &str) -> WmsResult<BoundingBox> {
        let result = sqlx::query_as::<_, (f64, f64, f64, f64)>(
            "SELECT \
                MIN(bbox_min_x) as min_x, \
                MIN(bbox_min_y) as min_y, \
                MAX(bbox_max_x) as max_x, \
                MAX(bbox_max_y) as max_y \
             FROM datasets \
             WHERE model = $1 AND status = 'available'",
        )
        .bind(model)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;
        
        Ok(BoundingBox::new(result.0, result.1, result.2, result.3))
    }
    
    /// Get list of models that have available data (alias for list_models).
    pub async fn get_available_models(&self) -> WmsResult<Vec<String>> {
        self.list_models().await
    }
    
    /// Get recent entries for a model (for cache warming).
    /// Returns the N most recent unique observations, ordered by reference_time DESC.
    pub async fn get_recent_entries(
        &self,
        model: &str,
        limit: usize,
    ) -> WmsResult<Vec<CatalogEntry>> {
        let rows = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND status = 'available' \
             ORDER BY reference_time DESC, parameter ASC \
             LIMIT $2",
        )
        .bind(model)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get the latest dataset for a model, optionally filtering by parameter.
    pub async fn get_latest_dataset(
        &self,
        model: &str,
        parameter: Option<&str>,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = if let Some(param) = parameter {
            sqlx::query_as::<_, DatasetRow>(
                "SELECT model, parameter, level, reference_time, forecast_hour, \
                 bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                 storage_path, file_size FROM datasets \
                 WHERE model = $1 AND parameter = $2 AND status = 'available' \
                 ORDER BY reference_time DESC, forecast_hour ASC LIMIT 1",
            )
            .bind(model)
            .bind(param)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?
        } else {
            sqlx::query_as::<_, DatasetRow>(
                "SELECT model, parameter, level, reference_time, forecast_hour, \
                 bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                 storage_path, file_size FROM datasets \
                 WHERE model = $1 AND status = 'available' \
                 ORDER BY reference_time DESC, forecast_hour ASC LIMIT 1",
            )
            .bind(model)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?
        };

        Ok(row.map(|r| r.into()))
    }
}

/// A catalog entry representing an ingested dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: DateTime<Utc>,
    pub forecast_hour: u32,
    pub bbox: BoundingBox,
    pub storage_path: String,
    pub file_size: u64,
}

impl CatalogEntry {
    pub fn valid_time(&self) -> DateTime<Utc> {
        self.reference_time + chrono::Duration::hours(self.forecast_hour as i64)
    }

    pub fn layer_id(&self) -> LayerId {
        LayerId::new(format!("{}:{}", self.model, self.parameter))
    }
}

/// Query parameters for finding datasets.
#[derive(Debug, Default)]
pub struct DatasetQuery {
    pub model: Option<String>,
    pub parameter: Option<String>,
    pub level: Option<String>,
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub bbox: Option<BoundingBox>,
}

/// Preview of what would be purged for a model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PurgePreview {
    /// Number of datasets that would be purged
    pub dataset_count: u64,
    /// Total size of datasets that would be purged
    pub total_size_bytes: u64,
}

/// Internal row type for database queries.
#[derive(FromRow)]
struct DatasetRow {
    model: String,
    parameter: String,
    level: String,
    reference_time: DateTime<Utc>,
    forecast_hour: i32,
    bbox_min_x: f64,
    bbox_min_y: f64,
    bbox_max_x: f64,
    bbox_max_y: f64,
    storage_path: String,
    file_size: i64,
}

impl From<DatasetRow> for CatalogEntry {
    fn from(row: DatasetRow) -> Self {
        CatalogEntry {
            model: row.model,
            parameter: row.parameter,
            level: row.level,
            reference_time: row.reference_time,
            forecast_hour: row.forecast_hour as u32,
            bbox: BoundingBox::new(
                row.bbox_min_x,
                row.bbox_min_y,
                row.bbox_max_x,
                row.bbox_max_y,
            ),
            storage_path: row.storage_path,
            file_size: row.file_size as u64,
        }
    }
}

/// Database schema SQL.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS datasets (
    id UUID PRIMARY KEY,
    model VARCHAR(50) NOT NULL,
    parameter VARCHAR(100) NOT NULL,
    level VARCHAR(50) NOT NULL,
    reference_time TIMESTAMPTZ NOT NULL,
    forecast_hour INTEGER NOT NULL,
    valid_time TIMESTAMPTZ NOT NULL,
    bbox_min_x DOUBLE PRECISION NOT NULL,
    bbox_min_y DOUBLE PRECISION NOT NULL,
    bbox_max_x DOUBLE PRECISION NOT NULL,
    bbox_max_y DOUBLE PRECISION NOT NULL,
    storage_path TEXT NOT NULL,
    file_size BIGINT NOT NULL,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status VARCHAR(20) NOT NULL DEFAULT 'available',
    
    UNIQUE(model, parameter, level, reference_time, forecast_hour)
);

CREATE INDEX IF NOT EXISTS idx_datasets_model_param ON datasets(model, parameter);
CREATE INDEX IF NOT EXISTS idx_datasets_valid_time ON datasets(valid_time DESC);
CREATE INDEX IF NOT EXISTS idx_datasets_status ON datasets(status);

CREATE TABLE IF NOT EXISTS layer_styles (
    id UUID PRIMARY KEY,
    layer_id VARCHAR(200) NOT NULL,
    style_name VARCHAR(100) NOT NULL,
    style_config JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(layer_id, style_name)
);
"#;
