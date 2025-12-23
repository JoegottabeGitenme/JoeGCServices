//! Metadata extraction utilities for weather data files.
//!
//! Provides functions to extract model names, forecast hours, and other
//! metadata from weather data filenames.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use std::path::Path;
use wms_common::BoundingBox;

/// Detected file type based on extension and content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// GRIB2 format (GFS, HRRR, MRMS)
    Grib2,
    /// Gzip-compressed GRIB2
    Grib2Gz,
    /// NetCDF format (GOES satellite)
    NetCdf,
    /// Unknown format
    Unknown,
}

/// Information extracted from a GOES filename.
#[derive(Debug, Clone)]
pub struct GoesFileInfo {
    /// Satellite identifier (goes16, goes18)
    pub satellite: String,
    /// Band number (1-16)
    pub band: u8,
    /// Scan mode (M3 = 15min, M6 = 10min)
    pub scan_mode: String,
    /// Observation start time
    pub observation_time: DateTime<Utc>,
    /// Product type (e.g., "MCMIPC" for CONUS cloud/moisture)
    pub product: String,
}

/// Detect file type from path.
pub fn detect_file_type(path: &str) -> FileType {
    let lower = path.to_lowercase();
    
    if lower.ends_with(".grib2.gz") || lower.ends_with(".grb2.gz") {
        FileType::Grib2Gz
    } else if lower.ends_with(".grib2") || lower.ends_with(".grb2") || lower.ends_with(".grib") {
        FileType::Grib2
    } else if lower.ends_with(".nc") || lower.ends_with(".nc4") || lower.ends_with(".netcdf") {
        FileType::NetCdf
    } else {
        FileType::Unknown
    }
}

/// Extract model name from filename.
///
/// Supports: GFS, HRRR, MRMS, GOES-16, GOES-18
pub fn extract_model_from_filename(file_path: &str) -> Option<String> {
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())?;
    
    let lower = filename.to_lowercase();
    
    if lower.contains("_g16_") || lower.contains("goes16") {
        Some("goes16".to_string())
    } else if lower.contains("_g18_") || lower.contains("goes18") {
        Some("goes18".to_string())
    } else if lower.starts_with("hrrr") || lower.contains("hrrr") {
        Some("hrrr".to_string())
    } else if lower.starts_with("gfs") || lower.contains("gfs") {
        Some("gfs".to_string())
    } else if lower.starts_with("mrms_") || lower.contains("mrms") {
        Some("mrms".to_string())
    } else {
        None
    }
}

/// Extract forecast hour from filename.
///
/// Supports patterns:
/// - `_f###` (e.g., `gfs_20241201_00z_f003.grib2`)
/// - `wrfsfcf##` (HRRR format)
/// - `z_f###` (download naming convention)
pub fn extract_forecast_hour(file_path: &str) -> Option<u32> {
    let filename = Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())?;
    
    // Pattern: _f### (e.g., gfs_20241201_00z_f003.grib2)
    if let Some(pos) = filename.rfind("_f") {
        let rest = &filename[pos + 2..];
        if let Some(hour) = rest.get(..3).and_then(|s| s.parse::<u32>().ok()) {
            return Some(hour);
        }
    }
    
    // Pattern: wrfsfcf## (HRRR)
    if let Some(pos) = filename.find("wrfsfcf") {
        let rest = &filename[pos + 7..];
        if let Some(hour) = rest.get(..2).and_then(|s| s.parse::<u32>().ok()) {
            return Some(hour);
        }
    }
    
    // Pattern: z_f### at end (our download naming)
    if let Some(pos) = filename.find("z_f") {
        let rest = &filename[pos + 3..];
        if let Ok(hour) = rest.parse::<u32>() {
            return Some(hour);
        }
    }
    
    None
}

/// Extract MRMS parameter name from filename.
///
/// Maps known MRMS products to standardized parameter names.
pub fn extract_mrms_param(file_path: &str) -> Option<String> {
    let filename = Path::new(file_path)
        .file_name()
        .and_then(|s| s.to_str())?;
    
    let lower = filename.to_lowercase();
    
    // SeamlessHSR is the fully merged radar composite - map to REFL
    if lower.contains("seamlesshsr") {
        Some("REFL".to_string())
    } else if lower.contains("reflectivity") || lower.contains("refl") {
        Some("REFL".to_string())
    } else if lower.contains("preciprate") || lower.contains("precip_rate") {
        Some("PRECIP_RATE".to_string())
    } else if lower.contains("qpe_01h") {
        Some("QPE_01H".to_string())
    } else if lower.contains("qpe") {
        Some("QPE".to_string())
    } else if filename.starts_with("MRMS_") {
        filename.strip_prefix("MRMS_")
            .and_then(|rest| rest.split('_').next())
            .map(|p| p.to_uppercase())
    } else {
        None
    }
}

