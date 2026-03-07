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
    /// NDFD GRIB2 format (with WMO bulletin headers)
    NdfdGrib2,
    /// NetCDF format (GOES satellite)
    NetCdf,
    /// GeoTIFF format (VIIRS light pollution)
    GeoTiff,
    /// Gzip-compressed GeoTIFF
    GeoTiffGz,
    /// Unknown format
    Unknown,
}

/// Information extracted from a NASA LIS (Land Information System) filename.
///
/// Covers NLDAS-2, GLDAS-2.1, and other LIS products served from GES DISC.
#[derive(Debug, Clone)]
pub struct LisFileInfo {
    /// Model identifier (e.g., nldas-noah, nldas-forcing, gldas-noah)
    pub model: String,
    /// Observation time
    pub observation_time: DateTime<Utc>,
}

/// Backward-compatible type alias for `LisFileInfo`.
///
/// Renamed from `NldasFileInfo` to `LisFileInfo` when GLDAS support was added,
/// since the struct now covers all LIS products, not just NLDAS.
pub type NldasFileInfo = LisFileInfo;

/// Information extracted from a GOES filename.
#[derive(Debug, Clone)]
pub struct GoesFileInfo {
    /// Satellite identifier (goes18, goes19)
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

    if lower.ends_with(".tif.gz") || lower.ends_with(".tiff.gz") {
        FileType::GeoTiffGz
    } else if lower.ends_with(".tif") || lower.ends_with(".tiff") {
        FileType::GeoTiff
    } else if lower.ends_with(".grib2.gz") || lower.ends_with(".grb2.gz") {
        FileType::Grib2Gz
    } else if lower.ends_with(".grib2") || lower.ends_with(".grb2") || lower.ends_with(".grib") {
        FileType::Grib2
    } else if lower.ends_with(".nc") || lower.ends_with(".nc4") || lower.ends_with(".netcdf") {
        FileType::NetCdf
    } else if lower.contains("ndfd") || (lower.starts_with("ds.") && lower.ends_with(".bin")) {
        // NDFD files use ds.{element}.bin naming (e.g., ds.temp.bin, ds.wspd.bin)
        FileType::NdfdGrib2
    } else {
        FileType::Unknown
    }
}

/// Extract model name from filename.
///
/// Supports: GFS, HRRR, MRMS, NDFD, NBM (all regions), GOES-18, GOES-19
pub fn extract_model_from_filename(file_path: &str) -> Option<String> {
    let filename = Path::new(file_path).file_name().and_then(|s| s.to_str())?;

    let lower = filename.to_lowercase();

    // GOES satellite detection - check for full disk (CMIPF) vs CONUS (CMIPC)
    let is_fulldisk = lower.contains("cmipf");
    if lower.contains("_g19_") || lower.contains("goes19") {
        if is_fulldisk {
            Some("goes19-fulldisk".to_string())
        } else {
            Some("goes19".to_string())
        }
    } else if lower.contains("_g18_") || lower.contains("goes18") {
        if is_fulldisk {
            Some("goes18-fulldisk".to_string())
        } else {
            Some("goes18".to_string())
        }
    } else if lower.starts_with("nldas_noah") {
        Some("nldas-noah".to_string())
    } else if lower.starts_with("nldas_fora") || lower.starts_with("nldas_for") {
        Some("nldas-forcing".to_string())
    } else if lower.starts_with("gldas_noah") {
        Some("gldas-noah".to_string())
    } else if lower.starts_with("hrrr") || lower.contains("hrrr") {
        Some("hrrr".to_string())
    } else if lower.starts_with("aigefs") || lower.contains("aigefs") {
        // AIGEFS - AI GFS Ensemble System (must check before AIGFS since name contains "aigfs")
        Some("aigefs".to_string())
    } else if lower.starts_with("aigfs") || lower.contains("aigfs") {
        // AIGFS - AI Global Forecast System (must check before GFS since name contains "gfs")
        Some("aigfs".to_string())
    } else if lower.starts_with("gfs") || lower.contains("gfs") {
        Some("gfs".to_string())
    } else if lower.starts_with("mrms_") || lower.contains("mrms") {
        Some("mrms".to_string())
    } else if lower.contains("ndfd") || lower.starts_with("ds.") {
        // NDFD files use ds.{element}.bin naming (e.g., ds.temp.bin)
        Some("ndfd".to_string())
    } else if lower.starts_with("nbm-") {
        // NBM files are named like nbm-conus_*, nbm-alaska_*, nbm-hawaii_*, etc.
        // Extract the full model name including region (e.g., "nbm-conus", "nbm-alaska")
        if let Some(underscore_pos) = lower.find('_') {
            Some(lower[..underscore_pos].to_string())
        } else {
            // Fallback: just "nbm" if no underscore found
            Some("nbm".to_string())
        }
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
    let filename = Path::new(file_path).file_stem().and_then(|s| s.to_str())?;

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
    let filename = Path::new(file_path).file_name().and_then(|s| s.to_str())?;

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
        filename
            .strip_prefix("MRMS_")
            .and_then(|rest| rest.split('_').next())
            .map(|p| p.to_uppercase())
    } else {
        None
    }
}

