//! WMTS Tile Matrix and TileMatrixSet definitions.
//!
//! Implements OGC WMTS tile matrix concepts for tiled map services.

use crate::{BoundingBox, CrsCode};
use serde::{Deserialize, Serialize};

/// A tile coordinate (z/x/y).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileCoord {
    /// Zoom level (TileMatrix identifier)
    pub z: u32,
    /// Column (x)
    pub x: u32,
    /// Row (y)
    pub y: u32,
}

impl TileCoord {
    pub fn new(z: u32, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    /// Generate a cache key string.
    pub fn cache_key(&self) -> String {
        format!("{}/{}/{}", self.z, self.x, self.y)
    }

    /// Get the parent tile (zoom - 1).
    pub fn parent(&self) -> Option<TileCoord> {
        if self.z == 0 {
            return None;
        }
        Some(TileCoord {
            z: self.z - 1,
            x: self.x / 2,
            y: self.y / 2,
        })
    }

    /// Get the four children tiles (zoom + 1).
    pub fn children(&self) -> [TileCoord; 4] {
        let x = self.x * 2;
        let y = self.y * 2;
        let z = self.z + 1;
        [
            TileCoord { z, x, y },
            TileCoord { z, x: x + 1, y },
            TileCoord { z, x, y: y + 1 },
            TileCoord {
                z,
                x: x + 1,
                y: y + 1,
            },
        ]
    }
}

/// A single tile matrix (zoom level) definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileMatrix {
    /// Identifier (usually zoom level as string)
    pub identifier: String,

    /// Scale denominator
    pub scale_denominator: f64,

    /// Top-left corner coordinates
    pub top_left_corner: (f64, f64),

    /// Tile width in pixels
    pub tile_width: u32,

    /// Tile height in pixels
    pub tile_height: u32,

    /// Number of tile columns
    pub matrix_width: u32,

    /// Number of tile rows
    pub matrix_height: u32,
}

impl TileMatrix {
    /// Calculate the resolution (units per pixel) for this matrix.
    pub fn resolution(&self) -> f64 {
        // Standard pixel size is 0.28mm (OGC WMTS spec)
        self.scale_denominator * 0.00028
    }

    /// Get the bounding box for a specific tile.
    pub fn tile_bbox(&self, col: u32, row: u32) -> BoundingBox {
        let res = self.resolution();
        let tile_span_x = res * self.tile_width as f64;
        let tile_span_y = res * self.tile_height as f64;

        let min_x = self.top_left_corner.0 + col as f64 * tile_span_x;
        let max_y = self.top_left_corner.1 - row as f64 * tile_span_y;
        let max_x = min_x + tile_span_x;
        let min_y = max_y - tile_span_y;

        BoundingBox::new(min_x, min_y, max_x, max_y)
    }

    /// Find which tile contains a given coordinate.
    pub fn coord_to_tile(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        let res = self.resolution();
        let tile_span_x = res * self.tile_width as f64;
        let tile_span_y = res * self.tile_height as f64;

        let col = ((x - self.top_left_corner.0) / tile_span_x).floor() as i64;
        let row = ((self.top_left_corner.1 - y) / tile_span_y).floor() as i64;

        if col < 0 || row < 0 || col >= self.matrix_width as i64 || row >= self.matrix_height as i64
        {
            return None;
        }

        Some((col as u32, row as u32))
    }
}

/// A complete tile matrix set definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileMatrixSet {
    /// Identifier for the tile matrix set
    pub identifier: String,

    /// Coordinate reference system
    pub crs: CrsCode,

    /// Bounding box of the tile matrix set
    pub bounding_box: BoundingBox,

    /// Well-known scale set URI (optional)
    pub well_known_scale_set: Option<String>,

    /// Individual tile matrices (zoom levels)
    pub tile_matrices: Vec<TileMatrix>,
}

impl TileMatrixSet {
    /// Get a tile matrix by identifier (zoom level).
    pub fn get_matrix(&self, identifier: &str) -> Option<&TileMatrix> {
        self.tile_matrices
            .iter()
            .find(|m| m.identifier == identifier)
    }

