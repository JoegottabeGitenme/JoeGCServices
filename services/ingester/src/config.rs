//! Ingester configuration.

use anyhow::Result;
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;

use storage::ObjectStorageConfig;

/// Top-level ingester configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngesterConfig {
    /// Object storage configuration
    pub storage: ObjectStorageConfig,

    /// Database connection URL
    pub database_url: String,

    /// Redis URL for coordination
    pub redis_url: String,

    /// Model-specific configurations
    pub models: HashMap<String, ModelConfig>,

    /// Global polling interval (seconds)
    pub poll_interval_secs: u64,

    /// Number of parallel downloads
    pub parallel_downloads: usize,

    /// Retention period in hours
    pub retention_hours: u64,
}

impl IngesterConfig {
    /// Load configuration from YAML files.
    pub fn from_yaml<P: AsRef<std::path::Path>>(base_path: P) -> Result<Self> {
        use crate::config_loader;
        
        let all_configs = config_loader::load_all_configs(base_path)?;
        all_configs.to_runtime_config()
    }
    
    /// Load configuration from environment variables (legacy fallback).
    pub fn from_env() -> Result<Self> {
        let storage = ObjectStorageConfig {
            endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
            bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
            access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
            secret_access_key: env::var("S3_SECRET_KEY")
                .unwrap_or_else(|_| "minioadmin".to_string()),
            region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            allow_http: env::var("S3_ALLOW_HTTP")
                .map(|v| v == "true")
                .unwrap_or(true),
        };

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://postgres:postgres@postgres:5432/weatherwms".to_string()
        });

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());

        // Default model configurations
        let mut models = HashMap::new();

        models.insert(
            "gfs".to_string(),
            ModelConfig {
                name: "GFS".to_string(),
                source: DataSource::NoaaAws {
                    bucket: "noaa-gfs-bdp-pds".to_string(),
                    prefix_template: "gfs.{date}/{cycle}/atmos".to_string(),
                },
                parameters: vec![
                    ParameterConfig {
                        name: "temperature_2m".to_string(),
                        grib_filter: GribFilter {
                            level: "2 m above ground".to_string(),
                            parameter: "TMP".to_string(),
                        },
                    },
                    ParameterConfig {
                        name: "wind_u_10m".to_string(),
                        grib_filter: GribFilter {
                            level: "10 m above ground".to_string(),
                            parameter: "UGRD".to_string(),
                        },
                    },
                    ParameterConfig {
                        name: "wind_v_10m".to_string(),
                        grib_filter: GribFilter {
                            level: "10 m above ground".to_string(),
                            parameter: "VGRD".to_string(),
                        },
                    },
                    ParameterConfig {
                        name: "pressure_msl".to_string(),
                        grib_filter: GribFilter {
                            level: "mean sea level".to_string(),
                            parameter: "PRMSL".to_string(),
                        },
                    },
                ],
                cycles: vec![0, 6, 12, 18],
                forecast_hours: (0..=120).step_by(3).collect(),
                resolution: "0p25".to_string(),
                poll_interval_secs: 3600, // 1 hour
            },
        );

        models.insert(
            "hrrr".to_string(),
            ModelConfig {
                name: "HRRR".to_string(),
                source: DataSource::NoaaAws {
                    bucket: "noaa-hrrr-bdp-pds".to_string(),
                    prefix_template: "hrrr.{date}/conus".to_string(),
                },
                parameters: vec![
                    ParameterConfig {
                        name: "temperature_2m".to_string(),
                        grib_filter: GribFilter {
                            level: "2 m above ground".to_string(),
                            parameter: "TMP".to_string(),
                        },
                    },
                    ParameterConfig {
                        name: "reflectivity".to_string(),
                        grib_filter: GribFilter {
                            level: "1000 m above ground".to_string(),
                            parameter: "REFC".to_string(),
                        },
                    },
                ],
                cycles: (0..24).collect(),
                forecast_hours: (0..=18).collect(),
                resolution: "3km".to_string(),
                poll_interval_secs: 3600,
            },
        );

        // GOES-16 (GOES-East) satellite imagery
        models.insert(
            "goes16".to_string(),
            ModelConfig {
                name: "GOES-16".to_string(),
                source: DataSource::GoesAws {
                    bucket: "noaa-goes16".to_string(),
                    product: "ABI-L2-CMIPC".to_string(),
                    bands: vec![1, 2, 13], // Visible blue, red visible, clean IR
                },
                parameters: vec![
                    ParameterConfig {
                        name: "visible".to_string(),
                        grib_filter: GribFilter {
                            level: "toa".to_string(),
                            parameter: "CMI_C02".to_string(), // 0.64µm red visible
                        },
                    },
                    ParameterConfig {
                        name: "ir".to_string(),
                        grib_filter: GribFilter {
                            level: "toa".to_string(),
                            parameter: "CMI_C13".to_string(), // 10.3µm clean IR
                        },
                    },
                ],
                cycles: (0..24).collect(), // Hourly
                forecast_hours: vec![1, 2, 13], // Reused as band numbers
                resolution: "1km".to_string(),
                poll_interval_secs: 300, // 5 minutes - GOES updates every 5-15 min
            },
        );

        // GOES-18 (GOES-West) satellite imagery
        models.insert(
            "goes18".to_string(),
            ModelConfig {
                name: "GOES-18".to_string(),
                source: DataSource::GoesAws {
                    bucket: "noaa-goes18".to_string(),
                    product: "ABI-L2-CMIPC".to_string(),
                    bands: vec![1, 2, 13],
                },
                parameters: vec![
                    ParameterConfig {
                        name: "visible".to_string(),
                        grib_filter: GribFilter {
                            level: "toa".to_string(),
                            parameter: "CMI_C02".to_string(),
                        },
                    },
                    ParameterConfig {
                        name: "ir".to_string(),
                        grib_filter: GribFilter {
                            level: "toa".to_string(),
                            parameter: "CMI_C13".to_string(),
                        },
                    },
                ],
                cycles: (0..24).collect(),
                forecast_hours: vec![1, 2, 13],
                resolution: "1km".to_string(),
                poll_interval_secs: 300,
            },
        );

        Ok(Self {
            storage,
            database_url,
            redis_url,
            models,
            poll_interval_secs: 300, // 5 minutes
            parallel_downloads: 4,
            retention_hours: 168, // 7 days
        })
    }
}

