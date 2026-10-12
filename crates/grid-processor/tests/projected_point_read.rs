//! Point reads on a projected grid land on the right cell.
//!
//! `read_point` used to locate a point by interpolating linearly across the grid's longitude /
//! latitude bounding box, which is only right for a regular lat/lon grid. On HRRR's Lambert grid
//! the answer for Denver came from a cell ~470 km away (production 2 m temperature differed from
//! the file by up to 5 K at six cities). Here a real-sized HRRR array holds, in every cell, its own
//! index (`row * 1799 + col`, exact in f32), so what a read returns says which cell it came from.
//!
//! Two independent references:
//! - `wgrib2 -ijlat` on a real HRRR message: the longitude/latitude NCEP assigns to named cells;
//! - pyproj (a different implementation of the projection) for fractional positions.

use std::sync::Arc;

use grid_processor::{
    BoundingBox, ChunkCache, GridMetadata, GridProcessor, GridProcessorConfig, ProjectionType,
    RowOrigin, ZarrGridProcessor,
};
use tokio::sync::RwLock;
use zarrs::array::{ArrayBuilder, DataType, FillValue};
use zarrs::array_subset::ArraySubset;
use zarrs_filesystem::FilesystemStore;

const NX: usize = 1799;
const NY: usize = 1059;

/// HRRR as the ingester writes it: south row first, bounding box = the corner-based envelope.
fn hrrr_processor(dir: &std::path::Path, model: &str) -> ZarrGridProcessor<FilesystemStore> {
    let path = dir.join("hrrr.zarr");
    std::fs::create_dir_all(&path).unwrap();
    let store = Arc::new(FilesystemStore::new(&path).unwrap());
    let array = ArrayBuilder::new(
        vec![NY as u64, NX as u64],
        DataType::Float32,
        vec![256u64, 256u64].try_into().unwrap(),
        FillValue::from(f32::NAN),
    )
    .build(store, "/")
    .unwrap();
    array.store_metadata().unwrap();
    let data: Vec<f32> = (0..NY)
        .flat_map(|row| (0..NX).map(move |col| (row * NX + col) as f32))
        .collect();
    array
        .store_array_subset_elements(
            &ArraySubset::new_with_start_shape(vec![0, 0], vec![NY as u64, NX as u64]).unwrap(),
            &data,
        )
        .unwrap();

    let metadata = GridMetadata {
        model: model.to_string(),
        parameter: "ID".to_string(),
        level: "surface".to_string(),
        units: "1".to_string(),
        reference_time: chrono::Utc::now(),
        forecast_hour: 0,
        bbox: BoundingBox::new(-134.09548, 21.138123, -60.917193, 47.842195),
        shape: (NX, NY),
        chunk_shape: (256, 256),
        num_chunks: (NX.div_ceil(256), NY.div_ceil(256)),
        fill_value: f32::NAN,
        row_origin: RowOrigin::South,
        projection: ProjectionType::LambertConformal,
    };
    ZarrGridProcessor::with_metadata(
        FilesystemStore::new(&path).unwrap(),
        "/",
        metadata,
        Arc::new(RwLock::new(ChunkCache::new(64 << 20))),
        GridProcessorConfig::default(),
    )
    .unwrap()
}

/// `(col, row, lat, lon)` from `wgrib2 -ijlat` on hrrr.t18z.wrfsubhf01 (0-based cells, 0-360 lon).
const FILE_CELLS: [(usize, usize, f64, f64); 9] = [
    (0, 0, 21.138123, 237.280472),
    (1798, 0, 21.140547, 287.710282),
    (0, 1058, 47.838623, 225.90452),
    (1798, 1058, 47.842195, 299.082807),
    (898, 528, 38.470261, 262.459567),
    (898, 0, 24.36457, 262.46624),
    (898, 1058, 52.615645, 262.449558),
    (448, 793, 44.477121, 245.439016),
    (1348, 263, 30.42687, 276.461898),
];