    /// Get a tile matrix by zoom level number.
    pub fn get_matrix_by_zoom(&self, zoom: u32) -> Option<&TileMatrix> {
        self.tile_matrices.get(zoom as usize)
    }

    /// Get the bounding box for a tile.
    pub fn tile_bbox(&self, coord: &TileCoord) -> Option<BoundingBox> {
        self.get_matrix_by_zoom(coord.z)
            .map(|m| m.tile_bbox(coord.x, coord.y))
    }
}

/// Standard Web Mercator (Google/OSM) tile matrix set.
pub fn web_mercator_tile_matrix_set() -> TileMatrixSet {
    let max_extent = 20037508.342789244;

    let tile_matrices: Vec<TileMatrix> = (0..=22)
        .map(|z| {
            let n = 2u32.pow(z);
            let scale = 559082264.0287178 / (n as f64);

            TileMatrix {
                identifier: z.to_string(),
                scale_denominator: scale,
                top_left_corner: (-max_extent, max_extent),
                tile_width: 256,
                tile_height: 256,
                matrix_width: n,
                matrix_height: n,
            }
        })
        .collect();

    TileMatrixSet {
        identifier: "WebMercatorQuad".to_string(),
        crs: CrsCode::Epsg3857,
        bounding_box: BoundingBox::new(-max_extent, -max_extent, max_extent, max_extent),
        well_known_scale_set: Some(
            "http://www.opengis.net/def/wkss/OGC/1.0/GoogleMapsCompatible".to_string(),
        ),
        tile_matrices,
    }
}

/// Standard WGS84 (geographic) tile matrix set.
pub fn wgs84_tile_matrix_set() -> TileMatrixSet {
    let tile_matrices: Vec<TileMatrix> = (0..=22)
        .map(|z| {
            let n_cols = 2u32.pow(z + 1);
            let n_rows = 2u32.pow(z);
            let scale = 559082264.0287178 / (n_rows as f64);

            TileMatrix {
                identifier: z.to_string(),
                scale_denominator: scale,
                top_left_corner: (-180.0, 90.0),
                tile_width: 256,
                tile_height: 256,
                matrix_width: n_cols,
                matrix_height: n_rows,
            }
        })
        .collect();

    TileMatrixSet {
        identifier: "WorldCRS84Quad".to_string(),
        crs: CrsCode::Epsg4326,
        bounding_box: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
        well_known_scale_set: Some(
            "http://www.opengis.net/def/wkss/OGC/1.0/GoogleCRS84Quad".to_string(),
        ),
        tile_matrices,
    }
}

/// Convert lat/lon to Web Mercator tile coordinates.
pub fn latlon_to_tile(lat: f64, lon: f64, zoom: u32) -> TileCoord {
    let n = 2u32.pow(zoom) as f64;

    let x = ((lon + 180.0) / 360.0 * n).floor() as u32;
    let lat_rad = lat.to_radians();
    let y = ((1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor() as u32;

    TileCoord { z: zoom, x, y }
}

/// Convert Web Mercator tile coordinates to lat/lon bounds.
pub fn tile_to_latlon_bounds(coord: &TileCoord) -> BoundingBox {
    let n = 2u32.pow(coord.z) as f64;

    let lon_min = coord.x as f64 / n * 360.0 - 180.0;
    let lon_max = (coord.x + 1) as f64 / n * 360.0 - 180.0;

    let lat_max = (std::f64::consts::PI * (1.0 - 2.0 * coord.y as f64 / n))
        .sinh()
        .atan()
        .to_degrees();
    let lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (coord.y + 1) as f64 / n))
        .sinh()
        .atan()
        .to_degrees();

    BoundingBox::new(lon_min, lat_min, lon_max, lat_max)
}

