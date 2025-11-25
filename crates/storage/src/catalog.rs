//! Metadata catalog using PostgreSQL.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool, FromRow};
use uuid::Uuid;

use wms_common::{BoundingBox, LayerId, ValidTime, WmsError, WmsResult};

/// Database connection pool and catalog operations.
pub struct Catalog {
    pool: PgPool,
}

impl Catalog {
    /// Create a new catalog connection from database URL.
    pub async fn connect(database_url: &str) -> WmsResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Connection failed: {}", e)))?;

        Ok(Self { pool })
    }

    /// Run database migrations.
    pub async fn migrate(&self) -> WmsResult<()> {
        sqlx::query(SCHEMA_SQL)
            .execute(&self.pool)
            .await
            .map_err(|e| WmsError::DatabaseError(format!("Migration failed: {}", e)))?;

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
            "#
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
        let mut sql = String::from(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets WHERE status = 'available'"
        );
        
        let mut params: Vec<String> = Vec::new();
        let mut param_idx = 1;

        if let Some(model) = &query.model {
            sql.push_str(&format!(" AND model = ${}", param_idx));
            params.push(model.clone());
            param_idx += 1;
        }

        if let Some(parameter) = &query.parameter {
            sql.push_str(&format!(" AND parameter = ${}", param_idx));
            params.push(parameter.clone());
            param_idx += 1;
        }

        // For now, use a simpler approach - full query building would need runtime SQL
        let rows = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets WHERE status = 'available' \
             ORDER BY valid_time DESC LIMIT 100"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Query failed: {}", e)))?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    /// Get the most recent dataset for a layer.
    pub async fn get_latest(&self, model: &str, parameter: &str) -> WmsResult<Option<CatalogEntry>> {
        let row = sqlx::query_as::<_, DatasetRow>(
            "SELECT model, parameter, level, reference_time, forecast_hour, \
             bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, \
             storage_path, file_size FROM datasets \
             WHERE model = $1 AND parameter = $2 AND status = 'available' \
             ORDER BY valid_time DESC LIMIT 1"
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
             ORDER BY ABS(EXTRACT(EPOCH FROM (valid_time - $3))) ASC LIMIT 1"
        )
        .bind(model)
        .bind(parameter)
        .bind(valid_time)
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
             ORDER BY valid_time DESC"
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
            "SELECT DISTINCT model FROM datasets WHERE status = 'available' ORDER BY model"
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

    /// Mark old datasets for cleanup.
    pub async fn mark_expired(&self, older_than: DateTime<Utc>) -> WmsResult<u64> {
        let result = sqlx::query(
            "UPDATE datasets SET status = 'expired' WHERE valid_time < $1 AND status = 'available'"
        )
        .bind(older_than)
        .execute(&self.pool)
        .await
        .map_err(|e| WmsError::DatabaseError(format!("Update failed: {}", e)))?;

        Ok(result.rows_affected())
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
