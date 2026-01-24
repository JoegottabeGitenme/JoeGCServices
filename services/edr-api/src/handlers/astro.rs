//! Astro collection handler.
//!
//! Provides on-demand astronomical data (solar and lunar) computed using the astro crate.
//! Unlike other collections, this doesn't query grid data but computes values in real-time.

use axum::{
    extract::{Extension, Query},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use edr_protocol::{
    coverage_json::CovJsonParameter,
    parameters::{I18nString, ObservedProperty, Unit, UnitSymbol},
    queries::PositionQuery,
    responses::ExceptionResponse,
    CoverageJson,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::astro::{compute_lunar, compute_solar, compute_sunrise_sunset, MoonPhase};
use crate::content_negotiation::{check_png_not_supported, negotiate_format};
use crate::metrics::{extract_client_ip, extract_user_agent, EndpointType, FormatType, Timer};
use crate::state::AppState;
use crate::temporal_interpolation::{expand_interval_with_step, parse_iso8601_duration};

/// Query parameters for astro position endpoint.
#[derive(Debug, Deserialize)]
pub struct AstroQueryParams {
    /// Coordinates as WKT POINT. Required.
    pub coords: Option<String>,

    /// Datetime instant or interval. If not provided, uses current time.
    pub datetime: Option<String>,

    /// Parameter name(s) to retrieve. If not provided, returns all parameters.
    #[serde(rename = "parameter-name")]
    pub parameter_name: Option<String>,

    /// Step interval for generating times within a datetime range.
    /// Specified as an ISO 8601 duration (e.g., PT1H for 1 hour, P1D for 1 day).
    /// Default is PT1H (hourly) if not specified.
    pub step: Option<String>,

    /// Output format (for consistency with other endpoints).
    pub f: Option<String>,
}

/// GET /edr/collections/astro/position
pub async fn position_handler(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<AstroQueryParams>,
    headers: HeaderMap,
) -> Response {
    let timer = Timer::start();
    let client_ip = extract_client_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    // Negotiate output format
    let output_format = match negotiate_format(&headers, params.f.as_deref()) {
        Ok(format) => format,
        Err(response) => {
            state
                .metrics
                .record_request(
                    EndpointType::Position,
                    Some("astro"),
                    &[],
                    FormatType::CoverageJson,
                    timer.elapsed_us(),
                    false,
                    client_ip.as_deref(),
                    user_agent.as_deref(),
                )
                .await;
            return response;
        }
    };

    // PNG not supported for astro
    if let Some(response) = check_png_not_supported(output_format, "astro") {
        return response;
    }

    // Parse coordinates
    let coords_str = match &params.coords {
        Some(c) if !c.trim().is_empty() => c.as_str(),
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request("Missing required parameter: coords"),
            );
        }
    };

    let (lon, lat) = match PositionQuery::parse_coords(coords_str) {
        Ok((lon, lat)) => (lon, lat),
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ExceptionResponse::bad_request(format!("Invalid coords parameter: {}", e)),
            );
        }
    };

    // Validate lat/lon ranges
    if !(-90.0..=90.0).contains(&lat) {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!("Latitude must be between -90 and 90: {}", lat)),
        );
    }
    if !(-180.0..=180.0).contains(&lon) {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Longitude must be between -180 and 180: {}",
                lon
            )),
        );
    }

    // Parse datetime (single or range)
    let times = match parse_datetime_param(&params.datetime, &params.step) {
        Ok(times) => times,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
        }
    };

    // Enforce reasonable limits
    if times.len() > 1000 {
        return error_response(
            StatusCode::BAD_REQUEST,
            ExceptionResponse::bad_request(format!(
                "Too many time steps requested: {}. Maximum is 1000.",
                times.len()
            )),
        );
    }

    // Parse requested parameters
    let requested_params = parse_requested_parameters(params.parameter_name.as_deref());

    // Validate requested parameters
    if let Err(e) = validate_parameters(&requested_params) {
        return error_response(StatusCode::BAD_REQUEST, ExceptionResponse::bad_request(e));
    }

    // Compute astro data for each time
    let mut solar_data = Vec::new();
    let mut lunar_data = Vec::new();

    for time in &times {
        solar_data.push(compute_solar(lat, lon, time));
        lunar_data.push(compute_lunar(lat, lon, time));
    }

    // Compute sunrise/sunset once per unique date
    use std::collections::{HashMap, HashSet};
    let unique_dates: HashSet<_> = times.iter().map(|dt| dt.date_naive()).collect();
    let mut sunrise_sunset_by_date: HashMap<_, _> = HashMap::new();
    for date in unique_dates {
        sunrise_sunset_by_date.insert(date, compute_sunrise_sunset(lat, lon, date));
    }

    // Build CoverageJSON response
    let coverage = build_coverage_json(
        lon,
        lat,
        &times,
        &solar_data,
        &lunar_data,
        &sunrise_sunset_by_date,
        &requested_params,
    );

    // Record metrics
    state
        .metrics
        .record_request(
            EndpointType::Position,
            Some("astro"),
            &requested_params,
            FormatType::CoverageJson,
            timer.elapsed_us(),
            true,
            client_ip.as_deref(),
            user_agent.as_deref(),
        )
        .await;

    // Return response
    (StatusCode::OK, Json(coverage)).into_response()
}

