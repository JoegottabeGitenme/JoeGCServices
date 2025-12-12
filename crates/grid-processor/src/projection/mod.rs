//! Projection utilities for grid processing.
//!
//! This module handles coordinate transformations and interpolation
//! for re-projecting grids to geographic coordinates.

pub mod interpolation;

pub use interpolation::{bilinear_interpolate, cubic_interpolate, nearest_interpolate};

use crate::types::BoundingBox;

/// Calculate the tile bounding box for a Web Mercator tile.
///
/// # Arguments
/// * `z` - Zoom level
/// * `x` - Tile X coordinate
/// * `y` - Tile Y coordinate
///
/// # Returns
/// Geographic bounding box (WGS84) for the tile
pub fn tile_to_bbox(z: u32, x: u32, y: u32) -> BoundingBox {
    let n = 2_u32.pow(z) as f64;

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

    BoundingBox::new(lon_min, lat_min, lon_max, lat_max)
}

/// Convert geographic coordinates to Web Mercator tile coordinates.
///
/// # Arguments
/// * `lon` - Longitude in degrees
/// * `lat` - Latitude in degrees
/// * `z` - Zoom level
///
/// # Returns
/// (x, y) tile coordinates
pub fn coords_to_tile(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = 2_u32.pow(z) as f64;

    let x = ((lon + 180.0) / 360.0 * n).floor() as u32;
    let lat_rad = lat.to_radians();
    let y = ((1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor() as u32;

    (x.min(n as u32 - 1), y.min(n as u32 - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_to_bbox() {
        // Test tile 0/0/0 covers the world
        let bbox = tile_to_bbox(0, 0, 0);
        assert!((bbox.min_lon - (-180.0)).abs() < 0.001);
        assert!((bbox.max_lon - 180.0).abs() < 0.001);

        // Test a specific tile at zoom 4
        let bbox = tile_to_bbox(4, 3, 5);
        assert!(bbox.min_lon < bbox.max_lon);
        assert!(bbox.min_lat < bbox.max_lat);
    }

    #[test]
    fn test_coords_to_tile() {
        // Test that (0, 0) maps to center tiles at various zooms
        let (x, y) = coords_to_tile(0.0, 0.0, 1);
        assert_eq!(x, 1);
        assert_eq!(y, 1);

        // Test that negative coordinates work
        let (x, _y) = coords_to_tile(-90.0, 45.0, 2);
        assert!(x < 2); // Should be in western hemisphere
    }

    #[test]
    fn test_roundtrip() {
        // A point in the center of a tile should roundtrip
        let z = 8;
        let x = 45;
        let y = 102;

        let bbox = tile_to_bbox(z, x, y);
        let (cx, cy) = bbox.center();
        let (rx, ry) = coords_to_tile(cx, cy, z);

        assert_eq!(rx, x);
        assert_eq!(ry, y);
    }
}
