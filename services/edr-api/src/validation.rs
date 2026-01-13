//! Z coordinate validation utilities for EDR API.
//!
//! This module provides shared validation functions for vertical (Z) coordinates
//! across all EDR query handlers.

use crate::config::{CollectionDefinition, LevelValue};

/// Extract unique numeric vertical levels from all parameters in a collection.
/// Returns None if collection has no vertical extent (e.g., surface-only).
pub fn get_collection_vertical_levels(collection_def: &CollectionDefinition) -> Option<Vec<f64>> {
    let mut levels: Vec<f64> = collection_def
        .parameters
        .iter()
        .flat_map(|p| p.levels.iter())
        .filter_map(|l| match l {
            LevelValue::Numeric(n) => Some(*n),
            LevelValue::Named(_) => None,
        })
        .collect();

    if levels.is_empty() {
        return None;
    }

    // Sort and deduplicate
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    levels.dedup();
    Some(levels)
}

/// Validate Z coordinate values against the collection's advertised vertical extent.
/// Per OGC EDR spec: "the Z coordinate shall be within the range of vertical levels
/// advertised in the Collection metadata"
///
/// Returns Ok(()) if valid, or Err with descriptive message if invalid.
pub fn validate_z_against_vertical_extent(
    z_values: &[f64],
    collection_def: &CollectionDefinition,
) -> Result<(), String> {
    // If collection has no vertical extent advertised, skip validation
    // (vertical is optional per OGC EDR spec)
    let Some(available_levels) = get_collection_vertical_levels(collection_def) else {
        return Ok(());
    };

    for z in z_values {
        if !available_levels
            .iter()
            .any(|level| (*level - *z).abs() < f64::EPSILON)
        {
            return Err(format!(
                "Z coordinate {} is outside the collection's vertical extent. Must be one of: {:?}",
                z, available_levels
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LevelFilter, ParameterDefinition, RunMode};

    fn make_collection_with_levels(levels: Vec<LevelValue>) -> CollectionDefinition {
        CollectionDefinition {
            id: "test".to_string(),
            title: "Test".to_string(),
            description: String::new(),
            level_filter: LevelFilter::default(),
            parameters: vec![ParameterDefinition {
                name: "TMP".to_string(),
                levels,
                valid_range: None,
            }],
            run_mode: RunMode::default(),
        }
    }

    #[test]
    fn test_get_vertical_levels_numeric() {
        let collection = make_collection_with_levels(vec![
            LevelValue::Numeric(500.0),
            LevelValue::Numeric(700.0),
            LevelValue::Numeric(850.0),
        ]);
        let levels = get_collection_vertical_levels(&collection).unwrap();
        assert_eq!(levels, vec![500.0, 700.0, 850.0]);
    }

    #[test]
    fn test_get_vertical_levels_empty() {
        let collection = make_collection_with_levels(vec![]);
        assert!(get_collection_vertical_levels(&collection).is_none());
    }

    #[test]
    fn test_get_vertical_levels_named_only() {
        let collection = make_collection_with_levels(vec![
            LevelValue::Named("surface".to_string()),
            LevelValue::Named("tropopause".to_string()),
        ]);
        assert!(get_collection_vertical_levels(&collection).is_none());
    }

    #[test]
    fn test_validate_z_valid() {
        let collection = make_collection_with_levels(vec![
            LevelValue::Numeric(500.0),
            LevelValue::Numeric(700.0),
            LevelValue::Numeric(850.0),
        ]);
        assert!(validate_z_against_vertical_extent(&[500.0, 700.0], &collection).is_ok());
    }

    #[test]
    fn test_validate_z_invalid() {
        let collection = make_collection_with_levels(vec![
            LevelValue::Numeric(500.0),
            LevelValue::Numeric(700.0),
        ]);
        let result = validate_z_against_vertical_extent(&[600.0], &collection);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("600"));
    }

    #[test]
    fn test_validate_z_no_vertical_extent() {
        let collection = make_collection_with_levels(vec![]);
        // Should pass when collection has no vertical extent
        assert!(validate_z_against_vertical_extent(&[500.0], &collection).is_ok());
    }
}
