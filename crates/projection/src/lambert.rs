//! Lambert Conformal Conic projection.
//!
//! This projection is commonly used for weather data including HRRR.
//! It maps a cone tangent or secant to the Earth's surface onto a flat plane.
//!
//! The projection parameters include:
//! - Reference latitude (lat0): The latitude of the origin
//! - Reference longitude (lon0): The central meridian (LoV in GRIB2)
//! - Standard parallel(s): Latin1 and Latin2 (can be equal for tangent cone)
//! - Grid spacing: dx, dy in meters
//! - First grid point: lat1, lon1

use std::f64::consts::PI;

/// Lambert Conformal Conic projection parameters.
///
/// These parameters define the projection from geographic (lat/lon) to
/// grid (i, j) coordinates and vice versa.
#[derive(Debug, Clone)]
pub struct LambertConformal {
    /// Central meridian (LoV) in radians
    pub lon0: f64,
    /// Reference latitude in radians (used for computing cone constant)
    pub lat0: f64,
    /// First standard parallel in radians
    pub latin1: f64,
    /// Second standard parallel in radians
    pub latin2: f64,
    /// Latitude of first grid point in radians
    pub lat1: f64,
    /// Longitude of first grid point in radians
    pub lon1: f64,
    /// Grid spacing in X direction (meters)
    pub dx: f64,
    /// Grid spacing in Y direction (meters)
    pub dy: f64,
    /// Number of grid points in X (i) direction
    pub nx: usize,
    /// Number of grid points in Y (j) direction
    pub ny: usize,
    /// Earth radius (meters)
    pub earth_radius: f64,
    /// Cone constant (n)
    n: f64,
    /// F constant
    f: f64,
    /// Rho at first grid point
    rho0: f64,
}

impl LambertConformal {
    /// Create a new Lambert Conformal projection from GRIB2 parameters.
    ///
    /// # Arguments
    /// * `lat1_deg` - Latitude of first grid point (degrees)
    /// * `lon1_deg` - Longitude of first grid point (degrees)
    /// * `lov_deg` - Central meridian / orientation of the grid (degrees)
    /// * `latin1_deg` - First standard parallel (degrees)
    /// * `latin2_deg` - Second standard parallel (degrees)
    /// * `dx` - Grid spacing X (meters)
    /// * `dy` - Grid spacing Y (meters)
    /// * `nx` - Number of X grid points
    /// * `ny` - Number of Y grid points
    pub fn from_grib2(
        lat1_deg: f64,
        lon1_deg: f64,
        lov_deg: f64,
        latin1_deg: f64,
        latin2_deg: f64,
        dx: f64,
        dy: f64,
        nx: usize,
        ny: usize,
    ) -> Self {
        let to_rad = PI / 180.0;

        let lat1 = lat1_deg * to_rad;
        let lon1 = lon1_deg * to_rad;
        let lon0 = lov_deg * to_rad;
        let latin1 = latin1_deg * to_rad;
        let latin2 = latin2_deg * to_rad;

        // Earth radius (WGS84 mean radius)
        let earth_radius = 6371229.0;

        // Compute cone constant n
        let n = if (latin1 - latin2).abs() < 1e-10 {
            // Tangent cone (single standard parallel)
            latin1.sin()
        } else {
            // Secant cone (two standard parallels)
            let ln_ratio = (latin1.cos() / latin2.cos()).ln();
            let tan_ratio =
                ((PI / 4.0 + latin2 / 2.0).tan() / (PI / 4.0 + latin1 / 2.0).tan()).ln();
            ln_ratio / tan_ratio
        };

        // Compute F constant
        let f = (latin1.cos() * (PI / 4.0 + latin1 / 2.0).tan().powf(n)) / n;

        // Compute rho at first grid point
        let rho0 = earth_radius * f / (PI / 4.0 + lat1 / 2.0).tan().powf(n);

        // Use latitude of first grid point as reference
        let lat0 = lat1;

        Self {
            lon0,
            lat0,
            latin1,
            latin2,
            lat1,
            lon1,
            dx,
            dy,
            nx,
            ny,
            earth_radius,
            n,
            f,
            rho0,
        }
    }

