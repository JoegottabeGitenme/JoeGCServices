//! Grid specifications for meteorological data.

use crate::BoundingBox;
use serde::{Deserialize, Serialize};

/// Specification of a regular lat/lon grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridSpec {
    /// Number of points in X (longitude) direction
    pub nx: usize,
    /// Number of points in Y (latitude) direction
    pub ny: usize,
    /// Grid resolution in X direction (degrees or meters depending on CRS)
    pub dx: f64,
    /// Grid resolution in Y direction
    pub dy: f64,
    /// First grid point longitude/X
    pub first_x: f64,
    /// First grid point latitude/Y
    pub first_y: f64,
    /// Scan mode flags (determines how data is ordered)
    pub scan_mode: ScanMode,
}

impl GridSpec {
    /// Create a new grid specification.
    pub fn new(
        nx: usize,
        ny: usize,
        dx: f64,
        dy: f64,
        first_x: f64,
        first_y: f64,
        scan_mode: ScanMode,
    ) -> Self {
        Self {
            nx,
            ny,
            dx,
            dy,
            first_x,
            first_y,
            scan_mode,
        }
    }

    /// Calculate the bounding box of this grid.
    pub fn bbox(&self) -> BoundingBox {
        let last_x = self.first_x + (self.nx - 1) as f64 * self.dx;
        let last_y = self.first_y + (self.ny - 1) as f64 * self.dy;

        BoundingBox {
            min_x: self.first_x.min(last_x),
            min_y: self.first_y.min(last_y),
            max_x: self.first_x.max(last_x),
            max_y: self.first_y.max(last_y),
        }
    }

    /// Convert a grid index to coordinates.
    pub fn index_to_coord(&self, i: usize, j: usize) -> Option<GridPoint> {
        if i >= self.nx || j >= self.ny {
            return None;
        }

        let (i_eff, j_eff) = self.scan_mode.adjust_indices(i, j, self.nx, self.ny);

        Some(GridPoint {
            x: self.first_x + i_eff as f64 * self.dx,
            y: self.first_y + j_eff as f64 * self.dy,
            i,
            j,
        })
    }

    /// Convert coordinates to the nearest grid index.
    pub fn coord_to_index(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let i_f = (x - self.first_x) / self.dx;
        let j_f = (y - self.first_y) / self.dy;

        let i = i_f.round() as isize;
        let j = j_f.round() as isize;

        if i < 0 || j < 0 || i >= self.nx as isize || j >= self.ny as isize {
            return None;
        }

        Some((i as usize, j as usize))
    }

    /// Get the 1D array index for a 2D grid position.
    pub fn flat_index(&self, i: usize, j: usize) -> usize {
        self.scan_mode.flat_index(i, j, self.nx, self.ny)
    }

    /// Total number of grid points.
    pub fn len(&self) -> usize {
        self.nx * self.ny
    }

    /// Check if grid is empty.
    pub fn is_empty(&self) -> bool {
        self.nx == 0 || self.ny == 0
    }
}

/// A point on the grid with both indices and coordinates.
#[derive(Debug, Clone, Copy)]
pub struct GridPoint {
    pub x: f64,
    pub y: f64,
    pub i: usize,
    pub j: usize,
}

/// Scan mode flags for grid data ordering.
///
/// Based on GRIB2 scanning mode (Flag Table 3.4).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScanMode {
    /// +i direction: false = +x (east), true = -x (west)
    pub i_negative: bool,
    /// +j direction: false = -y (south), true = +y (north)
    pub j_positive: bool,
    /// Adjacent points: false = i direction, true = j direction
    pub j_consecutive: bool,
    /// Row scan direction alternates
    pub alternating_rows: bool,
}

impl ScanMode {
    /// Most common mode: data starts at top-left, rows go west to east,
    /// columns go north to south.
    pub fn standard() -> Self {
        Self {
            i_negative: false,
            j_positive: false,
            j_consecutive: false,
            alternating_rows: false,
        }
    }

    /// Create from GRIB2 flag byte.
    pub fn from_grib2_flag(flag: u8) -> Self {
        Self {
            i_negative: (flag & 0x80) != 0,
            j_positive: (flag & 0x40) != 0,
            j_consecutive: (flag & 0x20) != 0,
            alternating_rows: (flag & 0x10) != 0,
        }
    }

    /// Adjust indices based on scan mode.
    pub fn adjust_indices(&self, i: usize, j: usize, nx: usize, ny: usize) -> (usize, usize) {
        let i_adj = if self.i_negative { nx - 1 - i } else { i };
        let j_adj = if self.j_positive { j } else { ny - 1 - j };
        (i_adj, j_adj)
    }

    /// Calculate flat array index from 2D indices.
    pub fn flat_index(&self, i: usize, j: usize, nx: usize, _ny: usize) -> usize {
        if self.j_consecutive {
            // Column-major order
            i * _ny + j
        } else {
            // Row-major order (most common)
            j * nx + i
        }
    }
}

/// Common grid definitions for NWP models.
pub mod grids {
    use super::*;

    /// GFS 0.25° global grid
    pub fn gfs_0p25() -> GridSpec {
        GridSpec::new(
            1440,
            721, // 0.25° resolution
            0.25,
            -0.25, // dx, dy (negative because data goes N to S)
            0.0,
            90.0, // starts at 0°E, 90°N
            ScanMode::standard(),
        )
    }

    /// GFS 0.5° global grid
    pub fn gfs_0p50() -> GridSpec {
        GridSpec::new(720, 361, 0.5, -0.5, 0.0, 90.0, ScanMode::standard())
    }

    /// HRRR CONUS grid (3km Lambert Conformal)
    pub fn hrrr_conus() -> GridSpec {
        GridSpec::new(
            1799,
            1059,
            3000.0,
            3000.0, // 3km in meters
            -2697568.0,
            -1587306.0, // SW corner in projection coords
            ScanMode {
                i_negative: false,
                j_positive: true, // HRRR goes south to north
                j_consecutive: false,
                alternating_rows: false,
            },
        )
    }

    /// NAM CONUS 12km grid
    pub fn nam_conus_12km() -> GridSpec {
        GridSpec::new(
            614,
            428,
            12191.0,
            12191.0, // ~12km
            -4226108.0,
            -832698.0,
            ScanMode::standard(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gfs_grid_bbox() {
        let grid = grids::gfs_0p25();
        let bbox = grid.bbox();

        // GFS global grid should cover entire world
        assert!(bbox.min_x >= -0.001 && bbox.min_x <= 0.001);
        assert!(bbox.min_y >= -90.001 && bbox.min_y <= -89.999);
        assert!(bbox.max_x >= 359.74 && bbox.max_x <= 359.76);
        assert!(bbox.max_y >= 89.999 && bbox.max_y <= 90.001);
    }

    #[test]
    fn test_index_to_coord() {
        let grid = grids::gfs_0p25();

        let point = grid.index_to_coord(0, 0).unwrap();
        assert!((point.x - 0.0).abs() < 0.001);
        assert!((point.y - 90.0).abs() < 0.001);
    }

    #[test]
    fn test_scan_mode_from_grib2() {
        let mode = ScanMode::from_grib2_flag(0x40);
        assert!(!mode.i_negative);
        assert!(mode.j_positive);
        assert!(!mode.j_consecutive);
    }
}
