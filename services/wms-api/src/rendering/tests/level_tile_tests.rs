//! Tests that a tile drawn from a pyramid level puts data where a tile drawn from level 0 does.
//!
//! Low-zoom tiles of a lat/lon layer (MRMS, GFS, ...) are read from an overview level
//! (`MultiscaleGridProcessorFactory::read_region_for_output`, level chosen by the tile size) and
//! resampled onto the tile like any region. A level used to be described by level 0's bbox with
//! its own smaller shape, so the cells were drawn up to `(scale - 1) / 2` native cells (about
//! 1.5 km at MRMS level 2) from their data and the error grew toward the east and south.
//!
//! The grids here are synthetic, written with the real multiscale writer and read back with the
//! real factory; the region becomes a tile through the same glue and resampling as
//! `load_grid_data_from_zarr` -> `resample_grid_for_bbox_with_proj`. The tile compared is the grid
//! the renderer colours, which is where a feature lands in the PNG. The same comparison runs
//! through the real loader in `a_loader_tile_from_a_level_matches_the_tile_from_level_0`
//! (Docker), still on synthetic MRMS-like and GFS-like pyramids. No real ingested (GRIB2) data is
//! compared.

use std::sync::Arc;

use chrono::TimeZone;
use grid_processor::{
    BoundingBox, ChunkCache, DownsampleMethod, GridProcessorConfig, GridProcessorFactory,
    MinioConfig, MultiscaleGridProcessorFactory, MultiscaleWriteResult, ProjectionType,
    PyramidConfig, RowOrigin, ZarrWriter,
};
use storage::{CatalogEntry, ObjectStorage, ObjectStorageConfig};
use test_utils::containers::TestInfrastructure;
use tokio::sync::RwLock;
use zarrs_filesystem::FilesystemStore;

use crate::rendering::loaders::load_grid_data_from_zarr;
use crate::rendering::resampling::resample_grid_for_bbox_with_proj;

const GROUP: &str = "/grid.zarr";

/// A grid layout: where it is, how many cells, how deep its pyramid goes.
#[derive(Clone, Copy)]
struct Layout {
    model: &'static str,
    /// First-to-last cell-centre extent, as the ingester stores it
    bbox: BoundingBox,
    shape: (usize, usize),
    min_dimension: usize,
}

impl Layout {
    /// MRMS: 0.01 degrees, north up. A corner of the real 7000 x 3500 grid, with a cell too many
    /// in each direction so that odd sizes are dropped on the way down:
    /// 2002 x 1001 -> 1001 x 500 -> 500 x 250 -> 250 x 125.
    fn mrms_like() -> Self {
        Self {
            model: "mrms",
            bbox: BoundingBox::new(-129.995, 44.995, -109.985, 54.995),
            shape: (2002, 1001),
            min_dimension: 100,
        }
    }

    /// GFS: 0.25 degrees, 0 to 359.75 east, 90 to -90 south. Real size; the pyramid goes to level
    /// 3 (180 x 90) so that a whole-globe tile can come from level 2.
    fn gfs_like() -> Self {
        Self {
            model: "gfs",
            bbox: BoundingBox::new(0.0, -90.0, 359.75, 90.0),
            shape: (1440, 721),
            min_dimension: 64,
        }
    }

    fn uses_360(&self) -> bool {
        self.bbox.min_lon >= 0.0 && self.bbox.max_lon > 180.0
    }

    /// Spacing in degrees, first-to-last cell centre over the gaps between cells
    fn spacing(&self) -> (f64, f64) {
        (
            self.bbox.width() / (self.shape.0 - 1) as f64,
            self.bbox.height() / (self.shape.1 - 1) as f64,
        )
    }

    /// Longitude and latitude of fractional cell `(col, row)` of level 0
    fn lon_lat(&self, (col, row): (f64, f64)) -> (f64, f64) {
        let (dx, dy) = self.spacing();
        (self.bbox.min_lon + col * dx, self.bbox.max_lat - row * dy)
    }

