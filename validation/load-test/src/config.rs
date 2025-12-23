//! Configuration loading and management.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Main test configuration loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    pub name: String,
    pub description: String,
    pub base_url: String,
    pub duration_secs: u64,
    pub concurrency: u32,
    #[serde(default)]
    pub requests_per_second: Option<f64>,
    #[serde(default)]
    pub warmup_secs: u64,
    #[serde(default)]
    pub seed: Option<u64>, // Optional RNG seed for reproducible tests
    pub layers: Vec<LayerConfig>,
    pub tile_selection: TileSelection,
    #[serde(default)]
    pub time_selection: Option<TimeSelection>,
    #[serde(default)]
    pub log_requests: bool, // Log all requests to file for debugging
}

/// Layer configuration for testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerConfig {
    pub name: String,
    pub style: Option<String>,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_weight() -> f64 {
    1.0
}

/// How to select tiles for testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TileSelection {
    Random {
        zoom_range: (u32, u32),
        #[serde(default)]
        bbox: Option<BBox>,
    },
    Sequential {
        zoom: u32,
        bbox: BBox,
    },
    Fixed {
        tiles: Vec<(u32, u32, u32)>,
    },
    PanSimulation {
        start: (u32, u32, u32),
        steps: u32,
    },
}

/// Geographic bounding box.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

/// How to select time dimension for temporal testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimeSelection {
    /// Cycle through a list of specific times sequentially
    Sequential {
        times: Vec<String>, // ISO 8601 timestamps
    },
    /// Randomly select from a list of times
    Random {
        times: Vec<String>, // ISO 8601 timestamps
    },
    /// Query times from WMS GetCapabilities and select sequentially
    QuerySequential {
        layer: String, // Layer name to query times from
        count: usize,  // Number of times to select (e.g., 5 = most recent 5)
        #[serde(default)]
        order: TimeOrder, // Order to select times (newest_first or oldest_first)
    },
    /// Query times from WMS GetCapabilities and select randomly
    QueryRandom {
        layer: String, // Layer name to query times from
        count: usize,  // Number of times to select randomly from
        #[serde(default)]
        order: TimeOrder, // Which times to select from (newest_first or oldest_first)
    },
    /// No time parameter (default behavior)
    None,
}

/// Order for selecting times from WMS GetCapabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeOrder {
    #[default]
    NewestFirst,
    OldestFirst,
}

impl TestConfig {
    /// Load configuration from YAML file.
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: TestConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Validate configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.duration_secs == 0 {
            anyhow::bail!("duration_secs must be > 0");
        }
        if self.concurrency == 0 {
            anyhow::bail!("concurrency must be > 0");
        }
        if self.layers.is_empty() {
            anyhow::bail!("at least one layer must be specified");
        }
        Ok(())
    }
}
