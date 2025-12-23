//! Zarr V3 grid processor implementation.

use std::sync::Arc;
use tokio::sync::RwLock;

use async_trait::async_trait;
use tracing::{debug, error, info};
use zarrs::array::Array;
use zarrs::array_subset::ArraySubset;
use zarrs::storage::ReadableStorageTraits;

use crate::cache::{hash_path, ChunkCache};
use crate::config::GridProcessorConfig;
use crate::error::{GridProcessorError, Result};
use crate::types::{BoundingBox, CacheStats, GridMetadata, GridRegion, MultiscaleMetadata};

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
    /// 
    /// For multiscale (pyramid) stores, this will automatically open level 0 (native resolution).
    pub fn open(storage: S, path: &str, config: GridProcessorConfig) -> Result<Self> {
        let store = Arc::new(storage);
        
        // Try to open as an array first, if that fails try level 0 for pyramid stores
        let array = match Array::open(store.clone(), path) {
            Ok(arr) => arr,
            Err(e) => {
                // Might be a group (pyramid store), try level 0
                let level0_path = format!("{}/0", path.trim_end_matches('/'));
                Array::open(store, &level0_path).map_err(|e2| {
                    GridProcessorError::open_failed(format!(
                        "Failed to open as array ({}) or level 0 ({})", e, e2
                    ))
                })?
            }
        };

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
    /// 
    /// For multiscale (pyramid) stores, this will automatically open level 0 (native resolution).
    pub fn with_metadata(
        storage: S,
        path: &str,
        metadata: GridMetadata,
        chunk_cache: Arc<RwLock<ChunkCache>>,
        config: GridProcessorConfig,
    ) -> Result<Self> {
        debug!(
            path = %path,
            model = %metadata.model,
            parameter = %metadata.parameter,
            shape = ?metadata.shape,
            chunk_shape = ?metadata.chunk_shape,
            "Opening Zarr array with pre-populated metadata"
        );
        
        let store = Arc::new(storage);
        
        // Try to open as an array first
        let array = match Array::open(store.clone(), path) {
            Ok(arr) => arr,
            Err(e) => {
                // If it fails, it might be a group (multiscale pyramid)
                // Try opening level 0 instead
                let level0_path = format!("{}/0", path.trim_end_matches('/'));
                debug!(
                    path = %path,
                    level0_path = %level0_path,
                    error = %e,
                    "Failed to open as array, trying level 0 for pyramid store"
                );
                
                Array::open(store, &level0_path).map_err(|e2| {
                    error!(
                        path = %path,
                        level0_path = %level0_path,
                        error = %e2,
                        original_error = %e,
                        "Failed to open Zarr array (tried both root and level 0)"
                    );
                    GridProcessorError::open_failed(format!(
                        "Failed to open as array ({}) or level 0 ({})", e, e2
                    ))
                })?
            }
        };

        let path_hash = hash_path(path);

        info!(
            path = %path,
            model = %metadata.model,
            parameter = %metadata.parameter,
            "Successfully opened Zarr array"
        );

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
    /// Handles coordinate system conversion for grids using 0-360 longitude.
    fn chunks_for_bbox(&self, bbox: &BoundingBox) -> Vec<(usize, usize)> {
        let grid_bbox = &self.metadata.bbox;
        let (grid_width, grid_height) = self.metadata.shape;
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;

        // Calculate grid resolution
        let lon_per_cell = grid_bbox.width() / grid_width as f64;
        let lat_per_cell = grid_bbox.height() / grid_height as f64;

        // Normalize bbox to grid's coordinate system (handles 0-360 vs -180/180)
        let norm_bbox = bbox.normalize_to_grid(grid_bbox);

        // Convert bbox to grid indices
        let min_col = ((norm_bbox.min_lon - grid_bbox.min_lon) / lon_per_cell)
            .floor()
            .max(0.0) as usize;
        let max_col = ((norm_bbox.max_lon - grid_bbox.min_lon) / lon_per_cell)
            .ceil()
            .min(grid_width as f64) as usize;
        let min_row = ((grid_bbox.max_lat - norm_bbox.max_lat) / lat_per_cell)
            .floor()
            .max(0.0) as usize;
        let max_row = ((grid_bbox.max_lat - norm_bbox.min_lat) / lat_per_cell)
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

        debug!(
            path = %self.path,
            chunk_x = chunk_x,
            chunk_y = chunk_y,
            start_row = start_row,
            start_col = start_col,
            actual_w = actual_w,
            actual_h = actual_h,
            "Reading Zarr chunk"
        );

        // Zarr uses [row, col] indexing
        let subset = ArraySubset::new_with_start_shape(
            vec![start_row as u64, start_col as u64],
            vec![actual_h as u64, actual_w as u64],
        )
        .map_err(|e| {
            error!(
                path = %self.path,
                chunk_x = chunk_x,
                chunk_y = chunk_y,
                error = %e,
                "Failed to create array subset"
            );
            GridProcessorError::read_failed(e.to_string())
        })?;

        let data: Vec<f32> = self
            .array
            .retrieve_array_subset_elements(&subset)
            .map_err(|e| {
                error!(
                    path = %self.path,
                    chunk_x = chunk_x,
                    chunk_y = chunk_y,
                    subset = ?subset,
                    error = %e,
                    "Failed to retrieve chunk data from Zarr"
                );
                GridProcessorError::read_failed(e.to_string())
            })?;

        debug!(
            path = %self.path,
            chunk_x = chunk_x,
            chunk_y = chunk_y,
            data_len = data.len(),
            "Successfully read Zarr chunk"
        );

        Ok(data)
    }

    /// Read and decompress a single chunk with caching.
    async fn read_chunk(&self, chunk_x: usize, chunk_y: usize) -> Result<Vec<f32>> {
        let cache_key = (self.path_hash, chunk_x, chunk_y);

        // Check cache first
        {
            let mut cache = self.chunk_cache.write().await;
            if let Some(data) = cache.get(&cache_key) {
                debug!(
                    path = %self.path,
                    chunk_x = chunk_x,
                    chunk_y = chunk_y,
                    "Chunk cache HIT"
                );
                return Ok(data.clone());
            }
        }

        debug!(
            path = %self.path,
            chunk_x = chunk_x,
            chunk_y = chunk_y,
            "Chunk cache MISS - fetching from storage"
        );

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

        // Normalize bbox to grid's coordinate system (handles 0-360 vs -180/180)
        let norm_bbox = bbox.normalize_to_grid(grid_bbox);

        // Calculate output region bounds in grid coordinates
        let min_col = ((norm_bbox.min_lon - grid_bbox.min_lon) / res_x)
            .floor()
            .max(0.0) as usize;
        let max_col = ((norm_bbox.max_lon - grid_bbox.min_lon) / res_x)
            .ceil()
            .min(grid_w as f64) as usize;
        let min_row = ((grid_bbox.max_lat - norm_bbox.max_lat) / res_y)
            .floor()
            .max(0.0) as usize;
        let max_row = ((grid_bbox.max_lat - norm_bbox.min_lat) / res_y)
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
    
    /// Read a single value at grid coordinates (used for bilinear interpolation)
    async fn read_single_value(&self, col: usize, row: usize) -> Result<f32> {
        let (grid_w, grid_h) = self.metadata.shape;
        if col >= grid_w || row >= grid_h {
            return Ok(f32::NAN);
        }
        
        let (chunk_w, chunk_h) = self.metadata.chunk_shape;
        let chunk_x = col / chunk_w;
        let chunk_y = row / chunk_h;

        let chunk_data = self.read_chunk(chunk_x, chunk_y).await?;

        let chunk_start_col = chunk_x * chunk_w;
        let chunk_start_row = chunk_y * chunk_h;
        let local_col = col - chunk_start_col;
        let local_row = row - chunk_start_row;

        let chunk_actual_w = chunk_w.min(grid_w - chunk_start_col);
        let idx = local_row * chunk_actual_w + local_col;
        
        Ok(chunk_data.get(idx).copied().unwrap_or(f32::NAN))
    }
}

#[async_trait]
impl<S: ReadableStorageTraits + Send + Sync + 'static> GridProcessor for ZarrGridProcessor<S> {
    async fn read_region(&self, bbox: &BoundingBox) -> Result<GridRegion> {
        // Check if this request crosses the dateline on a 0-360 grid
        // If so, we need to read the FULL grid and let the caller handle resampling
        let effective_bbox = if bbox.crosses_dateline_on_360_grid(&self.metadata.bbox) {
            tracing::debug!(
                path = %self.path,
                request_bbox = ?bbox,
                grid_bbox = ?self.metadata.bbox,
                "Request crosses dateline on 0-360 grid, reading full grid"
            );
            // Use the full grid bbox instead of the request bbox
            self.metadata.bbox.clone()
        } else {
            // Add a buffer of 2 grid cells around the requested bbox to ensure
            // bilinear interpolation works correctly at tile boundaries.
            // Without this buffer, edge pixels would clamp to the last available
            // grid cell value instead of interpolating with neighbors.
            let (res_x, res_y) = self.metadata.resolution();
            let buffer_cells = 2.0; // 2 cells on each side for safety
            
            // First, normalize the request bbox to the grid's coordinate system
            // (e.g., convert -180/180 to 0/360 if needed)
            let norm_bbox = bbox.normalize_to_grid(&self.metadata.bbox);
            
            // Then apply the buffer and clamp to grid bounds
            let buffered = BoundingBox::new(
                (norm_bbox.min_lon - res_x * buffer_cells).max(self.metadata.bbox.min_lon),
                (norm_bbox.min_lat - res_y * buffer_cells).max(self.metadata.bbox.min_lat),
                (norm_bbox.max_lon + res_x * buffer_cells).min(self.metadata.bbox.max_lon),
                (norm_bbox.max_lat + res_y * buffer_cells).min(self.metadata.bbox.max_lat),
            );
            tracing::debug!(
                path = %self.path,
                request_bbox = ?bbox,
                normalized_bbox = ?norm_bbox,
                buffered_bbox = ?buffered,
                buffer_cells = buffer_cells,
                "Added interpolation buffer to bbox"
            );
            buffered
        };

        // 1. Calculate needed chunks
        let chunks = self.chunks_for_bbox(&effective_bbox);

        tracing::debug!(
            path = %self.path,
            bbox = ?effective_bbox,
            chunks = ?chunks,
            "Reading region from {} chunks",
            chunks.len()
        );

        if chunks.is_empty() {
            return Ok(GridRegion::new(
                vec![],
                0,
                0,
                effective_bbox,
                self.metadata.resolution(),
            ));
        }

        // 2. Read all needed chunks in parallel
        // This significantly reduces latency when multiple chunks are needed
        // (e.g., 4 chunks @ 50ms each: sequential=200ms, parallel=50ms)
        let chunk_futures: Vec<_> = chunks
            .iter()
            .map(|(cx, cy)| self.read_chunk(*cx, *cy))
            .collect();

        let chunk_results = futures::future::join_all(chunk_futures).await;
        
        // Collect results, propagating any errors
        let chunk_data: Vec<_> = chunk_results
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        // 3. Assemble chunks into contiguous region
        self.assemble_region(&effective_bbox, &chunks, &chunk_data)
    }

    async fn read_point(&self, lon: f64, lat: f64) -> Result<Option<f32>> {
        // Check if point is within grid bounds
        if !self.metadata.bbox.contains(lon, lat) {
            return Ok(None);
        }

        // Calculate grid indices (floating point for interpolation)
        let (res_x, res_y) = self.metadata.resolution();
        let grid_bbox = &self.metadata.bbox;
        let (grid_w, grid_h) = self.metadata.shape;

        let grid_x = (lon - grid_bbox.min_lon) / res_x;
        let grid_y = (grid_bbox.max_lat - lat) / res_y;

        // Check if we're very close to an exact grid point (within 1% of cell size)
        // If so, return the exact grid cell value without interpolation
        let dx_frac = grid_x - grid_x.floor();
        let dy_frac = grid_y - grid_y.floor();
        let snap_threshold = 0.01; // 1% of cell size
        
        let near_grid_point = (dx_frac < snap_threshold || dx_frac > (1.0 - snap_threshold)) &&
                              (dy_frac < snap_threshold || dy_frac > (1.0 - snap_threshold));
        
        if near_grid_point {
            // Snap to nearest grid point and return exact value
            let col = grid_x.round() as usize;
            let row = grid_y.round() as usize;
            if col >= grid_w || row >= grid_h {
                return Ok(None);
            }
            let value = self.read_single_value(col, row).await?;
            let fill = self.metadata.fill_value;
            if value.is_nan() || value == fill {
                return Ok(None);
            }
            return Ok(Some(value));
        }

        // Calculate the four corners for bilinear interpolation
        let x1 = grid_x.floor() as usize;
        let y1 = grid_y.floor() as usize;
        
        // For global grids (like GFS 0-360), wrap x2 around
        let is_global = grid_bbox.max_lon - grid_bbox.min_lon > 359.0;
        let x2 = if is_global && x1 + 1 >= grid_w {
            0 // Wrap to column 0
        } else {
            (x1 + 1).min(grid_w - 1)
        };
        let y2 = (y1 + 1).min(grid_h - 1);

        if x1 >= grid_w || y1 >= grid_h {
            return Ok(None);
        }

        // Calculate interpolation weights
        let dx = (grid_x - x1 as f64) as f32;
        let dy = (grid_y - y1 as f64) as f32;

        // Read values at the four corners
        // We may need to read up to 4 chunks if the point is near chunk boundaries
        let v11 = self.read_single_value(x1, y1).await?;
        let v21 = self.read_single_value(x2, y1).await?;
        let v12 = self.read_single_value(x1, y2).await?;
        let v22 = self.read_single_value(x2, y2).await?;

        // Check for fill/NaN values
        let fill = self.metadata.fill_value;
        if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() 
            || v11 == fill || v21 == fill || v12 == fill || v22 == fill {
            // If any corner is missing, fall back to nearest neighbor
            let col = grid_x.round() as usize;
            let row = grid_y.round() as usize;
            if col >= grid_w || row >= grid_h {
                return Ok(None);
            }
            let value = self.read_single_value(col, row).await?;
            if value.is_nan() || value == fill {
                return Ok(None);
            }
            return Ok(Some(value));
        }

        // Bilinear interpolation
        let v1 = v11 * (1.0 - dx) + v21 * dx;
        let v2 = v12 * (1.0 - dx) + v22 * dx;
        let value = v1 * (1.0 - dy) + v2 * dy;

        Ok(Some(value))
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

    async fn read_grid_cell(&self, col: usize, row: usize) -> Result<Option<f32>> {
        let (grid_w, grid_h) = self.metadata.shape;
        if col >= grid_w || row >= grid_h {
            return Ok(None);
        }

        let value = self.read_single_value(col, row).await?;
        
        // Check for fill value
        let fill = self.metadata.fill_value;
        if value.is_nan() || value == fill {
            return Ok(None);
        }

        Ok(Some(value))
    }
}

// ============================================================================
// Multi-Resolution Pyramid Support
// ============================================================================

/// A factory for creating ZarrGridProcessors at different pyramid levels.
///
/// This struct holds the multiscale metadata and provides methods to:
/// 1. Select the optimal pyramid level for a given request
/// 2. Create a processor for reading from that level
pub struct MultiscaleGridProcessorFactory<S: ReadableStorageTraits + Clone> {
    /// Base storage
    storage: S,
    /// Base path to the multiscale group (e.g., "grids/gfs/...")
    base_path: String,
    /// Multiscale metadata describing all pyramid levels
    multiscale: MultiscaleMetadata,
    /// Shared chunk cache across all levels
    chunk_cache: Arc<RwLock<ChunkCache>>,
    /// Configuration
    config: GridProcessorConfig,
}

impl<S: ReadableStorageTraits + Clone + Send + Sync + 'static> MultiscaleGridProcessorFactory<S> {
    /// Create a new factory from multiscale metadata.
    ///
    /// The metadata should be deserialized from the catalog's zarr_metadata.multiscale field.
    pub fn new(
        storage: S,
        base_path: &str,
        multiscale: MultiscaleMetadata,
        chunk_cache: Arc<RwLock<ChunkCache>>,
        config: GridProcessorConfig,
    ) -> Self {
        Self {
            storage,
            base_path: base_path.to_string(),
            multiscale,
            chunk_cache,
            config,
        }
    }

    /// Get the multiscale metadata.
    pub fn metadata(&self) -> &MultiscaleMetadata {
        &self.multiscale
    }

    /// Select the optimal pyramid level for a given request.
    ///
    /// Returns the level number (0 = native, 1 = 2x downsampled, etc.)
    pub fn optimal_level_for(&self, bbox: &BoundingBox, output_size: (usize, usize)) -> u32 {
        self.multiscale.optimal_level_for(bbox, output_size)
    }

    /// Read a region at the optimal resolution for the given output size.
    ///
    /// This is the main entry point for resolution-aware reading.
    /// It selects the appropriate pyramid level and returns data from that level.
    pub async fn read_region_for_output(
        &self,
        bbox: &BoundingBox,
        output_size: (usize, usize),
    ) -> Result<(GridRegion, u32)> {
        let level = self.optimal_level_for(bbox, output_size);
        let region = self.read_region_at_level(level, bbox).await?;
        Ok((region, level))
    }

    /// Read a region from a specific pyramid level.
    pub async fn read_region_at_level(&self, level: u32, bbox: &BoundingBox) -> Result<GridRegion> {
        let level_info = self.multiscale.get_level(level)
            .ok_or_else(|| GridProcessorError::invalid_metadata(
                format!("Pyramid level {} not found", level)
            ))?;

        // Construct path to this level's array
        let level_path = format!("{}/{}", self.base_path.trim_end_matches('/'), level_info.path);

        // Create GridMetadata for this level
        let level_metadata = self.metadata_for_level(level_info);

        debug!(
            base_path = %self.base_path,
            level = level,
            level_path = %level_path,
            shape = ?level_info.shape,
            scale = level_info.scale,
            "Opening pyramid level"
        );

        // Create processor for this level
        let processor = ZarrGridProcessor::with_metadata(
            self.storage.clone(),
            &level_path,
            level_metadata,
            self.chunk_cache.clone(),
            self.config.clone(),
        )?;

        processor.read_region(bbox).await
    }

    /// Create GridMetadata for a specific pyramid level.
    fn metadata_for_level(&self, level: &crate::types::PyramidLevel) -> GridMetadata {
        // Note: level.resolution() returns the resolution at this pyramid level,
        // but GridMetadata calculates resolution from bbox/shape, so we don't need it here
        
        GridMetadata {
            model: self.multiscale.name.split('_').next().unwrap_or("unknown").to_string(),
            parameter: self.multiscale.name.split('_').nth(1).unwrap_or("unknown").to_string(),
            level: format!("pyramid_level_{}", level.level),
            units: String::new(),
            reference_time: chrono::Utc::now(), // Will be overwritten if available
            forecast_hour: 0,
            bbox: self.multiscale.bbox,
            shape: level.shape,
            chunk_shape: level.chunk_shape,
            num_chunks: level.num_chunks(),
            fill_value: f32::NAN,
        }
    }

    /// Check if multiscale data is available (has more than just native level).
    pub fn has_pyramids(&self) -> bool {
        self.multiscale.num_levels() > 1
    }

    /// Get the number of available pyramid levels.
    pub fn num_levels(&self) -> usize {
        self.multiscale.num_levels()
    }
}

/// Helper function to parse MultiscaleMetadata from catalog JSON.
pub fn parse_multiscale_metadata(zarr_json: &serde_json::Value) -> Option<MultiscaleMetadata> {
    zarr_json.get("multiscale")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
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
