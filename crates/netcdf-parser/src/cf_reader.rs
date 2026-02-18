//! Generic CF-convention NetCDF reader for regular lat/lon grids.
//!
//! This module reads CF-compliant NetCDF-4 files (like NLDAS-2, GLDAS, ERA5)
//! that use regular latitude/longitude grids. Unlike the GOES-specific reader
//! in `native.rs`, this handles:
//!
//! - Multiple data variables per file
//! - Standard `lat`/`lon`/`time` coordinate variables
//! - CF-convention `scale_factor`, `add_offset`, `_FillValue` attributes
//! - Automatic grid orientation detection (ascending vs descending lat)
//!
//! # NLDAS-2 File Structure
//!
//! NLDAS-2 Noah files have dimensions `(time=1, lat=224, lon=464)` at 0.125°
//! resolution. Time is encoded as "hours since 1979-01-01 00:00:00". Data
//! variables use `_FillValue = -9999.0` (converted to NaN during read).
//! `scale_factor` and `add_offset` are both 1.0/0.0 (data in final units).

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, TimeZone, Utc};
use tracing::{debug, info, warn};

use crate::error::{NetCdfError, NetCdfResult};
use crate::native::silence_hdf5_errors;

/// A single data variable extracted from a CF-convention NetCDF file.
#[derive(Debug, Clone)]
pub struct CfVariable {
    /// Variable name in the NetCDF file (e.g., "SoilM_0_10cm")
    pub name: String,
    /// Long name / description from the `long_name` attribute
    pub long_name: String,
    /// Units string from the `units` attribute
    pub units: String,
    /// Grid data as f32 values (fill values converted to NaN)
    pub data: Vec<f32>,
    /// Grid width (number of longitude points)
    pub width: usize,
    /// Grid height (number of latitude points)
    pub height: usize,
}

/// Metadata about the grid and time extracted from the file.
#[derive(Debug, Clone)]
pub struct CfGridMetadata {
    /// Minimum longitude (west edge)
    pub min_lon: f64,
    /// Maximum longitude (east edge)
    pub max_lon: f64,
    /// Minimum latitude (south edge)
    pub min_lat: f64,
    /// Maximum latitude (north edge)
    pub max_lat: f64,
    /// Grid spacing in longitude (degrees)
    pub dx: f64,
    /// Grid spacing in latitude (degrees)
    pub dy: f64,
    /// Whether latitude is ascending (south to north = true)
    pub lat_ascending: bool,
    /// Observation/valid time for this file
    pub time: DateTime<Utc>,
}

/// Result of reading a CF-convention NetCDF file.
#[derive(Debug)]
pub struct CfDataset {
    /// Grid metadata (bbox, resolution, time, orientation)
    pub metadata: CfGridMetadata,
    /// All data variables extracted from the file
    pub variables: Vec<CfVariable>,
}

/// Coordinate variable names to skip when enumerating data variables.
const COORDINATE_VARS: &[&str] = &[
    "lat",
    "lon",
    "time",
    "time_bnds",
    "x",
    "y",
    "bnds",
    "crs",
    "latitude",
    "longitude",
];

/// Load all data variables from a CF-convention NetCDF file.
///
/// Reads the file from bytes (via a temp file for the netcdf C library),
/// extracts all non-coordinate variables, and returns them with grid metadata.
///
/// # Arguments
/// * `data` - Raw file bytes
/// * `var_filter` - Optional set of variable names to extract. If `None`, all
///   data variables are extracted. If `Some`, only variables in the set are read.
///
/// # Returns
/// A `CfDataset` containing grid metadata and all extracted variables.
pub fn load_cf_netcdf(
    data: &[u8],
    var_filter: Option<&HashSet<String>>,
) -> NetCdfResult<CfDataset> {
    silence_hdf5_errors();

    // Write to temp file (netcdf C library needs a file handle)
    let temp_dir = get_optimal_temp_dir();
    let temp_file = temp_dir.join(generate_temp_filename());

    let mut file = std::fs::File::create(&temp_file)?;
    file.write_all(data)?;
    drop(file);

    let result = read_cf_file(&temp_file, var_filter);

    // Always clean up temp file
    let _ = std::fs::remove_file(&temp_file);

    result
}

