//! Bounding box types and operations.

use serde::{Deserialize, Serialize};

/// A geographic or projected bounding box.
///
/// For geographic CRS (EPSG:4326), coordinates are in degrees.
/// For projected CRS (EPSG:3857, etc.), coordinates are in meters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl BoundingBox {
    /// Create a new bounding box from corner coordinates.
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// Parse a WMS BBOX parameter string: "minx,miny,maxx,maxy"
    pub fn from_wms_string(s: &str) -> Result<Self, BboxParseError> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 4 {
            return Err(BboxParseError::InvalidFormat(s.to_string()));
        }

        Ok(Self {
            min_x: parts[0]
                .parse()
                .map_err(|_| BboxParseError::InvalidNumber(parts[0].to_string()))?,
            min_y: parts[1]
                .parse()
                .map_err(|_| BboxParseError::InvalidNumber(parts[1].to_string()))?,
            max_x: parts[2]
                .parse()
                .map_err(|_| BboxParseError::InvalidNumber(parts[2].to_string()))?,
            max_y: parts[3]
                .parse()
                .map_err(|_| BboxParseError::InvalidNumber(parts[3].to_string()))?,
        })
    }

    /// Width of the bounding box in coordinate units.
    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    /// Height of the bounding box in coordinate units.
    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    /// Check if this bbox intersects another.
    pub fn intersects(&self, other: &BoundingBox) -> bool {
        self.min_x < other.max_x
            && self.max_x > other.min_x
            && self.min_y < other.max_y
            && self.max_y > other.min_y
    }

    /// Compute the intersection of two bounding boxes.
    pub fn intersection(&self, other: &BoundingBox) -> Option<BoundingBox> {
        if !self.intersects(other) {
            return None;
        }

        Some(BoundingBox {
            min_x: self.min_x.max(other.min_x),
            min_y: self.min_y.max(other.min_y),
            max_x: self.max_x.min(other.max_x),
            max_y: self.max_y.min(other.max_y),
        })
    }

    /// Check if a point is contained within this bbox.
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    /// Generate a cache key fragment for this bbox (quantized to avoid floating point issues).
    pub fn cache_key(&self) -> String {
        // Quantize to 6 decimal places for cache key stability
        format!(
            "{:.6}_{:.6}_{:.6}_{:.6}",
            self.min_x, self.min_y, self.max_x, self.max_y
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BboxParseError {
    #[error("Invalid BBOX format: {0}. Expected 'minx,miny,maxx,maxy'")]
    InvalidFormat(String),

    #[error("Invalid number in BBOX: {0}")]
    InvalidNumber(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wms_bbox() {
        let bbox = BoundingBox::from_wms_string("-125.0,24.0,-66.0,50.0").unwrap();
        assert_eq!(bbox.min_x, -125.0);
        assert_eq!(bbox.min_y, 24.0);
        assert_eq!(bbox.max_x, -66.0);
        assert_eq!(bbox.max_y, 50.0);
    }

    #[test]
    fn test_intersection() {
        let a = BoundingBox::new(0.0, 0.0, 10.0, 10.0);
        let b = BoundingBox::new(5.0, 5.0, 15.0, 15.0);
        let c = BoundingBox::new(20.0, 20.0, 30.0, 30.0);

        assert!(a.intersects(&b));
        assert!(!a.intersects(&c));

        let intersection = a.intersection(&b).unwrap();
        assert_eq!(intersection.min_x, 5.0);
        assert_eq!(intersection.min_y, 5.0);
        assert_eq!(intersection.max_x, 10.0);
        assert_eq!(intersection.max_y, 10.0);
    }
}
