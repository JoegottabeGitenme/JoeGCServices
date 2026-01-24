//! Coordinate Reference System (CRS) types and transformations for EDR.
//!
//! This module provides CRS parsing and coordinate transformation functions
//! for supporting different output projections in EDR responses.
//!
//! # Supported CRS
//!
//! - **CRS:84** (default): WGS84 with lon/lat axis order
//! - **EPSG:4326**: WGS84 with lat/lon axis order (same coordinates, different semantics)
//! - **EPSG:3857**: Web Mercator in meters
//!
//! # Example
//!
//! ```rust
//! use edr_protocol::crs::{OutputCrs, wgs84_to_mercator};
//!
//! let crs = OutputCrs::from_str("EPSG:3857").unwrap();
//! let (x, y) = wgs84_to_mercator(-97.5, 35.2);
//! println!("Web Mercator: ({}, {})", x, y);
//! ```

use std::f64::consts::PI;
use std::fmt;

/// Supported output coordinate reference systems for EDR responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputCrs {
    /// WGS84 Geographic with lon/lat axis order (OGC CRS:84).
    /// This is the default and native CRS for EDR.
    #[default]
    Crs84,

    /// WGS84 Geographic with lat/lon axis order (EPSG:4326).
    /// Same coordinates as CRS:84, but with different axis order semantics.
    Epsg4326,

    /// Web Mercator projection in meters (EPSG:3857).
    /// Used by web mapping libraries (Leaflet, OpenLayers, Mapbox).
    Epsg3857,
}

impl OutputCrs {
    /// Parse a CRS string into an OutputCrs.
    ///
    /// Accepts formats like:
    /// - "CRS:84", "crs:84"
    /// - "EPSG:4326", "epsg:4326"
    /// - "EPSG:3857", "epsg:3857"
    /// - OGC URIs: "http://www.opengis.net/def/crs/OGC/1.3/CRS84"
    /// - OGC URIs: "http://www.opengis.net/def/crs/EPSG/0/4326"
    pub fn from_str(s: &str) -> Result<Self, CrsError> {
        let normalized = s.trim().to_uppercase();

        // Handle short codes
        match normalized.as_str() {
            "CRS:84" | "OGC:CRS84" => return Ok(OutputCrs::Crs84),
            "EPSG:4326" => return Ok(OutputCrs::Epsg4326),
            "EPSG:3857" | "EPSG:900913" | "EPSG:102100" => return Ok(OutputCrs::Epsg3857),
            _ => {}
        }

        // Handle OGC URIs
        if s.contains("opengis.net/def/crs") {
            if s.contains("CRS84") || s.contains("crs84") {
                return Ok(OutputCrs::Crs84);
            }
            if s.contains("/4326") {
                return Ok(OutputCrs::Epsg4326);
            }
            if s.contains("/3857") {
                return Ok(OutputCrs::Epsg3857);
            }
        }

        Err(CrsError::UnsupportedCrs(s.to_string()))
    }

    /// Get the OGC URI for this CRS.
    pub fn uri(&self) -> &'static str {
        match self {
            OutputCrs::Crs84 => "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
            OutputCrs::Epsg4326 => "http://www.opengis.net/def/crs/EPSG/0/4326",
            OutputCrs::Epsg3857 => "http://www.opengis.net/def/crs/EPSG/0/3857",
        }
    }

    /// Get the short code for this CRS (e.g., "CRS:84", "EPSG:4326").
    pub fn code(&self) -> &'static str {
        match self {
            OutputCrs::Crs84 => "CRS:84",
            OutputCrs::Epsg4326 => "EPSG:4326",
            OutputCrs::Epsg3857 => "EPSG:3857",
        }
    }

    /// Check if this CRS uses geographic (degree) coordinates.
    pub fn is_geographic(&self) -> bool {
        matches!(self, OutputCrs::Crs84 | OutputCrs::Epsg4326)
    }

    /// Check if this CRS uses projected (meter) coordinates.
    pub fn is_projected(&self) -> bool {
        matches!(self, OutputCrs::Epsg3857)
    }

    /// Get the CRS type name for CoverageJSON referencing.
    pub fn crs_type(&self) -> &'static str {
        match self {
            OutputCrs::Crs84 | OutputCrs::Epsg4326 => "GeographicCRS",
            OutputCrs::Epsg3857 => "ProjectedCRS",
        }
    }
}

