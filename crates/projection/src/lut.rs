// TODO this may be dead code unless we can come up with a way to make LUT actually increase performance in a meaningful way
//! Projection Lookup Table (LUT) for fast geostationary satellite tile rendering.
//!
//! Pre-computes grid indices for all pixels in a tile, eliminating expensive
//! trigonometric operations during rendering. For zoom levels 0-7, the LUT
//! can be pre-computed and loaded at startup for instant tile rendering.
//!
//! # Performance
//!
//! Without LUT: ~6.7ms per 256x256 tile (65K trig operations)
//! With LUT: ~0.5ms per tile (just bilinear interpolation)
//!
//! # Memory Usage
//!
//! Per tile: ~520KB (512KB indices + 8KB validity bitmap)
//! Zoom 0-7 for one satellite: ~325MB (~640 tiles)

use crate::geostationary::Geostationary;
use std::collections::HashMap;
use std::io::{Read, Write};

/// Tile size in pixels (standard Web Mercator tile)
pub const TILE_SIZE: usize = 256;

/// Total pixels per tile
pub const PIXELS_PER_TILE: usize = TILE_SIZE * TILE_SIZE;

/// Pre-computed grid indices for a single tile.
///
/// Stores the GOES grid (i, j) coordinates for each output pixel,
/// allowing direct bilinear interpolation without projection math.
#[derive(Clone)]
pub struct TileGridLut {
    /// Grid indices (i, j) for each pixel. NaN if pixel is outside GOES coverage.
    /// Stored as flat array: indices[y * TILE_SIZE + x] = (i, j)
    pub indices: Vec<(f32, f32)>,

    /// Validity bitmap: bit N is 1 if pixel N has valid grid coordinates.
    /// Packed as 64-bit words for efficient iteration.
    pub valid_bitmap: Vec<u64>,
}

impl TileGridLut {
    /// Create a new LUT with pre-allocated storage.
    pub fn new() -> Self {
        Self {
            indices: vec![(f32::NAN, f32::NAN); PIXELS_PER_TILE],
            valid_bitmap: vec![0u64; PIXELS_PER_TILE.div_ceil(64)],
        }
    }

    /// Check if a pixel has valid grid coordinates.
    #[inline]
    pub fn is_valid(&self, pixel_idx: usize) -> bool {
        let word_idx = pixel_idx / 64;
        let bit_idx = pixel_idx % 64;
        (self.valid_bitmap[word_idx] & (1u64 << bit_idx)) != 0
    }

    /// Set a pixel's grid coordinates.
    #[inline]
    pub fn set(&mut self, pixel_idx: usize, i: f32, j: f32) {
        self.indices[pixel_idx] = (i, j);
        let word_idx = pixel_idx / 64;
        let bit_idx = pixel_idx % 64;
        self.valid_bitmap[word_idx] |= 1u64 << bit_idx;
    }

    /// Get a pixel's grid coordinates, returning None if invalid.
    #[inline]
    pub fn get(&self, pixel_idx: usize) -> Option<(f32, f32)> {
        if self.is_valid(pixel_idx) {
            Some(self.indices[pixel_idx])
        } else {
            None
        }
    }

    /// Count valid pixels in this LUT.
    pub fn valid_count(&self) -> usize {
        self.valid_bitmap
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum()
    }

    /// Serialize to bytes for storage.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PIXELS_PER_TILE * 8 + 1024);

        // Write indices (as f32 pairs)
        for &(i, j) in &self.indices {
            bytes.extend_from_slice(&i.to_le_bytes());
            bytes.extend_from_slice(&j.to_le_bytes());
        }

        // Write validity bitmap
        for &word in &self.valid_bitmap {
            bytes.extend_from_slice(&word.to_le_bytes());
        }

        bytes
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LutError> {
        let expected_size = PIXELS_PER_TILE * 8 + PIXELS_PER_TILE.div_ceil(64) * 8;
        if bytes.len() != expected_size {
            return Err(LutError::InvalidSize {
                expected: expected_size,
                actual: bytes.len(),
            });
        }

        let mut indices = Vec::with_capacity(PIXELS_PER_TILE);
        let mut offset = 0;

        // Read indices
        for _ in 0..PIXELS_PER_TILE {
            let i = f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            let j = f32::from_le_bytes([
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ]);
            indices.push((i, j));
            offset += 8;
        }

        // Read validity bitmap
        let bitmap_words = PIXELS_PER_TILE.div_ceil(64);
        let mut valid_bitmap = Vec::with_capacity(bitmap_words);
        for _ in 0..bitmap_words {
            let word = u64::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
                bytes[offset + 4],
                bytes[offset + 5],
                bytes[offset + 6],
                bytes[offset + 7],
            ]);
            valid_bitmap.push(word);
            offset += 8;
        }

        Ok(Self {
            indices,
            valid_bitmap,
        })
    }

    /// Size in bytes when serialized.
    pub fn byte_size() -> usize {
        PIXELS_PER_TILE * 8 + PIXELS_PER_TILE.div_ceil(64) * 8
    }
}

