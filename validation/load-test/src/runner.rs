//! HTTP request execution and load test orchestration.

use crate::config::TestConfig;
use crate::generator::TileGenerator;
use crate::metrics::{MetricsCollector, SystemConfig, TestResults};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::sleep;

/// A single logged request for debugging and visualization.
#[derive(Debug, Serialize)]
pub struct RequestLog {
    pub timestamp_ms: u64,
    pub url: String,
    pub z: u32,
    pub x: u32,
    pub y: u32,
    pub layer: String,
    pub latency_ms: f64,
    pub cache_status: String,
    pub status: u16,
}

/// Executes load tests with controlled concurrency.
pub struct LoadRunner {
    client: reqwest::Client,
    config: TestConfig,
}

/// Result of a single HTTP request.
#[derive(Debug)]
pub struct RequestResult {
    pub url: String,
    pub status: u16,
    pub latency_us: u64,
    pub bytes: usize,
    pub cache_hit: bool,
    pub timestamp: Instant,
    pub error: Option<String>,
}

impl LoadRunner {
    /// Create a new load runner.
    pub fn new(config: TestConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(config.concurrency as usize)
            .build()
            .expect("Failed to create HTTP client");

        Self { client, config }
    }

    /// Run the load test.
    pub async fn run(&mut self) -> anyhow::Result<TestResults> {
        // Fetch system configuration from API
        let system_config = self.fetch_system_config().await.ok();

        // Resolve dynamic times if using QuerySequential/QueryRandom
        println!("Initializing tile generator...");
        let generator = TileGenerator::new_async(self.config.clone()).await?;

        let total_duration =
            Duration::from_secs(self.config.duration_secs + self.config.warmup_secs);
        let warmup_duration = Duration::from_secs(self.config.warmup_secs);

        println!("Starting load test: {}", self.config.name);
        println!("  Warmup: {}s", self.config.warmup_secs);
        println!("  Test duration: {}s", self.config.duration_secs);
        println!("  Concurrency: {}", self.config.concurrency);
        if let Some(rps) = self.config.requests_per_second {
            println!("  Rate limit: {:.1} req/s", rps);
        }

        // Display system configuration if available
        if let Some(ref config) = system_config {
            println!();
            println!("System Configuration:");
            println!(
                "  L1 Cache: {} (size: {}, ttl: {}s)",
                if config.l1_cache_enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                config.l1_cache_size,
                config.l1_cache_ttl_secs
            );
            println!(
                "  Chunk Cache: {} (size: {} MB)",
                if config.chunk_cache_enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                config.chunk_cache_size_mb
            );
            println!(
                "  Prefetch: {} (rings: {}, zoom: {}-{})",
                if config.prefetch_enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                config.prefetch_rings,
                config.prefetch_min_zoom,
                config.prefetch_max_zoom
            );
            println!(
                "  Cache Warming: {}",
                if config.cache_warming_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }

        println!();

        // Create progress bar
        let pb = ProgressBar::new(self.config.duration_secs);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len}s {msg}")
                .expect("Invalid progress bar template")
                .progress_chars("##-"),
        );