/// Parse GOES filename to extract metadata.
///
/// Example: `OR_ABI-L2-MCMIPC-M6C02_G18_s20241217180021_...`
pub fn parse_goes_filename(filename: &str) -> Option<GoesFileInfo> {
    // Extract satellite from _G16_ or _G18_
    let satellite = if filename.contains("_G16_") {
        "goes16"
    } else if filename.contains("_G18_") {
        "goes18"
    } else {
        return None;
    };
    
    // Extract band number from M6C## or M3C##
    let band = filename
        .find("M6C")
        .or_else(|| filename.find("M3C"))
        .and_then(|pos| {
            let band_str = &filename[pos + 3..pos + 5];
            band_str.parse::<u8>().ok()
        })?;
    
    // Extract scan mode
    let scan_mode = if filename.contains("M6C") {
        "M6".to_string()
    } else if filename.contains("M3C") {
        "M3".to_string()
    } else {
        "M6".to_string()
    };
    
    // Extract observation time from _s{timestamp}_
    let observation_time = filename
        .find("_s")
        .and_then(|pos| {
            let time_str = &filename[pos + 2..pos + 15]; // YYYYDDDHHMMSS
            parse_goes_timestamp(time_str)
        })?;
    
    // Extract product type
    let product = filename
        .find("ABI-L2-")
        .and_then(|pos| {
            let rest = &filename[pos + 7..];
            rest.split('-').next().map(|s| s.to_string())
        })
        .unwrap_or_else(|| "MCMIPC".to_string());
    
    Some(GoesFileInfo {
        satellite: satellite.to_string(),
        band,
        scan_mode,
        observation_time,
        product,
    })
}

/// Parse GOES timestamp format: YYYYDDDHHMMSS (day of year format).
fn parse_goes_timestamp(time_str: &str) -> Option<DateTime<Utc>> {
    if time_str.len() < 13 {
        return None;
    }
    
    let year: i32 = time_str[0..4].parse().ok()?;
    let day_of_year: u32 = time_str[4..7].parse().ok()?;
    let hour: u32 = time_str[7..9].parse().ok()?;
    let minute: u32 = time_str[9..11].parse().ok()?;
    let second: u32 = time_str[11..13].parse().ok()?;
    
    // Convert day of year to month/day
    let naive_date = chrono::NaiveDate::from_yo_opt(year, day_of_year)?;
    let naive_time = chrono::NaiveTime::from_hms_opt(hour, minute, second)?;
    let naive_dt = NaiveDateTime::new(naive_date, naive_time);
    
    Some(Utc.from_utc_datetime(&naive_dt))
}

/// Get model-specific default bounding box.
///
/// Returns appropriate geographic bounds for each weather model.
pub fn get_model_bbox(model: &str) -> BoundingBox {
    match model {
        "hrrr" => BoundingBox::new(-122.719528, 21.138123, -60.917193, 47.842195),
        "mrms" => BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
        "gfs" => BoundingBox::new(0.0, -90.0, 360.0, 90.0),
        "goes16" => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
        "goes18" => BoundingBox::new(-165.0, 14.5, -90.0, 55.5),
        _ => BoundingBox::new(0.0, -90.0, 360.0, 90.0),
    }
}

/// Extract bounding box from GRIB2 grid definition.
pub fn get_bbox_from_grid(grid: &grib2_parser::sections::GridDefinition) -> BoundingBox {
    // Convert millidegrees to degrees
    let first_lat = grid.first_latitude_millidegrees as f64 / 1_000.0;
    let first_lon = grid.first_longitude_millidegrees as f64 / 1_000.0;
    let last_lat = grid.last_latitude_millidegrees as f64 / 1_000.0;
    let last_lon = grid.last_longitude_millidegrees as f64 / 1_000.0;
    
    // Determine min/max (grid might scan in different directions)
    let min_lat = first_lat.min(last_lat);
    let max_lat = first_lat.max(last_lat);
    let min_lon = first_lon.min(last_lon);
    let max_lon = first_lon.max(last_lon);
    
    // Handle longitude wrapping (GRIB2 may use 0-360 instead of -180-180)
    let (min_lon, max_lon) = if min_lon > 180.0 {
        (min_lon - 360.0, max_lon - 360.0)
    } else {
        (min_lon, max_lon)
    };
    
    BoundingBox::new(min_lon, min_lat, max_lon, max_lat)
}

