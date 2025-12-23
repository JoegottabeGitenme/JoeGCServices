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
        1000, 975, 950, 925, 900, 850, 800, 750, 700, 650, 600, 550, 500, 450, 400, 350, 300, 250,
        200, 150, 100, 70, 50, 30, 20, 10,
    ]
    .into_iter()
    .collect()
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
    fn test_target_params_contains_expected() {
        let params = target_grib2_parameters();

        // Check essential parameters exist
        assert!(
            params.iter().any(|p| p.name == "TMP"),
            "Should have temperature"
        );
        assert!(
            params.iter().any(|p| p.name == "UGRD"),
            "Should have U-wind"
        );
        assert!(
            params.iter().any(|p| p.name == "VGRD"),
            "Should have V-wind"
        );
        assert!(params.iter().any(|p| p.name == "CAPE"), "Should have CAPE");
        assert!(
            params.iter().any(|p| p.name == "PRMSL"),
            "Should have pressure"
        );
        assert!(
            params.iter().any(|p| p.name == "RH"),
            "Should have relative humidity"
        );
        assert!(
            params.iter().any(|p| p.name == "HGT"),
            "Should have geopotential height"
        );
    }

    #[test]
    fn test_target_params_have_valid_levels() {
        let params = target_grib2_parameters();

        for param in &params {
            assert!(
                !param.levels.is_empty(),
                "Parameter {} should have at least one level",
                param.name
            );

            for (level_type, _) in &param.levels {
                // Level types should be valid GRIB2 level codes
                assert!(
                    *level_type == level_types::SURFACE
                        || *level_type == level_types::ISOBARIC
                        || *level_type == level_types::MSL
                        || *level_type == level_types::HEIGHT_ABOVE_GROUND
                        || *level_type == level_types::ENTIRE_ATMOSPHERE,
                    "Parameter {} has invalid level type: {}",
                    param.name,
                    level_type
                );
            }
        }
    }

    #[test]
    fn test_pressure_levels_contains_standard() {
        let levels = standard_pressure_levels();

        // Standard meteorological levels
        assert!(levels.contains(&1000), "Should have 1000 hPa");
        assert!(levels.contains(&850), "Should have 850 hPa");
        assert!(levels.contains(&700), "Should have 700 hPa");
        assert!(levels.contains(&500), "Should have 500 hPa");
        assert!(levels.contains(&300), "Should have 300 hPa");
        assert!(
            levels.contains(&250),
            "Should have 250 hPa (jet stream level)"
        );
        assert!(levels.contains(&200), "Should have 200 hPa");

        // Non-standard levels should not be present
        assert!(!levels.contains(&999));
        assert!(!levels.contains(&123));
        assert!(!levels.contains(&0));
    }

    #[test]
    fn test_pressure_levels_count() {
        let levels = standard_pressure_levels();
        // Should have a reasonable number of levels (not too few, not too many)
        assert!(
            levels.len() >= 20,
            "Should have at least 20 pressure levels"
        );
        assert!(levels.len() <= 50, "Should have at most 50 pressure levels");
    }

    #[test]
    fn test_should_ingest_2m_temperature() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();

        // 2m temperature should be ingested
        assert!(should_ingest_parameter(
            "TMP",
            level_types::HEIGHT_ABOVE_GROUND,
            2,
            &params,
            &levels
        ));

        // 10m temperature should NOT be ingested (we only want 2m)
        assert!(!should_ingest_parameter(
            "TMP",
            level_types::HEIGHT_ABOVE_GROUND,
            10,
            &params,
            &levels
        ));
    }

    #[test]
    fn test_should_ingest_10m_wind() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();

        // 10m wind should be ingested
        assert!(should_ingest_parameter(
            "UGRD",
            level_types::HEIGHT_ABOVE_GROUND,
            10,
            &params,
            &levels
        ));
        assert!(should_ingest_parameter(
            "VGRD",
            level_types::HEIGHT_ABOVE_GROUND,
            10,
            &params,
            &levels
        ));

        // 2m wind should NOT be ingested (we only want 10m)
        assert!(!should_ingest_parameter(
            "UGRD",
            level_types::HEIGHT_ABOVE_GROUND,
            2,
            &params,
            &levels
        ));
    }

    #[test]
    fn test_should_ingest_isobaric_levels() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();

        // 500mb temperature should be ingested
        assert!(should_ingest_parameter(
            "TMP",
            level_types::ISOBARIC,
            500,
            &params,
            &levels
        ));

        // 850mb height should be ingested
        assert!(should_ingest_parameter(
            "HGT",
            level_types::ISOBARIC,
            850,
            &params,
            &levels
        ));

        // 999mb (non-standard) should NOT be ingested
        assert!(!should_ingest_parameter(
            "TMP",
            level_types::ISOBARIC,
            999,
            &params,
            &levels
        ));
    }

    #[test]
    fn test_should_ingest_surface_params() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();

        // Surface CAPE should be ingested
        assert!(should_ingest_parameter(
            "CAPE",
            level_types::SURFACE,
            0,
            &params,
            &levels
        ));

        // Surface CIN should be ingested
        assert!(should_ingest_parameter(
            "CIN",
            level_types::SURFACE,
            0,
            &params,
            &levels
        ));

        // Surface visibility should be ingested
        assert!(should_ingest_parameter(
            "VIS",
            level_types::SURFACE,
            0,
            &params,
            &levels
        ));
    }

    #[test]
    fn test_should_not_ingest_unknown_parameter() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();

        assert!(!should_ingest_parameter(
            "UNKNOWN",
            level_types::SURFACE,
            0,
            &params,
            &levels
        ));
        assert!(!should_ingest_parameter(
            "FOOBAR",
            level_types::ISOBARIC,
            500,
            &params,
            &levels
        ));
        assert!(!should_ingest_parameter(
            "",
            level_types::MSL,
            0,
            &params,
            &levels
        ));
    }

    #[test]
    fn test_should_ingest_msl_pressure() {
        let params = target_grib2_parameters();
        let levels = standard_pressure_levels();

        // MSLP should be ingested
        assert!(should_ingest_parameter(
            "PRMSL",
            level_types::MSL,
            0,
            &params,
            &levels
        ));
    }

    #[test]
    fn test_level_type_constants() {
        // Verify level type constants match GRIB2 spec
        assert_eq!(level_types::SURFACE, 1);
        assert_eq!(level_types::ISOBARIC, 100);
        assert_eq!(level_types::MSL, 101);
        assert_eq!(level_types::HEIGHT_ABOVE_GROUND, 103);
        assert_eq!(level_types::ENTIRE_ATMOSPHERE, 200);
    }
}
