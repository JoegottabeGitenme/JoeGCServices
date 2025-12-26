//! REST API handlers for data discovery.
//!
//! Provides endpoints for:
//! - Listing available forecast times
//! - Listing available parameters
//! - Listing recent ingestion events

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, instrument};

use crate::state::AppState;

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ForecastTimesResponse {
    pub model: String,
    pub parameter: String,
    pub forecast_hours: Vec<i32>,
}

#[derive(Debug, Serialize)]
pub struct ParametersResponse {
    pub model: String,
    pub parameters: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IngestionEvent {
    pub model: String,
    pub parameter: String,
    pub reference_time: String,
    pub forecast_hour: i32,
    pub ingested_at: String,
}

// ============================================================================
// Query Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ForecastTimesQuery {
    pub parameter: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IngestionEventsQuery {
    pub limit: Option<i32>,
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /api/forecast-times/:model - Get available forecast hours
#[instrument(skip(state))]
pub async fn forecast_times_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(model): Path<String>,
    Query(query): Query<ForecastTimesQuery>,
) -> Result<Json<ForecastTimesResponse>, StatusCode> {
    info!(model = %model, parameter = ?query.parameter, "Forecast times request");

    let parameter = query.parameter.unwrap_or_else(|| "TMP".to_string());

    let forecast_hours = state
        .catalog
        .get_available_forecast_hours(&model, &parameter)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to get forecast hours");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ForecastTimesResponse {
        model,
        parameter,
        forecast_hours,
    }))
}

/// GET /api/parameters/:model - Get available parameters
#[instrument(skip(state))]
pub async fn parameters_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(model): Path<String>,
) -> Result<Json<ParametersResponse>, StatusCode> {
    info!(model = %model, "Parameters request");

    let parameters = state.catalog.list_parameters(&model).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to list parameters");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ParametersResponse { model, parameters }))
}

/// GET /api/ingestion/events - Get recent ingestion events
#[instrument(skip(state))]
pub async fn ingestion_events_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(query): Query<IngestionEventsQuery>,
) -> Result<Json<Vec<IngestionEvent>>, StatusCode> {
    let minutes = query.limit.unwrap_or(60) as i64; // Default to last 60 minutes
    info!(minutes = minutes, "Ingestion events request");

    let events = state
        .catalog
        .get_recent_ingestions(minutes)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to get ingestion events");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let response: Vec<IngestionEvent> = events
        .into_iter()
        .map(|e| IngestionEvent {
            model: e.model,
            parameter: e.parameter,
            reference_time: e.reference_time.to_rfc3339(),
            forecast_hour: e.forecast_hour as i32,
            ingested_at: e.reference_time.to_rfc3339(), // Use reference_time as proxy
        })
        .collect();

    Ok(Json(response))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forecast_times_response_serialization() {
        let response = ForecastTimesResponse {
            model: "gfs".to_string(),
            parameter: "TMP".to_string(),
            forecast_hours: vec![0, 3, 6, 12],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"model\":\"gfs\""));
        assert!(json.contains("\"forecast_hours\":[0,3,6,12]"));
    }

    #[test]
    fn test_parameters_response_serialization() {
        let response = ParametersResponse {
            model: "hrrr".to_string(),
            parameters: vec!["TMP".to_string(), "UGRD".to_string(), "VGRD".to_string()],
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"model\":\"hrrr\""));
        assert!(json.contains("TMP"));
    }

    #[test]
    fn test_ingestion_event_serialization() {
        let event = IngestionEvent {
            model: "gfs".to_string(),
            parameter: "TMP".to_string(),
            reference_time: "2024-01-15T12:00:00Z".to_string(),
            forecast_hour: 6,
            ingested_at: "2024-01-15T13:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"forecast_hour\":6"));
    }
}
