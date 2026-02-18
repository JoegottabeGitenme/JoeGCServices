//! Download scheduler with per-model polling and ingestion triggers.
//!
//! The scheduler coordinates multiple model download runners, each operating
//! independently with their own polling schedule and guaranteed download slots.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::concurrency::{ConcurrencyManager, ModelDownloadPermit};
use crate::config::{self, ModelConfig};
use crate::dart_runner::{self, DartConfig, DartRunner};
use crate::download::DownloadManager;
use crate::lis_runner;
use crate::model_runner::{EarthdataAuth, ModelRunner};
use crate::ndbc_runner::{self, NdbcConfig, NdbcRunner};
use crate::observation_runner::{self, ObservationConfig, ObservationRunner, TafRunner};
use crate::state::DownloadState;

/// Model schedule info for API display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSchedule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Model cycles (e.g., [0, 6, 12, 18] for GFS)
    pub cycles: Vec<u32>,
    /// Hours after cycle that data becomes available
    pub delay_hours: u32,
    /// Seconds between polls
    pub poll_interval_secs: u64,
    /// S3 bucket
    pub bucket: String,
    /// Prefix template with {date}, {cycle} placeholders
    pub prefix_template: String,
    /// File pattern (e.g., "pgrb2.0p25.f{forecast:03}")
    pub file_pattern: String,
    /// Forecast hours to download
    pub forecast_hours: Vec<u32>,
    /// Whether this is observation data (vs forecast)
    pub is_observation: bool,
    /// Maximum concurrent downloads for this model
    pub max_concurrent: usize,
}

impl From<&ModelConfig> for ModelSchedule {
    fn from(config: &ModelConfig) -> Self {
        Self {
            id: config.model.id.clone(),
            name: config.model.name.clone(),
            enabled: config.model.enabled,
            cycles: config.schedule.cycles.clone(),
            delay_hours: config.schedule.delay_hours,
            poll_interval_secs: config.schedule.poll_interval_secs,
            bucket: config.source.bucket.clone(),
            prefix_template: config.source.prefix_template.clone(),
            file_pattern: config.source.file_pattern.clone(),
            forecast_hours: config.forecast_hours(),
            is_observation: config.is_observation(),
            max_concurrent: config.schedule.max_concurrent,
        }
    }
}

/// Download scheduler coordinating multiple models.
///
/// The scheduler manages per-model download runners that operate independently,
/// each with their own polling schedule and guaranteed download slots.
#[allow(dead_code)]
pub struct Scheduler {
    download_manager: Arc<DownloadManager>,
    state: Arc<DownloadState>,
    /// Total maximum concurrent downloads across all models
    total_max_concurrent: usize,
    ingester_url: Option<String>,
    client: Client,
    config_dir: PathBuf,
    /// Output directory for completed downloads (for cleanup)
    output_dir: PathBuf,
    /// Cached model configs
    model_configs: Vec<ModelConfig>,
    /// Observation source configs (METAR)
    observation_configs: Vec<ObservationConfig>,
    /// TAF forecast configs
    taf_configs: Vec<ObservationConfig>,
    /// NDBC buoy observation configs
    ndbc_configs: Vec<NdbcConfig>,
    /// DART tsunami buoy configs
    dart_configs: Vec<DartConfig>,
    /// AWS S3 client for listing files
    s3_client: Option<aws_sdk_s3::Client>,
    /// Optional Earthdata auth for NASA GES DISC sources (NLDAS)
    earthdata_auth: Option<EarthdataAuth>,
}

