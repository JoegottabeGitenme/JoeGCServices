//! Core data types for the rendering module.

/// Grid data extracted from either GRIB2, NetCDF, or Zarr files
pub struct GridData {
    pub data: Vec<f32>,
    pub width: usize,
    pub height: usize,
    /// Actual bounding box of the returned data (may be subset of full grid)
    pub bbox: Option<[f32; 4]>,
    /// For GOES data: dynamic geostationary projection parameters
    pub goes_projection: Option<GoesProjectionParams>,
    /// Whether the underlying grid uses 0-360 longitude convention (like GFS).
    /// This is needed for proper coordinate handling even when bbox is a partial region.
    pub grid_uses_360: bool,
}

/// Dynamic GOES projection parameters extracted from NetCDF file
#[derive(Clone, Debug)]
pub struct GoesProjectionParams {
    pub x_origin: f64,
    pub y_origin: f64,
    pub dx: f64,
    pub dy: f64,
    pub perspective_point_height: f64,
    pub semi_major_axis: f64,
    pub semi_minor_axis: f64,
    pub longitude_origin: f64,
}
