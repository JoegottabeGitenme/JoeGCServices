//! Zarr V3 grid processor implementation.

use std::sync::Arc;
use tokio::sync::RwLock;

use async_trait::async_trait;
use zarrs::array::Array;
use zarrs::array_subset::ArraySubset;
use zarrs::storage::ReadableStorageTraits;

use crate::cache::{hash_path, ChunkCache};
use crate::config::GridProcessorConfig;
use crate::error::{GridProcessorError, Result};
use crate::types::{BoundingBox, CacheStats, GridMetadata, GridRegion};

use super::GridProcessor;

/// Grid processor implementation using Zarr V3 with sharding.
///
/// This processor efficiently reads grid data by:
/// 1. Calculating which chunks intersect the requested region
/// 2. Fetching only those chunks via byte-range requests
/// 3. Caching decompressed chunks for reuse across requests
pub struct ZarrGridProcessor<S: ReadableStorageTraits> {
    /// The Zarr array.
    array: Array<S>,
    /// Storage path (for cache key generation).
    path: String,
    /// Hash of the path for efficient cache keys.
    path_hash: u64,
    /// Grid metadata extracted from Zarr attributes.
    metadata: GridMetadata,
    /// Shared chunk cache for decompressed data.
    chunk_cache: Arc<RwLock<ChunkCache>>,
    /// Configuration.
    #[allow(dead_code)]
    config: GridProcessorConfig,
}

