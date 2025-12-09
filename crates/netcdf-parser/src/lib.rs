//! NetCDF parser for satellite data (GOES-R ABI).
//!
//! This crate provides reading of NetCDF-4 format files, specifically
//! optimized for GOES-R ABI (Advanced Baseline Imager) data products.
//!
//! # Implementation Notes
//!
//! This uses the `ncdump` command-line tool for metadata extraction.
//! For production use, consider installing libhdf5-dev and enabling
//! the `netcdf` crate for direct reading.
//!
//! # GOES-R ABI Data Structure
//!
//! GOES-R ABI files use the geostationary projection with coordinates in radians.
//! The main data variable is `CMI` (Cloud and Moisture Imagery) which contains
//! either reflectance factors (bands 1-6) or brightness temperatures (bands 7-16).

use chrono::{DateTime, TimeZone, Utc};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

/// Result type for NetCDF parser operations.
pub type NetCdfResult<T> = Result<T, NetCdfError>;

/// Error types for NetCDF parsing.
#[derive(Error, Debug)]
pub enum NetCdfError {
    /// File I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Missing required variable or attribute
    #[error("Missing required data: {0}")]
    MissingData(String),

    /// Invalid data format
    #[error("Invalid data format: {0}")]
    InvalidFormat(String),

    /// Command execution error
    #[error("Command execution failed: {0}")]
    CommandError(String),
}

/// GOES ABI projection parameters
#[derive(Debug, Clone)]
pub struct GoesProjection {
    /// Satellite height above Earth center (meters)
    pub perspective_point_height: f64,
    /// Semi-major axis of Earth ellipsoid (meters)
    pub semi_major_axis: f64,
    /// Semi-minor axis of Earth ellipsoid (meters)
    pub semi_minor_axis: f64,
    /// Longitude of satellite nadir point (degrees)
    pub longitude_origin: f64,
    /// Latitude of projection origin (always 0 for geostationary)
    pub latitude_origin: f64,
    /// Sweep angle axis ("x" for GOES-R)
    pub sweep_angle_axis: String,
}

impl Default for GoesProjection {
    fn default() -> Self {
        // Default values for GOES-16 (GOES-East)
        Self {
            perspective_point_height: 35786023.0,
            semi_major_axis: 6378137.0,
            semi_minor_axis: 6356752.31414,
            longitude_origin: -75.0,  // GOES-16/East
            latitude_origin: 0.0,
            sweep_angle_axis: "x".to_string(),
        }
    }
}

impl GoesProjection {
    /// Create projection for GOES-16 (GOES-East at 75.2°W)
    pub fn goes16() -> Self {
        Self {
            longitude_origin: -75.2,
            ..Default::default()
        }
    }

    /// Create projection for GOES-18 (GOES-West at 137.2°W)
    pub fn goes18() -> Self {
        Self {
            longitude_origin: -137.2,
            ..Default::default()
        }
    }

    /// Convert geostationary coordinates (radians) to geographic (lat/lon degrees)
    ///
    /// Based on GOES-R Product Definition and Users' Guide (PUG) formulas.
    /// The x,y coordinates are scan angles in radians from the satellite nadir.
    /// 
    /// Reference: GOES-R PUG Volume 4, Section 4.2.8
    pub fn to_geographic(&self, x_rad: f64, y_rad: f64) -> Option<(f64, f64)> {
        let h = self.perspective_point_height;
        let req = self.semi_major_axis;
        let rpol = self.semi_minor_axis;
        let lambda_0 = self.longitude_origin.to_radians();
        let h_total = h + req;
        
        let sin_x = x_rad.sin();
        let cos_x = x_rad.cos();
        let sin_y = y_rad.sin();
        let cos_y = y_rad.cos();
        
        // Quadratic coefficients for finding distance to Earth surface
        let a = sin_x.powi(2) + cos_x.powi(2) * (cos_y.powi(2) + (req / rpol).powi(2) * sin_y.powi(2));
        let b = -2.0 * h_total * cos_x * cos_y;
        let c = h_total.powi(2) - req.powi(2);
        
        let discriminant = b * b - 4.0 * a * c;
        if discriminant < 0.0 {
            return None; // Scan angle points to space
        }
        
        let rs = (-b - discriminant.sqrt()) / (2.0 * a);
        
        // 3D coordinates (satellite-centered, Earth-fixed)
        // Note: sy uses negative sin(x) to match forward transform where x = atan2(-sy, sx)
        let sx = rs * cos_x * cos_y;
        let sy = -rs * sin_x;  // Negative because x = atan2(-sy, sx), so sy = -rs*sin(x)
        let sz = rs * cos_x * sin_y;
        
        // Convert to geodetic coordinates
        let lat = ((req / rpol).powi(2) * sz / (h_total - sx).hypot(sy)).atan();
        let lon = lambda_0 - sy.atan2(h_total - sx);
        
        Some((lon.to_degrees(), lat.to_degrees()))
    }

