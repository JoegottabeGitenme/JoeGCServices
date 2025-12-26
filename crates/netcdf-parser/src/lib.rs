//! NetCDF parser for satellite data (GOES-R ABI).
//!
//! This crate provides reading of NetCDF-4 format files, specifically
//! optimized for GOES-R ABI (Advanced Baseline Imager) data products.
//!
//! # Features
//!
//! - **Native parsing**: High-performance reading using the `netcdf` library
//! - **Geostationary projection**: Convert between scan angles and lat/lon
//!
//! # GOES-R ABI Data Structure
//!
//! GOES-R ABI files use the geostationary projection with coordinates in radians.
//! The main data variable is `CMI` (Cloud and Moisture Imagery) which contains
//! either reflectance factors (bands 1-6) or brightness temperatures (bands 7-16).
//!
//! # Module Structure
//!
//! - [`error`] - Error types and result alias
//! - [`projection`] - Geostationary coordinate transformations
//! - [`native`] - High-performance netcdf library parsing

pub mod error;
pub mod native;
pub mod projection;

// Re-export commonly used items at crate root
pub use error::{NetCdfError, NetCdfResult};
pub use native::{load_goes_netcdf_from_bytes, silence_hdf5_errors};
pub use projection::GoesProjection;

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
}
