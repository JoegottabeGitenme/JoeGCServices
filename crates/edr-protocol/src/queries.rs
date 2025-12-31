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

/// Result of parsing position coordinates - can be single point or multiple points.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedCoords {
    /// Single point (lon, lat).
    Single(f64, f64),
    /// Multiple points.
    Multi(Vec<(f64, f64)>),
}

impl PositionQuery {
    /// Parse a WKT POINT or MULTIPOINT string.
    ///
    /// Accepts formats:
    /// - `POINT(lon lat)`
    /// - `POINT (lon lat)` (with space)
    /// - `MULTIPOINT((lon1 lat1),(lon2 lat2))`
    /// - Just `lon,lat`
    ///
    /// Returns ParsedCoords to handle both single and multi-point cases.
    pub fn parse_coords_multi(coords: &str) -> Result<ParsedCoords, CoordinateParseError> {
        let coords = coords.trim();
        let upper = coords.to_uppercase();

        // Try WKT MULTIPOINT format first
        if upper.starts_with("MULTIPOINT") {
            let points = Self::parse_wkt_multipoint(coords)?;
            return Ok(ParsedCoords::Multi(points));
        }

        // Try WKT POINT format
        if upper.starts_with("POINT") {
            let (lon, lat) = Self::parse_wkt_point(coords)?;
            return Ok(ParsedCoords::Single(lon, lat));
        }

        // Try simple lon,lat format
        if coords.contains(',') {
            let (lon, lat) = Self::parse_simple_coords(coords)?;
            return Ok(ParsedCoords::Single(lon, lat));
        }

        Err(CoordinateParseError::InvalidWkt(
            "Expected POINT(lon lat), MULTIPOINT((lon1 lat1),(lon2 lat2)), or lon,lat format".to_string(),
        ))
    }

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

    /// Parse a WKT MULTIPOINT string.
    ///
    /// Accepts format: `MULTIPOINT((lon1 lat1),(lon2 lat2),...)`
    fn parse_wkt_multipoint(wkt: &str) -> Result<Vec<(f64, f64)>, CoordinateParseError> {
        // Find the outer parentheses
        let start = wkt.find('(').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing opening parenthesis".to_string())
        })?;
        let end = wkt.rfind(')').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing closing parenthesis".to_string())
        })?;

        if end <= start {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid parenthesis order".to_string(),
            ));
        }

        let inner = &wkt[start + 1..end].trim();
        
        // Parse each point - they are enclosed in parentheses and separated by commas
        // Format: (lon1 lat1),(lon2 lat2)
        let mut points = Vec::new();
        let mut depth = 0;
        let mut current_point = String::new();
        
        for ch in inner.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    if depth > 1 {
                        current_point.push(ch);
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        // End of a point
                        let coords = current_point.trim();
                        if !coords.is_empty() {
                            let parts: Vec<&str> = coords.split_whitespace().collect();
                            if parts.len() != 2 {
                                return Err(CoordinateParseError::InvalidWkt(format!(
                                    "Expected 'lon lat' format, got '{}'",
                                    coords
                                )));
                            }
                            let lon: f64 = parts[0].parse().map_err(|_| {
                                CoordinateParseError::InvalidCoordinate(parts[0].to_string())
                            })?;
                            let lat: f64 = parts[1].parse().map_err(|_| {
                                CoordinateParseError::InvalidCoordinate(parts[1].to_string())
                            })?;
                            Self::validate_coordinates(lon, lat)?;
                            points.push((lon, lat));
                        }
                        current_point.clear();
                    } else {
                        current_point.push(ch);
                    }
                }
                ',' if depth == 0 => {
                    // Skip comma between points
                }
                _ => {
                    if depth > 0 {
                        current_point.push(ch);
                    }
                }
            }
        }

        if points.is_empty() {
            return Err(CoordinateParseError::InvalidWkt(
                "MULTIPOINT must contain at least one point".to_string(),
            ));
        }

        Ok(points)
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
    /// - Recurring intervals: `R5/1000/100` (R{count}/{start}/{increment})
    pub fn parse_z(z_param: &str) -> Result<Vec<f64>, CoordinateParseError> {
        let z_param = z_param.trim();

        // Check for recurring interval format: R{count}/{start}/{increment}
        if z_param.starts_with('R') || z_param.starts_with('r') {
            return Self::parse_z_recurring(&z_param[1..]);
        }

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

    /// Parse recurring z interval format: {count}/{start}/{increment}
    /// Example: "5/1000/100" -> [1000, 900, 800, 700, 600]
    fn parse_z_recurring(s: &str) -> Result<Vec<f64>, CoordinateParseError> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid recurring z format, expected R{count}/{start}/{increment}".to_string(),
            ));
        }

        let count: usize = parts[0]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[0].to_string()))?;
        let start: f64 = parts[1]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[1].to_string()))?;
        let increment: f64 = parts[2]
            .trim()
            .parse()
            .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[2].to_string()))?;

        if count == 0 || count > 100 {
            return Err(CoordinateParseError::OutOfRange(
                "Recurring count must be between 1 and 100".to_string(),
            ));
        }

        // Generate the levels: start, start-increment, start-2*increment, etc.
        // (For pressure levels, we typically decrement)
        let levels: Vec<f64> = (0..count)
            .map(|i| start - (i as f64 * increment))
            .collect();

        Ok(levels)
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

    /// Multiple specific instants (comma-separated list).
    List(Vec<String>),

    /// An interval with start and end.
    Interval {
        start: Option<String>,
        end: Option<String>,
    },
}

impl DateTimeQuery {
    /// Validate that a datetime string is a valid ISO 8601 format.
    fn validate_datetime(dt: &str) -> Result<(), CoordinateParseError> {
        // Allow ".." for open intervals
        if dt == ".." {
            return Ok(());
        }
        
        // Try to parse as RFC 3339 (ISO 8601 subset)
        if chrono::DateTime::parse_from_rfc3339(dt).is_ok() {
            return Ok(());
        }
        
        // Also try without timezone (common format)
        if chrono::NaiveDateTime::parse_from_str(dt, "%Y-%m-%dT%H:%M:%S").is_ok() {
            return Ok(());
        }
        
        // Try date-only format
        if chrono::NaiveDate::parse_from_str(dt, "%Y-%m-%d").is_ok() {
            return Ok(());
        }
        
        Err(CoordinateParseError::InvalidWkt(format!(
            "Invalid datetime format '{}'. Expected ISO 8601 format (e.g., 2024-12-29T12:00:00Z)",
            dt
        )))
    }

    /// Parse a datetime parameter.
    ///
    /// Accepts formats:
    /// - Instant: `2024-12-29T12:00:00Z`
    /// - List: `2024-12-29T12:00:00Z,2024-12-29T13:00:00Z,2024-12-29T14:00:00Z`
    /// - Interval: `2024-12-29T00:00:00Z/2024-12-29T23:59:59Z`
    /// - Open start: `../2024-12-29T23:59:59Z`
    /// - Open end: `2024-12-29T00:00:00Z/..`
    pub fn parse(datetime: &str) -> Result<Self, CoordinateParseError> {
        let datetime = datetime.trim();

        // Check for interval format first (contains / but not as part of a comma list)
        if datetime.contains('/') && !datetime.contains(',') {
            let parts: Vec<&str> = datetime.split('/').collect();
            if parts.len() != 2 {
                return Err(CoordinateParseError::InvalidWkt(
                    "Invalid datetime interval format".to_string(),
                ));
            }

            let start = if parts[0] == ".." {
                None
            } else {
                Self::validate_datetime(parts[0])?;
                Some(parts[0].to_string())
            };

            let end = if parts[1] == ".." {
                None
            } else {
                Self::validate_datetime(parts[1])?;
                Some(parts[1].to_string())
            };

            return Ok(DateTimeQuery::Interval { start, end });
        }

        // Check for comma-separated list
        if datetime.contains(',') {
            let times: Vec<String> = datetime
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            
            // Validate each datetime in the list
            for t in &times {
                Self::validate_datetime(t)?;
            }

            if times.len() == 1 {
                return Ok(DateTimeQuery::Instant(times.into_iter().next().unwrap()));
            }

            return Ok(DateTimeQuery::List(times));
        }

        // Single instant - validate it
        Self::validate_datetime(datetime)?;
        Ok(DateTimeQuery::Instant(datetime.to_string()))
    }