impl Scheduler {
    /// Create a new scheduler.
    ///
    /// # Arguments
    /// * `download_manager` - Shared download manager
    /// * `state` - Shared download state database
    /// * `total_max_concurrent` - Total maximum concurrent downloads across all models
    /// * `ingester_url` - Optional URL for triggering ingestion
    /// * `config_dir` - Directory containing model configuration files
    /// * `output_dir` - Directory for completed downloads
    pub async fn new(
        download_manager: Arc<DownloadManager>,
        state: Arc<DownloadState>,
        total_max_concurrent: usize,
        ingester_url: Option<String>,
        config_dir: PathBuf,
        output_dir: PathBuf,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        // Load configs at startup
        let model_configs = config::load_model_configs(&config_dir).unwrap_or_else(|e| {
            warn!(error = %e, "Failed to load model configs, using defaults");
            Self::default_configs()
        });

        // Load observation configs (METAR)
        let observation_configs = Self::load_observation_configs(&config_dir, &ingester_url);

        // Load TAF forecast configs
        let taf_configs = Self::load_taf_configs(&config_dir, &ingester_url);

        // Load NDBC buoy observation configs
        let ndbc_configs = Self::load_ndbc_configs(&config_dir, &ingester_url);

        // Load DART tsunami buoy configs
        let dart_configs = Self::load_dart_configs(&config_dir, &ingester_url);

        // Initialize AWS SDK for S3 listing
        // For NOAA public buckets, we need to explicitly allow anonymous access
        // by providing credentials (they won't be used but SDK requires them)
        let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .no_credentials() // Use unsigned requests for public buckets
            .load()
            .await;
        let s3_client = Some(aws_sdk_s3::Client::new(&aws_config));

        // Initialize Earthdata auth if any NLDAS models are configured
        let has_nldas = model_configs
            .iter()
            .any(|m| m.model.enabled && m.model.id.starts_with("nldas"));

        let earthdata_auth = if has_nldas {
            match lis_runner::build_earthdata_client() {
                Ok(Some((ed_client, username, password))) => {
                    info!("Earthdata authentication configured for NLDAS downloads");
                    Some(EarthdataAuth {
                        client: ed_client,
                        username,
                        password,
                    })
                }
                Ok(None) => {
                    warn!(
                        "NLDAS models configured but Earthdata credentials not set. \
                         Set EARTHDATA_USERNAME and EARTHDATA_PASSWORD environment variables."
                    );
                    None
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Failed to build Earthdata client, NLDAS downloads will be skipped"
                    );
                    None
                }
            }
        } else {
            None
        };

        // Log concurrency configuration
        let enabled_models = model_configs.iter().filter(|m| m.model.enabled).count();
        let shared_pool_size = total_max_concurrent.saturating_sub(enabled_models);
        info!(
            total_max_concurrent = total_max_concurrent,
            enabled_models = enabled_models,
            guaranteed_slots = enabled_models,
            shared_pool_size = shared_pool_size,
            "Scheduler concurrency configuration"
        );

        if !observation_configs.is_empty() {
            info!(
                count = observation_configs.len(),
                sources = ?observation_configs.iter().map(|c| &c.id).collect::<Vec<_>>(),
                "Loaded observation source configurations"
            );
        }

        if !taf_configs.is_empty() {
            info!(
                count = taf_configs.len(),
                sources = ?taf_configs.iter().map(|c| &c.id).collect::<Vec<_>>(),
                "Loaded TAF forecast configurations"
            );
        }

        if !ndbc_configs.is_empty() {
            info!(
                count = ndbc_configs.len(),
                sources = ?ndbc_configs.iter().map(|c| &c.id).collect::<Vec<_>>(),
                "Loaded NDBC buoy observation configurations"
            );
        }

        if !dart_configs.is_empty() {
            info!(
                count = dart_configs.len(),
                sources = ?dart_configs.iter().map(|c| &c.id).collect::<Vec<_>>(),
                "Loaded DART tsunami buoy configurations"
            );
        }