/// Parse GOES filename to extract metadata.
///
/// Example: `OR_ABI-L2-CMIPC-M6C02_G18_s20241217180021_...`
/// Full disk: `OR_ABI-L2-CMIPF-M6C02_G19_s20241217180021_...`
pub fn parse_goes_filename(filename: &str) -> Option<GoesFileInfo> {
    let lower = filename.to_lowercase();
    let is_fulldisk = lower.contains("cmipf");

    // Extract satellite from _G18_ or _G19_, including full disk suffix
    let satellite = if filename.contains("_G19_") {
        if is_fulldisk {
            "goes19-fulldisk"
        } else {
            "goes19"
        }
    } else if filename.contains("_G18_") {
        if is_fulldisk {
            "goes18-fulldisk"
        } else {
            "goes18"
        }
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
    let observation_time = filename.find("_s").and_then(|pos| {
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
        "goes19" => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
        "goes18" => BoundingBox::new(-175.0, 14.5, -100.0, 55.5),
        // GOES Full Disk - hemisphere coverage
        // GOES-19 (East at -75.2°): visible from ~-156° to ~+6° longitude
        "goes19-fulldisk" => BoundingBox::new(-156.3, -81.3, 6.3, 81.3),
        // GOES-18 (West at -137.2°): visible from ~-218° to ~-56° longitude
        // Note: Uses wrapped longitude for web mapping compatibility
        "goes18-fulldisk" => BoundingBox::new(-218.5, -81.3, -55.9, 81.3),
        // NDFD CONUS: Lambert Conformal projection covering continental US
        // Actual geographic bounds from Lambert projection corners:
        // SW (0,0): lat=20.19, lon=-121.55; NW (0,1376): lat=49.94, lon=-130.10
        // SE (2144,0): lat=20.33, lon=-69.21; NE (2144,1376): lat=50.11, lon=-60.89
        "ndfd" => BoundingBox::new(-130.1, 20.19, -60.89, 50.11),
        // NBM (National Blend of Models) regional grids
        // CONUS: Lambert Conformal 2.5km (2345x1597)
        "nbm-conus" => BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
        // Alaska: Polar Stereographic ~3km (1649x1105)
        // Grid spans from ~150°E to ~-94°W, crossing the Date Line
        // Using Western hemisphere notation: -210° to -94° (lon range ~116°)
        // Or approximately -170° to -130° for the main Alaska area
        "nbm-alaska" => BoundingBox::new(-180.0, 40.5, -120.0, 75.0),
        // Hawaii: Mercator 2.5km (625x561)
        "nbm-hawaii" => BoundingBox::new(-164.0, 15.0, -150.0, 26.0),
        // Puerto Rico/USVI: Mercator 2.5km (339x225)
        "nbm-puertorico" => BoundingBox::new(-69.0, 16.0, -63.0, 20.0),
        // Guam/Marianas: Mercator 2.5km (193x193)
        "nbm-guam" => BoundingBox::new(141.0, 11.0, 148.0, 18.0),
        // NLDAS-2 covers North America at 0.125° resolution
        "nldas-noah" | "nldas-forcing" => BoundingBox::new(-124.9375, 25.0625, -67.0625, 52.9375),
        // GLDAS-2.1 covers global land at 0.25° resolution
        "gldas-noah" => BoundingBox::new(-179.875, -59.875, 179.875, 89.875),
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

/// Parse a NASA LIS filename to extract metadata.
///
/// Supports NLDAS-2, GLDAS-2.1 EP, and other LIS products that use the
/// `.A{YYYYMMDD}.{HHMM}` date convention in their filenames.
///
/// Examples:
/// - NLDAS Noah: `NLDAS_NOAH0125_H.A20260205.0000.020.nc`
/// - NLDAS Forcing: `NLDAS_FORA0125_H.A20260205.0000.020.nc` (or `.nc4`)
/// - GLDAS Noah EP: `GLDAS_NOAH025_3H_EP.A20260110.0000.021.nc4`
pub fn parse_lis_filename(filename: &str) -> Option<LisFileInfo> {
    let upper = filename.to_uppercase();

    // Determine model
    let model = if upper.starts_with("NLDAS_NOAH") {
        "nldas-noah".to_string()
    } else if upper.starts_with("NLDAS_FORA") || upper.starts_with("NLDAS_FOR") {
        "nldas-forcing".to_string()
    } else if upper.starts_with("GLDAS_NOAH") {
        "gldas-noah".to_string()
    } else {
        return None;
    };

    // Extract date from .A{YYYYMMDD}.{HHMM} pattern
    let dot_a_pos = filename.find(".A").or_else(|| filename.find(".a"))?;
    let date_str = filename.get(dot_a_pos + 2..dot_a_pos + 10)?;

    // Find the hour part after the next dot
    let rest = filename.get(dot_a_pos + 10..)?;
    let hour_str = if rest.starts_with('.') {
        rest.get(1..5)? // ".HHMM"
    } else {
        return None;
    };

    // Parse date components
    let year: i32 = date_str[0..4].parse().ok()?;
    let month: u32 = date_str[4..6].parse().ok()?;
    let day: u32 = date_str[6..8].parse().ok()?;
    let hour: u32 = hour_str[0..2].parse().ok()?;
    let minute: u32 = hour_str[2..4].parse().ok()?;

    let naive_date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let naive_time = chrono::NaiveTime::from_hms_opt(hour, minute, 0)?;
    let naive_dt = chrono::NaiveDateTime::new(naive_date, naive_time);
    let observation_time = TimeZone::from_utc_datetime(&Utc, &naive_dt);

    Some(LisFileInfo {
        model,
        observation_time,
    })
}

/// Backward-compatible alias for `parse_lis_filename`.
pub fn parse_nldas_filename(filename: &str) -> Option<LisFileInfo> {
    parse_lis_filename(filename)
}

/// Map GOES band number to parameter name.
pub fn goes_band_to_parameter(band: u8) -> String {
    format!("CMI_C{:02}", band)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

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
        assert_eq!(
            detect_file_type("/path/to/data.GRIB2.GZ"),
            FileType::Grib2Gz
        );
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

    #[test]
    fn test_detect_file_type_ndfd() {
        // NDFD ds.*.bin naming convention
        assert_eq!(detect_file_type("ds.temp.bin"), FileType::NdfdGrib2);
        assert_eq!(detect_file_type("ds.wspd.bin"), FileType::NdfdGrib2);
        assert_eq!(detect_file_type("ds.maxt.bin"), FileType::NdfdGrib2);
        assert_eq!(
            detect_file_type("/data/ndfd/ds.qpf.bin"),
            FileType::NdfdGrib2
        );
        // ndfd in filename with unknown extension
        assert_eq!(detect_file_type("ndfd_conus_temp.bin"), FileType::NdfdGrib2);
        // Note: files with .grib2 extension are detected as Grib2, not NdfdGrib2
        // This is intentional - NDFD files from NWS use .bin extension
    }

    #[test]
    fn test_detect_file_type_geotiff() {
        // Uncompressed GeoTIFF
        assert_eq!(detect_file_type("test.tif"), FileType::GeoTiff);
        assert_eq!(detect_file_type("test.tiff"), FileType::GeoTiff);
        assert_eq!(detect_file_type("VNL_v22_npp_2023.TIF"), FileType::GeoTiff);
        assert_eq!(
            detect_file_type("/data/viirs/nighttime_lights.tiff"),
            FileType::GeoTiff
        );
    }

    #[test]
    fn test_detect_file_type_geotiff_gz() {
        // Gzip-compressed GeoTIFF (VIIRS data is distributed this way)
        assert_eq!(detect_file_type("test.tif.gz"), FileType::GeoTiffGz);
        assert_eq!(detect_file_type("test.tiff.gz"), FileType::GeoTiffGz);
        assert_eq!(
            detect_file_type("VNL_v22_npp_2023_global_vcmslcfg.average_masked.dat.tif.gz"),
            FileType::GeoTiffGz
        );
        assert_eq!(
            detect_file_type("/data/static/viirs/VNL_data.TIF.GZ"),
            FileType::GeoTiffGz
        );
    }

    // ==================== Model Extraction ====================

    #[test]
    fn test_extract_model_gfs() {
        assert_eq!(
            extract_model_from_filename("gfs_20241201.grib2"),
            Some("gfs".to_string())
        );
        assert_eq!(
            extract_model_from_filename("GFS_CONUS_2024.grib2"),
            Some("gfs".to_string())
        );
        assert_eq!(
            extract_model_from_filename("/data/gfs/gfs.t00z.pgrb2.grib2"),
            Some("gfs".to_string())
        );
    }

    #[test]
    fn test_extract_model_aigfs() {
        assert_eq!(
            extract_model_from_filename("aigfs_pres_20260130_00z_f000.grib2"),
            Some("aigfs".to_string())
        );
        assert_eq!(
            extract_model_from_filename("aigfs_sfc_20260130_00z_f006.grib2"),
            Some("aigfs".to_string())
        );
        assert_eq!(
            extract_model_from_filename("/data/downloads/aigfs_pres_20260130_12z_f012.grib2"),
            Some("aigfs".to_string())
        );
    }

    #[test]
    fn test_extract_model_hrrr() {
        assert_eq!(
            extract_model_from_filename("hrrr.t00z.wrfsfcf01.grib2"),
            Some("hrrr".to_string())
        );
        assert_eq!(
            extract_model_from_filename("HRRR_CONUS.grib2"),
            Some("hrrr".to_string())
        );
        assert_eq!(
            extract_model_from_filename("/data/hrrr/hrrr.grib2"),
            Some("hrrr".to_string())
        );
    }

    #[test]
    fn test_extract_model_mrms() {
        assert_eq!(
            extract_model_from_filename("MRMS_SeamlessHSR.grib2"),
            Some("mrms".to_string())
        );
        assert_eq!(
            extract_model_from_filename("mrms_reflectivity.grib2"),
            Some("mrms".to_string())
        );
        assert_eq!(
            extract_model_from_filename("MRMS_PrecipRate_00.00.grib2"),
            Some("mrms".to_string())
        );
    }

    #[test]
    fn test_extract_model_goes19() {
        // CONUS files
        assert_eq!(
            extract_model_from_filename("OR_ABI_G19_test.nc"),
            Some("goes19".to_string())
        );
        assert_eq!(
            extract_model_from_filename("OR_ABI-L2-CMIPC-M6C02_G19_s20241217180021.nc"),
            Some("goes19".to_string())
        );
        assert_eq!(
            extract_model_from_filename("goes19_conus.nc"),
            Some("goes19".to_string())
        );
    }

    #[test]
    fn test_extract_model_goes19_fulldisk() {
        // Full disk files have CMIPF in the name
        assert_eq!(
            extract_model_from_filename("OR_ABI-L2-CMIPF-M6C02_G19_s20241217180021.nc"),
            Some("goes19-fulldisk".to_string())
        );
        assert_eq!(
            extract_model_from_filename("OR_ABI-L2-CMIPF-M6C13_G19_s20241217180021.nc"),
            Some("goes19-fulldisk".to_string())
        );
    }

    #[test]
    fn test_extract_model_goes18() {
        // CONUS files
        assert_eq!(
            extract_model_from_filename("OR_ABI_G18_test.nc"),
            Some("goes18".to_string())
        );
        assert_eq!(
            extract_model_from_filename("OR_ABI-L2-CMIPC-M6C13_G18_s20241217180021.nc"),
            Some("goes18".to_string())
        );
        assert_eq!(
            extract_model_from_filename("goes18_west.nc"),
            Some("goes18".to_string())
        );
    }

    #[test]
    fn test_extract_model_goes18_fulldisk() {
        // Full disk files have CMIPF in the name
        assert_eq!(
            extract_model_from_filename("OR_ABI-L2-CMIPF-M6C02_G18_s20241217180021.nc"),
            Some("goes18-fulldisk".to_string())
        );
        assert_eq!(
            extract_model_from_filename("OR_ABI-L2-CMIPF-M6C08_G18_s20241217180021.nc"),
            Some("goes18-fulldisk".to_string())
        );
    }

    #[test]
    fn test_extract_model_unknown() {
        assert_eq!(extract_model_from_filename("unknown_data.grib2"), None);
        assert_eq!(extract_model_from_filename("random_file.nc"), None);
        assert_eq!(extract_model_from_filename(""), None);
    }

    #[test]
    fn test_extract_model_ndfd() {
        // NDFD ds.*.bin naming convention
        assert_eq!(
            extract_model_from_filename("ds.temp.bin"),
            Some("ndfd".to_string())
        );
        assert_eq!(
            extract_model_from_filename("ds.wspd.bin"),
            Some("ndfd".to_string())
        );
        assert_eq!(
            extract_model_from_filename("/data/ndfd/ds.maxt.bin"),
            Some("ndfd".to_string())
        );
        // Also recognize by ndfd in path
        assert_eq!(
            extract_model_from_filename("ndfd_conus_temp.grib2"),
            Some("ndfd".to_string())
        );
    }

    #[test]
    fn test_extract_model_nbm() {
        // NBM regional files
        assert_eq!(
            extract_model_from_filename("nbm-conus_20260108_12z_f006.grib2"),
            Some("nbm-conus".to_string())
        );
        assert_eq!(
            extract_model_from_filename("nbm-alaska_20260108_00z_f012.grib2"),
            Some("nbm-alaska".to_string())
        );
        assert_eq!(
            extract_model_from_filename("nbm-hawaii_20260108_06z_f024.grib2"),
            Some("nbm-hawaii".to_string())
        );
        assert_eq!(
            extract_model_from_filename("nbm-puertorico_20260108_18z_f001.grib2"),
            Some("nbm-puertorico".to_string())
        );
        assert_eq!(
            extract_model_from_filename("nbm-guam_20260108_21z_f048.grib2"),
            Some("nbm-guam".to_string())
        );
        // With full path
        assert_eq!(
            extract_model_from_filename("/data/downloads/nbm-hawaii_20260108_21z_f015.grib2"),
            Some("nbm-hawaii".to_string())
        );
    }

    // ==================== Forecast Hour Extraction ====================

    #[test]
    fn test_extract_forecast_hour_f_pattern() {
        assert_eq!(
            extract_forecast_hour("gfs_20241201_00z_f003.grib2"),
            Some(3)
        );
        assert_eq!(
            extract_forecast_hour("gfs_20241201_00z_f012.grib2"),
            Some(12)
        );
        assert_eq!(
            extract_forecast_hour("gfs_20241201_00z_f120.grib2"),
            Some(120)
        );
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
        assert_eq!(extract_forecast_hour("goes19_data.nc"), None);
        assert_eq!(extract_forecast_hour("no_forecast_hour"), None);
    }

    // ==================== MRMS Parameter Extraction ====================

    #[test]
    fn test_extract_mrms_param_seamless_hsr() {
        assert_eq!(
            extract_mrms_param("MRMS_SeamlessHSR_00.00.grib2"),
            Some("REFL".to_string())
        );
        assert_eq!(
            extract_mrms_param("SeamlessHSR_00.00_20241201.grib2"),
            Some("REFL".to_string())
        );
    }

    #[test]
    fn test_extract_mrms_param_reflectivity() {
        assert_eq!(
            extract_mrms_param("MRMS_Reflectivity_00.00.grib2"),
            Some("REFL".to_string())
        );
        assert_eq!(
            extract_mrms_param("MRMS_REFL_00.50.grib2"),
            Some("REFL".to_string())
        );
    }

    #[test]
    fn test_extract_mrms_param_precip_rate() {
        assert_eq!(
            extract_mrms_param("MRMS_PrecipRate_00.00.grib2"),
            Some("PRECIP_RATE".to_string())
        );
        assert_eq!(
            extract_mrms_param("MRMS_Precip_Rate_00.00.grib2"),
            Some("PRECIP_RATE".to_string())
        );
    }

    #[test]
    fn test_extract_mrms_param_qpe() {
        assert_eq!(
            extract_mrms_param("MRMS_QPE_01H_00.00.grib2"),
            Some("QPE_01H".to_string())
        );
        assert_eq!(
            extract_mrms_param("MRMS_QPE_00.00.grib2"),
            Some("QPE".to_string())
        );
    }

    #[test]
    fn test_extract_mrms_param_generic() {
        // Generic MRMS_ prefix extraction
        assert_eq!(
            extract_mrms_param("MRMS_SomeParam_00.00.grib2"),
            Some("SOMEPARAM".to_string())
        );
    }

    // ==================== GOES Filename Parsing ====================

    #[test]
    fn test_parse_goes_filename_goes19_conus() {
        // GOES timestamp format: YYYYDDDHHMMSS where DDD is day of year
        // Day 352 of 2024 = December 17, 2024
        // CMIPC = CONUS product
        let filename =
            "OR_ABI-L2-CMIPC-M6C02_G19_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");

        assert_eq!(info.satellite, "goes19");
        assert_eq!(info.band, 2);
        assert_eq!(info.scan_mode, "M6");
        assert_eq!(info.product, "CMIPC");
        assert_eq!(info.observation_time.year(), 2024);
    }

    #[test]
    fn test_parse_goes_filename_goes19_fulldisk() {
        // CMIPF = Full Disk product
        let filename =
            "OR_ABI-L2-CMIPF-M6C02_G19_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");

        assert_eq!(info.satellite, "goes19-fulldisk");
        assert_eq!(info.band, 2);
        assert_eq!(info.scan_mode, "M6");
        assert_eq!(info.product, "CMIPF");
    }

    #[test]
    fn test_parse_goes_filename_goes18_conus() {
        // Day 352 of 2024 = December 17, 2024
        let filename =
            "OR_ABI-L2-CMIPC-M6C13_G18_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");

        assert_eq!(info.satellite, "goes18");
        assert_eq!(info.band, 13);
        assert_eq!(info.scan_mode, "M6");
    }

    #[test]
    fn test_parse_goes_filename_goes18_fulldisk() {
        // CMIPF = Full Disk product
        let filename =
            "OR_ABI-L2-CMIPF-M6C08_G18_s20243521800210_e20243521800210_c20243521800210.nc";
        let info = parse_goes_filename(filename).expect("Should parse");

        assert_eq!(info.satellite, "goes18-fulldisk");
        assert_eq!(info.band, 8);
        assert_eq!(info.scan_mode, "M6");
        assert_eq!(info.product, "CMIPF");
    }

    #[test]
    fn test_parse_goes_filename_m3_scan_mode() {
        let filename =
            "OR_ABI-L2-CMIPC-M3C02_G19_s20243521800210_e20243521800210_c20243521800210.nc";
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
        let bbox19 = get_model_bbox("goes19");
        let bbox18 = get_model_bbox("goes18");

        // GOES-19 (East) should be more easterly
        assert!(
            bbox19.max_x > bbox18.max_x,
            "GOES-19 should extend further east"
        );

        // GOES-18 (West) should be more westerly
        assert!(
            bbox18.min_x < bbox19.min_x,
            "GOES-18 should extend further west"
        );
    }

    #[test]
    fn test_get_model_bbox_goes_fulldisk() {
        let bbox19_fd = get_model_bbox("goes19-fulldisk");
        let bbox18_fd = get_model_bbox("goes18-fulldisk");
        let bbox19_conus = get_model_bbox("goes19");
        let _bbox18_conus = get_model_bbox("goes18");

        // Full disk should have much larger coverage than CONUS
        assert!(
            bbox19_fd.min_y < bbox19_conus.min_y,
            "GOES-19 Full Disk should extend further south"
        );
        assert!(
            bbox19_fd.max_y > bbox19_conus.max_y,
            "GOES-19 Full Disk should extend further north"
        );

        // Full disk covers hemisphere (~162 degrees of longitude)
        let fd19_lon_range = bbox19_fd.max_x - bbox19_fd.min_x;
        assert!(
            fd19_lon_range > 150.0,
            "GOES-19 Full Disk should span >150 degrees, got {}",
            fd19_lon_range
        );

        // Full disk latitude should extend to ~81 degrees
        assert!(
            bbox19_fd.max_y > 80.0,
            "GOES-19 Full Disk should reach >80°N, got {}",
            bbox19_fd.max_y
        );
        assert!(
            bbox18_fd.min_y < -80.0,
            "GOES-18 Full Disk should reach <-80°S, got {}",
            bbox18_fd.min_y
        );
    }

    #[test]
    fn test_get_model_bbox_unknown() {
        let bbox = get_model_bbox("unknown_model");
        // Unknown models get global coverage
        assert_eq!(bbox.min_x, 0.0);
        assert_eq!(bbox.max_x, 360.0);
    }

    #[test]
    fn test_get_model_bbox_ndfd() {
        let bbox = get_model_bbox("ndfd");
        // NDFD covers CONUS - Lambert Conformal projection with corners:
        // SW: (20.19°, -121.55°), SE: (20.33°, -69.21°)
        // NW: (49.94°, -130.10°), NE: (50.11°, -60.89°)
        assert!(bbox.min_x < -120.0, "NDFD should extend west of -120°");
        assert!(bbox.max_x > -70.0, "NDFD should extend east of -70°");
        assert!(bbox.min_y >= 20.0, "NDFD should be at or north of 20°N");
        assert!(bbox.max_y < 55.0, "NDFD should be south of 55°N");
    }

    #[test]
    fn test_get_model_bbox_nbm() {
        // NBM CONUS covers continental US
        let bbox_conus = get_model_bbox("nbm-conus");
        assert!(
            bbox_conus.min_x < -120.0,
            "NBM CONUS should extend west of -120°"
        );
        assert!(
            bbox_conus.max_x > -70.0,
            "NBM CONUS should extend east of -70°"
        );
        assert!(
            bbox_conus.min_y >= 15.0,
            "NBM CONUS should be at or north of 15°N"
        );
        assert!(bbox_conus.max_y < 60.0, "NBM CONUS should be south of 60°N");

        // NBM Alaska
        let bbox_ak = get_model_bbox("nbm-alaska");
        assert!(
            bbox_ak.min_x < -170.0,
            "NBM Alaska should extend west of -170°"
        );
        assert!(
            bbox_ak.max_y > 70.0,
            "NBM Alaska should extend north of 70°N"
        );

        // NBM Hawaii
        let bbox_hi = get_model_bbox("nbm-hawaii");
        assert!(
            bbox_hi.min_x < -160.0 && bbox_hi.min_x > -170.0,
            "NBM Hawaii should be around -160°"
        );
        assert!(
            bbox_hi.min_y > 10.0 && bbox_hi.max_y < 30.0,
            "NBM Hawaii should be around 15-25°N"
        );

        // NBM Puerto Rico
        let bbox_pr = get_model_bbox("nbm-puertorico");
        assert!(
            bbox_pr.min_x > -70.0 && bbox_pr.max_x < -62.0,
            "NBM PR should be around -67°"
        );
        assert!(
            bbox_pr.min_y > 15.0 && bbox_pr.max_y < 21.0,
            "NBM PR should be around 18°N"
        );

        // NBM Guam
        let bbox_gu = get_model_bbox("nbm-guam");
        assert!(
            bbox_gu.min_x > 140.0 && bbox_gu.max_x < 150.0,
            "NBM Guam should be around 145°E"
        );
        assert!(
            bbox_gu.min_y > 10.0 && bbox_gu.max_y < 20.0,
            "NBM Guam should be around 13°N"
        );
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

    #[test]
    fn test_file_type_copy_clone() {
        let ft = FileType::NetCdf;
        let ft2 = ft; // Copy
        let ft3 = ft.clone(); // Clone
        assert_eq!(ft, ft2);
        assert_eq!(ft, ft3);
    }

    #[test]
    fn test_goes_file_info_debug_clone() {
        let info = GoesFileInfo {
            satellite: "goes19".to_string(),
            band: 2,
            scan_mode: "M6".to_string(),
            observation_time: Utc::now(),
            product: "CMIPC".to_string(),
        };
        let cloned = info.clone();
        assert_eq!(info.satellite, cloned.satellite);
        assert_eq!(info.band, cloned.band);

        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("GoesFileInfo"));
        assert!(debug_str.contains("goes19"));
    }

    #[test]
    fn test_extract_model_aigefs() {
        // AIGEFS must be tested before AIGFS due to string matching order
        assert_eq!(
            extract_model_from_filename("aigefs_pres_20260130_00z_f000.grib2"),
            Some("aigefs".to_string())
        );
        assert_eq!(
            extract_model_from_filename("AIGEFS_sfc_20260130_12z_f012.grib2"),
            Some("aigefs".to_string())
        );
    }

    #[test]
    fn test_parse_goes_filename_timestamp_edge_cases() {
        // Test with different days of year
        // Day 1 = January 1
        let filename =
            "OR_ABI-L2-CMIPC-M6C02_G19_s20240011200000_e20240011200000_c20240011200000.nc";
        let info = parse_goes_filename(filename).expect("Should parse");
        assert_eq!(info.observation_time.month(), 1);
        assert_eq!(info.observation_time.day(), 1);

        // Day 365 = December 31 (non-leap year) or December 30 (leap year 2024)
        let filename =
            "OR_ABI-L2-CMIPC-M6C02_G19_s20243660000000_e20243660000000_c20243660000000.nc";
        let info = parse_goes_filename(filename).expect("Should parse leap year day 366");
        assert_eq!(info.observation_time.month(), 12);
        assert_eq!(info.observation_time.day(), 31); // 2024 is leap year, day 366 = Dec 31
    }

    #[test]
    fn test_extract_forecast_hour_edge_cases() {
        // Test with leading zeros
        assert_eq!(extract_forecast_hour("model_f000.grib2"), Some(0));
        assert_eq!(extract_forecast_hour("model_f001.grib2"), Some(1));

        // Test pattern at different positions
        assert_eq!(
            extract_forecast_hour("prefix_model_f024_suffix.grib2"),
            Some(24)
        );
    }

    #[test]
    fn test_extract_mrms_param_none() {
        // Files without MRMS pattern
        assert_eq!(extract_mrms_param("gfs_temperature.grib2"), None);
        assert_eq!(extract_mrms_param("random_file.nc"), None);
    }

    // ==================== NLDAS Model Extraction ====================

    #[test]
    fn test_extract_model_nldas_noah() {
        assert_eq!(
            extract_model_from_filename("NLDAS_NOAH0125_H.A20260205.0000.020.nc"),
            Some("nldas-noah".to_string())
        );
        assert_eq!(
            extract_model_from_filename("nldas_noah0125_h.a20260205.0000.020.nc"),
            Some("nldas-noah".to_string())
        );
    }

    #[test]
    fn test_extract_model_nldas_forcing() {
        assert_eq!(
            extract_model_from_filename("NLDAS_FORA0125_H.002.grb.SUB.nc4"),
            Some("nldas-forcing".to_string())
        );
    }

    // ==================== NLDAS Filename Parsing ====================

    #[test]
    fn test_parse_nldas_noah_filename() {
        let info = parse_nldas_filename("NLDAS_NOAH0125_H.A20260205.0000.020.nc")
            .expect("Should parse NLDAS Noah filename");
        assert_eq!(info.model, "nldas-noah");
        assert_eq!(info.observation_time.year(), 2026);
        assert_eq!(info.observation_time.month(), 2);
        assert_eq!(info.observation_time.day(), 5);
        assert_eq!(info.observation_time.hour(), 0);
        assert_eq!(info.observation_time.minute(), 0);
    }

    #[test]
    fn test_parse_nldas_noah_filename_nonzero_hour() {
        let info =
            parse_nldas_filename("NLDAS_NOAH0125_H.A20260210.1300.020.nc").expect("Should parse");
        assert_eq!(info.observation_time.hour(), 13);
        assert_eq!(info.observation_time.minute(), 0);
    }

    #[test]
    fn test_parse_nldas_forcing_filename() {
        let info = parse_nldas_filename("NLDAS_FORA0125_H.A20260205.0000.020.nc")
            .expect("Should parse NLDAS Forcing filename");
        assert_eq!(info.model, "nldas-forcing");
        assert_eq!(info.observation_time.year(), 2026);
    }

    #[test]
    fn test_parse_nldas_filename_invalid() {
        assert!(parse_nldas_filename("random_file.nc").is_none());
        assert!(parse_nldas_filename("GOES_ABI_data.nc").is_none());
        assert!(parse_nldas_filename("").is_none());
    }

    // ==================== GLDAS Model Extraction ====================

    #[test]
    fn test_extract_model_gldas_noah() {
        assert_eq!(
            extract_model_from_filename("GLDAS_NOAH025_3H_EP.A20260110.0000.021.nc4"),
            Some("gldas-noah".to_string())
        );
        assert_eq!(
            extract_model_from_filename("gldas_noah025_3h_ep.a20260110.0000.021.nc4"),
            Some("gldas-noah".to_string())
        );
    }

    // ==================== GLDAS Filename Parsing ====================

    #[test]
    fn test_parse_gldas_noah_filename() {
        let info = parse_lis_filename("GLDAS_NOAH025_3H_EP.A20260110.0000.021.nc4")
            .expect("Should parse GLDAS Noah EP filename");
        assert_eq!(info.model, "gldas-noah");
        assert_eq!(info.observation_time.year(), 2026);
        assert_eq!(info.observation_time.month(), 1);
        assert_eq!(info.observation_time.day(), 10);
        assert_eq!(info.observation_time.hour(), 0);
        assert_eq!(info.observation_time.minute(), 0);
    }

    #[test]
    fn test_parse_gldas_noah_filename_nonzero_hour() {
        let info =
            parse_lis_filename("GLDAS_NOAH025_3H_EP.A20260110.1500.021.nc4").expect("Should parse");
        assert_eq!(info.model, "gldas-noah");
        assert_eq!(info.observation_time.hour(), 15);
        assert_eq!(info.observation_time.minute(), 0);
    }

    // ==================== NLDAS/GLDAS Bounding Box ====================

    #[test]
    fn test_get_model_bbox_nldas() {
        let bbox_noah = get_model_bbox("nldas-noah");
        assert!((bbox_noah.min_x - (-124.9375)).abs() < 0.01);
        assert!((bbox_noah.min_y - 25.0625).abs() < 0.01);
        assert!((bbox_noah.max_x - (-67.0625)).abs() < 0.01);
        assert!((bbox_noah.max_y - 52.9375).abs() < 0.01);

        let bbox_forcing = get_model_bbox("nldas-forcing");
        assert_eq!(bbox_noah.min_x, bbox_forcing.min_x);
        assert_eq!(bbox_noah.max_y, bbox_forcing.max_y);
    }

    #[test]
    fn test_get_model_bbox_gldas() {
        let bbox = get_model_bbox("gldas-noah");
        assert!((bbox.min_x - (-179.875)).abs() < 0.01);
        assert!((bbox.min_y - (-59.875)).abs() < 0.01);
        assert!((bbox.max_x - 179.875).abs() < 0.01);
        assert!((bbox.max_y - 89.875).abs() < 0.01);
    }

    #[test]
    fn test_get_bbox_from_grid_longitude_wrapping() {
        use grib2_parser::sections::GridDefinition;

        // Test grid with longitude > 180 (0-360 format)
        let grid = GridDefinition {
            grid_shape: 0,
            num_points_latitude: 50,
            num_points_longitude: 100,
            first_latitude_millidegrees: 40000,   // 40°N
            first_longitude_millidegrees: 200000, // 200° (should become -160°)
            last_latitude_millidegrees: 50000,    // 50°N
            last_longitude_millidegrees: 250000,  // 250° (should become -110°)
            latitude_increment_millidegrees: 500,
            longitude_increment_millidegrees: 500,
            scanning_mode: 0,
        };

        let bbox = get_bbox_from_grid(&grid);
        assert!(bbox.min_x < 0.0, "Longitude should be wrapped to negative");
        assert!(bbox.max_x < 0.0, "Longitude should be wrapped to negative");
        assert_eq!(bbox.min_x, -160.0);
        assert_eq!(bbox.max_x, -110.0);
    }

    #[test]
    fn test_get_bbox_from_grid_reversed_scan() {
        use grib2_parser::sections::GridDefinition;

        // Test grid where first > last (scanning in opposite direction)
        let grid = GridDefinition {
            grid_shape: 0,
            num_points_latitude: 50,
            num_points_longitude: 100,
            first_latitude_millidegrees: 50000,   // 50°N (higher)
            first_longitude_millidegrees: -60000, // -60°
            last_latitude_millidegrees: 20000,    // 20°N (lower)
            last_longitude_millidegrees: -130000, // -130°
            latitude_increment_millidegrees: 500,
            longitude_increment_millidegrees: 500,
            scanning_mode: 0,
        };

        let bbox = get_bbox_from_grid(&grid);
        // min/max should be sorted correctly regardless of scan order
        assert_eq!(bbox.min_y, 20.0);
        assert_eq!(bbox.max_y, 50.0);
        assert_eq!(bbox.min_x, -130.0);
        assert_eq!(bbox.max_x, -60.0);
    }
}