    /// A zero grid with a smooth round feature of height 100 and width `sigma` cells at each
    /// fractional cell `(col, row)` of `peaks`, which wraps round the end of a global grid.
    ///
    /// Each peak is on the middle of an 8 x 8 block of cells (`8 m + 3.5`) or, round the end of
    /// a global grid, on the boundary between two blocks. A block-aligned level holds the feature
    /// as the same shape sampled symmetrically about the peak, whether it keeps means or maxima,
    /// so where the feature is drawn stays the peak's whatever level it comes from. A feature
    /// this smooth is also drawn into a tile of a few cells to the pixel without aliasing, so its
    /// centre of mass in the tile can be placed to a few hundredths of a pixel.
    fn with_peaks(&self, peaks: &[(f64, f64)], sigma: f64) -> Vec<f32> {
        let (width, height) = self.shape;
        let wraps = self.uses_360();
        let mut data = vec![0.0f32; width * height];
        for &(peak_col, peak_row) in peaks {
            for row in 0..height {
                for col in 0..width {
                    let mut across = (col as f64 - peak_col).abs();
                    if wraps {
                        across = across.min(width as f64 - across);
                    }
                    let up = row as f64 - peak_row;
                    let r2 = (across * across + up * up) / (sigma * sigma);
                    if r2 < 40.0 {
                        data[row * width + col] += 100.0 * (-r2 / 2.0).exp() as f32;
                    }
                }
            }
        }
        data
    }

    /// Level-0 cells across a tile of `bbox` per pixel of a tile `width` pixels wide
    fn cells_per_pixel(&self, bbox: [f32; 4], width: usize) -> f64 {
        (bbox[2] - bbox[0]) as f64 / self.spacing().0 / width as f64
    }
}

struct Pyramid {
    _dir: tempfile::TempDir,
    factory: MultiscaleGridProcessorFactory<Arc<FilesystemStore>>,
    layout: Layout,
}

impl Pyramid {
    fn write(layout: Layout, data: &[f32], method: DownsampleMethod) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FilesystemStore::new(dir.path()).unwrap());
        let MultiscaleWriteResult {
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
                layout.model,
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
                RowOrigin::North,
                ProjectionType::Geographic,
            )
            .unwrap();
        let factory = MultiscaleGridProcessorFactory::new(
            store,
            GROUP,
            multiscale_metadata,
            Arc::new(RwLock::new(ChunkCache::new(256 << 20))),
            GridProcessorConfig::default(),
        );
        Self {
            _dir: dir,
            factory,
            layout,
        }
    }

    /// The tile `bbox` as `width` x `height` pixels, from the level the tile size picks
    /// (`None`: from `level`), and the level it came from. What `load_grid_data_from_zarr` does
    /// with a region, then what `render_weather_data_with_time` does with the grid.
    async fn tile(
        &self,
        bbox: [f32; 4],
        (width, height): (usize, usize),
        level: Option<u32>,
        use_mercator: bool,
    ) -> (Vec<f32>, u32) {
        let read_bbox = BoundingBox::new(
            bbox[0] as f64,
            bbox[1] as f64,
            bbox[2] as f64,
            bbox[3] as f64,
        );
        let (region, used) = match level {
            None => self
                .factory
                .read_region_for_output(&read_bbox, (width, height))
                .await
                .unwrap(),
            Some(level) => (
                self.factory
                    .read_region_at_level(level, &read_bbox)
                    .await
                    .unwrap(),
                level,
            ),
        };
        let data_bounds = [
            region.bbox.min_lon as f32,
            region.bbox.min_lat as f32,
            region.bbox.max_lon as f32,
            region.bbox.max_lat as f32,
        ];
        let tile = resample_grid_for_bbox_with_proj(
            &region.data,
            region.width,
            region.height,
            width,
            height,
            bbox,
            data_bounds,
            use_mercator,
            self.layout.model,
            None,
            self.layout.uses_360(),
        );
        (tile, used)
    }
}

