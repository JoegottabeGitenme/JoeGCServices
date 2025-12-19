//! Native NetCDF parsing using the netcdf library.
//!
//! This module provides high-performance GOES file parsing using the native
//! netcdf library (which wraps HDF5). It's significantly faster than the
//! ncdump subprocess approach.
//!
//! # Performance Notes
//!
//! The netcdf library requires a file path (it wraps libnetcdf/HDF5 which need
//! file handles). When reading from bytes, we write to a temp file first.
//!
//! On Linux, we use `/dev/shm` (memory-backed tmpfs) to minimize I/O latency.
//! Benchmarks show this reduces temp file overhead from ~2-5ms to ~0.5-1ms.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;

use crate::error::{NetCdfError, NetCdfResult};
use crate::projection::GoesProjection;

/// Silence HDF5's automatic error printing to stderr.
///
/// The HDF5 C library prints verbose error messages to stderr even when errors
/// are handled gracefully by the Rust code (e.g., when checking for optional
/// attributes that don't exist). This creates confusing log spam like:
///
/// ```text
/// HDF5-DIAG: Error detected in HDF5 (1.10.8) thread 3:
///   #003: ../../../src/H5Adense.c line 397 in H5A__dense_open(): can't locate attribute in name index
/// ```
///
/// This function disables that output by calling H5Eset_auto2 with null handlers.
/// It only needs to be called once per process, but is safe to call multiple times.
///
/// **Important**: Call this function early in your program's startup (e.g., in main())
/// before any HDF5/NetCDF operations occur. If HDF5 is initialized before this is called,
/// the error silencing may not take effect for all operations.
pub fn silence_hdf5_errors() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        // SAFETY: H5Eset_auto2 is thread-safe and we're passing null pointers
        // to disable error output, which is a documented valid use.
        unsafe {
            hdf5_metno_sys::h5e::H5Eset_auto2(
                hdf5_metno_sys::h5e::H5E_DEFAULT,
                None,
                std::ptr::null_mut(),
            );
        }
    });
}

