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
//! - `common`: Shared utilities (exceptions, coordinate conversion, XML helpers)

pub mod common;
pub mod wms;
pub mod wmts;
pub mod api;
pub mod metrics;
pub mod validation;
pub mod cache;
pub mod benchmarks;

// Re-export all public types and handlers for backwards compatibility
pub use common::{
    DimensionParams,
    WmtsDimensionParams,
    wms_exception,
    wmts_exception,
    mercator_to_wgs84,
    parse_iso8601_timestamp,
    get_styles_xml_from_file,
    get_wmts_styles_xml_from_file,
    generate_placeholder_image,
    write_chunk,
};

pub use wms::{
    WmsParams,
    wms_handler,
};

pub use wmts::{
    WmtsKvpParams,
    wmts_kvp_handler,
    wmts_rest_handler,
    xyz_tile_handler,
};

pub use api::{
    ForecastTimesResponse,
    ParametersResponse,
    IngestionEvent,
    forecast_times_handler,
    parameters_handler,
    ingestion_events_handler,
};

pub use metrics::{
    health_handler,
    ready_handler,
    metrics_handler,
    api_metrics_handler,
    storage_stats_handler,
    container_stats_handler,
    grid_processor_stats_handler,
    tile_heatmap_handler,
    tile_heatmap_clear_handler,
};

pub use validation::{
    validation_status_handler,
    validation_run_handler,
    startup_validation_run_handler,
};

pub use cache::{
    cache_clear_handler,
    cache_list_handler,
    config_reload_layers_handler,
    config_reload_handler,
    config_handler,
};

pub use benchmarks::{
    loadtest_results_handler,
    loadtest_files_handler,
    loadtest_file_handler,
    criterion_benchmarks_handler,
    benchmarks_handler,
    loadtest_dashboard_handler,
};