    /// Create HRRR projection with standard parameters.
    ///
    /// HRRR uses Lambert Conformal with:
    /// - First point: 21.138123°N, 237.280472°E (= -122.719528°W)
    /// - LoV: 262.5°E (= -97.5°W)
    /// - Standard parallels: 38.5°N (both)
    /// - Grid: 1799 x 1059, 3km spacing
    pub fn hrrr() -> Self {
        Self::from_grib2(
            21.138123,   // lat1
            -122.719528, // lon1 (237.280472 - 360)
            -97.5,       // LoV (262.5 - 360)
            38.5,        // latin1
            38.5,        // latin2
            3000.0,      // dx
            3000.0,      // dy
            1799,        // nx
            1059,        // ny
        )
    }

    /// Create projection parameters for NDFD CONUS 2.5km grid.
    ///
    /// NDFD uses Lambert Conformal with:
    /// - First point: 20.191999°N, 238.445999°E (= -121.554001°W)
    /// - LoV: 265.0°E (= -95.0°W)
    /// - Standard parallels: 25.0°N (both - tangent cone)
    /// - Grid: 2145 x 1377, 2539.703m spacing
    pub fn ndfd() -> Self {
        Self::from_grib2(
            20.191999,   // lat1
            -121.554001, // lon1 (238.445999 - 360)
            -95.0,       // LoV (265.0 - 360)
            25.0,        // latin1
            25.0,        // latin2
            2539.703,    // dx
            2539.703,    // dy
            2145,        // nx
            1377,        // ny
        )
    }

    /// Create projection parameters for NBM CONUS 2.5km grid.
    ///
    /// NBM CONUS uses Lambert Conformal with:
    /// - First point: 19.229°N, 233.7234°E (= -126.2766°W)
    /// - LoV: 265.0°E (= -95.0°W)
    /// - Standard parallels: 25.0°N (both - tangent cone)
    /// - Grid: 2345 x 1597, 2539.703m spacing
    pub fn nbm_conus() -> Self {
        Self::from_grib2(
            19.229,    // lat1
            -126.2766, // lon1 (233.7234 - 360)
            -95.0,     // LoV (265.0 - 360)
            25.0,      // latin1
            25.0,      // latin2
            2539.703,  // dx
            2539.703,  // dy
            2345,      // nx
            1597,      // ny
        )
    }

    /// Convert geographic coordinates (lat/lon in degrees) to grid indices (i, j).
    ///
    /// Returns (i, j) where i is the column (x) and j is the row (y).
    /// The indices may be fractional for interpolation purposes.
    pub fn geo_to_grid(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        let to_rad = PI / 180.0;
        let lat = lat_deg * to_rad;
        let lon = lon_deg * to_rad;

        // Normalize longitude difference to [-π, π]
        let mut dlon = lon - self.lon0;
        while dlon > PI {
            dlon -= 2.0 * PI;
        }
        while dlon < -PI {
            dlon += 2.0 * PI;
        }

        // Compute rho for this latitude
        let rho = self.earth_radius * self.f / (PI / 4.0 + lat / 2.0).tan().powf(self.n);

        // Compute theta (angle from central meridian)
        let theta = self.n * dlon;

        // Compute x, y in projection coordinates (meters from origin)
        let x = rho * theta.sin();
        let y = self.rho0 - rho * theta.cos();

        // Compute reference point (first grid point) in projection coordinates
        let mut dlon0 = self.lon1 - self.lon0;
        while dlon0 > PI {
            dlon0 -= 2.0 * PI;
        }
        while dlon0 < -PI {
            dlon0 += 2.0 * PI;
        }
        let theta0 = self.n * dlon0;
        let x0 = self.rho0 * theta0.sin();
        let y0 = self.rho0 - self.rho0 * theta0.cos();

        // Convert to grid indices
        let i = (x - x0) / self.dx;
        let j = (y - y0) / self.dy;

        (i, j)
    }