impl fmt::Display for OutputCrs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Errors that can occur when parsing or using CRS.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CrsError {
    /// The requested CRS is not supported.
    #[error("Unsupported CRS: {0}. Supported: CRS:84, EPSG:4326, EPSG:3857")]
    UnsupportedCrs(String),
}

/// Semi-major axis of WGS84 ellipsoid in meters.
const WGS84_SEMI_MAJOR_AXIS: f64 = 6378137.0;

/// Maximum extent of Web Mercator in meters (at ±180° longitude).
const MERCATOR_MAX_EXTENT: f64 = 20037508.342789244;

/// Convert WGS84 geographic coordinates (lon/lat in degrees) to Web Mercator (EPSG:3857).
///
/// # Arguments
///
/// * `lon` - Longitude in degrees (-180 to 180)
/// * `lat` - Latitude in degrees (-85.06 to 85.06, clamped if outside)
///
/// # Returns
///
/// Tuple of (x, y) in meters.
///
/// # Example
///
/// ```rust
/// use edr_protocol::crs::wgs84_to_mercator;
///
/// // New York City
/// let (x, y) = wgs84_to_mercator(-74.006, 40.7128);
/// assert!((x - -8238310.0).abs() < 1.0);
/// assert!((y - 4970072.0).abs() < 1.0);
/// ```
pub fn wgs84_to_mercator(lon: f64, lat: f64) -> (f64, f64) {
    // Clamp latitude to Web Mercator valid range (avoids infinity at poles)
    let lat_clamped = lat.clamp(-85.06, 85.06);

    // X coordinate: simple linear scaling
    let x = lon * MERCATOR_MAX_EXTENT / 180.0;

    // Y coordinate: Mercator projection formula
    let lat_rad = lat_clamped * PI / 180.0;
    let y = (((PI / 4.0) + (lat_rad / 2.0)).tan().ln()) * WGS84_SEMI_MAJOR_AXIS;

    (x, y)
}

/// Convert Web Mercator (EPSG:3857) coordinates to WGS84 geographic (lon/lat in degrees).
///
/// # Arguments
///
/// * `x` - X coordinate in meters
/// * `y` - Y coordinate in meters
///
/// # Returns
///
/// Tuple of (longitude, latitude) in degrees.
///
/// # Example
///
/// ```rust
/// use edr_protocol::crs::mercator_to_wgs84;
///
/// // New York City
/// let (lon, lat) = mercator_to_wgs84(-8238310.0, 4970072.0);
/// assert!((lon - -74.006).abs() < 0.001);
/// assert!((lat - 40.7128).abs() < 0.001);
/// ```
pub fn mercator_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    // Longitude: simple linear scaling
    let lon = x * 180.0 / MERCATOR_MAX_EXTENT;

    // Latitude: inverse Mercator projection
    let lat_rad = 2.0 * ((y / WGS84_SEMI_MAJOR_AXIS).exp().atan()) - (PI / 2.0);
    let lat = lat_rad * 180.0 / PI;

    (lon, lat)
}

/// Transform a single coordinate pair from WGS84 to the target CRS.
///
/// For CRS:84 and EPSG:4326, coordinates are returned unchanged.
/// For EPSG:3857, coordinates are projected to Web Mercator.
pub fn transform_point(lon: f64, lat: f64, target_crs: OutputCrs) -> (f64, f64) {
    match target_crs {
        OutputCrs::Crs84 | OutputCrs::Epsg4326 => (lon, lat),
        OutputCrs::Epsg3857 => wgs84_to_mercator(lon, lat),
    }
}