/// Convert WorldCRS84Quad tile coordinates to lat/lon bounds.
///
/// WorldCRS84Quad uses a 2:1 aspect ratio grid:
/// - matrix_width = 2^(z+1) columns
/// - matrix_height = 2^z rows
/// - Linear latitude/longitude mapping (no Mercator projection)
/// - Top-left origin at (-180, 90)
pub fn wgs84_tile_to_latlon_bounds(coord: &TileCoord) -> BoundingBox {
    let n_cols = 2u32.pow(coord.z + 1) as f64; // 2^(z+1) columns
    let n_rows = 2u32.pow(coord.z) as f64; // 2^z rows

    // Longitude spans 360 degrees across all columns
    let lon_min = (coord.x as f64 / n_cols) * 360.0 - 180.0;
    let lon_max = ((coord.x + 1) as f64 / n_cols) * 360.0 - 180.0;

    // Latitude spans 180 degrees across all rows (top-left origin)
    // lat decreases as row index increases
    let lat_max = 90.0 - (coord.y as f64 / n_rows) * 180.0;
    let lat_min = 90.0 - ((coord.y + 1) as f64 / n_rows) * 180.0;

    BoundingBox::new(lon_min, lat_min, lon_max, lat_max)
}

/// TMS (Tile Map Service) Y-flip conversion.
/// TMS uses bottom-left origin, while XYZ/WMTS uses top-left.
pub fn tms_to_xyz(z: u32, x: u32, y: u32) -> TileCoord {
    let n = 2u32.pow(z);
    TileCoord { z, x, y: n - 1 - y }
}

pub fn xyz_to_tms(coord: &TileCoord) -> (u32, u32, u32) {
    let n = 2u32.pow(coord.z);
    (coord.z, coord.x, n - 1 - coord.y)
}

/// Configuration for expanded tile rendering using full tile expansion.
/// Used to render a larger area and crop to get seamless tile boundaries.
///
/// **DEPRECATED**: Prefer `TileBufferConfig` for better performance.
/// The 3x3 expansion renders 9x the pixels but only uses the center tile.
/// A 60px buffer achieves similar results at ~4x less cost.
#[derive(Debug, Clone, Copy)]
pub struct ExpandedTileConfig {
    /// Number of tiles to expand in each direction (1 = 3x3 grid, 2 = 5x5 grid)
    pub expansion: u32,
    /// Tile size in pixels
    pub tile_size: u32,
}

impl Default for ExpandedTileConfig {
    fn default() -> Self {
        Self {
            expansion: 1, // 3x3 grid
            tile_size: 256,
        }
    }
}

impl ExpandedTileConfig {
    /// Create config for 3x3 tile rendering
    #[deprecated(
        since = "0.2.0",
        note = "Use TileBufferConfig::default() instead for 4x better performance"
    )]
    pub fn tiles_3x3() -> Self {
        Self {
            expansion: 1,
            tile_size: 256,
        }
    }

    /// Get the total size of the expanded render area in pixels
    pub fn expanded_size(&self) -> u32 {
        self.tile_size * (2 * self.expansion + 1)
    }

    /// Get the pixel offset where the center tile starts
    pub fn center_offset(&self) -> u32 {
        self.tile_size * self.expansion
    }
}

// =============================================================================
// TileBufferConfig - Efficient pixel-based buffer for tile edge handling
// =============================================================================

/// Configuration for rendering tiles with a pixel buffer margin.
///
/// The buffer allows features like wind barbs and numbers to have context
/// beyond the tile edge, preventing clipping artifacts at tile boundaries.
///
/// This is **~2.4x more efficient** than the 3x3 tile expansion approach:
/// - 3x3 expansion: renders 768x768 (589,824 pixels) for a 256x256 tile
/// - 120px buffer: renders 496x496 (246,016 pixels) for a 256x256 tile
///
/// # Example
/// ```
/// use wms_common::tile::TileBufferConfig;
///
/// // Default 120px buffer (good for 108px wind barbs)
/// let config = TileBufferConfig::default();
/// assert_eq!(config.render_width(), 496);
///
/// // Custom buffer size
/// let config = TileBufferConfig::new(80, 256);
/// assert_eq!(config.render_width(), 416);
///
/// // No buffer (for gradient rendering that doesn't need it)
/// let config = TileBufferConfig::no_buffer();
/// assert_eq!(config.render_width(), 256);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct TileBufferConfig {
    /// Buffer size in pixels on each side of the tile
    pub buffer_pixels: u32,
    /// Base tile size (typically 256)
    pub tile_size: u32,
}

