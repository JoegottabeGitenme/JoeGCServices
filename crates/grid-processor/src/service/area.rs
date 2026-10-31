//! Reading a dataset for an EDR area query.
//!
//! `GridDataService::read_region` answers "the cells in this box" and leaves the rest to the
//! caller. An area query needs more of the dataset than the cells: which dataset the catalog
//! matched (to know what time the answer is for), its metadata (units, row order, projection) and
//! whether it can be read cheaper than whole. `read_area` returns those together, from ONE catalog
//! lookup, so a handler does not look the dataset up once for the metadata and once for the grid.
//!
//! What it adds to a plain region read, opt-in through [`AreaReadOptions`]: for a projected grid,
//! just the cells the box covers (a [`NativeWindow`]) instead of all of it, from an overview level
//! when the image is small.
//!
//! A lat/lon grid is always read at native resolution, so that every parameter of a dataset family
//! (MRMS `REFL`, `MESH` and `PRECIP_FLAG`) comes back on the same grid for the same box. Overview
//! levels are never used for a categorical parameter ([`is_categorical_parameter`]).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use zarrs::storage::ReadableStorageTraits;

use super::{append_level_path, normalize_path, GridDataService, MATCH_TOLERANCE};
use crate::cache::ChunkCache;
use crate::config::GridProcessorConfig;
use crate::downsample::is_categorical_parameter;
use crate::error::{GridProcessorError, Result};
use crate::processor::{
    parse_multiscale_metadata, GridProcessor, MultiscaleGridProcessorFactory, ZarrGridProcessor,
};
use crate::projection::NativeGrid;
use crate::query::DatasetQuery;
use crate::types::{BoundingBox, GridMetadata, GridRegion, MultiscaleMetadata, RowOrigin};
use crate::window::{Footprint, NativeWindow};
use crate::writer::ZarrMetadata;

/// How an area query wants its grid read.
#[derive(Debug, Clone, Copy, Default)]
pub struct AreaReadOptions {
    /// Size of the image the caller will make from the grid. A cropped projected grid with at
    /// least twice as many cells across the box as the image has pixels is read from a coarser
    /// overview level, which is a fraction of the data. `None` reads native resolution.
    ///
    /// No effect elsewhere. A projected grid read whole (no `crop`) comes at native resolution:
    /// the caller gets the whole domain whatever box it sent, so the size of the box says nothing
    /// about how much of the domain the image needs. A lat/lon grid is read at native resolution
    /// whatever the image size: a read from an overview level lands on different cell edges (so a
    /// different extent) than the native one, and the parameters that must stay native
    /// (categorical ones) would come back on another grid than the quantities beside them.
    pub output_size: Option<(usize, usize)>,
    /// Read only the cells of a projected grid that this box covers, as a [`NativeWindow`]. `None`
    /// reads a projected grid whole, whatever the box. No effect on a lat/lon grid, which is always
    /// read for the `bbox` of the call.
    ///
    /// Not the same box as that `bbox` for a MULTIPOLYGON: a lat/lon read has always been made for
    /// the first polygon's box, while an image cropped to the request covers every polygon.
    pub crop: Option<BoundingBox>,
}

/// The cells an area read came back with.
#[derive(Debug, Clone)]
pub enum AreaData {
    /// What `read_region` returns: for a lat/lon grid the cells in the box (a little more, see
    /// `ZarrGridProcessor::read_region`), for a projected grid read without `crop` all of it. Empty
    /// (0 x 0) when the box misses the dataset.
    Region(GridRegion),
    /// A projected grid read with `crop`: only the cells the box needs, to be sampled onto the box
    /// with [`NativeWindow::resample`].
    Window(NativeWindow),
}

