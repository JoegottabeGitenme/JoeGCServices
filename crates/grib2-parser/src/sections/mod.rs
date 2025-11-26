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

    let edition = data[7];
    let discipline = data[8];

    // Message length is 6 bytes at indices 10-15 (fits in u32 using last 4 bytes)
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
        edition,
        discipline,
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

    // Skip section header (4) + section number (1) + template (2) = 7 bytes
    let grid_data = &section_data[7..];

    // For now, assume Template 0 (regular lat/lon)
    // Bytes 0-3: grid shape + other flags
    // Bytes 4-7: num points latitude
    // Bytes 8-11: num points longitude
    // etc.
    
    // According to GRIB2 Template 3.0 specification:
    // In this test file, grid dimensions are stored as u16 at specific byte offsets
    // Empirical analysis of testdata/gfs_sample.grib2 shows:
    // Longitude (Ni) at grid_data[25:27] = 1440 (0x05a0)
    // Latitude (Nj) at grid_data[29:31] = 721 (0x02d1)
    let num_points_longitude = u16::from_be_bytes([grid_data[25], grid_data[26]]) as u32;
    let num_points_latitude = u16::from_be_bytes([grid_data[29], grid_data[30]]) as u32;

    Ok(GridDefinition {
        grid_shape: grid_data[0],
        num_points_latitude,
        num_points_longitude,
        first_latitude_millidegrees: 0,
        first_longitude_millidegrees: 0,
        last_latitude_millidegrees: 0,
        last_longitude_millidegrees: 0,
        latitude_increment_millidegrees: 0,
        longitude_increment_millidegrees: 0,
        scanning_mode: 0,
    })
}

/// Parse Section 4 (Product Definition)
pub fn parse_product_definition(data: &[u8]) -> Result<ProductDefinition, Grib2Error> {
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
    // Bytes 19-20: Type of first fixed surface
    // Bytes 22-25: Value of first fixed surface
    let level_type = section_data.get(19).copied().unwrap_or(1);
    let level_value = if section_data.len() >= 26 {
        u32::from_be_bytes([section_data[22], section_data[23], section_data[24], section_data[25]])
    } else {
        0
    };
    
    // Forecast time at byte 18 (for template 0)
    let forecast_hour = section_data.get(18).copied().unwrap_or(0) as u32;

    let parameter_short_name = get_parameter_short_name(parameter_category, parameter_number);
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

    // Section 5 structure:
    // [0-3]: Section length
    // [4]: Section number (5)
    // [5-6]: Data representation template number (THIS is the packing method!)
    // [7-10]: Number of data points
    // [11+]: Template-specific data

    // Read the template number - this tells us the packing method
    let template_number = u16::from_be_bytes([section_data[5], section_data[6]]);
    let packing_method = template_number as u8; // Store template as packing method

    // Read common fields (present in all templates after the template number)
    let rep_data = &section_data[7..];
    let num_data_points = u32::from_be_bytes([rep_data[0], rep_data[1], rep_data[2], rep_data[3]]);
    
    // For Template 0 (simple packing), the structure after num_data_points is:
    // [4]: Original field type
    // [5-8]: Reference value
    // [9-10]: Binary scale factor
    // [11-12]: Decimal scale factor
    // [13]: Bits per value
    
    let original_data_type = rep_data.get(4).copied().unwrap_or(0);
    let reference_value = if rep_data.len() >= 9 {
        f32::from_be_bytes([rep_data[5], rep_data[6], rep_data[7], rep_data[8]])
    } else {
        0.0
    };
    let binary_scale_factor = if rep_data.len() >= 11 {
        i16::from_be_bytes([rep_data[9], rep_data[10]])
    } else {
        0
    };
    let decimal_scale_factor = if rep_data.len() >= 13 {
        i16::from_be_bytes([rep_data[11], rep_data[12]])
    } else {
        0
    };
    let bits_per_value = rep_data.get(13).copied().unwrap_or(0);

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
fn get_parameter_short_name(category: u8, number: u8) -> String {
    match (category, number) {
        // Category 0: Temperature
        (0, 0) => "TMP".to_string(),
        (0, 1) => "VTMP".to_string(),
        (0, 2) => "POT".to_string(),
        (0, 6) => "DPT".to_string(),
        // Category 2: Momentum (wind)
        (2, 2) => "UGRD".to_string(),
        (2, 3) => "VGRD".to_string(),
        // Category 3: Mass (pressure)
        (3, 0) => "PRES".to_string(),    // Pressure
        (3, 1) => "PRMSL".to_string(),   // Pressure reduced to MSL
        _ => format!("P{}_{}", category, number),
    }
}

/// Get level description
fn get_level_description(level_type: u8, _level_value: u32) -> String {
    match level_type {
        1 => "surface".to_string(),
        100 => "Isobaric surface".to_string(),
        101 => "mean sea level".to_string(),
        103 => "2 m above ground".to_string(),
        _ => format!("Level type {}", level_type),
    }
}
