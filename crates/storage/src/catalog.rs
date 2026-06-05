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
    pub async fn connect_with_pool_size(
        database_url: &str,
        max_connections: u32,
    ) -> WmsResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Connection failed: {}", e)))?;

        Ok(Self { pool })
    }

    /// Run database migrations for gridded datasets.
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

    /// Run database migrations for point observations (requires PostGIS).
    ///
    /// This creates the `locations` and `observations` tables with PostGIS
    /// spatial indexing. Call this after `migrate()` if observation support is needed.
    pub async fn migrate_observations(&self) -> WmsResult<()> {
        for statement in OBSERVATIONS_SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        WmsError::DatabaseError(format!("Observations migration failed: {}", e))
                    })?;
            }
        }

        Ok(())
    }

    /// Run database migrations for storm events (requires PostGIS).
    ///
    /// Creates the `tiger_counties` and `storm_events` tables plus the
    /// `mv_county_event_counts` materialized view. Call this after
    /// `migrate_observations()` (which enables PostGIS).
    pub async fn migrate_storm_events(&self) -> WmsResult<()> {
        for statement in STORM_EVENTS_SCHEMA_SQL.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| {
                        WmsError::DatabaseError(format!("Storm events migration failed: {}", e))
                    })?;
            }
        }

        Ok(())
    }

    /// Get a reference to the underlying connection pool.
    ///
    /// This allows creating an `ObservationCatalog` that shares the same pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Clone the connection pool for use in other components.
    pub fn pool_clone(&self) -> PgPool {
        self.pool.clone()
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
                storage_path, file_size, ingested_at, status, zarr_metadata
            ) VALUES (
                $1, $2, $3, $4,
                $5, $6, $7,
                $8, $9, $10, $11,
                $12, $13, $14, $15, $16
            )
            ON CONFLICT (model, parameter, level, reference_time, forecast_hour)
            DO UPDATE SET
                storage_path = EXCLUDED.storage_path,
                file_size = EXCLUDED.file_size,
                ingested_at = EXCLUDED.ingested_at,
                status = EXCLUDED.status,
                zarr_metadata = EXCLUDED.zarr_metadata
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
        .bind(&entry.zarr_metadata)
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
             storage_path, file_size, zarr_metadata FROM datasets WHERE status = 'available'",
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
             storage_path, file_size, zarr_metadata FROM datasets WHERE status = 'available' \
             ORDER BY valid_time DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get aggregated statistics per model.
    /// Returns (model_id, dataset_count, parameter_count, last_ingest_time, parameters_list)
    pub async fn get_model_stats(&self) -> WmsResult<Vec<ModelStats>> {
        #[derive(sqlx::FromRow)]
        struct ModelStatsRow {
            model: String,
            dataset_count: i64,
            param_count: i64,
            last_ingest: Option<chrono::DateTime<Utc>>,
        }

        let rows = sqlx::query_as::<_, ModelStatsRow>(
            "SELECT model, COUNT(*) as dataset_count, \
             COUNT(DISTINCT parameter) as param_count, \
             MAX(reference_time) as last_ingest \
             FROM datasets WHERE status = 'available' \
             GROUP BY model ORDER BY model",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Also get parameter names per model
        #[derive(sqlx::FromRow)]
        struct ParamRow {
            model: String,
            parameter: String,
        }

        let param_rows = sqlx::query_as::<_, ParamRow>(
            "SELECT DISTINCT model, parameter FROM datasets WHERE status = 'available' ORDER BY model, parameter",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Group parameters by model
        let mut params_map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for row in param_rows {
            params_map.entry(row.model).or_default().push(row.parameter);
        }

        Ok(rows
            .into_iter()
            .map(|r| ModelStats {
                model: r.model.clone(),
                dataset_count: r.dataset_count as u64,
                parameter_count: r.param_count as u64,
                last_ingest: r.last_ingest,
                parameters: params_map.remove(&r.model).unwrap_or_default(),
            })
            .collect())
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
             storage_path, file_size, zarr_metadata FROM datasets \
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

    /// Find dataset closest to requested valid time at a specific level.
    pub async fn find_by_time_and_level(
        &self,
        model: &str,
        parameter: &str,
        valid_time: DateTime<Utc>,
        level: &str,
    ) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size, zarr_metadata FROM datasets \
             WHERE model = $1 AND parameter = $2 AND level = $4 AND status = 'available' \
             ORDER BY ABS(EXTRACT(EPOCH FROM (valid_time - $3))) ASC LIMIT 1",
        )
        .bind(model)
        .bind(parameter)
        .bind(valid_time)
        .bind(level)
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
    /// Uses reference_time (model run initialization time) for simpler, more predictable cleanup.
    pub async fn mark_model_expired(
        &self,
        model: &str,
        older_than: DateTime<Utc>,
    ) -> WmsResult<u64> {
        let result = sqlx::query(
            "UPDATE datasets SET status = 'expired' WHERE model = $1 AND reference_time < $2 AND status = 'available'",
        )
        .bind(model)
        .bind(older_than)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Update failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Mark old datasets for a specific model as expired, but protect specified runs.
    ///
    /// This is the main retention safeguard method. It marks datasets as expired if:
    /// - reference_time < older_than (exceeds retention period)
    /// - reference_time is NOT in the protected_runs list
    ///
    /// This ensures we always keep at least N recent runs even during ingestion outages.
    pub async fn mark_model_expired_except_runs(
        &self,
        model: &str,
        older_than: DateTime<Utc>,
        protected_runs: &[DateTime<Utc>],
    ) -> WmsResult<u64> {
        if protected_runs.is_empty() {
            // No protection, use standard expiration
            return self.mark_model_expired(model, older_than).await;
        }

        // Build the query with protected runs excluded
        // We need to use a parameterized IN clause
        let placeholders: Vec<String> = (3..=protected_runs.len() + 2)
            .map(|i| format!("${}", i))
            .collect();
        let in_clause = placeholders.join(", ");

        let query = format!(
            "UPDATE datasets SET status = 'expired' \
             WHERE model = $1 AND reference_time < $2 AND status = 'available' \
             AND reference_time NOT IN ({})",
            in_clause
        );

        let mut query_builder = sqlx::query(&query).bind(model).bind(older_than);

        for run in protected_runs {
            query_builder = query_builder.bind(*run);
        }

        let result = query_builder
            .execute(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Update failed: {}", e)))?;

        Ok(result.rows_affected())
    }

    /// Get all runs (reference_times) for a model with their dataset counts.
    ///
    /// Returns a list of (reference_time, dataset_count) tuples, ordered by reference_time DESC.
    /// This is used to determine run completeness and decide which runs to protect.
    pub async fn get_model_runs_with_counts(
        &self,
        model: &str,
    ) -> WmsResult<Vec<(DateTime<Utc>, i64)>> {
        let rows = sqlx::query_as::<_, (DateTime<Utc>, i64)>(
            "SELECT reference_time, COUNT(*) as count \
             FROM datasets \
             WHERE model = $1 AND status = 'available' \
             GROUP BY reference_time \
             ORDER BY reference_time DESC",
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get the count of distinct forecast hours for a specific run.
    ///
    /// This is used to determine if a run is "complete" by comparing against expected forecast hours.
    pub async fn count_run_forecast_hours(
        &self,
        model: &str,
        reference_time: DateTime<Utc>,
    ) -> WmsResult<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT forecast_hour) \
             FROM datasets \
             WHERE model = $1 AND reference_time = $2 AND status = 'available'",
        )
        .bind(model)
        .bind(reference_time)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(count)
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
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM datasets WHERE status = 'expired'")
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
             storage_path, file_size, zarr_metadata FROM datasets \
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

    /// Get the forecast entry from the latest run with valid_time closest to now.
    ///
    /// For a given model/parameter/level, this finds:
    /// 1. The latest available model run (reference_time)
    /// 2. The forecast hour from that run where reference_time + forecast_hour is closest to now
    ///
    /// This is useful for getting the most relevant "current" forecast data.
    pub async fn get_forecast_closest_to_now(
        &self,
        model: &str,
        parameter: &str,
        level: Option<&str>,
    ) -> WmsResult<Option<CatalogEntry>> {
        let now = Utc::now();

        // Query that:
        // 1. Filters to latest reference_time
        // 2. Computes valid_time = reference_time + forecast_hour * interval '1 hour'
        // 3. Orders by absolute difference from now
        // 4. Takes the closest one
        let row = if let Some(lvl) = level {
            sqlx::query_as::<_, DatasetRow>(
                "WITH latest_run AS (
                    SELECT MAX(reference_time) as ref_time
                    FROM datasets
                    WHERE model = $1 AND parameter = $2 AND level = $3 AND status = 'available'
                )
                SELECT d.model, d.parameter, d.level, d.reference_time, d.forecast_hour,
                       d.bbox_min_x, d.bbox_min_y, d.bbox_max_x, d.bbox_max_y,
                       d.storage_path, d.file_size, d.zarr_metadata
                FROM datasets d, latest_run lr
                WHERE d.model = $1 AND d.parameter = $2 AND d.level = $3
                  AND d.status = 'available'
                  AND d.reference_time = lr.ref_time
                ORDER BY ABS(EXTRACT(EPOCH FROM (d.reference_time + d.forecast_hour * interval '1 hour' - $4)))
                LIMIT 1",
            )
            .bind(model)
            .bind(parameter)
            .bind(lvl)
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?
        } else {
            sqlx::query_as::<_, DatasetRow>(
                "WITH latest_run AS (
                    SELECT MAX(reference_time) as ref_time
                    FROM datasets
                    WHERE model = $1 AND parameter = $2 AND status = 'available'
                )
                SELECT d.model, d.parameter, d.level, d.reference_time, d.forecast_hour,
                       d.bbox_min_x, d.bbox_min_y, d.bbox_max_x, d.bbox_max_y,
                       d.storage_path, d.file_size, d.zarr_metadata
                FROM datasets d, latest_run lr
                WHERE d.model = $1 AND d.parameter = $2
                  AND d.status = 'available'
                  AND d.reference_time = lr.ref_time
                ORDER BY ABS(EXTRACT(EPOCH FROM (d.reference_time + d.forecast_hour * interval '1 hour' - $3)))
                LIMIT 1",
            )
            .bind(model)
            .bind(parameter)
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?
        };

        Ok(row.map(|r| r.into()))
    }

    /// Get all forecast entries from the latest run for a parameter, one per level,
    /// with forecast hour closest to now for each.
    ///
    /// Returns entries for all available levels at the optimal forecast hour.
    pub async fn get_all_levels_forecast_closest_to_now(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<CatalogEntry>> {
        let now = Utc::now();

        // Query that:
        // 1. Finds the latest reference_time
        // 2. For that run, finds the forecast_hour closest to now
        // 3. Returns all levels at that reference_time + forecast_hour
        let rows = sqlx::query_as::<_, DatasetRow>(
            "WITH latest_run AS (
                SELECT MAX(reference_time) as ref_time
                FROM datasets
                WHERE model = $1 AND parameter = $2 AND status = 'available'
            ),
            best_forecast AS (
                SELECT d.forecast_hour
                FROM datasets d, latest_run lr
                WHERE d.model = $1 AND d.parameter = $2
                  AND d.status = 'available'
                  AND d.reference_time = lr.ref_time
                ORDER BY ABS(EXTRACT(EPOCH FROM (d.reference_time + d.forecast_hour * interval '1 hour' - $3)))
                LIMIT 1
            )
            SELECT d.model, d.parameter, d.level, d.reference_time, d.forecast_hour,
                   d.bbox_min_x, d.bbox_min_y, d.bbox_max_x, d.bbox_max_y,
                   d.storage_path, d.file_size, d.zarr_metadata
            FROM datasets d, latest_run lr, best_forecast bf
            WHERE d.model = $1 AND d.parameter = $2
              AND d.status = 'available'
              AND d.reference_time = lr.ref_time
              AND d.forecast_hour = bf.forecast_hour
            ORDER BY d.level",
        )
        .bind(model)
        .bind(parameter)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get the latest observation entry (for GOES, MRMS, etc.) closest to now.
    ///
    /// For observation data, reference_time IS the observation time and forecast_hour is 0.
    pub async fn get_observation_closest_to_now(
        &self,
        model: &str,
        parameter: &str,
        level: Option<&str>,
    ) -> WmsResult<Option<CatalogEntry>> {
        let now = Utc::now();

        let row = if let Some(lvl) = level {
            sqlx::query_as::<_, DatasetRow>(
                "SELECT model, parameter, level, reference_time, forecast_hour, \
                 bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                 storage_path, file_size, zarr_metadata FROM datasets \
                 WHERE model = $1 AND parameter = $2 AND level = $3 AND status = 'available' \
                 ORDER BY ABS(EXTRACT(EPOCH FROM (reference_time - $4))) \
                 LIMIT 1",
            )
            .bind(model)
            .bind(parameter)
            .bind(lvl)
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?
        } else {
            sqlx::query_as::<_, DatasetRow>(
                "SELECT model, parameter, level, reference_time, forecast_hour, \
                 bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                 storage_path, file_size, zarr_metadata FROM datasets \
                 WHERE model = $1 AND parameter = $2 AND status = 'available' \
                 ORDER BY ABS(EXTRACT(EPOCH FROM (reference_time - $3))) \
                 LIMIT 1",
            )
            .bind(model)
            .bind(parameter)
            .bind(now)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?
        };

        Ok(row.map(|r| r.into()))
    }

    /// Get all levels for observation data closest to now.
    pub async fn get_all_levels_observation_closest_to_now(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<CatalogEntry>> {
        let now = Utc::now();

        // Find the observation time closest to now, then get all levels at that time
        let rows = sqlx::query_as::<_, DatasetRow>(
            "WITH best_time AS (
                SELECT reference_time as obs_time
                FROM datasets
                WHERE model = $1 AND parameter = $2 AND status = 'available'
                ORDER BY ABS(EXTRACT(EPOCH FROM (reference_time - $3)))
                LIMIT 1
            )
            SELECT d.model, d.parameter, d.level, d.reference_time, d.forecast_hour,
                   d.bbox_min_x, d.bbox_min_y, d.bbox_max_x, d.bbox_max_y,
                   d.storage_path, d.file_size, d.zarr_metadata
            FROM datasets d, best_time bt
            WHERE d.model = $1 AND d.parameter = $2
              AND d.status = 'available'
              AND d.reference_time = bt.obs_time
            ORDER BY d.level",
        )
        .bind(model)
        .bind(parameter)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
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
             storage_path, file_size, zarr_metadata FROM datasets \
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
                 storage_path, file_size, zarr_metadata FROM datasets \
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
                 storage_path, file_size, zarr_metadata FROM datasets \
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

    // ========== Sync/Orphan Detection Methods ==========

    /// Get all storage paths from the database (for sync validation).
    pub async fn get_all_storage_paths(&self) -> WmsResult<Vec<String>> {
        let paths = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT storage_path FROM datasets WHERE status = 'available'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(paths)
    }

    /// Delete database records for paths that no longer exist in storage.
    /// Returns the number of records deleted.
    pub async fn delete_orphan_records(&self, orphan_paths: &[String]) -> WmsResult<u64> {
        if orphan_paths.is_empty() {
            return Ok(0);
        }

        // Delete in batches to avoid query size limits
        let mut total_deleted = 0u64;
        for chunk in orphan_paths.chunks(100) {
            let placeholders: Vec<String> = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("${}", i + 1))
                .collect();
            let sql = format!(
                "DELETE FROM datasets WHERE storage_path IN ({})",
                placeholders.join(", ")
            );

            let mut query = sqlx::query(&sql);
            for path in chunk {
                query = query.bind(path);
            }

            let result = query
                .execute(&self.pool)
                .await
                .map_err(|e| WmsError::DatabaseError(format!("Delete failed: {}", e)))?;

            total_deleted += result.rows_affected();
        }

        Ok(total_deleted)
    }

    /// Get count of available datasets.
    pub async fn count_available(&self) -> WmsResult<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM datasets WHERE status = 'available'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(count)
    }

    /// Get total database record count (all statuses).
    pub async fn count_all(&self) -> WmsResult<i64> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM datasets")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(count)
    }

    /// Get detailed per-parameter statistics for the database panel.
    /// Returns stats for each model+parameter combination including counts and time ranges.
    pub async fn get_detailed_parameter_stats(&self) -> WmsResult<Vec<ParameterStats>> {
        #[derive(sqlx::FromRow)]
        struct ParamStatsRow {
            model: String,
            parameter: String,
            count: i64,
            oldest: Option<chrono::DateTime<Utc>>,
            newest: Option<chrono::DateTime<Utc>>,
            total_size: Option<i64>, // Cast in SQL to avoid NUMERIC
        }

        let rows = sqlx::query_as::<_, ParamStatsRow>(
            "SELECT model, parameter, COUNT(*) as count, \
             MIN(valid_time) as oldest, MAX(valid_time) as newest, \
             CAST(COALESCE(SUM(file_size), 0) AS BIGINT) as total_size \
             FROM datasets WHERE status = 'available' \
             GROUP BY model, parameter \
             ORDER BY model, parameter",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| ParameterStats {
                model: r.model,
                parameter: r.parameter,
                count: r.count as u64,
                oldest: r.oldest,
                newest: r.newest,
                total_size_bytes: r.total_size.unwrap_or(0) as u64,
            })
            .collect())
    }

    /// Get all datasets for a specific model and parameter.
    /// Returns full dataset details for drill-down views.
    pub async fn get_datasets_for_parameter(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Vec<DatasetInfo>> {
        #[derive(sqlx::FromRow)]
        struct DatasetRow {
            model: String,
            parameter: String,
            level: String,
            reference_time: chrono::DateTime<Utc>,
            forecast_hour: i32,
            valid_time: chrono::DateTime<Utc>,
            storage_path: String,
            file_size: i64,
        }

        let rows = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             valid_time, storage_path, file_size \
             FROM datasets WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY valid_time DESC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows
            .into_iter()
            .map(|r| DatasetInfo {
                model: r.model,
                parameter: r.parameter,
                level: r.level,
                reference_time: r.reference_time,
                forecast_hour: r.forecast_hour as u32,
                valid_time: r.valid_time,
                storage_path: r.storage_path,
                file_size: r.file_size as u64,
            })
            .collect())
    }

    /// Get the temporal extent (min/max valid times) for a model.
    /// Returns (oldest_valid_time, newest_valid_time) or None if no data exists.
    pub async fn get_model_temporal_extent(
        &self,
        model: &str,
    ) -> WmsResult<Option<(DateTime<Utc>, DateTime<Utc>)>> {
        let result = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
            "SELECT MIN(valid_time), MAX(valid_time) \
             FROM datasets \
             WHERE model = $1 AND status = 'available'",
        )
        .bind(model)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        match result {
            (Some(min), Some(max)) => Ok(Some((min, max))),
            _ => Ok(None),
        }
    }

    /// Get all available valid times for a model (for populating temporal extent values).
    /// Returns unique valid times sorted ascending.
    pub async fn get_model_valid_times(&self, model: &str) -> WmsResult<Vec<DateTime<Utc>>> {
        let times = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT DISTINCT valid_time FROM datasets \
             WHERE model = $1 AND status = 'available' \
             ORDER BY valid_time ASC",
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(times)
    }

    /// Get the forecast time range for a specific model run.
    /// Returns (start_time, end_time) where start_time is the run time
    /// and end_time is the latest valid_time from all forecasts in that run.
    pub async fn get_run_forecast_range(
        &self,
        model: &str,
        reference_time: DateTime<Utc>,
    ) -> WmsResult<Option<(DateTime<Utc>, DateTime<Utc>)>> {
        let result = sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
            "SELECT MIN(valid_time), MAX(valid_time) \
             FROM datasets \
             WHERE model = $1 AND reference_time = $2 AND status = 'available'",
        )
        .bind(model)
        .bind(reference_time)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        match result {
            (Some(min), Some(max)) => Ok(Some((min, max))),
            _ => Ok(None),
        }
    }

    /// Get availability information for a specific parameter.
    /// Returns None if no data exists for this model/parameter combination.
    /// Used by capabilities generation to ensure we only advertise available data.
    pub async fn get_parameter_availability(
        &self,
        model: &str,
        parameter: &str,
    ) -> WmsResult<Option<ParameterAvailability>> {
        // First check if any data exists for this model/parameter
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available'",
        )
        .bind(model)
        .bind(parameter)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        if count == 0 {
            return Ok(None);
        }

        // Get distinct reference times (RUN times for forecast, TIME for observation)
        let times = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT DISTINCT DATE_TRUNC('minute', reference_time) as ref_time FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY ref_time DESC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Get distinct forecast hours
        let forecast_hours = sqlx::query_scalar::<_, i32>(
            "SELECT DISTINCT forecast_hour FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY forecast_hour ASC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Get distinct levels
        let levels = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT level FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY level ASC",
        )
        .bind(model)
        .bind(parameter)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Get bounding box
        let bbox_result = sqlx::query_as::<_, (f64, f64, f64, f64)>(
            "SELECT \
                MIN(bbox_min_x) as min_x, \
                MIN(bbox_min_y) as min_y, \
                MAX(bbox_max_x) as max_x, \
                MAX(bbox_max_y) as max_y \
             FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available'",
        )
        .bind(model)
        .bind(parameter)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        // Format times as ISO8601 strings
        let time_strings: Vec<String> = times
            .into_iter()
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .collect();

        Ok(Some(ParameterAvailability {
            times: time_strings,
            forecast_hours,
            levels,
            bbox: BoundingBox::new(bbox_result.0, bbox_result.1, bbox_result.2, bbox_result.3),
        }))
    }

    // =========================================================================
    // Methods for custom dimension support (run/forecast-hour)
    // =========================================================================

    /// Get all available forecast hours for a specific model run.
    ///
    /// Returns a sorted list of forecast hours (ascending) that are available
    /// for the given model at the specified reference time.
    pub async fn get_run_forecast_hours(
        &self,
        model: &str,
        reference_time: DateTime<Utc>,
    ) -> WmsResult<Vec<i32>> {
        let rows = sqlx::query_scalar::<_, i32>(
            "SELECT DISTINCT forecast_hour FROM datasets \
             WHERE model = $1 AND reference_time = $2 AND status = 'available' \
             ORDER BY forecast_hour ASC",
        )
        .bind(model)
        .bind(reference_time)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }

    /// Get comprehensive info about the latest model run.
    ///
    /// Returns information about the most recent model run including:
    /// - The reference time
    /// - All available forecast hours for that run
    /// - The valid time range (start and end)
    ///
    /// Returns None if no runs exist for the model.
    pub async fn get_latest_run_info(&self, model: &str) -> WmsResult<Option<LatestRunInfo>> {
        // Get the latest reference_time
        let latest_ref = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT reference_time FROM datasets \
             WHERE model = $1 AND status = 'available' \
             ORDER BY reference_time DESC LIMIT 1",
        )
        .bind(model)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        let Some(reference_time) = latest_ref else {
            return Ok(None);
        };

        // Get all forecast hours for this run
        let forecast_hours = self.get_run_forecast_hours(model, reference_time).await?;

        // Get the valid time range for this run
        let time_range = sqlx::query_as::<_, (DateTime<Utc>, DateTime<Utc>)>(
            "SELECT MIN(valid_time), MAX(valid_time) \
             FROM datasets \
             WHERE model = $1 AND reference_time = $2 AND status = 'available'",
        )
        .bind(model)
        .bind(reference_time)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(Some(LatestRunInfo {
            reference_time,
            forecast_hours,
            valid_time_start: time_range.0,
            valid_time_end: time_range.1,
        }))
    }

    /// Find dataset by specific run (reference_time) and forecast hour.
    ///
    /// Unlike `find_by_forecast_hour` which uses the latest run, this method
    /// requires an exact run specification. Returns an error if the data is not
    /// available for the specified run.
    ///
    /// This is used for "strict mode" queries where the user has explicitly
    /// specified which model run to use.
    pub async fn find_by_run_and_forecast_hour(
        &self,
        model: &str,
        parameter: &str,
        reference_time: DateTime<Utc>,
        forecast_hour: i32,
        level: Option<&str>,
    ) -> WmsResult<CatalogEntry> {
        let row = if let Some(lvl) = level {
            sqlx::query_as::<_, DatasetRow>(
                "SELECT model, parameter, level, reference_time, forecast_hour, \
                 bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                 storage_path, file_size, zarr_metadata FROM datasets \
                 WHERE model = $1 AND parameter = $2 AND reference_time = $3 \
                   AND forecast_hour = $4 AND level = $5 AND status = 'available' \
                 LIMIT 1",
            )
            .bind(model)
            .bind(parameter)
            .bind(reference_time)
            .bind(forecast_hour)
            .bind(lvl)
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, DatasetRow>(
                "SELECT model, parameter, level, reference_time, forecast_hour, \
                 bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                 storage_path, file_size, zarr_metadata FROM datasets \
                 WHERE model = $1 AND parameter = $2 AND reference_time = $3 \
                   AND forecast_hour = $4 AND status = 'available' \
                 LIMIT 1",
            )
            .bind(model)
            .bind(parameter)
            .bind(reference_time)
            .bind(forecast_hour)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        row.map(|r| r.into()).ok_or_else(|| {
            WmsError::DataNotAvailable(format!(
                "No data for {}/{} at run {} forecast hour {}{}",
                model,
                parameter,
                reference_time.format("%Y-%m-%dT%H:%M:%SZ"),
                forecast_hour,
                level.map(|l| format!(" level {}", l)).unwrap_or_default()
            ))
        })
    }

    /// Get best available data for a time range, falling back across runs.
    ///
    /// For each valid_time in the requested range, returns the dataset from
    /// the most recent model run that has data for that time. This enables
    /// "best available" queries that can seamlessly merge data from multiple
    /// runs when the latest run doesn't cover the full requested time range.
    ///
    /// Results are ordered by valid_time ASC.
    pub async fn get_best_available_for_time_range(
        &self,
        model: &str,
        parameter: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        level: Option<&str>,
    ) -> WmsResult<Vec<CatalogEntry>> {
        // Use a window function to get the best (most recent run) dataset per valid_time
        let rows = if let Some(lvl) = level {
            sqlx::query_as::<_, DatasetRow>(
                "WITH ranked AS (
                    SELECT model, parameter, level, reference_time, forecast_hour, \
                           bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                           storage_path, file_size, zarr_metadata, valid_time,
                           ROW_NUMBER() OVER (
                               PARTITION BY valid_time
                               ORDER BY reference_time DESC
                           ) as rn
                    FROM datasets
                    WHERE model = $1 AND parameter = $2 AND level = $3
                      AND valid_time >= $4 AND valid_time <= $5
                      AND status = 'available'
                )
                SELECT model, parameter, level, reference_time, forecast_hour, \
                       bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                       storage_path, file_size, zarr_metadata
                FROM ranked WHERE rn = 1
                ORDER BY valid_time ASC",
            )
            .bind(model)
            .bind(parameter)
            .bind(lvl)
            .bind(start_time)
            .bind(end_time)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, DatasetRow>(
                "WITH ranked AS (
                    SELECT model, parameter, level, reference_time, forecast_hour, \
                           bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                           storage_path, file_size, zarr_metadata, valid_time,
                           ROW_NUMBER() OVER (
                               PARTITION BY valid_time
                               ORDER BY reference_time DESC
                           ) as rn
                    FROM datasets
                    WHERE model = $1 AND parameter = $2
                      AND valid_time >= $3 AND valid_time <= $4
                      AND status = 'available'
                )
                SELECT model, parameter, level, reference_time, forecast_hour, \
                       bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
                       storage_path, file_size, zarr_metadata
                FROM ranked WHERE rn = 1
                ORDER BY valid_time ASC",
            )
            .bind(model)
            .bind(parameter)
            .bind(start_time)
            .bind(end_time)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get all available model runs for a model (not filtered by parameter).
    ///
    /// Returns run times ordered by reference_time DESC (most recent first).
    /// This is used for populating the "run" custom dimension in collection metadata.
    pub async fn get_all_model_runs(&self, model: &str) -> WmsResult<Vec<DateTime<Utc>>> {
        let rows = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT DISTINCT reference_time FROM datasets \
             WHERE model = $1 AND status = 'available' \
             ORDER BY reference_time DESC",
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows)
    }
}

