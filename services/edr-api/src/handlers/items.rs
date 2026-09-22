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

use crate::handlers::{linear_features, storm_events};
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
        Some("linear_features") => {
            let trail_params = linear_features::TrailItemsParams {
                bbox: params.bbox.clone(),
                q: params.q.clone(),
                class: params.class.clone(),
                limit: params.limit,
                offset: params.offset,
                f: params.f.clone(),
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
