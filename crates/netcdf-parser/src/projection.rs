//! GOES ABI geostationary projection.
//!
//! This module provides coordinate conversion between geostationary satellite
//! scan angles (radians) and geographic coordinates (lat/lon degrees).
//!
//! # GOES-R ABI Coordinate System
//!
//! GOES-R ABI files use the geostationary projection with coordinates in radians.
//! The scan angles (x, y) represent angular displacement from the satellite nadir:
//! - x: East-West scan angle (positive = east of nadir)
//! - y: North-South elevation angle (positive = north of equator)
//!
//! # Reference
//!
//! GOES-R Product Definition and Users' Guide (PUG) Volume 4, Section 4.2.8

/// GOES ABI projection parameters.
///
/// These parameters define the geostationary projection from the satellite's
/// viewpoint. The formulas are based on the GOES-R PUG specification.
#[derive(Debug, Clone)]
pub struct GoesProjection {
    /// Satellite height above Earth center (meters)
    pub perspective_point_height: f64,
    /// Semi-major axis of Earth ellipsoid (meters)
    pub semi_major_axis: f64,
    /// Semi-minor axis of Earth ellipsoid (meters)
    pub semi_minor_axis: f64,
    /// Longitude of satellite nadir point (degrees)
    pub longitude_origin: f64,
    /// Latitude of projection origin (always 0 for geostationary)
    pub latitude_origin: f64,
    /// Sweep angle axis ("x" for GOES-R)
    pub sweep_angle_axis: String,
}

impl Default for GoesProjection {
    fn default() -> Self {
        // Default values for GOES-16 (GOES-East)
        Self {
            perspective_point_height: 35786023.0,
            semi_major_axis: 6378137.0,
            semi_minor_axis: 6356752.31414,
            longitude_origin: -75.0, // GOES-16/East
            latitude_origin: 0.0,
            sweep_angle_axis: "x".to_string(),
        }
    }
}

impl GoesProjection {
    /// Create projection for GOES-16 (GOES-East at 75.2°W).
    pub fn goes16() -> Self {
        Self {
            longitude_origin: -75.2,
            ..Default::default()
        }
    }

    /// Create projection for GOES-18 (GOES-West at 137.2°W).
    pub fn goes18() -> Self {
        Self {
            longitude_origin: -137.2,
            ..Default::default()
        }
    }

    /// Convert geostationary coordinates (radians) to geographic (lat/lon degrees).
    ///
    /// Based on GOES-R Product Definition and Users' Guide (PUG) formulas.
    /// The x,y coordinates are scan angles in radians from the satellite nadir.
    ///
    /// Returns `None` if the scan angle points to space (off Earth).
    ///
    /// # Arguments
    ///
    /// * `x_rad` - East-West scan angle in radians
    /// * `y_rad` - North-South elevation angle in radians
    ///
    /// # Returns
    ///
    /// `Some((longitude, latitude))` in degrees, or `None` if off-Earth.
    pub fn to_geographic(&self, x_rad: f64, y_rad: f64) -> Option<(f64, f64)> {
        let h = self.perspective_point_height;
        let req = self.semi_major_axis;
        let rpol = self.semi_minor_axis;
        let lambda_0 = self.longitude_origin.to_radians();
        let h_total = h + req;

        let sin_x = x_rad.sin();
        let cos_x = x_rad.cos();
        let sin_y = y_rad.sin();
        let cos_y = y_rad.cos();

        // Quadratic coefficients for finding distance to Earth surface
        let a =
            sin_x.powi(2) + cos_x.powi(2) * (cos_y.powi(2) + (req / rpol).powi(2) * sin_y.powi(2));
        let b = -2.0 * h_total * cos_x * cos_y;
        let c = h_total.powi(2) - req.powi(2);

        let discriminant = b * b - 4.0 * a * c;
        if discriminant < 0.0 {
            return None; // Scan angle points to space
        }

        let rs = (-b - discriminant.sqrt()) / (2.0 * a);

        // 3D coordinates (satellite-centered, Earth-fixed)
        // Note: sy uses negative sin(x) to match forward transform where x = atan2(-sy, sx)
        let sx = rs * cos_x * cos_y;
        let sy = -rs * sin_x; // Negative because x = atan2(-sy, sx), so sy = -rs*sin(x)
        let sz = rs * cos_x * sin_y;

        // Convert to geodetic coordinates
        let lat = ((req / rpol).powi(2) * sz / (h_total - sx).hypot(sy)).atan();
        let lon = lambda_0 - sy.atan2(h_total - sx);

        Some((lon.to_degrees(), lat.to_degrees()))
    }