/// Map GOES band number to parameter name.
pub fn goes_band_to_parameter(band: u8) -> String {
    format!("CMI_C{:02}", band)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    // ==================== File Type Detection ====================

    #[test]
    fn test_detect_file_type_grib2() {
        assert_eq!(detect_file_type("test.grib2"), FileType::Grib2);
        assert_eq!(detect_file_type("test.grb2"), FileType::Grib2);
        assert_eq!(detect_file_type("test.grib"), FileType::Grib2);
        assert_eq!(detect_file_type("/path/to/data.GRIB2"), FileType::Grib2);
    }

    #[test]
    fn test_detect_file_type_grib2_gz() {
        assert_eq!(detect_file_type("test.grib2.gz"), FileType::Grib2Gz);
        assert_eq!(detect_file_type("test.grb2.gz"), FileType::Grib2Gz);
        assert_eq!(detect_file_type("/path/to/data.GRIB2.GZ"), FileType::Grib2Gz);
    }

    #[test]
    fn test_detect_file_type_netcdf() {
        assert_eq!(detect_file_type("test.nc"), FileType::NetCdf);
        assert_eq!(detect_file_type("test.nc4"), FileType::NetCdf);
        assert_eq!(detect_file_type("test.netcdf"), FileType::NetCdf);
        assert_eq!(detect_file_type("/path/to/satellite.NC"), FileType::NetCdf);
    }

    #[test]
    fn test_detect_file_type_unknown() {
        assert_eq!(detect_file_type("test.txt"), FileType::Unknown);
        assert_eq!(detect_file_type("test.json"), FileType::Unknown);
        assert_eq!(detect_file_type("test"), FileType::Unknown);
        assert_eq!(detect_file_type(""), FileType::Unknown);
    }

    // ==================== Model Extraction ====================

    #[test]
    fn test_extract_model_gfs() {
        assert_eq!(extract_model_from_filename("gfs_20241201.grib2"), Some("gfs".to_string()));
        assert_eq!(extract_model_from_filename("GFS_CONUS_2024.grib2"), Some("gfs".to_string()));
        assert_eq!(extract_model_from_filename("/data/gfs/gfs.t00z.pgrb2.grib2"), Some("gfs".to_string()));
    }

    #[test]
    fn test_extract_model_hrrr() {
        assert_eq!(extract_model_from_filename("hrrr.t00z.wrfsfcf01.grib2"), Some("hrrr".to_string()));
        assert_eq!(extract_model_from_filename("HRRR_CONUS.grib2"), Some("hrrr".to_string()));
        assert_eq!(extract_model_from_filename("/data/hrrr/hrrr.grib2"), Some("hrrr".to_string()));
    }

    #[test]
    fn test_extract_model_mrms() {
        assert_eq!(extract_model_from_filename("MRMS_SeamlessHSR.grib2"), Some("mrms".to_string()));
        assert_eq!(extract_model_from_filename("mrms_reflectivity.grib2"), Some("mrms".to_string()));
        assert_eq!(extract_model_from_filename("MRMS_PrecipRate_00.00.grib2"), Some("mrms".to_string()));
    }

    #[test]
    fn test_extract_model_goes16() {
        assert_eq!(extract_model_from_filename("OR_ABI_G16_test.nc"), Some("goes16".to_string()));
        assert_eq!(extract_model_from_filename("OR_ABI-L2-MCMIPC-M6C02_G16_s20241217180021.nc"), Some("goes16".to_string()));
        assert_eq!(extract_model_from_filename("goes16_conus.nc"), Some("goes16".to_string()));
    }

    #[test]
    fn test_extract_model_goes18() {
        assert_eq!(extract_model_from_filename("OR_ABI_G18_test.nc"), Some("goes18".to_string()));
        assert_eq!(extract_model_from_filename("OR_ABI-L2-MCMIPC-M6C13_G18_s20241217180021.nc"), Some("goes18".to_string()));
        assert_eq!(extract_model_from_filename("goes18_west.nc"), Some("goes18".to_string()));
    }

    #[test]
    fn test_extract_model_unknown() {
        assert_eq!(extract_model_from_filename("unknown_data.grib2"), None);
        assert_eq!(extract_model_from_filename("random_file.nc"), None);
        assert_eq!(extract_model_from_filename(""), None);
    }

    // ==================== Forecast Hour Extraction ====================

    #[test]
    fn test_extract_forecast_hour_f_pattern() {
        assert_eq!(extract_forecast_hour("gfs_20241201_00z_f003.grib2"), Some(3));
        assert_eq!(extract_forecast_hour("gfs_20241201_00z_f012.grib2"), Some(12));
        assert_eq!(extract_forecast_hour("gfs_20241201_00z_f120.grib2"), Some(120));
        assert_eq!(extract_forecast_hour("model_f000.grib2"), Some(0));
    }

    #[test]
    fn test_extract_forecast_hour_hrrr_pattern() {
        assert_eq!(extract_forecast_hour("hrrr.t00z.wrfsfcf00.grib2"), Some(0));
        assert_eq!(extract_forecast_hour("hrrr.t00z.wrfsfcf12.grib2"), Some(12));
        assert_eq!(extract_forecast_hour("hrrr.t12z.wrfsfcf48.grib2"), Some(48));
    }

    #[test]
    fn test_extract_forecast_hour_z_f_pattern() {
        assert_eq!(extract_forecast_hour("test_z_f024"), Some(24));
        assert_eq!(extract_forecast_hour("gfs_00z_f006"), Some(6));
    }

    #[test]
    fn test_extract_forecast_hour_none() {
        assert_eq!(extract_forecast_hour("mrms_reflectivity.grib2"), None);
        assert_eq!(extract_forecast_hour("goes16_data.nc"), None);
        assert_eq!(extract_forecast_hour("no_forecast_hour"), None);
    }

    // ==================== MRMS Parameter Extraction ====================

    #[test]
    fn test_extract_mrms_param_seamless_hsr() {
        assert_eq!(extract_mrms_param("MRMS_SeamlessHSR_00.00.grib2"), Some("REFL".to_string()));
        assert_eq!(extract_mrms_param("SeamlessHSR_00.00_20241201.grib2"), Some("REFL".to_string()));
    }

    #[test]
    fn test_extract_mrms_param_reflectivity() {
        assert_eq!(extract_mrms_param("MRMS_Reflectivity_00.00.grib2"), Some("REFL".to_string()));
        assert_eq!(extract_mrms_param("MRMS_REFL_00.50.grib2"), Some("REFL".to_string()));
    }

    #[test]
    fn test_extract_mrms_param_precip_rate() {
        assert_eq!(extract_mrms_param("MRMS_PrecipRate_00.00.grib2"), Some("PRECIP_RATE".to_string()));
        assert_eq!(extract_mrms_param("MRMS_Precip_Rate_00.00.grib2"), Some("PRECIP_RATE".to_string()));
    }

    #[test]
    fn test_extract_mrms_param_qpe() {
        assert_eq!(extract_mrms_param("MRMS_QPE_01H_00.00.grib2"), Some("QPE_01H".to_string()));
        assert_eq!(extract_mrms_param("MRMS_QPE_00.00.grib2"), Some("QPE".to_string()));
    }

    #[test]
    fn test_extract_mrms_param_generic() {
        // Generic MRMS_ prefix extraction
        assert_eq!(extract_mrms_param("MRMS_SomeParam_00.00.grib2"), Some("SOMEPARAM".to_string()));
    }

    // ==================== GOES Filename Parsing ====================

    #[test]
    fn test_parse_goes_filename_goes16() {
        // GOES timestamp format: YYYYDDDHHMMSS where DDD is day of year
        // Day 352 of 2024 = December 17, 2024
        let filename = "OR_ABI-L2-MCMIPC-M6C02_G16_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");
        
        assert_eq!(info.satellite, "goes16");
        assert_eq!(info.band, 2);
        assert_eq!(info.scan_mode, "M6");
        assert_eq!(info.product, "MCMIPC");
        assert_eq!(info.observation_time.year(), 2024);
    }

    #[test]
    fn test_parse_goes_filename_goes18() {
        // Day 352 of 2024 = December 17, 2024
        let filename = "OR_ABI-L2-MCMIPC-M6C13_G18_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");
        
        assert_eq!(info.satellite, "goes18");
        assert_eq!(info.band, 13);
        assert_eq!(info.scan_mode, "M6");
    }

    #[test]
    fn test_parse_goes_filename_m3_scan_mode() {
        let filename = "OR_ABI-L2-MCMIPC-M3C02_G16_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");
        
        assert_eq!(info.scan_mode, "M3");
    }

    #[test]
    fn test_parse_goes_filename_invalid() {
        assert!(parse_goes_filename("invalid_filename.nc").is_none());
        assert!(parse_goes_filename("").is_none());
        assert!(parse_goes_filename("OR_ABI_G99_test.nc").is_none()); // Invalid satellite
    }

    // ==================== GOES Band to Parameter ====================

    #[test]
    fn test_goes_band_to_param_visible() {
        assert_eq!(goes_band_to_parameter(1), "CMI_C01");
        assert_eq!(goes_band_to_parameter(2), "CMI_C02");
    }

    #[test]
    fn test_goes_band_to_param_ir() {
        assert_eq!(goes_band_to_parameter(13), "CMI_C13");
        assert_eq!(goes_band_to_parameter(14), "CMI_C14");
        assert_eq!(goes_band_to_parameter(16), "CMI_C16");
    }

    #[test]
    fn test_goes_band_to_param_formatting() {
        // Verify zero-padding
        assert_eq!(goes_band_to_parameter(1), "CMI_C01");
        assert_eq!(goes_band_to_parameter(9), "CMI_C09");
        assert_eq!(goes_band_to_parameter(10), "CMI_C10");
    }

    // ==================== Model Bounding Boxes ====================

    #[test]
    fn test_get_model_bbox_hrrr() {
        let bbox = get_model_bbox("hrrr");
        // HRRR covers CONUS
        assert!(bbox.min_x < -100.0, "HRRR should extend west of -100°");
        assert!(bbox.max_x > -70.0, "HRRR should extend east of -70°");
        assert!(bbox.min_y > 20.0, "HRRR should be north of 20°N");
        assert!(bbox.max_y < 50.0, "HRRR should be south of 50°N");
    }

    #[test]
    fn test_get_model_bbox_gfs() {
        let bbox = get_model_bbox("gfs");
        // GFS is global (0-360 longitude)
        assert_eq!(bbox.min_x, 0.0);
        assert_eq!(bbox.max_x, 360.0);
        assert_eq!(bbox.min_y, -90.0);
        assert_eq!(bbox.max_y, 90.0);
    }

    #[test]
    fn test_get_model_bbox_mrms() {
        let bbox = get_model_bbox("mrms");
        // MRMS covers CONUS and surrounding area
        assert!(bbox.min_x < -120.0);
        assert!(bbox.max_x > -70.0);
        assert!(bbox.min_y > 15.0);
        assert!(bbox.max_y < 60.0);
    }

    #[test]
    fn test_get_model_bbox_goes() {
        let bbox16 = get_model_bbox("goes16");
        let bbox18 = get_model_bbox("goes18");
        
        // GOES-16 (East) should be more easterly
        assert!(bbox16.max_x > bbox18.max_x, "GOES-16 should extend further east");
        
        // GOES-18 (West) should be more westerly
        assert!(bbox18.min_x < bbox16.min_x, "GOES-18 should extend further west");
    }

    #[test]
    fn test_get_model_bbox_unknown() {
        let bbox = get_model_bbox("unknown_model");
        // Unknown models get global coverage
        assert_eq!(bbox.min_x, 0.0);
        assert_eq!(bbox.max_x, 360.0);
    }

    // ==================== FileType Enum ====================

    #[test]
    fn test_file_type_equality() {
        assert_eq!(FileType::Grib2, FileType::Grib2);
        assert_ne!(FileType::Grib2, FileType::NetCdf);
        assert_ne!(FileType::Grib2Gz, FileType::Grib2);
    }

    #[test]
    fn test_file_type_debug() {
        // Ensure FileType implements Debug
        let ft = FileType::Grib2;
        let debug_str = format!("{:?}", ft);
        assert!(debug_str.contains("Grib2"));
    }
}