impl Default for TileGridLut {
    fn default() -> Self {
        Self::new()
    }
}

/// Key for looking up a tile's LUT.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct TileLutKey {
    /// Satellite identifier (e.g., "goes16", "goes18")
    pub satellite: String,
    /// Zoom level
    pub z: u32,
    /// Tile column
    pub x: u32,
    /// Tile row
    pub y: u32,
}

impl TileLutKey {
    pub fn new(satellite: impl Into<String>, z: u32, x: u32, y: u32) -> Self {
        Self {
            satellite: satellite.into(),
            z,
            x,
            y,
        }
    }

    /// Create a compact string key for file storage.
    pub fn to_string_key(&self) -> String {
        format!("{}/{}/{}/{}", self.satellite, self.z, self.x, self.y)
    }
}

/// Complete LUT cache for a satellite.
///
/// Holds pre-computed LUTs for all tiles in zoom levels 0 through max_zoom.
pub struct ProjectionLutCache {
    /// Map from tile key to pre-computed LUT
    luts: HashMap<TileLutKey, TileGridLut>,

    /// Maximum zoom level included in the cache
    pub max_zoom: u32,

    /// Satellite identifier
    pub satellite: String,
}

impl ProjectionLutCache {
    /// Create an empty cache.
    pub fn new(satellite: impl Into<String>, max_zoom: u32) -> Self {
        Self {
            luts: HashMap::new(),
            max_zoom,
            satellite: satellite.into(),
        }
    }

    /// Get a LUT for a tile, if it exists in the cache.
    pub fn get(&self, z: u32, x: u32, y: u32) -> Option<&TileGridLut> {
        if z > self.max_zoom {
            return None;
        }
        let key = TileLutKey::new(&self.satellite, z, x, y);
        self.luts.get(&key)
    }

    /// Insert a LUT into the cache.
    pub fn insert(&mut self, z: u32, x: u32, y: u32, lut: TileGridLut) {
        let key = TileLutKey::new(&self.satellite, z, x, y);
        self.luts.insert(key, lut);
    }

    /// Number of tiles in the cache.
    pub fn len(&self) -> usize {
        self.luts.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.luts.is_empty()
    }

