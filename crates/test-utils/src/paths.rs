//! Path utilities for locating test data files.
//!
//! This module provides functions to find test data files across multiple
//! potential locations, supporting both local development and CI environments.

use std::path::PathBuf;

/// Returns the workspace root directory.
///
/// This is determined by walking up from the current crate's manifest directory
/// until we find the workspace Cargo.toml.
pub fn workspace_root() -> PathBuf {
    // Start from the test-utils crate manifest dir
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(manifest_dir))
}

/// Returns the path to the testdata directory for a specific crate.
///
/// # Arguments
///
/// * `crate_name` - The name of the crate (e.g., "grib2-parser", "grid-processor")
///
/// # Returns
///
/// The path to `crates/{crate_name}/testdata/`
pub fn crate_testdata_dir(crate_name: &str) -> PathBuf {
    workspace_root()
        .join("crates")
        .join(crate_name)
        .join("testdata")
}

/// Returns the path to the service testdata directory.
///
/// # Arguments
///
/// * `service_name` - The name of the service (e.g., "wms-api", "ingester")
///
/// # Returns
///
/// The path to `services/{service_name}/testdata/`
pub fn service_testdata_dir(service_name: &str) -> PathBuf {
    workspace_root()
        .join("services")
        .join(service_name)
        .join("testdata")
}

/// Searches for a test file in multiple locations.
///
/// This function checks the following locations in order:
/// 1. Environment variable `TEST_DATA_DIR` (if set)
/// 2. `crates/grib2-parser/testdata/`
/// 3. `crates/grid-processor/testdata/`
/// 4. `crates/netcdf-parser/testdata/`
/// 5. `/tmp/` (legacy location)
///
/// # Arguments
///
/// * `name` - The filename to search for (e.g., "gfs_sample.grib2")
///
/// # Returns
///
/// `Some(PathBuf)` if the file is found, `None` otherwise.
pub fn find_test_file(name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    // Check environment variable first
    if let Ok(test_data_dir) = std::env::var("TEST_DATA_DIR") {
        candidates.push(PathBuf::from(test_data_dir).join(name));
    }

    // Check common testdata directories
    let root = workspace_root();
    candidates.extend([
        root.join("crates/grib2-parser/testdata").join(name),
        root.join("crates/grid-processor/testdata").join(name),
        root.join("crates/netcdf-parser/testdata").join(name),
        root.join("testdata").join(name), // workspace-level testdata
        PathBuf::from("/tmp").join(name), // legacy location
    ]);

    for path in candidates {
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// Searches for a test file in a specific crate's testdata directory.
///
/// # Arguments
///
/// * `crate_name` - The crate name (e.g., "grib2-parser")
/// * `name` - The filename to search for
///
/// # Returns
///
/// `Some(PathBuf)` if the file is found, `None` otherwise.
pub fn find_crate_test_file(crate_name: &str, name: &str) -> Option<PathBuf> {
    let path = crate_testdata_dir(crate_name).join(name);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Creates a temporary directory for test output.
///
/// The directory is automatically cleaned up when the returned `TempDir` is dropped.
///
/// # Returns
///
/// A `tempfile::TempDir` that will be automatically cleaned up.
pub fn temp_test_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("Failed to create temporary test directory")
}

/// Creates a temporary directory with a specific prefix.
///
/// # Arguments
///
/// * `prefix` - A prefix for the directory name (e.g., "zarr_test")
///
/// # Returns
///
/// A `tempfile::TempDir` with the specified prefix.
pub fn temp_test_dir_with_prefix(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("Failed to create temporary test directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_root_is_valid() {
        let root = workspace_root();
        // Should contain Cargo.toml at workspace level
        assert!(
            root.join("Cargo.toml").exists(),
            "Workspace root should contain Cargo.toml: {:?}",
            root
        );
    }

    #[test]
    fn test_crate_testdata_dir() {
        let dir = crate_testdata_dir("grib2-parser");
        assert!(dir.to_string_lossy().contains("grib2-parser"));
        assert!(dir.to_string_lossy().contains("testdata"));
    }

    #[test]
    fn test_temp_test_dir() {
        let dir = temp_test_dir();
        assert!(dir.path().exists());
        // Dir is cleaned up when dropped
    }

    #[test]
    fn test_temp_test_dir_with_prefix() {
        let dir = temp_test_dir_with_prefix("weather_test_");
        let path_str = dir.path().to_string_lossy();
        assert!(path_str.contains("weather_test_"));
    }
}
