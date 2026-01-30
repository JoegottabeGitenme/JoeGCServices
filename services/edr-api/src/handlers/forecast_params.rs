//! Shared handling for forecast model run and forecast-hour parameters.
//!
//! This module provides types and functions for parsing and validating
//! the `run` and `forecast-hour` query parameters used in EDR queries
//! for forecast model data.
//!
//! # Parameter Rules
//!
//! - If `forecast-hour` is specified, `run` MUST also be specified (error otherwise)
//! - If neither is specified, the handler uses `datetime` or defaults to latest
//! - If only `run` is specified, all forecast hours for that run are returned
//! - If both are specified, only the specified hours from that run are returned
//!
//! # Examples
//!
//! ```text
//! # No params - use datetime or latest
//! /position?coords=POINT(-77 39)
//!
//! # Specific run, all forecast hours
//! /position?coords=POINT(-77 39)&run=2024-12-29T12:00:00Z
//!
//! # Specific run and forecast hours
//! /position?coords=POINT(-77 39)&run=2024-12-29T12:00:00Z&forecast-hour=0,6,12
//!
//! # Range of forecast hours
//! /position?coords=POINT(-77 39)&run=2024-12-29T12:00:00Z&forecast-hour=0/24
//! ```

use chrono::{DateTime, Utc};
use edr_protocol::responses::ExceptionResponse;
use edr_protocol::ForecastHourQuery;

/// Parsed forecast query parameters.
#[derive(Debug, Clone)]
pub struct ForecastParams {
    /// The model run time (reference_time). None means use latest/best available.
    pub run: Option<DateTime<Utc>>,
    /// The forecast hours to retrieve. None means use datetime or all available.
    pub forecast_hour: Option<ForecastHourQuery>,
}

impl ForecastParams {
    /// Parse the run and forecast-hour query parameters.
    ///
    /// # Rules
    /// - If `forecast_hour` is provided without `run`, returns an error
    /// - If `run` is provided, parses it as ISO8601 datetime
    /// - If `forecast_hour` is provided, parses it using ForecastHourQuery::parse
    ///
    /// # Returns
    /// - Ok(ForecastParams) on success
    /// - Err(ExceptionResponse) with appropriate error message on failure
    pub fn parse(
        run: Option<&str>,
        forecast_hour: Option<&str>,
    ) -> Result<Self, ExceptionResponse> {
        // Parse forecast-hour first to check if run is required
        let forecast_hour = if let Some(fh) = forecast_hour {
            let fh = fh.trim();
            if fh.is_empty() {
                None
            } else {
                Some(ForecastHourQuery::parse(fh).map_err(|e| {
                    ExceptionResponse::bad_request(format!(
                        "Invalid forecast-hour parameter: {}",
                        e
                    ))
                })?)
            }
        } else {
            None
        };

        // Parse run parameter
        let run = if let Some(r) = run {
            let r = r.trim();
            if r.is_empty() {
                None
            } else {
                Some(
                    DateTime::parse_from_rfc3339(r)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|_| {
                            ExceptionResponse::bad_request(format!(
                                "Invalid run parameter '{}'. Expected ISO8601 datetime (e.g., 2024-12-29T12:00:00Z)",
                                r
                            ))
                        })?,
                )
            }
        } else {
            None
        };

        // Validate: forecast-hour requires run
        if forecast_hour.is_some() && run.is_none() {
            return Err(ExceptionResponse::bad_request(
                "Parameter 'run' is required when 'forecast-hour' is specified",
            ));
        }

        Ok(Self { run, forecast_hour })
    }
}

/// Strategy for resolving forecast data based on parameters.
#[derive(Debug, Clone)]
pub enum ForecastQueryStrategy {
    /// Neither run nor forecast-hour specified.
    /// Use datetime parameter or default to latest data.
    UseDatetime,

    /// Only run specified, no forecast-hour.
    /// Return all available forecast hours for that run.
    AllHoursForRun(DateTime<Utc>),