impl Default for TileBufferConfig {
    fn default() -> Self {
        // 120px buffer ensures no clipping for 108px barbs even when
        // barb center is at tile edge and barb points away from tile.
        // Still provides ~2.4x speedup over 3x3 expansion (496² vs 768²)
        Self {
            buffer_pixels: 120,
            tile_size: 256,
        }
    }
}

impl TileBufferConfig {
    /// Create a new buffer configuration.
    ///
    /// # Arguments
    /// * `buffer_pixels` - Buffer size in pixels on each side
    /// * `tile_size` - Base tile size (typically 256)
    pub fn new(buffer_pixels: u32, tile_size: u32) -> Self {
        Self {
            buffer_pixels,
            tile_size,
        }
    }

    /// Create from environment variable (TILE_RENDER_BUFFER_PIXELS).
    /// Falls back to 120px if not set.
    pub fn from_env() -> Self {
        let buffer = std::env::var("TILE_RENDER_BUFFER_PIXELS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);

        let tile_size = std::env::var("TILE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(256);

        Self {
            buffer_pixels: buffer,
            tile_size,
        }
    }

    /// No buffer (for gradient/raster rendering that doesn't need it).
    pub fn no_buffer() -> Self {
        Self {
            buffer_pixels: 0,
            tile_size: 256,
        }
    }

    /// Total render width including buffer on both sides.
    pub fn render_width(&self) -> u32 {
        self.tile_size + 2 * self.buffer_pixels
    }

    /// Total render height including buffer on both sides.
    pub fn render_height(&self) -> u32 {
        self.tile_size + 2 * self.buffer_pixels
    }

    /// Calculate the expanded bounding box for a tile.
    ///
    /// # Arguments
    /// * `tile_bbox` - The original tile bounding box
    ///
    /// # Returns
    /// Expanded bounding box with buffer margin added
    pub fn expanded_bbox(&self, tile_bbox: &BoundingBox) -> BoundingBox {
        if self.buffer_pixels == 0 {
            return tile_bbox.clone();
        }

        // Calculate degrees per pixel for this tile
        let tile_width_deg = tile_bbox.max_x - tile_bbox.min_x;
        let tile_height_deg = tile_bbox.max_y - tile_bbox.min_y;
        let deg_per_pixel_lon = tile_width_deg / self.tile_size as f64;
        let deg_per_pixel_lat = tile_height_deg / self.tile_size as f64;

        // Expand by buffer amount
        let buffer_lon = self.buffer_pixels as f64 * deg_per_pixel_lon;
        let buffer_lat = self.buffer_pixels as f64 * deg_per_pixel_lat;

        BoundingBox::new(
            tile_bbox.min_x - buffer_lon,
            tile_bbox.min_y - buffer_lat,
            tile_bbox.max_x + buffer_lon,
            tile_bbox.max_y + buffer_lat,
        )
    }

    /// Crop the center tile from an expanded RGBA pixel buffer.
    ///
    /// # Arguments
    /// * `expanded_pixels` - The expanded render (RGBA, 4 bytes per pixel)
    ///
    /// # Returns
    /// The cropped center tile pixels (RGBA)
    pub fn crop_to_tile(&self, expanded_pixels: &[u8]) -> Vec<u8> {
        if self.buffer_pixels == 0 {
            return expanded_pixels.to_vec();
        }

        let render_width = self.render_width() as usize;
        let tile_size = self.tile_size as usize;
        let buffer = self.buffer_pixels as usize;

        let mut result = vec![0u8; tile_size * tile_size * 4];

        for row in 0..tile_size {
            let src_y = buffer + row;
            let src_start = (src_y * render_width + buffer) * 4;
            let src_end = src_start + tile_size * 4;

            let dst_start = row * tile_size * 4;
            let dst_end = dst_start + tile_size * 4;

            if src_end <= expanded_pixels.len() {
                result[dst_start..dst_end].copy_from_slice(&expanded_pixels[src_start..src_end]);
            }
        }

        result
    }
}

/// Calculate the bounding box for a single tile in WGS84 coordinates.
///
/// # Arguments
/// * `coord` - The tile coordinate
///
/// # Returns
/// The bounding box in WGS84 coordinates (lat/lon)
pub fn tile_bbox(coord: &TileCoord) -> BoundingBox {
    let n = 2u32.pow(coord.z) as f64;

    let lon_min = coord.x as f64 / n * 360.0 - 180.0;
    let lon_max = (coord.x + 1) as f64 / n * 360.0 - 180.0;

    let lat_max = (std::f64::consts::PI * (1.0 - 2.0 * coord.y as f64 / n))
        .sinh()
        .atan()
        .to_degrees();
    let lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (coord.y + 1) as f64 / n))
        .sinh()
        .atan()
        .to_degrees();

