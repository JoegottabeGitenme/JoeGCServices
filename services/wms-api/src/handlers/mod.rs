//! HTTP request handlers for WMS, WMTS, and API endpoints.
//!
//! This module is organized into submodules:
//! - `wms`: WMS GetCapabilities, GetMap, GetFeatureInfo handlers
//! - `wmts`: WMTS GetCapabilities, GetTile handlers (KVP, REST, XYZ)
//! - `api`: REST API handlers (forecast times, parameters, ingestion events)
//! - `metrics`: Health checks, Prometheus metrics, and monitoring
//! - `validation`: WMS/WMTS validation handlers
//! - `cache`: Cache management and config reload handlers
//! - `benchmarks`: Load test results and benchmark handlers
//! - `docs`: API documentation (Swagger UI, OpenAPI spec)
//! - `common`: Shared utilities (exceptions, coordinate conversion, XML helpers)

pub mod api;
pub mod benchmarks;
pub mod cache;
pub mod common;
pub mod docs;
pub mod metrics;
pub mod validation;
pub mod wms;
pub mod wmts;

// Re-export all public types and handlers for backwards compatibility
pub use common::{
    generate_placeholder_image, get_styles_xml_from_file, get_wmts_styles_xml_from_file,
    mercator_to_wgs84, parse_iso8601_timestamp, wms_exception, wmts_exception, write_chunk,
    DimensionParams, WmtsDimensionParams,
};

pub use wms::{wms_handler, WmsParams};

pub use wmts::{wmts_kvp_handler, wmts_rest_handler, xyz_tile_handler, WmtsKvpParams};

pub use api::{
    forecast_times_handler, ingestion_events_handler, parameters_handler, ForecastTimesResponse,
    IngestionEvent, ParametersResponse,
};

pub use metrics::{
    api_metrics_handler, container_stats_handler, grid_processor_stats_handler, health_handler,
    metrics_handler, ready_handler, storage_stats_handler, tile_heatmap_clear_handler,
    tile_heatmap_handler,
};

pub use validation::{
    startup_validation_run_handler, validation_run_handler, validation_status_handler,
};

pub use cache::{
    cache_clear_handler, cache_list_handler, config_handler, config_reload_handler,
    config_reload_layers_handler,
};

pub use benchmarks::{
    benchmarks_handler, criterion_benchmarks_handler, loadtest_dashboard_handler,
    loadtest_file_handler, loadtest_files_handler, loadtest_results_handler,
};

pub use docs::{openapi_json_handler, openapi_yaml_handler, swagger_ui_handler};
