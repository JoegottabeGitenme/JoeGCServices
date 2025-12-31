//! EDR API Server
//!
//! OGC API - Environmental Data Retrieval implementation for weather data.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Extension, Router};
use clap::Parser;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

use edr_api::handlers;
use edr_api::state::AppState;

/// EDR API Server
#[derive(Parser, Debug)]
#[command(name = "edr-api")]
#[command(about = "OGC API - Environmental Data Retrieval server for weather data")]
struct Args {
    /// Listen address
    #[arg(short, long, default_value = "0.0.0.0:8083", env = "EDR_LISTEN_ADDR")]
    listen: String,

    /// Log level
    #[arg(long, default_value = "info", env = "RUST_LOG")]
    log_level: String,

    /// Number of worker threads
    #[arg(long, env = "EDR_WORKER_THREADS")]
    worker_threads: Option<usize>,
}

fn main() {
    // Load .env file if present
    dotenvy::dotenv().ok();

    let args = Args::parse();

    // Build runtime with configured threads
    let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
    runtime_builder.enable_all();

    if let Some(threads) = args.worker_threads {
        runtime_builder.worker_threads(threads);
    }

    let runtime = runtime_builder
        .build()
        .expect("Failed to create Tokio runtime");

    runtime.block_on(async move {
        run_server(args).await;
    });
}

async fn run_server(args: Args) {
    // Initialize tracing
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level));

    fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        .json()
        .init();

    info!("Starting EDR API server");

    // Initialize application state
    let state = match AppState::new().await {
        Ok(state) => Arc::new(state),
        Err(e) => {
            tracing::error!("Failed to initialize application state: {}", e);
            std::process::exit(1);
        }
    };

    // Build router
    let app = Router::new()
        // Landing page
        .route("/edr", get(handlers::landing::landing_handler))
        .route("/edr/", get(handlers::landing::landing_handler))
        // Conformance
        .route(
            "/edr/conformance",
            get(handlers::conformance::conformance_handler),
        )
        // Collections
        .route(
            "/edr/collections",
            get(handlers::collections::list_collections_handler),
        )
        .route(
            "/edr/collections/:collection_id",
            get(handlers::collections::get_collection_handler),
        )
        // Instances
        .route(
            "/edr/collections/:collection_id/instances",
            get(handlers::instances::list_instances_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id",
            get(handlers::instances::get_instance_handler),
        )
        // Position query
        .route(
            "/edr/collections/:collection_id/position",
            get(handlers::position::position_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/position",
            get(handlers::position::instance_position_handler),
        )
        // Area query
        .route(
            "/edr/collections/:collection_id/area",
            get(handlers::area::area_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/area",
            get(handlers::area::instance_area_handler),
        )
        // Radius query
        .route(
            "/edr/collections/:collection_id/radius",
            get(handlers::radius::radius_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/radius",
            get(handlers::radius::instance_radius_handler),
        )
        // Trajectory query
        .route(
            "/edr/collections/:collection_id/trajectory",
            get(handlers::trajectory::trajectory_handler),
        )
        .route(
            "/edr/collections/:collection_id/instances/:instance_id/trajectory",
            get(handlers::trajectory::instance_trajectory_handler),
        )
        // Health and metrics
        .route("/health", get(handlers::health::health_handler))
        .route("/ready", get(handlers::health::ready_handler))
        .route("/metrics", get(handlers::health::metrics_handler))
        // Catalog check (for coverage validation)
        .route(
            "/edr/catalog-check",
            get(handlers::catalog_check::catalog_check_handler),
        )
        // Middleware
        .layer(Extension(state))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive());

    // Parse listen address
    let addr: SocketAddr = args.listen.parse().expect("Invalid listen address");

    info!("EDR API listening on {}", addr);

    // Start server
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");
    axum::serve(listener, app).await.expect("Server failed");
}
