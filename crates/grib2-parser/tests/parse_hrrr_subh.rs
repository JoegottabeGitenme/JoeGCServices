//! HRRR sub-hourly (`wrfsubhf`) products are labelled in minutes. These are real messages
//! from `hrrr.t18z.wrfsubhf{00,01,02}.grib2` of 2026-10-11, fetched by byte range from NOMADS
//! (REFC, PRATE, CRAIN, CSNOW, CFRZR, CICEP), so the test reads bytes NCEP wrote.

mod common;

use bytes::Bytes;
use chrono::{Duration, TimeZone, Utc};
use common::create_test_tables;
use grib2_parser::Grib2Reader;
use std::path::PathBuf;

fn fixture(name: &str) -> Bytes {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ingestion/tests/fixtures/hrrr-subh")
        .join(name);
    Bytes::from(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

fn read_all(name: &str) -> Vec<grib2_parser::Grib2Message> {
    let mut reader = Grib2Reader::new(fixture(name), create_test_tables());
    let mut out = Vec::new();
    while let Some(m) = reader.next_message().expect("message parses") {
        out.push(m);
    }
    out
}

#[test]
fn subhourly_messages_are_minutes_not_hours() {
    let msgs = read_all("hrrr-subh_20261011_18z_f001.grib2");
    assert_eq!(msgs.len(), 24, "4 steps x 6 parameters");
    // idx order: all six parameters for 15 min, then for 30, 45, 60
    let leads: Vec<u32> = msgs
        .iter()
        .map(|m| m.product_definition.lead_minutes)
        .collect();
    let want: Vec<u32> = [15u32, 30, 45, 60]
        .iter()
        .flat_map(|l| std::iter::repeat_n(*l, 6))
        .collect();
    assert_eq!(leads, want);
    // the old reading would have said 15, 30, 45 and 60 *hours*
    let hours: Vec<u32> = msgs
        .iter()
        .map(|m| m.product_definition.forecast_hour)
        .collect();
    assert_eq!(&hours[..6], &[0; 6], "15 minutes is hour 0");
    assert_eq!(&hours[18..], &[1; 6], "60 minutes is hour 1");
}

#[test]
fn valid_time_is_the_cycle_plus_the_lead_in_minutes() {
    let cycle = Utc.with_ymd_and_hms(2026, 10, 11, 18, 0, 0).unwrap();
    let msgs = read_all("hrrr-subh_20261011_18z_f001.grib2");
    assert!(msgs
        .iter()
        .all(|m| m.identification.reference_time == cycle));
    for m in &msgs {
        assert_eq!(
            m.valid_time(),
            cycle + Duration::minutes(m.product_definition.lead_minutes as i64)
        );
    }
    assert_eq!(
        msgs[0].valid_time(),
        Utc.with_ymd_and_hms(2026, 10, 11, 18, 15, 0).unwrap()
    );
    assert_eq!(
        msgs[23].valid_time(),
        Utc.with_ymd_and_hms(2026, 10, 11, 19, 0, 0).unwrap()
    );
}

#[test]
fn the_lead_is_counted_from_the_cycle_not_from_the_file_hour() {
    // wrfsubhf02 holds 75, 90, 105 and 120 minutes after the 18Z cycle
    let msgs = read_all("hrrr-subh_20261011_18z_f002.grib2");
    assert_eq!(msgs.len(), 1);
    let pd = &msgs[0].product_definition;
    assert_eq!(pd.lead_minutes, 75);
    assert_eq!(pd.forecast_hour, 1);
    assert_eq!(
        msgs[0].valid_time(),
        Utc.with_ymd_and_hms(2026, 10, 11, 19, 15, 0).unwrap()
    );
}

#[test]
fn the_analysis_file_is_lead_zero() {
    let msgs = read_all("hrrr-subh_20261011_18z_f000.grib2");
    assert_eq!(msgs.len(), 6);
    assert!(msgs.iter().all(|m| m.product_definition.lead_minutes == 0));
    assert_eq!(msgs[0].parameter(), "REFC");
}

#[test]
fn hourly_hrrr_is_unchanged() {
    // wrfprsf files use unit 1 (hours): value 2 is 120 minutes, hour 2, exactly as before
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ingestion/tests/fixtures/hrrr/hrrr_20261009_23z_f002.grib2");
    let mut reader = Grib2Reader::new(
        Bytes::from(std::fs::read(path).expect("fixture")),
        create_test_tables(),
    );
    let mut n = 0;
    while let Some(m) = reader.next_message().unwrap() {
        assert_eq!(m.product_definition.forecast_hour, 2);
        assert_eq!(m.product_definition.lead_minutes, 120);
        assert_eq!(
            m.valid_time(),
            Utc.with_ymd_and_hms(2026, 10, 10, 1, 0, 0).unwrap()
        );
        n += 1;
    }
    assert_eq!(n, 4);
}
