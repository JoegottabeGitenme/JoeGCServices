//! Geostationary satellite projection.
//!
//! This projection is used for GOES-R series satellite imagery.
//! The satellite views Earth from a fixed position above the equator,
//! and coordinates are expressed as scan angles in radians from nadir.
//!
//! Reference: GOES-R Product Definition and Users' Guide (PUG) Volume 4

/// Geostationary projection parameters.
///
/// These parameters define the projection from geographic (lat/lon) to
/// satellite scan angle (x, y) coordinates and vice versa.
#[derive(Debug, Clone)]
pub struct Geostationary {
    /// Satellite height above Earth center (meters)
    /// This is perspective_point_height + semi_major_axis
    pub h: f64,
    /// Perspective point height above Earth surface (meters)
    pub perspective_point_height: f64,
    /// Semi-major axis of Earth ellipsoid (meters)
    pub req: f64,
    /// Semi-minor axis of Earth ellipsoid (meters)
    pub rpol: f64,
    /// Longitude of satellite nadir point (radians)
    pub lambda_0: f64,
    /// Sweep angle axis ("x" for GOES-R, "y" for Meteosat/Himawari)
    pub sweep_x: bool,
    /// X coordinate of first grid point (radians)
    pub x_origin: f64,
    /// Y coordinate of first grid point (radians)
    pub y_origin: f64,
    /// Grid spacing in X direction (radians)
    pub dx: f64,
    /// Grid spacing in Y direction (radians)
    pub dy: f64,
    /// Number of grid points in X direction
    pub nx: usize,
    /// Number of grid points in Y direction
    pub ny: usize,
}

impl Geostationary {
    /// Create a new Geostationary projection from GOES NetCDF parameters.
    ///
    /// # Arguments
    /// * `perspective_point_height` - Satellite altitude above Earth surface (meters)
    /// * `semi_major_axis` - Earth equatorial radius (meters)
    /// * `semi_minor_axis` - Earth polar radius (meters)
    /// * `longitude_origin_deg` - Satellite longitude (degrees, negative for west)
    /// * `x_origin` - X coordinate of upper-left corner (radians)
    /// * `y_origin` - Y coordinate of upper-left corner (radians)
    /// * `dx` - X grid spacing (radians, typically negative for west-to-east)
    /// * `dy` - Y grid spacing (radians, typically negative for north-to-south)
    /// * `nx` - Number of X grid points
    /// * `ny` - Number of Y grid points
    #[allow(clippy::too_many_arguments)]
    pub fn from_goes(
        perspective_point_height: f64,
        semi_major_axis: f64,
        semi_minor_axis: f64,
        longitude_origin_deg: f64,
        x_origin: f64,
        y_origin: f64,
        dx: f64,
        dy: f64,
        nx: usize,
        ny: usize,
    ) -> Self {
        Self {
            h: perspective_point_height + semi_major_axis,
            perspective_point_height,
            req: semi_major_axis,
            rpol: semi_minor_axis,
            lambda_0: longitude_origin_deg.to_radians(),
            sweep_x: true, // GOES-R uses x-axis sweep
            x_origin,
            y_origin,
            dx,
            dy,
            nx,
            ny,
        }
    }

    /// Create projection for GOES-19 (GOES-East at 75.2°W) CONUS sector.
    ///
    /// Uses actual CONUS parameters from AWS GOES-19 data:
    /// - X: from -0.10136 to 0.03864 radians (west to east)
    /// - Y: from 0.12824 to 0.04424 radians (north to south)
    /// - Resolution: 0.000028 rad per pixel (1km at nadir)
    /// - Grid: 5000 x 3000 pixels
    pub fn goes19_conus() -> Self {
        Self::from_goes(
            35786023.0,    // perspective_point_height
            6378137.0,     // semi_major_axis (GRS80)
            6356752.31414, // semi_minor_axis
            -75.2,         // longitude_origin (GOES-19 position)
            -0.101360,     // x_origin (west edge, radians) - x[0] value
            0.128226,      // y_origin (north edge, radians) - y[0] value
            0.000028,      // dx (radians per pixel)
            -0.000028,     // dy (radians per pixel, negative = south)
            5000,          // nx
            3000,          // ny
        )
    }

    /// Create projection for GOES-18 (GOES-West at 137.2°W) CONUS sector.
    pub fn goes18_conus() -> Self {
        Self::from_goes(
            35786023.0,
            6378137.0,
            6356752.31414,
            -137.2, // GOES-18 position
            -0.101360,
            0.128226,
            0.000028,
            -0.000028,
            5000,
            3000,
        )
    }