    BoundingBox::new(lon_min, lat_min, lon_max, lat_max)
}

/// Calculate the expanded bounding box for rendering a tile with its neighbors.
/// This allows rendering features that cross tile boundaries correctly.
///
/// # Arguments
/// * `coord` - The center tile coordinate
/// * `config` - Expansion configuration
///
/// # Returns
/// The expanded bounding box in WGS84 coordinates (lat/lon)
pub fn expanded_tile_bbox(coord: &TileCoord, config: &ExpandedTileConfig) -> BoundingBox {
    let n = 2u32.pow(coord.z) as f64;
    let expansion = config.expansion;

    // Calculate expanded tile range, clamping to valid tile indices
    let x_min = coord.x.saturating_sub(expansion);
    let x_max = (coord.x + expansion + 1).min(2u32.pow(coord.z));
    let y_min = coord.y.saturating_sub(expansion);
    let y_max = (coord.y + expansion + 1).min(2u32.pow(coord.z));

    // Convert to lat/lon bounds
    let lon_min = x_min as f64 / n * 360.0 - 180.0;
    let lon_max = x_max as f64 / n * 360.0 - 180.0;

    let lat_max = (std::f64::consts::PI * (1.0 - 2.0 * y_min as f64 / n))
        .sinh()
        .atan()
        .to_degrees();
    let lat_min = (std::f64::consts::PI * (1.0 - 2.0 * y_max as f64 / n))
        .sinh()
        .atan()
        .to_degrees();

    BoundingBox::new(lon_min, lat_min, lon_max, lat_max)
}

/// Calculate the crop region within an expanded render to extract the center tile.
///
/// # Arguments
/// * `coord` - The center tile coordinate
/// * `config` - Expansion configuration
///
/// # Returns
/// (x_offset, y_offset, width, height) - The crop region in pixels
pub fn center_tile_crop_region(
    coord: &TileCoord,
    config: &ExpandedTileConfig,
) -> (u32, u32, u32, u32) {
    let expansion = config.expansion;
    let tile_size = config.tile_size;

    // Calculate actual expansion used (may be less at edges)
    let actual_x_before = coord.x.min(expansion);
    let actual_y_before = coord.y.min(expansion);

    // The center tile starts after the tiles before it
    let x_offset = actual_x_before * tile_size;
    let y_offset = actual_y_before * tile_size;

    (x_offset, y_offset, tile_size, tile_size)
}

/// Get the actual expanded dimensions when near tile grid edges.
///
/// # Arguments
/// * `coord` - The center tile coordinate
/// * `config` - Expansion configuration
///
/// # Returns
/// (width, height) in pixels of the expanded render area
pub fn actual_expanded_dimensions(coord: &TileCoord, config: &ExpandedTileConfig) -> (u32, u32) {
    let expansion = config.expansion;
    let tile_size = config.tile_size;
    let n = 2u32.pow(coord.z);

    // Calculate how many tiles we can actually expand to
    let tiles_left = coord.x.min(expansion);
    let tiles_right = (n - 1 - coord.x).min(expansion);
    let tiles_up = coord.y.min(expansion);
    let tiles_down = (n - 1 - coord.y).min(expansion);

    let width = (1 + tiles_left + tiles_right) * tile_size;
    let height = (1 + tiles_up + tiles_down) * tile_size;

    (width, height)
}

