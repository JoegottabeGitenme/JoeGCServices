//! Common test fixtures for weather-wms tests.
//!
//! This module provides pre-defined test data that represents common
//! scenarios in weather data processing.

/// Common bounding box definitions for testing.
pub mod bbox {
    /// Global bounding box (-180 to 180, -90 to 90)
    pub const GLOBAL: (f64, f64, f64, f64) = (-180.0, -90.0, 180.0, 90.0);

    /// Continental United States bounding box
    pub const CONUS: (f64, f64, f64, f64) = (-130.0, 20.0, -60.0, 55.0);

    /// Europe bounding box
    pub const EUROPE: (f64, f64, f64, f64) = (-15.0, 35.0, 45.0, 72.0);

    /// A small test tile (typical web map tile extent at ~zoom 8)
    pub const SMALL_TILE: (f64, f64, f64, f64) = (-100.0, 40.0, -99.0, 41.0);

    /// Single point (degenerate bbox)
    pub const POINT: (f64, f64, f64, f64) = (0.0, 0.0, 0.0, 0.0);

    /// Crosses antimeridian (Pacific-centric)
    pub const PACIFIC: (f64, f64, f64, f64) = (160.0, -50.0, -140.0, 50.0);

    /// Invalid bbox (min > max)
    pub const INVALID: (f64, f64, f64, f64) = (10.0, 10.0, 5.0, 5.0);
}

/// Common grid specifications for testing.
pub mod grid {
    /// GFS global grid (0.25 degree resolution)
    pub const GFS_GLOBAL: GridSpec = GridSpec {
        width: 1440,
        height: 721,
        min_lon: 0.0,
        max_lon: 359.75,
        min_lat: -90.0,
        max_lat: 90.0,
    };

    /// HRRR CONUS grid (3km resolution, ~1800x1060)
    pub const HRRR_CONUS: GridSpec = GridSpec {
        width: 1799,
        height: 1059,
        min_lon: -134.09,
        max_lon: -60.92,
        min_lat: 21.14,
        max_lat: 52.62,
    };

    /// MRMS CONUS grid (1km resolution)
    pub const MRMS_CONUS: GridSpec = GridSpec {
        width: 7000,
        height: 3500,
        min_lon: -130.0,
        max_lon: -60.0,
        min_lat: 20.0,
        max_lat: 55.0,
    };

    /// Simple 10x10 test grid
    pub const SIMPLE_10X10: GridSpec = GridSpec {
        width: 10,
        height: 10,
        min_lon: -10.0,
        max_lon: 10.0,
        min_lat: -10.0,
        max_lat: 10.0,
    };

    /// Standard tile size (256x256)
    pub const TILE_256: GridSpec = GridSpec {
        width: 256,
        height: 256,
        min_lon: -180.0,
        max_lon: 180.0,
        min_lat: -90.0,
        max_lat: 90.0,
    };

    /// Grid specification for testing.
    #[derive(Debug, Clone, Copy)]
    pub struct GridSpec {
        pub width: usize,
        pub height: usize,
        pub min_lon: f64,
        pub max_lon: f64,
        pub min_lat: f64,
        pub max_lat: f64,
    }

    impl GridSpec {
        /// Returns the total number of grid cells.
        pub fn size(&self) -> usize {
            self.width * self.height
        }

        /// Returns the resolution in degrees.
        pub fn resolution(&self) -> (f64, f64) {
            let dx = (self.max_lon - self.min_lon) / self.width as f64;
            let dy = (self.max_lat - self.min_lat) / self.height as f64;
            (dx, dy)
        }

        /// Returns the bounding box as (min_lon, min_lat, max_lon, max_lat).
        pub fn bbox(&self) -> (f64, f64, f64, f64) {
            (self.min_lon, self.min_lat, self.max_lon, self.max_lat)
        }
    }
}

/// Common time values for testing.
pub mod time {
    /// A fixed reference time for tests (2024-01-15T12:00:00Z)
    pub const REFERENCE_TIME: &str = "2024-01-15T12:00:00Z";

