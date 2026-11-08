//! HRRR's simulated infrared brightness temperature (SBT124) is the one parameter the platform
//! reads from GRIB2 discipline 3 (space products); every other message went through discipline 0
//! or MRMS's 209. These are real messages from the 12Z cycle of 2026-11-04, cut by wgrib2 to a
//! 21 x 18 window of the 1799 x 1059 grid (the full ones are 1.7 MB) and repacked as template 5.3
//! with a bitmap, which is how NCEP writes them.

mod common;

use bytes::Bytes;
use grib2_parser::{Grib2Message, Grib2Reader, Grib2Tables, LevelDescription};
use std::path::PathBuf;
use std::sync::Arc;

fn tables() -> Arc<Grib2Tables> {
    let mut t = Grib2Tables::new();
    t.add_parameter(3, 192, 2, "SBT124".to_string());
    t.add_level(8, LevelDescription::Static("top of atmosphere".to_string()));
    Arc::new(t)
}

fn message(dir: &str, name: &str, tables: Arc<Grib2Tables>) -> Grib2Message {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ingestion/tests/fixtures")
        .join(dir)
        .join(name);
    let data =
        Bytes::from(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    Grib2Reader::new(data, tables)
        .next_message()
        .expect("message parses")
        .expect("one message")
}

#[test]
fn a_discipline_3_message_is_named_by_its_discipline_category_and_number() {
    let m = message(
        "hrrr-subh",
        "hrrr-subh_20261104_12z_f002_sbt124_small.grib2",
        tables(),
    );
    assert_eq!(m.indicator.discipline, 3);
    assert_eq!(m.parameter(), "SBT124");
    assert_eq!(m.level(), "top of atmosphere");
    assert_eq!(m.product_definition.level_type, 8);
    assert_eq!(
        m.product_definition.lead_minutes, 75,
        "wrfsubhf02, first step"
    );

    // the same numbers without the table entry are an unnamed parameter, not another discipline's
    let m = message(
        "hrrr-subh",
        "hrrr-subh_20261104_12z_f002_sbt124_small.grib2",
        common::create_test_tables(),
    );
    assert_eq!(m.parameter(), "P3_192_2");
}

#[test]
fn bitmap_padding_bits_are_not_values() {
    // 21 x 18 = 378 points = 47.25 bytes of bitmap; the grib crate yields one value per bit, so
    // the 6 padding bits came out as NaNs (the full grid, 1905141 points, gave 1905144)
    for (dir, name) in [
        (
            "hrrr-subh",
            "hrrr-subh_20261104_12z_f002_sbt124_small.grib2",
        ),
        ("hrrr", "hrrr_20261104_12z_f006_sbt124_small.grib2"),
    ] {
        let m = message(dir, name, tables());
        assert_eq!(m.grid_dims(), (18, 21), "{name}");
        let v = m.unpack_data().expect("template 5.3 with a bitmap decodes");
        assert_eq!(v.len(), 21 * 18, "{name}");
        // four undefined points (NaN), the rest Kelvin
        assert_eq!(v.iter().filter(|x| x.is_nan()).count(), 4, "{name}");
        assert!(
            v.iter()
                .filter(|x| !x.is_nan())
                .all(|x| (200.0..300.0).contains(x)),
            "{name}: {v:?}"
        );
    }
}