/// An area read: the cells, and what the catalog matched to get them.
#[derive(Debug, Clone)]
pub struct AreaRead {
    pub data: AreaData,
    /// The dataset's own metadata (native level): units, row origin, projection, fill value.
    pub metadata: GridMetadata,
    /// Run time of the dataset the catalog matched.
    pub reference_time: DateTime<Utc>,
    /// Valid time of the dataset the catalog matched. A request for a time gets the NEAREST
    /// dataset, so this is not necessarily the time that was asked for.
    pub valid_time: DateTime<Utc>,
    /// Pyramid level the cells came from: 0 is native resolution.
    pub level: u32,
}

impl GridDataService {
    /// Look `query` up once and read the cells of `bbox` for an area query.
    ///
    /// Returns the dataset's metadata and the time of the dataset matched along with the cells;
    /// see [`AreaRead`] and [`AreaReadOptions`]. A run that is not in the catalog, or a time the
    /// run does not hold, is `NotFound` when the query names them (see
    /// [`crate::query::TimeSpecification::Instance`]).
    pub async fn read_area(
        &self,
        query: &DatasetQuery,
        bbox: &BoundingBox,
        options: AreaReadOptions,
    ) -> Result<AreaRead> {
        let entry = self.find_dataset(query).await?.ok_or_else(|| {
            GridProcessorError::NotFound(format!(
                "No dataset found for {}/{} with specified time/level",
                query.model, query.parameter
            ))
        })?;
        let zarr_json = entry.zarr_metadata.as_ref().ok_or_else(|| {
            GridProcessorError::Metadata("Catalog entry missing zarr_metadata".to_string())
        })?;
        let zarr_meta = ZarrMetadata::from_json(zarr_json)
            .map_err(|e| GridProcessorError::Metadata(e.to_string()))?;
        let metadata = GridMetadata::from(&zarr_meta);

        let (data, level) = read_area_from(
            self.factory.storage(),
            &normalize_path(&entry.storage_path),
            &metadata,
            parse_multiscale_metadata(zarr_json),
            self.factory.chunk_cache(),
            self.factory.config().clone(),
            bbox,
            options,
        )
        .await?;

        Ok(AreaRead {
            data,
            metadata,
            reference_time: entry.reference_time,
            valid_time: entry.valid_time(),
            level,
        })
    }

    /// The dataset of run `reference_time` valid at `valid_time` (within [`MATCH_TOLERANCE`]), or
    /// the run's first forecast when no time is named. Exactly that run: another run's dataset
    /// for the same time is not an answer.
    pub(super) async fn find_in_instance(
        &self,
        query: &DatasetQuery,
        reference_time: DateTime<Utc>,
        valid_time: Option<DateTime<Utc>>,
    ) -> Result<Option<storage::CatalogEntry>> {
        let level = query.level.as_deref();
        let catalog_error = |e: wms_common::WmsError| GridProcessorError::Catalog(e.to_string());

        let Some(valid_time) = valid_time else {
            // Not hour 0: a run need not have one (NBM starts at hour 1)
            return self
                .catalog
                .find_run_first_forecast(&query.model, &query.parameter, reference_time, level)
                .await
                .map_err(catalog_error);
        };

        // One dataset per run holds each instant, so the candidates for it are few
        let candidates = self
            .catalog
            .find_datasets_in_valid_time_range(
                &query.model,
                &query.parameter,
                level,
                valid_time - MATCH_TOLERANCE,
                valid_time + MATCH_TOLERANCE,
            )
            .await
            .map_err(catalog_error)?;
        Ok(candidates
            .into_iter()
            .filter(|(_, entry)| entry.reference_time == reference_time)
            .min_by_key(|(valid, _)| (*valid - valid_time).abs())
            .map(|(_, entry)| entry))
    }
}

