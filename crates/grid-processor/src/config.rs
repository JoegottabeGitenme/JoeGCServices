//! Configuration for the grid processor.

use crate::downsample::DownsampleMethod;
use crate::types::InterpolationMethod;
use serde::{Deserialize, Serialize};

/// Configuration for the grid processor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridProcessorConfig {
    /// Memory budget for the chunk cache in megabytes.
    pub chunk_cache_size_mb: usize,

    /// Chunk dimension for Zarr files (square chunks).
    pub zarr_chunk_size: usize,

    /// Compression codec for Zarr files.
    pub zarr_compression: ZarrCompression,

    /// Compression level (1-9).
    pub zarr_compression_level: u8,

    /// Enable byte shuffle filter for better compression.
    pub zarr_shuffle: bool,

    /// Interpolation method for grid resampling.
    pub interpolation: InterpolationMethod,
}

impl Default for GridProcessorConfig {
    fn default() -> Self {
        Self {
            chunk_cache_size_mb: 1024,
            zarr_chunk_size: 512,
            zarr_compression: ZarrCompression::BloscZstd,
            zarr_compression_level: 1,
            zarr_shuffle: true,
            interpolation: InterpolationMethod::Bilinear,
        }
    }
}

impl GridProcessorConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("CHUNK_CACHE_SIZE_MB") {
            if let Ok(size) = val.parse() {
                config.chunk_cache_size_mb = size;
            }
        }

        if let Ok(val) = std::env::var("ZARR_CHUNK_SIZE") {
            if let Ok(size) = val.parse() {
                config.zarr_chunk_size = size;
            }
        }

        if let Ok(val) = std::env::var("ZARR_COMPRESSION") {
            config.zarr_compression = ZarrCompression::from_str(&val);
        }

        if let Ok(val) = std::env::var("ZARR_COMPRESSION_LEVEL") {
            if let Ok(level) = val.parse() {
                config.zarr_compression_level = level;
            }
        }

        if let Ok(val) = std::env::var("ZARR_SHUFFLE") {
            config.zarr_shuffle = val.to_lowercase() == "true" || val == "1";
        }

        if let Ok(val) = std::env::var("GRID_INTERPOLATION") {
            config.interpolation = InterpolationMethod::from_str(&val);
        }

        config
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.chunk_cache_size_mb == 0 {
            return Err("chunk_cache_size_mb must be > 0".to_string());
        }

        if self.zarr_chunk_size == 0 {
            return Err("zarr_chunk_size must be > 0".to_string());
        }

        if self.zarr_compression_level == 0 || self.zarr_compression_level > 9 {
            return Err("zarr_compression_level must be 1-9".to_string());
        }

        Ok(())
    }

    /// Get the chunk cache size in bytes.
    pub fn chunk_cache_size_bytes(&self) -> usize {
        self.chunk_cache_size_mb * 1024 * 1024
    }
}

/// Compression codec for Zarr files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZarrCompression {
    /// No compression.
    None,
    /// LZ4 compression.
    Lz4,
    /// Zstd compression.
    Zstd,
    /// Blosc with LZ4.
    BloscLz4,
    /// Blosc with Zstd (recommended).
    BloscZstd,
}

impl Default for ZarrCompression {
    fn default() -> Self {
        Self::BloscZstd
    }
}

impl ZarrCompression {
    /// Parse from string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "none" => Self::None,
            "lz4" => Self::Lz4,
            "zstd" => Self::Zstd,
            "blosc_lz4" => Self::BloscLz4,
            "blosc_zstd" => Self::BloscZstd,
            _ => Self::BloscZstd,
        }
    }

    /// Get the codec name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
            Self::BloscLz4 => "blosc_lz4",
            Self::BloscZstd => "blosc_zstd",
        }
    }
}

impl std::fmt::Display for ZarrCompression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Pyramid Configuration
// ============================================================================

/// Configuration for multi-resolution pyramid generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyramidConfig {
    /// Whether to generate pyramids during ingestion.
    pub enabled: bool,

    /// Minimum dimension threshold - stop generating levels when the
    /// smaller dimension falls below this value.
    pub min_dimension: usize,

    /// Downscale factor per level (typically 2 for halving each level).
    pub downscale_factor: usize,

    /// Default downsampling method (can be overridden per parameter).
    pub default_method: DownsampleMethod,
}

impl Default for PyramidConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_dimension: 256,
            downscale_factor: 2,
            default_method: DownsampleMethod::Mean,
        }
    }
}