/// Transform a vector of longitude values from WGS84 to the target CRS.
///
/// For geographic CRS, values are returned unchanged.
/// For EPSG:3857, values are converted to meters.
pub fn transform_x_values(lons: &[f64], target_crs: OutputCrs) -> Vec<f64> {
    match target_crs {
        OutputCrs::Crs84 | OutputCrs::Epsg4326 => lons.to_vec(),
        OutputCrs::Epsg3857 => lons
            .iter()
            .map(|&lon| lon * MERCATOR_MAX_EXTENT / 180.0)
            .collect(),
    }
}

/// Transform a vector of latitude values from WGS84 to the target CRS.
///
/// For geographic CRS, values are returned unchanged.
/// For EPSG:3857, values are converted to meters.
pub fn transform_y_values(lats: &[f64], target_crs: OutputCrs) -> Vec<f64> {
    match target_crs {
        OutputCrs::Crs84 | OutputCrs::Epsg4326 => lats.to_vec(),
        OutputCrs::Epsg3857 => lats
            .iter()
            .map(|&lat| wgs84_to_mercator(0.0, lat).1)
            .collect(),
    }
}

/// Get the list of CRS URIs supported by the EDR API.
pub fn supported_crs_uris() -> Vec<&'static str> {
    vec![
        OutputCrs::Crs84.uri(),
        OutputCrs::Epsg4326.uri(),
        OutputCrs::Epsg3857.uri(),
    ]
}