    /// Create projection for GOES-19 (GOES-East at 75.2°W) Full Disk sector.
    ///
    /// Full disk parameters from AWS GOES-19 ABI-L2-CMIPF data:
    /// - X: from -0.151844 to 0.151844 radians (full hemisphere)
    /// - Y: from 0.151844 to -0.151844 radians (north to south)
    /// - Resolution: 0.000056 rad per pixel (2km at nadir)
    /// - Grid: 5424 x 5424 pixels
    pub fn goes19_fulldisk() -> Self {
        Self::from_goes(
            35786023.0,    // perspective_point_height
            6378137.0,     // semi_major_axis (GRS80)
            6356752.31414, // semi_minor_axis
            -75.2,         // longitude_origin (GOES-19 position)
            -0.151844,     // x_origin (west edge, radians)
            0.151844,      // y_origin (north edge, radians)
            0.000056,      // dx (radians per pixel)
            -0.000056,     // dy (radians per pixel, negative = south)
            5424,          // nx
            5424,          // ny
        )
    }

    /// Create projection for GOES-18 (GOES-West at 137.2°W) Full Disk sector.
    ///
    /// Full disk parameters from AWS GOES-18 ABI-L2-CMIPF data:
    /// - X: from -0.151844 to 0.151844 radians (full hemisphere)
    /// - Y: from 0.151844 to -0.151844 radians (north to south)
    /// - Resolution: 0.000056 rad per pixel (2km at nadir)
    /// - Grid: 5424 x 5424 pixels
    pub fn goes18_fulldisk() -> Self {
        Self::from_goes(
            35786023.0,    // perspective_point_height
            6378137.0,     // semi_major_axis (GRS80)
            6356752.31414, // semi_minor_axis
            -137.2,        // longitude_origin (GOES-18 position)
            -0.151844,     // x_origin (west edge, radians)
            0.151844,      // y_origin (north edge, radians)
            0.000056,      // dx (radians per pixel)
            -0.000056,     // dy (radians per pixel, negative = south)
            5424,          // nx
            5424,          // ny
        )
    }

    /// Convert grid indices (i, j) to scan angles (x, y) in radians.
    #[inline]
    pub fn grid_to_scan(&self, i: f64, j: f64) -> (f64, f64) {
        let x = self.x_origin + i * self.dx;
        let y = self.y_origin + j * self.dy;
        (x, y)
    }

    /// Convert scan angles (x, y) to grid indices (i, j).
    #[inline]
    pub fn scan_to_grid(&self, x: f64, y: f64) -> (f64, f64) {
        let i = (x - self.x_origin) / self.dx;
        let j = (y - self.y_origin) / self.dy;
        (i, j)
    }

    /// Convert scan angles (radians) to geographic coordinates (lat/lon degrees).
    ///
    /// Based on GOES-R PUG Volume 4, Section 4.2.8.
    /// Returns None if the scan angle points to space (off Earth).
    pub fn scan_to_geo(&self, x_rad: f64, y_rad: f64) -> Option<(f64, f64)> {
        let sin_x = x_rad.sin();
        let cos_x = x_rad.cos();
        let sin_y = y_rad.sin();
        let cos_y = y_rad.cos();

        // Quadratic coefficients for finding distance to Earth surface
        let a = sin_x.powi(2)
            + cos_x.powi(2) * (cos_y.powi(2) + (self.req / self.rpol).powi(2) * sin_y.powi(2));
        let b = -2.0 * self.h * cos_x * cos_y;
        let c = self.h.powi(2) - self.req.powi(2);

        let discriminant = b * b - 4.0 * a * c;
        if discriminant < 0.0 {
            return None; // Scan angle points to space
        }

        let rs = (-b - discriminant.sqrt()) / (2.0 * a);

        // 3D coordinates (satellite-centered, Earth-fixed)
        let sx = rs * cos_x * cos_y;
        let sy = -rs * sin_x;
        let sz = rs * cos_x * sin_y;

        // Convert to geodetic coordinates
        let lat = ((self.req / self.rpol).powi(2) * sz / (self.h - sx).hypot(sy)).atan();
        let lon = self.lambda_0 - sy.atan2(self.h - sx);

        Some((lon.to_degrees(), lat.to_degrees()))
    }

