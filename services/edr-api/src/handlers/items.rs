//! Generic dispatcher for `/edr/collections/:collection_id/items`.
//!
//! `/items` is registered once as a single route (unlike `/area`/`/radius`,
//! which are handled inline by their own dispatcher functions) because it has
//! no other caller today. This function reads the collection's
//! `observation_source` from config and forwards to whichever
//! feature-collection backend owns it, translating the generic query params
//! into that backend's specific param struct -- the same translate-then-
//! delegate pattern `area.rs`/`radius.rs` already use for storm events.
//!
//! Defaults to `storm_events` when `observation_source` is unset, preserving
//! exact prior behavior (every existing feature collection is storm-events;
//! this dispatcher only needed to exist once a second backend, `linear_features`,
//! was added).

use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;
use std::sync::Arc;

use edr_protocol::responses::ExceptionResponse;

use crate::handlers::{lightning, linear_features, storm_events};
use crate::state::AppState;

/// Superset of the storm-events and linear-features items params. Both
/// backends only read the fields they understand; `datetime` is
/// storm-events-only (trails have no temporal extent), `q`/`class` are
/// linear-features-only.
#[derive(Debug, Deserialize, Default)]
pub struct ItemsQueryParams {
    pub bbox: Option<String>,
    pub datetime: Option<String>,
    pub q: Option<String>,
    pub class: Option<String>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    pub f: Option<String>,
    /// linear-features-only: `?conditions=latest` merges trail-physics
    /// output into each feature's properties. See
    /// `linear_features::TrailItemsParams::conditions`.
    pub conditions: Option<String>,

    /// lightning-only: server-relative time window, an ISO-8601 duration
    /// (e.g. `PT10M`). See `lightning` module docs.
    pub window: Option<String>,

    /// lightning-only: change-feed cursor -- only flashes with `id` greater than this.
    pub after: Option<i64>,

    /// lightning-only: `goes-east` (default), `goes-west` or `both`.
    pub satellite: Option<String>,
}

/// Translate the generic params into the lightning backend's. A pure function
/// so a dropped field is caught by a unit test (see radius.rs for the bug that
/// motivated this pattern).
fn lightning_items_params(p: &ItemsQueryParams) -> lightning::LightningItemsParams {
    lightning::LightningItemsParams {
        bbox: p.bbox.clone(),
        datetime: p.datetime.clone(),
        window: p.window.clone(),
        after: p.after,
        satellite: p.satellite.clone(),
        limit: p.limit,
        f: p.f.clone(),
    }
}

/// GET /edr/collections/:collection_id/items
pub async fn items_handler(
    Extension(state): Extension<Arc<AppState>>,
    Path(collection_id): Path<String>,
    Query(params): Query<ItemsQueryParams>,
) -> Response {
    let observation_source = {
        let config = state.edr_config.read().await;
        let Some((model_config, _)) = config.find_collection(&collection_id) else {
            let exc =
                ExceptionResponse::not_found(format!("Collection not found: {}", collection_id));
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        };
        if !model_config.data_type.is_feature_data() {
            let exc = ExceptionResponse::bad_request(format!(
                "Collection {} does not support /items",
                collection_id
            ));
            let json = serde_json::to_string(&exc).unwrap_or_default();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(json.into())
                .unwrap();
        }
        model_config.observation_source.clone()
    };

    match observation_source.as_deref() {
        Some("lightning") => {
            lightning::lightning_items_handler(
                Extension(state),
                Path(collection_id),
                Query(lightning_items_params(&params)),
            )
            .await
        }
        Some("linear_features") => {
            let trail_params = linear_features::TrailItemsParams {
                bbox: params.bbox.clone(),
                q: params.q.clone(),
                class: params.class.clone(),
                limit: params.limit,
                offset: params.offset,
                f: params.f.clone(),
                conditions: params.conditions.clone(),
            };
            linear_features::trail_items_handler(
                Extension(state),
                Path(collection_id),
                Query(trail_params),
            )
            .await
        }
        // Default to storm_events: preserves exact prior behavior for
        // hail/wind/tornado, which predate `observation_source` being read
        // for anything other than documentation.
        _ => {
            let storm_params = storm_events::StormItemsParams {
                bbox: params.bbox.clone(),
                datetime: params.datetime.clone(),
                limit: params.limit,
                offset: params.offset,
                f: params.f.clone(),
            };
            storm_events::storm_items_handler(
                Extension(state),
                Path(collection_id),
                Query(storm_params),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(qs: &str) -> ItemsQueryParams {
        let uri: axum::http::Uri = format!("/items?{}", qs).parse().unwrap();
        Query::<ItemsQueryParams>::try_from_uri(&uri).unwrap().0
    }

    #[test]
    fn lightning_params_survive_real_query_string_parsing_and_translation() {
        let p = parse("bbox=-106,39,-105,40&datetime=2026-10-07T20:00:00Z/..&limit=50&window=PT5M&after=42&satellite=both");
        let l = lightning_items_params(&p);
        assert_eq!(l.bbox.as_deref(), Some("-106,39,-105,40"));
        assert_eq!(l.datetime.as_deref(), Some("2026-10-07T20:00:00Z/.."));
        assert_eq!(
            (
                l.limit,
                l.window.as_deref(),
                l.after,
                l.satellite.as_deref()
            ),
            (Some(50), Some("PT5M"), Some(42), Some("both"))
        );
    }

    #[test]
    fn omitted_lightning_params_stay_none() {
        let l = lightning_items_params(&parse("bbox=-106,39,-105,40"));
        assert_eq!(
            (l.window, l.after, l.satellite, l.limit),
            (None, None, None, None)
        );
    }
}