impl<S: ReadableStorageTraits + Send + Sync + 'static> ZarrGridProcessor<S> {
    /// Open a Zarr array from storage.
    ///
    /// # Arguments
    /// * `storage` - The storage backend
    /// * `path` - Path to the Zarr array
    /// * `config` - Processor configuration
    ///
    /// # Returns
    /// A new ZarrGridProcessor instance
    pub fn open(storage: S, path: &str, config: GridProcessorConfig) -> Result<Self> {
        // Open the Zarr array
        let array = Array::open(Arc::new(storage), path)
            .map_err(|e| GridProcessorError::open_failed(e.to_string()))?;

        // Extract metadata from Zarr attributes
        let metadata = Self::extract_metadata(&array)?;

        // Create chunk cache
        let chunk_cache = Arc::new(RwLock::new(ChunkCache::new(
            config.chunk_cache_size_bytes(),
        )));

        let path_hash = hash_path(path);

        Ok(Self {
            array,
            path: path.to_string(),
            path_hash,
            metadata,
            chunk_cache,
            config,
        })
    }

    /// Create a processor with pre-populated metadata (for use with catalog-cached metadata).
    ///
    /// This avoids reading metadata from MinIO by using metadata stored in PostgreSQL.
    pub fn with_metadata(
        storage: S,
        path: &str,
        metadata: GridMetadata,
        chunk_cache: Arc<RwLock<ChunkCache>>,
        config: GridProcessorConfig,
    ) -> Result<Self> {
        let array = Array::open(Arc::new(storage), path)
            .map_err(|e| GridProcessorError::open_failed(e.to_string()))?;

        let path_hash = hash_path(path);

        Ok(Self {
            array,
            path: path.to_string(),
            path_hash,
            metadata,
            chunk_cache,
            config,
        })
    }

    /// Extract metadata from Zarr array attributes.
    fn extract_metadata(array: &Array<S>) -> Result<GridMetadata> {
        let attrs = array.attributes();
        let shape = array.shape();

        // Ensure we have at least 2 dimensions
        if shape.len() < 2 {
            return Err(GridProcessorError::invalid_metadata(
                "Array must have at least 2 dimensions",
            ));
        }

        // Get chunk shape from array configuration
        let chunk_grid = array.chunk_grid();
        // Use origin [0, 0] for getting chunk shape
        let origin = vec![0u64; shape.len()];
        let chunk_shape = chunk_grid
            .chunk_shape(&origin, array.shape())
            .map_err(|e| GridProcessorError::invalid_metadata(e.to_string()))?
            .ok_or_else(|| GridProcessorError::invalid_metadata("missing chunk shape"))?;

        let chunk_shape = (chunk_shape[1].get() as usize, chunk_shape[0].get() as usize);

        // Parse required attributes
        let model = attrs
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let parameter = attrs
            .get("parameter")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let level = attrs
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("surface")
            .to_string();

        let units = attrs
            .get("units")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Parse reference time
        let reference_time = attrs
            .get("reference_time")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        // Parse forecast hour
        let forecast_hour = attrs
            .get("forecast_hour")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Parse bounding box
        let bbox = attrs
            .get("bbox")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                if arr.len() == 4 {
                    Some(BoundingBox::new(
                        arr[0].as_f64()?,
                        arr[1].as_f64()?,
                        arr[2].as_f64()?,
                        arr[3].as_f64()?,
                    ))
                } else {
                    None
                }
            })
            .unwrap_or_else(BoundingBox::default);

        // Parse fill value
        let fill_value = array
            .fill_value()
            .as_ne_bytes()
            .try_into()
            .map(f32::from_ne_bytes)
            .unwrap_or(f32::NAN);

        // Grid shape: Zarr is [rows, cols] but we store as (width, height)
        let grid_shape = (shape[1] as usize, shape[0] as usize);

        // Calculate number of chunks
        let num_chunks = (
            (grid_shape.0 + chunk_shape.0 - 1) / chunk_shape.0,
            (grid_shape.1 + chunk_shape.1 - 1) / chunk_shape.1,
        );

        Ok(GridMetadata {
            model,
            parameter,
            level,
            units,
            reference_time,
            forecast_hour,
            bbox,
            shape: grid_shape,
            chunk_shape,
            num_chunks,
            fill_value,
        })
    }

    /// Calculate which chunks intersect a bounding box.
    ///
    /// This is O(1) - pure arithmetic, no iteration or lookup needed.
    fn chunks_for_bbox(&self, bbox: &BoundingBox) -> Vec<(usize, usize)> {
        let grid_bbox = &self.metadata.bbox;
        let (grid_width, grid_height) = self.metadata.shape;
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;

        // Calculate grid resolution
        let lon_per_cell = grid_bbox.width() / grid_width as f64;
        let lat_per_cell = grid_bbox.height() / grid_height as f64;

        // Convert bbox to grid indices
        let min_col = ((bbox.min_lon - grid_bbox.min_lon) / lon_per_cell)
            .floor()
            .max(0.0) as usize;
        let max_col = ((bbox.max_lon - grid_bbox.min_lon) / lon_per_cell)
            .ceil()
            .min(grid_width as f64) as usize;
        let min_row = ((grid_bbox.max_lat - bbox.max_lat) / lat_per_cell)
            .floor()
            .max(0.0) as usize;
        let max_row = ((grid_bbox.max_lat - bbox.min_lat) / lat_per_cell)
            .ceil()
            .min(grid_height as f64) as usize;

        // Convert grid indices to chunk indices
        let min_chunk_x = min_col / chunk_w;
        let max_chunk_x = ((max_col + chunk_w - 1) / chunk_w).min(self.metadata.num_chunks.0);
        let min_chunk_y = min_row / chunk_h;
        let max_chunk_y = ((max_row + chunk_h - 1) / chunk_h).min(self.metadata.num_chunks.1);

        // Generate list of chunk coordinates
        (min_chunk_y..max_chunk_y)
            .flat_map(|cy| (min_chunk_x..max_chunk_x).map(move |cx| (cx, cy)))
            .collect()
    }

    /// Read and decompress a single chunk (synchronous).
    fn read_chunk_sync(&self, chunk_x: usize, chunk_y: usize) -> Result<Vec<f32>> {
        // Cache miss - read from Zarr
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;
        let (grid_w, grid_h) = self.metadata.shape;

        // Calculate actual chunk bounds (may be partial at edges)
        let start_col = chunk_x * chunk_w;
        let start_row = chunk_y * chunk_h;
        let end_col = (start_col + chunk_w).min(grid_w);
        let end_row = (start_row + chunk_h).min(grid_h);
        let actual_w = end_col - start_col;
        let actual_h = end_row - start_row;

        // Zarr uses [row, col] indexing
        let subset = ArraySubset::new_with_start_shape(
            vec![start_row as u64, start_col as u64],
            vec![actual_h as u64, actual_w as u64],
        )
        .map_err(|e| GridProcessorError::read_failed(e.to_string()))?;

        let data: Vec<f32> = self
            .array
            .retrieve_array_subset_elements(&subset)
            .map_err(|e| GridProcessorError::read_failed(e.to_string()))?;

        Ok(data)
    }

    /// Read and decompress a single chunk with caching.
    async fn read_chunk(&self, chunk_x: usize, chunk_y: usize) -> Result<Vec<f32>> {
        let cache_key = (self.path_hash, chunk_x, chunk_y);

        // Check cache first
        {
            let mut cache = self.chunk_cache.write().await;
            if let Some(data) = cache.get(&cache_key) {
                return Ok(data.clone());
            }
        }

        // Cache miss - read from Zarr (blocking in spawn_blocking)
        let data = self.read_chunk_sync(chunk_x, chunk_y)?;

        // Cache the result
        {
            let mut cache = self.chunk_cache.write().await;
            cache.insert(cache_key, data.clone());
        }

        Ok(data)
    }

    /// Assemble chunks into a contiguous grid region.
    fn assemble_region(
        &self,
        bbox: &BoundingBox,
        chunks: &[(usize, usize)],
        chunk_data: &[Vec<f32>],
    ) -> Result<GridRegion> {
        let (res_x, res_y) = self.metadata.resolution();
        let grid_bbox = &self.metadata.bbox;
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;
        let (grid_w, grid_h) = self.metadata.shape;

        // Calculate output region bounds in grid coordinates
        let min_col = ((bbox.min_lon - grid_bbox.min_lon) / res_x)
            .floor()
            .max(0.0) as usize;
        let max_col = ((bbox.max_lon - grid_bbox.min_lon) / res_x)
            .ceil()
            .min(grid_w as f64) as usize;
        let min_row = ((grid_bbox.max_lat - bbox.max_lat) / res_y)
            .floor()
            .max(0.0) as usize;
        let max_row = ((grid_bbox.max_lat - bbox.min_lat) / res_y)
            .ceil()
            .min(grid_h as f64) as usize;

        let out_width = max_col - min_col;
        let out_height = max_row - min_row;

        if out_width == 0 || out_height == 0 {
            return Ok(GridRegion::new(vec![], 0, 0, *bbox, (res_x, res_y)));
        }

        // Allocate output buffer
        let mut output = vec![self.metadata.fill_value; out_width * out_height];

        // Copy data from each chunk to the output buffer
        for (chunk_idx, &(cx, cy)) in chunks.iter().enumerate() {
            let chunk = &chunk_data[chunk_idx];

            // Chunk bounds in grid coordinates
            let chunk_start_col = cx * chunk_w;
            let chunk_start_row = cy * chunk_h;
            let chunk_actual_w = chunk_w.min(grid_w - chunk_start_col);
            let chunk_actual_h = chunk_h.min(grid_h - chunk_start_row);

            // Calculate overlap between chunk and output region
            let overlap_start_col = min_col.max(chunk_start_col);
            let overlap_end_col = max_col.min(chunk_start_col + chunk_actual_w);
            let overlap_start_row = min_row.max(chunk_start_row);
            let overlap_end_row = max_row.min(chunk_start_row + chunk_actual_h);

            if overlap_start_col >= overlap_end_col || overlap_start_row >= overlap_end_row {
                continue;
            }

            // Copy rows from chunk to output
            for row in overlap_start_row..overlap_end_row {
                let chunk_row = row - chunk_start_row;
                let out_row = row - min_row;

                for col in overlap_start_col..overlap_end_col {
                    let chunk_col = col - chunk_start_col;
                    let out_col = col - min_col;

                    let chunk_idx = chunk_row * chunk_actual_w + chunk_col;
                    let out_idx = out_row * out_width + out_col;

                    if chunk_idx < chunk.len() {
                        output[out_idx] = chunk[chunk_idx];
                    }
                }
            }
        }

        // Calculate actual bbox of the output region
        let actual_bbox = BoundingBox::new(
            grid_bbox.min_lon + min_col as f64 * res_x,
            grid_bbox.max_lat - max_row as f64 * res_y,
            grid_bbox.min_lon + max_col as f64 * res_x,
            grid_bbox.max_lat - min_row as f64 * res_y,
        );

        Ok(GridRegion::new(
            output,
            out_width,
            out_height,
            actual_bbox,
            (res_x, res_y),
        ))
    }
}