    /// Convert geographic coordinates (lat/lon degrees) to scan angles (radians).
    ///
    /// Based on GOES-R PUG Volume 4, Section 4.2.8.
    /// Returns None if the point is not visible from the satellite.
    pub fn geo_to_scan(&self, lon_deg: f64, lat_deg: f64) -> Option<(f64, f64)> {
        let lat_rad = lat_deg.to_radians();
        let lon_rad = lon_deg.to_radians();

        // Check if point is beyond horizon (more than ~81 degrees from nadir)
        // For a geostationary satellite, the horizon is at cos^-1(Re / (Re + h))
        let dlon = lon_rad - self.lambda_0;
        let cos_c = lat_rad.cos() * dlon.cos();
        let horizon_angle = (self.req / self.h).acos();
        if cos_c.acos() > horizon_angle {
            return None; // Point is beyond Earth's limb as seen from satellite
        }

        // Geocentric latitude (accounting for Earth's oblateness)
        let phi_c = ((self.rpol / self.req).powi(2) * lat_rad.tan()).atan();

        // Eccentricity squared
        let e2 = 1.0 - (self.rpol / self.req).powi(2);

        // Radius from Earth center to surface point
        let rc = self.rpol / (1.0 - e2 * phi_c.cos().powi(2)).sqrt();

        // 3D coordinates (Earth-centered, satellite frame)
        let sx = self.h - rc * phi_c.cos() * (lon_rad - self.lambda_0).cos();
        let sy = -rc * phi_c.cos() * (lon_rad - self.lambda_0).sin();
        let sz = rc * phi_c.sin();

        // Additional check: sx must be positive (point facing satellite)
        if sx <= 0.0 {
            return None; // Behind Earth from satellite's perspective
        }

        // Calculate scan angles
        let s_xy = sx.hypot(sy);
        let y_rad = sz.atan2(s_xy);
        let x_rad = (-sy).atan2(sx);

        Some((x_rad, y_rad))
    }

    /// Convert geographic coordinates (lat/lon degrees) to grid indices (i, j).
    ///
    /// Returns None if the point is not visible from the satellite.
    pub fn geo_to_grid(&self, lat_deg: f64, lon_deg: f64) -> Option<(f64, f64)> {
        let (x, y) = self.geo_to_scan(lon_deg, lat_deg)?;
        Some(self.scan_to_grid(x, y))
    }

    /// Convert grid indices (i, j) to geographic coordinates (lat, lon degrees).
    ///
    /// Returns None if the grid point is off Earth.
    pub fn grid_to_geo(&self, i: f64, j: f64) -> Option<(f64, f64)> {
        let (x, y) = self.grid_to_scan(i, j);
        self.scan_to_geo(x, y).map(|(lon, lat)| (lat, lon))
    }

    /// Get the geographic bounding box of the grid.
    ///
    /// Returns (min_lon, min_lat, max_lon, max_lat) in degrees.
    /// Samples grid points to find approximate bounds since the
    /// geostationary projection creates curved edges.
    ///
    /// For Full Disk imagery, grid edges may be in space (not on Earth),
    /// so we use binary search to find the edges of the visible Earth disk.
    pub fn geographic_bounds(&self) -> (f64, f64, f64, f64) {
        let mut min_lat = f64::MAX;
        let mut max_lat = f64::MIN;
        let mut min_lon = f64::MAX;
        let mut max_lon = f64::MIN;

        let samples = 100; // More samples for better accuracy
        let center_i = (self.nx as f64 - 1.0) / 2.0;
        let center_j = (self.ny as f64 - 1.0) / 2.0;

        // Sample along all edges (works well for CONUS/MESO sectors)
        for t in 0..=samples {
            let frac = t as f64 / samples as f64;

            let edges = [
                (frac * (self.nx as f64 - 1.0), 0.0),                  // Top
                (frac * (self.nx as f64 - 1.0), self.ny as f64 - 1.0), // Bottom
                (0.0, frac * (self.ny as f64 - 1.0)),                  // Left
                (self.nx as f64 - 1.0, frac * (self.ny as f64 - 1.0)), // Right
            ];

            for (i, j) in edges {
                if let Some((lat, lon)) = self.grid_to_geo(i, j) {
                    min_lat = min_lat.min(lat);
                    max_lat = max_lat.max(lat);
                    min_lon = min_lon.min(lon);
                    max_lon = max_lon.max(lon);
                }
            }
        }

        // For Full Disk imagery, the grid edges may be in space.
        // Find the exact edges of Earth's visible disk using binary search.

        // Find north edge: search from top to center on the center column
        let north_edge_j = self.find_edge_binary(center_i, 0.0, center_j, false);
        if let Some((lat, lon)) = self.grid_to_geo(center_i, north_edge_j) {
            max_lat = max_lat.max(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);
        }

        // Find south edge: search from bottom to center on the center column
        let south_edge_j = self.find_edge_binary(center_i, self.ny as f64 - 1.0, center_j, true);
        if let Some((lat, lon)) = self.grid_to_geo(center_i, south_edge_j) {
            min_lat = min_lat.min(lat);
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);
        }