/// Parse datetime parameter into a list of DateTime values.
fn parse_datetime_param(
    datetime: &Option<String>,
    step: &Option<String>,
) -> Result<Vec<DateTime<Utc>>, String> {
    match datetime {
        None => {
            // No datetime provided, use current time
            Ok(vec![Utc::now()])
        }
        Some(dt_str) => {
            // Check if it's a range (contains '/')
            if dt_str.contains('/') {
                let parts: Vec<&str> = dt_str.split('/').collect();
                if parts.len() != 2 {
                    return Err(format!("Invalid datetime interval: {}", dt_str));
                }

                let start: DateTime<Utc> = parts[0]
                    .parse()
                    .map_err(|_| format!("Invalid start datetime: {}", parts[0]))?;
                let end: DateTime<Utc> = parts[1]
                    .parse()
                    .map_err(|_| format!("Invalid end datetime: {}", parts[1]))?;

                if end <= start {
                    return Err("End datetime must be after start datetime".to_string());
                }

                // Parse step or use default (PT1H)
                let step_duration = match step {
                    Some(s) => parse_iso8601_duration(s)
                        .ok_or_else(|| format!("Invalid step duration: {}", s))?,
                    None => Duration::hours(1), // Default to hourly
                };

                // Generate times
                let times = expand_interval_with_step(start, end, step_duration);
                Ok(times)
            } else {
                // Single datetime
                let dt: DateTime<Utc> = dt_str
                    .parse()
                    .map_err(|_| format!("Invalid datetime: {}", dt_str))?;
                Ok(vec![dt])
            }
        }
    }
}

/// Parse requested parameter names from query string.
fn parse_requested_parameters(parameter_name: Option<&str>) -> Vec<String> {
    match parameter_name {
        None => {
            // Return all parameters
            vec![
                "sunrise_time".to_string(),
                "sunset_time".to_string(),
                "solar_noon".to_string(),
                "sun_altitude".to_string(),
                "sun_azimuth".to_string(),
                "moon_phase".to_string(),
                "moon_illumination".to_string(),
                "moon_age".to_string(),
            ]
        }
        Some(params) => params.split(',').map(|s| s.trim().to_string()).collect(),
    }
}

/// Validate that requested parameters are supported.
fn validate_parameters(params: &[String]) -> Result<(), String> {
    const VALID_PARAMS: &[&str] = &[
        "sunrise_time",
        "sunset_time",
        "solar_noon",
        "sun_altitude",
        "sun_azimuth",
        "moon_phase",
        "moon_illumination",
        "moon_age",
    ];

    for param in params {
        if !VALID_PARAMS.contains(&param.as_str()) {
            return Err(format!(
                "Invalid parameter name: {}. Valid parameters: {}",
                param,
                VALID_PARAMS.join(", ")
            ));
        }
    }
    Ok(())
}

