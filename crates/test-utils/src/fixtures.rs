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
}
