//! Grid processor trait and implementations.

mod zarr;

pub use zarr::ZarrGridProcessor;

use async_trait::async_trait;

use crate::error::Result;
use crate::types::{BoundingBox, CacheStats, GridMetadata, GridRegion};

/// Trait for accessing grid data with efficient partial reads.
///
/// This trait abstracts over different grid data formats (Zarr, etc.)
/// and provides efficient region-based access to gridded data.
#[async_trait]
pub trait GridProcessor: Send + Sync {
    /// Read grid data for a geographic region.
    ///
    /// Returns only the grid points that fall within the bounding box,
    /// efficiently fetching only the chunks needed.
    ///
    /// # Arguments
    /// * `bbox` - Geographic bounding box to read
    ///
    /// # Returns
    /// * `GridRegion` containing the data and metadata for the region
    async fn read_region(&self, bbox: &BoundingBox) -> Result<GridRegion>;

    /// Read a single point value (for GetFeatureInfo).
    ///
    /// # Arguments
    /// * `lon` - Longitude in degrees
    /// * `lat` - Latitude in degrees
    ///
    /// # Returns
    /// * `Some(value)` if the point is within the grid and has data
    /// * `None` if the point is outside the grid or is a fill value
    async fn read_point(&self, lon: f64, lat: f64) -> Result<Option<f32>>;

    /// Get metadata about the grid.
    fn metadata(&self) -> &GridMetadata;

    /// Prefetch chunks for anticipated requests.
    ///
    /// This is a hint to the processor that these regions will likely
    /// be requested soon. The implementation may choose to fetch
    /// and cache the relevant chunks proactively.
    async fn prefetch(&self, bboxes: &[BoundingBox]);

    /// Get cache statistics for monitoring.
    fn cache_stats(&self) -> CacheStats;
}