    /// Both run and forecast-hour specified.
    /// Return only the specified hours from that run.
    /// Error if any requested hour is not available.
    StrictRunHours {
        run: DateTime<Utc>,
        hours: ForecastHourQuery,
    },
}

impl ForecastParams {
    /// Determine the query strategy based on the parsed parameters.
    pub fn strategy(&self) -> ForecastQueryStrategy {
        match (&self.run, &self.forecast_hour) {
            (None, None) => ForecastQueryStrategy::UseDatetime,
            (Some(run), None) => ForecastQueryStrategy::AllHoursForRun(*run),
            (Some(run), Some(hours)) => ForecastQueryStrategy::StrictRunHours {
                run: *run,
                hours: hours.clone(),
            },
            // This case should not happen due to validation in parse()
            (None, Some(_)) => ForecastQueryStrategy::UseDatetime,
        }
    }
}

/// Validate that forecast parameters are not used with observation data.
///
/// Returns an error if run or forecast-hour are provided for non-forecast data.
pub fn validate_not_observation_data(
    params: &ForecastParams,
    is_observation_model: bool,
) -> Result<(), ExceptionResponse> {
    if is_observation_model && (params.run.is_some() || params.forecast_hour.is_some()) {
        return Err(ExceptionResponse::bad_request(
            "Parameters 'run' and 'forecast-hour' are not supported for observation data. \
             Use 'datetime' parameter instead.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_no_params() {
        let params = ForecastParams::parse(None, None).unwrap();
        assert!(params.run.is_none());
        assert!(params.forecast_hour.is_none());
        assert!(matches!(
            params.strategy(),
            ForecastQueryStrategy::UseDatetime
        ));
    }

    #[test]
    fn test_parse_run_only() {
        let params = ForecastParams::parse(Some("2024-12-29T12:00:00Z"), None).unwrap();
        assert!(params.run.is_some());
        assert!(params.forecast_hour.is_none());

        if let ForecastQueryStrategy::AllHoursForRun(run) = params.strategy() {
            assert_eq!(
                run.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "2024-12-29T12:00:00Z"
            );
        } else {
            panic!("Expected AllHoursForRun strategy");
        }
    }

    #[test]
    fn test_parse_run_and_forecast_hour() {
        let params = ForecastParams::parse(Some("2024-12-29T12:00:00Z"), Some("0,6,12")).unwrap();

        assert!(params.run.is_some());
        assert!(params.forecast_hour.is_some());

        if let ForecastQueryStrategy::StrictRunHours { run, hours } = params.strategy() {
            assert_eq!(
                run.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "2024-12-29T12:00:00Z"
            );
            if let ForecastHourQuery::List(h) = hours {
                assert_eq!(h, vec![0, 6, 12]);
            } else {
                panic!("Expected List");
            }
        } else {
            panic!("Expected StrictRunHours strategy");
        }
    }

    #[test]
    fn test_parse_forecast_hour_without_run_error() {
        let result = ForecastParams::parse(None, Some("0,6,12"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_run() {
        let result = ForecastParams::parse(Some("not-a-date"), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_forecast_hour() {
        let result = ForecastParams::parse(Some("2024-12-29T12:00:00Z"), Some("abc"));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_strings() {
        let params = ForecastParams::parse(Some(""), Some("")).unwrap();
        assert!(params.run.is_none());
        assert!(params.forecast_hour.is_none());
    }

    #[test]
    fn test_validate_observation_data() {
        let params = ForecastParams::parse(Some("2024-12-29T12:00:00Z"), None).unwrap();

        // Should fail for observation data
        let result = validate_not_observation_data(&params, true);
        assert!(result.is_err());

        // Should pass for forecast data
        let result = validate_not_observation_data(&params, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_observation_data_no_params() {
        let params = ForecastParams::parse(None, None).unwrap();

        // Should pass even for observation data when no forecast params
        let result = validate_not_observation_data(&params, true);
        assert!(result.is_ok());
    }
}