#[tokio::test]
async fn a_point_at_a_cells_own_coordinates_reads_that_cell() {
    let dir = tempfile::tempdir().unwrap();
    let p = hrrr_processor(dir.path(), "hrrr");
    for (col, row, lat, lon) in FILE_CELLS {
        let want = (row * NX + col) as f32;
        // a caller sends -180..180; the file's own longitudes are 0..360
        for lon in [lon - 360.0, lon] {
            let got =
                p.read_point(lon, lat).await.unwrap().unwrap_or_else(|| {
                    panic!("cell ({col},{row}) at ({lon},{lat}) read as outside")
                });
            assert_eq!(
                got, want,
                "cell ({col},{row}) at ({lon},{lat}) read cell id {got}"
            );
        }
    }
}

#[tokio::test]
async fn the_15_minute_model_reads_the_same_grid() {
    let dir = tempfile::tempdir().unwrap();
    let p = hrrr_processor(dir.path(), "hrrr-subh");
    let (col, row, lat, lon) = FILE_CELLS[7];
    assert_eq!(
        p.read_point(lon - 360.0, lat).await.unwrap(),
        Some((row * NX + col) as f32)
    );
}

#[tokio::test]
async fn fractional_positions_agree_with_pyproj() {
    // cell id = row * 1799 + col is linear in the grid coordinates, so a bilinear read at
    // fractional (col, row) is row * 1799 + col, and being 0.01 of a cell out moves it by ~18
    let dir = tempfile::tempdir().unwrap();
    let p = hrrr_processor(dir.path(), "hrrr");
    for (name, lon, lat, col, row) in [
        ("Denver", -104.99, 39.74, 685.8767, 583.7515),
        ("Seattle", -122.33, 47.61, 277.9904, 952.5597),
        ("Miami", -80.19, 25.76, 1487.3499, 108.5716),
        ("Chicago", -87.63, 41.88, 1171.5206, 669.0766),
        ("NYC", -74.0, 40.71, 1552.7939, 694.9385),
        // inside the grid at 52 N; the old bounding box (to 47.8 N) called this outside
        ("northern Montana-ish", -97.55, 52.0, 897.9975, 1034.4482),
    ] {
        let want = row * NX as f64 + col;
        let got = p
            .read_point(lon, lat)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{name} read as outside the grid")) as f64;
        assert!(
            (got - want).abs() < 20.0,
            "{name}: read {got}, pyproj puts it at {want} (cell {col:.2}, {row:.2})"
        );
    }
}

#[tokio::test]
async fn points_outside_the_grid_are_outside_even_when_inside_the_old_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let p = hrrr_processor(dir.path(), "hrrr");
    for (name, lon, lat) in [
        ("Hawaii", -155.5, 19.7),
        ("Mexico City", -99.1, 19.4),
        ("mid Atlantic", -50.0, 40.0),
        ("Arctic", -97.5, 70.0),
        // inside the old longitude/latitude envelope (-134..-61, 21.1..47.8) but south of the
        // grid's bottom edge, which curves up to 24.4 N at the middle column
        ("Gulf of Mexico", -97.5, 22.0),
    ] {
        assert_eq!(p.read_point(lon, lat).await.unwrap(), None, "{name}");
    }
}

#[tokio::test]
async fn the_outer_half_cell_belongs_to_the_grid_and_reads_the_edge_cell() {
    let dir = tempfile::tempdir().unwrap();
    let p = hrrr_processor(dir.path(), "hrrr");
    // bottom-left cell centre is (21.138123, -122.719528); 1 km (a third of a cell, 3 km grid)
    // south-west of it is still inside that cell's footprint ...
    let got = p
        .read_point(-122.719528 - 0.011, 21.138123 - 0.009)
        .await
        .unwrap();
    assert_eq!(got, Some(0.0), "edge cell, clamped");
    // ... and 10 km (more than 3 cells) beyond is not
    assert_eq!(
        p.read_point(-122.719528 - 0.12, 21.138123 - 0.09)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn nearest_reads_use_the_same_location() {
    let dir = tempfile::tempdir().unwrap();
    let p = hrrr_processor(dir.path(), "hrrr");
    let (col, row, lat, lon) = FILE_CELLS[4];
    assert_eq!(
        p.read_point_nearest(lon - 360.0, lat).await.unwrap(),
        Some((row * NX + col) as f32)
    );
}