/// [`GridDataService::read_area`] once the dataset is chosen: everything that needs the store and
/// not the catalog, so it can be exercised against a filesystem store. Returns the cells and the
/// pyramid level they came from.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn read_area_from<S>(
    store: S,
    zarr_path: &str,
    metadata: &GridMetadata,
    multiscale: Option<MultiscaleMetadata>,
    chunk_cache: Arc<RwLock<ChunkCache>>,
    config: GridProcessorConfig,
    bbox: &BoundingBox,
    options: AreaReadOptions,
) -> Result<(AreaData, u32)>
where
    S: ReadableStorageTraits + Clone + Send + Sync + 'static,
{
    // A parameter of codes or flags is never answered from an overview level: those hold the
    // codes of other cells, or (for grids ingested before the flags were categorical) fractions.
    let overview_size = options
        .output_size
        .filter(|_| !is_categorical_parameter(&metadata.parameter));
    // A dataset with only its native level has nothing to choose from
    let multiscale = multiscale.filter(|m| m.num_levels() > 1);

    let native_level = || {
        ZarrGridProcessor::with_metadata(
            store.clone(),
            &append_level_path(zarr_path, 0),
            metadata.clone(),
            chunk_cache.clone(),
            config.clone(),
        )
    };

    // Projected grid, cropped: only the cells the box covers. Needs the grid's own projection to
    // find them and rows counted from the south, as the full-grid reprojection assumes; a grid
    // that is neither is read whole as before.
    if let (Some(crop), true, RowOrigin::South) = (
        &options.crop,
        metadata.projection.requires_projection_transform(),
        metadata.row_origin,
    ) {
        if let Some(native) = NativeGrid::for_model(&metadata.model, metadata.projection) {
            let empty = || {
                (
                    AreaData::Region(GridRegion::new(vec![], 0, 0, *crop, metadata.resolution())),
                    0,
                )
            };
            let Some(footprint) = Footprint::of(&native, crop) else {
                return Ok(empty());
            };
            if !footprint.overlaps(metadata.shape.0, metadata.shape.1) {
                return Ok(empty());
            }

            let level = match (&multiscale, overview_size) {
                (Some(ms), Some(size)) => footprint.coarsest_level_for(&ms.levels, size),
                _ => 0,
            };
            let (processor, scale) = match (level, multiscale) {
                (level, Some(ms)) if level > 0 => {
                    let scale = ms.get_level(level).map_or(1.0, |l| l.scale);
                    let levels = MultiscaleGridProcessorFactory::new(
                        store.clone(),
                        zarr_path,
                        ms,
                        chunk_cache.clone(),
                        config.clone(),
                    )
                    .with_dataset_metadata(metadata.clone());
                    (levels.processor_for_level(level)?, scale)
                }
                _ => (native_level()?, 1.0),
            };
            return Ok(
                match processor.read_native_window(&footprint, scale).await? {
                    Some(window) => (AreaData::Window(window), level),
                    None => empty(),
                },
            );
        }
    }

    // Everything else at native resolution: a lat/lon grid for the box, a projected grid whole.
    Ok((
        AreaData::Region(native_level()?.read_region(bbox).await?),
        0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use zarrs_filesystem::FilesystemStore;

    use crate::config::PyramidConfig;
    use crate::downsample::DownsampleMethod;
    use crate::types::ProjectionType;
    use crate::window::level_index;
    use crate::writer::{MultiscaleWriteResult, ZarrWriter};

    const NX: usize = 1799;
    const NY: usize = 1059;
    /// HRRR's bounding box as the ingester stores it (the corners' envelope)
    fn hrrr_bbox() -> BoundingBox {
        BoundingBox::new(-134.09548, 21.138123, -60.917193, 47.842195)
    }

    /// A dataset on disk: `<dir>/grid.zarr` with its pyramid, ready for `read_area_from`.
    struct Fixture {
        _dir: tempfile::TempDir,
        store: Arc<FilesystemStore>,
        metadata: GridMetadata,
        multiscale: MultiscaleMetadata,
    }

    impl Fixture {
        async fn read(
            &self,
            bbox: &BoundingBox,
            options: AreaReadOptions,
        ) -> Result<(AreaData, u32)> {
            read_area_from(
                self.store.clone(),
                "/grid.zarr",
                &self.metadata,
                Some(self.multiscale.clone()),
                Arc::new(RwLock::new(ChunkCache::new(256 << 20))),
                GridProcessorConfig::default(),
                bbox,
                options,
            )
            .await
        }
    }

    /// HRRR-sized Lambert grid in which every cell holds `3 * col + 7 * row`: affine in the
    /// grid's own coordinates, so mean-downsampled levels hold the same field at their cell
    /// centres and a bilinear sample is exact at every level.
    fn hrrr_fixture(parameter: &str) -> Fixture {
        let data: Vec<f32> = (0..NY)
            .flat_map(|row| (0..NX).map(move |col| (3 * col + 7 * row) as f32))
            .collect();
        write_fixture(
            &data,
            NX,
            NY,
            hrrr_bbox(),
            "hrrr",
            parameter,
            RowOrigin::South,
            ProjectionType::LambertConformal,
            256, // as ingested: 1799x1059, 899x529, 449x264
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn write_fixture(
        data: &[f32],
        width: usize,
        height: usize,
        bbox: BoundingBox,
        model: &str,
        parameter: &str,
        row_origin: RowOrigin,
        projection: ProjectionType,
        min_dimension: usize,
    ) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FilesystemStore::new(dir.path()).unwrap());
        let MultiscaleWriteResult {
            zarr_metadata,
            multiscale_metadata,
            ..
        } = ZarrWriter::new(GridProcessorConfig::default())
            .write_multiscale(
                store.clone(),
                "/grid.zarr",
                data,
                width,
                height,
                &bbox,
                model,
                parameter,
                "surface",
                "dBZ",
                chrono::Utc
                    .with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
                    .unwrap(),
                0,
                &PyramidConfig {
                    min_dimension,
                    ..PyramidConfig::default()
                },
                DownsampleMethod::Mean,
                row_origin,
                projection,
            )
            .unwrap();
        Fixture {
            _dir: dir,
            store,
            metadata: GridMetadata::from(&zarr_metadata),
            multiscale: multiscale_metadata,
        }
    }

    fn window(data: &AreaData) -> &NativeWindow {
        match data {
            AreaData::Window(w) => w,
            AreaData::Region(r) => {
                panic!("expected a window, got a {}x{} region", r.width, r.height)
            }
        }
    }

    fn trail() -> BoundingBox {
        BoundingBox::new(-106.0, 39.0, -103.5, 40.5)
    }

    /// The field `3 * col + 7 * row` at a pixel centre, by the real HRRR projection
    fn field_at(bbox: &BoundingBox, (w, h): (usize, usize), x: usize, y: usize) -> f64 {
        let lon = bbox.min_lon + (x as f64 + 0.5) * bbox.width() / w as f64;
        let lat = bbox.max_lat - (y as f64 + 0.5) * bbox.height() / h as f64;
        let (i, j) = projection::LambertConformal::hrrr().geo_to_grid(lat, lon);
        3.0 * i + 7.0 * j
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_pyramid_of_the_fixture_is_the_one_hrrr_is_ingested_with() {
        let fx = hrrr_fixture("REFC");
        let shapes: Vec<_> = fx.multiscale.levels.iter().map(|l| l.shape).collect();
        assert_eq!(shapes, [(1799, 1059), (899, 529), (449, 264)]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cropped_projected_read_is_a_window_of_a_few_cells_that_resamples_to_the_field() {
        let fx = hrrr_fixture("REFC");
        let bbox = trail();
        let size = (250, 150);
        let (data, level) = fx
            .read(
                &bbox,
                AreaReadOptions {
                    output_size: Some(size),
                    crop: Some(bbox),
                },
            )
            .await
            .unwrap();
        // ~71 x 56 cells across the box cannot fill 250 x 150 pixels from a coarser level
        assert_eq!(level, 0);
        let win = window(&data);
        assert!(
            win.width * win.height < 100 * 90,
            "a trail view is a few thousand cells, not the {} of the grid: {}x{}",
            NX * NY,
            win.width,
            win.height
        );
        assert_eq!(
            (win.level_width, win.level_height, win.scale),
            (NX, NY, 1.0)
        );

        // Test (a): every pixel is the native grid's value where the real HRRR projection puts
        // that pixel's centre
        let out = win.resample(&bbox, size.0, size.1);
        for y in 0..size.1 {
            for x in 0..size.0 {
                let want = field_at(&bbox, size, x, y);
                let got = out.data[y * size.0 + x] as f64;
                assert!(
                    (got - want).abs() < 0.05,
                    "({x},{y}): got {got}, want {want}"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_small_image_of_a_large_box_comes_from_an_overview_level_with_the_same_values() {
        let fx = hrrr_fixture("REFC");
        // The central United States: ~590 x 420 cells of the 3 km grid
        let bbox = BoundingBox::new(-110.0, 33.0, -90.0, 45.0);
        for (size, want_level, want_scale) in [
            ((500, 350), 0, 1.0),
            ((250, 180), 1, 2.0),
            ((120, 90), 2, 4.0),
        ] {
            let (data, level) = fx
                .read(
                    &bbox,
                    AreaReadOptions {
                        output_size: Some(size),
                        crop: Some(bbox),
                    },
                )
                .await
                .unwrap();
            assert_eq!(level, want_level, "{size:?}");
            let win = window(&data);
            assert_eq!(win.scale, want_scale);
            // a level has fewer cells, so its window does
            assert!(win.width <= (700.0 / want_scale) as usize, "{}", win.width);

            // Test: reprojecting from a level puts each pixel on the cell that holds it. The
            // field is affine, so a level (cell means at block centres) still holds it exactly;
            // a level index that ignored the block centres would be off by half a level-0 cell
            // per level, i.e. 3 * 0.5 = 1.5 here.
            let out = win.resample(&bbox, size.0, size.1);
            let mut valid = 0;
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let got = out.data[y * size.0 + x] as f64;
                    assert!(!got.is_nan());
                    valid += 1;
                    let want = field_at(&bbox, size, x, y);
                    assert!(
                        (got - want).abs() < 0.05,
                        "level {level} ({x},{y}): got {got}, want {want}"
                    );
                }
            }
            assert_eq!(valid, size.0 * size.1);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_categorical_parameter_is_never_read_from_an_overview_level() {
        let bbox = BoundingBox::new(-110.0, 33.0, -90.0, 45.0);
        let size = (120, 90); // level 2 for a quantity
        for (parameter, want_level) in [("REFC", 2), ("CSNOW", 0), ("CFRZR", 0), ("PRECIP_FLAG", 0)]
        {
            let fx = hrrr_fixture(parameter);
            let (data, level) = fx
                .read(
                    &bbox,
                    AreaReadOptions {
                        output_size: Some(size),
                        crop: Some(bbox),
                    },
                )
                .await
                .unwrap();
            assert_eq!(level, want_level, "{parameter}");
            assert_eq!(window(&data).scale, if want_level == 0 { 1.0 } else { 4.0 });
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_crop_a_projected_grid_is_read_whole_at_native_resolution() {
        let fx = hrrr_fixture("REFC");
        // an image size that would pick level 2 if the box said how much of the domain is wanted
        let (data, level) = fx
            .read(
                &trail(),
                AreaReadOptions {
                    output_size: Some((20, 12)),
                    crop: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(level, 0);
        match data {
            AreaData::Region(r) => {
                assert_eq!((r.width, r.height), (NX, NY));
                assert_eq!(r.bbox.min_lon, hrrr_bbox().min_lon);
                assert_eq!(r.data[0], 0.0);
                assert_eq!(r.data[NX * NY - 1], (3 * (NX - 1) + 7 * (NY - 1)) as f32);
            }
            AreaData::Window(_) => panic!("no crop was asked for"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_box_that_misses_the_grid_is_an_empty_region_like_any_other_no_data_read() {
        let fx = hrrr_fixture("REFC");
        let europe = BoundingBox::new(0.0, 45.0, 10.0, 50.0);
        let (data, _) = fx
            .read(
                &europe,
                AreaReadOptions {
                    output_size: Some((100, 100)),
                    crop: Some(europe),
                },
            )
            .await
            .unwrap();
        match data {
            AreaData::Region(r) => assert_eq!((r.width, r.height, r.data.len()), (0, 0, 0)),
            AreaData::Window(_) => panic!("nothing to read"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_box_over_the_edge_of_the_grid_has_no_data_outside_it() {
        let fx = hrrr_fixture("REFC");
        // 15 degrees of longitude, the western ~5 of them west of the grid's first column
        let bbox = BoundingBox::new(-140.0, 35.0, -125.0, 40.0);
        let proj = projection::LambertConformal::hrrr();
        // level 0, then level 1
        for (size, level) in [((450, 150), 0), ((150, 50), 1)] {
            let (data, got) = fx
                .read(
                    &bbox,
                    AreaReadOptions {
                        output_size: Some(size),
                        crop: Some(bbox),
                    },
                )
                .await
                .unwrap();
            assert_eq!(got, level);
            let win = window(&data);
            let out = win.resample(&bbox, size.0, size.1);
            assert!(
                out.data[..10].iter().all(|v| v.is_nan()),
                "far west is off the grid"
            );
            assert!(
                out.data[size.0 - 10..size.0].iter().all(|v| !v.is_nan()),
                "the east end is on it"
            );
            // A pixel has data exactly where its centre is between the first and last cell centres
            // of the level that was read
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let lon = bbox.min_lon + (x as f64 + 0.5) * bbox.width() / size.0 as f64;
                    let lat = bbox.max_lat - (y as f64 + 0.5) * bbox.height() / size.1 as f64;
                    let (i, j) = proj.geo_to_grid(lat, lon);
                    let (li, lj) = (level_index(i, win.scale), level_index(j, win.scale));
                    let inside = li >= 0.0
                        && li < win.level_width as f64 - 1.0
                        && lj >= 0.0
                        && lj < win.level_height as f64 - 1.0;
                    let v = out.data[y * size.0 + x];
                    assert_eq!(
                        !v.is_nan(),
                        inside,
                        "level {level} ({x},{y}) at {li:.2},{lj:.2}"
                    );
                }
            }
        }
    }

    /// A 1000 x 500 lat/lon grid (0.1 degree) with a pyramid, `value = col + 2 * row`.
    fn latlon_fixture() -> Fixture {
        latlon_fixture_of("REFL")
    }

    fn latlon_fixture_of(parameter: &str) -> Fixture {
        let data: Vec<f32> = (0..500)
            .flat_map(|row| (0..1000).map(move |col| (col + 2 * row) as f32))
            .collect();
        write_fixture(
            &data,
            1000,
            500,
            BoundingBox::new(-130.0, 20.0, -30.05, 69.95),
            "mrms",
            parameter,
            RowOrigin::North,
            ProjectionType::Geographic,
            100, // 1000x500, 500x250, 250x125
        )
    }

    /// What a native read of `bbox` gives, with no `read_area` in between
    async fn native_region(fx: &Fixture, bbox: &BoundingBox) -> GridRegion {
        ZarrGridProcessor::with_metadata(
            fx.store.clone(),
            "/grid.zarr/0",
            fx.metadata.clone(),
            Arc::new(RwLock::new(ChunkCache::new(256 << 20))),
            GridProcessorConfig::default(),
        )
        .unwrap()
        .read_region(bbox)
        .await
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_lat_lon_grid_is_read_at_native_resolution_whatever_crop_and_the_image_size() {
        let fx = latlon_fixture();
        // 20 x 10 degrees = 200 x 100 cells of the 1000 x 500 grid, which has a pyramid to
        // 250 x 125. The sizes would pick levels 0 to 3 for a quantity.
        let bbox = BoundingBox::new(-100.0, 30.0, -80.0, 40.0);
        let want = native_region(&fx, &bbox).await;
        assert!(want.width >= 200 && want.height >= 100);

        for size in [
            None,
            Some((200, 100)),
            Some((100, 50)),
            Some((50, 25)),
            Some((4, 3)),
        ] {
            for crop in [None, Some(bbox)] {
                let options = AreaReadOptions {
                    output_size: size,
                    crop,
                };
                let (data, level) = fx.read(&bbox, options).await.unwrap();
                let AreaData::Region(got) = data else {
                    panic!("a lat/lon grid is always a region")
                };
                let what = format!("{size:?} crop {}", crop.is_some());
                assert_eq!(level, 0, "{what}");
                assert_eq!(
                    (got.width, got.height, got.bbox, got.resolution),
                    (want.width, want.height, want.bbox, want.resolution),
                    "{what}"
                );
                assert_eq!(got.data, want.data, "{what}");
            }
        }
    }

    /// The finding being fixed: a quantity came from an overview level and a flag from level 0,
    /// so for one box and image size the two came back on different grids (different extents).
    /// The app lays a flag over a quantity from one extent.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_wide_lat_lon_view_comes_back_on_one_grid_for_every_parameter() {
        // 80 x 40 degrees = 800 x 400 cells into 60 x 30 pixels
        let bbox = BoundingBox::new(-120.0, 25.0, -40.0, 65.0);
        let options = AreaReadOptions {
            output_size: Some((60, 30)),
            crop: None,
        };
        let mut grids = Vec::new();
        for parameter in ["REFL", "MESH", "PRECIP_FLAG"] {
            let fx = latlon_fixture_of(parameter);
            let (data, level) = fx.read(&bbox, options).await.unwrap();
            let AreaData::Region(r) = data else { panic!() };
            assert_eq!(level, 0, "{parameter}");
            // the native grid: 0.1 degree cells over the box and its 2-cell margin
            let native = native_region(&fx, &bbox).await;
            assert_eq!(
                (r.width, r.height, r.bbox),
                (native.width, native.height, native.bbox)
            );
            assert!(
                r.width >= 800 && r.height >= 400,
                "{}x{}",
                r.width,
                r.height
            );
            grids.push((r.width, r.height, r.bbox, r.resolution));
        }
        assert!(grids.windows(2).all(|pair| pair[0] == pair[1]), "{grids:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_level_keeps_the_datasets_model_parameter_and_fill_not_a_guess_from_its_name() {
        // The multiscale name is "mrms_PRECIP_FLAG", which split on '_' names the parameter PRECIP
        let mut fx = latlon_fixture_of("PRECIP_FLAG");
        fx.metadata.fill_value = -999.0;
        fx.metadata.units = "code".to_string();
        let factory = MultiscaleGridProcessorFactory::new(
            fx.store.clone(),
            "/grid.zarr",
            fx.multiscale.clone(),
            Arc::new(RwLock::new(ChunkCache::new(1 << 20))),
            GridProcessorConfig::default(),
        );

        let guessed = factory.processor_for_level(1).unwrap();
        assert_eq!(guessed.metadata().parameter, "PRECIP");
        assert!(guessed.metadata().fill_value.is_nan());

        let factory = factory.with_dataset_metadata(fx.metadata.clone());
        for level in 0..3 {
            let p = factory.processor_for_level(level).unwrap();
            let m = p.metadata();
            assert_eq!(
                (
                    m.model.as_str(),
                    m.parameter.as_str(),
                    m.units.as_str(),
                    m.fill_value
                ),
                ("mrms", "PRECIP_FLAG", "code", -999.0),
                "level {level}"
            );
            // geometry still the level's own
            assert_eq!(m.shape, fx.multiscale.levels[level as usize].shape);
        }
    }
}