/// Configuration for a specific weather model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Human-readable model name
    pub name: String,

    /// Data source configuration
    pub source: DataSource,

    /// Parameters to extract
    pub parameters: Vec<ParameterConfig>,

    /// Model run cycles (UTC hours)
    pub cycles: Vec<u32>,

    /// Forecast hours to download
    pub forecast_hours: Vec<u32>,

    /// Grid resolution identifier
    pub resolution: String,

    /// Model-specific polling interval
    pub poll_interval_secs: u64,
}

/// Data source configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DataSource {
    /// NOAA data on AWS Open Data
    NoaaAws {
        bucket: String,
        prefix_template: String,
    },

    /// NOMADS HTTP server
    Nomads {
        base_url: String,
        path_template: String,
    },

    /// THREDDS Data Server
    Thredds { catalog_url: String },

    /// GOES satellite data on AWS (NetCDF format)
    GoesAws {
        /// S3 bucket name (e.g., "noaa-goes16", "noaa-goes18")
        bucket: String,
        /// Product type (e.g., "ABI-L2-CMIPC" for CONUS CMI)
        product: String,
        /// Bands to ingest (e.g., [1, 2, 13] for visible, red visible, clean IR)
        bands: Vec<u8>,
    },
}

/// Configuration for a parameter to extract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterConfig {
    /// Internal parameter name
    pub name: String,

    /// GRIB filter criteria
    pub grib_filter: GribFilter,
}

/// GRIB2 message filter criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GribFilter {
    /// Level description (e.g., "2 m above ground")
    pub level: String,

    /// Parameter short name (e.g., "TMP")
    pub parameter: String,
}

impl DataSource {
    /// Build the URL/path for a specific date and cycle.
    pub fn build_path(&self, date: &str, cycle: u32) -> String {
        match self {
            DataSource::NoaaAws {
                prefix_template, ..
            } => prefix_template
                .replace("{date}", date)
                .replace("{cycle}", &format!("{:02}", cycle)),
            DataSource::Nomads { path_template, .. } => path_template
                .replace("{date}", date)
                .replace("{cycle}", &format!("{:02}", cycle)),
            DataSource::Thredds { .. } => {
                // THREDDS uses catalog navigation
                String::new()
            }
            DataSource::GoesAws { product, .. } => {
                // GOES uses year/day_of_year/hour structure
                // date format is YYYYMMDD, convert to year/doy
                if date.len() >= 8 {
                    let year = &date[0..4];
                    // Calculate day of year from date
                    if let Ok(parsed) = chrono::NaiveDate::parse_from_str(date, "%Y%m%d") {
                        let doy = parsed.ordinal();
                        format!("{}/{}/{:03}/{:02}", product, year, doy, cycle)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
        }
    }

    /// Get the file pattern for listing files.
    pub fn file_pattern(&self, model: &str, resolution: &str, cycle: u32, fhr: u32) -> String {
        match self {
            DataSource::NoaaAws { .. } | DataSource::Nomads { .. } => match model {
                "gfs" => format!("gfs.t{:02}z.pgrb2.{}.f{:03}", cycle, resolution, fhr),
                "hrrr" => format!("hrrr.t{:02}z.wrfsfcf{:02}.grib2", cycle, fhr),
                _ => format!("{}.t{:02}z.f{:03}.grib2", model, cycle, fhr),
            },
            DataSource::Thredds { .. } => String::new(),
            DataSource::GoesAws { .. } => {
                // GOES files match pattern: OR_ABI-L2-CMIPC-M6C{band:02}_G{sat}
                // fhr is reused as band number for GOES
                format!("C{:02}", fhr)
            }
        }
    }

    /// Check if this is a GOES data source
    pub fn is_goes(&self) -> bool {
        matches!(self, DataSource::GoesAws { .. })
    }
}