    /// Get all datetime values as a vector of strings.
    ///
    /// For instants, returns a single-element vector.
    /// For lists, returns all values.
    /// For intervals, returns start and end (if present).
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            DateTimeQuery::Instant(s) => vec![s.clone()],
            DateTimeQuery::List(list) => list.clone(),
            DateTimeQuery::Interval { start, end } => {
                let mut result = Vec::new();
                if let Some(s) = start {
                    result.push(s.clone());
                }
                if let Some(e) = end {
                    result.push(e.clone());
                }
                result
            }
        }
    }

    /// Check if this query contains multiple time values.
    pub fn is_multi_time(&self) -> bool {
        match self {
            DateTimeQuery::Instant(_) => false,
            DateTimeQuery::List(list) => list.len() > 1,
            DateTimeQuery::Interval { .. } => true,
        }
    }

    /// Get the number of discrete time values.
    ///
    /// For intervals, this returns 2 (start and end) - actual resolution
    /// depends on the collection's temporal resolution.
    pub fn len(&self) -> usize {
        match self {
            DateTimeQuery::Instant(_) => 1,
            DateTimeQuery::List(list) => list.len(),
            DateTimeQuery::Interval { start, end } => {
                (start.is_some() as usize) + (end.is_some() as usize)
            }
        }
    }

    /// Check if the datetime query is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Expand an interval query against available times.
    ///
    /// For open-ended intervals (e.g., `2024-01-01/..` or `../2024-12-31`),
    /// this resolves them to concrete time values from the provided list.
    ///
    /// Returns a list of time strings that fall within the interval.
    pub fn expand_against_available_times(&self, available_times: &[String]) -> Vec<String> {
        match self {
            DateTimeQuery::Instant(s) => vec![s.clone()],
            DateTimeQuery::List(list) => list.clone(),
            DateTimeQuery::Interval { start, end } => {
                // Filter available times to those within the interval
                let start_bound = start.as_ref().and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                });
                let end_bound = end.as_ref().and_then(|e| {
                    chrono::DateTime::parse_from_rfc3339(e)
                        .ok()
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                });

                available_times
                    .iter()
                    .filter(|t| {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
                            let dt_utc = dt.with_timezone(&chrono::Utc);
                            let after_start = start_bound.map_or(true, |s| dt_utc >= s);
                            let before_end = end_bound.map_or(true, |e| dt_utc <= e);
                            after_start && before_end
                        } else {
                            false
                        }
                    })
                    .cloned()
                    .collect()
            }
        }
    }

    /// Check if this is an interval query (with potential open ends).
    pub fn is_interval(&self) -> bool {
        matches!(self, DateTimeQuery::Interval { .. })
    }

    /// Check if this interval has an open end (no end bound).
    pub fn has_open_end(&self) -> bool {
        matches!(self, DateTimeQuery::Interval { end: None, .. })
    }

    /// Check if this interval has an open start (no start bound).
    pub fn has_open_start(&self) -> bool {
        matches!(self, DateTimeQuery::Interval { start: None, .. })
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

/// Area query parameters (polygon-based spatial query).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AreaQuery {
    /// Polygon coordinates as a ring of (lon, lat) points.
    /// The first and last point should be the same to close the ring.
    pub polygon: Vec<(f64, f64)>,

    /// Requested vertical level(s).
    pub z: Option<Vec<f64>>,

    /// Requested datetime or range.
    pub datetime: Option<DateTimeQuery>,

    /// Requested parameter names.
    pub parameter_names: Option<Vec<String>>,

    /// Coordinate reference system.
    pub crs: Option<String>,
}

/// Result of parsing polygon coordinates - can be single polygon or multiple polygons.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedPolygons {
    /// Single polygon (ring of points).
    Single(Vec<(f64, f64)>),
    /// Multiple polygons.
    Multi(Vec<Vec<(f64, f64)>>),
}

impl AreaQuery {
    /// Parse a WKT POLYGON or MULTIPOLYGON string.
    ///
    /// Accepts formats:
    /// - `POLYGON((lon1 lat1, lon2 lat2, lon3 lat3, lon1 lat1))`
    /// - `MULTIPOLYGON(((ring1)),((ring2)))`
    ///
    /// Returns ParsedPolygons to handle both cases.
    pub fn parse_polygon_multi(coords: &str) -> Result<ParsedPolygons, CoordinateParseError> {
        let coords = coords.trim();
        let upper = coords.to_uppercase();

        if upper.starts_with("MULTIPOLYGON") {
            let polygons = Self::parse_wkt_multipolygon(coords)?;
            return Ok(ParsedPolygons::Multi(polygons));
        }

        if upper.starts_with("POLYGON") {
            let polygon = Self::parse_polygon(coords)?;
            return Ok(ParsedPolygons::Single(polygon));
        }

        Err(CoordinateParseError::InvalidWkt(
            "Expected POLYGON or MULTIPOLYGON format".to_string(),
        ))
    }

