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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_valid_config() -> TestConfig {
        TestConfig {
            name: "test".to_string(),
            description: "Test config".to_string(),
            base_url: "http://localhost:8080".to_string(),
            duration_secs: 60,
            concurrency: 10,
            requests_per_second: None,
            warmup_secs: 5,
            seed: None,
            layers: vec![LayerConfig {
                name: "test_layer".to_string(),
                style: Some("temperature".to_string()),
                weight: 1.0,
            }],
            tile_selection: TileSelection::Random {
                zoom_range: (4, 8),
                bbox: Some(BBox {
                    min_lon: -130.0,
                    min_lat: 20.0,
                    max_lon: -60.0,
                    max_lat: 55.0,
                }),
            },
            time_selection: None,
            log_requests: false,
        }
    }

    #[test]
    fn test_validate_valid_config() {
        let config = create_valid_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_zero_duration() {
        let mut config = create_valid_config();
        config.duration_secs = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duration_secs"));
    }

    #[test]
    fn test_validate_zero_concurrency() {
        let mut config = create_valid_config();
        config.concurrency = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("concurrency"));
    }

    #[test]
    fn test_validate_empty_layers() {
        let mut config = create_valid_config();
        config.layers = vec![];
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("layer"));
    }

    #[test]
    fn test_default_weight() {
        assert_eq!(default_weight(), 1.0);
    }

    #[test]
    fn test_layer_config_deserialize_with_defaults() {
        let yaml = r#"
name: test_layer
"#;
        let layer: LayerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(layer.name, "test_layer");
        assert!(layer.style.is_none());
        assert_eq!(layer.weight, 1.0); // default weight
    }

    #[test]
    fn test_layer_config_deserialize_full() {
        let yaml = r#"
name: temperature
style: gradient
weight: 2.5
"#;
        let layer: LayerConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(layer.name, "temperature");
        assert_eq!(layer.style, Some("gradient".to_string()));
        assert_eq!(layer.weight, 2.5);
    }

    #[test]
    fn test_tile_selection_random_deserialize() {
        let yaml = r#"
type: random
zoom_range: [4, 10]
bbox:
  min_lon: -130.0
  min_lat: 20.0
  max_lon: -60.0
  max_lat: 55.0
"#;
        let selection: TileSelection = serde_yaml::from_str(yaml).unwrap();
        match selection {
            TileSelection::Random { zoom_range, bbox } => {
                assert_eq!(zoom_range, (4, 10));
                assert!(bbox.is_some());
                let bbox = bbox.unwrap();
                assert_eq!(bbox.min_lon, -130.0);
                assert_eq!(bbox.max_lat, 55.0);
            }
            _ => panic!("Expected Random selection"),
        }
    }

    #[test]
    fn test_tile_selection_fixed_deserialize() {
        let yaml = r#"
type: fixed
tiles:
  - [5, 10, 15]
  - [5, 11, 15]
"#;
        let selection: TileSelection = serde_yaml::from_str(yaml).unwrap();
        match selection {
            TileSelection::Fixed { tiles } => {
                assert_eq!(tiles.len(), 2);
                assert_eq!(tiles[0], (5, 10, 15));
                assert_eq!(tiles[1], (5, 11, 15));
            }
            _ => panic!("Expected Fixed selection"),
        }
    }

    #[test]
    fn test_time_order_default() {
        assert!(matches!(TimeOrder::default(), TimeOrder::NewestFirst));
    }

    #[test]
    fn test_time_selection_none_deserialize() {
        let yaml = "type: none";
        let selection: TimeSelection = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(selection, TimeSelection::None));
    }

    #[test]
    fn test_bbox_deserialize() {
        let yaml = r#"
min_lon: -180.0
min_lat: -90.0
max_lon: 180.0
max_lat: 90.0
"#;
        let bbox: BBox = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(bbox.min_lon, -180.0);
        assert_eq!(bbox.min_lat, -90.0);
        assert_eq!(bbox.max_lon, 180.0);
        assert_eq!(bbox.max_lat, 90.0);
    }

    #[test]
    fn test_full_config_deserialize() {
        let yaml = r#"
name: quick_test
description: Quick validation test
base_url: http://localhost:8080
duration_secs: 30
concurrency: 5
warmup_secs: 3
layers:
  - name: gfs_TMP
    style: temperature
tile_selection:
  type: random
  zoom_range: [4, 6]
"#;
        let config: TestConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "quick_test");
        assert_eq!(config.duration_secs, 30);
        assert_eq!(config.concurrency, 5);
        assert_eq!(config.warmup_secs, 3);
        assert_eq!(config.layers.len(), 1);
        assert!(config.validate().is_ok());
    }
}
