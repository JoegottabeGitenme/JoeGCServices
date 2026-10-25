//! Locating a longitude/latitude on a grid that is NOT laid out in degrees.
//!
//! HRRR, NDFD and NBM-CONUS are Lambert conformal grids, NBM-Alaska is polar stereographic and the
//! small NBM regions are Mercator. Their rows and columns follow projected metres, so a line of
//! constant latitude is a curve across the grid. Mapping a point by interpolating linearly between
//! the grid's geographic bounding-box edges (what a regular lat/lon grid allows) puts it in the
//! wrong cell: for HRRR the answer for Denver came from a cell about 470 km away, because the
//! corner-based longitude/latitude envelope neither bounds the grid (its top-middle cell is at
//! 52.6 N, the envelope stops at 47.8 N) nor maps linearly onto it.
//!
//! [`NativeGrid`] does the projection arithmetic instead. It is selected from the model a grid was
//! ingested for, because the stored array carries only the projection *kind*, not its parameters;
//! the same table in `ingestion` decides the projection when the grid is written.

use projection::{LambertConformal, Mercator, PolarStereographic};

use crate::types::ProjectionType;

/// A grid's own projection, able to turn a longitude/latitude into fractional column/row indices.
#[derive(Debug, Clone)]
pub enum NativeGrid {
    Lambert(LambertConformal),
    Polar(PolarStereographic),
    Mercator(Mercator),
}

impl NativeGrid {
    /// The projection of `model`'s grid, if `kind` is a projected type this knows how to invert.
    ///
    /// `None` for geographic grids (a linear map is exactly right for those) and for a model this
    /// table does not list (the caller then keeps the old linear behaviour rather than failing).
    pub fn for_model(model: &str, kind: ProjectionType) -> Option<Self> {
        match (kind, model) {
            // `hrrr-subh` (15-minute output) is on the same grid as `hrrr`
            (ProjectionType::LambertConformal, "hrrr" | "hrrr-subh") => {
                Some(Self::Lambert(LambertConformal::hrrr()))
            }
            (ProjectionType::LambertConformal, "ndfd") => {
                Some(Self::Lambert(LambertConformal::ndfd()))
            }
            (ProjectionType::LambertConformal, "nbm-conus") => {
                Some(Self::Lambert(LambertConformal::nbm_conus()))
            }
            (ProjectionType::PolarStereographic, "nbm-alaska") => {
                Some(Self::Polar(PolarStereographic::nbm_alaska()))
            }
            (ProjectionType::Mercator, "nbm-hawaii") => {
                Some(Self::Mercator(Mercator::nbm_hawaii()))
            }
            (ProjectionType::Mercator, "nbm-puertorico") => {
                Some(Self::Mercator(Mercator::nbm_puertorico()))
            }
            (ProjectionType::Mercator, "nbm-guam") => Some(Self::Mercator(Mercator::nbm_guam())),
            _ => None,
        }
    }