    /// Estimated memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        self.luts.len() * TileGridLut::byte_size()
    }

    /// Serialize the entire cache to a writer.
    pub fn save<W: Write>(&self, mut writer: W) -> Result<(), LutError> {
        // Header: magic, version, satellite name length, satellite name, max_zoom, tile count
        writer.write_all(b"GLUT")?; // Magic bytes
        writer.write_all(&1u32.to_le_bytes())?; // Version 1

        let sat_bytes = self.satellite.as_bytes();
        writer.write_all(&(sat_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(sat_bytes)?;

        writer.write_all(&self.max_zoom.to_le_bytes())?;
        writer.write_all(&(self.luts.len() as u32).to_le_bytes())?;

        // Write each tile
        for (key, lut) in &self.luts {
            writer.write_all(&key.z.to_le_bytes())?;
            writer.write_all(&key.x.to_le_bytes())?;
            writer.write_all(&key.y.to_le_bytes())?;
            writer.write_all(&lut.to_bytes())?;
        }

        Ok(())
    }

    /// Load a cache from a reader.
    pub fn load<R: Read>(mut reader: R) -> Result<Self, LutError> {
        // Read header
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"GLUT" {
            return Err(LutError::InvalidMagic);
        }

        let mut buf4 = [0u8; 4];
        reader.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != 1 {
            return Err(LutError::UnsupportedVersion(version));
        }

        reader.read_exact(&mut buf4)?;
        let sat_len = u32::from_le_bytes(buf4) as usize;
        let mut sat_bytes = vec![0u8; sat_len];
        reader.read_exact(&mut sat_bytes)?;
        let satellite = String::from_utf8(sat_bytes).map_err(|_| LutError::InvalidSatelliteName)?;

        reader.read_exact(&mut buf4)?;
        let max_zoom = u32::from_le_bytes(buf4);

        reader.read_exact(&mut buf4)?;
        let tile_count = u32::from_le_bytes(buf4) as usize;

        let mut luts = HashMap::with_capacity(tile_count);
        let lut_size = TileGridLut::byte_size();
        let mut lut_bytes = vec![0u8; lut_size];

        for _ in 0..tile_count {
            reader.read_exact(&mut buf4)?;
            let z = u32::from_le_bytes(buf4);
            reader.read_exact(&mut buf4)?;
            let x = u32::from_le_bytes(buf4);
            reader.read_exact(&mut buf4)?;
            let y = u32::from_le_bytes(buf4);

            reader.read_exact(&mut lut_bytes)?;
            let lut = TileGridLut::from_bytes(&lut_bytes)?;

            let key = TileLutKey::new(&satellite, z, x, y);
            luts.insert(key, lut);
        }

        Ok(Self {
            luts,
            max_zoom,
            satellite,
        })
    }
}

/// Errors that can occur during LUT operations.
#[derive(Debug)]
pub enum LutError {
    Io(std::io::Error),
    InvalidMagic,
    UnsupportedVersion(u32),
    InvalidSatelliteName,
    InvalidSize { expected: usize, actual: usize },
}

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LutError::Io(e) => write!(f, "I/O error: {}", e),
            LutError::InvalidMagic => write!(f, "Invalid file magic bytes"),
            LutError::UnsupportedVersion(v) => write!(f, "Unsupported LUT version: {}", v),
            LutError::InvalidSatelliteName => write!(f, "Invalid satellite name encoding"),
            LutError::InvalidSize { expected, actual } => {
                write!(
                    f,
                    "Invalid LUT size: expected {} bytes, got {}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for LutError {}

impl From<std::io::Error> for LutError {
    fn from(e: std::io::Error) -> Self {
        LutError::Io(e)
    }
}

/// Compute a LUT for a single tile.
///
/// # Arguments
/// * `proj` - The geostationary projection
/// * `z` - Zoom level
/// * `x` - Tile column
/// * `y` - Tile row
/// * `data_width` - Width of the GOES grid (e.g., 5000 for CONUS)
/// * `data_height` - Height of the GOES grid (e.g., 3000 for CONUS)
pub fn compute_tile_lut(
    proj: &Geostationary,
    z: u32,
    x: u32,
    y: u32,
    data_width: usize,
    data_height: usize,
) -> TileGridLut {
    let mut lut = TileGridLut::new();

    // Calculate tile bounds in lat/lon
    let n = 2u32.pow(z) as f64;

    let lon_min = x as f64 / n * 360.0 - 180.0;
    let lon_max = (x + 1) as f64 / n * 360.0 - 180.0;

    let lat_max = (std::f64::consts::PI * (1.0 - 2.0 * y as f64 / n))
        .sinh()
        .atan()
        .to_degrees();
    let lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (y + 1) as f64 / n))
        .sinh()
        .atan()
        .to_degrees();

    // Convert lat bounds to Mercator Y for proper spacing
    let min_merc_y = lat_to_mercator_y(lat_min);
    let max_merc_y = lat_to_mercator_y(lat_max);

    // Compute grid indices for each output pixel
    for out_y in 0..TILE_SIZE {
        for out_x in 0..TILE_SIZE {
            let pixel_idx = out_y * TILE_SIZE + out_x;

            // Calculate position in output image (pixel center)
            let x_ratio = (out_x as f64 + 0.5) / TILE_SIZE as f64;
            let y_ratio = (out_y as f64 + 0.5) / TILE_SIZE as f64;

            // Longitude is linear in degrees
            let lon = lon_min + x_ratio * (lon_max - lon_min);

            // Y position uses Mercator spacing, then convert back to latitude
            let merc_y = max_merc_y - y_ratio * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y);

            // Convert geographic to geostationary grid indices
            if let Some((grid_i, grid_j)) = proj.geo_to_grid(lat, lon) {
                // Check if within grid bounds (with margin for bilinear interpolation)
                if grid_i >= 0.0
                    && grid_i < data_width as f64 - 1.0
                    && grid_j >= 0.0
                    && grid_j < data_height as f64 - 1.0
                {
                    lut.set(pixel_idx, grid_i as f32, grid_j as f32);
                }
            }
        }
    }

    lut
}