    /// Convert grid indices (i, j) to geographic coordinates (lat/lon in degrees).
    ///
    /// Returns (lat, lon) in degrees.
    pub fn grid_to_geo(&self, i: f64, j: f64) -> (f64, f64) {
        let to_deg = 180.0 / PI;

        // Compute reference point in projection coordinates
        let mut dlon0 = self.lon1 - self.lon0;
        while dlon0 > PI {
            dlon0 -= 2.0 * PI;
        }
        while dlon0 < -PI {
            dlon0 += 2.0 * PI;
        }
        let theta0 = self.n * dlon0;
        let x0 = self.rho0 * theta0.sin();
        let y0 = self.rho0 - self.rho0 * theta0.cos();

        // Compute x, y in projection coordinates
        let x = x0 + i * self.dx;
        let y = y0 + j * self.dy;

        // Compute rho and theta from x, y
        let rho = (x * x + (self.rho0 - y) * (self.rho0 - y)).sqrt();
        let rho = if self.n < 0.0 { -rho } else { rho };

        let theta = (x / (self.rho0 - y)).atan();

        // Compute latitude
        let lat = 2.0 * ((self.earth_radius * self.f / rho).powf(1.0 / self.n)).atan() - PI / 2.0;

        // Compute longitude
        let lon = self.lon0 + theta / self.n;

        (lat * to_deg, lon * to_deg)
    }

    /// Get the geographic bounding box of the grid.
    ///
    /// Returns (min_lon, min_lat, max_lon, max_lat) in degrees.
    /// Note: For Lambert Conformal, the bounding box in geographic coordinates
    /// is NOT a rectangle - the edges are curved. This returns the approximate
    /// bounding box that encloses all grid points.
    pub fn geographic_bounds(&self) -> (f64, f64, f64, f64) {
        let mut min_lat = f64::MAX;
        let mut max_lat = f64::MIN;
        let mut min_lon = f64::MAX;
        let mut max_lon = f64::MIN;

        // Sample grid edges and corners
        let points = [
            // Corners
            (0.0, 0.0),
            (self.nx as f64 - 1.0, 0.0),
            (0.0, self.ny as f64 - 1.0),
            (self.nx as f64 - 1.0, self.ny as f64 - 1.0),
            // Edge midpoints
            (self.nx as f64 / 2.0, 0.0),
            (self.nx as f64 / 2.0, self.ny as f64 - 1.0),
            (0.0, self.ny as f64 / 2.0),
            (self.nx as f64 - 1.0, self.ny as f64 / 2.0),
        ];

        for (i, j) in points {
            let (lat, lon) = self.grid_to_geo(i, j);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);
        }

        // Also sample along edges for better accuracy
        for t in 0..=10 {
            let frac = t as f64 / 10.0;

            // Bottom edge
            let (lat, lon) = self.grid_to_geo(frac * (self.nx as f64 - 1.0), 0.0);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);

