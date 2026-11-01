//! Integration test: a pyramid level of a lat/lon grid puts its data where the grid does.
//!
//! A level `scale` times coarser than level 0 holds the downsample of blocks of `scale` x `scale`
//! level-0 cells, so reading it at a longitude/latitude must give the downsample of the level-0
//! cells at that longitude/latitude. The level used to be described by level 0's bbox with its own
//! smaller shape, which spaced its cells by the wrong amount (MRMS level 2: 69.99 / 1749 degrees
//! for 0.04) and put each one away from its data.
//!
//! These tests write synthetic grids with the real multiscale writer, read levels back with the
//! real factory, and place what they read the way every consumer does: cell `(col, row)` of a
//! region is at `bbox.min_lon + col * resolution.0`, `bbox.max_lat - row * resolution.1`.

use std::sync::Arc;

use chrono::TimeZone;
use grid_processor::{
    BoundingBox, ChunkCache, DownsampleMethod, GridMetadata, GridProcessor, GridProcessorConfig,
    GridRegion, MultiscaleGridProcessorFactory, MultiscaleMetadata, MultiscaleWriteResult,
    ProjectionType, PyramidConfig, RowOrigin, ZarrGridProcessor, ZarrWriter,
};
use tokio::sync::RwLock;
use zarrs_filesystem::FilesystemStore;

const GROUP: &str = "/grid.zarr";

/// A grid layout to write: where it is, how many cells, how deep the pyramid goes.
#[derive(Clone, Copy)]
struct Layout {
    name: &'static str,
    /// First-to-last cell-centre extent, as the ingester stores it
    bbox: BoundingBox,
    shape: (usize, usize),
    /// Pyramid levels stop when the smaller side would fall below this
    min_dimension: usize,
}

impl Layout {
    /// MRMS: 0.01 degrees, north up, with its cell centres on odd thousandths of a degree. The
    /// real grid is 7000 x 3500 (levels to 3); this is a corner of it, with a cell too many in
    /// each direction so that odd sizes get dropped on the way down:
    /// 2002 x 1001 -> 1001 x 500 -> 500 x 250 -> 250 x 125 (levels 0 to 3).
    fn mrms_like() -> Self {
        Self {
            name: "mrms-like",
            bbox: BoundingBox::new(-129.995, 44.995, -109.985, 54.995),
            shape: (2002, 1001),
            min_dimension: 100,
        }
    }

    /// GFS: 0.25 degrees, 0 to 359.75 east, 90 to -90 south, 721 rows (an odd count). Real size;
    /// the pyramid goes deeper than ingestion's, to level 3 (180 x 90).
    fn gfs_like() -> Self {
        Self {
            name: "gfs-like",
            bbox: BoundingBox::new(0.0, -90.0, 359.75, 90.0),
            shape: (1440, 721),
            min_dimension: 64,
        }
    }

    fn spacing(&self) -> (f64, f64) {
        (
            self.bbox.width() / (self.shape.0 - 1) as f64,
            self.bbox.height() / (self.shape.1 - 1) as f64,
        )
    }

    /// Longitude and latitude of the centre of a block of `size` x `size` level-0 cells whose
    /// first cell is `(col, row)`
    fn block_centre(&self, col: usize, row: usize, size: usize) -> (f64, f64) {
        let (dx, dy) = self.spacing();
        let middle = (size - 1) as f64 / 2.0;
        (
            self.bbox.min_lon + (col as f64 + middle) * dx,
            self.bbox.max_lat - (row as f64 + middle) * dy,
        )
    }
}

/// A written dataset: its files, the metadata the catalog would hold, and a factory over it.
struct Dataset {
    _dir: tempfile::TempDir,
    store: Arc<FilesystemStore>,
    /// What the catalog holds for the dataset's own (level 0) array
    metadata: GridMetadata,
    multiscale: MultiscaleMetadata,
}

