//! Configuration for the grid processor.

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
}
