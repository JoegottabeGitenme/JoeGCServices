//! A window of cells cut out of a projected grid.
//!
//! WHY: HRRR is laid out in projected metres, not degrees, so a longitude/latitude box is not a
//! rectangle of rows and columns: its outline is a curve across the grid. The linear arithmetic a
//! regular lat/lon grid allows (`chunks_for_bbox`, `assemble_region`) puts such a box in the wrong
//! cells, so a projected grid was always read whole and the caller threw most of it away (a 2.5 x
//! 1.5 degree trail view cost the same as the whole of the United States).
//!
//! This module does the projection arithmetic instead. [`Footprint::of`] traces a box's outline
//! through the grid's own projection to find which cells it needs, [`cell_range`] turns that into
//! a padded rectangle of cells at any pyramid level, and a [`NativeWindow`] holds the cells read
//! from it together with where they sit in the grid, so they can be sampled back onto a regular
//! longitude/latitude grid ([`NativeWindow::resample`]) with the same bilinear arithmetic the
//! full-grid reprojection uses.
//!
//! Column and row here are always the grid's own order (column 0 west, row 0 the first row of the
//! array), counted in cells with cell CENTRES at integers, as [`NativeGrid::geo_to_grid`] returns
//! them.

use std::ops::Range;

use crate::projection::{bilinear_interpolate, NativeGrid};
use crate::types::{BoundingBox, GridRegion, PyramidLevel};

/// Cells added on every side of the cells a box needs, so sampling at the box's edge still has
/// neighbours to interpolate with. The lat/lon read adds the same two cells.
pub const WINDOW_PAD_CELLS: usize = 2;

/// Points traced along each side of a box to find its extremes. The side of a lat/lon box is a
/// curve on a projected grid, so its furthest cell can lie between two corners.
const OUTLINE_STEPS: usize = 32;

/// Position of a pyramid level's cell in level-0 cell coordinates, and back.
///
/// A level `scale` times coarser than level 0 averages (or picks from) blocks of `scale` x `scale`
/// level-0 cells, so its cell `k` is centred on level-0 coordinate `k * scale + (scale - 1) / 2`,
/// not `k * scale`: at scale 2 the level's cell 0 sits half a level-0 cell east of level 0's cell
/// 0. Dividing by the scale alone (what the full-grid reprojection did) shifted every sample by
/// `(scale - 1) / (2 * scale)` level cells. Returns the fractional level cell for a level-0
/// position; the identity at scale 1.
pub fn level_index(native_index: f64, scale: f64) -> f64 {
    (native_index - (scale - 1.0) / 2.0) / scale
}

/// Where a longitude/latitude box falls on a projected grid, in level-0 cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    /// Lowest and highest fractional column the box's outline reaches. Not clamped to the grid:
    /// a box partly off it reaches below 0 or past the last cell.
    pub min_col: f64,
    pub max_col: f64,
    /// Lowest and highest fractional row the box's outline reaches.
    pub min_row: f64,
    pub max_row: f64,
    /// Level-0 cells across the box, west to east along its middle, and up it, south to north.
    /// What a caller that was not told an image size can use for one.
    pub cells: (usize, usize),
}

impl Footprint {
    /// Trace `bbox`'s outline through `native`. `None` when no point of it can be placed on the
    /// grid (a projection singularity), which a caller treats as "nothing to read".
    pub fn of(native: &NativeGrid, bbox: &BoundingBox) -> Option<Self> {
        let (mut min_col, mut max_col) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut min_row, mut max_row) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut placed = false;
        let mut take = |lon: f64, lat: f64| {
            let (col, row) = native.geo_to_grid(lon, lat);
            if col.is_finite() && row.is_finite() {
                min_col = min_col.min(col);
                max_col = max_col.max(col);
                min_row = min_row.min(row);
                max_row = max_row.max(row);
                placed = true;
            }
        };
        for step in 0..=OUTLINE_STEPS {
            let t = step as f64 / OUTLINE_STEPS as f64;
            let lon = bbox.min_lon + t * bbox.width();
            let lat = bbox.min_lat + t * bbox.height();
            take(lon, bbox.min_lat);
            take(lon, bbox.max_lat);
            take(bbox.min_lon, lat);
            take(bbox.max_lon, lat);
        }
        if !placed {
            return None;
        }