impl Dataset {
    fn write(
        layout: Layout,
        data: &[f32],
        method: DownsampleMethod,
        projection: ProjectionType,
        row_origin: RowOrigin,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FilesystemStore::new(dir.path()).unwrap());
        let MultiscaleWriteResult {
            zarr_metadata,
            multiscale_metadata,
            ..
        } = ZarrWriter::new(GridProcessorConfig::default())
            .write_multiscale(
                store.clone(),
                GROUP,
                data,
                layout.shape.0,
                layout.shape.1,
                &layout.bbox,
                "test",
                "FIELD",
                "surface",
                "units",
                chrono::Utc
                    .with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
                    .unwrap(),
                0,
                &PyramidConfig {
                    min_dimension: layout.min_dimension,
                    ..PyramidConfig::default()
                },
                method,
                row_origin,
                projection,
            )
            .unwrap();
        Self {
            _dir: dir,
            store,
            metadata: GridMetadata::from(&zarr_metadata),
            multiscale: multiscale_metadata,
        }
    }

    fn lat_lon(layout: Layout, data: &[f32], method: DownsampleMethod) -> Self {
        Self::write(
            layout,
            data,
            method,
            ProjectionType::Geographic,
            RowOrigin::North,
        )
    }

    fn factory(&self) -> MultiscaleGridProcessorFactory<Arc<FilesystemStore>> {
        MultiscaleGridProcessorFactory::new(
            self.store.clone(),
            GROUP,
            self.multiscale.clone(),
            Arc::new(RwLock::new(ChunkCache::new(256 << 20))),
            GridProcessorConfig::default(),
        )
        .with_dataset_metadata(self.metadata.clone())
    }

    fn levels(&self) -> Vec<u32> {
        self.multiscale.levels.iter().map(|l| l.level).collect()
    }
}

/// `(lon, lat)` of the cells of a region that hold a value above zero and lie in `near`, weighted
/// by the value: where the feature drawn into a zero background is, as the region says it is. A
/// read of a box that crosses the prime meridian returns the whole grid, so `near` picks out one
/// feature.
fn centroid(region: &GridRegion, near: &BoundingBox) -> (f64, f64) {
    let (mut weight, mut lon, mut lat) = (0.0, 0.0, 0.0);
    for (i, &v) in region.data.iter().enumerate() {
        let (col, row) = (i % region.width, i / region.width);
        let cell_lon = region.bbox.min_lon + col as f64 * region.resolution.0;
        let cell_lat = region.bbox.max_lat - row as f64 * region.resolution.1;
        if v > 0.0 && near.contains(cell_lon, cell_lat) {
            weight += v as f64;
            lon += v as f64 * cell_lon;
            lat += v as f64 * cell_lat;
        }
    }
    assert!(weight > 0.0, "the feature is not in the region");
    (lon / weight, lat / weight)
}

/// A zero grid with a square of `size` x `size` cells holding 100, first cell `(col, row)`
fn with_square(layout: Layout, squares: &[(usize, usize, usize)]) -> Vec<f32> {
    let (width, height) = layout.shape;
    let mut data = vec![0.0f32; width * height];
    for &(col, row, size) in squares {
        for r in row..row + size {
            for c in col..col + size {
                data[r * width + c] = 100.0;
            }
        }
    }
    data
}

/// Squares of 8 x 8 cells, aligned to blocks of 8 so that every level up to 3 holds it as whole
/// blocks: the level cells' mean (or max) is the square's own value, and the centre of what a
/// level holds is the square's centre. Near the west and north edges, in the middle, and near the
/// east and south ones (the last whole blocks of the odd-sized grids).
fn three_squares(layout: Layout) -> [(usize, usize, usize); 3] {
    let (width, height) = layout.shape;
    let last_col = (width - 16) / 8 * 8;
    let last_row = (height - 16) / 8 * 8;
    [
        (16, 16, 8),
        ((width / 2) / 8 * 8, (height / 2) / 8 * 8, 8),
        (last_col, last_row, 8),
    ]
}

/// A request box around a point: `half` degrees each way
fn around((lon, lat): (f64, f64), half: f64) -> BoundingBox {
    BoundingBox::new(lon - half, lat - half, lon + half, lat + half)
}

/// Test (1): the feature a level shows is where the feature is, to better than a hundredth of a
/// level cell, at every level, for both layouts and both ways of keeping peaks or means.
#[tokio::test(flavor = "multi_thread")]
async fn a_level_read_places_a_feature_at_its_longitude_and_latitude() {
    for layout in [Layout::mrms_like(), Layout::gfs_like()] {
        for method in [DownsampleMethod::Max, DownsampleMethod::Mean] {
            let squares = three_squares(layout);
            let ds = Dataset::lat_lon(layout, &with_square(layout, &squares), method);
            assert_eq!(ds.levels(), [0, 1, 2, 3], "{}", layout.name);
            let factory = ds.factory();
            let (dx, dy) = layout.spacing();
            // The request box is generous: a degree of MRMS, 8 of GFS (the west square's box
            // crosses the prime meridian, so reads the whole grid). The feature is what is
            // within a few of its own widths of where it is.
            let (half, near) = if layout.name == "gfs-like" {
                (8.0, 3.0)
            } else {
                (1.0, 0.2)
            };

            for &(col, row, size) in &squares {
                let truth = layout.block_centre(col, row, size);
                for level in 0..=3u32 {
                    let region = factory
                        .read_region_at_level(level, &around(truth, half))
                        .await
                        .unwrap();
                    let (lon, lat) = centroid(&region, &around(truth, near));
                    let scale = 2f64.powi(level as i32);
                    let what = format!(
                        "{} {:?} level {level} square at {truth:?}: read {:?}",
                        layout.name,
                        method,
                        (lon, lat)
                    );
                    // a cell of this level is `scale` level-0 cells wide
                    let tolerance = 0.01 * scale * dx.max(dy);
                    assert!((lon - truth.0).abs() < tolerance, "{what}");
                    assert!((lat - truth.1).abs() < tolerance, "{what}");
                }
            }
        }
    }
}

