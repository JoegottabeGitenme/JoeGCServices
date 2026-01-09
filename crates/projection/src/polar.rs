//! Polar Stereographic projection/transform.
//!
//! This projection is used for NBM Alaska and other high-latitude grids.
//! Polar Stereographic projects the Earth onto a plane tangent to one of the poles,
//! preserving angles (conformal) but distorting areas away from the pole.
//!
//! The projection parameters include:
//! - First grid point (lat1, lon1): Usually the SW corner
//! - True latitude (lat_d): Latitude where scale is true (typically 60°)
//! - Orientation longitude (lon_v): Longitude pointing up on the map
//! - Grid spacing (dx, dy): In meters at the true latitude
//! - Grid dimensions (nx, ny): Number of points in each direction

use std::f64::consts::PI;

/// Polar Stereographic projection parameters.
///
/// These parameters define the projection from geographic (lat/lon) to
/// grid (i, j) coordinates and vice versa.
#[derive(Debug, Clone)]
pub struct PolarStereographic {
    /// Latitude of first grid point (degrees)
    pub lat1: f64,
    /// Longitude of first grid point (degrees)
    pub lon1: f64,
    /// True latitude where grid spacing is accurate (degrees)
    pub lat_d: f64,
    /// Orientation longitude - points "up" on the map (degrees)
    pub lon_v: f64,
    /// Grid spacing in X direction at true latitude (meters)
    pub dx: f64,
    /// Grid spacing in Y direction at true latitude (meters)
    pub dy: f64,
    /// Number of grid points in X (i) direction
    pub nx: usize,
    /// Number of grid points in Y (j) direction
    pub ny: usize,
    /// Whether this is a North Pole projection (vs South Pole)
    pub is_north: bool,
    /// Earth radius (meters)
    pub earth_radius: f64,

    // Computed constants
    /// Scale factor at true latitude
    k0: f64,
    /// X coordinate of first grid point in projection space
    x1: f64,
    /// Y coordinate of first grid point in projection space
    y1: f64,
}

impl PolarStereographic {
    /// Create a new Polar Stereographic projection from GRIB2 parameters.
    ///
    /// # Arguments
    /// * `lat1_deg` - Latitude of first grid point (degrees)
    /// * `lon1_deg` - Longitude of first grid point (degrees)
    /// * `lat_d_deg` - True latitude where grid spacing is accurate (degrees)
    /// * `lon_v_deg` - Orientation longitude (degrees)
    /// * `dx` - Grid spacing X at true latitude (meters)
    /// * `dy` - Grid spacing Y at true latitude (meters)
    /// * `nx` - Number of X grid points
    /// * `ny` - Number of Y grid points
    /// * `is_north` - True for North Pole projection
    pub fn from_grib2(
        lat1_deg: f64,
        lon1_deg: f64,
        lat_d_deg: f64,
        lon_v_deg: f64,
        dx: f64,
        dy: f64,
        nx: usize,
        ny: usize,
        is_north: bool,
    ) -> Self {
        // Earth radius (same as GRIB2 default)
        let earth_radius = 6371200.0;

        // Scale factor at true latitude
        // k0 = (1 + sin(lat_d)) / 2 for north pole
        // k0 = (1 - sin(lat_d)) / 2 for south pole
        let lat_d_rad = lat_d_deg.abs() * PI / 180.0;
        let k0 = if is_north {
            (1.0 + lat_d_rad.sin()) / 2.0
        } else {
            (1.0 - lat_d_rad.sin()) / 2.0
        };

        // Calculate projection coordinates of first grid point
        let (x1, y1) =
            Self::geo_to_proj_internal(lat1_deg, lon1_deg, lon_v_deg, is_north, earth_radius, k0);

        Self {
            lat1: lat1_deg,
            lon1: lon1_deg,
            lat_d: lat_d_deg,
            lon_v: lon_v_deg,
            dx,
            dy,
            nx,
            ny,
            is_north,
            earth_radius,
            k0,
            x1,
            y1,
        }
    }

    /// Internal function to convert geographic to projection coordinates.
    fn geo_to_proj_internal(
        lat_deg: f64,
        lon_deg: f64,
        lon_v_deg: f64,
        is_north: bool,
        earth_radius: f64,
        k0: f64,
    ) -> (f64, f64) {
        let lat_rad = lat_deg * PI / 180.0;
        let lon_rad = lon_deg * PI / 180.0;
        let lon_v_rad = lon_v_deg * PI / 180.0;

        // Relative longitude from orientation
        let dlon = lon_rad - lon_v_rad;

        if is_north {
            // North Polar Stereographic
            // rho = 2 * R * k0 * tan(pi/4 - lat/2)
            let rho = 2.0 * earth_radius * k0 * (PI / 4.0 - lat_rad / 2.0).tan();
            let x = rho * dlon.sin();
            let y = -rho * dlon.cos();
            (x, y)
        } else {
            // South Polar Stereographic
            let rho = 2.0 * earth_radius * k0 * (PI / 4.0 + lat_rad / 2.0).tan();
            let x = rho * dlon.sin();
            let y = rho * dlon.cos();
            (x, y)
        }
    }