    /// GFS model run times
    pub const GFS_CYCLES: [&str; 4] = ["00", "06", "12", "18"];

    /// HRRR model run times (hourly)
    pub const HRRR_CYCLES: [&str; 24] = [
        "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14",
        "15", "16", "17", "18", "19", "20", "21", "22", "23",
    ];

    /// Common forecast hours
    pub const FORECAST_HOURS: [u32; 8] = [0, 1, 3, 6, 12, 24, 48, 120];
}

/// Common layer identifiers for testing.
pub mod layers {
    /// Temperature at 2m above ground
    pub const TMP_2M: &str = "gfs_TMP_2m";

    /// Wind speed at 10m above ground
    pub const WIND_10M: &str = "gfs_WIND_10m";

    /// Mean sea level pressure
    pub const PRMSL: &str = "gfs_PRMSL";

    /// Total cloud cover
    pub const TCDC: &str = "gfs_TCDC";

    /// Composite reflectivity
    pub const REFC: &str = "mrms_REFC";

    /// GOES visible imagery
    pub const GOES_VIS: &str = "goes18_C02";

    /// GOES infrared imagery
    pub const GOES_IR: &str = "goes18_C13";
}

/// Common style names for testing.
pub mod styles {
    pub const TEMPERATURE: &str = "temperature";
    pub const WIND: &str = "wind";
    pub const PRECIPITATION: &str = "precipitation";
    pub const PRESSURE: &str = "pressure";
    pub const REFLECTIVITY: &str = "reflectivity";
    pub const CLOUD: &str = "cloud";
}

/// Common CRS identifiers.
pub mod crs {
    /// WGS84 geographic
    pub const EPSG_4326: &str = "EPSG:4326";

    /// Web Mercator
    pub const EPSG_3857: &str = "EPSG:3857";

    /// WMS 1.1.1 style (lon/lat order)
    pub const CRS_84: &str = "CRS:84";
}

/// Common HTTP request parameters for EDR testing.
pub mod edr {
    /// Default EDR base URL for tests.
    pub const BASE_URL: &str = "http://localhost:8083/edr";

    /// Common coordinate test cases.
    pub mod coords {
        /// Oklahoma City (land, CONUS interior)
        pub const OKC: (f64, f64) = (-97.5, 35.2);

        /// Denver (high altitude)
        pub const DENVER: (f64, f64) = (-104.99, 39.74);

        /// San Francisco (coastal)
        pub const SAN_FRANCISCO: (f64, f64) = (-122.42, 37.77);

        /// Washington DC (east coast)
        pub const WASHINGTON_DC: (f64, f64) = (-77.04, 38.90);

        /// Chicago (midwest)
        pub const CHICAGO: (f64, f64) = (-87.63, 41.88);

        /// Ocean point (Gulf of Mexico)
        pub const GULF_OCEAN: (f64, f64) = (-92.0, 26.0);

        /// Origin point
        pub const ORIGIN: (f64, f64) = (0.0, 0.0);

        /// Max bounds (edge case)
        pub const MAX_BOUNDS: (f64, f64) = (180.0, 90.0);

        /// Min bounds (edge case)
        pub const MIN_BOUNDS: (f64, f64) = (-180.0, -90.0);
    }

    /// Common WKT coordinate strings.
    pub mod wkt {
        /// Oklahoma City as WKT POINT
        pub const OKC: &str = "POINT(-97.5 35.2)";

        /// Denver as WKT POINT
        pub const DENVER: &str = "POINT(-104.99 39.74)";

        /// San Francisco as WKT POINT
        pub const SAN_FRANCISCO: &str = "POINT(-122.42 37.77)";

        /// With space after POINT
        pub const WITH_SPACE: &str = "POINT (-97.5 35.2)";

        /// Lowercase point
        pub const LOWERCASE: &str = "point(-97.5 35.2)";
    }

