//! Data availability checking and caching for EDR collections.
//!
//! This module ensures that EDR only advertises collections, parameters, and levels
//! that actually have data available in the catalog. This prevents 404/500 errors
//! when clients request advertised resources.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use storage::Catalog;
use tokio::sync::RwLock;

/// Cached availability information for a model.
#[derive(Debug, Clone)]
pub struct ModelAvailability {
    /// Parameters that have data, mapped to their available levels.
    /// Key: parameter name (e.g., "TMP")
    /// Value: set of level strings (e.g., {"1000 mb", "850 mb", "500 mb"})
    pub parameters: HashMap<String, HashSet<String>>,
    /// When this cache entry was created.
    pub cached_at: Instant,
}

impl ModelAvailability {
    /// Check if a specific parameter exists with any data.
    pub fn has_parameter(&self, param: &str) -> bool {
        self.parameters.contains_key(param)
    }

    /// Get available levels for a parameter.
    pub fn get_levels(&self, param: &str) -> Option<&HashSet<String>> {
        self.parameters.get(param)
    }

    /// Check if a specific parameter/level combination is available.
    pub fn is_level_available(&self, param: &str, level: &str) -> bool {
        self.parameters
            .get(param)
            .map(|levels| levels.contains(level))
            .unwrap_or(false)
    }
}

/// Cache for data availability information.
///
/// This cache stores information about what data is actually available in the
/// catalog for each model. It has a configurable TTL to balance freshness
/// against database query load.
pub struct AvailabilityCache {
    /// TTL for cache entries.
    ttl: Duration,
    /// Cached availability by model name.
    cache: RwLock<HashMap<String, ModelAvailability>>,
}

impl AvailabilityCache {
    /// Create a new availability cache with the specified TTL.
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_seconds),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get availability for a model, fetching from catalog if not cached or expired.
    ///
    /// Returns `None` if the model has no data available.
    pub async fn get_model_availability(
        &self,
        catalog: &Catalog,
        model: &str,
    ) -> Option<ModelAvailability> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(model) {
                if entry.cached_at.elapsed() < self.ttl {
                    return Some(entry.clone());
                }
            }
        }

        // Fetch from catalog
        let availability = self.fetch_model_availability(catalog, model).await;

        // Update cache (even if None, to avoid repeated queries for models with no data)
        if let Some(ref avail) = availability {
            let mut cache = self.cache.write().await;
            cache.insert(model.to_string(), avail.clone());
        }

        availability
    }

    /// Fetch availability directly from the catalog.
    async fn fetch_model_availability(
        &self,
        catalog: &Catalog,
        model: &str,
    ) -> Option<ModelAvailability> {
        // Get available parameters for this model
        let params = match catalog.list_parameters(model).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to list parameters for model {}: {}", model, e);
                return None;
            }
        };

        if params.is_empty() {
            tracing::debug!("Model {} has no parameters with data", model);
            return None;
        }

        let mut parameters: HashMap<String, HashSet<String>> = HashMap::new();

        for param in params {
            // Get available levels for each parameter
            let levels = catalog
                .get_available_levels(model, &param)
                .await
                .unwrap_or_default();

            if !levels.is_empty() {
                parameters.insert(param, levels.into_iter().collect());
            }
        }

        if parameters.is_empty() {
            tracing::debug!("Model {} has no parameters with level data", model);
            return None;
        }

        // Log available parameters with their level counts
        let param_summary: Vec<String> = parameters
            .iter()
            .map(|(p, levels)| format!("{}({})", p, levels.len()))
            .collect();
        tracing::info!(
            "Model {} availability: {} params [{}]",
            model,
            parameters.len(),
            param_summary.join(", ")
        );

        Some(ModelAvailability {
            parameters,
            cached_at: Instant::now(),
        })
    }

    /// Invalidate cache for a specific model (call after data changes).
    pub async fn invalidate(&self, model: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(model);
        tracing::debug!("Invalidated availability cache for model {}", model);
    }

    /// Invalidate all cached entries.
    pub async fn invalidate_all(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        tracing::debug!("Invalidated all availability cache entries");
    }
}

/// Helper functions for matching config levels to database level strings.
pub mod level_matching {
    use crate::config::LevelFilter;

