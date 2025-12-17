//! Zarr writer for converting grid data to Zarr format.
//!
//! This module is used during ingestion to write grid data
//! in Zarr V3 format with sharding and optional multi-resolution pyramids.

mod zarr_writer;

pub use zarr_writer::{MultiscaleWriteResult, ZarrMetadata, ZarrWriteResult, ZarrWriter};