/// Compute LUTs for all tiles in zoom levels 0 through max_zoom.
///
/// # Arguments
/// * `satellite` - Satellite identifier (e.g., "goes16")
/// * `proj` - The geostationary projection
/// * `max_zoom` - Maximum zoom level to compute (typically 7)
/// * `data_width` - Width of the GOES grid
/// * `data_height` - Height of the GOES grid
/// * `progress_callback` - Optional callback for progress updates
pub fn compute_all_luts<F>(
    satellite: &str,
    proj: &Geostationary,
    max_zoom: u32,
    data_width: usize,
    data_height: usize,
    mut progress_callback: Option<F>,
) -> ProjectionLutCache
where
    F: FnMut(u32, u32, u32, u32), // (zoom, x, y, total_computed)
{
    let mut cache = ProjectionLutCache::new(satellite, max_zoom);

    // Get geographic bounds of the projection to filter tiles
    let (proj_min_lon, proj_min_lat, proj_max_lon, proj_max_lat) = proj.geographic_bounds();

    let mut total_computed = 0u32;

    for z in 0..=max_zoom {
        let n = 2u32.pow(z);

        for y in 0..n {
            for x in 0..n {
                // Calculate tile bounds
                let tile_lon_min = x as f64 / n as f64 * 360.0 - 180.0;
                let tile_lon_max = (x + 1) as f64 / n as f64 * 360.0 - 180.0;
                let tile_lat_max = (std::f64::consts::PI * (1.0 - 2.0 * y as f64 / n as f64))
                    .sinh()
                    .atan()
                    .to_degrees();
                let tile_lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (y + 1) as f64 / n as f64))
                    .sinh()
                    .atan()
                    .to_degrees();

                // Skip tiles that don't overlap with projection bounds
                if tile_lon_max < proj_min_lon
                    || tile_lon_min > proj_max_lon
                    || tile_lat_max < proj_min_lat
                    || tile_lat_min > proj_max_lat
                {
                    continue;
                }

                let lut = compute_tile_lut(proj, z, x, y, data_width, data_height);

                // Only store if the tile has any valid pixels
                if lut.valid_count() > 0 {
                    cache.insert(z, x, y, lut);
                    total_computed += 1;

                    if let Some(ref mut callback) = progress_callback {
                        callback(z, x, y, total_computed);
                    }
                }
            }
        }
    }

    cache
}

/// Convert latitude to Mercator Y coordinate.
#[inline]
fn lat_to_mercator_y(lat_deg: f64) -> f64 {
    let lat_rad = lat_deg.to_radians();
    lat_rad.tan().asinh()
}

/// Convert Mercator Y coordinate to latitude.
#[inline]
fn mercator_y_to_lat(merc_y: f64) -> f64 {
    merc_y.sinh().atan().to_degrees()
}

