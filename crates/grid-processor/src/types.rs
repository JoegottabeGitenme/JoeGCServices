//! Core types for grid processing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A geographic bounding box in WGS84 coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BoundingBox {
    /// Create a new bounding box.
    pub fn new(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Self {
        Self {
            min_lon,
            min_lat,
            max_lon,
            max_lat,
        }
    }

    /// Check if this bounding box intersects another.
    pub fn intersects(&self, other: &BoundingBox) -> bool {
        !(self.max_lon < other.min_lon
            || self.min_lon > other.max_lon
            || self.max_lat < other.min_lat
            || self.min_lat > other.max_lat)
    }

    /// Get the width in degrees.
    pub fn width(&self) -> f64 {
        self.max_lon - self.min_lon
    }

    /// Get the height in degrees.
    pub fn height(&self) -> f64 {
        self.max_lat - self.min_lat
    }

    /// Check if a point is contained within this bounding box.
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        lon >= self.min_lon && lon <= self.max_lon && lat >= self.min_lat && lat <= self.max_lat
    }

    /// Get the center point of the bounding box.
    pub fn center(&self) -> (f64, f64) {
        (
            (self.min_lon + self.max_lon) / 2.0,
            (self.min_lat + self.max_lat) / 2.0,
        )
    }

    /// Expand the bounding box by a buffer amount (in degrees).
    pub fn expand(&self, buffer: f64) -> Self {
        Self {
            min_lon: self.min_lon - buffer,
            min_lat: self.min_lat - buffer,
            max_lon: self.max_lon + buffer,
            max_lat: self.max_lat + buffer,
        }
    }

    /// Clamp this bounding box to valid geographic coordinates.
    pub fn clamp_to_valid(&self) -> Self {
        Self {
            min_lon: self.min_lon.max(-180.0).min(180.0),
            min_lat: self.min_lat.max(-90.0).min(90.0),
            max_lon: self.max_lon.max(-180.0).min(180.0),
            max_lat: self.max_lat.max(-90.0).min(90.0),
        }
    }

    /// Check if this bounding box uses 0-360 longitude convention.
    /// Returns true if the grid spans the 0-360 range (like GFS).
    pub fn uses_0_360_longitude(&self) -> bool {
        self.min_lon >= 0.0 && self.max_lon > 180.0
    }

    /// Check if this request bbox would cross the dateline when normalized to a 0-360 grid.
    /// This happens when the request spans from negative to positive longitude
    /// (e.g., min_lon=-100, max_lon=50 becomes 260 to 50, which is inverted).
    pub fn crosses_dateline_on_360_grid(&self, grid_bbox: &BoundingBox) -> bool {
        if !grid_bbox.uses_0_360_longitude() {
            return false;
        }
        // Request crosses dateline if min_lon is negative and max_lon is positive
        // After normalization, min_lon would be > max_lon
        self.min_lon < 0.0 && self.max_lon >= 0.0
    }

    /// Normalize a request bbox to match a grid's coordinate system.
    /// If the grid uses 0-360 longitude and the request uses -180/180,
    /// convert the request to 0-360.
    /// 
    /// NOTE: This does NOT handle cross-dateline requests properly.
    /// Use `crosses_dateline_on_360_grid()` to check first, and if true,
    /// either load the full grid or split into two requests.
    pub fn normalize_to_grid(&self, grid_bbox: &BoundingBox) -> Self {
        if grid_bbox.uses_0_360_longitude() && self.min_lon < 0.0 {
            // Grid uses 0-360, request uses -180/180
            // Convert request to 0-360 by adding 360 to negative longitudes
            Self {
                min_lon: if self.min_lon < 0.0 { self.min_lon + 360.0 } else { self.min_lon },
                min_lat: self.min_lat,
                max_lon: if self.max_lon < 0.0 { self.max_lon + 360.0 } else { self.max_lon },
                max_lat: self.max_lat,
            }
        } else {
            *self
        }
    }
}

impl Default for BoundingBox {
    fn default() -> Self {
        // Global coverage
        Self::new(-180.0, -90.0, 180.0, 90.0)
    }
}

/// Grid data for a specific region.
#[derive(Debug, Clone)]
pub struct GridRegion {
    /// The grid values (row-major order, top-to-bottom).
    pub data: Vec<f32>,
    /// Width of the region in grid points.
    pub width: usize,
    /// Height of the region in grid points.
    pub height: usize,
    /// Geographic bounds of this region.
    pub bbox: BoundingBox,
    /// Resolution in degrees per grid point (lon, lat).
    pub resolution: (f64, f64),
}

impl GridRegion {
    /// Create a new grid region.
    pub fn new(
        data: Vec<f32>,
        width: usize,
        height: usize,
        bbox: BoundingBox,
        resolution: (f64, f64),
    ) -> Self {
        Self {
            data,
            width,
            height,
            bbox,
            resolution,
        }
    }

    /// Get the value at a specific grid coordinate.
    pub fn get(&self, col: usize, row: usize) -> Option<f32> {
        if col >= self.width || row >= self.height {
            return None;
        }
        self.data.get(row * self.width + col).copied()
    }

    /// Get the value at a geographic coordinate using nearest neighbor.
    pub fn get_at_coords(&self, lon: f64, lat: f64) -> Option<f32> {
        if !self.bbox.contains(lon, lat) {
            return None;
        }

        let col = ((lon - self.bbox.min_lon) / self.resolution.0).floor() as usize;
        let row = ((self.bbox.max_lat - lat) / self.resolution.1).floor() as usize;

        self.get(col, row)
    }

