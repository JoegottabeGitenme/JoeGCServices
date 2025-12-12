//! Zarr V3 writer for converting grid data to chunked format.
//!
//! This module is used during ingestion to write grid data
//! in Zarr V3 format with optional sharding and compression.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use zarrs::array::{ArrayBuilder, DataType, FillValue};
use zarrs::array::codec::array_to_bytes::sharding::ShardingCodecBuilder;
use zarrs::array::codec::bytes_to_bytes::blosc::{
    BloscCodec, BloscCompressionLevel, BloscCompressor, BloscShuffleMode,
};
use zarrs::array_subset::ArraySubset;
use zarrs::storage::{ReadableStorageTraits, WritableStorageTraits};

use crate::config::{GridProcessorConfig, ZarrCompression};
use crate::error::{GridProcessorError, Result};
use crate::types::BoundingBox;

/// Helper for serde to skip NaN values.
fn is_nan_f32(v: &f32) -> bool {
    v.is_nan()
}

/// Default fill value (NaN).
fn default_fill_value() -> f32 {
    f32::NAN
}

/// Metadata for a Zarr grid to be stored in the catalog.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ZarrMetadata {
    /// Grid dimensions (width, height).
    pub shape: (usize, usize),
    /// Chunk dimensions.
    pub chunk_shape: (usize, usize),
    /// Number of chunks (x, y).
    pub num_chunks: (usize, usize),
    /// Data type.
    pub dtype: String,
    /// Fill value (stored as Option to handle NaN serialization).
    #[serde(default = "default_fill_value", skip_serializing_if = "is_nan_f32")]
    pub fill_value: f32,
    /// Bounding box.
    pub bbox: BoundingBox,
    /// Compression codec used.
    pub compression: String,
    /// Model identifier.
    pub model: String,
    /// Parameter name.
    pub parameter: String,
    /// Level description.
    pub level: String,
    /// Physical units.
    pub units: String,
    /// Reference time.
    pub reference_time: DateTime<Utc>,
    /// Forecast hour.
    pub forecast_hour: u32,
}

impl ZarrMetadata {
    /// Serialize to JSON for storage in catalog.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Deserialize from JSON.
    pub fn from_json(value: &serde_json::Value) -> Result<Self> {
        serde_json::from_value(value.clone())
            .map_err(|e| GridProcessorError::InvalidMetadata(e.to_string()))
    }
}

/// Result of writing a Zarr array.
#[derive(Debug)]
pub struct ZarrWriteResult {
    /// Metadata to store in catalog.
    pub metadata: ZarrMetadata,
    /// Total bytes written.
    pub bytes_written: u64,
}

/// Writer for creating Zarr V3 arrays from grid data.
pub struct ZarrWriter {
    config: GridProcessorConfig,
}

impl ZarrWriter {
    /// Create a new ZarrWriter with the given configuration.
    pub fn new(config: GridProcessorConfig) -> Self {
        Self { config }
    }