/// Information about the latest model run.
#[derive(Debug, Clone)]
pub struct LatestRunInfo {
    /// The reference time (model initialization time) of the latest run.
    pub reference_time: DateTime<Utc>,
    /// All available forecast hours for this run.
    pub forecast_hours: Vec<i32>,
    /// The earliest valid time in this run.
    pub valid_time_start: DateTime<Utc>,
    /// The latest valid time in this run.
    pub valid_time_end: DateTime<Utc>,
}

/// Full dataset information for tree views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetInfo {
    pub model: String,
    pub parameter: String,
    pub level: String,
    pub reference_time: DateTime<Utc>,
    pub forecast_hour: u32,
    pub valid_time: DateTime<Utc>,
    pub storage_path: String,
    pub file_size: u64,
}

/// Detailed statistics for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterStats {
    pub model: String,
    pub parameter: String,
    pub count: u64,
    pub oldest: Option<DateTime<Utc>>,
    pub newest: Option<DateTime<Utc>>,
    pub total_size_bytes: u64,
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
    /// Zarr metadata (for new Zarr-format grids).
    /// Contains shape, chunk_shape, bbox, compression info etc.
    /// None for legacy GRIB2/NetCDF format files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zarr_metadata: Option<serde_json::Value>,
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

/// Aggregated statistics for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStats {
    /// Model identifier (e.g., "gfs", "hrrr")
    pub model: String,
    /// Total number of datasets for this model
    pub dataset_count: u64,
    /// Number of unique parameters
    pub parameter_count: u64,
    /// Most recent reference time
    pub last_ingest: Option<DateTime<Utc>>,
    /// List of parameter names
    pub parameters: Vec<String>,
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
    zarr_metadata: Option<serde_json::Value>,
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
            zarr_metadata: row.zarr_metadata,
        }
    }
}

