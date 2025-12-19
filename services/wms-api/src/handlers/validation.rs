//! Validation API handlers for WMS/WMTS testing.

use axum::{
    extract::Extension,
    http::StatusCode,
    Json,
};
use std::sync::Arc;
use tracing::{info, instrument};

use crate::state::AppState;

/// GET /api/validation/status - Get current validation status
#[instrument(skip(_state))]
pub async fn validation_status_handler(
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<Json<crate::validation::ValidationStatus>, StatusCode> {
    info!("Validation status requested");
    
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let status = crate::validation::run_validation(&base_url).await;
    
    Ok(Json(status))
}

/// GET /api/validation/run - Run validation and return results
#[instrument(skip(_state))]
pub async fn validation_run_handler(
    Extension(_state): Extension<Arc<AppState>>,
) -> Result<Json<crate::validation::ValidationStatus>, StatusCode> {
    info!("Running validation");
    
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let status = crate::validation::run_validation(&base_url).await;
    
    Ok(Json(status))
}

/// GET /api/validation/startup - Run startup-style validation for each model
#[instrument(skip(state))]
pub async fn startup_validation_run_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    info!("Running startup validation");
    
    // Create validator with default config and run validation
    let config = crate::startup_validation::StartupValidationConfig::from_env();
    let validator = crate::startup_validation::StartupValidator::new(state, config);
    let summary = validator.validate().await;
    
    Ok(Json(serde_json::json!({
        "total_tests": summary.total_tests,
        "passed": summary.passed,
        "failed": summary.failed,
        "skipped": summary.skipped,
        "duration_ms": summary.duration_ms,
        "models_available": summary.models_available,
        "models_missing": summary.models_missing
    })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_validation_module_exists() {
        // Basic test to ensure module compiles
        assert!(true);
    }
}
