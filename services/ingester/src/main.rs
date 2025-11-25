//! Weather data ingester service.
//!
//! Polls NOAA data sources (NOMADS, AWS Open Data) and ingests
//! GRIB2/NetCDF files into object storage with catalog updates.

mod config;
mod ingest;
mod sources;

use anyhow::Result;
use clap::Parser;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use config::IngesterConfig;
use ingest::IngestionPipeline;
use std::fs;
use bytes::Bytes;
use chrono::Utc;

#[derive(Parser, Debug)]
#[command(name = "ingester")]
#[command(about = "Weather data ingester for WMS services")]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/ingester/config.yaml")]
    config: String,

    /// Run once and exit (vs continuous polling)
    #[arg(long)]
    once: bool,

    /// Specific model to ingest (default: all configured)
    #[arg(short, long)]
    model: Option<String>,

    /// Test with local GRIB2 file
    #[arg(long)]
    test_file: Option<String>,

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

    // Load configuration
    let config = IngesterConfig::from_env()?;
    info!(models = ?config.models.keys().collect::<Vec<_>>(), "Loaded configuration");

    // Handle test file mode
    if let Some(test_file) = &args.test_file {
        return test_file_ingestion(&config, test_file).await;
    }

    // Create ingestion pipeline
    let pipeline = IngestionPipeline::new(&config).await?;

    if args.once {
        // Single run mode
        info!("Running single ingestion cycle");

        if let Some(model) = &args.model {
            pipeline.ingest_model(model).await?;
        } else {
            pipeline.ingest_all().await?;
        }
    } else {
        // Continuous polling mode
        info!("Starting continuous polling");
        pipeline.run_forever().await?;
    }

    Ok(())
}

/// Ingest a local GRIB2 test file
async fn test_file_ingestion(config: &IngesterConfig, test_file: &str) -> Result<()> {
    use storage::{Catalog, CatalogEntry, ObjectStorage};
    use wms_common::BoundingBox;

    info!(file = %test_file, "Testing ingestion with local file");

    // Read file
    let data = fs::read(test_file)?;
    let data_bytes = Bytes::from(data);
    let file_size = data_bytes.len() as u64;

    // Setup storage and catalog
    let storage = ObjectStorage::new(&config.storage)?;
    let catalog = Catalog::connect(&config.database_url).await?;
    catalog.migrate().await?;

    // Store the raw file in object storage
    let storage_path = "test/gfs_sample.grib2";
    storage.put(storage_path, data_bytes.clone()).await?;
    info!(path = %storage_path, size = file_size, "Stored raw file in object storage");

    // Parse GRIB2
    let mut reader = grib2_parser::Grib2Reader::new(data_bytes);
    let mut message_count = 0;
    let mut parameter_count = 0;

    while let Some(message) = reader.next_message().ok().flatten() {
        message_count += 1;

        // Register first occurrence of each parameter/level combination
        let param = &message.product_definition.parameter_short_name;
        let level = &message.product_definition.level_description;

        info!(message = message_count, param = %param, level = %level, "Found parameter");

        // Register in catalog (just once for demo)
        if parameter_count < 4 {
            let entry = CatalogEntry {
                model: "gfs".to_string(),
                parameter: param.clone(),
                level: level.clone(),
                reference_time: Utc::now(),
                forecast_hour: 0,
                bbox: BoundingBox::new(0.0, -90.0, 360.0, 90.0),
                storage_path: storage_path.to_string(),
                file_size,
            };

            match catalog.register_dataset(&entry).await {
                Ok(id) => {
                    info!(id = %id, param = %param, "Registered dataset");
                    parameter_count += 1;
                }
                Err(e) => {
                    info!(param = %param, error = %e, "Could not register (may already exist)");
                }
            }
        }
    }

    info!(
        messages = message_count,
        parameters = parameter_count,
        "Test file ingestion completed"
    );

    Ok(())
}