/// Build CoverageJSON response from computed astro data.
fn build_coverage_json(
    lon: f64,
    lat: f64,
    times: &[DateTime<Utc>],
    solar_data: &[crate::astro::SolarData],
    lunar_data: &[crate::astro::LunarData],
    sunrise_sunset_by_date: &std::collections::HashMap<
        chrono::NaiveDate,
        (Option<i64>, Option<i64>),
    >,
    requested_params: &[String],
) -> CoverageJson {
    let time_strings: Vec<String> = times.iter().map(|t| t.to_rfc3339()).collect();

    let coverage = if times.len() == 1 {
        // Single point
        CoverageJson::point(lon, lat, Some(time_strings[0].clone()), None)
    } else {
        // Time series
        CoverageJson::point_series(lon, lat, time_strings, None)
    };

    let mut coverage = coverage;

    // Add each requested parameter
    for param_name in requested_params {
        match param_name.as_str() {
            "sunrise_time" => {
                let values: Vec<Option<i64>> = times
                    .iter()
                    .map(|dt| {
                        let date = dt.date_naive();
                        sunrise_sunset_by_date.get(&date).and_then(|(sr, _)| *sr)
                    })
                    .collect();
                coverage = add_timestamp_parameter(
                    coverage,
                    "sunrise_time",
                    "Sunrise Time",
                    "Time of sunrise for this date (Unix timestamp in seconds). Null during polar night.",
                    values,
                );
            }
            "sunset_time" => {
                let values: Vec<Option<i64>> = times
                    .iter()
                    .map(|dt| {
                        let date = dt.date_naive();
                        sunrise_sunset_by_date.get(&date).and_then(|(_, ss)| *ss)
                    })
                    .collect();
                coverage = add_timestamp_parameter(
                    coverage,
                    "sunset_time",
                    "Sunset Time",
                    "Time of sunset for this date (Unix timestamp in seconds). Null during midnight sun.",
                    values,
                );
            }
            "solar_noon" => {
                coverage = add_timestamp_parameter(
                    coverage,
                    "solar_noon",
                    "Solar Noon",
                    "Time when sun reaches highest point (Unix timestamp in seconds).",
                    solar_data.iter().map(|s| Some(s.solar_noon)).collect(),
                );
            }
            "sun_altitude" => {
                coverage = add_float_parameter(
                    coverage,
                    "sun_altitude",
                    "Sun Altitude",
                    "Sun elevation angle above horizon.",
                    "degrees",
                    "deg",
                    solar_data.iter().map(|s| s.altitude as f32).collect(),
                );
            }
            "sun_azimuth" => {
                coverage = add_float_parameter(
                    coverage,
                    "sun_azimuth",
                    "Sun Azimuth",
                    "Sun compass direction (0=North, 90=East, 180=South, 270=West).",
                    "degrees",
                    "deg",
                    solar_data.iter().map(|s| s.azimuth as f32).collect(),
                );
            }

            "moon_phase" => {
                coverage = add_phase_parameter(
                    coverage,
                    "moon_phase",
                    "Moon Phase",
                    "Current phase of the moon.",
                    lunar_data.iter().map(|l| l.phase).collect(),
                );
            }
            "moon_illumination" => {
                coverage = add_float_parameter(
                    coverage,
                    "moon_illumination",
                    "Moon Illumination",
                    "Fraction of moon disk illuminated (0.0 = new moon, 1.0 = full moon).",
                    "fraction",
                    "1",
                    lunar_data.iter().map(|l| l.illumination as f32).collect(),
                );
            }
            "moon_age" => {
                coverage = add_float_parameter(
                    coverage,
                    "moon_age",
                    "Moon Age",
                    "Days since last new moon.",
                    "days",
                    "d",
                    lunar_data.iter().map(|l| l.age_days as f32).collect(),
                );
            }
            _ => {} // Skip unknown parameters (already validated)
        }
    }

    coverage
}

