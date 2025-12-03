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
            let tan_ratio = ((PI / 4.0 + latin2 / 2.0).tan() / (PI / 4.0 + latin1 / 2.0).tan()).ln();
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
            21.138123,      // lat1
            -122.719528,    // lon1 (237.280472 - 360)
            -97.5,          // LoV (262.5 - 360)
            38.5,           // latin1
            38.5,           // latin2
            3000.0,         // dx
            3000.0,         // dy
            1799,           // nx
            1059,           // ny
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
        
        assert!((i - test_i).abs() < 0.01, "i roundtrip failed: {} vs {}", test_i, i);
        assert!((j - test_j).abs() < 0.01, "j roundtrip failed: {} vs {}", test_j, j);
    }
    
    #[test]
    fn test_hrrr_geographic_bounds() {
        let proj = LambertConformal::hrrr();
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();
        
        // HRRR should cover approximately CONUS
        println!("HRRR bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}", min_lon, max_lon, min_lat, max_lat);
        
        assert!(min_lon < -120.0, "min_lon should be < -120, got {}", min_lon);
        assert!(max_lon > -65.0, "max_lon should be > -65, got {}", max_lon);
        assert!(min_lat > 20.0 && min_lat < 25.0, "min_lat should be ~21-25, got {}", min_lat);
        assert!(max_lat > 45.0, "max_lat should be > 45, got {}", max_lat);
    }
    
    #[test]
    fn test_hrrr_conus_center() {
        let proj = LambertConformal::hrrr();
        
        // Kansas City, MO should be roughly in the center of CONUS
        let (i, j) = proj.geo_to_grid(39.0, -94.5);
        
        println!("Kansas City grid coords: i={}, j={}", i, j);
        
        // Should be roughly in the middle of the grid
        assert!(i > 700.0 && i < 1100.0, "KC should be in middle x, got {}", i);
        assert!(j > 400.0 && j < 700.0, "KC should be in middle y, got {}", j);
    }
}
