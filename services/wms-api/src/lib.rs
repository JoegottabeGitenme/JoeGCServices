//! WMS/WMTS API service library.
//!
//! This module exposes the internal modules for testing purposes.

pub mod admin;
pub mod capabilities_cache;
pub mod chunk_warming;
pub mod cleanup;
pub mod handlers;
pub mod layer_config;
pub mod memory_pressure;
pub mod metrics;
pub mod model_config;
pub mod rendering;
pub mod startup_validation;
pub mod state;
pub mod validation;
pub mod warming;

// Re-export handlers module for backwards compatibility
// The handlers module is now split into submodules:
// - handlers::wms - WMS handlers
// - handlers::wmts - WMTS handlers
// - handlers::api - REST API handlers
// - handlers::metrics - Health and metrics handlers
// - handlers::validation - Validation handlers
// - handlers::cache - Cache management handlers
// - handlers::benchmarks - Benchmark handlers
// - handlers::common - Shared utilities