    /// Parse a WKT POLYGON string.
    ///
    /// Accepts format: `POLYGON((lon1 lat1, lon2 lat2, lon3 lat3, lon1 lat1))`
    pub fn parse_polygon(coords: &str) -> Result<Vec<(f64, f64)>, CoordinateParseError> {
        let coords = coords.trim();
        let upper = coords.to_uppercase();

        if !upper.starts_with("POLYGON") {
            return Err(CoordinateParseError::InvalidWkt(
                "Expected POLYGON format".to_string(),
            ));
        }

        // Find the outer parentheses
        let start = coords.find("((").ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing opening parentheses".to_string())
        })?;
        let end = coords.rfind("))").ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing closing parentheses".to_string())
        })?;

        if end <= start {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid parenthesis order".to_string(),
            ));
        }

        // Extract the coordinate string (inside the double parens)
        let coords_str = &coords[start + 2..end].trim();
        
        Self::parse_ring(coords_str)
    }

    /// Parse a single polygon ring from coordinate string.
    fn parse_ring(coords_str: &str) -> Result<Vec<(f64, f64)>, CoordinateParseError> {
        // Split by comma to get individual coordinate pairs
        let points: Result<Vec<(f64, f64)>, _> = coords_str
            .split(',')
            .map(|pair| {
                let pair = pair.trim();
                let parts: Vec<&str> = pair.split_whitespace().collect();
                if parts.len() != 2 {
                    return Err(CoordinateParseError::InvalidWkt(format!(
                        "Expected 'lon lat' format, got '{}'",
                        pair
                    )));
                }

                let lon: f64 = parts[0]
                    .parse()
                    .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[0].to_string()))?;
                let lat: f64 = parts[1]
                    .parse()
                    .map_err(|_| CoordinateParseError::InvalidCoordinate(parts[1].to_string()))?;

                // Validate ranges
                PositionQuery::validate_coordinates(lon, lat)?;

                Ok((lon, lat))
            })
            .collect();

        let points = points?;

        if points.len() < 4 {
            return Err(CoordinateParseError::InvalidWkt(
                "Polygon must have at least 4 points (including closing point)".to_string(),
            ));
        }

        Ok(points)
    }

    /// Parse a WKT MULTIPOLYGON string.
    ///
    /// Accepts format: `MULTIPOLYGON(((ring1)),((ring2)))`
    fn parse_wkt_multipolygon(coords: &str) -> Result<Vec<Vec<(f64, f64)>>, CoordinateParseError> {
        // Find content after MULTIPOLYGON
        let start = coords.find('(').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing opening parenthesis".to_string())
        })?;
        let end = coords.rfind(')').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing closing parenthesis".to_string())
        })?;

        if end <= start {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid parenthesis order".to_string(),
            ));
        }

        let inner = &coords[start + 1..end];
        
        // Parse each polygon - they are enclosed in double parentheses
        // Format: ((ring1)),((ring2))
        let mut polygons = Vec::new();
        let mut depth = 0;
        let mut current_polygon = String::new();
        
        for ch in inner.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    if depth > 1 {
                        current_polygon.push(ch);
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 1 {
                        // End of a polygon's ring
                        let ring_str = current_polygon.trim();
                        if !ring_str.is_empty() {
                            // Remove extra parentheses if present
                            let ring_str = ring_str.trim_start_matches('(').trim_end_matches(')');
                            let ring = Self::parse_ring(ring_str)?;
                            polygons.push(ring);
                        }
                        current_polygon.clear();
                    } else if depth > 1 {
                        current_polygon.push(ch);
                    }
                }
                ',' if depth <= 1 => {
                    // Skip comma between polygons
                }
                _ => {
                    if depth > 1 {
                        current_polygon.push(ch);
                    }
                }
            }
        }

        if polygons.is_empty() {
            return Err(CoordinateParseError::InvalidWkt(
                "MULTIPOLYGON must contain at least one polygon".to_string(),
            ));
        }

        Ok(polygons)
    }

    /// Calculate the bounding box of the polygon.
    pub fn bbox(&self) -> BboxQuery {
        let mut west = f64::MAX;
        let mut south = f64::MAX;
        let mut east = f64::MIN;
        let mut north = f64::MIN;

        for (lon, lat) in &self.polygon {
            west = west.min(*lon);
            east = east.max(*lon);
            south = south.min(*lat);
            north = north.max(*lat);
        }

        BboxQuery {
            west,
            south,
            east,
            north,
        }
    }

    /// Calculate approximate area of the polygon's bounding box in square degrees.
    pub fn area_sq_degrees(&self) -> f64 {
        self.bbox().area_sq_degrees()
    }

    /// Check if a point is inside the polygon using ray casting algorithm.
    pub fn contains_point(&self, lon: f64, lat: f64) -> bool {
        let n = self.polygon.len();
        if n < 3 {
            return false;
        }

        let mut inside = false;
        let mut j = n - 1;

        for i in 0..n {
            let (xi, yi) = self.polygon[i];
            let (xj, yj) = self.polygon[j];

            if ((yi > lat) != (yj > lat)) && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }

        inside
    }
}

/// Distance units supported for radius queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceUnit {
    /// Kilometers
    Kilometers,
    /// Miles
    Miles,
    /// Meters
    Meters,
    /// Nautical miles
    NauticalMiles,
}

impl DistanceUnit {
    /// Parse a distance unit string.
    ///
    /// Accepts: "km", "kilometers", "mi", "miles", "m", "meters", "nm", "nautical_miles"
    pub fn parse(unit: &str) -> Result<Self, CoordinateParseError> {
        match unit.to_lowercase().trim() {
            "km" | "kilometers" | "kilometre" | "kilometres" => Ok(DistanceUnit::Kilometers),
            "mi" | "miles" | "mile" => Ok(DistanceUnit::Miles),
            "m" | "meters" | "metre" | "metres" => Ok(DistanceUnit::Meters),
            "nm" | "nautical_miles" | "nautical miles" | "nauticalmiles" => Ok(DistanceUnit::NauticalMiles),
            _ => Err(CoordinateParseError::InvalidWkt(format!(
                "Unknown distance unit '{}'. Supported units: km, mi, m, nm",
                unit
            ))),
        }
    }

    /// Convert a value in this unit to meters.
    pub fn to_meters(&self, value: f64) -> f64 {
        match self {
            DistanceUnit::Kilometers => value * 1000.0,
            DistanceUnit::Miles => value * 1609.344,
            DistanceUnit::Meters => value,
            DistanceUnit::NauticalMiles => value * 1852.0,
        }
    }

    /// Convert a value in this unit to kilometers.
    pub fn to_kilometers(&self, value: f64) -> f64 {
        self.to_meters(value) / 1000.0
    }

    /// Get the string representation of this unit.
    pub fn as_str(&self) -> &'static str {
        match self {
            DistanceUnit::Kilometers => "km",
            DistanceUnit::Miles => "mi",
            DistanceUnit::Meters => "m",
            DistanceUnit::NauticalMiles => "nm",
        }
    }
}

/// Radius query parameters (circle around a point).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RadiusQuery {
    /// Center point longitude.
    pub center_lon: f64,

    /// Center point latitude.
    pub center_lat: f64,

    /// Radius value in meters (converted from original units).
    pub radius_meters: f64,

    /// Requested vertical level(s).
    pub z: Option<Vec<f64>>,

    /// Requested datetime or range.
    pub datetime: Option<DateTimeQuery>,

    /// Requested parameter names.
    pub parameter_names: Option<Vec<String>>,

    /// Coordinate reference system.
    pub crs: Option<String>,
}

impl RadiusQuery {
    /// Earth's radius in meters (WGS84 mean radius).
    const EARTH_RADIUS_M: f64 = 6_371_008.8;

    /// Parse the 'within' parameter (radius value).
    ///
    /// Accepts numeric string like "100" or "50.5"
    pub fn parse_within(within: &str) -> Result<f64, CoordinateParseError> {
        let within = within.trim();
        within.parse::<f64>().map_err(|_| {
            CoordinateParseError::InvalidCoordinate(format!(
                "Invalid radius value '{}'. Expected a number.",
                within
            ))
        }).and_then(|v| {
            if v <= 0.0 {
                Err(CoordinateParseError::OutOfRange(
                    "Radius must be a positive number".to_string(),
                ))
            } else {
                Ok(v)
            }
        })
    }

    /// Create a RadiusQuery from parsed parameters.
    pub fn new(
        center_lon: f64,
        center_lat: f64,
        within: f64,
        within_units: DistanceUnit,
    ) -> Self {
        Self {
            center_lon,
            center_lat,
            radius_meters: within_units.to_meters(within),
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        }
    }

    /// Get the radius in kilometers.
    pub fn radius_km(&self) -> f64 {
        self.radius_meters / 1000.0
    }

    /// Calculate the Haversine distance between two points in meters.
    ///
    /// Uses the Haversine formula for great-circle distance.
    pub fn haversine_distance(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
        let lat1_rad = lat1.to_radians();
        let lat2_rad = lat2.to_radians();
        let delta_lat = (lat2 - lat1).to_radians();
        let delta_lon = (lon2 - lon1).to_radians();

        let a = (delta_lat / 2.0).sin().powi(2)
            + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().asin();

        Self::EARTH_RADIUS_M * c
    }