            // Top edge
            let (lat, lon) = self.grid_to_geo(frac * (self.nx as f64 - 1.0), self.ny as f64 - 1.0);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);

            // Left edge
            let (lat, lon) = self.grid_to_geo(0.0, frac * (self.ny as f64 - 1.0));
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);

            // Right edge
            let (lat, lon) = self.grid_to_geo(self.nx as f64 - 1.0, frac * (self.ny as f64 - 1.0));
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);
        }

        (min_lon, min_lat, max_lon, max_lat)
    }

    /// Check if a geographic point is within the grid.
    pub fn contains(&self, lat_deg: f64, lon_deg: f64) -> bool {
        let (i, j) = self.geo_to_grid(lat_deg, lon_deg);
        i >= 0.0 && i < self.nx as f64 && j >= 0.0 && j < self.ny as f64
    }

    /// Get grid dimensions.
    pub fn dimensions(&self) -> (usize, usize) {
        (self.nx, self.ny)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hrrr_first_grid_point() {
        let proj = LambertConformal::hrrr();

        // First grid point should map to (0, 0)
        let (i, j) = proj.geo_to_grid(21.138123, -122.719528);
        assert!((i - 0.0).abs() < 0.1, "i should be ~0, got {}", i);
        assert!((j - 0.0).abs() < 0.1, "j should be ~0, got {}", j);
    }

    #[test]
    fn test_hrrr_roundtrip() {
        let proj = LambertConformal::hrrr();

        // Test roundtrip at grid center
        let test_i = 900.0;
        let test_j = 500.0;

        let (lat, lon) = proj.grid_to_geo(test_i, test_j);
        let (i, j) = proj.geo_to_grid(lat, lon);

        assert!(
            (i - test_i).abs() < 0.01,
            "i roundtrip failed: {} vs {}",
            test_i,
            i
        );
        assert!(
            (j - test_j).abs() < 0.01,
            "j roundtrip failed: {} vs {}",
            test_j,
            j
        );
    }

    #[test]
    fn test_hrrr_geographic_bounds() {
        let proj = LambertConformal::hrrr();
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();

        // HRRR should cover approximately CONUS
        println!(
            "HRRR bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
            min_lon, max_lon, min_lat, max_lat
        );

        assert!(
            min_lon < -120.0,
            "min_lon should be < -120, got {}",
            min_lon
        );
        assert!(max_lon > -65.0, "max_lon should be > -65, got {}", max_lon);
        assert!(
            min_lat > 20.0 && min_lat < 25.0,
            "min_lat should be ~21-25, got {}",
            min_lat
        );
        assert!(max_lat > 45.0, "max_lat should be > 45, got {}", max_lat);
    }

    #[test]
    fn test_nbm_conus_projection() {
        let proj = LambertConformal::nbm_conus();

        // First grid point should map to (0, 0)
        let (i, j) = proj.geo_to_grid(19.229, -126.2766);
        println!("First point (19.229, -126.28): i={:.2}, j={:.2}", i, j);
        assert!((i - 0.0).abs() < 0.5, "i should be ~0, got {}", i);
        assert!((j - 0.0).abs() < 0.5, "j should be ~0, got {}", j);

        // Test Kansas City (center of CONUS)
        let (i, j) = proj.geo_to_grid(39.0, -94.5);
        println!("Kansas City (39.0, -94.5): i={:.0}, j={:.0}", i, j);
        // Should be roughly in the middle of the 2345x1597 grid
        assert!(
            i > 900.0 && i < 1500.0,
            "KC should be in middle x, got {}",
            i
        );
        assert!(
            j > 600.0 && j < 1100.0,
            "KC should be in middle y, got {}",
            j
        );

        // Test bounds
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
        println!(
            "NBM CONUS bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
            min_lon, max_lon, min_lat, max_lat
        );
        assert!(
            min_lon < -124.0,
            "min_lon should be < -124, got {}",
            min_lon
        );
        assert!(max_lon > -65.0, "max_lon should be > -65, got {}", max_lon);
        assert!(min_lat > 15.0, "min_lat should be > 15, got {}", min_lat);
        assert!(max_lat > 50.0, "max_lat should be > 50, got {}", max_lat);
    }

    #[test]
    fn test_hrrr_conus_center() {
        let proj = LambertConformal::hrrr();

        // Kansas City, MO should be roughly in the center of CONUS
        let (i, j) = proj.geo_to_grid(39.0, -94.5);

        println!("Kansas City grid coords: i={}, j={}", i, j);

        // Should be roughly in the middle of the grid
        assert!(
            i > 700.0 && i < 1100.0,
            "KC should be in middle x, got {}",
            i
        );
        assert!(
            j > 400.0 && j < 700.0,
            "KC should be in middle y, got {}",
            j
        );
    }
}

#[test]
fn test_ndfd_projection() {
    let proj = LambertConformal::ndfd();

    // First grid point should map to (0, 0)
    let (i, j) = proj.geo_to_grid(20.191999, -121.554001);
    println!("First point (20.19, -121.55): i={:.2}, j={:.2}", i, j);
    assert!((i - 0.0).abs() < 0.5, "i should be ~0, got {}", i);
    assert!((j - 0.0).abs() < 0.5, "j should be ~0, got {}", j);

    // Test some known cities - Seattle should be in upper left area
    let (i, j) = proj.geo_to_grid(47.6, -122.3);
    println!("Seattle (47.6, -122.3): i={:.0}, j={:.0}", i, j);

    // Seattle should have small i (west) and large j (north)
    assert!(
        i < 500.0,
        "Seattle should be in western part of grid, i={}",
        i
    );
    assert!(
        j > 800.0,
        "Seattle should be in northern part of grid, j={}",
        j
    );

    // Miami should be in lower right area
    let (i, j) = proj.geo_to_grid(25.8, -80.2);
    println!("Miami (25.8, -80.2): i={:.0}, j={:.0}", i, j);

    // Miami should have large i (east) and small j (south)
    assert!(
        i > 1500.0,
        "Miami should be in eastern part of grid, i={}",
        i
    );
    assert!(
        j < 400.0,
        "Miami should be in southern part of grid, j={}",
        j
    );

    // Check corners
    println!("\nGrid corners:");
    let (lat, lon) = proj.grid_to_geo(0.0, 0.0);
    println!("  SW (0,0): lat={:.2}, lon={:.2}", lat, lon);

    let (lat, lon) = proj.grid_to_geo(2144.0, 0.0);
    println!("  SE (2144,0): lat={:.2}, lon={:.2}", lat, lon);

    let (lat, lon) = proj.grid_to_geo(0.0, 1376.0);
    println!("  NW (0,1376): lat={:.2}, lon={:.2}", lat, lon);

    let (lat, lon) = proj.grid_to_geo(2144.0, 1376.0);
    println!("  NE (2144,1376): lat={:.2}, lon={:.2}", lat, lon);
}

