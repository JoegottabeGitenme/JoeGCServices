//! Coordinate Reference System types and utilities.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Well-known CRS codes supported by the WMS server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CrsCode {
    /// WGS84 Geographic (lat/lon in degrees)
    Epsg4326,
    /// Web Mercator (meters)
    Epsg3857,
    /// NAD83 Geographic
    Epsg4269,
    /// Lambert Conformal Conic (CONUS)
    Epsg5070,
    /// Polar Stereographic North
    Epsg3413,
    /// Polar Stereographic South
    Epsg3031,
}

impl CrsCode {
    /// Parse a CRS string from WMS request (supports both SRS and CRS parameter formats).
    ///
    /// Accepts formats like:
    /// - "EPSG:4326"
    /// - "epsg:4326"
    /// - "CRS:84" (equivalent to EPSG:4326 with lon/lat axis order)
    pub fn from_wms_string(s: &str) -> Result<Self, CrsParseError> {
        let normalized = s.to_uppercase();

        match normalized.as_str() {
            "EPSG:4326" | "CRS:84" => Ok(CrsCode::Epsg4326),
            "EPSG:3857" | "EPSG:900913" => Ok(CrsCode::Epsg3857),
            "EPSG:4269" => Ok(CrsCode::Epsg4269),
            "EPSG:5070" => Ok(CrsCode::Epsg5070),
            "EPSG:3413" => Ok(CrsCode::Epsg3413),
            "EPSG:3031" => Ok(CrsCode::Epsg3031),
            _ => Err(CrsParseError::UnsupportedCrs(s.to_string())),
        }
    }

    /// Get the axis order for this CRS in WMS 1.3.0.
    ///
    /// WMS 1.3.0 uses the "natural" axis order of the CRS:
    /// - Geographic CRS: lat, lon (y, x)
    /// - Projected CRS: easting, northing (x, y)
    pub fn axis_order_wms_1_3(&self) -> AxisOrder {
        match self {
            CrsCode::Epsg4326 | CrsCode::Epsg4269 => AxisOrder::LatLon,
            _ => AxisOrder::XY,
        }
    }

    /// Get the axis order for WMS 1.1.1 (always x, y regardless of CRS).
    pub fn axis_order_wms_1_1(&self) -> AxisOrder {
        AxisOrder::XY
    }

    /// Check if this is a geographic (lat/lon) CRS.
    pub fn is_geographic(&self) -> bool {
        matches!(self, CrsCode::Epsg4326 | CrsCode::Epsg4269)
    }
}

impl fmt::Display for CrsCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            CrsCode::Epsg4326 => "EPSG:4326",
            CrsCode::Epsg3857 => "EPSG:3857",
            CrsCode::Epsg4269 => "EPSG:4269",
            CrsCode::Epsg5070 => "EPSG:5070",
            CrsCode::Epsg3413 => "EPSG:3413",
            CrsCode::Epsg3031 => "EPSG:3031",
        };
        write!(f, "{}", code)
    }
}

/// Axis order for coordinate interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisOrder {
    /// X (longitude/easting), Y (latitude/northing)
    XY,
    /// Y (latitude/northing), X (longitude/easting)
    LatLon,
}

/// Full CRS definition with projection parameters.
///
/// This will be expanded to include full projection math.
#[derive(Debug, Clone)]
pub struct Crs {
    pub code: CrsCode,
    // TODO: Add projection parameters for transformation
}

impl Crs {
    pub fn new(code: CrsCode) -> Self {
        Self { code }
    }

    /// Get the valid bounds for this CRS.
    pub fn valid_bounds(&self) -> crate::BoundingBox {
        use crate::BoundingBox;

        match self.code {
            CrsCode::Epsg4326 | CrsCode::Epsg4269 => BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
            CrsCode::Epsg3857 => {
                // Web Mercator bounds (approx ±85.06° latitude)
                let max_extent = 20037508.342789244;
                BoundingBox::new(-max_extent, -max_extent, max_extent, max_extent)
            }
            CrsCode::Epsg5070 => {
                // CONUS Albers Equal Area - approximate bounds in meters
                BoundingBox::new(-2500000.0, -2500000.0, 2500000.0, 2500000.0)
            }
            CrsCode::Epsg3413 | CrsCode::Epsg3031 => {
                // Polar stereographic - approximate bounds
                BoundingBox::new(-4000000.0, -4000000.0, 4000000.0, 4000000.0)
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CrsParseError {
    #[error("Unsupported CRS: {0}")]
    UnsupportedCrs(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_crs() {
        assert_eq!(
            CrsCode::from_wms_string("EPSG:4326").unwrap(),
            CrsCode::Epsg4326
        );
        assert_eq!(
            CrsCode::from_wms_string("epsg:3857").unwrap(),
            CrsCode::Epsg3857
        );
        assert_eq!(
            CrsCode::from_wms_string("CRS:84").unwrap(),
            CrsCode::Epsg4326
        );
        assert!(CrsCode::from_wms_string("EPSG:99999").is_err());
    }

    #[test]
    fn test_axis_order() {
        assert_eq!(CrsCode::Epsg4326.axis_order_wms_1_3(), AxisOrder::LatLon);
        assert_eq!(CrsCode::Epsg3857.axis_order_wms_1_3(), AxisOrder::XY);

        // WMS 1.1.1 always uses X,Y
        assert_eq!(CrsCode::Epsg4326.axis_order_wms_1_1(), AxisOrder::XY);
    }
}