    /// Internal function to convert projection coordinates to geographic.
    fn proj_to_geo_internal(
        x: f64,
        y: f64,
        lon_v_deg: f64,
        is_north: bool,
        earth_radius: f64,
        k0: f64,
    ) -> (f64, f64) {
        let lon_v_rad = lon_v_deg * PI / 180.0;
        let rho = (x * x + y * y).sqrt();

        if rho < 1e-10 {
            // At the pole
            let lat = if is_north { 90.0 } else { -90.0 };
            let lon = lon_v_deg;
            return (lat, lon);
        }

        let lat_rad;
        let lon_rad;

        if is_north {
            // North Polar Stereographic
            // lat = pi/2 - 2 * atan(rho / (2 * R * k0))
            lat_rad = PI / 2.0 - 2.0 * (rho / (2.0 * earth_radius * k0)).atan();
            // lon = lon_v + atan2(x, -y)
            lon_rad = lon_v_rad + x.atan2(-y);
        } else {
            // South Polar Stereographic
            lat_rad = -PI / 2.0 + 2.0 * (rho / (2.0 * earth_radius * k0)).atan();
            lon_rad = lon_v_rad + x.atan2(y);
        }

        let mut lon_deg = lon_rad * 180.0 / PI;
        let lat_deg = lat_rad * 180.0 / PI;

        // Normalize longitude to -180 to 180
        while lon_deg > 180.0 {
            lon_deg -= 360.0;
        }
        while lon_deg < -180.0 {
            lon_deg += 360.0;
        }

        (lat_deg, lon_deg)
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
        // Convert to projection coordinates
        let (x, y) = Self::geo_to_proj_internal(
            lat_deg,
            lon_deg,
            self.lon_v,
            self.is_north,
            self.earth_radius,
            self.k0,
        );

        // Convert to grid indices relative to first point
        // i increases to the right (positive x direction)
        // j increases upward (positive y direction)
        let i = (x - self.x1) / self.dx;
        let j = (y - self.y1) / self.dy;

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
        // Convert grid indices to projection coordinates
        let x = self.x1 + i * self.dx;
        let y = self.y1 + j * self.dy;

        // Convert to geographic
        Self::proj_to_geo_internal(x, y, self.lon_v, self.is_north, self.earth_radius, self.k0)
    }

    /// Get the geographic bounding box of this grid by sampling all corners and edges.
    ///
    /// For polar stereographic, the bbox is complex because the grid doesn't
    /// align with parallels/meridians. We sample many points to find the true extent.
    ///
    /// # Returns
    /// (min_lon, min_lat, max_lon, max_lat) in degrees
    pub fn geographic_bounds(&self) -> (f64, f64, f64, f64) {
        let mut min_lat: f64 = 90.0;
        let mut max_lat: f64 = -90.0;
        let mut min_lon: f64 = 180.0;
        let mut max_lon: f64 = -180.0;

        // Sample all four edges of the grid
        let samples = 50;

        // Bottom edge (j=0)
        for s in 0..=samples {
            let i = (s as f64 / samples as f64) * (self.nx - 1) as f64;
            let (lat, lon) = self.grid_to_geo(i, 0.0);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            Self::update_lon_bounds(&mut min_lon, &mut max_lon, lon);
        }

        // Top edge (j=ny-1)
        for s in 0..=samples {
            let i = (s as f64 / samples as f64) * (self.nx - 1) as f64;
            let (lat, lon) = self.grid_to_geo(i, (self.ny - 1) as f64);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            Self::update_lon_bounds(&mut min_lon, &mut max_lon, lon);
        }

        // Left edge (i=0)
        for s in 0..=samples {
            let j = (s as f64 / samples as f64) * (self.ny - 1) as f64;
            let (lat, lon) = self.grid_to_geo(0.0, j);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            Self::update_lon_bounds(&mut min_lon, &mut max_lon, lon);
        }

        // Right edge (i=nx-1)
        for s in 0..=samples {
            let j = (s as f64 / samples as f64) * (self.ny - 1) as f64;
            let (lat, lon) = self.grid_to_geo((self.nx - 1) as f64, j);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
            Self::update_lon_bounds(&mut min_lon, &mut max_lon, lon);
        }

        (min_lon, min_lat, max_lon, max_lat)
    }