    /// Fractional `(column, row)` of a point, counted from the grid's first point: `(0, 0)` is the
    /// centre of the first cell, west to east and south to north, as the GRIB2 scan order stores
    /// it. Not bounds-checked.
    pub fn geo_to_grid(&self, lon: f64, lat: f64) -> (f64, f64) {
        match self {
            Self::Lambert(p) => p.geo_to_grid(lat, lon),
            Self::Polar(p) => p.geo_to_grid(lat, lon),
            Self::Mercator(p) => p.geo_to_grid(lat, lon),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(model, kind, columns, rows, cells)` where each cell is `(col, row, lat, lon)`: the
    /// longitude/latitude the DATA FILE ITSELF assigns to that cell (`wgrib2 -ijlat`, 0-based
    /// here), read from real NCEP messages of 2026-10-11 -- HRRR `wrfsubhf01` and the NBM
    /// `blend.t18z.core.f001.{co,ak,hi,pr,gu}` 2 m temperature -- not computed by this code.
    /// Corners, edge midpoints and interior cells of each grid.
    type Truth = (
        &'static str,
        ProjectionType,
        usize,
        usize,
        &'static [(usize, usize, f64, f64)],
    );

    const TRUTH: &[Truth] = &[
        (
            "hrrr",
            ProjectionType::LambertConformal,
            1799,
            1059,
            &[
                (0, 0, 21.138123, 237.280472),
                (1798, 0, 21.140547, 287.710282),
                (0, 1058, 47.838623, 225.90452),
                (1798, 1058, 47.842195, 299.082807),
                (898, 528, 38.470261, 262.459567),
                (898, 0, 24.36457, 262.46624),
                (898, 1058, 52.615645, 262.449558),
                (448, 793, 44.477121, 245.439016),
                (1348, 263, 30.42687, 276.461898),
            ],
        ),
        (
            "nbm-conus",
            ProjectionType::LambertConformal,
            2345,
            1597,
            &[
                (0, 0, 19.229, 233.7234),
                (2344, 0, 20.331746, 290.791772),
                (0, 1596, 53.050563, 221.626732),
                (2344, 1596, 54.372791, 300.95781),
                (1171, 797, 40.594194, 261.616027),
                (1171, 0, 22.599913, 262.107694),
                (1171, 1596, 57.055227, 260.92093),
                (585, 1196, 47.959129, 242.96742),
                (1757, 398, 31.212567, 277.467169),
            ],
        ),
        (
            "nbm-alaska",
            ProjectionType::PolarStereographic,
            1649,
            1105,
            &[
                (0, 0, 40.53, 181.429),
                (1648, 0, 41.739537, 235.419362),
                (0, 1104, 61.399756, 150.190869),
                (1648, 1104, 63.97579, 266.31049),
                (823, 551, 60.099534, 206.936147),
                (823, 0, 45.919834, 207.979734),
                (823, 1104, 75.301553, 203.646945),
                (411, 827, 64.116144, 179.271136),
                (1235, 275, 51.69546, 224.827644),
            ],
        ),
        (
            "nbm-hawaii",
            ProjectionType::Mercator,
            625,
            561,
            &[
                (0, 0, 14.3515, 195.0305),
                (624, 0, 14.3515, 209.959825),
                (0, 560, 26.860432, 195.0305),
                (624, 560, 26.860432, 209.959825),
                (311, 279, 20.712638, 202.471237),
                (155, 419, 23.811985, 198.738906),
                (467, 139, 17.54859, 206.203569),
            ],
        ),
        (
            "nbm-puertorico",
            ProjectionType::Mercator,
            339,
            225,
            &[
                (0, 0, 16.9775, 291.9722),
                (338, 224, 19.522084, 296.015559),
                (168, 111, 18.24309, 293.981917),
                (83, 167, 18.878148, 292.965096),
                (253, 55, 17.605703, 294.998738),
            ],
        ),
        (
            "nbm-guam",
            ProjectionType::Mercator,
            193,
            193,
            &[
                (0, 0, 12.349884, 143.686538),
                (192, 192, 16.794486, 148.280176),
                (95, 95, 14.560241, 145.959432),
                (47, 143, 15.668903, 144.811022),
                (143, 47, 13.445978, 147.107842),
            ],
        ),
    ];

    /// Longitude as a user would send it: -180..180
    fn west_positive(lon: f64) -> f64 {
        if lon > 180.0 {
            lon - 360.0
        } else {
            lon
        }
    }

    #[test]
    fn every_projected_grid_puts_the_files_own_coordinates_on_the_files_own_cells() {
        for (model, kind, nx, ny, cells) in TRUTH {
            let grid = NativeGrid::for_model(model, *kind)
                .unwrap_or_else(|| panic!("{model} has no native grid"));
            for &(col, row, lat, lon) in *cells {
                // the same point given as 0-360 (the files) and as -180..180 (callers)
                for lon in [lon, west_positive(lon)] {
                    let (i, j) = grid.geo_to_grid(lon, lat);
                    assert!(
                        (i - col as f64).abs() < 0.02 && (j - row as f64).abs() < 0.02,
                        "{model} ({lat},{lon}) is cell ({col},{row}) in the file but the projection \
                         says ({i:.3},{j:.3}) of {nx}x{ny}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_old_linear_envelope_mapping_was_hundreds_of_km_out() {
        // What `point_to_grid` did for every projected grid: interpolate linearly across the
        // longitude/latitude envelope of the grid's corners. Kept as a measured baseline so the
        // test above is known to be able to fail: the HRRR cell at the top middle of the grid
        // (898, 1058) lies at 52.6 N, beyond the envelope's 47.8 N.
        let (min_lon, max_lon, min_lat, max_lat) = (-134.09548, -60.917193, 21.138123, 47.842195);
        let (nx, ny) = (1799.0, 1059.0);
        let linear = |lon: f64, lat: f64| {
            (
                (lon - min_lon) / ((max_lon - min_lon) / nx),
                (lat - min_lat) / ((max_lat - min_lat) / ny),
            )
        };
        // Wichita-ish grid centre (898, 528) at 38.47 N, -97.54 E
        let (i, j) = linear(-97.540433, 38.470261);
        let km = ((i - 898.0).hypot(j - 528.0)) * 3.0;
        assert!(
            km > 400.0,
            "the linear mapping is {km:.0} km off at the grid centre"
        );
        let top = NativeGrid::for_model("hrrr", ProjectionType::LambertConformal).unwrap();
        let (i, j) = top.geo_to_grid(-97.550442, 52.615645);
        assert!(
            (i - 898.0).abs() < 0.02 && (j - 1058.0).abs() < 0.02,
            "({i},{j})"
        );
    }

    #[test]
    fn geographic_grids_and_unknown_models_have_no_native_grid() {
        assert!(NativeGrid::for_model("gfs", ProjectionType::Geographic).is_none());
        assert!(NativeGrid::for_model("hrrr", ProjectionType::Geographic).is_none());
        // a projected kind for a model this table does not list
        assert!(NativeGrid::for_model("mystery", ProjectionType::LambertConformal).is_none());
        // the projection kind must agree with the model's grid
        assert!(NativeGrid::for_model("nbm-alaska", ProjectionType::LambertConformal).is_none());
    }

    #[test]
    fn hrrr_subh_shares_the_hrrr_grid() {
        let a = NativeGrid::for_model("hrrr", ProjectionType::LambertConformal).unwrap();
        let b = NativeGrid::for_model("hrrr-subh", ProjectionType::LambertConformal).unwrap();
        assert_eq!(a.geo_to_grid(-104.99, 39.74), b.geo_to_grid(-104.99, 39.74));
    }
}
