//! GRIB2 section parsing.
//!
//! This module handles parsing of individual GRIB2 message sections.
//! Each GRIB2 message consists of multiple sections containing
//! metadata, grid information, and compressed data.

use crate::Grib2Error;
use bytes::Bytes;
use chrono::{DateTime, NaiveDate, Utc};

/// Section 0: Indicator Section (16 bytes)
#[derive(Debug, Clone)]
pub struct Indicator {
    pub magic: [u8; 4],
    pub reserved: u16,
    pub edition: u8,
    pub discipline: u8,
    pub message_length: u32,
}

/// Section 1: Identification Section
#[derive(Debug, Clone)]
pub struct Identification {
    pub center: u16,
    pub sub_center: u16,
    pub table_version: u8,
    pub local_table_version: u8,
    pub significance_of_reference_time: u8,
    pub reference_time: DateTime<Utc>,
    pub production_status: u8,
    pub data_type: u8,
}

/// Section 3: Grid Definition Section
#[derive(Debug, Clone)]
pub struct GridDefinition {
    pub grid_shape: u8,
    pub num_points_latitude: u32,
    pub num_points_longitude: u32,
    pub first_latitude_millidegrees: i32,
    pub first_longitude_millidegrees: i32,
    pub last_latitude_millidegrees: i32,
    pub last_longitude_millidegrees: i32,
    pub latitude_increment_millidegrees: u32,
    pub longitude_increment_millidegrees: u32,
    pub scanning_mode: u8,
}

/// Section 4: Product Definition Section
#[derive(Debug, Clone)]
pub struct ProductDefinition {
    pub parameter_category: u8,
    pub parameter_number: u8,
    pub parameter_short_name: String,
    pub level_type: u8,
    pub level_value: u32,
    pub level_description: String,
    pub forecast_hour: u32,
}

/// Section 5: Data Representation Section
#[derive(Debug, Clone)]
pub struct DataRepresentation {
    pub num_data_points: u32,
    pub packing_method: u8,
    pub original_data_type: u8,
    pub reference_value: f32,
    pub binary_scale_factor: i16,
    pub decimal_scale_factor: i16,
    pub bits_per_value: u8,
}

/// Section 6: Bitmap Section
#[derive(Debug, Clone)]
pub struct Bitmap {
    pub indicator: u8,
    pub data: Bytes,
}

/// Section 7: Data Section
#[derive(Debug, Clone)]
pub struct DataSection {
    pub data: Bytes,
}

// ===== Parsing Functions =====

/// Parse Section 0 (Indicator) from start of message
pub fn parse_indicator(data: &[u8]) -> Result<Indicator, Grib2Error> {
    if data.len() < 16 {
        return Err(Grib2Error::InvalidFormat(
            "Not enough data for indicator section".to_string(),
        ));
    }

    if &data[0..4] != b"GRIB" {
        return Err(Grib2Error::InvalidFormat(
            "Invalid GRIB magic bytes".to_string(),
        ));
    }

    // Section 0 (Indicator) layout per GRIB2 spec:
    // Octets 1-4: "GRIB" (indices 0-3)
    // Octets 5-6: Reserved (indices 4-5)  
    // Octet 7: Discipline (index 6)
    // Octet 8: GRIB Edition Number (index 7)
    // Octets 9-16: Total length of GRIB message (indices 8-15, 8-byte big-endian)
    let discipline = data[6];
    let edition = data[7];
    
    // Message length is 8 bytes at indices 8-15
    // Most GRIB2 files fit in u32, so we read the lower 4 bytes
    let message_length = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);

    if edition != 2 {
        return Err(Grib2Error::InvalidFormat(format!(
            "Expected GRIB edition 2, got {}",
            edition
        )));
    }

    Ok(Indicator {
        magic: [data[0], data[1], data[2], data[3]],
        reserved: u16::from_be_bytes([data[4], data[5]]),
        discipline,
        edition,
        message_length,
    })
}

