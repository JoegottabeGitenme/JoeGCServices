//! Load testing framework for Weather WMS/WMTS service.
//!
//! This crate provides tools to:
//! - Generate realistic WMTS tile request patterns
//! - Execute load tests with controlled concurrency
//! - Collect detailed performance metrics
//! - Output results in multiple formats (console, JSON, CSV)

pub mod config;
pub mod generator;
pub mod metrics;
pub mod report;
pub mod runner;
pub mod wms_client;

pub use config::{BBox, LayerConfig, TestConfig, TileSelection};
pub use generator::TileGenerator;
pub use metrics::{MetricsCollector, TestResults};
pub use report::ResultsReport;
pub use runner::{LoadRunner, RequestResult};
