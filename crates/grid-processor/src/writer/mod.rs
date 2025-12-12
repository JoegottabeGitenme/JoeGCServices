//! Zarr writer for converting grid data to Zarr format.
//!
//! This module is used during ingestion to write grid data
//! in Zarr V3 format with sharding.

mod zarr_writer;

pub use zarr_writer::{ZarrMetadata, ZarrWriteResult, ZarrWriter};