        // Shared state
        let metrics = Arc::new(Mutex::new(MetricsCollector::new()));
        let generator = Arc::new(Mutex::new(generator));
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency as usize));

        // Request logging setup
        let request_log: Option<Arc<Mutex<BufWriter<File>>>> = if self.config.log_requests {
            // Ensure results directory exists - use validation/load-test/results if it exists,
            // otherwise create results/ in current directory
            let results_dir = if std::path::Path::new("validation/load-test/results").exists()
                || std::path::Path::new("validation/load-test").exists()
            {
                "validation/load-test/results"
            } else {
                "results"
            };
            std::fs::create_dir_all(results_dir)?;
            // Include scenario name in filename for easier identification
            let scenario_name = self.config.name.replace(' ', "_").to_lowercase();
            let log_path = format!(
                "{}/{}_{}.jsonl",
                results_dir,
                scenario_name,
                chrono::Utc::now().format("%Y%m%d_%H%M%S")
            );
            println!("  Logging requests to: {}", log_path);
            let file = File::create(&log_path)?;
            Some(Arc::new(Mutex::new(BufWriter::new(file))))
        } else {
            None
        };

        let start_time = Instant::now();
        let mut _requests_sent = 0u64;
        let mut warmup_complete = false;

        // Rate limiting setup
        let request_interval = self
            .config
            .requests_per_second
            .map(|rps| Duration::from_secs_f64(1.0 / rps));
        let mut last_request_time = Instant::now();

        // Main request loop
        while start_time.elapsed() < total_duration {
            // Check if warmup is complete
            if !warmup_complete && start_time.elapsed() >= warmup_duration {
                warmup_complete = true;
                pb.set_message("Test phase");
                // Reset metrics after warmup
                let mut m = metrics.lock().await;
                *m = MetricsCollector::new();
            }

            // Rate limiting
            if let Some(interval) = request_interval {
                let time_since_last = last_request_time.elapsed();
                if time_since_last < interval {
                    sleep(interval - time_since_last).await;
                }
                last_request_time = Instant::now();
            }

            // Generate URL and tile info
            let (url, tile_info) = {
                let mut gen = generator.lock().await;
                gen.next_url_with_info()
            };

            // Acquire semaphore permit for concurrency control
            let permit = semaphore.clone().acquire_owned().await?;
            let client = self.client.clone();
            let metrics_clone = metrics.clone();
            let in_warmup = !warmup_complete;
            let request_log_clone = request_log.clone();
            let elapsed_ms = start_time.elapsed().as_millis() as u64;

            // Spawn request task
            tokio::spawn(async move {
                let result = Self::execute_request_static(&client, &url).await;

                // Record metrics (skip during warmup)
                if !in_warmup {
                    let mut m = metrics_clone.lock().await;
                    if let Some(ref err) = result.error {
                        m.record_failure();
                        eprintln!("Request failed: {} - {}", url, err);
                    } else if result.status == 200 {
                        m.record_success(result.latency_us, result.bytes, result.cache_hit);
                    } else {
                        m.record_failure();
                        eprintln!("Request returned {}: {}", result.status, url);
                    }

                    // Log request if enabled
                    if let Some(ref log) = request_log_clone {
                        let cache_status = if result.cache_hit { "HIT" } else { "MISS" };
                        let log_entry = RequestLog {
                            timestamp_ms: elapsed_ms,
                            url: url.clone(),
                            z: tile_info.0,
                            x: tile_info.1,
                            y: tile_info.2,
                            layer: tile_info.3.clone(),
                            latency_ms: result.latency_us as f64 / 1000.0,
                            cache_status: cache_status.to_string(),
                            status: result.status,
                        };
                        if let Ok(json) = serde_json::to_string(&log_entry) {
                            let mut writer = log.lock().await;
                            let _ = writeln!(writer, "{}", json);
                        }
                    }
                }

                drop(permit);
            });

            _requests_sent += 1;

            // Update progress bar (only during test phase)
            if warmup_complete {
                let test_elapsed = (start_time.elapsed() - warmup_duration).as_secs();
                pb.set_position(test_elapsed.min(self.config.duration_secs));
            } else {
                pb.set_message(format!(
                    "Warmup ({}/{}s)",
                    start_time.elapsed().as_secs(),
                    self.config.warmup_secs
                ));
            }

            // Small yield to prevent tight loop
            tokio::task::yield_now().await;
        }

        // Wait for all in-flight requests to complete
        pb.set_message("Waiting for in-flight requests...");
        for _ in 0..self.config.concurrency {
            let _ = semaphore.acquire().await;
        }

        pb.finish_with_message("Complete!");
        println!();

        // Generate results
        let m = metrics.lock().await;

        // Extract layer names
        let layers = self.config.layers.iter().map(|l| l.name.clone()).collect();

        Ok(m.results(
            self.config.name.clone(),
            self.config.name.clone(), // scenario_name same as config_name for now
            layers,
            self.config.concurrency,
            system_config,
        ))
    }

    /// Fetch system configuration from WMS API
    async fn fetch_system_config(&self) -> anyhow::Result<SystemConfig> {
        let config_url = format!("{}/api/config", self.config.base_url);

        let response = self
            .client
            .get(&config_url)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch system config: HTTP {}", response.status());
        }

        let json: serde_json::Value = response.json().await?;

        // Parse the config from the API response
        let opts = &json["optimizations"];

        Ok(SystemConfig {
            l1_cache_enabled: opts["l1_cache"]["enabled"].as_bool().unwrap_or(false),
            l1_cache_size: opts["l1_cache"]["size"].as_u64().unwrap_or(0) as usize,
            l1_cache_ttl_secs: opts["l1_cache"]["ttl_secs"].as_u64().unwrap_or(0),
            l2_cache_enabled: opts["l2_cache"]["enabled"].as_bool().unwrap_or(true),
            chunk_cache_enabled: opts["chunk_cache"]["enabled"].as_bool().unwrap_or(false),
            chunk_cache_size_mb: opts["chunk_cache"]["size_mb"].as_u64().unwrap_or(0) as usize,
            prefetch_enabled: opts["prefetch"]["enabled"].as_bool().unwrap_or(false),
            prefetch_rings: opts["prefetch"]["rings"].as_u64().unwrap_or(0) as u32,
            prefetch_min_zoom: opts["prefetch"]["min_zoom"].as_u64().unwrap_or(0) as u32,
            prefetch_max_zoom: opts["prefetch"]["max_zoom"].as_u64().unwrap_or(0) as u32,
            cache_warming_enabled: opts["cache_warming"]["enabled"].as_bool().unwrap_or(false),
        })
    }

    /// Execute a single HTTP request (static version for use in spawned tasks).
    async fn execute_request_static(client: &reqwest::Client, url: &str) -> RequestResult {
        let start = Instant::now();

        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status().as_u16();

                // Check for cache hit from X-Cache header
                let cache_hit = response
                    .headers()
                    .get("x-cache")
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.to_uppercase().contains("HIT"))
                    .unwrap_or(false);

                // Read response body
                let bytes = match response.bytes().await {
                    Ok(b) => b.len(),
                    Err(_) => 0,
                };

                RequestResult {
                    url: url.to_string(),
                    status,
                    latency_us: start.elapsed().as_micros() as u64,
                    bytes,
                    cache_hit,
                    timestamp: start,
                    error: None,
                }
            }
            Err(e) => RequestResult {
                url: url.to_string(),
                status: 0,
                latency_us: start.elapsed().as_micros() as u64,
                bytes: 0,
                cache_hit: false,
                timestamp: start,
                error: Some(e.to_string()),
            },
        }
    }
}