/// Pixel `(x, y)` of the centre of mass of the tile within `window` pixels of `around` (a lone
/// feature in a zero background), `None` when there is nothing there
fn centroid(tile: &[f32], width: usize, around: (f64, f64), window: f64) -> Option<(f64, f64)> {
    let (mut weight, mut x, mut y) = (0.0, 0.0, 0.0);
    for (i, &v) in tile.iter().enumerate() {
        let (px, py) = ((i % width) as f64, (i / width) as f64);
        if v > 0.0 && (px - around.0).abs() <= window && (py - around.1).abs() <= window {
            weight += v as f64;
            x += v as f64 * px;
            y += v as f64 * py;
        }
    }
    (weight > 0.0).then(|| (x / weight, y / weight))
}

/// Where a longitude/latitude falls in a tile of `bbox`, in pixels (pixel centres at x + 0.5)
fn pixel_of(
    bbox: [f32; 4],
    size: (usize, usize),
    (lon, lat): (f64, f64),
    mercator: bool,
) -> (f64, f64) {
    let merc = |lat: f64| crate::rendering::resampling::lat_to_mercator_y(lat);
    let x = (lon - bbox[0] as f64) / (bbox[2] - bbox[0]) as f64 * size.0 as f64 - 0.5;
    let y = if mercator {
        (merc(bbox[3] as f64) - merc(lat)) / (merc(bbox[3] as f64) - merc(bbox[1] as f64))
    } else {
        (bbox[3] as f64 - lat) / (bbox[3] - bbox[1]) as f64
    } * size.1 as f64
        - 0.5;
    (x, y)
}

/// A tile to draw: its bbox, its size in pixels, the level that size should pick, and how many of
/// the test's features lie whole in it
type TileCase = ([f32; 4], (usize, usize), u32, usize);

/// Where each of `peaks` (fractional cells of level 0) is in the tile, and how far in pixels its
/// feature of `sigma` cells reaches (four widths), for the peaks that are whole in the tile
fn features_in_tile(
    layout: &Layout,
    peaks: &[(f64, f64)],
    sigma: f64,
    bbox: [f32; 4],
    size: (usize, usize),
    mercator: bool,
) -> Vec<((f64, f64), f64)> {
    let reach = 4.0 * sigma / layout.cells_per_pixel(bbox, size.0);
    peaks
        .iter()
        .map(|&peak| {
            let (mut lon, lat) = layout.lon_lat(peak);
            if lon > bbox[2] as f64 {
                lon -= 360.0;
            }
            (pixel_of(bbox, size, (lon, lat), mercator), reach)
        })
        .filter(|&((x, y), reach)| {
            x >= reach && y >= reach && x <= size.0 as f64 - reach && y <= size.1 as f64 - reach
        })
        .collect()
}