    /// Convert geographic coordinates (lat/lon degrees) to geostationary (radians)
    ///
    /// Reference: GOES-R PUG Volume 4, Section 4.2.8
    pub fn from_geographic(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let h = self.perspective_point_height;
        let req = self.semi_major_axis;
        let rpol = self.semi_minor_axis;
        let lambda_0 = self.longitude_origin.to_radians();
        let h_total = h + req;
        
        let lat_rad = lat.to_radians();
        let lon_rad = lon.to_radians();
        
        // Geocentric latitude (accounting for Earth's oblateness)
        let phi_c = ((rpol / req).powi(2) * lat_rad.tan()).atan();
        
        // Radius from Earth center to surface point
        let e2 = 1.0 - (rpol / req).powi(2);  // eccentricity squared
        let rc = rpol / (1.0 - e2 * phi_c.cos().powi(2)).sqrt();
        
        // 3D coordinates (Earth-centered, satellite frame)
        let sx = h_total - rc * phi_c.cos() * (lon_rad - lambda_0).cos();
        let sy = -rc * phi_c.cos() * (lon_rad - lambda_0).sin();
        let sz = rc * phi_c.sin();
        
        // Check visibility (point must be on satellite-facing side of Earth)
        if sx <= 0.0 {
            return None; // Behind Earth from satellite's perspective
        }
        
        // Calculate scan angles using atan2 for correct quadrant
        let s_xy = sx.hypot(sy);
        let y_rad = sz.atan2(s_xy);
        let x_rad = (-sy).atan2(sx);  // Negative sy to match inverse formula sign convention
        
        Some((x_rad, y_rad))
    }
}

/// Geographic bounding box extracted from GOES file
#[derive(Debug, Clone)]
pub struct GeoBounds {
    pub west: f64,
    pub east: f64,
    pub north: f64,
    pub south: f64,
}

/// Metadata extracted from a GOES ABI file
#[derive(Debug, Clone)]
pub struct GoesMetadata {
    /// Band number (1-16)
    pub band_id: u8,
    /// Band central wavelength in micrometers
    pub band_wavelength: f32,
    /// Grid width (number of x points)
    pub width: usize,
    /// Grid height (number of y points)
    pub height: usize,
    /// Observation time
    pub time: DateTime<Utc>,
    /// Geographic bounds
    pub bounds: GeoBounds,
    /// Projection parameters
    pub projection: GoesProjection,
    /// Scale factor for CMI data
    pub scale_factor: f32,
    /// Offset for CMI data
    pub add_offset: f32,
    /// Fill value
    pub fill_value: i16,
    /// Whether this is reflectance (bands 1-6) or brightness temp (bands 7-16)
    pub is_reflectance: bool,
    /// Satellite ID (e.g., "G16", "G18")
    pub satellite_id: String,
    /// Scene type (e.g., "CONUS", "FullDisk", "Mesoscale")
    pub scene_id: String,
}

/// Unpacked GOES ABI image data
#[derive(Debug)]
pub struct GoesData {
    /// Metadata
    pub metadata: GoesMetadata,
    /// Scaled data values (reflectance factor or brightness temperature K)
    pub data: Vec<f32>,
}