#[test]
fn test_ndfd_detailed_transform() {
    let proj = LambertConformal::ndfd();

    // Test Miami (25.8, -80.2) - should be in lower-right of grid
    let (i, j) = proj.geo_to_grid(25.8, -80.2);
    println!("Miami (25.8°N, 80.2°W):");
    println!("  Grid indices: i={:.2}, j={:.2}", i, j);

    // Verify roundtrip
    let (lat, lon) = proj.grid_to_geo(i, j);
    println!("  Roundtrip: lat={:.6}°, lon={:.6}°", lat, lon);

    // Test Denver (39.7, -105.0) - should be center-west
    let (i, j) = proj.geo_to_grid(39.7, -105.0);
    println!("\nDenver (39.7°N, 105.0°W):");
    println!("  Grid indices: i={:.2}, j={:.2}", i, j);
    let (lat, lon) = proj.grid_to_geo(i, j);
    println!("  Roundtrip: lat={:.6}°, lon={:.6}°", lat, lon);

    // Test a point in Kansas at center of grid
    let (i, j) = proj.geo_to_grid(38.5, -98.0);
    println!("\nKansas center (38.5°N, 98.0°W):");
    println!("  Grid indices: i={:.2}, j={:.2}", i, j);
    let (lat, lon) = proj.grid_to_geo(i, j);
    println!("  Roundtrip: lat={:.6}°, lon={:.6}°", lat, lon);

    // Test grid bounds
    let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
    println!("\nGrid bounds:");
    println!("  Lon: {:.2}° to {:.2}°", min_lon, max_lon);
    println!("  Lat: {:.2}° to {:.2}°", min_lat, max_lat);

    // Verify dimensions
    let (nx, ny) = proj.dimensions();
    println!("  Size: {} x {}", nx, ny);

    // Check that grid indices for corners are correct
    let (lat_sw, lon_sw) = proj.grid_to_geo(0.0, 0.0);
    let (lat_ne, lon_ne) = proj.grid_to_geo((nx - 1) as f64, (ny - 1) as f64);
    println!("\nGrid corner geographic coords:");
    println!("  (0,0) -> ({:.4}°, {:.4}°)", lat_sw, lon_sw);
    println!(
        "  ({},{}) -> ({:.4}°, {:.4}°)",
        nx - 1,
        ny - 1,
        lat_ne,
        lon_ne
    );

    // Test that Miami is in valid range
    assert!(i >= 0.0 && i < nx as f64, "Miami i out of range");
    assert!(j >= 0.0 && j < ny as f64, "Miami j out of range");
}