/// Add a timestamp parameter (Unix seconds) to the coverage.
fn add_timestamp_parameter(
    coverage: CoverageJson,
    name: &str,
    label: &str,
    description: &str,
    values: Vec<Option<i64>>,
) -> CoverageJson {
    let param = CovJsonParameter {
        type_: "Parameter".to_string(),
        description: Some(I18nString::english(description)),
        observed_property: ObservedProperty {
            id: None,
            label: Some(I18nString::english(label)),
            description: Some(I18nString::english(description)),
            categories: None,
        },
        unit: Some(Unit {
            label: Some(I18nString::english("Unix timestamp (seconds since epoch)")),
            symbol: Some(UnitSymbol::Simple("s".to_string())),
        }),
    };

    if values.len() == 1 {
        // Single value
        if let Some(v) = values[0] {
            coverage.with_parameter(name, param, v as f32)
        } else {
            coverage.with_parameter_null(name, param)
        }
    } else {
        // Array of values (with nulls)
        let float_values: Vec<Option<f32>> = values.iter().map(|v| v.map(|x| x as f32)).collect();
        coverage.with_parameter_array_nullable(
            name,
            param,
            float_values,
            vec![values.len()],
            vec!["t".to_string()],
        )
    }
}

/// Add a float parameter to the coverage.
fn add_float_parameter(
    coverage: CoverageJson,
    name: &str,
    label: &str,
    description: &str,
    unit_label: &str,
    unit_symbol: &str,
    values: Vec<f32>,
) -> CoverageJson {
    let param = CovJsonParameter {
        type_: "Parameter".to_string(),
        description: Some(I18nString::english(description)),
        observed_property: ObservedProperty {
            id: None,
            label: Some(I18nString::english(label)),
            description: Some(I18nString::english(description)),
            categories: None,
        },
        unit: Some(Unit {
            label: Some(I18nString::english(unit_label)),
            symbol: Some(UnitSymbol::Simple(unit_symbol.to_string())),
        }),
    };

    if values.len() == 1 {
        coverage.with_parameter(name, param, values[0])
    } else {
        let len = values.len();
        coverage.with_parameter_array(name, param, values, vec![len], vec!["t".to_string()])
    }
}

/// Add a moon phase parameter (categorical string) to the coverage.
fn add_phase_parameter(
    coverage: CoverageJson,
    name: &str,
    label: &str,
    description: &str,
    phases: Vec<MoonPhase>,
) -> CoverageJson {
    // For moon phase, we'll encode as numeric with category mapping
    // 0=new_moon, 1=waxing_crescent, etc.
    let phase_to_index = |phase: MoonPhase| -> f32 {
        match phase {
            MoonPhase::NewMoon => 0.0,
            MoonPhase::WaxingCrescent => 1.0,
            MoonPhase::FirstQuarter => 2.0,
            MoonPhase::WaxingGibbous => 3.0,
            MoonPhase::FullMoon => 4.0,
            MoonPhase::WaningGibbous => 5.0,
            MoonPhase::LastQuarter => 6.0,
            MoonPhase::WaningCrescent => 7.0,
        }
    };

    let values: Vec<f32> = phases.iter().map(|p| phase_to_index(*p)).collect();

    // Build categories
    let mut categories = Vec::new();
    for (i, phase_name) in [
        "new_moon",
        "waxing_crescent",
        "first_quarter",
        "waxing_gibbous",
        "full_moon",
        "waning_gibbous",
        "last_quarter",
        "waning_crescent",
    ]
    .iter()
    .enumerate()
    {
        categories.push(edr_protocol::parameters::Category {
            id: i.to_string(),
            label: Some(I18nString::english(phase_name)),
            description: None,
        });
    }

    let param = CovJsonParameter {
        type_: "Parameter".to_string(),
        description: Some(I18nString::english(description)),
        observed_property: ObservedProperty {
            id: None,
            label: Some(I18nString::english(label)),
            description: Some(I18nString::english(description)),
            categories: Some(categories),
        },
        unit: None,
    };

    if values.len() == 1 {
        coverage.with_parameter(name, param, values[0])
    } else {
        let len = values.len();
        coverage.with_parameter_array(name, param, values, vec![len], vec!["t".to_string()])
    }
}

/// Helper to create error responses.
fn error_response(status: StatusCode, exception: ExceptionResponse) -> Response {
    (status, Json(exception)).into_response()
}