impl PyramidConfig {
    /// Load pyramid configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("PYRAMID_ENABLED") {
            config.enabled = val.to_lowercase() == "true" || val == "1";
        }

        if let Ok(val) = std::env::var("PYRAMID_MIN_DIMENSION") {
            if let Ok(size) = val.parse() {
                config.min_dimension = size;
            }
        }

        if let Ok(val) = std::env::var("PYRAMID_DOWNSCALE_FACTOR") {
            if let Ok(factor) = val.parse() {
                config.downscale_factor = factor;
            }
        }

        if let Ok(val) = std::env::var("PYRAMID_DOWNSAMPLE_METHOD") {
            config.default_method = match val.to_lowercase().as_str() {
                "max" => DownsampleMethod::Max,
                "nearest" => DownsampleMethod::Nearest,
                _ => DownsampleMethod::Mean,
            };
        }

        config
    }

    /// Validate the pyramid configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.min_dimension == 0 {
            return Err("pyramid min_dimension must be > 0".to_string());
        }

        if self.downscale_factor < 2 {
            return Err("pyramid downscale_factor must be >= 2".to_string());
        }

        Ok(())
    }

    /// Calculate how many pyramid levels would be generated for a given grid size.
    ///
    /// Returns the total number of levels including level 0 (native).
    pub fn calculate_num_levels(&self, width: usize, height: usize) -> usize {
        if !self.enabled {
            return 1; // Just native
        }

        let mut levels = 1; // Start with native
        let mut w = width;
        let mut h = height;

        while w.min(h) >= self.min_dimension {
            w /= self.downscale_factor;
            h /= self.downscale_factor;
            if w > 0 && h > 0 {
                levels += 1;
            }
        }

        levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GridProcessorConfig::default();
        assert_eq!(config.chunk_cache_size_mb, 1024);
        assert_eq!(config.zarr_chunk_size, 512);
        assert_eq!(config.zarr_compression, ZarrCompression::BloscZstd);
        assert_eq!(config.zarr_compression_level, 1);
        assert!(config.zarr_shuffle);
        assert_eq!(config.interpolation, InterpolationMethod::Bilinear);
    }

    #[test]
    fn test_config_validation() {
        let mut config = GridProcessorConfig::default();
        assert!(config.validate().is_ok());

        config.chunk_cache_size_mb = 0;
        assert!(config.validate().is_err());

        config = GridProcessorConfig::default();
        config.zarr_chunk_size = 0;
        assert!(config.validate().is_err());

        config = GridProcessorConfig::default();
        config.zarr_compression_level = 0;
        assert!(config.validate().is_err());

        config.zarr_compression_level = 10;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_zarr_compression_from_str() {
        assert_eq!(ZarrCompression::from_str("none"), ZarrCompression::None);
        assert_eq!(ZarrCompression::from_str("lz4"), ZarrCompression::Lz4);
        assert_eq!(ZarrCompression::from_str("zstd"), ZarrCompression::Zstd);
        assert_eq!(
            ZarrCompression::from_str("blosc_lz4"),
            ZarrCompression::BloscLz4
        );
        assert_eq!(
            ZarrCompression::from_str("BLOSC_ZSTD"),
            ZarrCompression::BloscZstd
        );
        assert_eq!(
            ZarrCompression::from_str("invalid"),
            ZarrCompression::BloscZstd
        );
    }

    #[test]
    fn test_chunk_cache_size_bytes() {
        let config = GridProcessorConfig::default();
        assert_eq!(config.chunk_cache_size_bytes(), 1024 * 1024 * 1024);

        let config = GridProcessorConfig {
            chunk_cache_size_mb: 512,
            ..Default::default()
        };
        assert_eq!(config.chunk_cache_size_bytes(), 512 * 1024 * 1024);
    }

    #[test]
    fn test_zarr_compression_as_str() {
        assert_eq!(ZarrCompression::None.as_str(), "none");
        assert_eq!(ZarrCompression::Lz4.as_str(), "lz4");
        assert_eq!(ZarrCompression::Zstd.as_str(), "zstd");
        assert_eq!(ZarrCompression::BloscLz4.as_str(), "blosc_lz4");
        assert_eq!(ZarrCompression::BloscZstd.as_str(), "blosc_zstd");
    }

    #[test]
    fn test_zarr_compression_display() {
        assert_eq!(format!("{}", ZarrCompression::None), "none");
        assert_eq!(format!("{}", ZarrCompression::Lz4), "lz4");
        assert_eq!(format!("{}", ZarrCompression::Zstd), "zstd");
        assert_eq!(format!("{}", ZarrCompression::BloscLz4), "blosc_lz4");
        assert_eq!(format!("{}", ZarrCompression::BloscZstd), "blosc_zstd");
    }

    #[test]
    fn test_zarr_compression_default() {
        assert_eq!(ZarrCompression::default(), ZarrCompression::BloscZstd);
    }

    #[test]
    fn test_zarr_compression_eq() {
        assert_eq!(ZarrCompression::None, ZarrCompression::None);
        assert_ne!(ZarrCompression::None, ZarrCompression::Lz4);
    }

    #[test]
    fn test_zarr_compression_clone() {
        let compression = ZarrCompression::Zstd;
        let cloned = compression.clone();
        assert_eq!(compression, cloned);
    }

    #[test]
    fn test_zarr_compression_debug() {
        let debug_str = format!("{:?}", ZarrCompression::BloscZstd);
        assert!(debug_str.contains("BloscZstd"));
    }

    // PyramidConfig tests
    #[test]
    fn test_pyramid_config_default() {
        let config = PyramidConfig::default();
        assert!(config.enabled);
        assert_eq!(config.min_dimension, 256);
        assert_eq!(config.downscale_factor, 2);
        assert_eq!(config.default_method, DownsampleMethod::Mean);
    }

    #[test]
    fn test_pyramid_config_validate_ok() {
        let config = PyramidConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_pyramid_config_validate_zero_min_dimension() {
        let config = PyramidConfig {
            min_dimension: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("min_dimension"));
    }

    #[test]
    fn test_pyramid_config_validate_small_downscale_factor() {
        let config = PyramidConfig {
            downscale_factor: 1,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("downscale_factor"));
    }

    #[test]
    fn test_calculate_num_levels_disabled() {
        let config = PyramidConfig {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(config.calculate_num_levels(4096, 4096), 1);
        assert_eq!(config.calculate_num_levels(256, 256), 1);
    }

    #[test]
    fn test_calculate_num_levels_small_grid() {
        let config = PyramidConfig {
            enabled: true,
            min_dimension: 256,
            downscale_factor: 2,
            ..Default::default()
        };
        // Grid smaller than min_dimension - only native level
        assert_eq!(config.calculate_num_levels(128, 128), 1);
    }

    #[test]
    fn test_calculate_num_levels_exact_threshold() {
        let config = PyramidConfig {
            enabled: true,
            min_dimension: 256,
            downscale_factor: 2,
            ..Default::default()
        };
        // 512x512 (native, level 0)
        // 512 >= 256, divide: 256x256 (level 1)
        // 256 >= 256, divide: 128x128 (level 2)
        // 128 < 256, stop
        // Total: 3 levels
        assert_eq!(config.calculate_num_levels(512, 512), 3);
    }

    #[test]
    fn test_calculate_num_levels_large_grid() {
        let config = PyramidConfig {
            enabled: true,
            min_dimension: 256,
            downscale_factor: 2,
            ..Default::default()
        };
        // 4096x4096 (native, level 0)
        // 4096 >= 256, divide: 2048x2048 (level 1)
        // 2048 >= 256, divide: 1024x1024 (level 2)
        // 1024 >= 256, divide: 512x512 (level 3)
        // 512 >= 256, divide: 256x256 (level 4)
        // 256 >= 256, divide: 128x128 (level 5)
        // 128 < 256, stop
        // Total: 6 levels
        assert_eq!(config.calculate_num_levels(4096, 4096), 6);
    }

    #[test]
    fn test_calculate_num_levels_rectangular() {
        let config = PyramidConfig {
            enabled: true,
            min_dimension: 256,
            downscale_factor: 2,
            ..Default::default()
        };
        // Uses the smaller dimension (height)
        // 4096x512 (native, level 0)
        // min(4096,512)=512 >= 256, divide: 2048x256 (level 1)
        // min(2048,256)=256 >= 256, divide: 1024x128 (level 2)
        // min(1024,128)=128 < 256, stop
        // Total: 3 levels
        assert_eq!(config.calculate_num_levels(4096, 512), 3);
    }

    #[test]
    fn test_calculate_num_levels_factor_3() {
        let config = PyramidConfig {
            enabled: true,
            min_dimension: 100,
            downscale_factor: 3,
            ..Default::default()
        };
        // 2700x2700 (native, level 0)
        // 2700 >= 100, divide: 900x900 (level 1)
        // 900 >= 100, divide: 300x300 (level 2)
        // 300 >= 100, divide: 100x100 (level 3)
        // 100 >= 100, divide: 33x33 (level 4)
        // 33 < 100, stop
        // Total: 5 levels
        assert_eq!(config.calculate_num_levels(2700, 2700), 5);
    }

    #[test]
    fn test_grid_processor_config_clone() {
        let config = GridProcessorConfig::default();
        let cloned = config.clone();
        assert_eq!(config.chunk_cache_size_mb, cloned.chunk_cache_size_mb);
        assert_eq!(config.zarr_chunk_size, cloned.zarr_chunk_size);
    }

    #[test]
    fn test_grid_processor_config_debug() {
        let config = GridProcessorConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("GridProcessorConfig"));
        assert!(debug_str.contains("chunk_cache_size_mb"));
    }

    #[test]
    fn test_pyramid_config_clone() {
        let config = PyramidConfig::default();
        let cloned = config.clone();
        assert_eq!(config.enabled, cloned.enabled);
        assert_eq!(config.min_dimension, cloned.min_dimension);
    }

    #[test]
    fn test_pyramid_config_debug() {
        let config = PyramidConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("PyramidConfig"));
        assert!(debug_str.contains("enabled"));
    }
}