/// Load GOES NetCDF data directly from bytes using native netcdf library.
///
/// This is much faster than using ncdump subprocess.
///
/// # Returns
///
/// A tuple of `(data, width, height, projection, x_offset, y_offset, x_scale, y_scale)`:
/// - `data`: Scaled CMI values (reflectance or brightness temperature)
/// - `width`, `height`: Grid dimensions
/// - `projection`: GOES projection parameters
/// - `x_offset`, `y_offset`: Scan angle offsets (radians)
/// - `x_scale`, `y_scale`: Scan angle scale factors (radians/pixel)
pub fn load_goes_netcdf_from_bytes(
    data: &[u8],
) -> NetCdfResult<(Vec<f32>, usize, usize, GoesProjection, f32, f32, f32, f32)> {
    // Silence HDF5's verbose stderr output for missing attributes
    silence_hdf5_errors();

    // Use memory-backed filesystem on Linux for faster I/O
    let temp_dir = get_optimal_temp_dir();
    let temp_file = temp_dir.join(generate_temp_filename());

    // Write temp file
    let mut file = std::fs::File::create(&temp_file)?;
    file.write_all(data)?;
    drop(file);

    // Open with netcdf library
    let nc_file = netcdf::open(&temp_file)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to open NetCDF: {}", e)))?;

    // Get dimensions
    let width = nc_file
        .dimension("x")
        .ok_or_else(|| NetCdfError::MissingData("x dimension".to_string()))?
        .len();
    let height = nc_file
        .dimension("y")
        .ok_or_else(|| NetCdfError::MissingData("y dimension".to_string()))?
        .len();

    // Get CMI variable and its data
    let cmi_var = nc_file
        .variable("CMI")
        .ok_or_else(|| NetCdfError::MissingData("CMI variable".to_string()))?;

    // Read raw data as i16 using (..) to read all extents
    let raw_data: Vec<i16> = cmi_var
        .get_values(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read CMI: {}", e)))?;

    // Get attributes using get_value helper
    let scale_factor = get_f32_attr(&cmi_var, "scale_factor").unwrap_or(1.0);
    let add_offset = get_f32_attr(&cmi_var, "add_offset").unwrap_or(0.0);
    let fill_value = get_i16_attr(&cmi_var, "_FillValue").unwrap_or(-1);

    // Apply scale and offset
    let data: Vec<f32> = raw_data
        .iter()
        .map(|&val| {
            if val == fill_value {
                f32::NAN
            } else {
                val as f32 * scale_factor + add_offset
            }
        })
        .collect();

    // Get coordinate attributes
    let x_var = nc_file
        .variable("x")
        .ok_or_else(|| NetCdfError::MissingData("x variable".to_string()))?;
    let x_scale = get_f32_attr(&x_var, "scale_factor").unwrap_or(1.4e-05);
    let x_offset = get_f32_attr(&x_var, "add_offset").unwrap_or(-0.101353);

    let y_var = nc_file
        .variable("y")
        .ok_or_else(|| NetCdfError::MissingData("y variable".to_string()))?;
    let y_scale = get_f32_attr(&y_var, "scale_factor").unwrap_or(-1.4e-05);
    let y_offset = get_f32_attr(&y_var, "add_offset").unwrap_or(0.128233);

    // Get projection
    let proj_var = nc_file
        .variable("goes_imager_projection")
        .ok_or_else(|| NetCdfError::MissingData("goes_imager_projection variable".to_string()))?;

    let projection = GoesProjection {
        perspective_point_height: get_f64_attr(&proj_var, "perspective_point_height")
            .unwrap_or(35786023.0),
        semi_major_axis: get_f64_attr(&proj_var, "semi_major_axis").unwrap_or(6378137.0),
        semi_minor_axis: get_f64_attr(&proj_var, "semi_minor_axis").unwrap_or(6356752.31414),
        longitude_origin: get_f64_attr(&proj_var, "longitude_of_projection_origin")
            .unwrap_or(-75.0),
        latitude_origin: 0.0,
        sweep_angle_axis: "x".to_string(),
    };

    // Clean up
    let _ = std::fs::remove_file(&temp_file);

    Ok((
        data, width, height, projection, x_offset, y_offset, x_scale, y_scale,
    ))
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Get the optimal temp directory for NetCDF file operations.
///
/// On Linux, uses /dev/shm (memory-backed tmpfs) if available for faster I/O.
/// Falls back to the system temp directory on other platforms or if /dev/shm is unavailable.
fn get_optimal_temp_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;
        let shm_path = Path::new("/dev/shm");
        if shm_path.exists() && shm_path.is_dir() {
            // Verify we can write to /dev/shm
            let test_path = shm_path.join(format!(".netcdf_test_{}", std::process::id()));
            if std::fs::write(&test_path, b"test").is_ok() {
                let _ = std::fs::remove_file(&test_path);
                return shm_path.to_path_buf();
            }
        }
    }

    std::env::temp_dir()
}

/// Generate a unique temp file name for concurrent safety.
/// Uses process ID, thread ID, and a counter to ensure uniqueness.
fn generate_temp_filename() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let pid = std::process::id();
    let tid = std::thread::current().id();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("goes_native_{}_{:?}_{}.nc", pid, tid, count)
}

/// Check if a variable has an attribute with the given name.
/// This avoids HDF5 error spam when checking for optional attributes.
fn has_attr(var: &netcdf::Variable, name: &str) -> bool {
    var.attributes().any(|attr| attr.name() == name)
}

/// Helper to get f32 attribute.
fn get_f32_attr(var: &netcdf::Variable, name: &str) -> Option<f32> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    f32::try_from(attr_value).ok()
}

/// Helper to get f64 attribute.
fn get_f64_attr(var: &netcdf::Variable, name: &str) -> Option<f64> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    f64::try_from(attr_value).ok()
}

/// Helper to get i16 attribute.
fn get_i16_attr(var: &netcdf::Variable, name: &str) -> Option<i16> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    i16::try_from(attr_value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimal_temp_dir() {
        let dir = get_optimal_temp_dir();
        assert!(dir.exists(), "Temp dir should exist");
    }

    #[test]
    fn test_temp_filename_uniqueness() {
        let name1 = generate_temp_filename();
        let name2 = generate_temp_filename();
        assert_ne!(name1, name2, "Temp filenames should be unique");
    }
}
