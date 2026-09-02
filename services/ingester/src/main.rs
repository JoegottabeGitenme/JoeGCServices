//! Weather data ingester service.
//!
//! HTTP-triggered ingestion service that receives ingestion requests from the downloader
//! and processes GRIB2/NetCDF files into Zarr format.
//!
//! # Usage
//!
//! ## Server mode (default)
//! ```bash
//! ingester --port 8082
//! ```
//!
//! ## Test file mode (development)
//! ```bash
//! ingester --test-file /path/to/data.grib2 --test-model gfs
//! ```

mod server;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use ingestion::{IngestOptions, Ingester};
use std::env;
use storage::observations::ObservationCatalog;
use storage::storm_events::StormEventCatalog;
use storage::{Catalog, ObjectStorage, ObjectStorageConfig};

use server::{start_server, IngestionTracker, ServerState};

#[derive(Parser, Debug)]
#[command(name = "ingester")]
#[command(about = "Weather data ingester HTTP service")]
struct Args {
    /// HTTP server port
    #[arg(short, long, default_value = "8082")]
    port: u16,

    /// Test with local file (bypasses HTTP server)
    #[arg(long)]
    test_file: Option<String>,

    /// Model name for test file (e.g., "gfs", "hrrr", "goes19")
    #[arg(long)]
    test_model: Option<String>,

    /// Forecast hour for test file
    #[arg(long)]
    forecast_hour: Option<u32>,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(true)
        .with_thread_ids(true)
        .json()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting weather data ingester");

    // Create storage and catalog connections
    let storage_config = ObjectStorageConfig {
        endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
        bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
        access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
        secret_access_key: env::var("S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
        region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        allow_http: env::var("S3_ALLOW_HTTP")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true),
    };
    let storage = Arc::new(ObjectStorage::new(&storage_config)?);

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://weather:weather@localhost:5432/weather".to_string());
    let catalog = Catalog::connect(&database_url).await?;
    catalog.migrate().await?;

    // Migrate observations schema (adds PostGIS tables if not present)
    catalog.migrate_observations().await?;
    info!("Observation schema migrated");

    // Migrate storm events schema (tiger_counties, storm_events, aggregate view)
    catalog.migrate_storm_events().await?;
    info!("Storm events schema migrated");

    // Create observation catalog using the same connection pool
    let observation_catalog = ObservationCatalog::new(catalog.pool_clone());

    // Create storm event catalog sharing the same connection pool
    let storm_event_catalog = StormEventCatalog::new(catalog.pool_clone());

    // Bootstrap locations if needed (loads initial airport data)
    // Threshold of 100 means: if we have fewer than 100 locations, populate from embedded data
    match storage::stations_bootstrap::bootstrap_locations(&observation_catalog, 100).await {
        Ok(count) => {
            if count > 0 {
                info!(count = count, "Bootstrapped initial station locations");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to bootstrap stations (continuing anyway)");
        }
    }

    // Bootstrap US populated places for the EDR `populated` collection.
    // Threshold of 1000: if fewer than 1000 populated_place rows exist, load
    // the embedded ~10k-city dataset. Runs independently of the airport set.
    match storage::stations_bootstrap::bootstrap_populated_places(&observation_catalog, 1000).await
    {
        Ok(count) => {
            if count > 0 {
                info!(count = count, "Bootstrapped US populated places");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to bootstrap populated places (continuing anyway)");
        }
    }

    // Bootstrap US ZIP codes for ZIP lookup in the EDR `populated` collection.
    // Threshold of 1000: if fewer than 1000 zip rows exist, load the embedded
    // ~33k-ZIP dataset.
    match storage::stations_bootstrap::bootstrap_zip_codes(&observation_catalog, 1000).await {
        Ok(count) => {
            if count > 0 {
                info!(count = count, "Bootstrapped US ZIP codes");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to bootstrap ZIP codes (continuing anyway)");
        }
    }

    // ------------------------------------------------------------------
    // Data retention + storage reconciliation background tasks.
    //
    // These used to run inside wms-api. A Redis outage there stopped them for
    // ~3 weeks, which filled the disk and took the whole stack down (see
    // docs/incident-2026-08-disk-exhaustion.md). They now live here: the
    // ingester has a much smaller dependency surface (no Redis) and is already
    // required for new data to exist at all.
    //
    // Context is built BEFORE `Ingester::new` takes ownership of the storage
    // and catalog handles. Both clones are cheap (Arc / shared pool).
    // ------------------------------------------------------------------
    {
        let retention_ctx = retention::RetentionContext::new(catalog.clone(), Arc::clone(&storage));

        let config_dir = env::var("CONFIG_DIR").unwrap_or_else(|_| "/app/config".to_string());
        let cleanup_config = retention::CleanupConfig::from_env_and_configs(&config_dir);

        if cleanup_config.enabled {
            info!(
                interval_secs = cleanup_config.interval_secs,
                models = ?cleanup_config.model_configs.keys().collect::<Vec<_>>(),
                "Starting data cleanup background task"
            );
            let cleanup_task = retention::CleanupTask::new(retention_ctx.clone(), cleanup_config);
            tokio::spawn(async move {
                cleanup_task.run_forever().await;
            });
        } else {
            info!("Data cleanup disabled (set ENABLE_CLEANUP=true to enable)");
        }

        let sync_config = retention::SyncConfig::from_env();
        if sync_config.enabled {
            info!(
                interval_secs = sync_config.interval_secs,
                "Starting database sync background task"
            );
            let sync_task = retention::SyncTask::new(retention_ctx, sync_config);
            tokio::spawn(async move {
                sync_task.run_forever().await;
            });
        } else {
            info!("Database sync disabled (set ENABLE_SYNC=true to enable)");
        }
    }

    // Create ingester
    let ingester = Ingester::new(storage, catalog);

    // Handle test file mode (for development)
    if let Some(test_file) = &args.test_file {
        return run_test_file(ingester, test_file, args.test_model, args.forecast_hour).await;
    }

    // Create server state
    let state = Arc::new(ServerState {
        ingester,
        observation_catalog: Some(observation_catalog),
        storm_event_catalog: Some(storm_event_catalog),
        tracker: IngestionTracker::new(),
    });

    // Start HTTP server
    info!(port = args.port, "Starting HTTP server");
    start_server(state, args.port).await?;

    Ok(())
}

/// Run test file ingestion (development mode).
async fn run_test_file(
    ingester: Ingester,
    test_file: &str,
    model: Option<String>,
    forecast_hour: Option<u32>,
) -> Result<()> {
    info!(
        file = %test_file,
        model = ?model,
        forecast_hour = ?forecast_hour,
        "Test file ingestion mode"
    );

    let options = IngestOptions {
        model,
        forecast_hour,
    };

    let result = ingester.ingest_file(test_file, options).await?;

    info!(
        datasets = result.datasets_registered,
        model = %result.model,
        reference_time = %result.reference_time,
        parameters = ?result.parameters,
        bytes_written = result.bytes_written,
        "Ingestion completed"
    );

    Ok(())
}
