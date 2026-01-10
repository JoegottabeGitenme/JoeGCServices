//! Mercator projection/transform.
//!
//! This projection is used for NBM Hawaii, Puerto Rico, and Guam grids.
//! Mercator is a cylindrical projection that preserves angles (conformal)
//! but distorts areas, especially at high latitudes.
//!
//! The projection parameters include:
//! - First grid point (lat1, lon1): SW corner of the grid
//! - Last grid point (lat2, lon2): NE corner of the grid
//! - Reference latitude (lat_d): Latitude where grid spacing is true
//! - Grid dimensions (nx, ny): Number of points in each direction

use std::f64::consts::PI;

/// Mercator projection parameters.
///
/// These parameters define the projection from geographic (lat/lon) to
/// grid (i, j) coordinates and vice versa.
#[derive(Debug, Clone)]
pub struct Mercator {
    /// Latitude of first grid point (degrees)
    pub lat1: f64,
    /// Longitude of first grid point (degrees)
    pub lon1: f64,
    /// Latitude of last grid point (degrees)
    pub lat2: f64,
    /// Longitude of last grid point (degrees)
    pub lon2: f64,
    /// Reference latitude where dx/dy spacing is specified (degrees)
    pub lat_d: f64,
    /// Number of grid points in X (i) direction
    pub nx: usize,
    /// Number of grid points in Y (j) direction
    pub ny: usize,
    /// Earth radius (meters)
    pub earth_radius: f64,

    // Computed constants
    /// Scale factor at reference latitude
    k0: f64,
    /// Mercator Y coordinate of first grid point
    y1: f64,
    /// Mercator Y coordinate of last grid point
    y2: f64,
    /// X coordinate of first grid point (in projection units)
    x1: f64,
    /// X coordinate of last grid point (in projection units)
    x2: f64,
}

impl Mercator {
    /// Create a new Mercator projection from GRIB2 parameters.
    ///
    /// # Arguments
    /// * `lat1_deg` - Latitude of first grid point (degrees)
    /// * `lon1_deg` - Longitude of first grid point (degrees)
    /// * `lat2_deg` - Latitude of last grid point (degrees)
    /// * `lon2_deg` - Longitude of last grid point (degrees)
    /// * `lat_d_deg` - Reference latitude for grid spacing (degrees)
    /// * `nx` - Number of X grid points
    /// * `ny` - Number of Y grid points
    pub fn from_grib2(
        lat1_deg: f64,
        lon1_deg: f64,
        lat2_deg: f64,
        lon2_deg: f64,
        lat_d_deg: f64,
        nx: usize,
        ny: usize,
    ) -> Self {
        // Earth radius (WGS84 mean radius, same as used in GRIB2)
        let earth_radius = 6371200.0;

        // Scale factor at reference latitude
        let lat_d_rad = lat_d_deg * PI / 180.0;
        let k0 = lat_d_rad.cos();

        // Compute Mercator Y coordinates for first and last points
        let y1 = Self::lat_to_mercator_y(lat1_deg, earth_radius);
        let y2 = Self::lat_to_mercator_y(lat2_deg, earth_radius);

        // X coordinates (simple longitude scaling)
        let x1 = lon1_deg * PI / 180.0 * earth_radius;
        let x2 = lon2_deg * PI / 180.0 * earth_radius;

        Self {
            lat1: lat1_deg,
            lon1: lon1_deg,
            lat2: lat2_deg,
            lon2: lon2_deg,
            lat_d: lat_d_deg,
            nx,
            ny,
            earth_radius,
            k0,
            y1,
            y2,
            x1,
            x2,
        }
    }

    /// Convert latitude to Mercator Y coordinate.
    fn lat_to_mercator_y(lat_deg: f64, earth_radius: f64) -> f64 {
        let lat_rad = lat_deg * PI / 180.0;
        // Mercator Y = R * ln(tan(pi/4 + lat/2))
        earth_radius * (PI / 4.0 + lat_rad / 2.0).tan().ln()
    }

    /// Convert Mercator Y coordinate to latitude.
    fn mercator_y_to_lat(y: f64, earth_radius: f64) -> f64 {
        // lat = 2 * atan(exp(y/R)) - pi/2
        let lat_rad = 2.0 * (y / earth_radius).exp().atan() - PI / 2.0;
        lat_rad * 180.0 / PI
    }