    /// Get the total number of grid points.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if the region is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Metadata about a grid dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridMetadata {
    /// Model identifier (e.g., "gfs", "hrrr").
    pub model: String,
    /// Parameter name (e.g., "TMP").
    pub parameter: String,
    /// Level description (e.g., "2 m above ground").
    pub level: String,
    /// Physical units (e.g., "K").
    pub units: String,
    /// Reference time (model run time).
    pub reference_time: DateTime<Utc>,
    /// Forecast hour.
    pub forecast_hour: u32,
    /// Full grid bounding box.
    pub bbox: BoundingBox,
    /// Grid dimensions (width, height).
    pub shape: (usize, usize),
    /// Chunk dimensions.
    pub chunk_shape: (usize, usize),
    /// Number of chunks (x, y).
    pub num_chunks: (usize, usize),
    /// Fill/missing value.
    pub fill_value: f32,
}

impl GridMetadata {
    /// Calculate the grid resolution in degrees per point.
    pub fn resolution(&self) -> (f64, f64) {
        (
            self.bbox.width() / self.shape.0 as f64,
            self.bbox.height() / self.shape.1 as f64,
        )
    }

    /// Calculate how many chunks exist along each dimension.
    pub fn calculate_num_chunks(&self) -> (usize, usize) {
        let chunks_x = (self.shape.0 + self.chunk_shape.0 - 1) / self.chunk_shape.0;
        let chunks_y = (self.shape.1 + self.chunk_shape.1 - 1) / self.chunk_shape.1;
        (chunks_x, chunks_y)
    }

    /// Convert a grid cell index to geographic coordinates (center of cell).
    pub fn cell_to_coords(&self, col: usize, row: usize) -> (f64, f64) {
        let (res_x, res_y) = self.resolution();
        let lon = self.bbox.min_lon + (col as f64 + 0.5) * res_x;
        let lat = self.bbox.max_lat - (row as f64 + 0.5) * res_y;
        (lon, lat)
    }

    /// Convert geographic coordinates to grid cell indices.
    pub fn coords_to_cell(&self, lon: f64, lat: f64) -> Option<(usize, usize)> {
        if !self.bbox.contains(lon, lat) {
            return None;
        }

        let (res_x, res_y) = self.resolution();
        let col = ((lon - self.bbox.min_lon) / res_x).floor() as usize;
        let row = ((self.bbox.max_lat - lat) / res_y).floor() as usize;

        if col < self.shape.0 && row < self.shape.1 {
            Some((col, row))
        } else {
            None
        }
    }
}

/// Interpolation method for grid resampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InterpolationMethod {
    /// Nearest neighbor (preserves exact values).
    Nearest,
    /// Bilinear interpolation (smooth, slight value changes).
    #[default]
    Bilinear,
    /// Bicubic interpolation (smoothest, more compute).
    Cubic,
}

impl InterpolationMethod {
    /// Parse from string (case-insensitive).
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "nearest" => Self::Nearest,
            "cubic" | "bicubic" => Self::Cubic,
            _ => Self::Bilinear,
        }
    }
}

impl std::fmt::Display for InterpolationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nearest => write!(f, "nearest"),
            Self::Bilinear => write!(f, "bilinear"),
            Self::Cubic => write!(f, "cubic"),
        }
    }
}

/// Statistics about the chunk cache.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub memory_bytes: u64,
    pub evictions: u64,
}

impl CacheStats {
    /// Calculate the cache hit rate (0.0 - 1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbox_intersects() {
        let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
        let b = BoundingBox::new(5.0, 5.0, 15.0, 15.0);
        let c = BoundingBox::new(20.0, 20.0, 30.0, 30.0);

        assert!(a.intersects(&b));
        assert!(b.intersects(&a));
        assert!(!a.intersects(&c));
        assert!(!c.intersects(&a));
    }

    #[test]
    fn test_bbox_contains() {
        let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
        assert!(bbox.contains(-95.0, 35.0));
        assert!(!bbox.contains(-105.0, 35.0));
        assert!(!bbox.contains(-95.0, 45.0));
    }

    #[test]
    fn test_bbox_dimensions() {
        let bbox = BoundingBox::new(-100.0, 30.0, -90.0, 40.0);
        assert!((bbox.width() - 10.0).abs() < f64::EPSILON);
        assert!((bbox.height() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_grid_region_get() {
        let data: Vec<f32> = (0..9).map(|i| i as f32).collect();
        let region = GridRegion::new(
            data,
            3,
            3,
            BoundingBox::new(0.0, 0.0, 3.0, 3.0),
            (1.0, 1.0),
        );

        assert_eq!(region.get(0, 0), Some(0.0));
        assert_eq!(region.get(2, 2), Some(8.0));
        assert_eq!(region.get(1, 1), Some(4.0));
        assert_eq!(region.get(3, 0), None);
    }

    #[test]
    fn test_interpolation_method_from_str() {
        assert_eq!(
            InterpolationMethod::from_str("nearest"),
            InterpolationMethod::Nearest
        );
        assert_eq!(
            InterpolationMethod::from_str("BILINEAR"),
            InterpolationMethod::Bilinear
        );
        assert_eq!(
            InterpolationMethod::from_str("cubic"),
            InterpolationMethod::Cubic
        );
        assert_eq!(
            InterpolationMethod::from_str("bicubic"),
            InterpolationMethod::Cubic
        );
        assert_eq!(
            InterpolationMethod::from_str("invalid"),
            InterpolationMethod::Bilinear
        );
    }

    #[test]
    fn test_cache_stats_hit_rate() {
        let mut stats = CacheStats::default();
        assert!((stats.hit_rate() - 0.0).abs() < f64::EPSILON);

        stats.hits = 80;
        stats.misses = 20;
        assert!((stats.hit_rate() - 0.8).abs() < f64::EPSILON);
    }
}
