//! OGC WMS and WMTS protocol implementation.
//!
//! Supports:
//! - WMS 1.1.1 and WMS 1.3.0 specifications
//! - WMTS 1.0.0 specification (KVP and RESTful bindings)

pub mod exceptions;
pub mod getfeatureinfo;
pub mod getmap;
pub mod wmts;

// Re-export GetFeatureInfo types
pub use getfeatureinfo::{
    mercator_to_wgs84, pixel_to_geographic, FeatureInfo, FeatureInfoResponse,
    GetFeatureInfoRequest, InfoFormat, Location,
};

pub use wmts::{
    wmts_exception, GetCapabilitiesRequest, GetTileRequest, WmtsCapabilitiesBuilder,
    WmtsDimensionInfo, WmtsKvpParams, WmtsLayerInfo, WmtsRequest, WmtsRestPath, WmtsStyleInfo,
};