/// Test (2): each feature is at the same pixel in a tile from a level as in a tile from level 0,
/// and at the pixel its longitude and latitude put it, for tiles that the tile size sends to
/// levels 1 and 2, in both projections.
#[tokio::test(flavor = "multi_thread")]
async fn a_mrms_like_tile_from_a_level_matches_the_tile_from_level_0() {
    let layout = Layout::mrms_like();
    // near the west and north edges, in the middle, near the east and south edges: where the old
    // geometry was furthest wrong in each direction
    let peaks = [
        (203.5, 131.5),
        (499.5, 251.5),
        (803.5, 363.5),
        (1699.5, 699.5),
        (1851.5, 851.5),
    ];
    let sigma = 24.0;
    for method in [DownsampleMethod::Max, DownsampleMethod::Mean] {
        let pyramid = Pyramid::write(layout, &layout.with_peaks(&peaks, sigma), method);
        let tiles: [TileCase; 2] = [
            // 10 x 5 degrees = 1000 x 500 cells into 256 x 128 pixels: level 1 has 500 x 250
            ([-129.99, 49.995, -119.99, 54.99], (256, 128), 1, 3),
            // the whole grid, 2000 x 1000 cells: level 2 has 500 x 250
            ([-129.99, 45.0, -109.99, 54.99], (256, 128), 2, 5),
        ];
        for (bbox, size, want_level, want_features) in tiles {
            for mercator in [false, true] {
                let (reference, _) = pyramid.tile(bbox, size, Some(0), mercator).await;
                let (tile, level) = pyramid.tile(bbox, size, None, mercator).await;
                assert_eq!(level, want_level, "{bbox:?} {size:?}");

                let features = features_in_tile(&layout, &peaks, sigma, bbox, size, mercator);
                assert!(
                    features.len() >= want_features,
                    "{bbox:?}: {} features",
                    features.len()
                );
                for (at, reach) in features {
                    let from_level_0 = centroid(&reference, size.0, at, reach).unwrap();
                    let from_level = centroid(&tile, size.0, at, reach).unwrap();
                    let what = format!(
                        "{method:?} {bbox:?} level {level} mercator {mercator} feature at {at:?}"
                    );
                    // In a tile from a level the feature is where level 0 draws it. A level cell
                    // is under 2 pixels here and the old geometry put it up to 0.4 of one away.
                    assert!(
                        (from_level.0 - from_level_0.0).abs() < 0.05
                            && (from_level.1 - from_level_0.1).abs() < 0.05,
                        "{what}: level 0 draws it at {from_level_0:?}, the level at {from_level:?}"
                    );
                    // And level 0 draws it where its longitude and latitude put it. (In Web
                    // Mercator the feature's own shape is stretched, which moves its centre of
                    // mass a few hundredths of a pixel, so only the plate carree tile says so.)
                    if !mercator {
                        assert!(
                            (from_level_0.0 - at.0).abs() < 0.05
                                && (from_level_0.1 - at.1).abs() < 0.05,
                            "{what}: level 0 draws it at {from_level_0:?}"
                        );
                    }
                }
            }
        }
    }
}

/// The same for a global 0-360 grid: whole-globe and hemisphere tiles, from levels 2 and 1. The
/// whole-globe tile crosses the prime meridian, so reads the whole grid, and has a feature astride
/// the meridian (round the end of the grid from itself).
#[tokio::test(flavor = "multi_thread")]
async fn a_gfs_like_tile_from_a_level_matches_the_tile_from_level_0() {
    let layout = Layout::gfs_like();
    let peaks = [
        (1259.5, 339.5), // -45.1, 5.1
        (163.5, 123.5),  // 40.9, 59.1
        (899.5, 403.5),  // -135.1, -10.9
        (1439.5, 403.5), // -0.125, -10.9: between the last column and the first
        (1099.5, 603.5), // -85.1, -60.9
        (563.5, 131.5),  // 140.9, 57.1
    ];
    let sigma = 16.0;
    for method in [DownsampleMethod::Max, DownsampleMethod::Mean] {
        let pyramid = Pyramid::write(layout, &layout.with_peaks(&peaks, sigma), method);
        let tiles: [TileCase; 3] = [
            // the globe to 80 degrees: 1440 x 640 cells into 256 x 128 pixels; level 2 has
            // 360 x 160
            ([-180.0, -80.0, 180.0, 80.0], (256, 128), 2, 6),
            // the eastern hemisphere: 720 x 640 cells into 256 x 256; level 1 has 360 x 320
            ([0.0, -80.0, 180.0, 80.0], (256, 256), 1, 2),
            // the western one, in -180..180 notation
            ([-180.0, -80.0, 0.0, 80.0], (256, 256), 1, 3),
        ];
        for (bbox, size, want_level, want_features) in tiles {
            for mercator in [false, true] {
                let (reference, _) = pyramid.tile(bbox, size, Some(0), mercator).await;
                let (tile, level) = pyramid.tile(bbox, size, None, mercator).await;
                assert_eq!(level, want_level, "{bbox:?} {size:?}");

                let features = features_in_tile(&layout, &peaks, sigma, bbox, size, mercator);
                // (a feature that straddles the edge of a hemisphere is left out)
                assert!(
                    features.len() >= want_features,
                    "{bbox:?}: {} features",
                    features.len()
                );
                for (at, reach) in features {
                    let from_level_0 = centroid(&reference, size.0, at, reach).unwrap();
                    let from_level = centroid(&tile, size.0, at, reach).unwrap();
                    let what = format!(
                        "{method:?} {bbox:?} level {level} mercator {mercator} feature at {at:?}"
                    );
                    assert!(
                        (from_level.0 - from_level_0.0).abs() < 0.05
                            && (from_level.1 - from_level_0.1).abs() < 0.05,
                        "{what}: level 0 draws it at {from_level_0:?}, the level at {from_level:?}"
                    );
                }
            }
        }
    }
}