/// Internal: read from an on-disk NetCDF file.
fn read_cf_file(
    path: &std::path::Path,
    var_filter: Option<&HashSet<String>>,
) -> NetCdfResult<CfDataset> {
    let nc_file = netcdf::open(path)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to open NetCDF: {}", e)))?;

    // --- Read coordinate variables ---

    // Latitude
    let lat_var = nc_file
        .variable("lat")
        .ok_or_else(|| NetCdfError::MissingData("lat coordinate variable".to_string()))?;
    let lat_values: Vec<f32> = lat_var
        .get_values(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read lat: {}", e)))?;

    if lat_values.is_empty() {
        return Err(NetCdfError::MissingData("lat array is empty".to_string()));
    }

    // Longitude
    let lon_var = nc_file
        .variable("lon")
        .ok_or_else(|| NetCdfError::MissingData("lon coordinate variable".to_string()))?;
    let lon_values: Vec<f32> = lon_var
        .get_values(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read lon: {}", e)))?;

    if lon_values.is_empty() {
        return Err(NetCdfError::MissingData("lon array is empty".to_string()));
    }

    let width = lon_values.len();
    let height = lat_values.len();

    // Detect orientation
    let lat_ascending = lat_values[0] < lat_values[lat_values.len() - 1];

    // Compute bbox from coordinate arrays (cell centers -> edges)
    let (min_lat, max_lat) = if lat_ascending {
        (
            lat_values[0] as f64,
            lat_values[lat_values.len() - 1] as f64,
        )
    } else {
        (
            lat_values[lat_values.len() - 1] as f64,
            lat_values[0] as f64,
        )
    };

    let min_lon = lon_values[0] as f64;
    let max_lon = lon_values[lon_values.len() - 1] as f64;

    // Grid spacing
    let dx = if lon_values.len() > 1 {
        (lon_values[1] - lon_values[0]).abs() as f64
    } else {
        0.125 // fallback
    };
    let dy = if lat_values.len() > 1 {
        (lat_values[1] - lat_values[0]).abs() as f64
    } else {
        0.125 // fallback
    };

    // --- Read time ---
    let time = read_time(&nc_file)?;

    info!(
        width = width,
        height = height,
        lat_ascending = lat_ascending,
        min_lat = min_lat,
        max_lat = max_lat,
        min_lon = min_lon,
        max_lon = max_lon,
        dx = dx,
        dy = dy,
        time = %time,
        "Parsed CF-convention grid metadata"
    );

    let metadata = CfGridMetadata {
        min_lon,
        max_lon,
        min_lat,
        max_lat,
        dx,
        dy,
        lat_ascending,
        time,
    };

    // --- Read data variables ---
    let coord_names: HashSet<&str> = COORDINATE_VARS.iter().copied().collect();
    let mut variables = Vec::new();

    for var in nc_file.variables() {
        let var_name = var.name();

        // Skip coordinate variables
        if coord_names.contains(var_name.as_str()) {
            continue;
        }

        // Apply variable filter if provided
        if let Some(filter) = var_filter {
            if !filter.contains(&var_name) {
                continue;
            }
        }

        // Only read 2D or 3D (time, lat, lon) float variables
        let dims = var.dimensions();
        let ndims = dims.len();
        if ndims < 2 || ndims > 3 {
            debug!(var = %var_name, ndims = ndims, "Skipping variable (wrong dimensionality)");
            continue;
        }

        // Read the variable data
        match read_cf_variable(&var, width, height, lat_ascending) {
            Ok(cf_var) => {
                debug!(
                    name = %cf_var.name,
                    long_name = %cf_var.long_name,
                    units = %cf_var.units,
                    "Read variable"
                );
                variables.push(cf_var);
            }
            Err(e) => {
                warn!(var = %var_name, error = %e, "Failed to read variable, skipping");
            }
        }
    }

    info!(
        total_variables = variables.len(),
        "CF-convention NetCDF read complete"
    );

    Ok(CfDataset {
        metadata,
        variables,
    })
}

/// Read a single data variable, applying scale/offset/fill and optionally flipping latitude.
fn read_cf_variable(
    var: &netcdf::Variable,
    width: usize,
    height: usize,
    lat_ascending: bool,
) -> NetCdfResult<CfVariable> {
    let name = var.name();

    // Get attributes
    let long_name = get_string_attr(var, "long_name").unwrap_or_else(|| name.clone());
    let units = get_string_attr(var, "units").unwrap_or_default();
    let scale_factor = get_f32_attr(var, "scale_factor").unwrap_or(1.0);
    let add_offset = get_f32_attr(var, "add_offset").unwrap_or(0.0);
    let fill_value = get_f32_attr(var, "_FillValue")
        .or_else(|| get_f32_attr(var, "missing_value"))
        .unwrap_or(-9999.0);

    // Read raw data as f32
    let raw_data: Vec<f32> = var
        .get_values(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read {}: {}", name, e)))?;

    let expected_size = width * height;

    // Handle 3D data (time, lat, lon) - take first (and usually only) time step
    let grid_data = if raw_data.len() == expected_size {
        raw_data
    } else if raw_data.len() > expected_size {
        // Take the first time step
        raw_data[..expected_size].to_vec()
    } else {
        return Err(NetCdfError::InvalidFormat(format!(
            "Variable {} has {} values, expected at least {} ({}x{})",
            name,
            raw_data.len(),
            expected_size,
            width,
            height
        )));
    };

    // Apply scale/offset/fill -> f32, converting fill to NaN
    let data: Vec<f32> = grid_data
        .iter()
        .map(|&val| {
            if val == fill_value || val <= fill_value + 0.5 {
                // NLDAS uses -9999.0 as fill; check with small tolerance
                if fill_value < -9990.0 && val < -9990.0 {
                    return f32::NAN;
                }
                if (val - fill_value).abs() < 0.01 {
                    return f32::NAN;
                }
            }
            val * scale_factor + add_offset
        })
        .collect();

    // If lat is ascending (south to north), flip rows so row 0 = north
    // This matches our Zarr storage convention (RowOrigin::North)
    let data = if lat_ascending {
        flip_rows(&data, width, height)
    } else {
        data
    };

    Ok(CfVariable {
        name,
        long_name,
        units,
        data,
        width,
        height,
    })
}

/// Flip grid rows vertically (south-to-north becomes north-to-south).
fn flip_rows(data: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut flipped = Vec::with_capacity(data.len());
    for row in (0..height).rev() {
        let start = row * width;
        let end = start + width;
        flipped.extend_from_slice(&data[start..end]);
    }
    flipped
}

/// Read the time coordinate and convert to `DateTime<Utc>`.
///
/// Handles the common "hours since YYYY-MM-DD HH:MM:SS" convention.
fn read_time(nc_file: &netcdf::File) -> NetCdfResult<DateTime<Utc>> {
    let time_var = nc_file
        .variable("time")
        .ok_or_else(|| NetCdfError::MissingData("time coordinate variable".to_string()))?;

    let time_units = get_string_attr(&time_var, "units")
        .ok_or_else(|| NetCdfError::MissingData("time:units attribute".to_string()))?;

    let time_values: Vec<f64> = time_var
        .get_values(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read time: {}", e)))?;

    if time_values.is_empty() {
        return Err(NetCdfError::MissingData("time array is empty".to_string()));
    }

    let time_val = time_values[0];

    // Parse units string: "hours since 1979-01-01 00:00:00"
    parse_cf_time(time_val, &time_units)
}

/// Parse a CF-convention time value given its units string.
///
/// Supports formats:
/// - "hours since YYYY-MM-DD HH:MM:SS"
/// - "hours since YYYY-MM-DD"
/// - "days since YYYY-MM-DD"
/// - "seconds since YYYY-MM-DD HH:MM:SS"
fn parse_cf_time(value: f64, units: &str) -> NetCdfResult<DateTime<Utc>> {
    let parts: Vec<&str> = units.splitn(3, ' ').collect();
    if parts.len() < 3 || parts[1] != "since" {
        return Err(NetCdfError::InvalidFormat(format!(
            "Unsupported time units format: '{}'",
            units
        )));
    }

    let time_unit = parts[0].to_lowercase();
    let epoch_str = parts[2];

    // Parse epoch datetime
    let epoch = parse_epoch_datetime(epoch_str)?;

    // Convert value to a Duration based on the unit
    let duration = match time_unit.as_str() {
        "hours" => Duration::seconds((value * 3600.0) as i64),
        "days" => Duration::seconds((value * 86400.0) as i64),
        "seconds" => Duration::seconds(value as i64),
        "minutes" => Duration::seconds((value * 60.0) as i64),
        _ => {
            return Err(NetCdfError::InvalidFormat(format!(
                "Unsupported time unit: '{}'",
                time_unit
            )));
        }
    };

    Ok(epoch + duration)
}

/// Parse an epoch datetime string like "1979-01-01 00:00:00" or "1979-01-01".
fn parse_epoch_datetime(s: &str) -> NetCdfResult<DateTime<Utc>> {
    // Try "YYYY-MM-DD HH:MM:SS" first
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(Utc.from_utc_datetime(&dt));
    }

    // Try "YYYY-MM-DD"
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = d.and_hms_opt(0, 0, 0).unwrap();
        return Ok(Utc.from_utc_datetime(&dt));
    }

    Err(NetCdfError::InvalidFormat(format!(
        "Cannot parse epoch datetime: '{}'",
        s
    )))
}

// =============================================================================
// Attribute helpers
// =============================================================================

/// Check if a variable has an attribute.
fn has_attr(var: &netcdf::Variable, name: &str) -> bool {
    var.attributes().any(|attr| attr.name() == name)
}

/// Get a string attribute value.
fn get_string_attr(var: &netcdf::Variable, name: &str) -> Option<String> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    String::try_from(attr_value).ok()
}

/// Get an f32 attribute value.
fn get_f32_attr(var: &netcdf::Variable, name: &str) -> Option<f32> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    f32::try_from(attr_value).ok()
}

