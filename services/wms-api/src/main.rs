//! WMS/WMTS API service.
//!
//! HTTP server implementing OGC WMS 1.1.1/1.3.0 and WMTS 1.0.0 specifications.

use wms_api::{
    admin, chunk_warming, cleanup, handlers, memory_pressure, startup_validation, state, warming,
};

use anyhow::Result;
use axum::{
    extract::Extension,
    routing::{get, post},
    Router,
};
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

    /// Number of tokio worker threads (default: number of CPU cores)
    #[arg(long)]
    worker_threads: Option<usize>,
}

fn main() -> Result<()> {
    // Silence HDF5's verbose stderr error messages before any NetCDF operations
    // This must be called early, before any HDF5/NetCDF libraries are initialized
    netcdf_parser::silence_hdf5_errors();

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
                info!(
                    "Configuring tokio runtime with {} worker threads (from env)",
                    threads
                );
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

    // Run database migrations
    info!("Running database migrations...");
    state.catalog.migrate().await?;
    info!("Database migrations completed successfully");

    // Validate layer configs against catalog
    // This ensures all parameters in the catalog have proper layer configs
    {
        let models = state.catalog.list_models().await.unwrap_or_default();
        let mut model_params = std::collections::HashMap::new();
        for model in &models {
            let params = state
                .catalog
                .list_parameters(model)
                .await
                .unwrap_or_default();
            model_params.insert(model.clone(), params);
        }

        if let Err(e) = state
            .layer_configs
            .read()
            .await
            .validate_catalog_coverage(&models, &model_params)
        {
            // Check if we should fail on missing configs (default: warn only)
            let strict_validation = env::var("STRICT_LAYER_VALIDATION")
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(false);

            if strict_validation {
                return Err(anyhow::anyhow!(
                    "Layer configuration validation failed:\n{}",
                    e
                ));
            } else {
                tracing::warn!("Layer configuration validation warning:\n{}", e);
            }
        } else {
            info!("Layer configuration validation passed");
        }
    }

    // Run cache warming if enabled (Phase 7.D)
    {
        let warming_config = warming::WarmingConfig {
            enabled: state.optimization_config.cache_warming_enabled,
            max_zoom: env::var("CACHE_WARMING_MAX_ZOOM")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(4),
            forecast_hours: env::var("CACHE_WARMING_HOURS")
                .ok()
                .and_then(|v| {
                    v.split(',')
                        .filter_map(|s| s.trim().parse::<u32>().ok())
                        .collect::<Vec<u32>>()
                        .into()
                })
                .unwrap_or_else(|| vec![0]),
            layers: env::var("CACHE_WARMING_LAYERS")
                .ok()
                .and_then(|v| {
                    v.split(';')
                        .filter_map(|pair| {
                            let parts: Vec<&str> = pair.split(':').collect();
                            if parts.len() == 2 {
                                Some(warming::WarmingLayer {
                                    name: parts[0].trim().to_string(),
                                    style: parts[1].trim().to_string(),
                                })
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<warming::WarmingLayer>>()
                        .into()
                })
                .unwrap_or_else(|| {
                    vec![warming::WarmingLayer {
                        name: "gfs_TMP".to_string(),
                        style: "temperature".to_string(),
                    }]
                }),
            concurrency: env::var("CACHE_WARMING_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(10),
        };

        if warming_config.enabled {
            info!("Cache warming enabled - starting background warming task");
            let warmer = warming::CacheWarmer::new(state.clone(), warming_config);
            warmer.warm_startup().await;
        } else {
            info!("Cache warming disabled (set ENABLE_CACHE_WARMING=true to enable)");
        }
    }

    // Run startup validation to verify data ingestion and warm caches
    {
        let validation_config = startup_validation::StartupValidationConfig::from_env();

        if validation_config.enabled {
            info!("Running startup validation...");
            let validator =
                startup_validation::StartupValidator::new(state.clone(), validation_config.clone());
            let summary = validator.validate().await;

            // Log available models prominently
            if !summary.models_available.is_empty() {
                info!(
                    models = ?summary.models_available,
                    "Data available for models"
                );
            }

            if !summary.models_missing.is_empty() {
                info!(
                    models = ?summary.models_missing,
                    "No data for models (run ingestion scripts to populate)"
                );
            }

            // Check if we should fail on validation errors
            if validation_config.fail_on_error && summary.failed > 0 {
                return Err(anyhow::anyhow!(
                    "Startup validation failed: {} tests failed out of {}",
                    summary.failed,
                    summary.total_tests
                ));
            }
        } else {
            info!("Startup validation disabled (set ENABLE_STARTUP_VALIDATION=true to enable)");
        }
    }

    // Start data cleanup background task
    {
        let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| "/app/config".to_string());
        let cleanup_config = cleanup::CleanupConfig::from_env_and_configs(&config_dir);

        if cleanup_config.enabled {
            info!(
                interval_secs = cleanup_config.interval_secs,
                models = ?cleanup_config.model_retentions.keys().collect::<Vec<_>>(),
                "Starting data cleanup background task"
            );
            let cleanup_task = cleanup::CleanupTask::new(state.clone(), cleanup_config);
            tokio::spawn(async move {
                cleanup_task.run_forever().await;
            });
        } else {
            info!("Data cleanup disabled (set ENABLE_CLEANUP=true to enable)");
        }
    }

    // Start database sync background task (cleans orphan MinIO files not tracked in DB)
    {
        let sync_config = cleanup::SyncConfig::from_env();

        if sync_config.enabled {
            info!(
                interval_secs = sync_config.interval_secs,
                "Starting database sync background task"
            );
            let sync_task = cleanup::SyncTask::new(state.clone(), sync_config);
            tokio::spawn(async move {
                sync_task.run_forever().await;
            });
        } else {
            info!("Database sync disabled (set ENABLE_SYNC=true to enable)");
        }
    }

    // Start chunk warming background task (proactive cache warming for GOES/observation data)
    {
        let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| "/app/config".to_string());
        let chunk_warmer = Arc::new(chunk_warming::ChunkWarmer::new(state.clone(), &config_dir));

        // Store warmer in state for use by ingestion handler
        {
            let mut warmer_lock = state.chunk_warmer.write().await;
            *warmer_lock = Some(chunk_warmer.clone());
        }

        // Spawn background task
        let warmer_clone = chunk_warmer.clone();
        tokio::spawn(async move {
            warmer_clone.run_forever().await;
        });

        info!("Chunk warming background task started");
    }

    // Start memory pressure monitor background task
    if state.optimization_config.memory_pressure_enabled {
        let monitor = memory_pressure::MemoryPressureMonitor::new(state.clone());
        tokio::spawn(async move {
            monitor.run_forever().await;
        });
        info!("Memory pressure monitor started");
    } else {
        info!("Memory pressure monitoring disabled (set ENABLE_MEMORY_PRESSURE=true to enable)");
    }

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
        .route(
            "/api/forecast-times/:model/:parameter",
            get(handlers::forecast_times_handler),
        )
        .route("/api/parameters/:model", get(handlers::parameters_handler))
        // Ingestion events API
        .route(
            "/api/ingestion/events",
            get(handlers::ingestion_events_handler),
        )
        // Validation API
        .route(
            "/api/validation/status",
            get(handlers::validation_status_handler),
        )
        .route("/api/validation/run", get(handlers::validation_run_handler))
        // Startup validation API (test renders + cache warming)
        .route(
            "/api/validation/startup",
            get(handlers::startup_validation_run_handler),
        )
        // Storage stats API
        .route("/api/storage/stats", get(handlers::storage_stats_handler))
        // Container/pod resource stats API
        .route(
            "/api/container/stats",
            get(handlers::container_stats_handler),
        )
        // Grid processor stats API (Zarr chunk cache and processing)
        .route(
            "/api/grid-processor/stats",
            get(handlers::grid_processor_stats_handler),
        )
        // Tile request heatmap API (for geographic visualization)
        .route("/api/tile-heatmap", get(handlers::tile_heatmap_handler))
        .route(
            "/api/tile-heatmap/clear",
            post(handlers::tile_heatmap_clear_handler),
        )
        // Application metrics API
        .route("/api/metrics", get(handlers::api_metrics_handler))
        // Configuration API - shows optimization settings
        .route("/api/config", get(handlers::config_handler))
        // Cache API endpoints
        .route("/api/cache/list", get(handlers::cache_list_handler))
        .route("/api/cache/clear", post(handlers::cache_clear_handler))
        // Config reload endpoints (hot reload)
        .route("/api/config/reload", post(handlers::config_reload_handler))
        .route(
            "/api/config/reload/layers",
            post(handlers::config_reload_layers_handler),
        )
        // API Documentation (Swagger UI)
        .route("/api/docs", get(handlers::swagger_ui_handler))
        .route(
            "/api/docs/openapi.yaml",
            get(handlers::openapi_yaml_handler),
        )
        .route(
            "/api/docs/openapi.json",
            get(handlers::openapi_json_handler),
        )
        // Load test dashboard
        .route("/loadtest", get(handlers::loadtest_dashboard_handler))
        .route(
            "/api/loadtest/results",
            get(handlers::loadtest_results_handler),
        )
        // Benchmark comparison API (for web/benchmarks.html - load test comparisons)
        .route("/api/benchmarks", get(handlers::benchmarks_handler))
        // Criterion microbenchmark results API
        .route(
            "/api/criterion",
            get(handlers::criterion_benchmarks_handler),
        )
        .route("/api/loadtest/files", get(handlers::loadtest_files_handler))
        .route(
            "/api/loadtest/file/:filename",
            get(handlers::loadtest_file_handler),
        )
        // Admin dashboard
        .route(
            "/api/admin/ingestion/status",
            get(admin::ingestion_status_handler),
        )
        .route(
            "/api/admin/database/details",
            get(admin::database_details_handler),
        )
        .route(
            "/api/admin/database/datasets/:model/:parameter",
            get(admin::database_datasets_handler),
        )
        .route("/api/admin/storage/tree", get(admin::storage_tree_handler))
        .route(
            "/api/admin/ingestion/log",
            get(admin::ingestion_log_handler),
        )
        .route(
            "/api/admin/preview-shred",
            get(admin::preview_shred_handler),
        )
        .route("/api/admin/config/models", get(admin::list_models_handler))
        .route(
            "/api/admin/config/models/:id",
            get(admin::get_model_config_handler).put(admin::update_model_config_handler),
        )
        .route("/api/admin/config/full", get(admin::full_config_handler))
        // Ingest endpoint (called by downloader service)
        .route("/admin/ingest", post(admin::ingest_handler))
        // Cleanup/retention endpoints
        .route(
            "/api/admin/cleanup/status",
            get(admin::cleanup_status_handler),
        )
        .route("/api/admin/cleanup/run", post(admin::cleanup_run_handler))
        // Database/storage sync endpoints
        .route("/api/admin/sync/status", get(admin::sync_status_handler))
        .route("/api/admin/sync/preview", get(admin::sync_preview_handler))
        .route("/api/admin/sync/run", post(admin::sync_run_handler))
        // Ingestion tracking endpoint
        .route(
            "/api/admin/ingestion/active",
            get(admin::ingestion_active_handler),
        )
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