/// A global level starts east of 0 (GFS level 2: 0.375 degrees) and ends before 360, and the
/// longitudes between its last cell and its first are round the end of the grid, not off it: a
/// tile across the prime meridian has no gap there. A constant grid makes any NaN a gap.
#[tokio::test(flavor = "multi_thread")]
async fn a_tile_from_a_global_level_has_no_gap_at_the_prime_meridian() {
    let layout = Layout::gfs_like();
    let data = vec![7.0f32; layout.shape.0 * layout.shape.1];
    let pyramid = Pyramid::write(layout, &data, DownsampleMethod::Mean);
    // 20 x 10 degrees in 160 x 80 pixels of 0.125 degrees: ten pixel columns lie in the 0.375
    // degrees between 0 and level 2's first cell
    let bbox = [-10.0f32, -5.0, 10.0, 5.0];
    for level in 0..=3u32 {
        for mercator in [false, true] {
            let (tile, _) = pyramid.tile(bbox, (160, 80), Some(level), mercator).await;
            let gaps: Vec<usize> = (0..160)
                .filter(|&x| (0..80).any(|y| !(tile[y * 160 + x] - 7.0).abs().lt(&1e-3)))
                .collect();
            assert!(
                gaps.is_empty(),
                "level {level} mercator {mercator}: columns {gaps:?} have no data"
            );
        }
    }
}