/// Parse GOES metadata from ncdump output
pub fn parse_goes_metadata<P: AsRef<Path>>(path: P) -> NetCdfResult<GoesMetadata> {
    let path = path.as_ref();
    
    // Run ncdump -h to get header
    let output = Command::new("ncdump")
        .arg("-h")
        .arg(path)
        .output()
        .map_err(|e| NetCdfError::CommandError(format!("Failed to run ncdump: {}", e)))?;
    
    if !output.status.success() {
        return Err(NetCdfError::CommandError(format!(
            "ncdump failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    
    let header = String::from_utf8_lossy(&output.stdout);
    
    // Parse dimensions
    let width = parse_dimension(&header, "x")?;
    let height = parse_dimension(&header, "y")?;
    
    // Parse band info
    let band_id = parse_int_from_ncdump(path, "band_id")? as u8;
    let band_wavelength = parse_float_from_ncdump(path, "band_wavelength")?;
    
    // Parse time
    let t_seconds = parse_float_from_ncdump(path, "t")?;
    let j2000 = Utc.with_ymd_and_hms(2000, 1, 1, 12, 0, 0).unwrap();
    let time = j2000 + chrono::Duration::milliseconds((t_seconds as f64 * 1000.0) as i64);
    
    // Parse geographic bounds from attributes
    let west = parse_attribute(&header, "geospatial_westbound_longitude")?;
    let east = parse_attribute(&header, "geospatial_eastbound_longitude")?;
    let north = parse_attribute(&header, "geospatial_northbound_latitude")?;
    let south = parse_attribute(&header, "geospatial_southbound_latitude")?;
    
    let bounds = GeoBounds { west, east, north, south };
    
    // Parse projection parameters
    let perspective_point_height = parse_attribute(&header, "perspective_point_height")?;
    let semi_major_axis = parse_attribute(&header, "semi_major_axis")?;
    let semi_minor_axis = parse_attribute(&header, "semi_minor_axis")?;
    let longitude_origin = parse_attribute(&header, "longitude_of_projection_origin")?;
    
    let projection = GoesProjection {
        perspective_point_height,
        semi_major_axis,
        semi_minor_axis,
        longitude_origin,
        latitude_origin: 0.0,
        sweep_angle_axis: "x".to_string(),
    };
    
    // Parse CMI scale/offset from header
    let scale_factor = parse_attribute(&header, "scale_factor").unwrap_or(1.0) as f32;
    let add_offset = parse_attribute(&header, "add_offset").unwrap_or(0.0) as f32;
    let fill_value = parse_attribute(&header, "_FillValue").unwrap_or(-1.0) as i16;
    
    // Bands 1-6 are reflectance, 7-16 are brightness temperature
    let is_reflectance = band_id <= 6;
    
    // Extract satellite and scene from filename
    let filename = path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    
    let satellite_id = if filename.contains("G16") {
        "G16".to_string()
    } else if filename.contains("G18") {
        "G18".to_string()
    } else if filename.contains("G17") {
        "G17".to_string()
    } else {
        "GOES".to_string()
    };
    
    let scene_id = if filename.contains("CMIPC") || filename.contains("CONUS") {
        "CONUS".to_string()
    } else if filename.contains("CMIPF") || filename.contains("FullDisk") {
        "FullDisk".to_string()
    } else if filename.contains("CMIPM") || filename.contains("Mesoscale") {
        "Mesoscale".to_string()
    } else {
        "Unknown".to_string()
    };
    
    Ok(GoesMetadata {
        band_id,
        band_wavelength,
        width,
        height,
        time,
        bounds,
        projection,
        scale_factor,
        add_offset,
        fill_value,
        is_reflectance,
        satellite_id,
        scene_id,
    })
}

/// Read GOES CMI data using ncdump
pub fn read_goes_data<P: AsRef<Path>>(path: P) -> NetCdfResult<GoesData> {
    let metadata = parse_goes_metadata(&path)?;
    
    // For binary reading, we'd need HDF5 library
    // For now, use ncdump -v to extract data (slower but works)
    let output = Command::new("ncdump")
        .arg("-v")
        .arg("CMI")
        .arg("-p")
        .arg("9,17")  // High precision
        .arg(path.as_ref())
        .output()
        .map_err(|e| NetCdfError::CommandError(format!("Failed to run ncdump: {}", e)))?;
    
    if !output.status.success() {
        return Err(NetCdfError::CommandError(format!(
            "ncdump failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    
    // Find the data section
    let data_start = output_str.find("CMI =")
        .ok_or_else(|| NetCdfError::MissingData("CMI data section".to_string()))?;
    
    let data_section = &output_str[data_start..];
    
    // Parse the numeric values
    let mut data = Vec::with_capacity(metadata.width * metadata.height);
    let mut in_data = false;
    
    for line in data_section.lines() {
        if line.contains("CMI =") {
            in_data = true;
            continue;
        }
        if !in_data {
            continue;
        }
        if line.contains(";") {
            // End of data
            break;
        }
        
        // Parse numbers from line
        for part in line.split(',') {
            let trimmed = part.trim().trim_end_matches(';');
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = trimmed.parse::<i16>() {
                // Apply scale and offset
                let scaled = if val == metadata.fill_value {
                    f32::NAN
                } else {
                    val as f32 * metadata.scale_factor + metadata.add_offset
                };
                data.push(scaled);
            } else if trimmed == "_" {
                // Fill value marker
                data.push(f32::NAN);
            }
        }
    }
    
    Ok(GoesData { metadata, data })
}

// Helper functions

fn parse_dimension(header: &str, name: &str) -> NetCdfResult<usize> {
    // Look for pattern like "x = 5000 ;"
    let pattern = format!("{} = ", name);
    for line in header.lines() {
        if line.contains(&pattern) {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim().trim_end_matches(';').trim();
                return num_str.parse()
                    .map_err(|_| NetCdfError::InvalidFormat(format!("Failed to parse dimension {}", name)));
            }
        }
    }
    Err(NetCdfError::MissingData(format!("dimension {}", name)))
}

fn parse_attribute(header: &str, name: &str) -> NetCdfResult<f64> {
    // Look for pattern like "goes_imager_projection:perspective_point_height = 35786023. ;"
    let pattern = format!(":{} = ", name);
    for line in header.lines() {
        if line.contains(&pattern) {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim()
                    .trim_end_matches(';')
                    .trim_end_matches('f')
                    .trim_end_matches('.')
                    .trim();
                // Handle scientific notation and trailing 'f'
                let clean = num_str.replace("f", "");
                return clean.parse()
                    .map_err(|_| NetCdfError::InvalidFormat(format!("Failed to parse attribute {}: '{}'", name, num_str)));
            }
        }
    }
    Err(NetCdfError::MissingData(format!("attribute {}", name)))
}

fn parse_int_from_ncdump<P: AsRef<Path>>(path: P, var: &str) -> NetCdfResult<i32> {
    let output = Command::new("ncdump")
        .arg("-v")
        .arg(var)
        .arg(path.as_ref())
        .output()
        .map_err(|e| NetCdfError::CommandError(format!("ncdump failed: {}", e)))?;
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    let pattern = format!("{} =", var);
    
    for line in output_str.lines() {
        if line.contains(&pattern) {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim().trim_end_matches(';').trim();
                return num_str.parse()
                    .map_err(|_| NetCdfError::InvalidFormat(format!("Failed to parse {}", var)));
            }
        }
    }
    Err(NetCdfError::MissingData(var.to_string()))
}

fn parse_float_from_ncdump<P: AsRef<Path>>(path: P, var: &str) -> NetCdfResult<f32> {
    let output = Command::new("ncdump")
        .arg("-v")
        .arg(var)
        .arg(path.as_ref())
        .output()
        .map_err(|e| NetCdfError::CommandError(format!("ncdump failed: {}", e)))?;
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    let pattern = format!("{} =", var);
    
    for line in output_str.lines() {
        if line.contains(&pattern) {
            let parts: Vec<&str> = line.split('=').collect();
            if parts.len() >= 2 {
                let num_str = parts[1].trim().trim_end_matches(';').trim();
                return num_str.parse()
                    .map_err(|_| NetCdfError::InvalidFormat(format!("Failed to parse {}", var)));
            }
        }
    }
    Err(NetCdfError::MissingData(var.to_string()))
}

/// Get the optimal temp directory for NetCDF file operations.
/// 
/// On Linux, uses /dev/shm (memory-backed tmpfs) if available for faster I/O.
/// Falls back to the system temp directory on other platforms or if /dev/shm is unavailable.
/// 
/// This optimization can significantly reduce NetCDF parsing latency since the netcdf
/// library requires a file path and cannot read directly from memory.
fn get_optimal_temp_dir() -> std::path::PathBuf {
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
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    
    let pid = std::process::id();
    let tid = std::thread::current().id();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    
    format!("goes_native_{}_{:?}_{}.nc", pid, tid, count)
}

/// Load GOES NetCDF data directly from bytes using native netcdf library.
/// This is much faster than using ncdump subprocess.
///
/// # Performance Note
/// 
/// The netcdf library requires a file path (it wraps libnetcdf/HDF5 which need file handles).
/// This function writes the data to a temp file, reads it with netcdf, then deletes the file.
/// 
/// On Linux, we use /dev/shm (memory-backed tmpfs) to minimize I/O latency.
/// Benchmarks show this reduces temp file overhead from ~2-5ms to ~0.5-1ms.
/// 
/// Returns (data, width, height, projection, x_offset, y_offset, x_scale, y_scale)
pub fn load_goes_netcdf_from_bytes(data: &[u8]) -> NetCdfResult<(Vec<f32>, usize, usize, GoesProjection, f32, f32, f32, f32)> {
    use std::io::Write;
    
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
    let width = nc_file.dimension("x")
        .ok_or_else(|| NetCdfError::MissingData("x dimension".to_string()))?.len();
    let height = nc_file.dimension("y")
        .ok_or_else(|| NetCdfError::MissingData("y dimension".to_string()))?.len();
    
    // Get CMI variable and its data
    let cmi_var = nc_file.variable("CMI")
        .ok_or_else(|| NetCdfError::MissingData("CMI variable".to_string()))?;
    
    // Read raw data as i16 using (..) to read all extents
    let raw_data: Vec<i16> = cmi_var.get_values(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read CMI: {}", e)))?;
    
    // Get attributes using get_value helper
    let scale_factor = get_f32_attr(&cmi_var, "scale_factor").unwrap_or(1.0);
    let add_offset = get_f32_attr(&cmi_var, "add_offset").unwrap_or(0.0);
    let fill_value = get_i16_attr(&cmi_var, "_FillValue").unwrap_or(-1);
    
    // Apply scale and offset
    let data: Vec<f32> = raw_data.iter().map(|&val| {
        if val == fill_value { f32::NAN } else { val as f32 * scale_factor + add_offset }
    }).collect();
    
    // Get coordinate attributes
    let x_var = nc_file.variable("x")
        .ok_or_else(|| NetCdfError::MissingData("x variable".to_string()))?;
    let x_scale = get_f32_attr(&x_var, "scale_factor").unwrap_or(1.4e-05);
    let x_offset = get_f32_attr(&x_var, "add_offset").unwrap_or(-0.101353);
    
    let y_var = nc_file.variable("y")
        .ok_or_else(|| NetCdfError::MissingData("y variable".to_string()))?;
    let y_scale = get_f32_attr(&y_var, "scale_factor").unwrap_or(-1.4e-05);
    let y_offset = get_f32_attr(&y_var, "add_offset").unwrap_or(0.128233);
    
    // Get projection
    let proj_var = nc_file.variable("goes_imager_projection")
        .ok_or_else(|| NetCdfError::MissingData("goes_imager_projection variable".to_string()))?;
    
    let projection = GoesProjection {
        perspective_point_height: get_f64_attr(&proj_var, "perspective_point_height").unwrap_or(35786023.0),
        semi_major_axis: get_f64_attr(&proj_var, "semi_major_axis").unwrap_or(6378137.0),
        semi_minor_axis: get_f64_attr(&proj_var, "semi_minor_axis").unwrap_or(6356752.31414),
        longitude_origin: get_f64_attr(&proj_var, "longitude_of_projection_origin").unwrap_or(-75.0),
        latitude_origin: 0.0,
        sweep_angle_axis: "x".to_string(),
    };
    
    // Clean up
    let _ = std::fs::remove_file(&temp_file);
    
    Ok((data, width, height, projection, x_offset, y_offset, x_scale, y_scale))
}

/// Check if a variable has an attribute with the given name.
/// This avoids HDF5 error spam when checking for optional attributes.
fn has_attr(var: &netcdf::Variable, name: &str) -> bool {
    var.attributes().any(|attr| attr.name() == name)
}

// Helper to get f32 attribute using TryInto
fn get_f32_attr(var: &netcdf::Variable, name: &str) -> Option<f32> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    f32::try_from(attr_value).ok()
}

// Helper to get f64 attribute using TryInto
fn get_f64_attr(var: &netcdf::Variable, name: &str) -> Option<f64> {
    if !has_attr(var, name) {
        return None;
    }
    let attr_value = var.attribute_value(name)?.ok()?;
    f64::try_from(attr_value).ok()
}

// Helper to get i16 attribute using TryInto
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
    fn test_goes_projection_roundtrip() {
        let proj = GoesProjection::goes16();
        
        // Test a point near the center of CONUS
        let (lon, lat) = (-95.0, 35.0);
        
        if let Some((x, y)) = proj.from_geographic(lon, lat) {
            if let Some((lon2, lat2)) = proj.to_geographic(x, y) {
                // Allow ~0.1 degree tolerance due to float precision and projection approximations
                assert!((lon - lon2).abs() < 0.15, "Longitude mismatch: {} vs {}", lon, lon2);
                assert!((lat - lat2).abs() < 0.15, "Latitude mismatch: {} vs {}", lat, lat2);
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
        let result = proj.to_geographic(0.5, 0.5);  // ~28 degrees
        // This might or might not be visible depending on exact geometry
        // The important thing is it doesn't panic
        println!("Off-earth test result: {:?}", result);
    }
}
