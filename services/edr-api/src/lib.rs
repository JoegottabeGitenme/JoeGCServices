//! EDR API Service Library
//!
//! This crate provides the HTTP server implementation for the
//! OGC API - Environmental Data Retrieval specification.

pub mod config;
pub mod content_negotiation;
pub mod handlers;
pub mod limits;
pub mod location_cache;
pub mod state;
