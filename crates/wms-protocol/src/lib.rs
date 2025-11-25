//! OGC WMS and WMTS protocol implementation.
//!
//! Supports:
//! - WMS 1.1.1 and WMS 1.3.0 specifications
//! - WMTS 1.0.0 specification (KVP and RESTful bindings)

pub mod capabilities;
pub mod getmap;
pub mod getfeatureinfo;
pub mod exceptions;
pub mod wmts;

pub use wmts::{
    WmtsRequest, WmtsKvpParams, WmtsRestPath,
    GetTileRequest, GetCapabilitiesRequest, GetFeatureInfoRequest,
    WmtsCapabilitiesBuilder, WmtsLayerInfo, WmtsStyleInfo, WmtsDimensionInfo,
    wmts_exception,
};