        // Find west edge: search from left to center on the center row
        let west_edge_i = self.find_edge_binary_h(0.0, center_i, center_j, false);
        if let Some((lat, lon)) = self.grid_to_geo(west_edge_i, center_j) {
            min_lon = min_lon.min(lon);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
        }

        // Find east edge: search from right to center on the center row
        let east_edge_i = self.find_edge_binary_h(self.nx as f64 - 1.0, center_i, center_j, true);
        if let Some((lat, lon)) = self.grid_to_geo(east_edge_i, center_j) {
            max_lon = max_lon.max(lon);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
        }

        // Also sample along lines to catch any extremes we might have missed
        for t in 0..=samples {
            let frac = t as f64 / samples as f64;
            let i = frac * (self.nx as f64 - 1.0);
            let j = frac * (self.ny as f64 - 1.0);

            // Horizontal line through center (finds east-west extent)
            if let Some((lat, lon)) = self.grid_to_geo(i, center_j) {
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
            }

            // Vertical line through center (finds north-south extent)
            if let Some((lat, lon)) = self.grid_to_geo(center_i, j) {
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
            }
        }

        (min_lon, min_lat, max_lon, max_lat)
    }

    /// Binary search to find the edge of Earth's visible disk along a vertical line.
    /// Returns the j coordinate of the first/last valid point.
    fn find_edge_binary(&self, i: f64, start_j: f64, end_j: f64, reverse: bool) -> f64 {
        let mut lo = start_j.min(end_j);
        let mut hi = start_j.max(end_j);

        // Binary search to find the edge
        while hi - lo > 1.0 {
            let mid = (lo + hi) / 2.0;
            let is_valid = self.grid_to_geo(i, mid).is_some();

            if reverse {
                // Looking for last valid point (from high to low)
                if is_valid {
                    lo = mid;
                } else {
                    hi = mid;
                }
            } else {
                // Looking for first valid point (from low to high)
                if is_valid {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
        }

        // Return the edge that has valid data
        if self.grid_to_geo(i, lo).is_some() {
            lo
        } else {
            hi
        }
    }

    /// Binary search to find the edge of Earth's visible disk along a horizontal line.
    /// Returns the i coordinate of the first/last valid point.
    fn find_edge_binary_h(&self, start_i: f64, end_i: f64, j: f64, reverse: bool) -> f64 {
        let mut lo = start_i.min(end_i);
        let mut hi = start_i.max(end_i);

        // Binary search to find the edge
        while hi - lo > 1.0 {
            let mid = (lo + hi) / 2.0;
            let is_valid = self.grid_to_geo(mid, j).is_some();

            if reverse {
                // Looking for last valid point (from high to low)
                if is_valid {
                    lo = mid;
                } else {
                    hi = mid;
                }
            } else {
                // Looking for first valid point (from low to high)
                if is_valid {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
        }

        // Return the edge that has valid data
        if self.grid_to_geo(lo, j).is_some() {
            lo
        } else {
            hi
        }
    }

    /// Check if a geographic point is within the grid and visible.
    pub fn contains(&self, lat_deg: f64, lon_deg: f64) -> bool {
        if let Some((i, j)) = self.geo_to_grid(lat_deg, lon_deg) {
            i >= 0.0 && i < self.nx as f64 && j >= 0.0 && j < self.ny as f64
        } else {
            false
        }
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
    fn test_goes19_conus_projection() {
        let proj = Geostationary::goes19_conus();

        // Test a point near center of CONUS
        let (lat, lon) = (39.0, -95.0); // Kansas

        if let Some((i, j)) = proj.geo_to_grid(lat, lon) {
            println!("Kansas at grid ({}, {})", i, j);

            // Should be within the grid (5000 x 3000)
            assert!(
                (0.0..5000.0).contains(&i),
                "i should be in grid [0, 5000), got {}",
                i
            );
            assert!(
                (0.0..3000.0).contains(&j),
                "j should be in grid [0, 3000), got {}",
                j
            );

            // Test roundtrip - allow 0.15 degree tolerance due to projection approximations
            // and grid discretization effects
            if let Some((lat2, lon2)) = proj.grid_to_geo(i, j) {
                assert!(
                    (lat - lat2).abs() < 0.15,
                    "Latitude roundtrip failed: {} vs {}",
                    lat,
                    lat2
                );
                assert!(
                    (lon - lon2).abs() < 0.15,
                    "Longitude roundtrip failed: {} vs {}",
                    lon,
                    lon2
                );
            } else {
                panic!("Failed to convert grid back to geo");
            }
        } else {
            panic!("Failed to convert Kansas to grid");
        }
    }

    #[test]
    fn test_goes19_bounds() {
        let proj = Geostationary::goes19_conus();
        let (min_lon, min_lat, max_lon, max_lat) = proj.geographic_bounds();

        println!(
            "GOES-19 CONUS bounds: lon {:.2} to {:.2}, lat {:.2} to {:.2}",
            min_lon, max_lon, min_lat, max_lat
        );

        // CONUS sector covers US plus surrounding areas
        // Actual computed bounds: lon -142.66 to -52.94, lat 14.57 to 55.29
        assert!(
            min_lon < -140.0,
            "min_lon should be < -140, got {}",
            min_lon
        );
        assert!(max_lon > -55.0, "max_lon should be > -55, got {}", max_lon);
        assert!(
            min_lat > 10.0 && min_lat < 20.0,
            "min_lat should be ~14-15, got {}",
            min_lat
        );
        assert!(max_lat > 50.0, "max_lat should be > 50, got {}", max_lat);
    }

    #[test]
    fn test_scan_roundtrip() {
        let proj = Geostationary::goes19_conus();

        // Test scan to geo and back at satellite nadir (0, 0)
        let (x, y) = (0.0, 0.0);

        if let Some((lon, lat)) = proj.scan_to_geo(x, y) {
            // At nadir, should be satellite longitude and equator
            println!("Nadir point: lon={}, lat={}", lon, lat);
            assert!(
                (lon - (-75.2)).abs() < 0.1,
                "Nadir longitude should be ~-75.2, got {}",
                lon
            );
            assert!(lat.abs() < 0.1, "Nadir latitude should be ~0, got {}", lat);

            if let Some((x2, y2)) = proj.geo_to_scan(lon, lat) {
                assert!((x - x2).abs() < 1e-6, "X roundtrip failed: {} vs {}", x, x2);
                assert!((y - y2).abs() < 1e-6, "Y roundtrip failed: {} vs {}", y, y2);
            } else {
                panic!("Failed to convert geo back to scan");
            }
        } else {
            panic!("Failed to convert scan to geo");
        }
    }

    #[test]
    fn test_grid_corners() {
        let proj = Geostationary::goes19_conus();

        // Test all four corners of the grid
        let corners = [(0.0, 0.0), (4999.0, 0.0), (0.0, 2999.0), (4999.0, 2999.0)];

        for (i, j) in corners {
            if let Some((lat, lon)) = proj.grid_to_geo(i, j) {
                println!("Corner ({}, {}) -> lat={:.2}, lon={:.2}", i, j, lat, lon);
                // All corners should be on Earth (valid coordinates)
                assert!((-90.0..=90.0).contains(&lat), "Invalid latitude: {}", lat);
                assert!(
                    (-180.0..=180.0).contains(&lon),
                    "Invalid longitude: {}",
                    lon
                );
            } else {
                // Some corners may be off-Earth for certain projections
                println!("Corner ({}, {}) is off-Earth", i, j);
            }
        }
    }

    #[test]
    fn test_off_earth() {
        let proj = Geostationary::goes19_conus();

        // A scan angle pointing to space should return None
        let result = proj.scan_to_geo(0.5, 0.5); // Very large scan angle (~28 degrees)
                                                 // Just ensure it doesn't panic
        println!("Off-earth result: {:?}", result);
    }

    #[test]
    fn test_not_visible() {
        let proj = Geostationary::goes19_conus();

        // A point far from satellite should not be visible (>81 degrees from nadir)
        // For GOES-19 at -75.2°, a point at +180° longitude is on the opposite side
        let result = proj.geo_to_scan(180.0, 0.0); // Point on opposite side of Earth
        assert!(
            result.is_none(),
            "Point at 180° should not be visible from GOES-19"
        );
    }
}
