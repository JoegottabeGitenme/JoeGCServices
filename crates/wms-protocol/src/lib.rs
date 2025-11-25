//! OGC WMS and WMTS protocol implementation.
//!
//! Supports:
//! - WMS 1.1.1 and WMS 1.3.0 specifications
//! - WMTS 1.0.0 specification (KVP and RESTful bindings)

pub mod capabilities;
pub mod exceptions;
pub mod getfeatureinfo;
pub mod getmap;
pub mod wmts;

pub use wmts::{
    wmts_exception, GetCapabilitiesRequest, GetFeatureInfoRequest, GetTileRequest,
    WmtsCapabilitiesBuilder, WmtsDimensionInfo, WmtsKvpParams, WmtsLayerInfo, WmtsRequest,
    WmtsRestPath, WmtsStyleInfo,
};