/// The same, for a feature that does not fill whole blocks: one cell. A level cannot say which
/// cell of its block the value came from, but it must place it within the block, which is half a
/// level cell either way of the block's centre, never further.
#[tokio::test(flavor = "multi_thread")]
async fn a_single_cell_is_placed_within_half_a_level_cell_of_where_it_is() {
    for layout in [Layout::mrms_like(), Layout::gfs_like()] {
        let (width, height) = layout.shape;
        // an awkward cell, in no block's middle, near the east and south edges and in the middle
        let cells = [(width - 21, height - 22), (width / 2 + 3, height / 2 + 5)];
        let squares: Vec<_> = cells.iter().map(|&(c, r)| (c, r, 1)).collect();
        let ds = Dataset::lat_lon(
            layout,
            &with_square(layout, &squares),
            DownsampleMethod::Max,
        );
        let factory = ds.factory();
        let (dx, dy) = layout.spacing();
        let (half, near) = if layout.name == "gfs-like" {
            (8.0, 3.0)
        } else {
            (1.0, 0.2)
        };

        for &(col, row, size) in &squares {
            let truth = layout.block_centre(col, row, size);
            for level in 1..=3u32 {
                let region = factory
                    .read_region_at_level(level, &around(truth, half))
                    .await
                    .unwrap();
                let (lon, lat) = centroid(&region, &around(truth, near));
                let cell = 2f64.powi(level as i32);
                assert!(
                    (lon - truth.0).abs() <= 0.5 * cell * dx + 1e-9
                        && (lat - truth.1).abs() <= 0.5 * cell * dy + 1e-9,
                    "{} level {level} cell {:?}: read {:?}, want {truth:?} within half of {} x {}",
                    layout.name,
                    (col, row),
                    (lon, lat),
                    cell * dx,
                    cell * dy,
                );
            }
        }
    }
}

/// The field `col + 3 * row` is affine, so the mean of a block of it is the field at the block's
/// centre, and a level read at a longitude/latitude must say what the level-0 field is there: for
/// EVERY cell the read returns, not one feature. Picking the top-left cell of each block (what
/// category codes use) is the field at the block's first cell, which the level must say too.
#[tokio::test(flavor = "multi_thread")]
async fn every_cell_of_a_level_holds_the_level_0_field_at_the_place_the_read_gives_it() {
    for layout in [Layout::mrms_like(), Layout::gfs_like()] {
        let (width, height) = layout.shape;
        let data: Vec<f32> = (0..height)
            .flat_map(|row| (0..width).map(move |col| (col + 3 * row) as f32))
            .collect();
        let (dx, dy) = layout.spacing();
        let field = |lon: f64, lat: f64| {
            (lon - layout.bbox.min_lon) / dx + 3.0 * (layout.bbox.max_lat - lat) / dy
        };

        for method in [DownsampleMethod::Mean, DownsampleMethod::Nearest] {
            let ds = Dataset::lat_lon(layout, &data, method);
            let factory = ds.factory();
            // the whole of the western half of GFS, all of the MRMS-like grid: edges included
            let request = if layout.name == "gfs-like" {
                BoundingBox::new(0.0, -90.0, 180.0, 90.0)
            } else {
                layout.bbox
            };
            for level in 1..=3u32 {
                let region = factory.read_region_at_level(level, &request).await.unwrap();
                assert!(region.width > 20 && region.height > 20);
                let mut worst = 0.0f64;
                for row in 0..region.height {
                    for col in 0..region.width {
                        let lon = region.bbox.min_lon + col as f64 * region.resolution.0;
                        let lat = region.bbox.max_lat - row as f64 * region.resolution.1;
                        let got = region.data[row * region.width + col] as f64;
                        worst = worst.max((got - field(lon, lat)).abs());
                    }
                }
                // f32 holds these sums exactly; the slack is the position arithmetic
                assert!(
                    worst < 0.01,
                    "{} {method:?} level {level}: a cell is {worst} away from the level-0 field \
                     where the read puts it",
                    layout.name
                );
            }
        }
    }
}

