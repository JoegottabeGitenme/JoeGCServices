//! Weather data downloader service.
//!
//! Downloads weather data files from NOAA sources with:
//! - Resumable downloads (HTTP Range requests)
//! - Automatic retry with exponential backoff
//! - Progress persistence to survive restarts
//! - Triggers ingestion after download completes
//! - HTTP status API for monitoring

mod config;
mod download;
mod scheduler;
mod server;
mod state;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use tokio::sync::broadcast;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use download::{DownloadConfig, DownloadManager};
use scheduler::Scheduler;
use server::ServerState;
use state::DownloadState;

#[derive(Parser, Debug)]
#[command(name = "downloader")]
#[command(about = "Weather data downloader with resumable downloads")]
struct Args {
    /// Run once and exit (vs continuous polling)
    #[arg(long)]
    once: bool,

    /// Specific model to download (default: all configured)
    #[arg(short, long)]
    model: Option<String>,

    /// Directory for download state database
    #[arg(long, default_value = "/data/downloader")]
    state_dir: PathBuf,

    /// Directory for temporary downloads
    #[arg(long, default_value = "/tmp/weather-downloads")]
    temp_dir: PathBuf,

    /// Directory for completed downloads
    #[arg(long, default_value = "/data/downloads")]
    output_dir: PathBuf,

    /// Maximum concurrent downloads
    #[arg(long, default_value = "4")]
    max_concurrent: usize,

    /// Maximum retry attempts
    #[arg(long, default_value = "5")]
    max_retries: u32,

    /// Ingester URL for triggering ingestion after download
    #[arg(long, env = "INGESTER_URL")]
    ingester_url: Option<String>,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Port for status HTTP server
    #[arg(long, env = "STATUS_PORT", default_value = "8081")]
    status_port: u16,

    /// Configuration directory (contains models/*.yaml)
    #[arg(long, env = "CONFIG_DIR", default_value = "config")]
    config_dir: PathBuf,

    /// Disable status HTTP server
    #[arg(long)]
    no_status_server: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment from .env file if present
    dotenvy::dotenv().ok();

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

    info!("Starting weather data downloader");

    // Create directories
    tokio::fs::create_dir_all(&args.state_dir).await?;
    tokio::fs::create_dir_all(&args.temp_dir).await?;
    tokio::fs::create_dir_all(&args.output_dir).await?;

    // Open state database
    let state_path = args.state_dir.join("downloads.db");
    let state: Arc<DownloadState> = Arc::new(DownloadState::open(&state_path).await?);

    // Create download manager
    let download_config = DownloadConfig {
        max_retries: args.max_retries,
        initial_retry_delay: Duration::from_secs(2),
        max_retry_delay: Duration::from_secs(120),
        request_timeout: Duration::from_secs(600),
        chunk_size: 64 * 1024,
        temp_dir: args.temp_dir.clone(),
        output_dir: args.output_dir.clone(),
    };
    let download_manager = Arc::new(DownloadManager::new(download_config)?);

    // Resume any in-progress downloads
    let in_progress = state.get_in_progress().await?;
    if !in_progress.is_empty() {
        info!(
            count = in_progress.len(),
            "Found in-progress downloads to resume"
        );
    }

    // Create scheduler
    let scheduler = Scheduler::new(
        download_manager.clone(),
        state.clone(),
        args.max_concurrent,
        args.ingester_url.clone(),
        args.config_dir.clone(),
    )
    .await;

    // Get model schedules for the status API
    let model_schedules = scheduler.get_model_schedules();

    // Create server state
    let server_state = Arc::new(ServerState {
        download_state: state.clone(),
        model_schedules,
    });

    // Shutdown signal
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Start status server (unless disabled or in --once mode)
    if !args.no_status_server && !args.once {
        let server_state_clone = server_state.clone();
        let status_port = args.status_port;
        tokio::spawn(async move {
            if let Err(e) = server::run_server(server_state_clone, status_port).await {
                tracing::error!(error = %e, "Status server failed");
            }
        });
    }

    if args.once {
        // Single run mode
        info!("Running single download cycle");

        if let Some(model) = &args.model {
            scheduler.run_model(model).await?;
        } else {
            scheduler.run_all().await?;
        }
    } else {
        // Continuous polling mode
        info!("Starting continuous polling");

        // Handle Ctrl+C
        let shutdown_tx_clone = shutdown_tx.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            info!("Received shutdown signal");
            shutdown_tx_clone.send(()).ok();
        });

        scheduler.run_forever(shutdown_tx.subscribe()).await?;
    }

    // Print stats
    let stats = state.get_stats().await?;
    info!(
        pending = stats.pending,
        in_progress = stats.in_progress,
        completed = stats.completed,
        failed = stats.failed,
        total_bytes = stats.total_bytes_downloaded,
        "Download session complete"
    );

    Ok(())
}