    /// Check if a point is within the radius of the center point.
    pub fn contains_point(&self, lon: f64, lat: f64) -> bool {
        let distance = Self::haversine_distance(self.center_lon, self.center_lat, lon, lat);
        distance <= self.radius_meters
    }

    /// Calculate the bounding box that encloses this radius.
    ///
    /// Returns (west, south, east, north) in degrees.
    pub fn bounding_box(&self) -> BboxQuery {
        // Convert radius to degrees (approximate, varies with latitude)
        // At the equator, 1 degree ≈ 111.32 km
        // For latitude: 1 degree ≈ 111.32 km (constant)
        // For longitude: 1 degree ≈ 111.32 * cos(lat) km (varies with latitude)
        
        let lat_rad = self.center_lat.to_radians();
        let radius_km = self.radius_km();
        
        // Degrees per km at this latitude
        let deg_per_km_lat = 1.0 / 111.32;
        let deg_per_km_lon = 1.0 / (111.32 * lat_rad.cos().max(0.01)); // Avoid division by zero near poles
        
        let delta_lat = radius_km * deg_per_km_lat;
        let delta_lon = radius_km * deg_per_km_lon;
        
        BboxQuery {
            west: (self.center_lon - delta_lon).max(-180.0),
            south: (self.center_lat - delta_lat).max(-90.0),
            east: (self.center_lon + delta_lon).min(180.0),
            north: (self.center_lat + delta_lat).min(90.0),
        }
    }

    /// Calculate approximate area of the circle in square degrees.
    ///
    /// Uses the bounding box as an approximation.
    pub fn area_sq_degrees(&self) -> f64 {
        // π * r² but in degree-space (approximate)
        let bbox = self.bounding_box();
        let width = bbox.east - bbox.west;
        let height = bbox.north - bbox.south;
        // Circle area ≈ π/4 of the bounding box area
        std::f64::consts::PI / 4.0 * width * height
    }
}

/// A single waypoint in a trajectory.
///
/// Supports 2D (lon, lat), 3D with height (lon, lat, z), 3D with time (lon, lat, m),
/// and 4D (lon, lat, z, m) waypoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryWaypoint {
    /// Longitude in degrees.
    pub lon: f64,

    /// Latitude in degrees.
    pub lat: f64,

    /// Optional height/vertical level (from LINESTRINGZ or LINESTRINGZM).
    /// Units depend on CRS, defaults to meters above sea level.
    pub z: Option<f64>,

    /// Optional time as Unix epoch seconds (from LINESTRINGM or LINESTRINGZM).
    pub m: Option<i64>,
}

impl TrajectoryWaypoint {
    /// Create a 2D waypoint (lon, lat).
    pub fn new_2d(lon: f64, lat: f64) -> Self {
        Self { lon, lat, z: None, m: None }
    }

    /// Create a 3D waypoint with height (lon, lat, z).
    pub fn new_3d_z(lon: f64, lat: f64, z: f64) -> Self {
        Self { lon, lat, z: Some(z), m: None }
    }

    /// Create a 3D waypoint with time (lon, lat, m).
    pub fn new_3d_m(lon: f64, lat: f64, m: i64) -> Self {
        Self { lon, lat, z: None, m: Some(m) }
    }

    /// Create a 4D waypoint (lon, lat, z, m).
    pub fn new_4d(lon: f64, lat: f64, z: f64, m: i64) -> Self {
        Self { lon, lat, z: Some(z), m: Some(m) }
    }
}

/// Type of LINESTRING in the trajectory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStringType {
    /// 2D: LINESTRING (lon lat, lon lat, ...)
    LineString,
    /// 3D with height: LINESTRINGZ (lon lat z, lon lat z, ...)
    LineStringZ,
    /// 3D with time: LINESTRINGM (lon lat m, lon lat m, ...)
    LineStringM,
    /// 4D: LINESTRINGZM (lon lat z m, lon lat z m, ...)
    LineStringZM,
}

impl LineStringType {
    /// Check if this type includes height (Z) coordinates.
    pub fn has_z(&self) -> bool {
        matches!(self, LineStringType::LineStringZ | LineStringType::LineStringZM)
    }

    /// Check if this type includes time (M) coordinates.
    pub fn has_m(&self) -> bool {
        matches!(self, LineStringType::LineStringM | LineStringType::LineStringZM)
    }
}

/// Result of parsing trajectory coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTrajectory {
    /// The type of LINESTRING parsed.
    pub line_type: LineStringType,

    /// The waypoints along the trajectory.
    pub waypoints: Vec<TrajectoryWaypoint>,

    /// Whether this is a MULTI* variant (multiple line segments).
    pub is_multi: bool,
}

/// Trajectory query parameters.
///
/// Represents a path through space (and optionally time) along which
/// data should be sampled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryQuery {
    /// The waypoints defining the trajectory path.
    pub waypoints: Vec<TrajectoryWaypoint>,

    /// The type of LINESTRING (indicates what coordinates are embedded).
    #[serde(skip)]
    pub line_type: Option<LineStringType>,

    /// Requested vertical level(s) - only valid if coords doesn't include Z.
    pub z: Option<Vec<f64>>,

    /// Requested datetime or range - only valid if coords doesn't include M.
    pub datetime: Option<DateTimeQuery>,

    /// Requested parameter names.
    pub parameter_names: Option<Vec<String>>,

    /// Coordinate reference system.
    pub crs: Option<String>,

    /// Output format.
    pub format: Option<String>,
}

impl Default for TrajectoryQuery {
    fn default() -> Self {
        Self::new()
    }
}

impl TrajectoryQuery {
    /// Create an empty trajectory query.
    pub fn new() -> Self {
        Self {
            waypoints: Vec::new(),
            line_type: None,
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
            format: None,
        }
    }

    /// Parse a WKT LINESTRING or MULTILINESTRING coordinate string.
    ///
    /// Supports:
    /// - LINESTRING(lon lat, lon lat, ...)
    /// - LINESTRINGZ(lon lat z, lon lat z, ...)
    /// - LINESTRINGM(lon lat m, lon lat m, ...)
    /// - LINESTRINGZM(lon lat z m, lon lat z m, ...)
    /// - MULTILINESTRING((lon lat, ...),(lon lat, ...))
    /// - And MULTI variants of Z, M, ZM
    pub fn parse_coords(coords: &str) -> Result<ParsedTrajectory, CoordinateParseError> {
        let coords = coords.trim();
        let upper = coords.to_uppercase();

        // Determine the type and whether it's a MULTI variant
        let (line_type, is_multi) = Self::detect_linestring_type(&upper)?;

        // Parse based on whether it's MULTI or single
        let waypoints = if is_multi {
            Self::parse_multi_linestring(coords, line_type)?
        } else {
            Self::parse_single_linestring(coords, line_type)?
        };

        if waypoints.is_empty() {
            return Err(CoordinateParseError::InvalidWkt(
                "LINESTRING must contain at least one waypoint".to_string(),
            ));
        }

        Ok(ParsedTrajectory {
            line_type,
            waypoints,
            is_multi,
        })
    }