#[test]
fn test_ndfd_vs_hrrr_comparison() {
    // Both NDFD and HRRR use Lambert Conformal
    // Compare how they handle similar geographic points

    let ndfd = LambertConformal::ndfd();
    let hrrr = LambertConformal::hrrr();

    // Test point in center of CONUS
    let kansas = (38.5, -98.0);

    let (ndfd_i, ndfd_j) = ndfd.geo_to_grid(kansas.0, kansas.1);
    let (hrrr_i, hrrr_j) = hrrr.geo_to_grid(kansas.0, kansas.1);

    println!("Kansas (38.5°N, 98°W):");
    println!(
        "  NDFD: i={:.1}, j={:.1} (grid: {} x {})",
        ndfd_i, ndfd_j, ndfd.nx, ndfd.ny
    );
    println!(
        "  HRRR: i={:.1}, j={:.1} (grid: {} x {})",
        hrrr_i, hrrr_j, hrrr.nx, hrrr.ny
    );

    // Verify Kansas is within both grids
    assert!(
        ndfd_i >= 0.0 && ndfd_i < ndfd.nx as f64,
        "Kansas should be in NDFD grid"
    );
    assert!(
        ndfd_j >= 0.0 && ndfd_j < ndfd.ny as f64,
        "Kansas should be in NDFD grid"
    );
    assert!(
        hrrr_i >= 0.0 && hrrr_i < hrrr.nx as f64,
        "Kansas should be in HRRR grid"
    );
    assert!(
        hrrr_j >= 0.0 && hrrr_j < hrrr.ny as f64,
        "Kansas should be in HRRR grid"
    );

    // Verify Kansas is roughly in the middle of both grids
    let ndfd_i_ratio = ndfd_i / ndfd.nx as f64;
    let ndfd_j_ratio = ndfd_j / ndfd.ny as f64;
    let hrrr_i_ratio = hrrr_i / hrrr.nx as f64;
    let hrrr_j_ratio = hrrr_j / hrrr.ny as f64;

    println!(
        "  NDFD ratios: i={:.2}, j={:.2}",
        ndfd_i_ratio, ndfd_j_ratio
    );
    println!(
        "  HRRR ratios: i={:.2}, j={:.2}",
        hrrr_i_ratio, hrrr_j_ratio
    );

    // Both should have Kansas somewhere in the middle 30-70% range
    assert!(
        ndfd_i_ratio > 0.3 && ndfd_i_ratio < 0.7,
        "Kansas i should be ~middle in NDFD"
    );
    assert!(
        ndfd_j_ratio > 0.3 && ndfd_j_ratio < 0.7,
        "Kansas j should be ~middle in NDFD"
    );

    // Verify roundtrip
    let (lat, lon) = ndfd.grid_to_geo(ndfd_i, ndfd_j);
    println!("  NDFD roundtrip: ({:.6}, {:.6})", lat, lon);
    assert!((lat - kansas.0).abs() < 0.001, "Latitude roundtrip failed");
    assert!((lon - kansas.1).abs() < 0.001, "Longitude roundtrip failed");
}

#[test]
fn test_ndfd_east_west_coordinates() {
    let proj = LambertConformal::ndfd();

    // Western point: Los Angeles
    let (la_i, la_j) = proj.geo_to_grid(34.0, -118.2);
    println!(
        "Los Angeles (34.0°N, 118.2°W): i={:.1}, j={:.1}",
        la_i, la_j
    );

    // Eastern point: New York
    let (ny_i, ny_j) = proj.geo_to_grid(40.7, -74.0);
    println!("New York (40.7°N, 74.0°W): i={:.1}, j={:.1}", ny_i, ny_j);

    // Los Angeles should have SMALLER i than New York (LA is west, NY is east)
    // In grid coordinates: i increases from west to east
    assert!(
        la_i < ny_i,
        "LA (west) should have smaller i than NY (east): LA i={}, NY i={}",
        la_i,
        ny_i
    );

    // Check that both are within grid bounds
    assert!(la_i >= 0.0 && la_i < 2145.0, "LA i out of bounds: {}", la_i);
    assert!(ny_i >= 0.0 && ny_i < 2145.0, "NY i out of bounds: {}", ny_i);

    // Verify grid corners make sense
    let (_sw_lat, sw_lon) = proj.grid_to_geo(0.0, 0.0);
    let (_se_lat, se_lon) = proj.grid_to_geo(2144.0, 0.0);
    println!("SW corner (i=0): lon={:.2}°", sw_lon);
    println!("SE corner (i=2144): lon={:.2}°", se_lon);

    // SW should be more westerly (more negative longitude) than SE
    assert!(
        sw_lon < se_lon,
        "SW should be west of SE: SW lon={}, SE lon={}",
        sw_lon,
        se_lon
    );
}

