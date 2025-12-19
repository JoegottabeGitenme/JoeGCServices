//! Ingestion configuration for parameter filtering.
//!
//! Defines which GRIB2 parameters and levels to extract during ingestion.

use std::collections::HashSet;

/// GRIB2 level type codes.
pub mod level_types {
    /// Surface level
    pub const SURFACE: u8 = 1;
    /// Isobaric (pressure) level
    pub const ISOBARIC: u8 = 100;
    /// Mean sea level
    pub const MSL: u8 = 101;
    /// Height above ground
    pub const HEIGHT_ABOVE_GROUND: u8 = 103;
    /// Entire atmosphere
    pub const ENTIRE_ATMOSPHERE: u8 = 200;
}

/// Specification for a parameter to extract.
#[derive(Debug, Clone)]
pub struct ParameterSpec {
    /// Parameter short name (e.g., "TMP", "UGRD")
    pub name: &'static str,
    /// Accepted level specifications: (level_type, optional_level_value)
    /// If level_value is None, all values of that level type are accepted
    /// For isobaric levels (100), check against standard_pressure_levels()
    pub levels: Vec<(u8, Option<u32>)>,
}

/// Get the list of target GRIB2 parameters to extract.
///
/// This defines which meteorological parameters are ingested
/// and at which vertical levels.
pub fn target_grib2_parameters() -> Vec<ParameterSpec> {
    use level_types::*;
    
    vec![
        // Pressure
        ParameterSpec {
            name: "PRMSL",
            levels: vec![(MSL, None)], // Mean sea level pressure
        },
        
        // Temperature
        ParameterSpec {
            name: "TMP",
            levels: vec![
                (HEIGHT_ABOVE_GROUND, Some(2)), // 2m temperature
                (ISOBARIC, None),               // All pressure levels
            ],
        },
        ParameterSpec {
            name: "DPT",
            levels: vec![(HEIGHT_ABOVE_GROUND, Some(2))], // 2m dew point
        },
        
        // Wind
        ParameterSpec {
            name: "UGRD",
            levels: vec![
                (HEIGHT_ABOVE_GROUND, Some(10)), // 10m wind
                (ISOBARIC, None),                // All pressure levels
            ],
        },
        ParameterSpec {
            name: "VGRD",
            levels: vec![
                (HEIGHT_ABOVE_GROUND, Some(10)), // 10m wind
                (ISOBARIC, None),                // All pressure levels
            ],
        },
        ParameterSpec {
            name: "GUST",
            levels: vec![(SURFACE, None)], // Surface wind gust
        },
        
        // Moisture
        ParameterSpec {
            name: "RH",
            levels: vec![
                (HEIGHT_ABOVE_GROUND, Some(2)), // 2m relative humidity
                (ISOBARIC, None),               // All pressure levels
            ],
        },
        ParameterSpec {
            name: "PWAT",
            levels: vec![(ENTIRE_ATMOSPHERE, None)], // Precipitable water
        },
        
        // Geopotential
        ParameterSpec {
            name: "HGT",
            levels: vec![(ISOBARIC, None)], // Geopotential height
        },
        
        // Precipitation
        ParameterSpec {
            name: "APCP",
            levels: vec![(SURFACE, None)], // Total precipitation
        },
        
        // Convective/Stability
        ParameterSpec {
            name: "CAPE",
            levels: vec![(SURFACE, None)], // Surface-based CAPE
        },
        ParameterSpec {
            name: "CIN",
            levels: vec![(SURFACE, None)], // Surface-based CIN
        },
        
        // Cloud cover
        ParameterSpec {
            name: "TCDC",
            levels: vec![(ENTIRE_ATMOSPHERE, None)], // Total cloud cover
        },
        
        // Visibility
        ParameterSpec {
            name: "VIS",
            levels: vec![(SURFACE, None)], // Surface visibility
        },
        
        // Radar reflectivity (for models that include it)
        ParameterSpec {
            name: "REFC",
            levels: vec![(ENTIRE_ATMOSPHERE, None)], // Composite reflectivity
        },
        ParameterSpec {
            name: "REFD",
            levels: vec![(HEIGHT_ABOVE_GROUND, Some(1000))], // 1km AGL reflectivity
        },
    ]
}

/// Get the set of standard pressure levels to ingest (in hPa/mb).
///
/// These are the commonly-used meteorological pressure levels.
pub fn standard_pressure_levels() -> HashSet<u32> {
    [
        1000, 975, 950, 925, 900, 850, 800, 750, 700, 650,
        600, 550, 500, 450, 400, 350, 300, 250, 200, 150,
        100, 70, 50, 30, 20, 10
    ].into_iter().collect()
}

/// Check if a parameter/level combination should be ingested.
///
/// # Arguments
/// * `param` - Parameter short name
/// * `level_type` - GRIB2 level type code
/// * `level_value` - Level value (e.g., pressure in hPa, height in m)
/// * `target_params` - List of target parameters to ingest
/// * `pressure_levels` - Set of valid pressure levels
///
/// # Returns
/// `true` if this parameter/level should be ingested
pub fn should_ingest_parameter(
    param: &str,
    level_type: u8,
    level_value: u32,
    target_params: &[ParameterSpec],
    pressure_levels: &HashSet<u32>,
) -> bool {
    target_params.iter().any(|spec| {
        if param != spec.name {
            return false;
        }
        
        spec.levels.iter().any(|(lt, lv)| {
            if level_type != *lt {
                return false;
            }
            
            // For isobaric levels, check against pressure levels set
            if level_type == level_types::ISOBARIC {
                return pressure_levels.contains(&level_value);
            }
            
            // For other levels, check specific value if required
            if let Some(required_value) = lv {
                level_value == *required_value
            } else {
                true
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_params() {
        let params = target_grib2_parameters();
        assert!(params.iter().any(|p| p.name == "TMP"));
        assert!(params.iter().any(|p| p.name == "UGRD"));
        assert!(params.iter().any(|p| p.name == "CAPE"));
    }

    #[test]
    fn test_pressure_levels() {
        let levels = standard_pressure_levels();
        assert!(levels.contains(&500));
        assert!(levels.contains(&850));
        assert!(!levels.contains(&999));
    }

    #[test]
    fn test_should_ingest() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();
        
        // 2m temperature should be ingested
        assert!(should_ingest_parameter("TMP", 103, 2, &params, &levels));
        
        // 500mb temperature should be ingested
        assert!(should_ingest_parameter("TMP", 100, 500, &params, &levels));
        
        // 999mb temperature should NOT be ingested (not a standard level)
        assert!(!should_ingest_parameter("TMP", 100, 999, &params, &levels));
        
        // Unknown parameter should NOT be ingested
        assert!(!should_ingest_parameter("UNKNOWN", 1, 0, &params, &levels));
    }
}