    /// Convert geographic coordinates to grid indices.
    ///
    /// # Arguments
    /// * `lat_deg` - Latitude in degrees
    /// * `lon_deg` - Longitude in degrees
    ///
    /// # Returns
    /// (i, j) grid indices (can be fractional for interpolation)
    pub fn geo_to_grid(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64) {
        // Convert latitude to Mercator Y
        let y = Self::lat_to_mercator_y(lat_deg, self.earth_radius);

        // Convert longitude to X (handle 0-360 vs -180/180)
        let mut lon = lon_deg;
        // Normalize longitude to match grid convention
        if self.lon1 > 180.0 && lon < 0.0 {
            lon += 360.0;
        } else if self.lon1 < 0.0 && lon > 180.0 {
            lon -= 360.0;
        }
        let x = lon * PI / 180.0 * self.earth_radius;

        // Calculate grid indices
        // i increases with longitude (west to east)
        let i = (x - self.x1) / (self.x2 - self.x1) * (self.nx - 1) as f64;

        // j increases with latitude (south to north for SN scanning)
        let j = (y - self.y1) / (self.y2 - self.y1) * (self.ny - 1) as f64;

        (i, j)
    }

    /// Convert grid indices to geographic coordinates.
    ///
    /// # Arguments
    /// * `i` - Grid index in X direction
    /// * `j` - Grid index in Y direction
    ///
    /// # Returns
    /// (latitude, longitude) in degrees
    pub fn grid_to_geo(&self, i: f64, j: f64) -> (f64, f64) {
        // Interpolate X and Y in projection space
        let x = self.x1 + (i / (self.nx - 1) as f64) * (self.x2 - self.x1);
        let y = self.y1 + (j / (self.ny - 1) as f64) * (self.y2 - self.y1);

        // Convert back to geographic
        let lon_deg = x / self.earth_radius * 180.0 / PI;
        let lat_deg = Self::mercator_y_to_lat(y, self.earth_radius);

        (lat_deg, lon_deg)
    }

    /// Get the geographic bounding box of this grid.
    ///
    /// # Returns
    /// (min_lon, min_lat, max_lon, max_lat) in degrees
    pub fn geographic_bounds(&self) -> (f64, f64, f64, f64) {
        // For Mercator, the bounds are simply the first and last points
        let min_lon = self.lon1.min(self.lon2);
        let max_lon = self.lon1.max(self.lon2);
        let min_lat = self.lat1.min(self.lat2);
        let max_lat = self.lat1.max(self.lat2);
        (min_lon, min_lat, max_lon, max_lat)
    }

    /// Get the geographic bounding box normalized to -180/180 longitude.
    ///
    /// # Returns
    /// (min_lon, min_lat, max_lon, max_lat) in degrees, with lon in [-180, 180]
    pub fn geographic_bounds_normalized(&self) -> (f64, f64, f64, f64) {
        let (mut min_lon, min_lat, mut max_lon, max_lat) = self.geographic_bounds();

        // Normalize to -180/180
        if min_lon > 180.0 {
            min_lon -= 360.0;
        }
        if max_lon > 180.0 {
            max_lon -= 360.0;
        }

        (min_lon, min_lat, max_lon, max_lat)
    }

    /// Check if a geographic point is within this grid's bounds.
    pub fn contains(&self, lat_deg: f64, lon_deg: f64) -> bool {
        let (i, j) = self.geo_to_grid(lat_deg, lon_deg);
        i >= 0.0 && i <= (self.nx - 1) as f64 && j >= 0.0 && j <= (self.ny - 1) as f64
    }

    // =========================================================================
    // NBM Regional Presets
    // =========================================================================

    /// Create projection for NBM Hawaii grid.
    ///
    /// NBM Hawaii uses Mercator with:
    /// - First point: 14.3515°N, 195.0305°E (-164.9695°W)
    /// - Last point: 26.8605°N, 209.9598°E (-150.0402°W)
    /// - Reference latitude: 20.0°N
    /// - Grid: 625 x 561
    pub fn nbm_hawaii() -> Self {
        Self::from_grib2(
            14.3515,  // lat1
            195.0305, // lon1 (0-360 convention)
            26.8605,  // lat2
            209.9598, // lon2
            20.0,     // lat_d
            625,      // nx
            561,      // ny
        )
    }

    /// Create projection for NBM Puerto Rico grid.
    ///
    /// NBM Puerto Rico uses Mercator with:
    /// - First point: 16.9775°N, 291.9722°E (-68.0278°W)
    /// - Last point: 19.5221°N, 296.0156°E (-63.9844°W)
    /// - Reference latitude: 20.0°N
    /// - Grid: 339 x 225
    pub fn nbm_puertorico() -> Self {
        Self::from_grib2(
            16.9775,  // lat1
            291.9722, // lon1 (0-360 convention)
            19.5221,  // lat2
            296.0156, // lon2
            20.0,     // lat_d
            339,      // nx
            225,      // ny
        )
    }

