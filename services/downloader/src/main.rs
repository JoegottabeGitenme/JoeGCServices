//! Weather data downloader service.
//!
//! Downloads weather data files from NOAA sources with:
//! - Resumable downloads (HTTP Range requests)
//! - Automatic retry with exponential backoff
//! - Progress persistence to survive restarts
//! - Triggers ingestion after download completes
//! - Automatic cleanup of ingested files
//! - HTTP status API for monitoring

mod cleanup;
mod concurrency;
mod config;
mod download;
mod model_runner;
mod scheduler;
mod server;
mod state;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use tokio::sync::broadcast;
use tracing::{info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use cleanup::{CleanupConfig, CleanupMetrics, CleanupTask};
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

    /// Total maximum concurrent downloads across all models.
    /// Each model gets at least 1 guaranteed slot, with remaining slots
    /// available as a shared pool for additional concurrency.
    #[arg(long, default_value = "10")]
    total_max_concurrent: usize,

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

    /// Disable cleanup task
    #[arg(long)]
    disable_cleanup: bool,

    /// Cleanup dry run mode - log what would be deleted without actually deleting
    #[arg(long)]
    cleanup_dry_run: bool,

    /// Cleanup interval in seconds (default: 3600 = 1 hour)
    #[arg(long, env = "CLEANUP_INTERVAL_SECS", default_value = "3600")]
    cleanup_interval_secs: u64,

    /// Days to retain completed download records (default: 7)
    #[arg(long, env = "COMPLETED_RECORD_RETENTION_DAYS", default_value = "7")]
    completed_record_retention_days: u32,

    /// Days to retain failed download records (default: 7)
    #[arg(long, env = "FAILED_RECORD_RETENTION_DAYS", default_value = "7")]
    failed_record_retention_days: u32,

    /// Max age for partial files before cleanup in seconds (default: 3600 = 1 hour)
    #[arg(long, env = "PARTIAL_FILE_MAX_AGE_SECS", default_value = "3600")]
    partial_file_max_age_secs: u64,
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

    // Create cleanup metrics (shared with server for Prometheus export)
    let cleanup_metrics = Arc::new(CleanupMetrics::new());

    // Create cleanup config
    let cleanup_config = CleanupConfig {
        enabled: !args.disable_cleanup,
        dry_run: args.cleanup_dry_run,
        interval_secs: args.cleanup_interval_secs,
        partial_file_max_age_secs: args.partial_file_max_age_secs,
        completed_record_retention_days: args.completed_record_retention_days,
        failed_record_retention_days: args.failed_record_retention_days,
        output_dir: args.output_dir.clone(),
        temp_dir: args.temp_dir.clone(),
    };

    // Create cleanup task
    let cleanup_task = CleanupTask::new(
        cleanup_config.clone(),
        state.clone(),
        cleanup_metrics.clone(),
    );

    // Run startup cleanup to handle any orphan files from previous runs
    if cleanup_config.enabled {
        info!("Running startup cleanup");
        if let Err(e) = cleanup_task.run_startup_cleanup().await {
            warn!(error = %e, "Startup cleanup failed (continuing anyway)");
        }
    }

    // Create scheduler
    let scheduler = Scheduler::new(
        download_manager.clone(),
        state.clone(),
        args.total_max_concurrent,
        args.ingester_url.clone(),
        args.config_dir.clone(),
        args.output_dir.clone(),
    )
    .await;

    // Get model schedules for the status API
    let model_schedules = scheduler.get_model_schedules();

    // Create server state
    let server_state = Arc::new(ServerState {
        download_state: state.clone(),
        model_schedules,
        cleanup_metrics: cleanup_metrics.clone(),
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

        // Start background cleanup task
        if cleanup_config.enabled {
            let cleanup_shutdown = shutdown_tx.subscribe();
            tokio::spawn(async move {
                cleanup_task.run_forever(cleanup_shutdown).await;
            });
        }

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
