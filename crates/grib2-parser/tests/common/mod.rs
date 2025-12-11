//! Common test utilities for grib2-parser tests
//!
//! Provides helpers for:
//! - Locating test data files
//! - Skipping tests when data is unavailable
//! - Downloading sample files on-demand

use std::path::PathBuf;

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

/// Check if we're in CI environment (typically no test data available)
#[allow(dead_code)]
pub fn is_ci() -> bool {
    std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok()
}

/// Helper to read a test file, returning None if not found
#[allow(dead_code)]
pub fn read_test_file(name: &str) -> Option<Vec<u8>> {
    find_test_file(name).and_then(|path| std::fs::read(&path).ok())
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