/// Availability information for a specific parameter.
/// Used by capabilities generation to determine which dimensions to advertise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterAvailability {
    /// For forecast models: RUN times (ISO8601)
    /// For observation models: TIME values (ISO8601)
    pub times: Vec<String>,
    /// Forecast hours (empty for observation models)
    pub forecast_hours: Vec<i32>,
    /// Available levels for this parameter
    pub levels: Vec<String>,
    /// Bounding box for this parameter's data
    pub bbox: BoundingBox,
}

/// Database schema SQL for gridded datasets.
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
    zarr_metadata JSONB,

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
)
"#;

/// Database schema SQL for point observations (requires PostGIS).
/// This is run separately after the main schema to handle PostGIS extension.
pub const OBSERVATIONS_SCHEMA_SQL: &str = r#"
-- Enable PostGIS extension (requires superuser or extension already installed)
CREATE EXTENSION IF NOT EXISTS postgis;

-- Observation stations / locations (airports, weather stations, etc.)
-- This table serves as the canonical location registry for all EDR location queries.
CREATE TABLE IF NOT EXISTS locations (
    id VARCHAR(20) PRIMARY KEY,
    name VARCHAR(200) NOT NULL,
    description TEXT,
    location GEOGRAPHY(Point, 4326) NOT NULL,
    elevation_m REAL,
    location_type VARCHAR(50),
    country VARCHAR(10),
    region VARCHAR(50),
    properties JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_locations_geo ON locations USING GIST(location);
CREATE INDEX IF NOT EXISTS idx_locations_type ON locations(location_type);
CREATE INDEX IF NOT EXISTS idx_locations_country ON locations(country);

-- Surface observations (METARs, MADIS surface data, etc.)
-- All values stored in SI units for consistency.
CREATE TABLE IF NOT EXISTS observations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    location_id VARCHAR(20) NOT NULL REFERENCES locations(id) ON DELETE CASCADE,
    source VARCHAR(50) NOT NULL,
    obs_time TIMESTAMPTZ NOT NULL,
    receipt_time TIMESTAMPTZ,

    -- Core meteorological parameters (SI units)
    temperature_k REAL,
    dewpoint_k REAL,
    wind_direction_deg SMALLINT,
    wind_speed_ms REAL,
    wind_gust_ms REAL,
    altimeter_pa REAL,
    sea_level_pressure_pa REAL,
    visibility_m REAL,
    precip_1hr_mm REAL,
    relative_humidity_pct REAL,

    -- Aviation-specific fields
    raw_text TEXT,
    flight_category VARCHAR(10),
    wx_string VARCHAR(100),
    cloud_layers JSONB,

    -- Ocean/marine parameters (SI units, for NDBC buoy data)
    wave_height_m REAL,             -- Significant wave height (meters)
    dominant_wave_period_s REAL,    -- Dominant wave period (seconds)
    average_wave_period_s REAL,     -- Average wave period (seconds)
    mean_wave_direction_deg SMALLINT, -- Mean wave direction (degrees true)
    water_temp_k REAL,              -- Sea surface temperature (Kelvin)
    tide_m REAL,                    -- Water level above/below MLLW (meters)
    water_column_height_m REAL,     -- Water column height (meters, for DART tsunami buoys)

    -- QC flags (MADIS convention: V=valid, S=suspect, X=failed, C=coarse)
    temperature_qc CHAR(1),
    dewpoint_qc CHAR(1),
    wind_qc CHAR(1),
    pressure_qc CHAR(1),

    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Unique constraint to prevent duplicate observations
CREATE UNIQUE INDEX IF NOT EXISTS idx_observations_dedup
    ON observations(location_id, source, obs_time);

-- Query indexes
CREATE INDEX IF NOT EXISTS idx_observations_time ON observations(obs_time DESC);
CREATE INDEX IF NOT EXISTS idx_observations_source_time ON observations(source, obs_time DESC);
CREATE INDEX IF NOT EXISTS idx_observations_location_time
    ON observations(location_id, obs_time DESC);

-- Compound index for common query pattern: source + time range
CREATE INDEX IF NOT EXISTS idx_observations_source_location_time
    ON observations(source, location_id, obs_time DESC);

-- =============================================================================
-- TAF (Terminal Aerodrome Forecast) tables
-- =============================================================================

-- TAF forecasts (header/metadata)
-- Each TAF has an issue time and validity period (typically 24-30 hours)
CREATE TABLE IF NOT EXISTS taf_forecasts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    location_id VARCHAR(20) NOT NULL REFERENCES locations(id) ON DELETE CASCADE,
    issue_time TIMESTAMPTZ NOT NULL,
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ NOT NULL,
    raw_taf TEXT,
    remarks TEXT,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT taf_forecasts_unique UNIQUE (location_id, issue_time)
);

