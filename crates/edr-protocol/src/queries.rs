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
}