    /// Format a numeric level value to match catalog format.
    ///
    /// Examples:
    /// - isobaric: 1000 -> "1000 mb"
    /// - height_above_ground: 2 -> "2 m above ground"
    /// - height_above_msl: 0 -> "0 m above MSL"
    pub fn format_level_string(value: f64, level_filter: &LevelFilter) -> String {
        let int_value = value as i64;
        match level_filter.level_type.as_str() {
            "isobaric" | "pressure" => format!("{} mb", int_value),
            "height_above_ground" | "height_agl" => format!("{} m above ground", int_value),
            "height_above_msl" | "height_msl" => format!("{} m above MSL", int_value),
            "surface" => "surface".to_string(),
            "entire_atmosphere" | "atmosphere" => "entire atmosphere".to_string(),
            "cloud_layer" => {
                // Cloud layers use special codes
                match level_filter.level_code {
                    Some(214) => "low cloud layer".to_string(),
                    Some(224) => "middle cloud layer".to_string(),
                    Some(234) => "high cloud layer".to_string(),
                    _ => format!("cloud layer {}", int_value),
                }
            }
            _ => {
                // Generic fallback - try common patterns
                if value == 0.0 {
                    "surface".to_string()
                } else {
                    format!("{}", int_value)
                }
            }
        }
    }

    /// Format a named level value to match catalog format.
    ///
    /// Handles special named levels like "surface", "entire_atmosphere", etc.
    pub fn format_named_level(name: &str, level_filter: &LevelFilter) -> String {
        match name.to_lowercase().as_str() {
            "surface" => "surface".to_string(),
            "entire_atmosphere" | "atmosphere" => "entire atmosphere".to_string(),
            "mean_sea_level" | "msl" => "mean sea level".to_string(),
            "low_cloud_layer" => "low cloud layer".to_string(),
            "middle_cloud_layer" => "middle cloud layer".to_string(),
            "high_cloud_layer" => "high cloud layer".to_string(),
            _ => {
                // Try to parse as numeric and format
                if let Ok(v) = name.parse::<f64>() {
                    format_level_string(v, level_filter)
                } else {
                    name.to_string()
                }
            }
        }
    }

    /// Parse numeric value from level string.
    ///
    /// Examples:
    /// - "1000 mb" -> Some(1000.0)
    /// - "2 m above ground" -> Some(2.0)
    /// - "surface" -> None
    pub fn parse_level_numeric(level_str: &str) -> Option<f64> {
        // Extract first number from string
        level_str
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<f64>().ok())
    }

    /// Check if a level string matches a given level filter type.
    pub fn level_matches_filter(level_str: &str, level_filter: &LevelFilter) -> bool {
        let level_lower = level_str.to_lowercase();

        match level_filter.level_type.as_str() {
            "isobaric" | "pressure" => level_lower.contains("mb") || level_lower.contains("hpa"),
            "height_above_ground" | "height_agl" => level_lower.contains("above ground"),
            "height_above_msl" | "height_msl" => level_lower.contains("above msl"),
            "surface" => level_lower == "surface",
            "entire_atmosphere" | "atmosphere" => {
                level_lower.contains("entire atmosphere") || level_lower.contains("atmosphere")
            }
            "cloud_layer" => level_lower.contains("cloud layer"),
            "mean_sea_level" | "msl" => level_lower.contains("mean sea level"),
            _ => true, // Unknown type - accept all
        }
    }
}

#[cfg(test)]
mod tests {
    use super::level_matching::*;
    use crate::config::LevelFilter;

    #[test]
    fn test_format_level_string_isobaric() {
        let filter = LevelFilter {
            level_type: "isobaric".to_string(),
            level_code: Some(100),
            level_codes: None,
        };
        assert_eq!(format_level_string(1000.0, &filter), "1000 mb");
        assert_eq!(format_level_string(500.0, &filter), "500 mb");
    }

    #[test]
    fn test_format_level_string_height_agl() {
        let filter = LevelFilter {
            level_type: "height_above_ground".to_string(),
            level_code: Some(103),
            level_codes: None,
        };
        assert_eq!(format_level_string(2.0, &filter), "2 m above ground");
        assert_eq!(format_level_string(10.0, &filter), "10 m above ground");
    }

    #[test]
    fn test_parse_level_numeric() {
        assert_eq!(parse_level_numeric("1000 mb"), Some(1000.0));
        assert_eq!(parse_level_numeric("2 m above ground"), Some(2.0));
        assert_eq!(parse_level_numeric("surface"), None);
    }
}