/// Parse Section 1 (Identification)
/// Located at offset 16 in the message
pub fn parse_identification(data: &[u8]) -> Result<Identification, Grib2Error> {
    const OFFSET: usize = 16;
    
    if data.len() < OFFSET + 22 {
        return Err(Grib2Error::InvalidSection {
            section: 1,
            reason: "Not enough data".to_string(),
        });
    }

    // Skip section header (4 bytes) and section number (1 byte)
    let sec_data = &data[OFFSET + 5..];

    let center = u16::from_be_bytes([sec_data[0], sec_data[1]]);
    let sub_center = u16::from_be_bytes([sec_data[2], sec_data[3]]);
    let table_version = sec_data[4];
    let local_table_version = sec_data[5];
    let significance_of_reference_time = sec_data[6];

    // Reference time
    let year = u16::from_be_bytes([sec_data[7], sec_data[8]]);
    let month = sec_data[9];
    let day = sec_data[10];
    let hour = sec_data[11];
    let minute = sec_data[12];
    let second = sec_data[13];

    let reference_time = NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
        .and_then(|date| date.and_hms_opt(hour as u32, minute as u32, second as u32))
        .ok_or_else(|| Grib2Error::InvalidSection {
            section: 1,
            reason: format!("Invalid date: {}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hour, minute, second),
        })?;

    let reference_time = DateTime::<Utc>::from_naive_utc_and_offset(reference_time, Utc);

    let production_status = sec_data.get(14).copied().unwrap_or(0);
    let data_type = sec_data.get(15).copied().unwrap_or(0);

    Ok(Identification {
        center,
        sub_center,
        table_version,
        local_table_version,
        significance_of_reference_time,
        reference_time,
        production_status,
        data_type,
    })
}

/// Parse Section 3 (Grid Definition)
pub fn parse_grid_definition(data: &[u8]) -> Result<GridDefinition, Grib2Error> {
    // Section 3 typically follows Section 1, but can skip if Section 2 present
    // For now, find Section 3 by looking for section number 3
    let section_offset = find_section(data, 3)?;

    let section_data = &data[section_offset..];
    if section_data.len() < 72 {
        return Err(Grib2Error::InvalidSection {
            section: 3,
            reason: "Not enough data".to_string(),
        });
    }

    // Section 3 structure:
    // Bytes 0-3: Section length
    // Byte 4: Section number (3)
    // Byte 5: Source of grid definition
    // Bytes 6-9: Number of data points (u32)
    // Byte 10: Number of optional list
    // Byte 11: Interpretation of optional list
    // Bytes 12-13: Grid definition template number (u16)
    // Bytes 14+: Template-specific data
    
    let grid_template = u16::from_be_bytes([section_data[12], section_data[13]]);
    
    // Template data starts at byte 14
    let gd = &section_data[14..];
    
    if grid_template == 0 {
        // Template 0: Latitude/longitude (or equidistant cylindrical or Plate Carree)
        // GRIB2 Code Table 3.1 - Template 3.0
        //
        // Byte 0: Shape of the Earth (Table 3.2)
        // Byte 1: Scale factor of radius of spherical Earth
        // Bytes 2-5: Scaled value of radius of spherical Earth
        // Byte 6: Scale factor of major axis of oblate spheroid Earth
        // Bytes 7-10: Scaled value of major axis
        // Byte 11: Scale factor of minor axis
        // Bytes 12-15: Scaled value of minor axis
        // Bytes 16-19: Ni - number of points along a parallel (u32)
        // Bytes 20-23: Nj - number of points along a meridian (u32)
        // Bytes 24-27: Basic angle of the initial production domain
        // Bytes 28-31: Subdivisions of basic angle
        // Bytes 32-35: La1 - latitude of first grid point (i32, microdegrees)
        // Bytes 36-39: Lo1 - longitude of first grid point (i32, microdegrees)
        // Byte 40: Resolution and component flags
        // Bytes 41-44: La2 - latitude of last grid point (i32, microdegrees)
        // Bytes 45-48: Lo2 - longitude of last grid point (i32, microdegrees)
        // Bytes 49-52: Di - i direction increment (u32, microdegrees)
        // Bytes 53-56: Dj - j direction increment (u32, microdegrees)
        // Byte 57: Scanning mode (flags)
        
        if gd.len() < 58 {
            return Err(Grib2Error::InvalidSection {
                section: 3,
                reason: format!("Template 0 needs at least 58 bytes, got {}", gd.len()),
            });
        }
        
        let grid_shape = gd[0];
        let ni = u32::from_be_bytes([gd[16], gd[17], gd[18], gd[19]]);
        let nj = u32::from_be_bytes([gd[20], gd[21], gd[22], gd[23]]);
        
        // Latitudes and longitudes are in microdegrees (10^-6 degrees)
        let la1 = i32::from_be_bytes([gd[32], gd[33], gd[34], gd[35]]);
        let lo1 = i32::from_be_bytes([gd[36], gd[37], gd[38], gd[39]]);
        let la2 = i32::from_be_bytes([gd[41], gd[42], gd[43], gd[44]]);
        let lo2 = i32::from_be_bytes([gd[45], gd[46], gd[47], gd[48]]);
        let di = u32::from_be_bytes([gd[49], gd[50], gd[51], gd[52]]);
        let dj = u32::from_be_bytes([gd[53], gd[54], gd[55], gd[56]]);
        let scanning_mode = gd[57];
        
        // Convert from microdegrees to millidegrees (divide by 1000)
        // Note: Our struct uses millidegrees for historical reasons
        Ok(GridDefinition {
            grid_shape,
            num_points_longitude: ni,
            num_points_latitude: nj,
            first_latitude_millidegrees: la1 / 1000,
            first_longitude_millidegrees: lo1 / 1000,
            last_latitude_millidegrees: la2 / 1000,
            last_longitude_millidegrees: lo2 / 1000,
            latitude_increment_millidegrees: di / 1000,
            longitude_increment_millidegrees: dj / 1000,
            scanning_mode,
        })
    } else {
        // Fallback for other templates - just get dimensions
        // Try to extract Ni/Nj from common positions
        let ni = if gd.len() >= 20 {
            u32::from_be_bytes([gd[16], gd[17], gd[18], gd[19]])
        } else {
            0
        };
        let nj = if gd.len() >= 24 {
            u32::from_be_bytes([gd[20], gd[21], gd[22], gd[23]])
        } else {
            0
        };
        
        Ok(GridDefinition {
            grid_shape: gd.first().copied().unwrap_or(0),
            num_points_latitude: nj,
            num_points_longitude: ni,
            first_latitude_millidegrees: 0,
            first_longitude_millidegrees: 0,
            last_latitude_millidegrees: 0,
            last_longitude_millidegrees: 0,
            latitude_increment_millidegrees: 0,
            longitude_increment_millidegrees: 0,
            scanning_mode: 0,
        })
    }
}

/// Parse Section 4 (Product Definition)
pub fn parse_product_definition(data: &[u8], discipline: u8) -> Result<ProductDefinition, Grib2Error> {
    let section_offset = find_section(data, 4)?;
    let section_data = &data[section_offset..];

    if section_data.len() < 34 {
        return Err(Grib2Error::InvalidSection {
            section: 4,
            reason: "Not enough data".to_string(),
        });
    }

    // GRIB2 Section 4 structure:
    // Bytes 0-3: Section length
    // Byte 4: Section number (4)
    // Bytes 5-6: Number of coordinate values
    // Bytes 7-8: Product definition template number
    // Byte 9: Parameter category
    // Byte 10: Parameter number
    // ... (rest depends on template)
    
    let parameter_category = section_data[9];
    let parameter_number = section_data[10];
    
    // For template 0 (analysis/forecast at horizontal level):
    // Byte 18-21: Forecast time (4 bytes)
    // Byte 22: Type of first fixed surface
    // Byte 23: Scale factor of first fixed surface
    // Byte 24-27: Scaled value of first fixed surface (4 bytes)
    let forecast_hour = if section_data.len() >= 22 {
        u32::from_be_bytes([section_data[18], section_data[19], section_data[20], section_data[21]])
    } else {
        0
    };
    
    let level_type = section_data.get(22).copied().unwrap_or(1);
    let scale_factor = section_data.get(23).copied().unwrap_or(0) as i8;
    let scaled_value = if section_data.len() >= 28 {
        u32::from_be_bytes([section_data[24], section_data[25], section_data[26], section_data[27]])
    } else {
        0
    };
    
    // Apply scale factor: actual_value = scaled_value / (10^scale_factor)
    // For heights in meters, scale_factor is typically 0, so level_value = scaled_value
    let level_value = if scale_factor == 0 {
        scaled_value
    } else {
        // For non-zero scale factors, we'd need to compute 10^scale_factor
        // For now, just use the scaled value as-is since most levels use scale_factor=0
        scaled_value
    };

    let parameter_short_name = get_parameter_short_name(discipline, parameter_category, parameter_number);
    let level_description = get_level_description(level_type, level_value);

    Ok(ProductDefinition {
        parameter_category,
        parameter_number,
        parameter_short_name,
        level_type,
        level_value,
        level_description,
        forecast_hour,
    })
}

/// Parse Section 5 (Data Representation)
pub fn parse_data_representation(data: &[u8]) -> Result<DataRepresentation, Grib2Error> {
    let section_offset = find_section(data, 5)?;
    let section_data = &data[section_offset..];

    if section_data.len() < 21 {
        return Err(Grib2Error::InvalidSection {
            section: 5,
            reason: "Not enough data".to_string(),
        });
    }

    // Section 5 structure (GRIB2 spec):
    // Octets 1-4 [0-3]: Section length
    // Octet 5 [4]: Section number (5)
    // Octets 6-9 [5-8]: Number of data points (N)
    // Octets 10-11 [9-10]: Data representation template number
    // Octets 12+ [11+]: Template-specific data
    //
    // For Template 5.0 (simple packing):
    // Octets 12-15 [11-14]: Reference value (R) - IEEE 32-bit float
    // Octets 16-17 [15-16]: Binary scale factor (E) - signed 16-bit
    // Octets 18-19 [17-18]: Decimal scale factor (D) - signed 16-bit
    // Octet 20 [19]: Number of bits per packed value
    // Octet 21 [20]: Type of original field values

    let num_data_points = u32::from_be_bytes([
        section_data[5], section_data[6], section_data[7], section_data[8]
    ]);
    
    let template_number = u16::from_be_bytes([section_data[9], section_data[10]]);
    let packing_method = template_number as u8;

    // Template-specific data starts at offset 11
    let template_data = &section_data[11..];
    
    // Parse Template 5.0 fields
    let reference_value = if template_data.len() >= 4 {
        f32::from_be_bytes([template_data[0], template_data[1], template_data[2], template_data[3]])
    } else {
        0.0
    };
    let binary_scale_factor = if template_data.len() >= 6 {
        i16::from_be_bytes([template_data[4], template_data[5]])
    } else {
        0
    };
    let decimal_scale_factor = if template_data.len() >= 8 {
        i16::from_be_bytes([template_data[6], template_data[7]])
    } else {
        0
    };
    let bits_per_value = template_data.get(8).copied().unwrap_or(0);
    let original_data_type = template_data.get(9).copied().unwrap_or(0);

    Ok(DataRepresentation {
        num_data_points,
        packing_method,
        original_data_type,
        reference_value,
        binary_scale_factor,
        decimal_scale_factor,
        bits_per_value,
    })
}

/// Parse Section 6 (Bitmap)
pub fn parse_bitmap(data: &[u8]) -> Result<Bitmap, Grib2Error> {
    let section_offset = find_section(data, 6)?;
    let section_data = &data[section_offset..];

    if section_data.len() < 6 {
        return Err(Grib2Error::InvalidSection {
            section: 6,
            reason: "Not enough data".to_string(),
        });
    }

    let section_length = u32::from_be_bytes([
        section_data[0],
        section_data[1],
        section_data[2],
        section_data[3],
    ]) as usize;
    let indicator = section_data[5];

    if indicator == 255 {
        return Err(Grib2Error::InvalidSection {
            section: 6,
            reason: "No bitmap present".to_string(),
        });
    }

    let bitmap_data = if section_length > 6 {
        Bytes::copy_from_slice(&section_data[6..section_length])
    } else {
        Bytes::new()
    };

    Ok(Bitmap {
        indicator,
        data: bitmap_data,
    })
}

/// Parse Section 7 (Data)
pub fn parse_data_section(data: &[u8]) -> Result<DataSection, Grib2Error> {
    let section_offset = find_section(data, 7)?;
    let section_data = &data[section_offset..];

    if section_data.len() < 5 {
        return Err(Grib2Error::InvalidSection {
            section: 7,
            reason: "Not enough data".to_string(),
        });
    }

    let section_length = u32::from_be_bytes([
        section_data[0],
        section_data[1],
        section_data[2],
        section_data[3],
    ]) as usize;

    if section_length > section_data.len() {
        return Err(Grib2Error::InvalidSection {
            section: 7,
            reason: "Section length exceeds available data".to_string(),
        });
    }

    let data_bytes = if section_length > 5 {
        Bytes::copy_from_slice(&section_data[5..section_length])
    } else {
        Bytes::new()
    };

    Ok(DataSection { data: data_bytes })
}

// ===== Helper Functions =====

/// Find a section by number within a message
fn find_section(data: &[u8], section_num: u8) -> Result<usize, Grib2Error> {
    let mut offset = 16; // After Section 0

    loop {
        if offset + 5 > data.len() {
            return Err(Grib2Error::InvalidSection {
                section: section_num,
                reason: "Section not found".to_string(),
            });
        }

        let section_length = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;

        if section_length < 5 || offset + section_length > data.len() {
            return Err(Grib2Error::InvalidSection {
                section: section_num,
                reason: "Invalid section length".to_string(),
            });
        }

        let current_section = data[offset + 4];

        if current_section == section_num {
            return Ok(offset);
        }

        offset += section_length;

        if offset >= data.len() || current_section == 8 {
            return Err(Grib2Error::InvalidSection {
                section: section_num,
                reason: "Reached end of message without finding section".to_string(),
            });
        }
    }
}

/// Get parameter short name
fn get_parameter_short_name(discipline: u8, category: u8, number: u8) -> String {
    match (discipline, category, number) {
        // ==========================================================================
        // Discipline 0: Meteorological products
        // ==========================================================================
        
        // Category 0: Temperature
        (0, 0, 0) => "TMP".to_string(),
        (0, 0, 1) => "VTMP".to_string(),
        (0, 0, 2) => "POT".to_string(),
        (0, 0, 6) => "DPT".to_string(),     // Dew point temperature
        
        // Category 1: Moisture
        (0, 1, 0) => "SPFH".to_string(),    // Specific humidity
        (0, 1, 1) => "RH".to_string(),      // Relative humidity
        (0, 1, 3) => "PWAT".to_string(),    // Precipitable water
        (0, 1, 7) => "PRATE".to_string(),   // Precipitation rate
        (0, 1, 8) => "APCP".to_string(),    // Total precipitation (accumulated)
        (0, 1, 9) => "NCPCP".to_string(),   // Large scale precipitation
        (0, 1, 10) => "ACPCP".to_string(),  // Convective precipitation
        
        // Category 2: Momentum (wind)
        (0, 2, 0) => "WDIR".to_string(),    // Wind direction
        (0, 2, 1) => "WIND".to_string(),    // Wind speed
        (0, 2, 2) => "UGRD".to_string(),    // U-component of wind
        (0, 2, 3) => "VGRD".to_string(),    // V-component of wind
        (0, 2, 8) => "VVEL".to_string(),    // Vertical velocity (pressure)
        (0, 2, 10) => "ABSV".to_string(),   // Absolute vorticity
        (0, 2, 22) => "GUST".to_string(),   // Wind gust
        
        // Category 3: Mass (pressure/height)
        (0, 3, 0) => "PRES".to_string(),    // Pressure
        (0, 3, 1) => "PRMSL".to_string(),   // Pressure reduced to MSL
        (0, 3, 5) => "HGT".to_string(),     // Geopotential height
        
        // Category 6: Cloud
        (0, 6, 1) => "TCDC".to_string(),    // Total cloud cover
        (0, 6, 3) => "LCDC".to_string(),    // Low cloud cover
        (0, 6, 4) => "MCDC".to_string(),    // Medium cloud cover
        (0, 6, 5) => "HCDC".to_string(),    // High cloud cover
        (0, 6, 6) => "CWAT".to_string(),    // Cloud water
        
        // Category 7: Thermodynamic Stability
        (0, 7, 6) => "CAPE".to_string(),    // Convective Available Potential Energy
        (0, 7, 7) => "CIN".to_string(),     // Convective Inhibition
        (0, 7, 8) => "HLCY".to_string(),    // Storm-relative helicity
        
        // Category 16: Forecast Radar Imagery
        (0, 16, 195) => "REFD".to_string(),   // Reflectivity
        (0, 16, 196) => "REFC".to_string(),   // Composite reflectivity
        (0, 16, 197) => "RETOP".to_string(),  // Echo top
        (0, 16, 198) => "MAXREF".to_string(), // Max reflectivity
        
        // Category 19: Physical Atmospheric Properties
        (0, 19, 0) => "VIS".to_string(),    // Visibility
        (0, 19, 2) => "TSTM".to_string(),   // Thunderstorm probability
        (0, 19, 11) => "TKE".to_string(),   // Turbulent kinetic energy
        
        // ==========================================================================
        // NCEP Local Use Parameters (discipline 0, local tables)
        // ==========================================================================
        
        // Category 2: Momentum (NCEP extensions)
        (0, 2, 194) => "USTM".to_string(),  // U-component storm motion
        (0, 2, 195) => "VSTM".to_string(),  // V-component storm motion
        
        // Category 7: Stability (NCEP extensions for severe weather)
        (0, 7, 192) => "LFTX".to_string(),  // Surface lifted index
        (0, 7, 193) => "4LFTX".to_string(), // Best (4-layer) lifted index
        
        // Category 16: Radar (NCEP extensions)
        // Note: REFC (196) already covered above
        
        // Category 17: Electrodynamics (Lightning)
        (0, 17, 192) => "LTNG".to_string(), // Lightning (NCEP local)
        
        // ==========================================================================
        // HRRR-specific parameters (discipline 0, local use)
        // ==========================================================================
        
        // Updraft Helicity is often encoded as NCEP local
        // The exact encoding may vary - common ones:
        (0, 7, 199) => "MXUPHL".to_string(), // Max updraft helicity (alternative)
        
        // ==========================================================================
        // Discipline 209: MRMS (local use)
        // ==========================================================================
        
        // Category 0: Reflectivity
        (209, 0, 16) => "REFL".to_string(),  // MergedReflectivityQC
        // Category 1: Precipitation
        (209, 1, 0) => "PRECIP_RATE".to_string(),  // PrecipRate
        (209, 1, 1) => "QPE".to_string(),          // Quantitative Precipitation Estimate
        
        _ => format!("P{}_{}_{}", discipline, category, number),
    }
}

/// Get level description
fn get_level_description(level_type: u8, level_value: u32) -> String {
    match level_type {
        1 => "surface".to_string(),
        2 => "cloud base".to_string(),
        3 => "cloud top".to_string(),
        4 => "0C isotherm".to_string(),
        5 => "adiabatic condensation level".to_string(),
        6 => "max wind".to_string(),
        7 => "tropopause".to_string(),
        8 => "top of atmosphere".to_string(),
        10 => "entire atmosphere".to_string(),  // Alternative code for entire atmos
        100 => format!("{} mb", level_value),  // Isobaric surface (pressure in mb)
        101 => "mean sea level".to_string(),
        102 => format!("{} m above MSL", level_value),
        103 => format!("{} m above ground", level_value),  // Height above ground in meters
        104 => format!("sigma level {}", level_value),
        105 => "hybrid level".to_string(),
        106 => format!("{} m below surface", level_value),  // Depth below land surface
        108 => format!("{} mb above ground", level_value),  // Pressure level above ground
        200 => "entire atmosphere".to_string(),
        204 => "highest tropospheric freezing level".to_string(),
        206 => "grid-scale cloud bottom".to_string(),
        207 => "grid-scale cloud top".to_string(),
        211 => "boundary layer cloud layer".to_string(),
        212 => "low cloud layer".to_string(),
        213 => "low cloud bottom".to_string(),
        214 => "low cloud top".to_string(),
        222 => "middle cloud layer".to_string(),
        223 => "middle cloud bottom".to_string(),
        224 => "middle cloud top".to_string(),
        232 => "high cloud layer".to_string(),
        233 => "high cloud bottom".to_string(),
        234 => "high cloud top".to_string(),
        220 => "planetary boundary layer".to_string(),
        _ => format!("Level type {} value {}", level_type, level_value),
    }
}
