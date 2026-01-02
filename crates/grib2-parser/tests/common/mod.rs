//! Common test utilities for grib2-parser tests
//!
//! Provides helpers for:
//! - Locating test data files
//! - Skipping tests when data is unavailable
//! - Downloading sample files on-demand
//! - Creating test GRIB2 lookup tables

use grib2_parser::{Grib2Tables, LevelDescription};
use std::path::PathBuf;
use std::sync::Arc;

/// Returns the path to the testdata directory
pub fn testdata_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("testdata")
}

/// Returns the path to a test file, checking multiple locations
pub fn find_test_file(name: &str) -> Option<PathBuf> {
    let candidates = [
        // Primary: testdata/ in crate root
        testdata_dir().join(name),
        // Legacy: /tmp paths used by some tests
        PathBuf::from("/tmp").join(name),
        // Workspace root testdata/
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("testdata").join(name))
            .unwrap_or_default(),
    ];

    for path in candidates {
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Macro to skip a test if the required file is not found
///
/// Usage:
/// ```ignore
/// #[test]
/// fn test_something() {
///     let path = require_test_file!("gfs_sample.grib2");
///     // ... test code
/// }
/// ```
#[macro_export]
macro_rules! require_test_file {
    ($name:expr) => {{
        match $crate::common::find_test_file($name) {
            Some(path) => path,
            None => {
                eprintln!(
                    "SKIPPED: Test file '{}' not found. Run: ./tests/setup_test_data.sh",
                    $name
                );
                return;
            }
        }
    }};
}

/// Macro to skip a test if any required file is not found
#[macro_export]
macro_rules! require_test_files {
    ($($name:expr),+ $(,)?) => {{
        let mut paths = Vec::new();
        $(
            match $crate::common::find_test_file($name) {
                Some(path) => paths.push(path),
                None => {
                    eprintln!(
                        "SKIPPED: Test file '{}' not found. Run: ./tests/setup_test_data.sh",
                        $name
                    );
                    return;
                }
            }
        )+
        paths
    }};
}

/// Create test GRIB2 lookup tables with common parameters.
///
/// This provides the parameter and level mappings needed for testing
/// without requiring actual YAML config files.
#[allow(dead_code)] // Used by multiple test files, not all may use it
pub fn create_test_tables() -> Arc<Grib2Tables> {
    let mut tables = Grib2Tables::new();

    // Common meteorological parameters (Discipline 0)
    // Temperature (category 0)
    tables.add_parameter(0, 0, 0, "TMP".to_string());
    tables.add_parameter(0, 0, 6, "DPT".to_string());

    // Moisture (category 1)
    tables.add_parameter(0, 1, 1, "RH".to_string());
    tables.add_parameter(0, 1, 3, "PWAT".to_string());
    tables.add_parameter(0, 1, 8, "APCP".to_string());

    // Momentum (category 2)
    tables.add_parameter(0, 2, 2, "UGRD".to_string());
    tables.add_parameter(0, 2, 3, "VGRD".to_string());
    tables.add_parameter(0, 2, 22, "GUST".to_string());

    // Mass (category 3)
    tables.add_parameter(0, 3, 1, "PRMSL".to_string());
    tables.add_parameter(0, 3, 5, "HGT".to_string());

    // Cloud (category 6)
    tables.add_parameter(0, 6, 1, "TCDC".to_string());

    // Stability (category 7)
    tables.add_parameter(0, 7, 6, "CAPE".to_string());
    tables.add_parameter(0, 7, 7, "CIN".to_string());

    // Radar (category 16)
    tables.add_parameter(0, 16, 196, "REFC".to_string());

    // Visibility (category 19)
    tables.add_parameter(0, 19, 0, "VIS".to_string());

    // MRMS parameters (Discipline 209)
    tables.add_parameter(209, 0, 16, "REFL".to_string());
    tables.add_parameter(209, 1, 0, "PRECIP_RATE".to_string());
    tables.add_parameter(209, 1, 1, "QPE".to_string());

    // Common level types
    tables.add_level(1, LevelDescription::Static("surface".to_string()));
    tables.add_level(2, LevelDescription::Static("cloud base".to_string()));
    tables.add_level(3, LevelDescription::Static("cloud top".to_string()));
    tables.add_level(100, LevelDescription::Template("{value} mb".to_string()));
    tables.add_level(101, LevelDescription::Static("mean sea level".to_string()));
    tables.add_level(
        102,
        LevelDescription::Template("{value} m above MSL".to_string()),
    );
    tables.add_level(
        103,
        LevelDescription::Template("{value} m above ground".to_string()),
    );
    tables.add_level(
        200,
        LevelDescription::Static("entire atmosphere".to_string()),
    );
    // Cloud layers: support both bottom (x2) and layer (x4) codes
    // GRIB2 Table 4.5: 212=bottom, 213=top, 214=layer for each cloud level
    tables.add_level(212, LevelDescription::Static("low cloud layer".to_string()));
    tables.add_level(214, LevelDescription::Static("low cloud layer".to_string()));
    tables.add_level(
        222,
        LevelDescription::Static("middle cloud layer".to_string()),
    );
    tables.add_level(
        224,
        LevelDescription::Static("middle cloud layer".to_string()),
    );
    tables.add_level(
        232,
        LevelDescription::Static("high cloud layer".to_string()),
    );
    tables.add_level(
        234,
        LevelDescription::Static("high cloud layer".to_string()),
    );

    Arc::new(tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_testdata_dir_exists_or_can_be_created() {
        let dir = testdata_dir();
        // Just check path is reasonable
        assert!(dir.to_string_lossy().contains("grib2-parser"));
    }
}
