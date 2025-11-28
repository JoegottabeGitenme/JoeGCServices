//! WMS/WMTS API service.
//!
//! HTTP server implementing OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications.

mod handlers;
pub mod metrics;
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
use metrics_exporter_prometheus::PrometheusHandle;

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

    /// Number of tokio worker threads (default: number of CPU cores)
    #[arg(long)]
    worker_threads: Option<usize>,
}

fn main() -> Result<()> {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();
    
    let args = Args::parse();

    // Build tokio runtime with configurable worker threads
    let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
    runtime_builder.enable_all();
    
    if let Some(threads) = args.worker_threads {
        info!("Configuring tokio runtime with {} worker threads", threads);
        runtime_builder.worker_threads(threads);
    } else {
        // Use environment variable if CLI arg not provided
        if let Ok(threads_str) = env::var("TOKIO_WORKER_THREADS") {
            if let Ok(threads) = threads_str.parse::<usize>() {
                info!("Configuring tokio runtime with {} worker threads (from env)", threads);
                runtime_builder.worker_threads(threads);
            }
        }
    }
    
    let runtime = runtime_builder.build()?;
    runtime.block_on(async_main(args))?;
    Ok(())
}

async fn async_main(args: Args) -> Result<()> {

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

    // Initialize Prometheus metrics exporter
    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");
    
    info!("Prometheus metrics exporter initialized");
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
        // Storage stats API
        .route("/api/storage/stats", get(handlers::storage_stats_handler))
        // Container/pod resource stats API
        .route("/api/container/stats", get(handlers::container_stats_handler))
        // Application metrics API
        .route("/api/metrics", get(handlers::api_metrics_handler))
        // Layer extensions
        .layer(Extension(state))
        .layer(Extension(prometheus_handle))
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