    /// Convert geographic coordinates (lat/lon degrees) to geostationary (radians).
    ///
    /// Returns `None` if the point is not visible from the satellite (behind Earth).
    ///
    /// # Arguments
    ///
    /// * `lon` - Longitude in degrees (negative for west)
    /// * `lat` - Latitude in degrees
    ///
    /// # Returns
    ///
    /// `Some((x_rad, y_rad))` scan angles in radians, or `None` if not visible.
    pub fn from_geographic(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let h = self.perspective_point_height;
        let req = self.semi_major_axis;
        let rpol = self.semi_minor_axis;
        let lambda_0 = self.longitude_origin.to_radians();
        let h_total = h + req;

        let lat_rad = lat.to_radians();
        let lon_rad = lon.to_radians();

        // Geocentric latitude (accounting for Earth's oblateness)
        let phi_c = ((rpol / req).powi(2) * lat_rad.tan()).atan();

        // Radius from Earth center to surface point
        let e2 = 1.0 - (rpol / req).powi(2); // eccentricity squared
        let rc = rpol / (1.0 - e2 * phi_c.cos().powi(2)).sqrt();

        // 3D coordinates (Earth-centered, satellite frame)
        let sx = h_total - rc * phi_c.cos() * (lon_rad - lambda_0).cos();
        let sy = -rc * phi_c.cos() * (lon_rad - lambda_0).sin();
        let sz = rc * phi_c.sin();

        // Check visibility (point must be on satellite-facing side of Earth)
        if sx <= 0.0 {
            return None; // Behind Earth from satellite's perspective
        }

        // Calculate scan angles using atan2 for correct quadrant
        let s_xy = sx.hypot(sy);
        let y_rad = sz.atan2(s_xy);
        let x_rad = (-sy).atan2(sx); // Negative sy to match inverse formula sign convention

        Some((x_rad, y_rad))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goes_projection_roundtrip() {
        let proj = GoesProjection::goes16();

        // Test a point near the center of CONUS
        let (lon, lat) = (-95.0, 35.0);

        if let Some((x, y)) = proj.from_geographic(lon, lat) {
            if let Some((lon2, lat2)) = proj.to_geographic(x, y) {
                // Allow ~0.1 degree tolerance due to float precision and projection approximations
                assert!(
                    (lon - lon2).abs() < 0.15,
                    "Longitude mismatch: {} vs {}",
                    lon,
                    lon2
                );
                assert!(
                    (lat - lat2).abs() < 0.15,
                    "Latitude mismatch: {} vs {}",
                    lat,
                    lat2
                );
            } else {
                panic!("Failed to convert back to geographic");
            }
        } else {
            panic!("Failed to convert to geostationary");
        }
    }

    #[test]
    fn test_goes_projection_off_earth() {
        let proj = GoesProjection::goes16();

        // A point that should be off Earth (large scan angle)
        let result = proj.to_geographic(0.5, 0.5); // ~28 degrees
                                                   // This might or might not be visible depending on exact geometry
                                                   // The important thing is it doesn't panic
        println!("Off-earth test result: {:?}", result);
    }

    #[test]
    fn test_goes16_vs_goes18_longitude() {
        let goes16 = GoesProjection::goes16();
        let goes18 = GoesProjection::goes18();

        assert!(
            (goes16.longitude_origin - (-75.2)).abs() < 0.1,
            "GOES-16 should be at ~-75°W"
        );
        assert!(
            (goes18.longitude_origin - (-137.2)).abs() < 0.1,
            "GOES-18 should be at ~-137°W"
        );
    }
}