        // Cells across the middle of the box. Measured along the box and not read off the extremes
        // above, which for a box skewed on the grid would count its corners' spread.
        let (mid_lon, mid_lat) = bbox.center();
        let across = |from: (f64, f64), to: (f64, f64)| {
            let (a, b) = (
                native.geo_to_grid(from.0, from.1),
                native.geo_to_grid(to.0, to.1),
            );
            let d = (b.0 - a.0).hypot(b.1 - a.1);
            d.is_finite().then_some(d)
        };
        let cells_x =
            across((bbox.min_lon, mid_lat), (bbox.max_lon, mid_lat)).unwrap_or(max_col - min_col);
        let cells_y =
            across((mid_lon, bbox.min_lat), (mid_lon, bbox.max_lat)).unwrap_or(max_row - min_row);

        Some(Self {
            min_col,
            max_col,
            min_row,
            max_row,
            cells: (
                cells_x.round().max(1.0) as usize,
                cells_y.round().max(1.0) as usize,
            ),
        })
    }

    /// Whether any point of the box can be sampled on a `width` x `height` grid. A pixel is
    /// sampled between the first and last cell centres, so a box that ends before 0 or starts at
    /// the last cell has nothing to read.
    pub fn overlaps(&self, width: usize, height: usize) -> bool {
        self.overlaps_level(1.0, width, height)
    }

    /// [`Self::overlaps`] for a pyramid level `scale` times coarser, `width` x `height` cells.
    fn overlaps_level(&self, scale: f64, width: usize, height: usize) -> bool {
        level_index(self.max_col, scale) >= 0.0
            && level_index(self.min_col, scale) < width as f64 - 1.0
            && level_index(self.max_row, scale) >= 0.0
            && level_index(self.min_row, scale) < height as f64 - 1.0
    }

    /// The coarsest of `levels` that still has at least `output` cells across the box on both
    /// axes, else level 0. The same rule as `MultiscaleMetadata::optimal_level_for`, but counted
    /// on the projected grid's real cells and not on degrees (which for a Lambert grid are
    /// neither square nor constant).
    pub fn coarsest_level_for(&self, levels: &[PyramidLevel], output: (usize, usize)) -> u32 {
        levels
            .iter()
            .rev()
            .find(|level| {
                (self.cells.0 as f64 / level.scale).ceil() >= output.0 as f64
                    && (self.cells.1 as f64 / level.scale).ceil() >= output.1 as f64
            })
            .map_or(0, |level| level.level)
    }
}

/// The columns and rows of a level (`width` x `height` cells, `scale` level-0 cells per cell)
/// that sampling `footprint` needs, [`WINDOW_PAD_CELLS`] added on each side and clamped to the
/// level. `None` when the footprint misses the level altogether.
pub fn cell_range(
    footprint: &Footprint,
    scale: f64,
    width: usize,
    height: usize,
) -> Option<(Range<usize>, Range<usize>)> {
    if !footprint.overlaps_level(scale, width, height) {
        return None;
    }
    let axis = |min: f64, max: f64, n: usize| {
        // A sample at fractional index x uses cells floor(x) and floor(x) + 1
        let start = (level_index(min, scale).floor() as i64 - WINDOW_PAD_CELLS as i64).max(0);
        let end =
            (level_index(max, scale).floor() as i64 + 2 + WINDOW_PAD_CELLS as i64).min(n as i64);
        (start < end).then_some(start as usize..end as usize)
    };
    Some((
        axis(footprint.min_col, footprint.max_col, width)?,
        axis(footprint.min_row, footprint.max_row, height)?,
    ))
}

/// The cells of one pyramid level of a projected grid that a box needs, with where they sit.
#[derive(Debug, Clone)]
pub struct NativeWindow {
    /// The cells, row-major, `width` x `height`, in the grid's own row order.
    pub data: Vec<f32>,
    pub width: usize,
    pub height: usize,
    /// Column and row, in the level read, of the window's first cell.
    pub col0: usize,
    pub row0: usize,
    /// Size of the whole level the window was cut from.
    pub level_width: usize,
    pub level_height: usize,
    /// Level-0 cells per cell of the level read: 1 at level 0, 2 at level 1, 4 at level 2.
    pub scale: f64,
    /// The grid's projection, to place longitude/latitude on its cells.
    pub native: NativeGrid,
    /// Level-0 cells across and up the box that was asked for (see [`Footprint::cells`]).
    pub cells: (usize, usize),
}