    /// Write grid data to a Zarr array.
    ///
    /// # Arguments
    /// * `storage` - The storage backend to write to (must be readable AND writable)
    /// * `path` - Path for the Zarr array (e.g., "/")
    /// * `data` - Grid data in row-major order (top-to-bottom, left-to-right)
    /// * `width` - Grid width (number of columns)
    /// * `height` - Grid height (number of rows)
    /// * `bbox` - Geographic bounding box
    /// * `model` - Model identifier
    /// * `parameter` - Parameter name
    /// * `level` - Level description
    /// * `units` - Physical units
    /// * `reference_time` - Reference time
    /// * `forecast_hour` - Forecast hour
    ///
    /// # Returns
    /// `ZarrWriteResult` containing metadata and bytes written
    pub fn write<S: ReadableStorageTraits + WritableStorageTraits + 'static>(
        &self,
        storage: S,
        path: &str,
        data: &[f32],
        width: usize,
        height: usize,
        bbox: &BoundingBox,
        model: &str,
        parameter: &str,
        level: &str,
        units: &str,
        reference_time: DateTime<Utc>,
        forecast_hour: u32,
    ) -> Result<ZarrWriteResult> {
        let chunk_size = self.config.zarr_chunk_size;

        // Calculate number of chunks
        let chunks_x = (width + chunk_size - 1) / chunk_size;
        let chunks_y = (height + chunk_size - 1) / chunk_size;

        // Create storage
        let store = Arc::new(storage);

        // Build the array
        let array = self.build_array(
            store.clone(),
            path,
            width,
            height,
            chunk_size,
            bbox,
            model,
            parameter,
            level,
            units,
            reference_time,
            forecast_hour,
        )?;

        // Store metadata
        array
            .store_metadata()
            .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        // Write data
        let subset = ArraySubset::new_with_start_shape(
            vec![0, 0],
            vec![height as u64, width as u64],
        )
        .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        array
            .store_array_subset_elements(&subset, data)
            .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        // Calculate bytes written (approximate)
        let bytes_written = (data.len() * std::mem::size_of::<f32>()) as u64;

        // Create metadata for catalog
        let metadata = ZarrMetadata {
            shape: (width, height),
            chunk_shape: (chunk_size, chunk_size),
            num_chunks: (chunks_x, chunks_y),
            dtype: "float32".to_string(),
            fill_value: f32::NAN,
            bbox: *bbox,
            compression: self.config.zarr_compression.as_str().to_string(),
            model: model.to_string(),
            parameter: parameter.to_string(),
            level: level.to_string(),
            units: units.to_string(),
            reference_time,
            forecast_hour,
        };

        Ok(ZarrWriteResult {
            metadata,
            bytes_written,
        })
    }

    /// Build a Zarr array with the configured settings.
    fn build_array<S: ReadableStorageTraits + WritableStorageTraits + 'static>(
        &self,
        storage: Arc<S>,
        path: &str,
        width: usize,
        height: usize,
        chunk_size: usize,
        bbox: &BoundingBox,
        model: &str,
        parameter: &str,
        level: &str,
        units: &str,
        reference_time: DateTime<Utc>,
        forecast_hour: u32,
    ) -> Result<zarrs::array::Array<S>> {
        // Build attributes
        let mut attrs = serde_json::Map::new();
        attrs.insert("model".to_string(), serde_json::json!(model));
        attrs.insert("parameter".to_string(), serde_json::json!(parameter));
        attrs.insert("level".to_string(), serde_json::json!(level));
        attrs.insert("units".to_string(), serde_json::json!(units));
        attrs.insert(
            "reference_time".to_string(),
            serde_json::json!(reference_time.to_rfc3339()),
        );
        attrs.insert("forecast_hour".to_string(), serde_json::json!(forecast_hour));
        attrs.insert(
            "bbox".to_string(),
            serde_json::json!([bbox.min_lon, bbox.min_lat, bbox.max_lon, bbox.max_lat]),
        );

        // Create chunk grid
        let chunk_grid: zarrs::array::ChunkGrid = 
            vec![chunk_size as u64, chunk_size as u64]
                .try_into()
                .map_err(|e| GridProcessorError::ConfigError(format!("{:?}", e)))?;

        // Create array builder
        let mut binding = ArrayBuilder::new(
            vec![height as u64, width as u64], // shape [rows, cols]
            DataType::Float32,
            chunk_grid,
            FillValue::from(f32::NAN),
        );
        let mut builder = binding.attributes(attrs);

        // Add compression if configured
        if self.config.zarr_compression != ZarrCompression::None {
            let codec = self.create_compression_codec()?;
            builder = builder.bytes_to_bytes_codecs(vec![codec]);
        }

        // Build the array
        builder
            .build(storage, path)
            .map_err(|e| GridProcessorError::StorageError(e.to_string()))
    }

    /// Create the compression codec based on configuration.
    fn create_compression_codec(
        &self,
    ) -> Result<Arc<dyn zarrs::array::codec::BytesToBytesCodecTraits>> {
        let level = BloscCompressionLevel::try_from(self.config.zarr_compression_level)
            .map_err(|_| GridProcessorError::ConfigError("Invalid compression level".to_string()))?;

        let shuffle = if self.config.zarr_shuffle {
            BloscShuffleMode::Shuffle
        } else {
            BloscShuffleMode::NoShuffle
        };

        // typesize is required when shuffle is enabled
        let typesize = if self.config.zarr_shuffle {
            Some(4) // f32 = 4 bytes
        } else {
            None
        };

        let compressor = match self.config.zarr_compression {
            ZarrCompression::None => {
                return Err(GridProcessorError::ConfigError(
                    "No compression configured".to_string(),
                ))
            }
            ZarrCompression::Lz4 | ZarrCompression::BloscLz4 => BloscCompressor::LZ4,
            ZarrCompression::Zstd | ZarrCompression::BloscZstd => BloscCompressor::Zstd,
        };

        // BloscCodec::new(cname, clevel, blocksize, shuffle_mode, typesize)
        let codec = BloscCodec::new(compressor, level, None, shuffle, typesize)
            .map_err(|e| GridProcessorError::ConfigError(e.to_string()))?;

        Ok(Arc::new(codec))
    }

    /// Write grid data with sharding (single file containing all chunks).
    ///
    /// This is more efficient for object storage as it reduces the number of objects.
    pub fn write_sharded<S: ReadableStorageTraits + WritableStorageTraits + 'static>(
        &self,
        storage: S,
        path: &str,
        data: &[f32],
        width: usize,
        height: usize,
        bbox: &BoundingBox,
        model: &str,
        parameter: &str,
        level: &str,
        units: &str,
        reference_time: DateTime<Utc>,
        forecast_hour: u32,
    ) -> Result<ZarrWriteResult> {
        let chunk_size = self.config.zarr_chunk_size;

        // Calculate number of chunks
        let chunks_x = (width + chunk_size - 1) / chunk_size;
        let chunks_y = (height + chunk_size - 1) / chunk_size;

        // Shard shape covers entire grid (single shard)
        let shard_shape = vec![
            (chunks_y * chunk_size) as u64,
            (chunks_x * chunk_size) as u64,
        ];

        // Create storage
        let store = Arc::new(storage);

        // Build sharding codec
        let sharding_codec = self.build_sharding_codec(chunk_size)?;

        // Build attributes
        let mut attrs = serde_json::Map::new();
        attrs.insert("model".to_string(), serde_json::json!(model));
        attrs.insert("parameter".to_string(), serde_json::json!(parameter));
        attrs.insert("level".to_string(), serde_json::json!(level));
        attrs.insert("units".to_string(), serde_json::json!(units));
        attrs.insert(
            "reference_time".to_string(),
            serde_json::json!(reference_time.to_rfc3339()),
        );
        attrs.insert("forecast_hour".to_string(), serde_json::json!(forecast_hour));
        attrs.insert(
            "bbox".to_string(),
            serde_json::json!([bbox.min_lon, bbox.min_lat, bbox.max_lon, bbox.max_lat]),
        );

        // Create chunk grid for shard
        let chunk_grid: zarrs::array::ChunkGrid = 
            shard_shape.clone()
                .try_into()
                .map_err(|e| GridProcessorError::ConfigError(format!("{:?}", e)))?;

        // Create array with sharding
        let mut binding = ArrayBuilder::new(
            vec![height as u64, width as u64],
            DataType::Float32,
            chunk_grid,
            FillValue::from(f32::NAN),
        );
        let array = binding
            .array_to_bytes_codec(Arc::new(sharding_codec))
            .attributes(attrs)
            .build(store, path)
            .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        // Store metadata
        array
            .store_metadata()
            .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        // Write data
        let subset = ArraySubset::new_with_start_shape(
            vec![0, 0],
            vec![height as u64, width as u64],
        )
        .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        array
            .store_array_subset_elements(&subset, data)
            .map_err(|e| GridProcessorError::StorageError(e.to_string()))?;

        // Calculate bytes written
        let bytes_written = (data.len() * std::mem::size_of::<f32>()) as u64;

        // Create metadata for catalog
        let metadata = ZarrMetadata {
            shape: (width, height),
            chunk_shape: (chunk_size, chunk_size),
            num_chunks: (chunks_x, chunks_y),
            dtype: "float32".to_string(),
            fill_value: f32::NAN,
            bbox: *bbox,
            compression: format!("sharded_{}", self.config.zarr_compression.as_str()),
            model: model.to_string(),
            parameter: parameter.to_string(),
            level: level.to_string(),
            units: units.to_string(),
            reference_time,
            forecast_hour,
        };

        Ok(ZarrWriteResult {
            metadata,
            bytes_written,
        })
    }

    /// Build a sharding codec with compression.
    fn build_sharding_codec(
        &self,
        chunk_size: usize,
    ) -> Result<zarrs::array::codec::array_to_bytes::sharding::ShardingCodec> {
        // Create inner chunk shape
        let inner_chunk_shape: Vec<std::num::NonZeroU64> = vec![
            std::num::NonZeroU64::new(chunk_size as u64).unwrap(),
            std::num::NonZeroU64::new(chunk_size as u64).unwrap(),
        ];

        let builder = if self.config.zarr_compression != ZarrCompression::None {
            let codec = self.create_compression_codec()?;
            ShardingCodecBuilder::new(inner_chunk_shape.into())
                .bytes_to_bytes_codecs(vec![codec])
                .build()
        } else {
            ShardingCodecBuilder::new(inner_chunk_shape.into()).build()
        };

        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zarrs_filesystem::FilesystemStore;

    fn create_test_data(width: usize, height: usize) -> Vec<f32> {
        (0..width * height).map(|i| i as f32).collect()
    }

    #[test]
    fn test_zarr_writer_simple() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let zarr_path = temp_dir.path().join("test.zarr");
        std::fs::create_dir_all(&zarr_path).expect("Failed to create dir");

        let store = FilesystemStore::new(&zarr_path).expect("Failed to create store");

        let config = GridProcessorConfig {
            zarr_compression: ZarrCompression::None,
            ..Default::default()
        };
        let writer = ZarrWriter::new(config);

        let data = create_test_data(100, 80);
        let bbox = BoundingBox::new(0.0, 0.0, 100.0, 80.0);

        let result = writer
            .write(
                store,
                "/",
                &data,
                100,
                80,
                &bbox,
                "test",
                "TEST_VAR",
                "surface",
                "K",
                Utc::now(),
                0,
            )
            .expect("Failed to write");

        assert_eq!(result.metadata.shape, (100, 80));
        assert_eq!(result.metadata.model, "test");
        assert_eq!(result.metadata.parameter, "TEST_VAR");
    }

    #[test]
    fn test_zarr_writer_with_compression() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let zarr_path = temp_dir.path().join("test_compressed.zarr");
        std::fs::create_dir_all(&zarr_path).expect("Failed to create dir");

        let store = FilesystemStore::new(&zarr_path).expect("Failed to create store");

        let config = GridProcessorConfig {
            zarr_compression: ZarrCompression::BloscZstd,
            zarr_compression_level: 1,
            zarr_shuffle: true,
            zarr_chunk_size: 32,
            ..Default::default()
        };
        let writer = ZarrWriter::new(config);

        let data = create_test_data(100, 80);
        let bbox = BoundingBox::new(0.0, 0.0, 100.0, 80.0);

        let result = writer
            .write(
                store,
                "/",
                &data,
                100,
                80,
                &bbox,
                "test",
                "TEST_VAR",
                "surface",
                "K",
                Utc::now(),
                0,
            )
            .expect("Failed to write");

        assert_eq!(result.metadata.compression, "blosc_zstd");
    }

    #[test]
    fn test_zarr_metadata_serialization() {
        let metadata = ZarrMetadata {
            shape: (1440, 721),
            chunk_shape: (512, 512),
            num_chunks: (3, 2),
            dtype: "float32".to_string(),
            fill_value: f32::NAN,
            bbox: BoundingBox::new(0.0, -90.0, 360.0, 90.0),
            compression: "blosc_zstd".to_string(),
            model: "gfs".to_string(),
            parameter: "TMP".to_string(),
            level: "2 m".to_string(),
            units: "K".to_string(),
            reference_time: Utc::now(),
            forecast_hour: 6,
        };

        let json = metadata.to_json();
        let restored = ZarrMetadata::from_json(&json).expect("Failed to deserialize");

        assert_eq!(restored.shape, metadata.shape);
        assert_eq!(restored.model, metadata.model);
        assert_eq!(restored.parameter, metadata.parameter);
    }
}
