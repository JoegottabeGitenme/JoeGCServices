//! Dataset query types for the grid-processor.
//!
//! This module provides a fluent builder API for specifying which dataset to access.
//! It supports both forecast models (GFS, HRRR) and observation data (GOES, MRMS).
//!
//! # Examples
//!
//! ```rust
//! use grid_processor::{DatasetQuery, TimeSpecification};
//! use chrono::Utc;
//!
//! // Query for a specific forecast
//! let query = DatasetQuery::forecast("gfs", "TMP")
//!     .at_level("2 m above ground")
//!     .at_forecast_hour(6);
//!
//! // Query for the latest available data
//! let query = DatasetQuery::forecast("gfs", "TMP")
//!     .at_level("2 m above ground")
//!     .latest();
//!
//! // Query for observation data (GOES, MRMS)
//! let query = DatasetQuery::observation("goes18", "CMI_C13")
//!     .at_time(Utc::now());
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Query parameters for finding a dataset.
///
/// Use the builder methods to construct a query:
/// - [`DatasetQuery::forecast`] for forecast models (GFS, HRRR)
/// - [`DatasetQuery::observation`] for observation data (GOES, MRMS)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetQuery {
    /// Model identifier (e.g., "gfs", "hrrr", "goes18", "mrms")
    pub model: String,

    /// Parameter name (e.g., "TMP", "UGRD", "CMI_C13")
    pub parameter: String,

    /// Optional level (e.g., "2 m above ground", "500 mb")
    pub level: Option<String>,

    /// Time specification for finding the dataset
    pub time_spec: TimeSpecification,
}

/// Time specification for finding a dataset.
///
/// Different weather data types use different temporal semantics:
/// - Forecast models have a reference (run) time and forecast hours
/// - Observation data has a single observation timestamp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeSpecification {
    /// For observation data (GOES, MRMS): specific timestamp.
    Observation {
        /// The observation timestamp
        time: DateTime<Utc>,
    },

    /// For forecast data (GFS, HRRR): run time + forecast hour.
    Forecast {
        /// Reference/run time. None means use the latest available run.
        reference_time: Option<DateTime<Utc>>,
        /// Forecast hour. None means use the earliest available (typically 0).
        forecast_hour: Option<u32>,
    },

    /// Get the latest available data regardless of type.
    /// For forecasts: latest run, earliest forecast hour.
    /// For observations: most recent observation.
    Latest,
}

impl DatasetQuery {
    /// Create a query for a forecast model dataset.
    ///
    /// # Arguments
    /// * `model` - Model identifier (e.g., "gfs", "hrrr")
    /// * `parameter` - Parameter name (e.g., "TMP", "UGRD")
    ///
    /// # Example
    /// ```rust
    /// use grid_processor::DatasetQuery;
    ///
    /// let query = DatasetQuery::forecast("gfs", "TMP")
    ///     .at_level("2 m above ground")
    ///     .at_forecast_hour(6);
    /// ```
    pub fn forecast(model: impl Into<String>, parameter: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            parameter: parameter.into(),
            level: None,
            time_spec: TimeSpecification::Latest,
        }
    }

    /// Create a query for observation data.
    ///
    /// # Arguments
    /// * `model` - Model/source identifier (e.g., "goes18", "mrms")
    /// * `parameter` - Parameter name (e.g., "CMI_C13", "REFL")
    ///
    /// # Example
    /// ```rust
    /// use grid_processor::DatasetQuery;
    /// use chrono::Utc;
    ///
    /// let query = DatasetQuery::observation("goes18", "CMI_C13")
    ///     .at_time(Utc::now());
    /// ```
    pub fn observation(model: impl Into<String>, parameter: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            parameter: parameter.into(),
            level: None,
            time_spec: TimeSpecification::Latest,
        }
    }

    /// Specify the level for this query.
    ///
    /// # Arguments
    /// * `level` - Level description (e.g., "2 m above ground", "500 mb", "surface")
    pub fn at_level(mut self, level: impl Into<String>) -> Self {
        self.level = Some(level.into());
        self
    }

    /// Specify the forecast hour for a forecast query.
    ///
    /// # Arguments
    /// * `hour` - Forecast hour (0, 3, 6, 12, etc.)
    pub fn at_forecast_hour(mut self, hour: u32) -> Self {
        match &mut self.time_spec {
            TimeSpecification::Forecast { forecast_hour, .. } => {
                *forecast_hour = Some(hour);
            }
            _ => {
                self.time_spec = TimeSpecification::Forecast {
                    reference_time: None,
                    forecast_hour: Some(hour),
                };
            }
        }
        self
    }

    /// Specify the reference (run) time for a forecast query.
    ///
    /// # Arguments
    /// * `time` - Model run time (e.g., 2024-12-22T00:00:00Z for the 00z run)
    pub fn at_run(mut self, time: DateTime<Utc>) -> Self {
        match &mut self.time_spec {
            TimeSpecification::Forecast { reference_time, .. } => {
                *reference_time = Some(time);
            }
            _ => {
                self.time_spec = TimeSpecification::Forecast {
                    reference_time: Some(time),
                    forecast_hour: None,
                };
            }
        }
        self
    }

    /// Specify the observation time for an observation query.
    ///
    /// # Arguments
    /// * `time` - Observation timestamp
    pub fn at_time(mut self, time: DateTime<Utc>) -> Self {
        self.time_spec = TimeSpecification::Observation { time };
        self
    }

    /// Request the latest available data.
    ///
    /// For forecast models: latest run, earliest forecast hour.
    /// For observation data: most recent observation.
    pub fn latest(mut self) -> Self {
        self.time_spec = TimeSpecification::Latest;
        self
    }

    /// Check if this query is for observation data.
    pub fn is_observation(&self) -> bool {
        matches!(self.time_spec, TimeSpecification::Observation { .. })
    }

    /// Check if this query is for forecast data.
    pub fn is_forecast(&self) -> bool {
        matches!(self.time_spec, TimeSpecification::Forecast { .. })
    }

    /// Get the forecast hour if specified.
    pub fn forecast_hour(&self) -> Option<u32> {
        match &self.time_spec {
            TimeSpecification::Forecast { forecast_hour, .. } => *forecast_hour,
            _ => None,
        }
    }

    /// Get the reference time if specified.
    pub fn reference_time(&self) -> Option<DateTime<Utc>> {
        match &self.time_spec {
            TimeSpecification::Forecast { reference_time, .. } => *reference_time,
            _ => None,
        }
    }

    /// Get the observation time if specified.
    pub fn observation_time(&self) -> Option<DateTime<Utc>> {
        match &self.time_spec {
            TimeSpecification::Observation { time } => Some(*time),
            _ => None,
        }
    }
}

