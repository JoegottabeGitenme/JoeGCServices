//! WMS/WMTS API service.
//!
//! HTTP server implementing OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications.

mod handlers;
mod rendering;
mod state;
mod validation;

use anyhow::Result;
use axum::{extract::Extension, routing::get, Router};
use clap::Parser;
use std::{env, net::SocketAddr, sync::Arc};
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use state::AppState;

#[derive(Parser, Debug)]
#[command(name = "wms-api")]
#[command(about = "OGC WMS/WMTS API server")]
struct Args {
    /// Listen address
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    listen: String,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    
    let args = Args::parse();

    // Initialize tracing
    let level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .json()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting WMS/WMTS API server");

    // Initialize application state
    let state = Arc::new(AppState::new().await?);

    // Build router
    let app = Router::new()
        // WMS endpoints
        .route("/wms", get(handlers::wms_handler))
        .route("/wms/", get(handlers::wms_handler))
        // WMTS endpoints (KVP)
        .route("/wmts", get(handlers::wmts_kvp_handler))
        .route("/wmts/", get(handlers::wmts_kvp_handler))
        // WMTS RESTful endpoints
        .route("/wmts/rest/*path", get(handlers::wmts_rest_handler))
        // Simple tile endpoints (XYZ/TMS style for easy integration)
        .route(
            "/tiles/:layer/:style/:z/:x/:y",
            get(handlers::xyz_tile_handler),
        )
        // Health check
        .route("/health", get(handlers::health_handler))
        .route("/ready", get(handlers::ready_handler))
        // Metrics
        .route("/metrics", get(handlers::metrics_handler))
        // API endpoints
        .route("/api/forecast-times/:model/:parameter", get(handlers::forecast_times_handler))
        .route("/api/parameters/:model", get(handlers::parameters_handler))
        // Ingestion events API
        .route("/api/ingestion/events", get(handlers::ingestion_events_handler))
        // Validation API
        .route("/api/validation/status", get(handlers::validation_status_handler))
        .route("/api/validation/run", get(handlers::validation_run_handler))
        // Layer extensions
        .layer(Extension(state))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive());

    // Parse listen address
    let addr: SocketAddr = args.listen.parse()?;
    info!(address = %addr, "Listening");

    // Start server
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