    /// Create projection for NBM Guam grid.
    ///
    /// NBM Guam uses Mercator with:
    /// - First point: 12.3499°N, 143.6865°E
    /// - Last point: 16.7944°N, 148.2800°E
    /// - Reference latitude: 20.0°N
    /// - Grid: 193 x 193
    pub fn nbm_guam() -> Self {
        Self::from_grib2(
            12.3499,  // lat1
            143.6865, // lon1 (already in 0-180 range)
            16.7944,  // lat2
            148.2800, // lon2
            20.0,     // lat_d
            193,      // nx
            193,      // ny
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hawaii_first_point() {
        let proj = Mercator::nbm_hawaii();

        // First grid point should map to (0, 0)
        let (i, j) = proj.geo_to_grid(14.3515, 195.0305);
        println!("Hawaii first point: i={:.4}, j={:.4}", i, j);
        assert!((i - 0.0).abs() < 0.01, "i should be ~0, got {}", i);
        assert!((j - 0.0).abs() < 0.01, "j should be ~0, got {}", j);
    }

    #[test]
    fn test_hawaii_last_point() {
        let proj = Mercator::nbm_hawaii();

        // Last grid point should map to (nx-1, ny-1)
        let (i, j) = proj.geo_to_grid(26.8605, 209.9598);
        println!("Hawaii last point: i={:.4}, j={:.4}", i, j);
        assert!((i - 624.0).abs() < 0.5, "i should be ~624, got {}", i);
        assert!((j - 560.0).abs() < 0.5, "j should be ~560, got {}", j);
    }

    #[test]
    fn test_hawaii_roundtrip() {
        let proj = Mercator::nbm_hawaii();

        // Test roundtrip for center point
        let (lat, lon) = proj.grid_to_geo(312.0, 280.0);
        println!("Hawaii center: lat={:.4}, lon={:.4}", lat, lon);

        let (i, j) = proj.geo_to_grid(lat, lon);
        assert!((i - 312.0).abs() < 0.001, "i roundtrip failed: {}", i);
        assert!((j - 280.0).abs() < 0.001, "j roundtrip failed: {}", j);
    }

    #[test]
    fn test_hawaii_negative_longitude() {
        let proj = Mercator::nbm_hawaii();

        // Test with negative longitude (-157° = Honolulu area)
        let (i, j) = proj.geo_to_grid(21.3, -157.0);
        println!("Honolulu area: i={:.2}, j={:.2}", i, j);

        // Should be roughly in the middle of the grid
        assert!(i > 100.0 && i < 500.0, "i should be in grid, got {}", i);
        assert!(j > 100.0 && j < 450.0, "j should be in grid, got {}", j);
    }

    #[test]
    fn test_puertorico_bounds() {
        let proj = Mercator::nbm_puertorico();
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds_normalized();

        println!(
            "PR bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
            min_lon, max_lon, min_lat, max_lat
        );

        // PR should be in the Caribbean
        assert!(min_lon < -63.0 && min_lon > -69.0);
        assert!(max_lon < -63.0 && max_lon > -69.0);
        assert!(min_lat > 16.0 && min_lat < 18.0);
        assert!(max_lat > 19.0 && max_lat < 20.0);
    }

    #[test]
    fn test_guam_bounds() {
        let proj = Mercator::nbm_guam();
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();

        println!(
            "Guam bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
            min_lon, max_lon, min_lat, max_lat
        );

        // Guam should be in the western Pacific
        assert!(min_lon > 143.0 && min_lon < 144.0);
        assert!(max_lon > 148.0 && max_lon < 149.0);
        assert!(min_lat > 12.0 && min_lat < 13.0);
        assert!(max_lat > 16.0 && max_lat < 17.0);
    }

    #[test]
    fn test_mercator_y_conversion() {
        // Test Mercator Y conversion at various latitudes
        let earth_radius = 6371200.0;

        // Equator should give Y = 0
        let y_eq = Mercator::lat_to_mercator_y(0.0, earth_radius);
        assert!(y_eq.abs() < 1.0, "Equator Y should be ~0, got {}", y_eq);

        // Roundtrip test
        let lat_test = 20.0;
        let y = Mercator::lat_to_mercator_y(lat_test, earth_radius);
        let lat_back = Mercator::mercator_y_to_lat(y, earth_radius);
        assert!(
            (lat_back - lat_test).abs() < 0.0001,
            "Lat roundtrip failed: {} vs {}",
            lat_test,
            lat_back
        );
    }
}
