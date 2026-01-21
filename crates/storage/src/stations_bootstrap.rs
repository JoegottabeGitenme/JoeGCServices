//! Station database bootstrap functionality.
//!
//! This module provides functionality to bootstrap the locations database
//! with known airport/station data. The data comes from NOAA/FAA sources
//! and is embedded in the binary for initial population.
//!
//! ## Data Sources
//!
//! - US airports: FAA NASR data, filtered to IFR/VFR airports with ICAO codes
//! - Additional stations can be loaded from external files
//!
//! ## Bootstrap Process
//!
//! 1. Check if locations table is empty or has fewer than threshold entries
//! 2. If so, load embedded station data and insert into database
//! 3. Mark all bootstrap locations as type "airport" with country "US"

use crate::observations::{Location, ObservationCatalog};
use tracing::{debug, info, warn};
use wms_common::WmsResult;

/// Embedded station data for US airports.
///
/// Format: ICAO_ID,NAME,LATITUDE,LONGITUDE,ELEVATION_M
/// This is a subset of ~5000 US airports with ICAO codes.
///
/// Data derived from FAA NASR and filtered to airports likely to report METARs.
const EMBEDDED_STATIONS_CSV: &str = include_str!("../data/us_airports.csv");

/// Bootstrap the locations database with known airport data.
///
/// This will only populate the database if it has fewer than `min_threshold`
/// locations. Set `min_threshold` to 0 to always skip, or a high number to
/// always populate.
///
/// Returns the number of locations inserted.
pub async fn bootstrap_locations(
    catalog: &ObservationCatalog,
    min_threshold: i64,
) -> WmsResult<usize> {
    // Check current location count
    let current_count = catalog.count_locations().await?;

    if current_count >= min_threshold {
        debug!(
            current = current_count,
            threshold = min_threshold,
            "Skipping station bootstrap - already have enough locations"
        );
        return Ok(0);
    }

    info!(
        current = current_count,
        threshold = min_threshold,
        "Bootstrapping locations database with airport data"
    );

    // Parse embedded CSV data
    let locations = parse_stations_csv(EMBEDDED_STATIONS_CSV)?;

    info!(count = locations.len(), "Parsed airport station data");

    // Insert locations in batches
    let mut inserted = 0;
    for chunk in locations.chunks(100) {
        inserted += catalog.upsert_locations(chunk).await?;
    }

    info!(inserted = inserted, "Bootstrap complete");
    Ok(inserted)
}

/// Parse station data from CSV format.
///
/// Expected format: ICAO_ID,NAME,LONGITUDE,LATITUDE,ELEVATION_M,STATE
/// Note: Longitude comes before Latitude (matches GeoJSON convention)
/// Lines starting with # are comments.
fn parse_stations_csv(csv_data: &str) -> WmsResult<Vec<Location>> {
    let mut locations = Vec::new();

    for (line_num, line) in csv_data.lines().enumerate() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 {
            warn!(
                line = line_num + 1,
                content = line,
                "Skipping malformed station line"
            );
            continue;
        }

        let icao = parts[0].trim();
        let name = parts[1].trim();
        // CSV format is LON,LAT (GeoJSON convention)
        let lon: f64 = match parts[2].trim().parse() {
            Ok(v) => v,
            Err(_) => {
                warn!(
                    line = line_num + 1,
                    icao = icao,
                    "Invalid longitude, skipping"
                );
                continue;
            }
        };
        let lat: f64 = match parts[3].trim().parse() {
            Ok(v) => v,
            Err(_) => {
                warn!(
                    line = line_num + 1,
                    icao = icao,
                    "Invalid latitude, skipping"
                );
                continue;
            }
        };

        let elevation_m: Option<f32> = if parts.len() > 4 {
            parts[4].trim().parse().ok()
        } else {
            None
        };

        // Get state/region if provided (column 5)
        let region: Option<String> = if parts.len() > 5 {
            let r = parts[5].trim();
            if !r.is_empty() {
                Some(r.to_string())
            } else {
                None
            }
        } else {
            None
        };

        let mut location = Location::new(icao, name, lon, lat)
            .with_type("airport")
            .with_country("US");

        if let Some(elev) = elevation_m {
            location = location.with_elevation(elev);
        }

        if let Some(reg) = region {
            location = location.with_region(reg);
        }

        locations.push(location);
    }

    Ok(locations)
}

/// Load additional stations from a file path.
///
/// This can be used to supplement the embedded data with additional
/// stations from external sources.
pub async fn load_stations_from_file(
    catalog: &ObservationCatalog,
    file_path: &str,
) -> WmsResult<usize> {
    let csv_data = tokio::fs::read_to_string(file_path).await.map_err(|e| {
        wms_common::WmsError::StorageError(format!("Failed to read stations file: {}", e))
    })?;

    let locations = parse_stations_csv(&csv_data)?;

    info!(
        file = file_path,
        count = locations.len(),
        "Loading additional stations from file"
    );

    let mut inserted = 0;
    for chunk in locations.chunks(100) {
        inserted += catalog.upsert_locations(chunk).await?;
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_stations_csv() {
        let csv = r#"
# Test airport data
KJFK,John F Kennedy Intl,-73.7781,40.6413,4,NY
KLAX,Los Angeles Intl,-118.4085,33.9416,38,CA
KORD,Chicago O Hare Intl,-87.9073,41.9742,201,IL
"#;

        let locations = parse_stations_csv(csv).unwrap();
        assert_eq!(locations.len(), 3);

        let jfk = &locations[0];
        assert_eq!(jfk.id, "KJFK");
        assert_eq!(jfk.name, "John F Kennedy Intl");
        // Note: CSV has lon,lat order but Location stores lon,lat
        assert!((jfk.lon - (-73.7781)).abs() < 0.001);
        assert!((jfk.lat - 40.6413).abs() < 0.001);
        assert_eq!(jfk.elevation_m, Some(4.0));
        assert_eq!(jfk.region, Some("NY".to_string()));
    }

    #[test]
    fn test_parse_stations_csv_minimal() {
        let csv = "KDEN,Denver Intl,-104.6737,39.8561";
        let locations = parse_stations_csv(csv).unwrap();
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].id, "KDEN");
        assert!(locations[0].elevation_m.is_none());
    }

    #[test]
    fn test_parse_stations_csv_with_comments() {
        let csv = r#"
# Header comment
KABC,Test Airport,-100.0,40.0,100

# Another comment
KXYZ,Another Airport,-90.0,35.0,50
"#;

        let locations = parse_stations_csv(csv).unwrap();
        assert_eq!(locations.len(), 2);
    }
}
