//! Shared test utilities for the weather-wms workspace.
//!
//! This crate provides common testing infrastructure including:
//! - Test data path helpers
//! - Skip macros for optional test data
//! - Grid data generators
//! - Common test fixtures
//!
//! # Usage
//!
//! Add to your crate's `Cargo.toml`:
//!
//! ```toml
//! [dev-dependencies]
//! test-utils = { path = "../test-utils" }
//! ```
//!
//! Then import in your tests:
//!
//! ```ignore
//! use test_utils::{require_test_file, fixtures};
//! ```

pub mod fixtures;
pub mod generators;
pub mod paths;

// Re-export commonly used items at the crate root
pub use fixtures::*;
pub use generators::*;
pub use paths::*;

/// Macro to skip a test if the required file is not found.
///
/// This is useful for tests that depend on external data files that may not
/// be present in all environments (e.g., CI without large test data).
///
/// # Usage
///
/// ```ignore
/// use test_utils::require_test_file;
///
/// #[test]
/// fn test_gfs_parsing() {
///     let path = require_test_file!("gfs_sample.grib2");
///     // Test code using path...
/// }
/// ```
///
/// If the file is not found, the test will print a skip message and return early.
#[macro_export]
macro_rules! require_test_file {
    ($name:expr) => {{
        match $crate::find_test_file($name) {
            Some(path) => path,
            None => {
                eprintln!(
                    "SKIPPED: Test file '{}' not found. Download test data or set TEST_DATA_DIR.",
                    $name
                );
                return;
            }
        }
    }};
}

/// Macro to skip a test if any of the required files are not found.
///
/// # Usage
///
/// ```ignore
/// use test_utils::require_test_files;
///
/// #[test]
/// fn test_multiple_files() {
///     let paths = require_test_files!("file1.grib2", "file2.grib2");
///     // paths is Vec<PathBuf>
/// }
/// ```
#[macro_export]
macro_rules! require_test_files {
    ($($name:expr),+ $(,)?) => {{
        let mut paths = Vec::new();
        $(
            match $crate::find_test_file($name) {
                Some(path) => paths.push(path),
                None => {
                    eprintln!(
                        "SKIPPED: Test file '{}' not found. Download test data or set TEST_DATA_DIR.",
                        $name
                    );
                    return;
                }
            }
        )+
        paths
    }};
}

/// Macro for approximate floating-point equality assertions.
///
/// # Usage
///
/// ```ignore
/// use test_utils::assert_approx_eq;
///
/// assert_approx_eq!(1.0001_f64, 1.0_f64, 0.001_f64); // passes
/// assert_approx_eq!(1.1_f32, 1.0_f32, 0.001_f32);    // fails
/// ```
#[macro_export]
macro_rules! assert_approx_eq {
    ($left:expr, $right:expr, $epsilon:expr) => {{
        let left: f64 = $left as f64;
        let right: f64 = $right as f64;
        let epsilon: f64 = $epsilon as f64;
        let diff = (left - right).abs();
        if diff > epsilon {
            panic!(
                "assertion failed: `(left ≈ right)`\n  left: `{:?}`,\n right: `{:?}`,\n  diff: `{:?}` > epsilon `{:?}`",
                left, right, diff, epsilon
            );
        }
    }};
}

/// Macro for approximate equality of coordinate pairs.
///
/// # Usage
///
/// ```ignore
/// use test_utils::assert_coords_approx_eq;
///
/// assert_coords_approx_eq!((1.0001, 2.0001), (1.0, 2.0), 0.001);
/// ```
#[macro_export]
macro_rules! assert_coords_approx_eq {
    (($x1:expr, $y1:expr), ($x2:expr, $y2:expr), $epsilon:expr) => {{
        $crate::assert_approx_eq!($x1, $x2, $epsilon);
        $crate::assert_approx_eq!($y1, $y2, $epsilon);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_approx_eq_passes() {
        assert_approx_eq!(1.0001, 1.0, 0.001);
        assert_approx_eq!(0.0, 0.0, 0.0001);
        assert_approx_eq!(-5.5, -5.500001, 0.0001);
    }

    #[test]
    #[should_panic(expected = "assertion failed")]
    fn test_assert_approx_eq_fails() {
        assert_approx_eq!(1.1, 1.0, 0.001);
    }

    #[test]
    fn test_assert_coords_approx_eq_passes() {
        assert_coords_approx_eq!((1.0001, 2.0001), (1.0, 2.0), 0.001);
    }
}