    /// Common pressure levels (hPa/mb).
    pub mod levels {
        /// Standard isobaric levels for upper-air analysis
        pub const STANDARD_ISOBARIC: [f64; 7] = [1000.0, 925.0, 850.0, 700.0, 500.0, 300.0, 250.0];

        /// Common surface levels
        pub const SURFACE: &str = "surface";

        /// Common height above ground levels (meters)
        pub const HEIGHT_2M: &str = "2";
        pub const HEIGHT_10M: &str = "10";
    }

    /// Common parameter names.
    pub mod params {
        /// Temperature
        pub const TMP: &str = "TMP";

        /// U-component of wind
        pub const UGRD: &str = "UGRD";

        /// V-component of wind
        pub const VGRD: &str = "VGRD";

        /// Relative humidity
        pub const RH: &str = "RH";

        /// Geopotential height
        pub const HGT: &str = "HGT";

        /// Mean sea level pressure
        pub const PRMSL: &str = "PRMSL";

        /// Total precipitation
        pub const APCP: &str = "APCP";

        /// Composite reflectivity
        pub const REFC: &str = "REFC";

        /// CAPE
        pub const CAPE: &str = "CAPE";

        /// All standard surface parameters
        pub const SURFACE_PARAMS: [&str; 4] = ["TMP", "UGRD", "VGRD", "PRMSL"];

        /// All standard isobaric parameters
        pub const ISOBARIC_PARAMS: [&str; 5] = ["TMP", "UGRD", "VGRD", "RH", "HGT"];
    }

    /// Common datetime test strings.
    pub mod datetime {
        /// Fixed reference datetime for tests
        pub const REFERENCE: &str = "2024-12-29T12:00:00Z";

        /// Interval spanning a day
        pub const DAY_INTERVAL: &str = "2024-12-29T00:00:00Z/2024-12-29T23:59:59Z";

        /// Open-ended start interval
        pub const OPEN_START: &str = "../2024-12-29T23:59:59Z";

        /// Open-ended end interval
        pub const OPEN_END: &str = "2024-12-29T00:00:00Z/..";

        /// Multiple hours for testing
        pub const HOURS: [&str; 4] = [
            "2024-12-29T00:00:00Z",
            "2024-12-29T06:00:00Z",
            "2024-12-29T12:00:00Z",
            "2024-12-29T18:00:00Z",
        ];
    }

    /// Common collection IDs.
    pub mod collections {
        pub const HRRR_SURFACE: &str = "hrrr-surface";
        pub const HRRR_ISOBARIC: &str = "hrrr-isobaric";
        pub const HRRR_ATMOSPHERE: &str = "hrrr-atmosphere";
        pub const GFS_SURFACE: &str = "gfs-surface";
        pub const GFS_ISOBARIC: &str = "gfs-isobaric";
    }

    /// Default position query parameters.
    pub struct PositionQueryParams {
        pub coords: &'static str,
        pub z: Option<&'static str>,
        pub datetime: Option<&'static str>,
        pub parameter_name: Option<&'static str>,
        pub crs: Option<&'static str>,
        pub f: Option<&'static str>,
    }

    impl Default for PositionQueryParams {
        fn default() -> Self {
            Self {
                coords: wkt::OKC,
                z: None,
                datetime: None,
                parameter_name: Some(params::TMP),
                crs: None,
                f: None,
            }
        }
    }

    impl PositionQueryParams {
        /// Convert to query string for URL.
        pub fn to_query_string(&self) -> String {
            let mut parts = vec![format!("coords={}", urlencoding(self.coords))];

            if let Some(z) = self.z {
                parts.push(format!("z={}", z));
            }
            if let Some(dt) = self.datetime {
                parts.push(format!("datetime={}", urlencoding(dt)));
            }
            if let Some(param) = self.parameter_name {
                parts.push(format!("parameter-name={}", param));
            }
            if let Some(crs) = self.crs {
                parts.push(format!("crs={}", crs));
            }
            if let Some(f) = self.f {
                parts.push(format!("f={}", f));
            }

            parts.join("&")
        }