#[test]
fn test_ndfd_central_meridian() {
    // Test that points at same latitude but different sides of central meridian
    // map to monotonically increasing grid indices (not mirrored)
    let proj = LambertConformal::ndfd();

    // Central meridian is -95°W (LoV)
    let lat = 38.5;

    println!(
        "Testing points at lat={}° around central meridian (-95°W):",
        lat
    );
    println!("Grid is 2145 wide, center would be at i=1072");

    // Points from west to east
    let (i_120, _) = proj.geo_to_grid(lat, -120.0); // West (California)
    let (i_105, _) = proj.geo_to_grid(lat, -105.0); // West (Colorado)
    let (i_95, _) = proj.geo_to_grid(lat, -95.0); // Center (Kansas)
    let (i_85, _) = proj.geo_to_grid(lat, -85.0); // East (Illinois)
    let (i_70, _) = proj.geo_to_grid(lat, -70.0); // East (Atlantic coast)

    println!("  -120°W: i={:.1}", i_120);
    println!("  -105°W: i={:.1}", i_105);
    println!("   -95°W: i={:.1} (central meridian)", i_95);
    println!("   -85°W: i={:.1}", i_85);
    println!("   -70°W: i={:.1}", i_70);

    // Verify monotonic increase from west to east
    assert!(
        i_120 < i_105,
        "i should increase west to east: -120° ({:.1}) < -105° ({:.1})",
        i_120,
        i_105
    );
    assert!(
        i_105 < i_95,
        "i should increase west to east: -105° ({:.1}) < -95° ({:.1})",
        i_105,
        i_95
    );
    assert!(
        i_95 < i_85,
        "i should increase west to east: -95° ({:.1}) < -85° ({:.1})",
        i_95,
        i_85
    );
    assert!(
        i_85 < i_70,
        "i should increase west to east: -85° ({:.1}) < -70° ({:.1})",
        i_85,
        i_70
    );
}

#[test]
fn test_ndfd_grid_to_geo_quadrant() {
    // Test that grid_to_geo returns correct quadrant for all grid positions
    // This tests for the atan vs atan2 bug where negative y_diff gives wrong quadrant
    let proj = LambertConformal::ndfd();

    println!("Testing grid_to_geo roundtrip at various grid positions:");

    // Test corners and middle
    let test_points = [
        (0.0, 0.0, "SW corner"),
        (2144.0, 0.0, "SE corner"),
        (0.0, 1376.0, "NW corner"),
        (2144.0, 1376.0, "NE corner"),
        (1072.0, 688.0, "Center"),
        (239.0, 572.0, "LA position"),
        (1810.0, 856.0, "NY position"),
    ];

    for (i, j, label) in test_points {
        let (lat, lon) = proj.grid_to_geo(i, j);
        let (i_back, j_back) = proj.geo_to_grid(lat, lon);

        let i_err = (i - i_back).abs();
        let j_err = (j - j_back).abs();

        println!("  {} (i={:.0}, j={:.0}): lat={:.2}°, lon={:.2}° -> i={:.1}, j={:.1} (err: {:.2}, {:.2})",
                 label, i, j, lat, lon, i_back, j_back, i_err, j_err);

        // Roundtrip should be accurate
        assert!(
            i_err < 0.1,
            "{} i roundtrip error too large: {:.4}",
            label,
            i_err
        );
        assert!(
            j_err < 0.1,
            "{} j roundtrip error too large: {:.4}",
            label,
            j_err
        );

        // Check that longitude is in expected range (roughly -130 to -60 for CONUS)
        assert!(
            lon > -140.0 && lon < -50.0,
            "{} longitude out of range: {:.2}",
            label,
            lon
        );
    }

    // Specific check: LA (west) should have more negative longitude than NY (east)
    let (_, la_lon) = proj.grid_to_geo(239.0, 572.0);
    let (_, ny_lon) = proj.grid_to_geo(1810.0, 856.0);
    println!("\nLA lon: {:.2}°, NY lon: {:.2}°", la_lon, ny_lon);
    assert!(
        la_lon < ny_lon,
        "LA should be west of NY: LA lon={:.2}, NY lon={:.2}",
        la_lon,
        ny_lon
    );

    // Intensive test: verify longitude increases monotonically across rows
    println!("\nChecking longitude monotonicity across rows:");
    for j in [0, 688, 1376] {
        let mut last_lon = f64::NEG_INFINITY;
        let mut monotonic = true;
        for i in (0..2145).step_by(100) {
            let (_, lon) = proj.grid_to_geo(i as f64, j as f64);
            if lon <= last_lon {
                println!(
                    "  MONOTONICITY VIOLATION at j={}: i={} lon={:.2} <= prev {:.2}",
                    j, i, lon, last_lon
                );
                monotonic = false;
            }
            last_lon = lon;
        }
        println!("  j={}: monotonic={}", j, monotonic);
        assert!(
            monotonic,
            "Longitude should increase monotonically from west to east at j={}",
            j
        );
    }
}