    /// Detect the LINESTRING type from the WKT string.
    fn detect_linestring_type(upper: &str) -> Result<(LineStringType, bool), CoordinateParseError> {
        // Check for MULTI variants first (they contain the non-MULTI type name)
        let is_multi = upper.starts_with("MULTI");
        let check_str = if is_multi { &upper[5..] } else { upper };

        let line_type = if check_str.starts_with("LINESTRINGZM") {
            LineStringType::LineStringZM
        } else if check_str.starts_with("LINESTRINGZ") {
            LineStringType::LineStringZ
        } else if check_str.starts_with("LINESTRINGM") {
            LineStringType::LineStringM
        } else if check_str.starts_with("LINESTRING") {
            LineStringType::LineString
        } else {
            return Err(CoordinateParseError::InvalidWkt(
                "Expected LINESTRING, LINESTRINGZ, LINESTRINGM, LINESTRINGZM, or MULTI* variant".to_string(),
            ));
        };

        Ok((line_type, is_multi))
    }

    /// Parse a single LINESTRING.
    fn parse_single_linestring(
        coords: &str,
        line_type: LineStringType,
    ) -> Result<Vec<TrajectoryWaypoint>, CoordinateParseError> {
        // Find the parentheses
        let start = coords.find('(').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing opening parenthesis".to_string())
        })?;
        let end = coords.rfind(')').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing closing parenthesis".to_string())
        })?;

        if end <= start {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid parenthesis order".to_string(),
            ));
        }

        let coords_str = &coords[start + 1..end].trim();
        Self::parse_waypoints(coords_str, line_type)
    }

    /// Parse a MULTILINESTRING into waypoints (concatenates all segments).
    fn parse_multi_linestring(
        coords: &str,
        line_type: LineStringType,
    ) -> Result<Vec<TrajectoryWaypoint>, CoordinateParseError> {
        // Find content after MULTI*LINESTRING*
        let start = coords.find('(').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing opening parenthesis".to_string())
        })?;
        let end = coords.rfind(')').ok_or_else(|| {
            CoordinateParseError::InvalidWkt("Missing closing parenthesis".to_string())
        })?;

        if end <= start {
            return Err(CoordinateParseError::InvalidWkt(
                "Invalid parenthesis order".to_string(),
            ));
        }

        let inner = &coords[start + 1..end];
        
        // Parse each linestring segment
        let mut all_waypoints = Vec::new();
        let mut depth = 0;
        let mut current_segment = String::new();

        for ch in inner.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    if depth > 1 {
                        current_segment.push(ch);
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        // End of a segment
                        let segment_str = current_segment.trim();
                        if !segment_str.is_empty() {
                            let waypoints = Self::parse_waypoints(segment_str, line_type)?;
                            all_waypoints.extend(waypoints);
                        }
                        current_segment.clear();
                    } else if depth > 0 {
                        current_segment.push(ch);
                    }
                }
                ',' if depth == 0 => {
                    // Skip comma between segments
                }
                _ => {
                    if depth > 0 {
                        current_segment.push(ch);
                    }
                }
            }
        }

        if all_waypoints.is_empty() {
            return Err(CoordinateParseError::InvalidWkt(
                "MULTILINESTRING must contain at least one linestring with waypoints".to_string(),
            ));
        }

        Ok(all_waypoints)
    }

    /// Parse waypoints from a coordinate string based on the LINESTRING type.
    fn parse_waypoints(
        coords_str: &str,
        line_type: LineStringType,
    ) -> Result<Vec<TrajectoryWaypoint>, CoordinateParseError> {
        let expected_coords = match line_type {
            LineStringType::LineString => 2,
            LineStringType::LineStringZ | LineStringType::LineStringM => 3,
            LineStringType::LineStringZM => 4,
        };

        // Split by comma to get individual waypoints
        coords_str
            .split(',')
            .map(|waypoint_str| {
                let waypoint_str = waypoint_str.trim();
                let parts: Vec<&str> = waypoint_str.split_whitespace().collect();

                if parts.len() != expected_coords {
                    return Err(CoordinateParseError::InvalidWkt(format!(
                        "Expected {} coordinates for {:?}, got {} in '{}'",
                        expected_coords, line_type, parts.len(), waypoint_str
                    )));
                }

                let lon: f64 = parts[0].parse().map_err(|_| {
                    CoordinateParseError::InvalidCoordinate(parts[0].to_string())
                })?;
                let lat: f64 = parts[1].parse().map_err(|_| {
                    CoordinateParseError::InvalidCoordinate(parts[1].to_string())
                })?;

                // Validate lon/lat ranges
                PositionQuery::validate_coordinates(lon, lat)?;

                let waypoint = match line_type {
                    LineStringType::LineString => TrajectoryWaypoint::new_2d(lon, lat),
                    LineStringType::LineStringZ => {
                        let z: f64 = parts[2].parse().map_err(|_| {
                            CoordinateParseError::InvalidCoordinate(parts[2].to_string())
                        })?;
                        TrajectoryWaypoint::new_3d_z(lon, lat, z)
                    }
                    LineStringType::LineStringM => {
                        let m: i64 = parts[2].parse().map_err(|_| {
                            CoordinateParseError::InvalidCoordinate(format!(
                                "Invalid Unix epoch time: {}",
                                parts[2]
                            ))
                        })?;
                        TrajectoryWaypoint::new_3d_m(lon, lat, m)
                    }
                    LineStringType::LineStringZM => {
                        let z: f64 = parts[2].parse().map_err(|_| {
                            CoordinateParseError::InvalidCoordinate(parts[2].to_string())
                        })?;
                        let m: i64 = parts[3].parse().map_err(|_| {
                            CoordinateParseError::InvalidCoordinate(format!(
                                "Invalid Unix epoch time: {}",
                                parts[3]
                            ))
                        })?;
                        TrajectoryWaypoint::new_4d(lon, lat, z, m)
                    }
                };

                Ok(waypoint)
            })
            .collect()
    }

    /// Calculate the bounding box that encloses this trajectory.
    pub fn bounding_box(&self) -> BboxQuery {
        let mut west = f64::MAX;
        let mut south = f64::MAX;
        let mut east = f64::MIN;
        let mut north = f64::MIN;

        for wp in &self.waypoints {
            west = west.min(wp.lon);
            east = east.max(wp.lon);
            south = south.min(wp.lat);
            north = north.max(wp.lat);
        }

        BboxQuery {
            west: west.max(-180.0),
            south: south.max(-90.0),
            east: east.min(180.0),
            north: north.min(90.0),
        }
    }

    /// Get the number of waypoints in the trajectory.
    pub fn len(&self) -> usize {
        self.waypoints.len()
    }

    /// Check if the trajectory is empty.
    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty()
    }

    /// Calculate approximate total path length in meters using Haversine distance.
    pub fn path_length_meters(&self) -> f64 {
        if self.waypoints.len() < 2 {
            return 0.0;
        }

        let mut total = 0.0;
        for i in 1..self.waypoints.len() {
            let prev = &self.waypoints[i - 1];
            let curr = &self.waypoints[i];
            total += RadiusQuery::haversine_distance(prev.lon, prev.lat, curr.lon, curr.lat);
        }
        total
    }

    /// Get the time range covered by this trajectory (if M coordinates present).
    ///
    /// Returns (min_epoch, max_epoch) in seconds.
    pub fn time_range(&self) -> Option<(i64, i64)> {
        let times: Vec<i64> = self.waypoints.iter()
            .filter_map(|wp| wp.m)
            .collect();

        if times.is_empty() {
            return None;
        }

        let min = *times.iter().min().unwrap();
        let max = *times.iter().max().unwrap();
        Some((min, max))
    }

    /// Get the vertical level range covered by this trajectory (if Z coordinates present).
    ///
    /// Returns (min_z, max_z).
    pub fn z_range(&self) -> Option<(f64, f64)> {
        let z_values: Vec<f64> = self.waypoints.iter()
            .filter_map(|wp| wp.z)
            .collect();

        if z_values.is_empty() {
            return None;
        }

        let min = z_values.iter().cloned().fold(f64::MAX, f64::min);
        let max = z_values.iter().cloned().fold(f64::MIN, f64::max);
        Some((min, max))
    }

    /// Check if the trajectory coordinates include height (Z).
    pub fn has_embedded_z(&self) -> bool {
        self.line_type.map_or(false, |lt| lt.has_z())
    }

    /// Check if the trajectory coordinates include time (M).
    pub fn has_embedded_m(&self) -> bool {
        self.line_type.map_or(false, |lt| lt.has_m())
    }
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
    fn test_datetime_list() {
        let dt =
            DateTimeQuery::parse("2024-12-29T12:00:00Z,2024-12-29T13:00:00Z,2024-12-29T14:00:00Z")
                .unwrap();
        if let DateTimeQuery::List(times) = dt {
            assert_eq!(times.len(), 3);
            assert_eq!(times[0], "2024-12-29T12:00:00Z");
            assert_eq!(times[1], "2024-12-29T13:00:00Z");
            assert_eq!(times[2], "2024-12-29T14:00:00Z");
        } else {
            panic!("Expected list, got {:?}", dt);
        }
    }

    #[test]
    fn test_datetime_list_with_spaces() {
        let dt = DateTimeQuery::parse(
            "2024-12-29T12:00:00Z , 2024-12-29T13:00:00Z , 2024-12-29T14:00:00Z",
        )
        .unwrap();
        if let DateTimeQuery::List(times) = dt {
            assert_eq!(times.len(), 3);
            assert_eq!(times[0], "2024-12-29T12:00:00Z");
        } else {
            panic!("Expected list");
        }
    }

    #[test]
    fn test_datetime_single_becomes_instant() {
        let dt = DateTimeQuery::parse("2024-12-29T12:00:00Z,").unwrap();
        assert!(matches!(dt, DateTimeQuery::Instant(_)));
    }

    #[test]
    fn test_datetime_to_vec() {
        let instant = DateTimeQuery::Instant("2024-12-29T12:00:00Z".to_string());
        assert_eq!(instant.to_vec(), vec!["2024-12-29T12:00:00Z"]);

        let list = DateTimeQuery::List(vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
        ]);
        assert_eq!(
            list.to_vec(),
            vec!["2024-12-29T12:00:00Z", "2024-12-29T13:00:00Z"]
        );

        let interval = DateTimeQuery::Interval {
            start: Some("2024-12-29T00:00:00Z".to_string()),
            end: Some("2024-12-29T23:59:59Z".to_string()),
        };
        assert_eq!(
            interval.to_vec(),
            vec!["2024-12-29T00:00:00Z", "2024-12-29T23:59:59Z"]
        );
    }

    #[test]
    fn test_datetime_is_multi_time() {
        let instant = DateTimeQuery::Instant("2024-12-29T12:00:00Z".to_string());
        assert!(!instant.is_multi_time());

        let list = DateTimeQuery::List(vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
        ]);
        assert!(list.is_multi_time());

        let interval = DateTimeQuery::Interval {
            start: Some("2024-12-29T00:00:00Z".to_string()),
            end: Some("2024-12-29T23:59:59Z".to_string()),
        };
        assert!(interval.is_multi_time());
    }

    #[test]
    fn test_datetime_expand_open_end() {
        // Test expanding an open-ended interval against available times
        let available = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
            "2024-12-29T15:00:00Z".to_string(),
            "2024-12-29T16:00:00Z".to_string(),
        ];

        // Open-ended: from 13:00 to the end
        let open_end = DateTimeQuery::parse("2024-12-29T13:00:00Z/..").unwrap();
        let expanded = open_end.expand_against_available_times(&available);
        assert_eq!(expanded.len(), 4); // 13:00, 14:00, 15:00, 16:00
        assert_eq!(expanded[0], "2024-12-29T13:00:00Z");
        assert_eq!(expanded[3], "2024-12-29T16:00:00Z");
    }

    #[test]
    fn test_datetime_expand_open_start() {
        let available = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
            "2024-12-29T15:00:00Z".to_string(),
        ];

        // Open-start: from beginning to 14:00
        let open_start = DateTimeQuery::parse("../2024-12-29T14:00:00Z").unwrap();
        let expanded = open_start.expand_against_available_times(&available);
        assert_eq!(expanded.len(), 3); // 12:00, 13:00, 14:00
        assert_eq!(expanded[0], "2024-12-29T12:00:00Z");
        assert_eq!(expanded[2], "2024-12-29T14:00:00Z");
    }

    #[test]
    fn test_datetime_expand_closed_interval() {
        let available = vec![
            "2024-12-29T12:00:00Z".to_string(),
            "2024-12-29T13:00:00Z".to_string(),
            "2024-12-29T14:00:00Z".to_string(),
            "2024-12-29T15:00:00Z".to_string(),
        ];

        // Closed interval: from 13:00 to 14:00
        let closed = DateTimeQuery::parse("2024-12-29T13:00:00Z/2024-12-29T14:00:00Z").unwrap();
        let expanded = closed.expand_against_available_times(&available);
        assert_eq!(expanded.len(), 2); // 13:00, 14:00
        assert_eq!(expanded[0], "2024-12-29T13:00:00Z");
        assert_eq!(expanded[1], "2024-12-29T14:00:00Z");
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

    #[test]
    fn test_parse_polygon() {
        let polygon =
            AreaQuery::parse_polygon("POLYGON((-100 35, -98 35, -98 37, -100 37, -100 35))")
                .unwrap();
        assert_eq!(polygon.len(), 5);
        assert_eq!(polygon[0], (-100.0, 35.0));
        assert_eq!(polygon[1], (-98.0, 35.0));
        assert_eq!(polygon[2], (-98.0, 37.0));
        assert_eq!(polygon[3], (-100.0, 37.0));
        assert_eq!(polygon[4], (-100.0, 35.0)); // Closed ring
    }

    #[test]
    fn test_parse_polygon_lowercase() {
        let polygon =
            AreaQuery::parse_polygon("polygon((-100 35, -98 35, -98 37, -100 37, -100 35))")
                .unwrap();
        assert_eq!(polygon.len(), 5);
    }

    #[test]
    fn test_parse_polygon_invalid() {
        // Not enough points
        let result = AreaQuery::parse_polygon("POLYGON((-100 35, -98 35, -100 35))");
        assert!(result.is_err());

        // Missing parentheses
        let result = AreaQuery::parse_polygon("POLYGON(-100 35, -98 35, -98 37, -100 37, -100 35)");
        assert!(result.is_err());

        // Not a polygon
        let result = AreaQuery::parse_polygon("POINT(-100 35)");
        assert!(result.is_err());
    }

    #[test]
    fn test_area_query_bbox() {
        let area_query = AreaQuery {
            polygon: vec![
                (-100.0, 35.0),
                (-98.0, 35.0),
                (-98.0, 37.0),
                (-100.0, 37.0),
                (-100.0, 35.0),
            ],
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        };

        let bbox = area_query.bbox();
        assert_eq!(bbox.west, -100.0);
        assert_eq!(bbox.east, -98.0);
        assert_eq!(bbox.south, 35.0);
        assert_eq!(bbox.north, 37.0);
    }

    #[test]
    fn test_area_query_contains_point() {
        let area_query = AreaQuery {
            polygon: vec![
                (-100.0, 35.0),
                (-98.0, 35.0),
                (-98.0, 37.0),
                (-100.0, 37.0),
                (-100.0, 35.0),
            ],
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        };

        // Point inside
        assert!(area_query.contains_point(-99.0, 36.0));

        // Point outside
        assert!(!area_query.contains_point(-101.0, 36.0));
        assert!(!area_query.contains_point(-99.0, 38.0));

        // Point on edge (behavior may vary)
        // We test a clearly inside point near the edge
        assert!(area_query.contains_point(-99.99, 35.01));
    }

    #[test]
    fn test_area_query_area() {
        let area_query = AreaQuery {
            polygon: vec![
                (-100.0, 35.0),
                (-98.0, 35.0),
                (-98.0, 37.0),
                (-100.0, 37.0),
                (-100.0, 35.0),
            ],
            z: None,
            datetime: None,
            parameter_names: None,
            crs: None,
        };

        let area = area_query.area_sq_degrees();
        // 2 degrees x 2 degrees = 4 sq degrees
        assert!((area - 4.0).abs() < 0.01);
    }

    // =========== RadiusQuery tests ===========

    #[test]
    fn test_distance_unit_parse() {
        assert_eq!(DistanceUnit::parse("km").unwrap(), DistanceUnit::Kilometers);
        assert_eq!(DistanceUnit::parse("KM").unwrap(), DistanceUnit::Kilometers);
        assert_eq!(DistanceUnit::parse("kilometers").unwrap(), DistanceUnit::Kilometers);
        assert_eq!(DistanceUnit::parse("mi").unwrap(), DistanceUnit::Miles);
        assert_eq!(DistanceUnit::parse("miles").unwrap(), DistanceUnit::Miles);
        assert_eq!(DistanceUnit::parse("m").unwrap(), DistanceUnit::Meters);
        assert_eq!(DistanceUnit::parse("meters").unwrap(), DistanceUnit::Meters);
        assert_eq!(DistanceUnit::parse("nm").unwrap(), DistanceUnit::NauticalMiles);
        assert_eq!(DistanceUnit::parse("nautical_miles").unwrap(), DistanceUnit::NauticalMiles);
        
        assert!(DistanceUnit::parse("invalid").is_err());
    }

    #[test]
    fn test_distance_unit_conversions() {
        // 1 km = 1000 m
        assert!((DistanceUnit::Kilometers.to_meters(1.0) - 1000.0).abs() < 0.01);
        
        // 1 mile = 1609.344 m
        assert!((DistanceUnit::Miles.to_meters(1.0) - 1609.344).abs() < 0.01);
        
        // 1 m = 1 m
        assert!((DistanceUnit::Meters.to_meters(1.0) - 1.0).abs() < 0.01);
        
        // 1 nm = 1852 m
        assert!((DistanceUnit::NauticalMiles.to_meters(1.0) - 1852.0).abs() < 0.01);
        
        // km conversion
        assert!((DistanceUnit::Miles.to_kilometers(1.0) - 1.609344).abs() < 0.001);
    }

    #[test]
    fn test_radius_query_parse_within() {
        assert!((RadiusQuery::parse_within("100").unwrap() - 100.0).abs() < 0.01);
        assert!((RadiusQuery::parse_within("50.5").unwrap() - 50.5).abs() < 0.01);
        assert!((RadiusQuery::parse_within(" 25 ").unwrap() - 25.0).abs() < 0.01);
        
        // Invalid values
        assert!(RadiusQuery::parse_within("abc").is_err());
        assert!(RadiusQuery::parse_within("-10").is_err());
        assert!(RadiusQuery::parse_within("0").is_err());
    }

    #[test]
    fn test_radius_query_new() {
        let query = RadiusQuery::new(-97.5, 35.2, 100.0, DistanceUnit::Kilometers);
        assert_eq!(query.center_lon, -97.5);
        assert_eq!(query.center_lat, 35.2);
        assert!((query.radius_meters - 100_000.0).abs() < 0.01);
        assert!((query.radius_km() - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_haversine_distance() {
        // Distance from London to Paris is approximately 344 km
        let london = (-0.1276, 51.5074);
        let paris = (2.3522, 48.8566);
        
        let distance_m = RadiusQuery::haversine_distance(london.0, london.1, paris.0, paris.1);
        let distance_km = distance_m / 1000.0;
        
        // Should be approximately 344 km (allow some tolerance)
        assert!((distance_km - 344.0).abs() < 5.0, "Distance was {} km", distance_km);
    }

    #[test]
    fn test_haversine_distance_same_point() {
        let distance = RadiusQuery::haversine_distance(-97.5, 35.2, -97.5, 35.2);
        assert!(distance.abs() < 0.001);
    }

    #[test]
    fn test_radius_query_contains_point() {
        // 100 km radius around Oklahoma City (-97.5, 35.5)
        let query = RadiusQuery::new(-97.5, 35.5, 100.0, DistanceUnit::Kilometers);
        
        // Center point should be inside
        assert!(query.contains_point(-97.5, 35.5));
        
        // Point 50 km away should be inside (approximately 0.5 degrees at this latitude)
        assert!(query.contains_point(-97.0, 35.5));
        
        // Point 200 km away should be outside
        assert!(!query.contains_point(-95.0, 35.5));
    }

    #[test]
    fn test_radius_query_bounding_box() {
        // 100 km radius around the equator
        let query = RadiusQuery::new(0.0, 0.0, 100.0, DistanceUnit::Kilometers);
        let bbox = query.bounding_box();
        
        // At the equator, 100 km ≈ 0.9 degrees
        assert!(bbox.west < -0.8);
        assert!(bbox.east > 0.8);
        assert!(bbox.south < -0.8);
        assert!(bbox.north > 0.8);
        
        // Should be symmetric around center
        assert!((bbox.west + bbox.east).abs() < 0.01);
        assert!((bbox.south + bbox.north).abs() < 0.01);
    }

    #[test]
    fn test_radius_query_bounding_box_high_latitude() {
        // 100 km radius at 60° latitude - longitude degrees should be wider
        let query = RadiusQuery::new(0.0, 60.0, 100.0, DistanceUnit::Kilometers);
        let bbox = query.bounding_box();
        
        // At 60°N, 100 km ≈ 1.8 degrees longitude (twice as much as at equator)
        assert!(bbox.west < -1.5);
        assert!(bbox.east > 1.5);
        
        // Latitude should still be ~0.9 degrees
        assert!((bbox.north - 60.0 - 0.9).abs() < 0.1);
    }

    // =========== TrajectoryQuery tests ===========

    #[test]
    fn test_parse_linestring_2d() {
        let result = TrajectoryQuery::parse_coords("LINESTRING(-3.53 50.72, -3.35 50.92, -3.11 51.02)").unwrap();
        assert_eq!(result.line_type, LineStringType::LineString);
        assert_eq!(result.waypoints.len(), 3);
        assert!(!result.is_multi);
        
        assert_eq!(result.waypoints[0].lon, -3.53);
        assert_eq!(result.waypoints[0].lat, 50.72);
        assert!(result.waypoints[0].z.is_none());
        assert!(result.waypoints[0].m.is_none());
    }

    #[test]
    fn test_parse_linestring_lowercase() {
        let result = TrajectoryQuery::parse_coords("linestring(-3.53 50.72, -3.35 50.92)").unwrap();
        assert_eq!(result.line_type, LineStringType::LineString);
        assert_eq!(result.waypoints.len(), 2);
    }

    #[test]
    fn test_parse_linestringz() {
        let result = TrajectoryQuery::parse_coords("LINESTRINGZ(-3.53 50.72 100, -3.35 50.92 200, -3.11 51.02 300)").unwrap();
        assert_eq!(result.line_type, LineStringType::LineStringZ);
        assert_eq!(result.waypoints.len(), 3);
        
        assert_eq!(result.waypoints[0].lon, -3.53);
        assert_eq!(result.waypoints[0].lat, 50.72);
        assert_eq!(result.waypoints[0].z, Some(100.0));
        assert!(result.waypoints[0].m.is_none());
        
        assert_eq!(result.waypoints[2].z, Some(300.0));
    }

    #[test]
    fn test_parse_linestringm() {
        // Unix epoch times (e.g., 1560507000 = 2019-06-14T07:30:00Z)
        let result = TrajectoryQuery::parse_coords("LINESTRINGM(-3.53 50.72 1560507000, -3.35 50.92 1560508800)").unwrap();
        assert_eq!(result.line_type, LineStringType::LineStringM);
        assert_eq!(result.waypoints.len(), 2);
        
        assert_eq!(result.waypoints[0].lon, -3.53);
        assert_eq!(result.waypoints[0].lat, 50.72);
        assert!(result.waypoints[0].z.is_none());
        assert_eq!(result.waypoints[0].m, Some(1560507000));
        
        assert_eq!(result.waypoints[1].m, Some(1560508800));
    }

    #[test]
    fn test_parse_linestringzm() {
        let result = TrajectoryQuery::parse_coords("LINESTRINGZM(-3.53 50.72 100 1560507000, -3.35 50.92 200 1560508800)").unwrap();
        assert_eq!(result.line_type, LineStringType::LineStringZM);
        assert_eq!(result.waypoints.len(), 2);
        
        assert_eq!(result.waypoints[0].lon, -3.53);
        assert_eq!(result.waypoints[0].lat, 50.72);
        assert_eq!(result.waypoints[0].z, Some(100.0));
        assert_eq!(result.waypoints[0].m, Some(1560507000));
    }

    #[test]
    fn test_parse_multilinestring() {
        let result = TrajectoryQuery::parse_coords("MULTILINESTRING((-3.53 50.72, -3.35 50.92), (-3.11 51.02, -2.85 51.42))").unwrap();
        assert_eq!(result.line_type, LineStringType::LineString);
        assert_eq!(result.waypoints.len(), 4); // Concatenated
        assert!(result.is_multi);
        
        assert_eq!(result.waypoints[0].lon, -3.53);
        assert_eq!(result.waypoints[2].lon, -3.11);
    }

    #[test]
    fn test_parse_multilinestringz() {
        let result = TrajectoryQuery::parse_coords("MULTILINESTRINGZ((-3.53 50.72 100, -3.35 50.92 200), (-3.11 51.02 300, -2.85 51.42 400))").unwrap();
        assert_eq!(result.line_type, LineStringType::LineStringZ);
        assert_eq!(result.waypoints.len(), 4);
        assert!(result.is_multi);
        
        assert_eq!(result.waypoints[0].z, Some(100.0));
        assert_eq!(result.waypoints[3].z, Some(400.0));
    }

    #[test]
    fn test_parse_linestring_invalid_not_linestring() {
        let result = TrajectoryQuery::parse_coords("POINT(-3.53 50.72)");
        assert!(result.is_err());
        
        let result = TrajectoryQuery::parse_coords("POLYGON((-3.53 50.72, -3.35 50.92, -3.11 51.02, -3.53 50.72))");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_linestring_invalid_coords() {
        // Wrong number of coordinates for type
        let result = TrajectoryQuery::parse_coords("LINESTRING(-3.53 50.72 100, -3.35 50.92 200)"); // 3 coords but not Z type
        assert!(result.is_err());
        
        let result = TrajectoryQuery::parse_coords("LINESTRINGZ(-3.53 50.72, -3.35 50.92)"); // Missing Z
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_linestring_out_of_range() {
        // Longitude out of range
        let result = TrajectoryQuery::parse_coords("LINESTRING(-200 50.72, -3.35 50.92)");
        assert!(matches!(result, Err(CoordinateParseError::OutOfRange(_))));
        
        // Latitude out of range
        let result = TrajectoryQuery::parse_coords("LINESTRING(-3.53 100, -3.35 50.92)");
        assert!(matches!(result, Err(CoordinateParseError::OutOfRange(_))));
    }

    #[test]
    fn test_trajectory_query_bounding_box() {
        let mut query = TrajectoryQuery::new();
        query.waypoints = vec![
            TrajectoryWaypoint::new_2d(-100.0, 35.0),
            TrajectoryWaypoint::new_2d(-98.0, 37.0),
            TrajectoryWaypoint::new_2d(-96.0, 36.0),
        ];
        
        let bbox = query.bounding_box();
        assert_eq!(bbox.west, -100.0);
        assert_eq!(bbox.east, -96.0);
        assert_eq!(bbox.south, 35.0);
        assert_eq!(bbox.north, 37.0);
    }

    #[test]
    fn test_trajectory_query_path_length() {
        let mut query = TrajectoryQuery::new();
        // Approximately 100 km apart at mid-latitudes
        query.waypoints = vec![
            TrajectoryWaypoint::new_2d(0.0, 0.0),
            TrajectoryWaypoint::new_2d(1.0, 0.0), // ~111 km at equator
        ];
        
        let length = query.path_length_meters();
        assert!((length - 111_000.0).abs() < 1000.0); // Within 1 km
    }

    #[test]
    fn test_trajectory_query_time_range() {
        let mut query = TrajectoryQuery::new();
        query.waypoints = vec![
            TrajectoryWaypoint::new_3d_m(-3.53, 50.72, 1560507000),
            TrajectoryWaypoint::new_3d_m(-3.35, 50.92, 1560510600),
            TrajectoryWaypoint::new_3d_m(-3.11, 51.02, 1560508800),
        ];
        
        let (min, max) = query.time_range().unwrap();
        assert_eq!(min, 1560507000);
        assert_eq!(max, 1560510600);
    }

    #[test]
    fn test_trajectory_query_z_range() {
        let mut query = TrajectoryQuery::new();
        query.waypoints = vec![
            TrajectoryWaypoint::new_3d_z(-3.53, 50.72, 100.0),
            TrajectoryWaypoint::new_3d_z(-3.35, 50.92, 500.0),
            TrajectoryWaypoint::new_3d_z(-3.11, 51.02, 300.0),
        ];
        
        let (min, max) = query.z_range().unwrap();
        assert_eq!(min, 100.0);
        assert_eq!(max, 500.0);
    }

    #[test]
    fn test_linestring_type_has_z_m() {
        assert!(!LineStringType::LineString.has_z());
        assert!(!LineStringType::LineString.has_m());
        
        assert!(LineStringType::LineStringZ.has_z());
        assert!(!LineStringType::LineStringZ.has_m());
        
        assert!(!LineStringType::LineStringM.has_z());
        assert!(LineStringType::LineStringM.has_m());
        
        assert!(LineStringType::LineStringZM.has_z());
        assert!(LineStringType::LineStringZM.has_m());
    }
}