        /// Build a full URL for a position query.
        pub fn to_url(&self, collection_id: &str) -> String {
            format!(
                "{}/collections/{}/position?{}",
                BASE_URL,
                collection_id,
                self.to_query_string()
            )
        }
    }

    /// Simple URL encoding for test strings (handles common chars).
    fn urlencoding(s: &str) -> String {
        s.replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
            .replace('/', "%2F")
            .replace(':', "%3A")
    }
}

/// Common HTTP request parameters for WMS testing.
pub mod wms {
    /// Default WMS GetMap parameters.
    pub struct GetMapParams {
        pub service: &'static str,
        pub version: &'static str,
        pub request: &'static str,
        pub layers: &'static str,
        pub styles: &'static str,
        pub crs: &'static str,
        pub bbox: &'static str,
        pub width: u32,
        pub height: u32,
        pub format: &'static str,
    }

    /// Default GetMap parameters for testing.
    pub const DEFAULT_GETMAP: GetMapParams = GetMapParams {
        service: "WMS",
        version: "1.3.0",
        request: "GetMap",
        layers: "gfs_TMP_2m",
        styles: "temperature",
        crs: "EPSG:4326",
        bbox: "-130,20,-60,55",
        width: 256,
        height: 256,
        format: "image/png",
    };

    impl GetMapParams {
        /// Converts parameters to a query string.
        pub fn to_query_string(&self) -> String {
            format!(
                "SERVICE={}&VERSION={}&REQUEST={}&LAYERS={}&STYLES={}&CRS={}&BBOX={}&WIDTH={}&HEIGHT={}&FORMAT={}",
                self.service,
                self.version,
                self.request,
                self.layers,
                self.styles,
                self.crs,
                self.bbox,
                self.width,
                self.height,
                self.format
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_spec_size() {
        assert_eq!(grid::GFS_GLOBAL.size(), 1440 * 721);
        assert_eq!(grid::SIMPLE_10X10.size(), 100);
    }

    #[test]
    fn test_grid_spec_resolution() {
        let (dx, dy) = grid::GFS_GLOBAL.resolution();
        assert!((dx - 0.25).abs() < 0.01);
        assert!((dy - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_getmap_query_string() {
        let query = wms::DEFAULT_GETMAP.to_query_string();
        assert!(query.contains("SERVICE=WMS"));
        assert!(query.contains("VERSION=1.3.0"));
        assert!(query.contains("LAYERS=gfs_TMP_2m"));
    }

    #[test]
    fn test_edr_position_query_default() {
        let params = edr::PositionQueryParams::default();
        let query = params.to_query_string();
        assert!(query.contains("coords="));
        assert!(query.contains("parameter-name=TMP"));
    }

    #[test]
    fn test_edr_position_query_full() {
        let params = edr::PositionQueryParams {
            coords: edr::wkt::DENVER,
            z: Some("850"),
            datetime: Some(edr::datetime::REFERENCE),
            parameter_name: Some("TMP,UGRD,VGRD"),
            crs: Some("CRS:84"),
            f: Some("application/vnd.cov+json"),
        };
        let query = params.to_query_string();
        assert!(query.contains("z=850"));
        assert!(query.contains("datetime="));
    }

    #[test]
    fn test_edr_position_query_url() {
        let params = edr::PositionQueryParams::default();
        let url = params.to_url(edr::collections::HRRR_SURFACE);
        assert!(url.contains("/collections/hrrr-surface/position"));
        assert!(url.contains("coords="));
    }

    #[test]
    fn test_edr_coords() {
        let (lon, lat) = edr::coords::OKC;
        assert!(lon < 0.0); // Western hemisphere
        assert!(lat > 0.0); // Northern hemisphere
    }

    #[test]
    fn test_edr_levels() {
        assert_eq!(edr::levels::STANDARD_ISOBARIC.len(), 7);
        assert_eq!(edr::levels::STANDARD_ISOBARIC[0], 1000.0);
    }
}