/// Get the list of short CRS codes supported by the EDR API.
///
/// Returns codes like "CRS:84", "EPSG:4326", "EPSG:3857" instead of full OGC URIs.
/// Per OGC EDR spec, the CRS name "MAY be an EPSG code" - short codes are valid.
pub fn supported_crs_codes() -> Vec<&'static str> {
    vec![
        OutputCrs::Crs84.code(),
        OutputCrs::Epsg4326.code(),
        OutputCrs::Epsg3857.code(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_crs_short_codes() {
        assert_eq!(OutputCrs::from_str("CRS:84").unwrap(), OutputCrs::Crs84);
        assert_eq!(OutputCrs::from_str("crs:84").unwrap(), OutputCrs::Crs84);
        assert_eq!(
            OutputCrs::from_str("EPSG:4326").unwrap(),
            OutputCrs::Epsg4326
        );
        assert_eq!(
            OutputCrs::from_str("epsg:4326").unwrap(),
            OutputCrs::Epsg4326
        );
        assert_eq!(
            OutputCrs::from_str("EPSG:3857").unwrap(),
            OutputCrs::Epsg3857
        );
        assert_eq!(
            OutputCrs::from_str("EPSG:900913").unwrap(),
            OutputCrs::Epsg3857
        );
    }

    #[test]
    fn test_parse_crs_uris() {
        assert_eq!(
            OutputCrs::from_str("http://www.opengis.net/def/crs/OGC/1.3/CRS84").unwrap(),
            OutputCrs::Crs84
        );
        assert_eq!(
            OutputCrs::from_str("http://www.opengis.net/def/crs/EPSG/0/4326").unwrap(),
            OutputCrs::Epsg4326
        );
        assert_eq!(
            OutputCrs::from_str("http://www.opengis.net/def/crs/EPSG/0/3857").unwrap(),
            OutputCrs::Epsg3857
        );
    }

    #[test]
    fn test_parse_crs_invalid() {
        assert!(OutputCrs::from_str("EPSG:99999").is_err());
        assert!(OutputCrs::from_str("invalid").is_err());
    }

    #[test]
    fn test_crs_uri() {
        assert_eq!(
            OutputCrs::Crs84.uri(),
            "http://www.opengis.net/def/crs/OGC/1.3/CRS84"
        );
        assert_eq!(
            OutputCrs::Epsg4326.uri(),
            "http://www.opengis.net/def/crs/EPSG/0/4326"
        );
        assert_eq!(
            OutputCrs::Epsg3857.uri(),
            "http://www.opengis.net/def/crs/EPSG/0/3857"
        );
    }

    #[test]
    fn test_wgs84_to_mercator_origin() {
        let (x, y) = wgs84_to_mercator(0.0, 0.0);
        assert!((x - 0.0).abs() < 0.001);
        assert!((y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_wgs84_to_mercator_known_point() {
        // New York City: -74.006, 40.7128
        let (x, y) = wgs84_to_mercator(-74.006, 40.7128);
        // Expected approximately: -8238310, 4970072
        assert!((x - -8238310.0).abs() < 10.0);
        assert!((y - 4970072.0).abs() < 10.0);
    }

    #[test]
    fn test_wgs84_to_mercator_extremes() {
        // At max longitude
        let (x, _) = wgs84_to_mercator(180.0, 0.0);
        assert!((x - MERCATOR_MAX_EXTENT).abs() < 1.0);

        // At min longitude
        let (x, _) = wgs84_to_mercator(-180.0, 0.0);
        assert!((x - -MERCATOR_MAX_EXTENT).abs() < 1.0);
    }

    #[test]
    fn test_mercator_to_wgs84_roundtrip() {
        let test_points = [
            (0.0, 0.0),
            (-74.006, 40.7128),  // NYC
            (139.6917, 35.6895), // Tokyo
            (-97.5, 35.2),       // Oklahoma
        ];

        for (lon, lat) in test_points {
            let (x, y) = wgs84_to_mercator(lon, lat);
            let (lon2, lat2) = mercator_to_wgs84(x, y);
            assert!(
                (lon - lon2).abs() < 0.0001,
                "Longitude mismatch: {} vs {}",
                lon,
                lon2
            );
            assert!(
                (lat - lat2).abs() < 0.0001,
                "Latitude mismatch: {} vs {}",
                lat,
                lat2
            );
        }
    }

    #[test]
    fn test_transform_point() {
        let (x, y) = transform_point(-97.5, 35.2, OutputCrs::Crs84);
        assert_eq!(x, -97.5);
        assert_eq!(y, 35.2);

        let (x, y) = transform_point(-97.5, 35.2, OutputCrs::Epsg3857);
        // Expected: x = -10853650.4, y = 4191093.7
        assert!((x - -10853650.0).abs() < 10.0);
        assert!((y - 4191094.0).abs() < 10.0);
    }

    #[test]
    fn test_transform_x_values() {
        let lons = vec![-97.5, -97.4, -97.3];

        // Geographic - unchanged
        let result = transform_x_values(&lons, OutputCrs::Crs84);
        assert_eq!(result, lons);

        // Mercator - converted to meters
        let result = transform_x_values(&lons, OutputCrs::Epsg3857);
        assert!(result[0] < 0.0); // Negative meters for west longitude
        assert!(result[1] > result[0]); // Increasing eastward
    }

    #[test]
    fn test_transform_y_values() {
        let lats = vec![35.0, 35.1, 35.2];

        // Geographic - unchanged
        let result = transform_y_values(&lats, OutputCrs::Crs84);
        assert_eq!(result, lats);

        // Mercator - converted to meters
        let result = transform_y_values(&lats, OutputCrs::Epsg3857);
        assert!(result[0] > 0.0); // Positive meters for north latitude
        assert!(result[1] > result[0]); // Increasing northward
    }

    #[test]
    fn test_crs_type() {
        assert_eq!(OutputCrs::Crs84.crs_type(), "GeographicCRS");
        assert_eq!(OutputCrs::Epsg4326.crs_type(), "GeographicCRS");
        assert_eq!(OutputCrs::Epsg3857.crs_type(), "ProjectedCRS");
    }

    #[test]
    fn test_is_geographic_projected() {
        assert!(OutputCrs::Crs84.is_geographic());
        assert!(OutputCrs::Epsg4326.is_geographic());
        assert!(!OutputCrs::Epsg3857.is_geographic());

        assert!(!OutputCrs::Crs84.is_projected());
        assert!(!OutputCrs::Epsg4326.is_projected());
        assert!(OutputCrs::Epsg3857.is_projected());
    }
}
