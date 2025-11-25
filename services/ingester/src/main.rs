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