    /// Update longitude bounds, handling the Date Line crossing.
    fn update_lon_bounds(min_lon: &mut f64, max_lon: &mut f64, lon: f64) {
        // For grids crossing the Date Line, we track both extremes
        // The calling code will need to detect if the span crosses 180°
        if lon < *min_lon {
            *min_lon = lon;
        }
        if lon > *max_lon {
            *max_lon = lon;
        }
    }

    /// Check if this grid crosses the Date Line (180°/-180° longitude).
    pub fn crosses_dateline(&self) -> bool {
        let (min_lon, _, max_lon, _) = self.geographic_bounds();
        // If the longitude span is > 180°, it likely crosses the date line
        // Or if sampling shows points on both sides of 180°
        (max_lon - min_lon) > 180.0
    }

    /// Get two bounding boxes if the grid crosses the Date Line.
    ///
    /// # Returns
    /// Some((west_bbox, east_bbox)) if crossing, None otherwise
    /// west_bbox covers the area from min_lon to 180°
    /// east_bbox covers the area from -180° to max_lon
    pub fn split_bounds_at_dateline(&self) -> Option<((f64, f64, f64, f64), (f64, f64, f64, f64))> {
        // Sample the grid to find the actual longitude range
        let mut lons_positive = Vec::new(); // 0 to 180
        let mut lons_negative = Vec::new(); // -180 to 0
        let mut min_lat = 90.0f64;
        let mut max_lat = -90.0f64;

        let samples = 20;
        for si in 0..=samples {
            for sj in 0..=samples {
                let i = (si as f64 / samples as f64) * (self.nx - 1) as f64;
                let j = (sj as f64 / samples as f64) * (self.ny - 1) as f64;
                let (lat, lon) = self.grid_to_geo(i, j);

                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);

                if lon >= 0.0 {
                    lons_positive.push(lon);
                } else {
                    lons_negative.push(lon);
                }
            }
        }

        // If we have longitudes on both sides of 0, we might cross the Date Line
        if !lons_positive.is_empty() && !lons_negative.is_empty() {
            // Check if the positive longitudes are > 90 and negative < -90
            // This indicates crossing at 180° rather than at 0°
            let max_positive = lons_positive.iter().cloned().fold(f64::MIN, f64::max);
            let min_negative = lons_negative.iter().cloned().fold(f64::MAX, f64::min);

            if max_positive > 90.0 && min_negative < -90.0 {
                // Crosses Date Line
                // West box: from min of positive lons to 180
                let west_min_lon = lons_positive.iter().cloned().fold(f64::MAX, f64::min);
                // East box: from -180 to max of negative lons
                let east_max_lon = lons_negative.iter().cloned().fold(f64::MIN, f64::max);

                return Some((
                    (west_min_lon, min_lat, 180.0, max_lat),
                    (-180.0, min_lat, east_max_lon, max_lat),
                ));
            }
        }

