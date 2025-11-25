//! GRIB2 parser implementation (WMO FM 92 GRIB Edition 2).
//! 
//! This crate provides a pure Rust implementation for parsing GRIB2 files,
//! the standard format for meteorological data exchange.

pub mod sections;
pub mod templates;
pub mod unpacking;

// TODO: Implement full GRIB2 parsing
// For now, export placeholder types

pub struct Grib2Message {
    // Placeholder
}

pub struct Grib2Reader {
    // Placeholder
}

impl Grib2Reader {
    pub fn new(_data: bytes::Bytes) -> Self {
        Self {}
    }
}
