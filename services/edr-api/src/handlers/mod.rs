//! HTTP request handlers for the EDR API.

pub mod api;
pub mod area;
pub mod astro;
pub mod catalog_check;
pub mod collections;
pub mod conformance;
pub mod corridor;
pub mod cube;
pub mod forecast_params;
pub mod health;
pub mod instances;
pub mod items;
pub mod landing;
pub mod light_pollution;
pub mod lightning;
pub mod linear_features;
pub mod locations;
pub mod nws_alerts;
pub mod observations;
pub mod position;
pub mod radius;
pub mod storm_events;
pub mod trajectory;

/// The valid times an interval `datetime` expands to, as `YYYY-MM-DDTHH:MM:SSZ`: those any run
/// holds for one of `parameters`, not the whole model's. A collection whose parameters only some
/// instants carry (SBT124 in hrrr-satellite, which HRRR writes for fewer hours than TMP) would
/// otherwise expand to instants it lacks, and each would be answered by the nearest grid, hours
/// away. No parameters keeps the model-wide axis.
pub(crate) async fn interval_time_strings(
    catalog: &storage::Catalog,
    model: &str,
    parameters: &[String],
) -> Vec<String> {
    let mut times: Vec<_> = if parameters.is_empty() {
        catalog
            .get_model_valid_times(model)
            .await
            .unwrap_or_default()
    } else {
        catalog
            .get_model_run_valid_times(model, parameters)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(_, valid)| valid)
            .collect()
    };
    times.dedup();
    times
        .into_iter()
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .collect()
}