impl NativeWindow {
    /// Sample the window onto a regular `width` x `height` longitude/latitude grid covering
    /// `bbox`, north up, as pixel centres: the same bilinear arithmetic, bounds rule and NaN rule
    /// as the full-grid reprojection, so a pixel has the value it would have had if the whole
    /// grid had been read. Pixels outside the grid, or whose four neighbours are not all present,
    /// are NaN.
    pub fn resample(&self, bbox: &BoundingBox, width: usize, height: usize) -> GridRegion {
        let res_lon = bbox.width() / width as f64;
        let res_lat = bbox.height() / height as f64;
        let mut output = vec![f32::NAN; width * height];

        for out_y in 0..height {
            let lat = bbox.max_lat - (out_y as f64 + 0.5) * res_lat;
            for out_x in 0..width {
                let lon = bbox.min_lon + (out_x as f64 + 0.5) * res_lon;
                let (col, row) = self.native.geo_to_grid(lon, lat);
                let (x, y) = (level_index(col, self.scale), level_index(row, self.scale));

                // The level's own sampling area, between its first and last cell centres
                // (negated so NaN falls out)
                if !(x >= 0.0
                    && x < self.level_width as f64 - 1.0
                    && y >= 0.0
                    && y < self.level_height as f64 - 1.0)
                {
                    continue;
                }
                // ...and the window's: padding makes a miss here a rounding sliver, but a
                // neighbour that was not read must not be replaced by an edge cell's value
                let (wx, wy) = (x - self.col0 as f64, y - self.row0 as f64);
                if wx < 0.0
                    || wy < 0.0
                    || wx >= self.width as f64 - 1.0
                    || wy >= self.height as f64 - 1.0
                {
                    continue;
                }
                output[out_y * width + out_x] =
                    bilinear_interpolate(&self.data, self.width, self.height, wx, wy);
            }
        }

        GridRegion::new(output, width, height, *bbox, (res_lon, res_lat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use projection::LambertConformal;

    use crate::types::ProjectionType;

    fn hrrr() -> NativeGrid {
        NativeGrid::for_model("hrrr", ProjectionType::LambertConformal).unwrap()
    }

    /// The trail view the app sends: 2.5 x 1.5 degrees around Boulder.
    fn trail() -> BoundingBox {
        BoundingBox::new(-106.0, 39.0, -103.5, 40.5)
    }

    #[test]
    fn level_index_is_the_identity_at_native_resolution_and_centres_blocks_above_it() {
        assert_eq!(level_index(123.456, 1.0), 123.456);
        // Level 1 cell k averages level-0 cells 2k and 2k+1: centred on 2k + 0.5
        assert_eq!(level_index(0.5, 2.0), 0.0);
        assert_eq!(level_index(2.5, 2.0), 1.0);
        // Level 2 cell k covers 4k..4k+3: centred on 4k + 1.5
        assert_eq!(level_index(1.5, 4.0), 0.0);
        assert_eq!(level_index(9.5, 4.0), 2.0);
    }

    #[test]
    fn a_trail_box_is_a_few_dozen_cells_of_the_3km_grid() {
        let f = Footprint::of(&hrrr(), &trail()).unwrap();
        // 2.5 deg of longitude at 40N is ~213 km, 1.5 deg of latitude ~167 km: 71 x 56 cells
        assert!((68..=75).contains(&f.cells.0), "{:?}", f.cells);
        assert!((53..=59).contains(&f.cells.1), "{:?}", f.cells);
        // and the covering rectangle is a little larger than that: the outline is skewed
        assert!(f.max_col - f.min_col >= f.cells.0 as f64 - 1.0);
        assert!(f.max_col - f.min_col < f.cells.0 as f64 + 8.0);
        assert!(f.overlaps(1799, 1059));
    }

    #[test]
    fn the_outline_reaches_further_than_the_corners_on_a_curved_edge() {
        // A wide box: the Lambert grid bends its north edge, so the highest row is mid-edge
        let native = hrrr();
        let wide = BoundingBox::new(-125.0, 30.0, -70.0, 48.0);
        let f = Footprint::of(&native, &wide).unwrap();
        let corners = [
            (wide.min_lon, wide.min_lat),
            (wide.max_lon, wide.min_lat),
            (wide.min_lon, wide.max_lat),
            (wide.max_lon, wide.max_lat),
        ]
        .map(|(lon, lat)| native.geo_to_grid(lon, lat));
        let corner_min_row = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min);
        let corner_min_col = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min);
        // the parallels are arcs: the southern edge dips below both of its corners
        assert!(
            f.min_row < corner_min_row - 5.0,
            "{} {}",
            f.min_row,
            corner_min_row
        );
        assert!(f.min_col <= corner_min_col);
    }

    #[test]
    fn a_box_off_the_grid_overlaps_nothing() {
        let native = hrrr();
        for (name, bbox) in [
            ("Europe", BoundingBox::new(0.0, 45.0, 10.0, 50.0)),
            ("Pacific", BoundingBox::new(-170.0, 10.0, -160.0, 20.0)),
            ("Arctic", BoundingBox::new(-100.0, 70.0, -90.0, 75.0)),
        ] {
            let f = Footprint::of(&native, &bbox).unwrap();
            assert!(!f.overlaps(1799, 1059), "{name}: {f:?}");
        }
        // one that straddles the western edge does
        let west = BoundingBox::new(-140.0, 35.0, -125.0, 40.0);
        assert!(Footprint::of(&native, &west).unwrap().overlaps(1799, 1059));
    }

    #[test]
    fn the_cell_range_is_padded_and_clamped_to_the_level() {
        let f = Footprint::of(&hrrr(), &trail()).unwrap();
        let (cols, rows) = cell_range(&f, 1.0, 1799, 1059).unwrap();
        let (full_cols, full_rows) = (cols.clone(), rows.clone());
        assert!(cols.start as f64 <= f.min_col.floor() - WINDOW_PAD_CELLS as f64);
        assert!(cols.end as f64 >= f.max_col.floor() + 2.0 + WINDOW_PAD_CELLS as f64);
        assert!(rows.start as f64 <= f.min_row.floor() - WINDOW_PAD_CELLS as f64);
        assert!(cols.end <= 1799 && rows.end <= 1059);
        // a trail view is a tiny part of the grid
        assert!(cols.len() * rows.len() < 100 * 90, "{cols:?} {rows:?}");

        // a box over the corner of the grid is clamped, not rejected
        let corner = Footprint::of(&hrrr(), &BoundingBox::new(-125.0, 21.5, -120.0, 25.0)).unwrap();
        let (cols, rows) = cell_range(&corner, 1.0, 1799, 1059).unwrap();
        assert!(cols.start == 0 || rows.start == 0, "{cols:?} {rows:?}");

        // the same footprint at level 2 (449 x 264) needs a quarter of the cells across
        let (c2, r2) = cell_range(&f, 4.0, 449, 264).unwrap();
        assert!(c2.len() * 2 < full_cols.len(), "{c2:?} vs {full_cols:?}");
        assert!(r2.len() * 2 < full_rows.len(), "{r2:?} vs {full_rows:?}");

        // off the grid: nothing to read
        let off = Footprint::of(&hrrr(), &BoundingBox::new(0.0, 45.0, 10.0, 50.0)).unwrap();
        assert!(cell_range(&off, 1.0, 1799, 1059).is_none());
    }

    #[test]
    fn the_level_is_the_coarsest_that_still_has_a_cell_per_pixel() {
        let levels = [
            PyramidLevel::new(0, "0", (1799, 1059), 1.0, (512, 512)),
            PyramidLevel::new(1, "1", (899, 529), 2.0, (512, 512)),
            PyramidLevel::new(2, "2", (449, 264), 4.0, (512, 512)),
        ];
        let f = Footprint::of(&hrrr(), &trail()).unwrap();
        // ~71 x 56 cells across the box
        assert_eq!(f.coarsest_level_for(&levels, (250, 150)), 0);
        assert_eq!(f.coarsest_level_for(&levels, (71, 56)), 0);
        assert_eq!(f.coarsest_level_for(&levels, (35, 28)), 1);
        assert_eq!(f.coarsest_level_for(&levels, (18, 14)), 2);
        assert_eq!(f.coarsest_level_for(&levels, (8, 8)), 2);
        // both axes must have enough: a tall, narrow image holds the level back
        assert_eq!(f.coarsest_level_for(&levels, (18, 56)), 0);
        // no levels beyond 0 known
        assert_eq!(f.coarsest_level_for(&levels[..1], (8, 8)), 0);
    }

    /// An in-memory window over an affine field, for the sampling arithmetic alone.
    fn affine_window(
        native: NativeGrid,
        scale: f64,
        origin: (usize, usize),
        size: (usize, usize),
        level: (usize, usize),
    ) -> NativeWindow {
        // value = 3 * (level-0 column) + 7 * (level-0 row) at the cell's centre: affine in the
        // grid's own coordinates, so a bilinear sample is exact wherever it lands
        let centre = |k: usize| k as f64 * scale + (scale - 1.0) / 2.0;
        let data = (0..size.1)
            .flat_map(|j| {
                (0..size.0)
                    .map(move |i| (3.0 * centre(origin.0 + i) + 7.0 * centre(origin.1 + j)) as f32)
            })
            .collect();
        NativeWindow {
            data,
            width: size.0,
            height: size.1,
            col0: origin.0,
            row0: origin.1,
            level_width: level.0,
            level_height: level.1,
            scale,
            native,
            cells: (1, 1),
        }
    }

    #[test]
    fn resampling_a_window_gives_the_field_at_each_pixel_centre_at_every_level() {
        let proj = LambertConformal::hrrr();
        let bbox = trail();
        let (w, h) = (40, 24);
        for (scale, level) in [(1.0, (1799, 1059)), (2.0, (899, 529)), (4.0, (449, 264))] {
            let f = Footprint::of(&hrrr(), &bbox).unwrap();
            let (cols, rows) = cell_range(&f, scale, level.0, level.1).unwrap();
            let win = affine_window(
                hrrr(),
                scale,
                (cols.start, rows.start),
                (cols.len(), rows.len()),
                level,
            );
            let out = win.resample(&bbox, w, h);
            assert_eq!((out.width, out.height, out.bbox), (w, h, bbox));

            for y in 0..h {
                for x in 0..w {
                    let lon = bbox.min_lon + (x as f64 + 0.5) * bbox.width() / w as f64;
                    let lat = bbox.max_lat - (y as f64 + 0.5) * bbox.height() / h as f64;
                    let (i, j) = proj.geo_to_grid(lat, lon);
                    let want = 3.0 * i + 7.0 * j;
                    let got = out.data[y * w + x] as f64;
                    assert!(
                        (got - want).abs() < 0.05,
                        "scale {scale} pixel ({x},{y}): got {got}, want {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn pixels_off_the_grid_are_nan_and_those_on_it_are_not() {
        // A box straddling the grid's western edge: the western part is off the grid
        let bbox = BoundingBox::new(-140.0, 35.0, -125.0, 40.0);
        let native = hrrr();
        let f = Footprint::of(&native, &bbox).unwrap();
        let (cols, rows) = cell_range(&f, 1.0, 1799, 1059).unwrap();
        let win = affine_window(
            native,
            1.0,
            (cols.start, rows.start),
            (cols.len(), rows.len()),
            (1799, 1059),
        );
        let out = win.resample(&bbox, 60, 20);
        assert!(out.data[0].is_nan(), "far west of the grid");
        let (nan, ok) =
            out.data.iter().fold(
                (0, 0),
                |(n, k), v| {
                    if v.is_nan() {
                        (n + 1, k)
                    } else {
                        (n, k + 1)
                    }
                },
            );
        assert!(nan > 0 && ok > 0, "nan {nan}, on-grid {ok}");
        // every row is NaN up to the edge and valid after it: a straight western boundary
        for y in 0..20 {
            let row = &out.data[y * 60..(y + 1) * 60];
            let first_valid = row.iter().position(|v| !v.is_nan()).unwrap_or(60);
            assert!(row[first_valid..].iter().all(|v| !v.is_nan()), "row {y}");
        }
    }

    #[test]
    fn a_pixel_whose_neighbours_were_not_read_is_nan_not_an_edge_value() {
        // A window that is deliberately too small for the box: pixels past it stay NaN
        let bbox = trail();
        let native = hrrr();
        let f = Footprint::of(&native, &bbox).unwrap();
        let (cols, rows) = cell_range(&f, 1.0, 1799, 1059).unwrap();
        let half = cols.len() / 2;
        let win = affine_window(
            native,
            1.0,
            (cols.start, rows.start),
            (half, rows.len()),
            (1799, 1059),
        );
        let out = win.resample(&bbox, 40, 24);
        let east = (0..24).all(|y| out.data[y * 40 + 39].is_nan());
        let west = (0..24).any(|y| !out.data[y * 40].is_nan());
        assert!(east && west);
    }
}