/// Crop the center tile from an expanded RGBA pixel buffer.
///
/// # Arguments
/// * `expanded_pixels` - The full expanded render (RGBA, 4 bytes per pixel)
/// * `expanded_width` - Width of the expanded render
/// * `coord` - The center tile coordinate
/// * `config` - Expansion configuration
///
/// # Returns
/// The cropped center tile pixels (RGBA)
pub fn crop_center_tile(
    expanded_pixels: &[u8],
    expanded_width: u32,
    coord: &TileCoord,
    config: &ExpandedTileConfig,
) -> Vec<u8> {
    let (x_offset, y_offset, _width, _height) = center_tile_crop_region(coord, config);
    let tile_size = config.tile_size as usize;

    let mut result = vec![0u8; tile_size * tile_size * 4];

    for row in 0..tile_size {
        let src_y = y_offset as usize + row;
        let src_start = (src_y * expanded_width as usize + x_offset as usize) * 4;
        let src_end = src_start + tile_size * 4;

        let dst_start = row * tile_size * 4;
        let dst_end = dst_start + tile_size * 4;

        if src_end <= expanded_pixels.len() {
            result[dst_start..dst_end].copy_from_slice(&expanded_pixels[src_start..src_end]);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // TileBufferConfig Tests
    // =========================================================================

    #[test]
    fn test_tile_buffer_config_default() {
        let config = TileBufferConfig::default();
        assert_eq!(config.buffer_pixels, 120);
        assert_eq!(config.tile_size, 256);
        assert_eq!(config.render_width(), 496);
        assert_eq!(config.render_height(), 496);
    }

    #[test]
    fn test_tile_buffer_config_no_buffer() {
        let config = TileBufferConfig::no_buffer();
        assert_eq!(config.buffer_pixels, 0);
        assert_eq!(config.render_width(), 256);
    }

    #[test]
    fn test_tile_buffer_config_expanded_bbox() {
        let config = TileBufferConfig::new(50, 256);
        let tile_bbox = BoundingBox::new(-100.0, 35.0, -99.0, 36.0);

        let expanded = config.expanded_bbox(&tile_bbox);

        // Buffer should add ~0.195° on each side (50/256 * 1°)
        let expected_buffer = 50.0 / 256.0 * 1.0;
        assert!((expanded.min_x - (-100.0 - expected_buffer)).abs() < 0.001);
        assert!((expanded.max_x - (-99.0 + expected_buffer)).abs() < 0.001);
        assert!((expanded.min_y - (35.0 - expected_buffer)).abs() < 0.001);
        assert!((expanded.max_y - (36.0 + expected_buffer)).abs() < 0.001);
    }

    #[test]
    fn test_tile_buffer_config_no_buffer_bbox() {
        let config = TileBufferConfig::no_buffer();
        let tile_bbox = BoundingBox::new(-100.0, 35.0, -99.0, 36.0);

        let expanded = config.expanded_bbox(&tile_bbox);

        assert_eq!(expanded.min_x, tile_bbox.min_x);
        assert_eq!(expanded.max_x, tile_bbox.max_x);
        assert_eq!(expanded.min_y, tile_bbox.min_y);
        assert_eq!(expanded.max_y, tile_bbox.max_y);
    }

    #[test]
    fn test_tile_buffer_config_crop() {
        let config = TileBufferConfig::new(50, 256);

        // Create test pattern: expanded area with distinct border
        let render_w = config.render_width() as usize; // 356
        let render_h = config.render_height() as usize; // 356
        let mut expanded = vec![0u8; render_w * render_h * 4];

        // Fill center (tile area) with white
        for y in 50..(50 + 256) {
            for x in 50..(50 + 256) {
                let idx = (y * render_w + x) * 4;
                expanded[idx] = 255; // R
                expanded[idx + 1] = 255; // G
                expanded[idx + 2] = 255; // B
                expanded[idx + 3] = 255; // A
            }
        }

        // Crop
        let cropped = config.crop_to_tile(&expanded);

        // Verify correct size
        assert_eq!(cropped.len(), 256 * 256 * 4);

        // Verify all pixels are white
        for chunk in cropped.chunks(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn test_tile_buffer_config_crop_no_buffer() {
        let config = TileBufferConfig::no_buffer();
        let original = vec![128u8; 256 * 256 * 4];

        let cropped = config.crop_to_tile(&original);

        assert_eq!(cropped.len(), original.len());
        assert_eq!(cropped, original);
    }

    // =========================================================================
    // Original Tests
    // =========================================================================

    #[test]
    fn test_latlon_to_tile() {
        // Test known coordinates
        let coord = latlon_to_tile(0.0, 0.0, 0);
        assert_eq!(coord, TileCoord { z: 0, x: 0, y: 0 });

        let coord = latlon_to_tile(40.7128, -74.0060, 10); // NYC
        assert_eq!(coord.z, 10);
        // x should be around 301, y around 384
        assert!(coord.x > 290 && coord.x < 310);
        assert!(coord.y > 370 && coord.y < 400);
    }

    #[test]
    fn test_tile_bbox() {
        let tms = web_mercator_tile_matrix_set();
        let bbox = tms.tile_bbox(&TileCoord { z: 0, x: 0, y: 0 }).unwrap();

        // Zoom 0 should cover entire extent
        let max_extent = 20037508.342789244;
        assert!((bbox.min_x - (-max_extent)).abs() < 1.0);
        assert!((bbox.max_x - max_extent).abs() < 1.0);
    }

    #[test]
    fn test_parent_children() {
        let tile = TileCoord { z: 5, x: 10, y: 15 };
        let parent = tile.parent().unwrap();
        assert_eq!(parent, TileCoord { z: 4, x: 5, y: 7 });

        let children = parent.children();
        assert!(children.contains(&tile));
    }

    #[test]
    fn test_tms_xyz_conversion() {
        let xyz = TileCoord { z: 3, x: 4, y: 2 };
        let (z, x, y) = xyz_to_tms(&xyz);
        let back = tms_to_xyz(z, x, y);
        assert_eq!(xyz, back);
    }

    #[test]
    fn test_wgs84_tile_to_latlon_bounds() {
        // At zoom 0: 2 columns, 1 row
        // Tile (0,0,0) should cover western hemisphere: -180 to 0 lon, -90 to 90 lat
        let bbox = wgs84_tile_to_latlon_bounds(&TileCoord { z: 0, x: 0, y: 0 });
        assert!((bbox.min_x - (-180.0)).abs() < 0.001);
        assert!((bbox.max_x - 0.0).abs() < 0.001);
        assert!((bbox.min_y - (-90.0)).abs() < 0.001);
        assert!((bbox.max_y - 90.0).abs() < 0.001);

        // Tile (0,1,0) should cover eastern hemisphere: 0 to 180 lon, -90 to 90 lat
        let bbox = wgs84_tile_to_latlon_bounds(&TileCoord { z: 0, x: 1, y: 0 });
        assert!((bbox.min_x - 0.0).abs() < 0.001);
        assert!((bbox.max_x - 180.0).abs() < 0.001);
        assert!((bbox.min_y - (-90.0)).abs() < 0.001);
        assert!((bbox.max_y - 90.0).abs() < 0.001);

        // At zoom 1: 4 columns, 2 rows
        // Tile (1,0,0) should cover: -180 to -90 lon, 0 to 90 lat
        let bbox = wgs84_tile_to_latlon_bounds(&TileCoord { z: 1, x: 0, y: 0 });
        assert!((bbox.min_x - (-180.0)).abs() < 0.001);
        assert!((bbox.max_x - (-90.0)).abs() < 0.001);
        assert!((bbox.min_y - 0.0).abs() < 0.001);
        assert!((bbox.max_y - 90.0).abs() < 0.001);

        // Tile (1,2,1) should cover: -90 to 0 lon, -90 to 0 lat
        let bbox = wgs84_tile_to_latlon_bounds(&TileCoord { z: 1, x: 1, y: 1 });
        assert!((bbox.min_x - (-90.0)).abs() < 0.001);
        assert!((bbox.max_x - 0.0).abs() < 0.001);
        assert!((bbox.min_y - (-90.0)).abs() < 0.001);
        assert!((bbox.max_y - 0.0).abs() < 0.001);
    }
}