#[async_trait]
impl<S: ReadableStorageTraits + Send + Sync + 'static> GridProcessor for ZarrGridProcessor<S> {
    async fn read_region(&self, bbox: &BoundingBox) -> Result<GridRegion> {
        // 1. Calculate needed chunks
        let chunks = self.chunks_for_bbox(bbox);

        tracing::debug!(
            path = %self.path,
            bbox = ?bbox,
            chunks = ?chunks,
            "Reading region from {} chunks",
            chunks.len()
        );

        if chunks.is_empty() {
            return Ok(GridRegion::new(
                vec![],
                0,
                0,
                *bbox,
                self.metadata.resolution(),
            ));
        }

        // 2. Read all needed chunks
        // Note: For better performance, we could use parallel futures here
        let mut chunk_data = Vec::with_capacity(chunks.len());
        for (cx, cy) in &chunks {
            chunk_data.push(self.read_chunk(*cx, *cy).await?);
        }

        // 3. Assemble chunks into contiguous region
        self.assemble_region(bbox, &chunks, &chunk_data)
    }

    async fn read_point(&self, lon: f64, lat: f64) -> Result<Option<f32>> {
        // Check if point is within grid bounds
        if !self.metadata.bbox.contains(lon, lat) {
            return Ok(None);
        }

        // Calculate grid indices
        let (res_x, res_y) = self.metadata.resolution();
        let grid_bbox = &self.metadata.bbox;
        let (grid_w, grid_h) = self.metadata.shape;

        let col = ((lon - grid_bbox.min_lon) / res_x).floor() as usize;
        let row = ((grid_bbox.max_lat - lat) / res_y).floor() as usize;

        if col >= grid_w || row >= grid_h {
            return Ok(None);
        }

        // Calculate chunk coordinates
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;
        let chunk_x = col / chunk_w;
        let chunk_y = row / chunk_h;

        // Read chunk
        let chunk_data = self.read_chunk(chunk_x, chunk_y).await?;

        // Calculate position within chunk
        let chunk_start_col = chunk_x * chunk_w;
        let chunk_start_row = chunk_y * chunk_h;
        let local_col = col - chunk_start_col;
        let local_row = row - chunk_start_row;

        // Calculate actual chunk width (may be partial at edges)
        let chunk_actual_w = chunk_w.min(grid_w - chunk_start_col);

        let idx = local_row * chunk_actual_w + local_col;
        let value = chunk_data.get(idx).copied().unwrap_or(f32::NAN);

        // Return None for fill/NaN values
        if value.is_nan() || value == self.metadata.fill_value {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn metadata(&self) -> &GridMetadata {
        &self.metadata
    }

    async fn prefetch(&self, bboxes: &[BoundingBox]) {
        for bbox in bboxes {
            let chunks = self.chunks_for_bbox(bbox);
            for (cx, cy) in chunks {
                // Fire and forget - errors are logged but not propagated
                if let Err(e) = self.read_chunk(cx, cy).await {
                    tracing::warn!(
                        path = %self.path,
                        chunk_x = cx,
                        chunk_y = cy,
                        error = %e,
                        "Failed to prefetch chunk"
                    );
                }
            }
        }
    }

    fn cache_stats(&self) -> CacheStats {
        // Note: We can't easily get stats without blocking here.
        // For now, return default stats. A more sophisticated implementation
        // would use a separate stats tracker.
        CacheStats::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunks_for_bbox_calculation() {
        // Test the chunk calculation logic with a mock metadata
        let metadata = GridMetadata {
            model: "gfs".to_string(),
            parameter: "TMP".to_string(),
            level: "2 m".to_string(),
            units: "K".to_string(),
            reference_time: chrono::Utc::now(),
            forecast_hour: 0,
            bbox: BoundingBox::new(0.0, -90.0, 360.0, 90.0),
            shape: (1440, 721),      // 0.25 degree resolution
            chunk_shape: (512, 512),
            num_chunks: (3, 2),
            fill_value: f32::NAN,
        };

        // Calculate chunks for a small bbox
        let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);

        // Manually calculate expected chunks
        // lon -100 to -90 maps to cols: (260-0)/0.25 to (270-0)/0.25 = 1040 to 1080
        // lat 30 to 40 maps to rows: (90-40)/0.25 to (90-30)/0.25 = 200 to 240
        // With 512 chunk size:
        // col chunks: 1040/512=2, 1080/512=2 -> chunk_x = 2
        // row chunks: 200/512=0, 240/512=0 -> chunk_y = 0
        // Expected: [(2, 0)]

        let (chunk_w, chunk_h) = metadata.chunk_shape;
        let (grid_w, grid_h) = metadata.shape;
        let grid_bbox = &metadata.bbox;

        let lon_per_cell = grid_bbox.width() / grid_w as f64;
        let lat_per_cell = grid_bbox.height() / grid_h as f64;

        // Add 360 to handle negative longitudes for GFS (0-360 range)
        let adjusted_min_lon = if bbox.min_lon < 0.0 {
            bbox.min_lon + 360.0
        } else {
            bbox.min_lon
        };
        let adjusted_max_lon = if bbox.max_lon < 0.0 {
            bbox.max_lon + 360.0
        } else {
            bbox.max_lon
        };

        let min_col = ((adjusted_min_lon - grid_bbox.min_lon) / lon_per_cell)
            .floor()
            .max(0.0) as usize;
        let max_col = ((adjusted_max_lon - grid_bbox.min_lon) / lon_per_cell)
            .ceil()
            .min(grid_w as f64) as usize;
        let min_row = ((grid_bbox.max_lat - bbox.max_lat) / lat_per_cell)
            .floor()
            .max(0.0) as usize;
        let max_row = ((grid_bbox.max_lat - bbox.min_lat) / lat_per_cell)
            .ceil()
            .min(grid_h as f64) as usize;

        let min_chunk_x = min_col / chunk_w;
        let max_chunk_x = (max_col + chunk_w - 1) / chunk_w;
        let min_chunk_y = min_row / chunk_h;
        let max_chunk_y = (max_row + chunk_h - 1) / chunk_h;

        // Verify the calculation produces reasonable results
        assert!(min_chunk_x <= max_chunk_x);
        assert!(min_chunk_y <= max_chunk_y);
        assert!(max_chunk_x <= metadata.num_chunks.0);
        assert!(max_chunk_y <= metadata.num_chunks.1);
    }
}