/// Test (2), end to end through the real loader: the same comparison, but the pyramid is in MinIO
/// and the tile's grid comes from `load_grid_data_from_zarr` (catalog entry -> level choice ->
/// region -> `GridData`), as GetTile does. Needs Docker:
///
/// ```text
/// cargo test -p wms-api --lib level_tile_tests -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn a_loader_tile_from_a_level_matches_the_tile_from_level_0() {
    let infra = TestInfrastructure::start().await;
    infra
        .create_minio_bucket("test-bucket")
        .await
        .expect("bucket");
    let object_storage = ObjectStorage::new(&ObjectStorageConfig {
        endpoint: infra.minio_url(),
        bucket: "test-bucket".to_string(),
        access_key_id: "minioadmin".to_string(),
        secret_access_key: "minioadmin".to_string(),
        region: "us-east-1".to_string(),
        allow_http: true,
    })
    .unwrap();
    let factory = GridProcessorFactory::new(
        MinioConfig {
            endpoint: infra.minio_url(),
            bucket: "test-bucket".to_string(),
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
            allow_http: true,
        },
        64,
    )
    .unwrap();

    // (layout, the peaks drawn into it, their width, the way it is downsampled, a tile and the
    // level its size should pick). The kind of data and the zoom each layout is used at.
    let cases = [
        (
            Layout::mrms_like(),
            vec![
                (203.5, 131.5),
                (499.5, 251.5),
                (803.5, 363.5),
                (1699.5, 699.5),
            ],
            24.0,
            DownsampleMethod::Max,
            [-129.99f32, 45.0, -109.99, 54.99],
            (256usize, 128usize),
            2u32,
        ),
        (
            Layout::gfs_like(),
            vec![
                (1259.5, 339.5),
                (163.5, 123.5),
                (899.5, 403.5),
                (1439.5, 403.5),
            ],
            16.0,
            DownsampleMethod::Mean,
            [-180.0, -80.0, 180.0, 80.0],
            (256, 128),
            2,
        ),
    ];
    for (layout, peaks, sigma, method, bbox, size, want_level) in cases {
        // The pyramid as the ingester stores it: written to a directory, uploaded under the
        // dataset's path, catalogued with its multiscale metadata.
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("grid.zarr");
        std::fs::create_dir_all(&local).unwrap();
        let MultiscaleWriteResult {
            zarr_metadata,
            multiscale_metadata,
            ..
        } = ZarrWriter::new(GridProcessorConfig::default())
            .write_multiscale(
                FilesystemStore::new(&local).unwrap(),
                "/",
                &layout.with_peaks(&peaks, sigma),
                layout.shape.0,
                layout.shape.1,
                &layout.bbox,
                layout.model,
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
                RowOrigin::North,
                ProjectionType::Geographic,
            )
            .unwrap();
        let storage_path = format!("grids/{}/level-geometry/FIELD.zarr", layout.model);
        for file in walkdir::WalkDir::new(&local)
            .into_iter()
            .map(Result::unwrap)
            .filter(|f| f.file_type().is_file())
        {
            let relative = file.path().strip_prefix(&local).unwrap();
            object_storage
                .put(
                    &format!("{storage_path}/{}", relative.display()),
                    std::fs::read(file.path()).unwrap().into(),
                )
                .await
                .unwrap();
        }
        let mut zarr_json = zarr_metadata.to_json();
        zarr_json["multiscale"] = serde_json::to_value(&multiscale_metadata).unwrap();
        let entry = CatalogEntry {
            model: layout.model.to_string(),
            parameter: "FIELD".to_string(),
            level: "surface".to_string(),
            reference_time: chrono::Utc
                .with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
                .unwrap(),
            forecast_hour: 0,
            forecast_minute: 0,
            bbox: wms_common::BoundingBox::new(
                layout.bbox.min_lon,
                layout.bbox.min_lat,
                layout.bbox.max_lon,
                layout.bbox.max_lat,
            ),
            storage_path,
            file_size: 0,
            zarr_metadata: Some(zarr_json),
        };

        // A tile from the grid the loader returns for `output_size`: what GetTile does with the
        // tile's own size, or with a size no level can fill (level 0).
        let tile = |output_size: (usize, usize)| {
            let (factory, entry) = (&factory, &entry);
            async move {
                let grid =
                    load_grid_data_from_zarr(factory, entry, Some(bbox), Some(output_size), false)
                        .await
                        .unwrap();
                let tile = resample_grid_for_bbox_with_proj(
                    &grid.data,
                    grid.width,
                    grid.height,
                    size.0,
                    size.1,
                    bbox,
                    grid.bbox.unwrap(),
                    false,
                    layout.model,
                    None,
                    grid.grid_uses_360,
                );
                (tile, grid.width)
            }
        };
        let (reference, width_0) = tile((1 << 20, 1 << 20)).await;
        let (from_level, width_n) = tile(size).await;
        let level = (width_0 as f64 / width_n as f64).log2().round() as u32;
        assert_eq!(
            level, want_level,
            "{} read from level {level}",
            layout.model
        );

        let features = features_in_tile(&layout, &peaks, sigma, bbox, size, false);
        assert!(
            features.len() >= 3,
            "{}: {} features",
            layout.model,
            features.len()
        );
        for (at, reach) in features {
            let from_level_0 = centroid(&reference, size.0, at, reach).unwrap();
            let from_level = centroid(&from_level, size.0, at, reach).unwrap();
            assert!(
                (from_level.0 - from_level_0.0).abs() < 0.05
                    && (from_level.1 - from_level_0.1).abs() < 0.05,
                "{} feature at {at:?}: level 0 draws it at {from_level_0:?}, level {level} at \
                 {from_level:?}",
                layout.model
            );
            assert!(
                (from_level_0.0 - at.0).abs() < 0.05 && (from_level_0.1 - at.1).abs() < 0.05,
                "{} feature at {at:?}: level 0 draws it at {from_level_0:?}",
                layout.model
            );
        }
    }
}