impl Default for TimeSpecification {
    fn default() -> Self {
        Self::Latest
    }
}

/// Result of a point query containing value and metadata.
#[derive(Debug, Clone)]
pub struct PointValue {
    /// The data value at the queried point, if available
    pub value: Option<f32>,
    /// Physical units of the value
    pub units: String,
    /// Model/source identifier
    pub model: String,
    /// Parameter name
    pub parameter: String,
    /// Level description
    pub level: String,
    /// Reference time (for forecasts) or observation time
    pub time: DateTime<Utc>,
    /// Forecast hour (for forecast data)
    pub forecast_hour: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_forecast_query_builder() {
        let query = DatasetQuery::forecast("gfs", "TMP")
            .at_level("2 m above ground")
            .at_forecast_hour(6);

        assert_eq!(query.model, "gfs");
        assert_eq!(query.parameter, "TMP");
        assert_eq!(query.level, Some("2 m above ground".to_string()));
        assert_eq!(query.forecast_hour(), Some(6));
        assert!(query.is_forecast());
        assert!(!query.is_observation());
    }

    #[test]
    fn test_observation_query_builder() {
        let time = Utc.with_ymd_and_hms(2024, 12, 22, 12, 0, 0).unwrap();
        let query = DatasetQuery::observation("goes18", "CMI_C13").at_time(time);

        assert_eq!(query.model, "goes18");
        assert_eq!(query.parameter, "CMI_C13");
        assert_eq!(query.observation_time(), Some(time));
        assert!(query.is_observation());
        assert!(!query.is_forecast());
    }

    #[test]
    fn test_latest_query() {
        let query = DatasetQuery::forecast("gfs", "TMP")
            .at_level("surface")
            .latest();

        assert!(matches!(query.time_spec, TimeSpecification::Latest));
        assert_eq!(query.forecast_hour(), None);
        assert_eq!(query.reference_time(), None);
    }

    #[test]
    fn test_full_forecast_query() {
        let run_time = Utc.with_ymd_and_hms(2024, 12, 22, 0, 0, 0).unwrap();
        let query = DatasetQuery::forecast("hrrr", "REFC")
            .at_level("entire atmosphere")
            .at_run(run_time)
            .at_forecast_hour(12);

        assert_eq!(query.model, "hrrr");
        assert_eq!(query.parameter, "REFC");
        assert_eq!(query.level, Some("entire atmosphere".to_string()));
        assert_eq!(query.reference_time(), Some(run_time));
        assert_eq!(query.forecast_hour(), Some(12));
    }
}