// =============================================================================
// Temp file helpers (shared pattern with native.rs)
// =============================================================================

fn get_optimal_temp_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;
        let shm_path = Path::new("/dev/shm");
        if shm_path.exists() && shm_path.is_dir() {
            let test_path = shm_path.join(format!(".cf_netcdf_test_{}", std::process::id()));
            if std::fs::write(&test_path, b"test").is_ok() {
                let _ = std::fs::remove_file(&test_path);
                return shm_path.to_path_buf();
            }
        }
    }
    std::env::temp_dir()
}

fn generate_temp_filename() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let tid = std::thread::current().id();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("cf_netcdf_{}_{:?}_{}.nc", pid, tid, count)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_parse_cf_time_hours_since() {
        // NLDAS-2 time encoding: "hours since 1979-01-01 00:00:00"
        // Value 412848 should be 2026-02-05 00:00:00 UTC
        let result = parse_cf_time(412848.0, "hours since 1979-01-01 00:00:00").unwrap();
        assert_eq!(result.year(), 2026);
        assert_eq!(result.month(), 2);
        assert_eq!(result.day(), 5);
        assert_eq!(result.hour(), 0);
    }

    #[test]
    fn test_parse_cf_time_days_since() {
        let result = parse_cf_time(365.0, "days since 2000-01-01 00:00:00").unwrap();
        assert_eq!(result.year(), 2000);
        assert_eq!(result.month(), 12);
        assert_eq!(result.day(), 31);
    }

    #[test]
    fn test_parse_cf_time_seconds_since() {
        let result = parse_cf_time(3600.0, "seconds since 2020-06-15 12:00:00").unwrap();
        assert_eq!(result.hour(), 13);
    }

    #[test]
    fn test_parse_cf_time_short_epoch() {
        // Some files only have "YYYY-MM-DD" without time
        let result = parse_cf_time(24.0, "hours since 2020-01-01").unwrap();
        assert_eq!(result.day(), 2);
        assert_eq!(result.hour(), 0);
    }

    #[test]
    fn test_parse_cf_time_invalid_units() {
        let result = parse_cf_time(1.0, "fortnights since 2020-01-01");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_cf_time_bad_format() {
        let result = parse_cf_time(1.0, "not a valid units string");
        assert!(result.is_err());
    }

    #[test]
    fn test_flip_rows() {
        // 3x2 grid:
        // Row 0 (south): [1, 2, 3]
        // Row 1 (north): [4, 5, 6]
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let flipped = flip_rows(&data, 3, 2);
        // After flip:
        // Row 0 (north): [4, 5, 6]
        // Row 1 (south): [1, 2, 3]
        assert_eq!(flipped, vec![4.0, 5.0, 6.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_flip_rows_single_row() {
        let data = vec![1.0, 2.0, 3.0];
        let flipped = flip_rows(&data, 3, 1);
        assert_eq!(flipped, data);
    }

    #[test]
    fn test_flip_rows_4x3() {
        let data = vec![
            1.0, 2.0, 3.0, 4.0, // row 0 (south)
            5.0, 6.0, 7.0, 8.0, // row 1
            9.0, 10.0, 11.0, 12.0, // row 2 (north)
        ];
        let flipped = flip_rows(&data, 4, 3);
        assert_eq!(
            flipped,
            vec![
                9.0, 10.0, 11.0, 12.0, // row 0 (north)
                5.0, 6.0, 7.0, 8.0, // row 1
                1.0, 2.0, 3.0, 4.0, // row 2 (south)
            ]
        );
    }

    #[test]
    fn test_coordinate_vars_filter() {
        let coord_names: HashSet<&str> = COORDINATE_VARS.iter().copied().collect();
        assert!(coord_names.contains("lat"));
        assert!(coord_names.contains("lon"));
        assert!(coord_names.contains("time"));
        assert!(coord_names.contains("time_bnds"));
        assert!(coord_names.contains("bnds"));
        assert!(!coord_names.contains("SoilM_0_10cm"));
        assert!(!coord_names.contains("SWE"));
    }

    #[test]
    fn test_parse_epoch_datetime_full() {
        let dt = parse_epoch_datetime("1979-01-01 00:00:00").unwrap();
        assert_eq!(dt.year(), 1979);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 1);
    }

    #[test]
    fn test_parse_epoch_datetime_date_only() {
        let dt = parse_epoch_datetime("2000-01-01").unwrap();
        assert_eq!(dt.year(), 2000);
    }

    #[test]
    fn test_parse_epoch_datetime_invalid() {
        assert!(parse_epoch_datetime("not-a-date").is_err());
    }

    /// Integration test that reads an actual NLDAS-2 sample file.
    /// Skipped if the sample file is not present (won't fail in CI).
    #[test]
    fn test_read_nldas2_sample_file() {
        let path = std::path::Path::new("/tmp/nldas_test_sample.nc");
        if !path.exists() {
            eprintln!("Skipping NLDAS-2 integration test (sample file not present at /tmp/nldas_test_sample.nc)");
            return;
        }

        let data = std::fs::read(path).expect("Failed to read sample file");
        let result = load_cf_netcdf(&data, None).expect("Failed to parse NLDAS-2 file");

        // Grid metadata
        assert_eq!(result.metadata.dx, 0.125);
        assert_eq!(result.metadata.dy, 0.125);
        assert!(
            result.metadata.lat_ascending,
            "NLDAS-2 lat should be ascending"
        );
        assert!((result.metadata.min_lat - 25.0625).abs() < 0.01);
        assert!((result.metadata.max_lat - 52.9375).abs() < 0.01);
        assert!((result.metadata.min_lon - (-124.9375)).abs() < 0.01);
        assert!((result.metadata.max_lon - (-67.0625)).abs() < 0.01);

        // Time: should be 2026-02-05 00:00 UTC
        assert_eq!(result.metadata.time.year(), 2026);
        assert_eq!(result.metadata.time.month(), 2);
        assert_eq!(result.metadata.time.day(), 5);
        assert_eq!(result.metadata.time.hour(), 0);

        // Should have many variables (NLDAS-2 Noah has ~40)
        assert!(
            result.variables.len() >= 30,
            "Expected at least 30 variables, got {}",
            result.variables.len()
        );

        // Check a known variable exists
        let soil_m = result
            .variables
            .iter()
            .find(|v| v.name == "SoilM_0_10cm")
            .expect("SoilM_0_10cm variable not found");

        assert_eq!(soil_m.width, 464);
        assert_eq!(soil_m.height, 224);
        assert_eq!(soil_m.data.len(), 464 * 224);
        assert!(soil_m.units.contains("kg"));

        // Check that fill values were converted to NaN (NLDAS uses -9999)
        let nan_count = soil_m.data.iter().filter(|v| v.is_nan()).count();
        let valid_count = soil_m.data.len() - nan_count;
        assert!(valid_count > 0, "All values are NaN");
        assert!(
            nan_count > 0,
            "Expected some NaN values (ocean/outside domain)"
        );

        // Valid soil moisture should be positive
        let valid_min = soil_m
            .data
            .iter()
            .filter(|v| !v.is_nan())
            .copied()
            .fold(f32::INFINITY, f32::min);
        assert!(
            valid_min >= 0.0,
            "Soil moisture should be non-negative, got {}",
            valid_min
        );

        // Test filtered read
        let mut filter = HashSet::new();
        filter.insert("SWE".to_string());
        filter.insert("AvgSurfT".to_string());
        let filtered = load_cf_netcdf(&data, Some(&filter)).expect("Filtered read failed");
        assert_eq!(
            filtered.variables.len(),
            2,
            "Should have exactly 2 filtered variables, got {}",
            filtered.variables.len()
        );

        eprintln!(
            "NLDAS-2 integration test passed: {} variables, grid {}x{}, time {}",
            result.variables.len(),
            464,
            224,
            result.metadata.time
        );
    }
}