        Self {
            download_manager,
            state,
            total_max_concurrent,
            ingester_url,
            client,
            config_dir,
            output_dir,
            model_configs,
            observation_configs,
            taf_configs,
            ndbc_configs,
            dart_configs,
            s3_client,
            earthdata_auth,
        }
    }

    /// Load observation source configurations from model config files (METAR).
    fn load_observation_configs(
        config_dir: &std::path::Path,
        ingester_url: &Option<String>,
    ) -> Vec<ObservationConfig> {
        let mut configs = Vec::new();
        let models_dir = config_dir.join("models");

        let ingester_base = ingester_url.as_deref().unwrap_or("http://localhost:8082");

        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    match observation_runner::load_observation_config(&path, ingester_base) {
                        Ok(Some(config)) => {
                            info!(
                                source = %config.id,
                                "Loaded observation source config"
                            );
                            configs.push(config);
                        }
                        Ok(None) => {
                            // Not an observation source, skip
                        }
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "Failed to load observation config"
                            );
                        }
                    }
                }
            }
        }

        configs
    }

    /// Load TAF forecast configurations from model config files.
    fn load_taf_configs(
        config_dir: &std::path::Path,
        ingester_url: &Option<String>,
    ) -> Vec<ObservationConfig> {
        let mut configs = Vec::new();
        let models_dir = config_dir.join("models");

        let ingester_base = ingester_url.as_deref().unwrap_or("http://localhost:8082");

        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    match observation_runner::load_taf_config(&path, ingester_base) {
                        Ok(Some(config)) => {
                            info!(
                                source = %config.id,
                                "Loaded TAF forecast config"
                            );
                            configs.push(config);
                        }
                        Ok(None) => {
                            // Not a TAF source, skip
                        }
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "Failed to load TAF config"
                            );
                        }
                    }
                }
            }
        }

        configs
    }

    /// Create a ModelRunner for the given model config, attaching Earthdata auth if needed.
    fn create_runner(&self, model: &ModelConfig, permit: ModelDownloadPermit) -> ModelRunner {
        let mut runner = ModelRunner::new(
            model.clone(),
            self.download_manager.clone(),
            self.state.clone(),
            permit,
            self.ingester_url.clone(),
            self.client.clone(),
            self.s3_client.clone(),
            self.output_dir.clone(),
        );

        // Attach Earthdata auth for NLDAS models
        if model.model.id.starts_with("nldas") {
            if let Some(ref auth) = self.earthdata_auth {
                runner = runner.with_earthdata_auth(auth.clone());
            }
        }

        runner
    }

    /// Load NDBC buoy observation configurations from model config files.
    fn load_ndbc_configs(
        config_dir: &std::path::Path,
        ingester_url: &Option<String>,
    ) -> Vec<NdbcConfig> {
        let mut configs = Vec::new();
        let models_dir = config_dir.join("models");

        let ingester_base = ingester_url.as_deref().unwrap_or("http://localhost:8082");

        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    match ndbc_runner::load_ndbc_config(&path, ingester_base) {
                        Ok(Some(config)) => {
                            info!(
                                source = %config.id,
                                "Loaded NDBC buoy observation config"
                            );
                            configs.push(config);
                        }
                        Ok(None) => {
                            // Not an NDBC source, skip
                        }
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "Failed to load NDBC config"
                            );
                        }
                    }
                }
            }
        }

        configs
    }

    /// Load DART tsunami buoy configurations from model config files.
    fn load_dart_configs(
        config_dir: &std::path::Path,
        ingester_url: &Option<String>,
    ) -> Vec<DartConfig> {
        let mut configs = Vec::new();
        let models_dir = config_dir.join("models");

        let ingester_base = ingester_url.as_deref().unwrap_or("http://localhost:8082");

        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    match dart_runner::load_dart_config(&path, ingester_base) {
                        Ok(Some(config)) => {
                            info!(
                                source = %config.id,
                                "Loaded DART tsunami buoy config"
                            );
                            configs.push(config);
                        }
                        Ok(None) => {
                            // Not a DART source, skip
                        }
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "Failed to load DART config"
                            );
                        }
                    }
                }
            }
        }

        configs
    }

    /// Get the model schedules for status display.
    pub fn get_model_schedules(&self) -> Vec<ModelSchedule> {
        self.model_configs.iter().map(ModelSchedule::from).collect()
    }

    /// Get the total maximum concurrent downloads
    #[allow(dead_code)]
    pub fn total_max_concurrent(&self) -> usize {
        self.total_max_concurrent
    }

    /// Run a single download cycle for all models sequentially.
    ///
    /// This is used for `--once` mode. For continuous operation, use `run_forever()`
    /// which runs models in parallel with proper concurrency control.
    pub async fn run_all(&self) -> Result<()> {
        // For single-run mode, create a temporary concurrency manager
        let enabled_models: Vec<_> = self
            .model_configs
            .iter()
            .filter(|m| m.model.enabled)
            .collect();

        if enabled_models.is_empty() {
            warn!("No enabled models to download");
            return Ok(());
        }

        let concurrency_manager =
            ConcurrencyManager::new(self.total_max_concurrent, enabled_models.len());

        for model in enabled_models {
            let permit = ModelDownloadPermit::new(
                model.model.id.clone(),
                concurrency_manager.shared_pool(),
                model.schedule.max_concurrent,
                concurrency_manager.active_downloads_counter(),
            );

            let runner = self.create_runner(model, permit);

            if let Err(e) = runner.run_cycle().await {
                error!(model = %model.model.id, error = %e, "Model download failed");
            }
        }

        Ok(())
    }

    /// Run a single download cycle for a specific model.
    pub async fn run_model(&self, model_id: &str) -> Result<()> {
        let model = self
            .model_configs
            .iter()
            .find(|m| m.model.id == model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        if !model.model.enabled {
            info!(model = %model_id, "Model is disabled, skipping");
            return Ok(());
        }

        // Create a single-model concurrency manager
        let concurrency_manager = ConcurrencyManager::new(self.total_max_concurrent, 1);

        let permit = ModelDownloadPermit::new(
            model.model.id.clone(),
            concurrency_manager.shared_pool(),
            model.schedule.max_concurrent,
            concurrency_manager.active_downloads_counter(),
        );

        let runner = self.create_runner(model, permit);

        runner.run_cycle().await
    }

    /// Run continuously with parallel per-model download runners.
    ///
    /// Each model gets its own independent download loop with:
    /// - Guaranteed access to at least 1 download slot
    /// - Access to shared pool slots for additional concurrency
    /// - Its own polling schedule based on `poll_interval_secs`
    pub async fn run_forever(&self, shutdown: broadcast::Receiver<()>) -> Result<()> {
        let enabled_models: Vec<_> = self
            .model_configs
            .iter()
            .filter(|m| m.model.enabled)
            .cloned()
            .collect();

        if enabled_models.is_empty() && self.observation_configs.is_empty() {
            warn!("No enabled models or observation sources to download");
            return Ok(());
        }

        // Create concurrency manager for model downloads
        let concurrency_manager =
            ConcurrencyManager::new(self.total_max_concurrent, enabled_models.len().max(1));

        info!(
            total_max = self.total_max_concurrent,
            num_models = enabled_models.len(),
            num_observation_sources = self.observation_configs.len(),
            shared_pool = concurrency_manager.shared_pool_size(),
            "Starting parallel runners"
        );

        let mut handles = Vec::new();

        // Spawn independent runner for each model
        for model in enabled_models {
            let permit = ModelDownloadPermit::new(
                model.model.id.clone(),
                concurrency_manager.shared_pool(),
                model.schedule.max_concurrent,
                concurrency_manager.active_downloads_counter(),
            );

            let runner = self.create_runner(&model, permit);

            let shutdown_rx = shutdown.resubscribe();
            let model_id = model.model.id.clone();

            handles.push(tokio::spawn(async move {
                if let Err(e) = runner.run_forever(shutdown_rx).await {
                    error!(model = %model_id, error = %e, "Model runner failed");
                }
            }));
        }

        // Spawn observation runners (METAR)
        for obs_config in &self.observation_configs {
            let runner = match ObservationRunner::new(obs_config.clone()) {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        source = %obs_config.id,
                        error = %e,
                        "Failed to create observation runner"
                    );
                    continue;
                }
            };

            let shutdown_rx = shutdown.resubscribe();
            let source_id = obs_config.id.clone();

            handles.push(tokio::spawn(async move {
                if let Err(e) = runner.run_forever(shutdown_rx).await {
                    error!(source = %source_id, error = %e, "Observation runner failed");
                }
            }));
        }

        // Spawn NDBC buoy runners
        for ndbc_config in &self.ndbc_configs {
            let runner = match NdbcRunner::new(ndbc_config.clone()) {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        source = %ndbc_config.id,
                        error = %e,
                        "Failed to create NDBC runner"
                    );
                    continue;
                }
            };

            let shutdown_rx = shutdown.resubscribe();
            let source_id = ndbc_config.id.clone();

            handles.push(tokio::spawn(async move {
                if let Err(e) = runner.run_forever(shutdown_rx).await {
                    error!(source = %source_id, error = %e, "NDBC runner failed");
                }
            }));
        }

        // Spawn DART tsunami buoy runners
        for dart_config in &self.dart_configs {
            let runner = match DartRunner::new(dart_config.clone()) {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        source = %dart_config.id,
                        error = %e,
                        "Failed to create DART runner"
                    );
                    continue;
                }
            };

            let shutdown_rx = shutdown.resubscribe();
            let source_id = dart_config.id.clone();

            handles.push(tokio::spawn(async move {
                if let Err(e) = runner.run_forever(shutdown_rx).await {
                    error!(source = %source_id, error = %e, "DART runner failed");
                }
            }));
        }

        // Spawn TAF runners
        for taf_config in &self.taf_configs {
            let runner = match TafRunner::new(taf_config.clone()) {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        source = %taf_config.id,
                        error = %e,
                        "Failed to create TAF runner"
                    );
                    continue;
                }
            };

            let shutdown_rx = shutdown.resubscribe();
            let source_id = taf_config.id.clone();

            handles.push(tokio::spawn(async move {
                if let Err(e) = runner.run_forever(shutdown_rx).await {
                    error!(source = %source_id, error = %e, "TAF runner failed");
                }
            }));
        }

        // Wait for all runners to complete (usually via shutdown signal)
        futures::future::join_all(handles).await;

        info!("All runners stopped");
        Ok(())
    }

    /// Default model configurations when YAML files aren't available.
    fn default_configs() -> Vec<ModelConfig> {
        // Return empty - configs should come from YAML files
        // In production, this would fail loudly if configs are missing
        warn!("Using default (empty) model configurations - no models will be downloaded");
        Vec::new()
    }
}
