//! Query parameter parsing for EDR endpoints.
//!
//! This module handles parsing and validation of query parameters
//! for EDR data queries (position, area, cube, etc.).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur when parsing coordinates.
#[derive(Debug, Error, PartialEq)]
pub enum CoordinateParseError {
    /// Invalid WKT format.
    #[error("Invalid WKT format: {0}")]
    InvalidWkt(String),

    /// Invalid coordinate value.
    #[error("Invalid coordinate value: {0}")]
    InvalidCoordinate(String),

    /// Missing required coordinate.
    #[error("Missing required coordinate: {0}")]
    MissingCoordinate(String),

    /// Coordinate out of valid range.
    #[error("Coordinate out of range: {0}")]
    OutOfRange(String),
}

/// Parsed position query parameters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PositionQuery {
    /// Longitude.
    pub lon: f64,

    /// Latitude.
    pub lat: f64,

    /// Requested vertical level(s).
    pub z: Option<Vec<f64>>,

    /// Requested datetime or range.
    pub datetime: Option<DateTimeQuery>,

    /// Requested parameter names.
    pub parameter_names: Option<Vec<String>>,

    /// Coordinate reference system.
    pub crs: Option<String>,

    /// Output format.
    pub format: Option<String>,
}

impl PositionQuery {
    /// Parse a WKT POINT string into a PositionQuery.
    ///
    /// Accepts formats:
    /// - `POINT(lon lat)`
    /// - `POINT (lon lat)` (with space)
    /// - Just `lon,lat`
    pub fn parse_coords(coords: &str) -> Result<(f64, f64), CoordinateParseError> {
        let coords = coords.trim();

        // Try WKT POINT format
        if coords.to_uppercase().starts_with("POINT") {
            return Self::parse_wkt_point(coords);
        }

        // Try simple lon,lat format
        if coords.contains(',') {
            return Self::parse_simple_coords(coords);
        }

        Err(CoordinateParseError::InvalidWkt(
            "Expected POINT(lon lat) or lon,lat format".to_string(),
        ))
    }

    fn parse_wkt_point(wkt: &str) -> Result<(f64, f64), CoordinateParseError> {
        // Find the parentheses
        let start = wkt.find('(').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing opening parenthesis".to_string())
        })?;
        let end = wkt.find(')').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing closing parenthesis".to_string())
        })?;

        if end <= start {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid parenthesis order".to_string(),
            ));
        }

        let coords_str = &wkt[start + 1..end].trim();

        // Split by space
        let parts: Vec<&str> = coords_str.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(CoordinateParseError::InvalidWkt(format!(
                "Expected 2 coordinates, got {}",
                parts.len()
            )));
        }

        let lon: f64 = parts[0]
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[0].to_string()))?;

        let lat: f64 = parts[1]
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[1].to_string()))?;

        // Validate ranges
        Self::validate_coordinates(lon, lat)?;

        Ok((lon, lat))
    }

    fn parse_simple_coords(coords: &str) -> Result<(f64, f64), CoordinateParseError> {
        let parts: Vec<&str> = coords.split(',').collect();
        if parts.len() != 2 {
            return Err(CoordinateParseError::InvalidWkt(format!(
                "Expected lon,lat format, got {} parts",
                parts.len()
            )));
        }

        let lon: f64 = parts[0]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[0].to_string()))?;

        let lat: f64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[1].to_string()))?;

        Self::validate_coordinates(lon, lat)?;

        Ok((lon, lat))
    }

    fn validate_coordinates(lon: f64, lat: f64) -> Result<(), CoordinateParseError> {
        if !(-180.0..=180.0).contains(&lon) {
            return Err(CoordinateParseError::OutOfRange(format!(
                "Longitude {} is out of range [-180, 180]",
                lon
            )));
        }

        if !(-90.0..=90.0).contains(&lat) {
            return Err(CoordinateParseError::OutOfRange(format!(
                "Latitude {} is out of range [-90, 90]",
                lat
            )));
        }

        Ok(())
    }

    /// Parse vertical level parameter.
    ///
    /// Accepts formats:
    /// - Single value: `850`
    /// - Multiple values: `850,700,500`
    /// - Range: `1000/250` (from/to)
    pub fn parse_z(z_param: &str) -> Result<Vec<f64>, CoordinateParseError> {
        let z_param = z_param.trim();

        // Check for range format
        if z_param.contains('/') {
            let parts: Vec<&str> = z_param.split('/').collect();
            if parts.len() != 2 {
                return Err(CoordinateParseError::InvalidWkt(
                    "Invalid z range format, expected from/to".to_string(),
                ));
            }

            let from: f64 = parts[0]
                .trim()
                .parse()
                .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[0].to_string()))?;
            let to: f64 = parts[1]
                .trim()
                .parse()
                .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[1].to_string()))?;

            // Return as range markers (actual levels would be determined by collection)
            return Ok(vec![from, to]);
        }

        // Check for multiple values
        if z_param.contains(',') {
            let values: Result<Vec<f64>, _> = z_param
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<f64>()
                        .map_err(|_| CoordinateParseError::InvalidCoordinate(s.to_string()))
                })
                .collect();
            return values;
        }

        // Single value
        let value: f64 = z_param
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(z_param.to_string()))?;

        Ok(vec![value])
    }

    /// Parse parameter-name query parameter.
    pub fn parse_parameter_names(param: &str) -> Vec<String> {
        param
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Datetime query specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DateTimeQuery {
    /// A specific instant.
    Instant(String),

    /// An interval with start and end.
    Interval {
        start: Option<String>,
        end: Option<String>,
    },
}

impl DateTimeQuery {
    /// Parse a datetime parameter.
    ///
    /// Accepts formats:
    /// - Instant: `2024-12-29T12:00:00Z`
    /// - Interval: `2024-12-29T00:00:00Z/2024-12-29T23:59:59Z`
    /// - Open start: `../2024-12-29T23:59:59Z`
    /// - Open end: `2024-12-29T00:00:00Z/..`
    pub fn parse(datetime: &str) -> Result<Self, CoordinateParseError> {
        let datetime = datetime.trim();

        if datetime.contains('/') {
            let parts: Vec<&str> = datetime.split('/').collect();
            if parts.len() != 2 {
                return Err(CoordinateParseError::InvalidWkt(
                    "Invalid datetime interval format".to_string(),
                ));
            }

            let start = if parts[0] == ".." {
                None
            } else {
                Some(parts[0].to_string())
            };

            let end = if parts[1] == ".." {
                None
            } else {
                Some(parts[1].to_string())
            };

            Ok(DateTimeQuery::Interval { start, end })
        } else {
            Ok(DateTimeQuery::Instant(datetime.to_string()))
        }
    }
}

/// Bounding box query parameters (for area/cube queries).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BboxQuery {
    /// Western longitude.
    pub west: f64,

    /// Southern latitude.
    pub south: f64,

    /// Eastern longitude.
    pub east: f64,

    /// Northern latitude.
    pub north: f64,
}