        None
    }

    /// Check if a geographic point is within this grid's bounds.
    pub fn contains(&self, lat_deg: f64, lon_deg: f64) -> bool {
        let (i, j) = self.geo_to_grid(lat_deg, lon_deg);
        i >= -0.5 && i <= (self.nx as f64 - 0.5) && j >= -0.5 && j <= (self.ny as f64 - 0.5)
    }

    // =========================================================================
    // NBM Regional Presets
    // =========================================================================

    /// Create projection for NBM Alaska grid.
    ///
    /// NBM Alaska uses North Polar Stereographic with:
    /// - First point: 40.530°N, 181.429°E (-178.571°W)
    /// - True latitude: 60.0°N
    /// - Orientation: 210.0°E (-150.0°W) - Alaska centered
    /// - Grid spacing: 2976.56m at true latitude
    /// - Grid: 1649 x 1105
    pub fn nbm_alaska() -> Self {
        Self::from_grib2(
            40.530,  // lat1
            181.429, // lon1 (will be normalized internally)
            60.0,    // lat_d (true latitude)
            210.0,   // lon_v (orientation, points up)
            2976.56, // dx
            2976.56, // dy
            1649,    // nx
            1105,    // ny
            true,    // is_north
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alaska_first_point() {
        let proj = PolarStereographic::nbm_alaska();

        // First grid point should map to (0, 0)
        // lon1 = 181.429 = -178.571 in -180/180 convention
        let (i, j) = proj.geo_to_grid(40.530, 181.429 - 360.0);
        println!("Alaska first point: i={:.4}, j={:.4}", i, j);
        assert!((i - 0.0).abs() < 0.5, "i should be ~0, got {}", i);
        assert!((j - 0.0).abs() < 0.5, "j should be ~0, got {}", j);
    }

    #[test]
    fn test_alaska_roundtrip() {
        let proj = PolarStereographic::nbm_alaska();

        // Test roundtrip for various grid points
        for &(test_i, test_j) in &[(0.0, 0.0), (824.0, 552.0), (1648.0, 1104.0), (500.0, 800.0)] {
            let (lat, lon) = proj.grid_to_geo(test_i, test_j);
            let (i, j) = proj.geo_to_grid(lat, lon);
            println!(
                "Roundtrip ({}, {}): lat={:.4}, lon={:.4} -> ({:.4}, {:.4})",
                test_i, test_j, lat, lon, i, j
            );
            assert!(
                (i - test_i).abs() < 0.01,
                "i roundtrip failed for ({}, {}): {} vs {}",
                test_i,
                test_j,
                test_i,
                i
            );
            assert!(
                (j - test_j).abs() < 0.01,
                "j roundtrip failed for ({}, {}): {} vs {}",
                test_i,
                test_j,
                test_j,
                j
            );
        }
    }

    #[test]
    fn test_alaska_fairbanks() {
        let proj = PolarStereographic::nbm_alaska();

        // Fairbanks, AK: ~64.8°N, -147.7°W
        // Should be somewhere in the grid
        let (i, j) = proj.geo_to_grid(64.8, -147.7);
        println!("Fairbanks: i={:.2}, j={:.2}", i, j);

        assert!(i > 0.0 && i < 1649.0, "Fairbanks i out of range: {}", i);
        assert!(j > 0.0 && j < 1105.0, "Fairbanks j out of range: {}", j);

        // Verify with reverse
        let (lat, lon) = proj.grid_to_geo(i, j);
        println!("Fairbanks reverse: lat={:.2}, lon={:.2}", lat, lon);
        assert!((lat - 64.8).abs() < 0.1, "Latitude mismatch: {}", lat);
        assert!((lon - (-147.7)).abs() < 0.5, "Longitude mismatch: {}", lon);
    }

    #[test]
    fn test_alaska_bounds() {
        let proj = PolarStereographic::nbm_alaska();
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();

        println!(
            "Alaska bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
            min_lon, max_lon, min_lat, max_lat
        );

        // Alaska grid should cover roughly:
        // Lat: ~40°N to ~76°N (extends into Arctic)
        // Lon: ~150°E to ~-94°W (crossing Date Line)
        assert!(
            min_lat > 35.0 && min_lat < 45.0,
            "min_lat wrong: {}",
            min_lat
        );
        assert!(
            max_lat > 60.0 && max_lat < 80.0,
            "max_lat wrong: {}",
            max_lat
        );
    }

    #[test]
    fn test_alaska_crosses_dateline() {
        let proj = PolarStereographic::nbm_alaska();

        // Check if it detects Date Line crossing
        let crosses = proj.crosses_dateline();
        println!("Alaska crosses dateline: {}", crosses);

        // Try to split
        if let Some((west, east)) = proj.split_bounds_at_dateline() {
            println!("West bbox: {:?}", west);
            println!("East bbox: {:?}", east);

            // West should be positive longitudes (near 180)
            assert!(west.0 > 100.0, "West min_lon should be > 100: {}", west.0);
            assert!(west.2 == 180.0, "West max_lon should be 180: {}", west.2);

            // East should be negative longitudes (near -180)
            assert!(east.0 == -180.0, "East min_lon should be -180: {}", east.0);
            assert!(east.2 < -90.0, "East max_lon should be < -90: {}", east.2);
        }
    }

    #[test]
    fn test_polar_stereo_math() {
        // Test basic polar stereographic math
        let earth_radius = 6371200.0;
        let k0 = (1.0 + (60.0_f64 * PI / 180.0).sin()) / 2.0;

        // Point at 60°N, 0°E should have a known distance from pole
        let (x, y) =
            PolarStereographic::geo_to_proj_internal(60.0, 0.0, 0.0, true, earth_radius, k0);
        println!("60°N, 0°E -> x={:.0}, y={:.0}", x, y);

        // At 60°N with lon_v=0, x should be 0, y should be negative
        assert!(x.abs() < 100.0, "x should be ~0 at lon=0");
        assert!(y < 0.0, "y should be negative (south of pole)");

        // Roundtrip
        let (lat, lon) =
            PolarStereographic::proj_to_geo_internal(x, y, 0.0, true, earth_radius, k0);
        assert!((lat - 60.0).abs() < 0.01, "Lat roundtrip failed: {}", lat);
        assert!(lon.abs() < 0.01, "Lon roundtrip failed: {}", lon);
    }
}