CREATE INDEX IF NOT EXISTS idx_taf_forecasts_location_valid
    ON taf_forecasts(location_id, valid_from DESC);
CREATE INDEX IF NOT EXISTS idx_taf_forecasts_valid_range
    ON taf_forecasts(valid_from, valid_to);
CREATE INDEX IF NOT EXISTS idx_taf_forecasts_issue_time
    ON taf_forecasts(issue_time DESC);

-- TAF forecast periods (detail)
-- Each TAF contains multiple periods: base forecast + FM/BECMG/TEMPO changes
-- All values stored in SI units for consistency with METAR observations
CREATE TABLE IF NOT EXISTS taf_periods (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    taf_id UUID NOT NULL REFERENCES taf_forecasts(id) ON DELETE CASCADE,
    period_from TIMESTAMPTZ NOT NULL,
    period_to TIMESTAMPTZ NOT NULL,
    change_indicator VARCHAR(10),  -- null (base), FM, BECMG, TEMPO, PROB
    probability SMALLINT,          -- null, 30, or 40
    wind_direction_deg SMALLINT,
    wind_speed_ms REAL,
    wind_gust_ms REAL,
    visibility_m REAL,
    wx_string VARCHAR(100),
    cloud_layers JSONB,
    period_order SMALLINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_taf_periods_taf ON taf_periods(taf_id);
CREATE INDEX IF NOT EXISTS idx_taf_periods_time ON taf_periods(period_from, period_to)
"#;

/// Schema for SPC / NOAA Storm Events severe-convective reports and the
/// TIGER county polygons used to stamp `county_fips` onto each event.
///
/// Requires the PostGIS extension (enabled by `OBSERVATIONS_SCHEMA_SQL`).
/// Geometry is stored in two typed columns: `geom_point` (begin location, all
/// event types) and `geom_track` (LineString, tornadoes only).
pub const STORM_EVENTS_SCHEMA_SQL: &str = r#"
-- PostGIS is enabled by the observations schema migration.
-- Enabling it here too so this script works standalone.
CREATE EXTENSION IF NOT EXISTS postgis;

-- TIGER/Line county polygons (Census). Loaded once via scripts/load_tiger_counties.sh.
-- Source of truth for stamping county_fips and for serving county boundary geometry.
CREATE TABLE IF NOT EXISTS tiger_counties (
    geoid CHAR(5) PRIMARY KEY,          -- 5-digit county FIPS (state + county)
    name VARCHAR(100) NOT NULL,
    state_fips CHAR(2),
    state_abbr CHAR(2),
    geom GEOMETRY(MultiPolygon, 4326) NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tiger_counties_geom ON tiger_counties USING GIST(geom);
CREATE INDEX IF NOT EXISTS idx_tiger_counties_state ON tiger_counties(state_abbr);

-- Storm Events (Hail / Thunderstorm Wind / Tornado).
-- One row per EVENT_ID. Geometry split into point (begin) and track (tornado line).
CREATE TABLE IF NOT EXISTS storm_events (
    event_id BIGINT PRIMARY KEY,        -- NOAA Storm Events EVENT_ID (dedup key)
    episode_id BIGINT,
    event_type VARCHAR(20) NOT NULL,    -- 'hail' | 'wind' | 'tornado'
    begin_time TIMESTAMPTZ NOT NULL,
    end_time TIMESTAMPTZ,

    -- Geometry (separate typed columns)
    geom_point GEOMETRY(Point, 4326),       -- begin location (all types)
    geom_track GEOMETRY(LineString, 4326),  -- tornado track (nullable)

    -- Raw coordinates (for reference / re-derivation)
    begin_lat DOUBLE PRECISION,
    begin_lon DOUBLE PRECISION,
    end_lat DOUBLE PRECISION,
    end_lon DOUBLE PRECISION,

    -- Magnitude (canonical units): hail inches, wind knots, tornado EF in tor_f_scale
    magnitude DOUBLE PRECISION,
    magnitude_unit VARCHAR(20),
    tor_f_scale SMALLINT,               -- 0..5, tornadoes only

    -- Administrative / reference fields from the CSV
    state VARCHAR(50),
    cz_name VARCHAR(100),
    cz_fips VARCHAR(10),
    cz_type CHAR(1),                    -- C=county, Z=zone (raw, not authoritative)

    -- County stamped via spatial join to tiger_counties (source of truth)
    county_fips CHAR(5),

    source VARCHAR(30) NOT NULL DEFAULT 'storm_events',
    raw JSONB DEFAULT '{}',
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_storm_events_geom_point ON storm_events USING GIST(geom_point);
CREATE INDEX IF NOT EXISTS idx_storm_events_geom_track ON storm_events USING GIST(geom_track);
CREATE INDEX IF NOT EXISTS idx_storm_events_type_time ON storm_events(event_type, begin_time);
CREATE INDEX IF NOT EXISTS idx_storm_events_county ON storm_events(county_fips);

-- Monthly-refreshed county aggregate: counts by (county, type, year).
-- Refreshed via REFRESH MATERIALIZED VIEW CONCURRENTLY (needs a unique index).
-- Note: state is NOT in the GROUP BY because the same county_fips can appear
-- with different state strings in the raw CSV data (data quality variation).
-- State is sourced from tiger_counties at query time instead.
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_county_event_counts AS
SELECT
    county_fips,
    event_type,
    EXTRACT(YEAR FROM begin_time)::INT AS year,
    COUNT(*)::BIGINT AS count
FROM storm_events
WHERE county_fips IS NOT NULL
GROUP BY county_fips, event_type, EXTRACT(YEAR FROM begin_time);

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_county_event_counts_key
    ON mv_county_event_counts(county_fips, event_type, year)
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // =========================================================================
    // CatalogEntry Tests
    // =========================================================================

    fn create_test_entry(reference_time: DateTime<Utc>, forecast_hour: u32) -> CatalogEntry {
        CatalogEntry {
            model: "gfs".to_string(),
            parameter: "temperature_2m".to_string(),
            level: "surface".to_string(),
            reference_time,
            forecast_hour,
            bbox: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
            storage_path: "/data/gfs/2024/01/15/12z/000/temperature_2m.zarr".to_string(),
            file_size: 1024 * 1024,
            zarr_metadata: None,
        }
    }

    #[test]
    fn test_catalog_entry_valid_time_analysis() {
        // Analysis (F00) - valid_time equals reference_time
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let entry = create_test_entry(ref_time, 0);
        assert_eq!(entry.valid_time(), ref_time);
    }

    #[test]
    fn test_catalog_entry_valid_time_forecast() {
        // F006 - valid_time is 6 hours after reference_time
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let entry = create_test_entry(ref_time, 6);
        let expected = Utc.with_ymd_and_hms(2024, 1, 15, 18, 0, 0).unwrap();
        assert_eq!(entry.valid_time(), expected);
    }

    #[test]
    fn test_catalog_entry_valid_time_next_day() {
        // F024 - valid_time crosses to next day
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let entry = create_test_entry(ref_time, 24);
        let expected = Utc.with_ymd_and_hms(2024, 1, 16, 12, 0, 0).unwrap();
        assert_eq!(entry.valid_time(), expected);
    }

    #[test]
    fn test_catalog_entry_valid_time_long_range() {
        // F384 (16 days) - long range forecast
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let entry = create_test_entry(ref_time, 384);
        let expected = Utc.with_ymd_and_hms(2024, 1, 31, 0, 0, 0).unwrap();
        assert_eq!(entry.valid_time(), expected);
    }

    #[test]
    fn test_catalog_entry_layer_id() {
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let entry = create_test_entry(ref_time, 0);
        let layer_id = entry.layer_id();
        assert_eq!(layer_id.0, "gfs:temperature_2m");
    }

    #[test]
    fn test_catalog_entry_layer_id_different_model() {
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let mut entry = create_test_entry(ref_time, 0);
        entry.model = "hrrr".to_string();
        entry.parameter = "wind_speed".to_string();
        let layer_id = entry.layer_id();
        assert_eq!(layer_id.0, "hrrr:wind_speed");
    }

    // =========================================================================
    // DatasetQuery Tests
    // =========================================================================

    #[test]
    fn test_dataset_query_default() {
        let query = DatasetQuery::default();
        assert!(query.model.is_none());
        assert!(query.parameter.is_none());
        assert!(query.level.is_none());
        assert!(query.time_range.is_none());
        assert!(query.bbox.is_none());
    }

    #[test]
    fn test_dataset_query_with_filters() {
        let query = DatasetQuery {
            model: Some("gfs".to_string()),
            parameter: Some("temperature_2m".to_string()),
            level: None,
            time_range: None,
            bbox: None,
        };
        assert_eq!(query.model.as_deref(), Some("gfs"));
        assert_eq!(query.parameter.as_deref(), Some("temperature_2m"));
    }

    // =========================================================================
    // PurgePreview Tests
    // =========================================================================

    #[test]
    fn test_purge_preview_default() {
        let preview = PurgePreview::default();
        assert_eq!(preview.dataset_count, 0);
        assert_eq!(preview.total_size_bytes, 0);
    }

    // =========================================================================
    // ModelStats Tests
    // =========================================================================

    #[test]
    fn test_model_stats_creation() {
        let stats = ModelStats {
            model: "gfs".to_string(),
            dataset_count: 1000,
            parameter_count: 25,
            last_ingest: Some(Utc::now()),
            parameters: vec!["temperature_2m".to_string(), "wind_speed".to_string()],
        };
        assert_eq!(stats.model, "gfs");
        assert_eq!(stats.dataset_count, 1000);
        assert_eq!(stats.parameters.len(), 2);
    }

    // =========================================================================
    // ParameterAvailability Tests
    // =========================================================================

    #[test]
    fn test_parameter_availability_forecast_model() {
        let avail = ParameterAvailability {
            times: vec![
                "2024-01-15T00:00:00Z".to_string(),
                "2024-01-15T06:00:00Z".to_string(),
            ],
            forecast_hours: vec![0, 6, 12, 18, 24],
            levels: vec!["surface".to_string()],
            bbox: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
        };
        assert_eq!(avail.times.len(), 2);
        assert_eq!(avail.forecast_hours.len(), 5);
    }

    #[test]
    fn test_parameter_availability_observation_model() {
        // Observation models have times but no forecast hours
        let avail = ParameterAvailability {
            times: vec![
                "2024-01-15T12:00:00Z".to_string(),
                "2024-01-15T12:15:00Z".to_string(),
            ],
            forecast_hours: vec![], // Empty for observations
            levels: vec!["surface".to_string()],
            bbox: BoundingBox::new(-125.0, 24.0, -66.0, 50.0),
        };
        assert!(avail.forecast_hours.is_empty());
        assert_eq!(avail.times.len(), 2);
    }

    // =========================================================================
    // DatasetInfo Tests
    // =========================================================================

    #[test]
    fn test_dataset_info_creation() {
        let info = DatasetInfo {
            model: "hrrr".to_string(),
            parameter: "reflectivity".to_string(),
            level: "surface".to_string(),
            reference_time: Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap(),
            forecast_hour: 3,
            valid_time: Utc.with_ymd_and_hms(2024, 1, 15, 15, 0, 0).unwrap(),
            storage_path: "/data/hrrr/reflectivity.zarr".to_string(),
            file_size: 512 * 1024,
        };
        assert_eq!(info.model, "hrrr");
        assert_eq!(info.forecast_hour, 3);
    }

    // =========================================================================
    // LatestRunInfo Tests
    // =========================================================================

    #[test]
    fn test_latest_run_info_creation() {
        let ref_time = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let info = LatestRunInfo {
            reference_time: ref_time,
            forecast_hours: vec![0, 1, 2, 3, 6, 12, 18, 24],
            valid_time_start: ref_time,
            valid_time_end: ref_time + chrono::Duration::hours(24),
        };
        assert_eq!(info.forecast_hours.len(), 8);
        assert_eq!(
            info.valid_time_end - info.valid_time_start,
            chrono::Duration::hours(24)
        );
    }

    // =========================================================================
    // ParameterStats Tests
    // =========================================================================

    #[test]
    fn test_parameter_stats_with_data() {
        let stats = ParameterStats {
            model: "gfs".to_string(),
            parameter: "temperature_2m".to_string(),
            count: 500,
            oldest: Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()),
            newest: Some(Utc.with_ymd_and_hms(2024, 1, 15, 18, 0, 0).unwrap()),
            total_size_bytes: 1024 * 1024 * 500, // 500 MB
        };
        assert!(stats.oldest.is_some());
        assert!(stats.newest.is_some());
        assert!(stats.newest.unwrap() > stats.oldest.unwrap());
    }

    #[test]
    fn test_parameter_stats_empty() {
        let stats = ParameterStats {
            model: "new_model".to_string(),
            parameter: "new_param".to_string(),
            count: 0,
            oldest: None,
            newest: None,
            total_size_bytes: 0,
        };
        assert_eq!(stats.count, 0);
        assert!(stats.oldest.is_none());
    }
}