impl BboxQuery {
    /// Parse a bbox parameter.
    ///
    /// Format: `west,south,east,north`
    pub fn parse(bbox: &str) -> Result<Self, CoordinateParseError> {
        let parts: Vec<&str> = bbox.split(',').collect();
        if parts.len() != 4 {
            return Err(CoordinateParseError::InvalidWkt(format!(
                "Expected 4 values for bbox, got {}",
                parts.len()
            )));
        }

        let west: f64 = parts[0]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[0].to_string()))?;
        let south: f64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[1].to_string()))?;
        let east: f64 = parts[2]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[2].to_string()))?;
        let north: f64 = parts[3]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[3].to_string()))?;

        // Validate ranges
        PositionQuery::validate_coordinates(west, south)?;
        PositionQuery::validate_coordinates(east, north)?;

        if south > north {
            return Err(CoordinateParseError::OutOfRange(
                "South must be less than or equal to north".to_string(),
            ));
        }

        Ok(BboxQuery {
            west,
            south,
            east,
            north,
        })
    }

    /// Calculate the area of the bbox in square degrees.
    pub fn area_sq_degrees(&self) -> f64 {
        let width = if self.east >= self.west {
            self.east - self.west
        } else {
            // Handle antimeridian crossing
            (180.0 - self.west) + (self.east + 180.0)
        };
        let height = self.north - self.south;
        width * height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wkt_point() {
        let (lon, lat) = PositionQuery::parse_coords("POINT(-97.5 35.2)").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_wkt_point_with_space() {
        let (lon, lat) = PositionQuery::parse_coords("POINT (-97.5 35.2)").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_wkt_point_lowercase() {
        let (lon, lat) = PositionQuery::parse_coords("point(-97.5 35.2)").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_simple_coords() {
        let (lon, lat) = PositionQuery::parse_coords("-97.5,35.2").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_simple_coords_with_spaces() {
        let (lon, lat) = PositionQuery::parse_coords(" -97.5 , 35.2 ").unwrap();
        assert_eq!(lon, -97.5);
        assert_eq!(lat, 35.2);
    }

    #[test]
    fn test_parse_coords_out_of_range_lon() {
        let result = PositionQuery::parse_coords("POINT(-200 35.2)");
        assert!(matches!(result, Err(CoordinateParseError::OutOfRange(_))));
    }

    #[test]
    fn test_parse_coords_out_of_range_lat() {
        let result = PositionQuery::parse_coords("POINT(-97.5 100)");
        assert!(matches!(result, Err(CoordinateParseError::OutOfRange(_))));
    }

    #[test]
    fn test_parse_coords_invalid_wkt() {
        let result = PositionQuery::parse_coords("POINT-97.5 35.2");
        assert!(matches!(result, Err(CoordinateParseError::InvalidWkt(_))));
    }

    #[test]
    fn test_parse_coords_invalid_number() {
        let result = PositionQuery::parse_coords("POINT(abc 35.2)");
        assert!(matches!(
            result,
            Err(CoordinateParseError::InvalidCoordinate(_))
        ));
    }

    #[test]
    fn test_parse_z_single() {
        let z = PositionQuery::parse_z("850").unwrap();
        assert_eq!(z, vec![850.0]);
    }

    #[test]
    fn test_parse_z_multiple() {
        let z = PositionQuery::parse_z("850,700,500").unwrap();
        assert_eq!(z, vec![850.0, 700.0, 500.0]);
    }

    #[test]
    fn test_parse_z_range() {
        let z = PositionQuery::parse_z("1000/250").unwrap();
        assert_eq!(z, vec![1000.0, 250.0]);
    }

    #[test]
    fn test_parse_parameter_names() {
        let params = PositionQuery::parse_parameter_names("TMP,UGRD,VGRD");
        assert_eq!(params, vec!["TMP", "UGRD", "VGRD"]);
    }

    #[test]
    fn test_parse_parameter_names_with_spaces() {
        let params = PositionQuery::parse_parameter_names(" TMP , UGRD , VGRD ");
        assert_eq!(params, vec!["TMP", "UGRD", "VGRD"]);
    }

    #[test]
    fn test_datetime_instant() {
        let dt = DateTimeQuery::parse("2024-12-29T12:00:00Z").unwrap();
        assert!(matches!(dt, DateTimeQuery::Instant(_)));
        if let DateTimeQuery::Instant(s) = dt {
            assert_eq!(s, "2024-12-29T12:00:00Z");
        }
    }

    #[test]
    fn test_datetime_interval() {
        let dt = DateTimeQuery::parse("2024-12-29T00:00:00Z/2024-12-29T23:59:59Z").unwrap();
        if let DateTimeQuery::Interval { start, end } = dt {
            assert_eq!(start, Some("2024-12-29T00:00:00Z".to_string()));
            assert_eq!(end, Some("2024-12-29T23:59:59Z".to_string()));
        } else {
            panic!("Expected interval");
        }
    }

    #[test]
    fn test_datetime_open_start() {
        let dt = DateTimeQuery::parse("../2024-12-29T23:59:59Z").unwrap();
        if let DateTimeQuery::Interval { start, end } = dt {
            assert!(start.is_none());
            assert_eq!(end, Some("2024-12-29T23:59:59Z".to_string()));
        } else {
            panic!("Expected interval");
        }
    }

    #[test]
    fn test_datetime_open_end() {
        let dt = DateTimeQuery::parse("2024-12-29T00:00:00Z/..").unwrap();
        if let DateTimeQuery::Interval { start, end } = dt {
            assert_eq!(start, Some("2024-12-29T00:00:00Z".to_string()));
            assert!(end.is_none());
        } else {
            panic!("Expected interval");
        }
    }

    #[test]
    fn test_bbox_parse() {
        let bbox = BboxQuery::parse("-125,24,-66,50").unwrap();
        assert_eq!(bbox.west, -125.0);
        assert_eq!(bbox.south, 24.0);
        assert_eq!(bbox.east, -66.0);
        assert_eq!(bbox.north, 50.0);
    }

    #[test]
    fn test_bbox_parse_with_spaces() {
        let bbox = BboxQuery::parse(" -125 , 24 , -66 , 50 ").unwrap();
        assert_eq!(bbox.west, -125.0);
        assert_eq!(bbox.south, 24.0);
    }

    #[test]
    fn test_bbox_invalid_count() {
        let result = BboxQuery::parse("-125,24,-66");
        assert!(matches!(result, Err(CoordinateParseError::InvalidWkt(_))));
    }

    #[test]
    fn test_bbox_south_greater_than_north() {
        let result = BboxQuery::parse("-125,50,-66,24");
        assert!(matches!(result, Err(CoordinateParseError::OutOfRange(_))));
    }

    #[test]
    fn test_bbox_area() {
        let bbox = BboxQuery::parse("-125,24,-66,50").unwrap();
        let area = bbox.area_sq_degrees();
        // Width: -66 - (-125) = 59 degrees
        // Height: 50 - 24 = 26 degrees
        // Area: 59 * 26 = 1534 sq degrees
        assert!((area - 1534.0).abs() < 0.01);
    }
}