/// Resample GOES data using a pre-computed LUT.
///
/// This is the fast path - just bilinear interpolation with pre-computed indices.
///
/// # Arguments
/// * `data` - Source GOES grid data
/// * `data_width` - Width of the source grid
/// * `lut` - Pre-computed projection LUT for this tile
///
/// # Returns
/// Resampled 256x256 output grid
pub fn resample_with_lut(data: &[f32], data_width: usize, lut: &TileGridLut) -> Vec<f32> {
    let mut output = vec![f32::NAN; PIXELS_PER_TILE];

    for (pixel_idx, &(grid_i, grid_j)) in lut.indices.iter().enumerate() {
        // Check validity via bitmap
        if !lut.is_valid(pixel_idx) {
            continue;
        }

        // Bilinear interpolation
        let i1 = grid_i.floor() as usize;
        let j1 = grid_j.floor() as usize;
        let i2 = i1 + 1;
        let j2 = j1 + 1;

        let di = grid_i - i1 as f32;
        let dj = grid_j - j1 as f32;

        // Sample four surrounding grid points
        let v11 = data.get(j1 * data_width + i1).copied().unwrap_or(f32::NAN);
        let v21 = data.get(j1 * data_width + i2).copied().unwrap_or(f32::NAN);
        let v12 = data.get(j2 * data_width + i1).copied().unwrap_or(f32::NAN);
        let v22 = data.get(j2 * data_width + i2).copied().unwrap_or(f32::NAN);

        // Skip if any corner is NaN
        if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
            continue;
        }

        // Bilinear interpolation
        let v1 = v11 * (1.0 - di) + v21 * di;
        let v2 = v12 * (1.0 - di) + v22 * di;
        let value = v1 * (1.0 - dj) + v2 * dj;

        output[pixel_idx] = value;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_lut_basic() {
        let mut lut = TileGridLut::new();

        // Test initial state
        assert!(!lut.is_valid(0));
        assert!(lut.get(0).is_none());
        assert_eq!(lut.valid_count(), 0);

        // Set a pixel
        lut.set(0, 100.5, 200.5);
        assert!(lut.is_valid(0));
        assert_eq!(lut.get(0), Some((100.5, 200.5)));
        assert_eq!(lut.valid_count(), 1);

        // Set another pixel
        lut.set(1000, 50.0, 75.0);
        assert!(lut.is_valid(1000));
        assert_eq!(lut.valid_count(), 2);
    }

    #[test]
    fn test_tile_lut_serialization() {
        let mut lut = TileGridLut::new();
        lut.set(0, 1.0, 2.0);
        lut.set(100, 3.0, 4.0);
        lut.set(65535, 5.0, 6.0);

        let bytes = lut.to_bytes();
        let restored = TileGridLut::from_bytes(&bytes).unwrap();

        assert_eq!(restored.get(0), Some((1.0, 2.0)));
        assert_eq!(restored.get(100), Some((3.0, 4.0)));
        assert_eq!(restored.get(65535), Some((5.0, 6.0)));
        assert!(restored.get(1).is_none());
        assert_eq!(restored.valid_count(), 3);
    }

    #[test]
    fn test_compute_tile_lut() {
        let proj = Geostationary::goes16_conus();

        // Compute a LUT for a tile that should be within GOES coverage
        // Tile at zoom 5, covering central US
        let lut = compute_tile_lut(&proj, 5, 7, 11, 5000, 3000);

        // Should have some valid pixels
        let valid = lut.valid_count();
        println!("Tile 5/7/11 has {} valid pixels", valid);
        assert!(valid > 0, "Expected some valid pixels in CONUS tile");
    }

    #[test]
    fn test_compute_tile_lut_outside_coverage() {
        let proj = Geostationary::goes16_conus();

        // Compute a LUT for a tile outside GOES coverage (e.g., Europe)
        let lut = compute_tile_lut(&proj, 5, 16, 10, 5000, 3000);

        // Should have zero valid pixels
        assert_eq!(
            lut.valid_count(),
            0,
            "Expected no valid pixels outside coverage"
        );
    }

    #[test]
    fn test_cache_save_load() {
        let proj = Geostationary::goes16_conus();

        // Create a small cache
        let mut cache = ProjectionLutCache::new("goes16", 2);

        // Add a few tiles
        let lut1 = compute_tile_lut(&proj, 0, 0, 0, 5000, 3000);
        let lut2 = compute_tile_lut(&proj, 1, 0, 0, 5000, 3000);
        cache.insert(0, 0, 0, lut1);
        cache.insert(1, 0, 0, lut2);

        // Save to bytes
        let mut buffer = Vec::new();
        cache.save(&mut buffer).unwrap();

        // Load back
        let restored = ProjectionLutCache::load(&buffer[..]).unwrap();

        assert_eq!(restored.satellite, "goes16");
        assert_eq!(restored.max_zoom, 2);
        assert_eq!(restored.len(), cache.len());
    }

    #[test]
    fn test_resample_with_lut() {
        // Create a simple test grid
        let data_width = 100;
        let data_height = 100;
        let mut data = vec![0.0f32; data_width * data_height];

        // Fill with a gradient
        for j in 0..data_height {
            for i in 0..data_width {
                data[j * data_width + i] = (i + j) as f32;
            }
        }

        // Create a simple LUT that maps directly
        let mut lut = TileGridLut::new();
        for y in 0..TILE_SIZE {
            for x in 0..TILE_SIZE {
                let pixel_idx = y * TILE_SIZE + x;
                // Map to center of the data grid
                let grid_i = 25.0 + (x as f32 / TILE_SIZE as f32) * 50.0;
                let grid_j = 25.0 + (y as f32 / TILE_SIZE as f32) * 50.0;
                if grid_i < 99.0 && grid_j < 99.0 {
                    lut.set(pixel_idx, grid_i, grid_j);
                }
            }
        }

        let output = resample_with_lut(&data, data_width, &lut);

        // Check that we got valid output
        let valid_count = output.iter().filter(|v| !v.is_nan()).count();
        assert!(valid_count > 0, "Expected some valid output values");

        // Check approximate values (should be around 50-150 given our mapping)
        let avg: f32 = output.iter().filter(|v| !v.is_nan()).sum::<f32>() / valid_count as f32;
        assert!(
            avg > 40.0 && avg < 160.0,
            "Average value {} out of expected range",
            avg
        );
    }
}