/// A level's resolution is its scale times level 0's, to rounding, and its cells span what
/// downsampling kept
#[tokio::test(flavor = "multi_thread")]
async fn a_levels_resolution_is_its_scale_times_level_0s() {
    for layout in [Layout::mrms_like(), Layout::gfs_like()] {
        let ds = Dataset::lat_lon(layout, &with_square(layout, &[]), DownsampleMethod::Mean);
        let factory = ds.factory();
        let (dx, dy) = layout.spacing();
        for level in 0..=3u32 {
            let scale = 2f64.powi(level as i32);
            let processor = factory.processor_for_level(level).unwrap();
            let (rx, ry) = processor.metadata().resolution();
            assert!(
                (rx - scale * dx).abs() < 1e-9 && (ry - scale * dy).abs() < 1e-9,
                "{} level {level}: {rx} x {ry}, want {} x {}",
                layout.name,
                scale * dx,
                scale * dy
            );
        }
    }
}

/// Test (3): level 0 is what it was. Its metadata is the dataset's own, bit for bit, whether or
/// not the factory was given it, and a read of it equals a plain read of the level-0 array.
#[tokio::test(flavor = "multi_thread")]
async fn level_0_is_the_dataset_itself() {
    for layout in [Layout::mrms_like(), Layout::gfs_like()] {
        let (width, height) = layout.shape;
        let data: Vec<f32> = (0..width * height).map(|i| (i % 977) as f32).collect();
        let ds = Dataset::lat_lon(layout, &data, DownsampleMethod::Mean);

        let guessed = MultiscaleGridProcessorFactory::new(
            ds.store.clone(),
            GROUP,
            ds.multiscale.clone(),
            Arc::new(RwLock::new(ChunkCache::new(256 << 20))),
            GridProcessorConfig::default(),
        );
        for factory in [&guessed, &ds.factory()] {
            let level_0 = factory.processor_for_level(0).unwrap();
            let m = level_0.metadata();
            assert_eq!(m.bbox, ds.metadata.bbox, "{}", layout.name);
            assert_eq!(m.shape, ds.metadata.shape, "{}", layout.name);
            assert_eq!(m.resolution(), ds.metadata.resolution(), "{}", layout.name);
            assert_eq!(m.row_origin, ds.metadata.row_origin);
        }

        let plain = ZarrGridProcessor::with_metadata(
            ds.store.clone(),
            &format!("{GROUP}/0"),
            ds.metadata.clone(),
            Arc::new(RwLock::new(ChunkCache::new(256 << 20))),
            GridProcessorConfig::default(),
        )
        .unwrap();
        let mid = layout.block_centre(width / 2, height / 2, 1);
        for request in [around(mid, 2.0), layout.bbox, around(mid, 0.01)] {
            let want = plain.read_region(&request).await.unwrap();
            let got = ds
                .factory()
                .read_region_at_level(0, &request)
                .await
                .unwrap();
            assert_eq!(
                (got.width, got.height, got.bbox, got.resolution),
                (want.width, want.height, want.bbox, want.resolution),
                "{} {request:?}",
                layout.name
            );
            assert_eq!(got.data, want.data, "{} {request:?}", layout.name);
        }
    }
}

/// A projected grid's levels are placed by index (`level_index`), and a read of one takes the whole
/// grid, so their bbox stays the dataset's
#[tokio::test(flavor = "multi_thread")]
async fn a_projected_levels_bbox_is_still_the_datasets() {
    let layout = Layout {
        name: "projected",
        bbox: BoundingBox::new(-134.09548, 21.138123, -60.917193, 47.842195),
        shape: (1799, 1059),
        min_dimension: 256,
    };
    let (width, height) = layout.shape;
    let data: Vec<f32> = (0..width * height).map(|i| (i % 311) as f32).collect();
    let ds = Dataset::write(
        layout,
        &data,
        DownsampleMethod::Mean,
        ProjectionType::LambertConformal,
        RowOrigin::South,
    );
    assert_eq!(ds.levels(), [0, 1, 2]);
    let factory = ds.factory();
    for (level, info) in ds.multiscale.levels.iter().enumerate() {
        let processor = factory.processor_for_level(level as u32).unwrap();
        assert_eq!(processor.metadata().bbox, layout.bbox, "level {level}");
        assert_eq!(processor.metadata().shape, info.shape, "level {level}");
    }
}
