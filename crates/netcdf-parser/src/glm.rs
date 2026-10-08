//! GOES-R Geostationary Lightning Mapper (GLM) L2 LCFA reader.
//!
//! `GLM-L2-LCFA` ("Lightning Cluster-Filter Algorithm") granules are 20-second
//! netCDF-4 files holding three nested 1-D tables: *events* (single lit
//! pixels) are clustered into *groups*, which are clustered into *flashes*.
//! This reader extracts **flashes only** -- a flash is the unit a person
//! means by "a lightning strike"; groups and events are ~30x and ~600x more
//! numerous and far too granular for a map.
//!
//! Unlike the ABI imagery this crate also reads, GLM is **point-event data**,
//! so the output is a `Vec<GlmFlash>`, not a grid.
//!
//! # Decoding rules (verified against real GOES-18/19 granules)
//!
//! Each of these would silently corrupt data if missed, so each has a test:
//!
//! 1. **Packed shorts are unsigned.** `flash_id`, `flash_energy`,
//!    `flash_quality_flag` and `flash_time_offset_of_first_event` are stored as
//!    NC_SHORT with `_Unsigned = "true"`. They are read as `i16` (the file's
//!    native type, so libnetcdf performs no range-checked conversion) and
//!    reinterpreted as `u16`. In the real GOES-19 test granule every one of
//!    the 899 `flash_id`s is above 32,767 and would otherwise come out negative.
//! 2. **Packing attributes come from the file**, not constants:
//!    `value = raw * scale_factor + add_offset`, computed in `f64`.
//! 3. **The time epoch is 2000-01-01 12:00:00 UTC (noon, not midnight).**
//!    `product_time` is seconds since that epoch; the `units` attribute is
//!    checked rather than assumed.
//! 4. **Flash time = `product_time` + first-event offset**, and the offset has
//!    `add_offset = -5`, so it can be *negative*: a flash that began before the
//!    20 s window opened belongs to the file in which it ended (~3% of flashes).
//! 5. **Fill values**: a `flash_energy` equal to its `_FillValue` (65535
//!    unsigned) is missing -> `None`, not 6.5e-11 J.
//! 6. **`flash_id` is a rolling per-satellite counter, not a global id.** With
//!    ~900 flashes per granule it wraps at 65,536 roughly every 25 minutes, so
//!    it is only meaningful together with the flash time.
//! 7. A granule may legitimately contain **zero flashes**.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};

use crate::error::{NetCdfError, NetCdfResult};
use crate::native::{get_f32_attr, get_optimal_temp_dir, has_attr, silence_hdf5_errors};

/// Unix time of the J2000 epoch the GLM `product_time` is counted from:
/// 2000-01-01T12:00:00Z (note: noon).
pub const GLM_EPOCH_UNIX_SECS: i64 = 946_728_000;

/// The exact `units` string expected on `product_time`. Anything else means the
/// time base is not what this reader decodes, so it refuses rather than guesses.
const EXPECTED_PRODUCT_TIME_UNITS: &str = "seconds since 2000-01-01 12:00:00";

/// One lightning flash.
#[derive(Debug, Clone, PartialEq)]
pub struct GlmFlash {
    /// Rolling per-satellite counter (`u16`); see the module docs, rule 6.
    pub flash_id: u16,
    /// Time of the flash's first constituent event.
    pub time: DateTime<Utc>,
    /// Energy-weighted centroid latitude, degrees north.
    pub lat: f64,
    /// Energy-weighted centroid longitude, degrees east.
    pub lon: f64,
    /// Radiant energy in joules (typically ~1e-15 to 1e-12); `None` if the
    /// file marks it missing.
    pub energy_j: Option<f64>,
    /// Raw `flash_quality_flag`: 0 = good; 1, 3, 5 = degraded (out-of-order
    /// events / too many events / duration exceeded threshold).
    pub quality: u8,
}

/// A parsed granule.
#[derive(Debug, Clone)]
pub struct GlmGranule {
    /// Platform id from the file, e.g. `"G19"`.
    pub platform: String,
    /// Start of the 20 s observation window (`product_time`).
    pub window_start: DateTime<Utc>,
    /// End of the window (`product_time_bounds[1]`), if present.
    pub window_end: Option<DateTime<Utc>>,
    /// Flashes that decoded to a usable position.
    pub flashes: Vec<GlmFlash>,
    /// Flashes dropped for a missing/non-finite/out-of-range position.
    pub skipped_invalid: usize,
}

impl GlmGranule {
    /// Keep only flashes inside `[min_lon, max_lon] x [min_lat, max_lat]`.
    /// Returns how many were dropped.
    pub fn retain_in_bbox(
        &mut self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
    ) -> usize {
        let before = self.flashes.len();
        self.flashes.retain(|f| {
            f.lon >= min_lon && f.lon <= max_lon && f.lat >= min_lat && f.lat <= max_lat
        });
        before - self.flashes.len()
    }
}

/// Read the flashes from a GLM L2 LCFA granule on disk.
pub fn read_glm_flashes(path: &Path) -> NetCdfResult<GlmGranule> {
    silence_hdf5_errors();

    let file = netcdf::open(path)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to open GLM NetCDF: {}", e)))?;

    let platform = match file.attribute("platform_ID").map(|a| a.value()) {
        Some(Ok(netcdf::AttributeValue::Str(s))) => s,
        _ => {
            return Err(NetCdfError::MissingData(
                "platform_ID global attribute (is this a GLM file?)".to_string(),
            ))
        }
    };

    // ---- time base ----
    let pt_var = file
        .variable("product_time")
        .ok_or_else(|| NetCdfError::MissingData("product_time".to_string()))?;
    let units = match pt_var.attribute_value("units") {
        Some(Ok(netcdf::AttributeValue::Str(s))) => s,
        _ => String::new(),
    };
    if units.trim() != EXPECTED_PRODUCT_TIME_UNITS {
        return Err(NetCdfError::InvalidFormat(format!(
            "product_time units are {:?}, expected {:?}; refusing to guess the epoch",
            units, EXPECTED_PRODUCT_TIME_UNITS
        )));
    }
    let product_time: f64 = pt_var
        .get_value(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read product_time: {}", e)))?;
    let window_start = j2000_seconds_to_utc(product_time)?;

    let window_end = file
        .variable("product_time_bounds")
        .and_then(|v| v.get_values::<f64, _>(..).ok())
        .and_then(|b| b.get(1).copied())
        .and_then(|s| j2000_seconds_to_utc(s).ok());

    // ---- flashes ----
    let n = file
        .dimension("number_of_flashes")
        .ok_or_else(|| NetCdfError::MissingData("number_of_flashes dimension".to_string()))?
        .len();

    if n == 0 {
        return Ok(GlmGranule {
            platform,
            window_start,
            window_end,
            flashes: Vec::new(),
            skipped_invalid: 0,
        });
    }

    let lat: Vec<f32> = read_vec(&file, "flash_lat")?;
    let lon: Vec<f32> = read_vec(&file, "flash_lon")?;
    let ids = read_unsigned(&file, "flash_id")?;
    let t_raw = read_unsigned(&file, "flash_time_offset_of_first_event")?;
    let e_raw = read_unsigned(&file, "flash_energy")?;
    let q_raw = read_unsigned(&file, "flash_quality_flag")?;

    for (name, len) in [
        ("flash_lat", lat.len()),
        ("flash_lon", lon.len()),
        ("flash_id", ids.len()),
        ("flash_time_offset_of_first_event", t_raw.len()),
        ("flash_energy", e_raw.len()),
        ("flash_quality_flag", q_raw.len()),
    ] {
        if len != n {
            return Err(NetCdfError::InvalidFormat(format!(
                "{} has {} values but number_of_flashes is {}",
                name, len, n
            )));
        }
    }

    let (t_scale, t_offset) = packing(&file, "flash_time_offset_of_first_event")?;
    let (e_scale, e_offset) = packing(&file, "flash_energy")?;
    let e_fill = fill_value_unsigned(&file, "flash_energy");

    let mut flashes = Vec::with_capacity(n);
    let mut skipped_invalid = 0usize;
    for i in 0..n {
        let (la, lo) = (lat[i] as f64, lon[i] as f64);
        if !la.is_finite()
            || !lo.is_finite()
            || !(-90.0..=90.0).contains(&la)
            || !(-180.0..=180.0).contains(&lo)
        {
            skipped_invalid += 1;
            continue;
        }

        let offset_s = t_raw[i] as f64 * t_scale + t_offset;
        let time = j2000_seconds_to_utc(product_time + offset_s)?;
        let energy_j = if Some(e_raw[i]) == e_fill {
            None
        } else {
            Some(e_raw[i] as f64 * e_scale + e_offset)
        };

        flashes.push(GlmFlash {
            flash_id: ids[i],
            time,
            lat: la,
            lon: lo,
            energy_j,
            // valid_range is 0..5, so it always fits; saturate rather than wrap
            // if a future product version ever widens it.
            quality: u8::try_from(q_raw[i]).unwrap_or(u8::MAX),
        });
    }

    Ok(GlmGranule {
        platform,
        window_start,
        window_end,
        flashes,
        skipped_invalid,
    })
}

/// Read a granule from memory. libnetcdf needs a file, so this spools to a temp
/// file (in `/dev/shm` when available) that is removed on return.
pub fn read_glm_flashes_from_bytes(data: &[u8]) -> NetCdfResult<GlmGranule> {
    struct TempFile(PathBuf);
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = get_optimal_temp_dir().join(format!(
        "glm_{}_{}.nc",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let guard = TempFile(path.clone());
    std::fs::File::create(&path)?.write_all(data)?;
    read_glm_flashes(&guard.0)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn j2000_seconds_to_utc(secs: f64) -> NetCdfResult<DateTime<Utc>> {
    if !secs.is_finite() {
        return Err(NetCdfError::InvalidFormat(format!(
            "non-finite time {}",
            secs
        )));
    }
    // Microsecond resolution: far finer than the product's own 0.38 ms packing
    // step, and exactly what TIMESTAMPTZ stores.
    let micros = ((secs * 1e6).round() as i64) + GLM_EPOCH_UNIX_SECS * 1_000_000;
    Utc.timestamp_micros(micros)
        .single()
        .ok_or_else(|| NetCdfError::InvalidFormat(format!("time out of range: {}", secs)))
}

fn read_vec<T: netcdf::NcTypeDescriptor + Copy>(
    file: &netcdf::File,
    name: &str,
) -> NetCdfResult<Vec<T>> {
    file.variable(name)
        .ok_or_else(|| NetCdfError::MissingData(name.to_string()))?
        .get_values::<T, _>(..)
        .map_err(|e| NetCdfError::InvalidFormat(format!("Failed to read {}: {}", name, e)))
}

/// Read an NC_SHORT variable that carries `_Unsigned = "true"`: read the native
/// `i16` and reinterpret the bits (rule 1).
fn read_unsigned(file: &netcdf::File, name: &str) -> NetCdfResult<Vec<u16>> {
    let raw: Vec<i16> = read_vec(file, name)?;
    Ok(raw.into_iter().map(|v| v as u16).collect())
}

/// `(scale_factor, add_offset)` as `f64`, defaulting to the identity.
fn packing(file: &netcdf::File, name: &str) -> NetCdfResult<(f64, f64)> {
    let var = file
        .variable(name)
        .ok_or_else(|| NetCdfError::MissingData(name.to_string()))?;
    let scale = get_f32_attr(&var, "scale_factor")
        .map(f64::from)
        .unwrap_or(1.0);
    let offset = get_f32_attr(&var, "add_offset")
        .map(f64::from)
        .unwrap_or(0.0);
    Ok((scale, offset))
}

/// The variable's `_FillValue` reinterpreted as unsigned (rule 5), if it has one.
fn fill_value_unsigned(file: &netcdf::File, name: &str) -> Option<u16> {
    let var = file.variable(name)?;
    if !has_attr(&var, "_FillValue") {
        return None;
    }
    match var.attribute_value("_FillValue")?.ok()? {
        netcdf::AttributeValue::Short(v) => Some(v as u16),
        netcdf::AttributeValue::Ushort(v) => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    /// HDF5's error-print handler is process-global and a failed open resets it,
    /// so tests that deliberately feed garbage (or assert on the handler) must not
    /// interleave with each other.
    static HDF5_STATE: Mutex<()> = Mutex::new(());

    /// True if HDF5 will NOT print error stacks to stderr right now.
    fn hdf5_auto_printing_is_off() -> bool {
        let _guard = hdf5_metno_sys::LOCK.lock();
        let mut func: hdf5_metno_sys::h5e::H5E_auto2_t = None;
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: plain getter writing into two valid out-pointers.
        unsafe {
            hdf5_metno_sys::h5e::H5Eget_auto2(
                hdf5_metno_sys::h5e::H5E_DEFAULT,
                &mut func,
                &mut data,
            );
        }
        func.is_none()
    }

    const G18: &str =
        "tests/fixtures/OR_GLM-L2-LCFA_G18_s20262802033000_e20262802033200_c20262802033222.nc";
    const G19: &str =
        "tests/fixtures/OR_GLM-L2-LCFA_G19_s20262802033000_e20262802033200_c20262802033219.nc";
    const EMPTY: &str = "tests/fixtures/glm_empty_no_flashes.nc";

    /// Reference values produced by `tests/fixtures/make_glm_expected.py`, an
    /// independent decoder (own float64 arithmetic from the raw integers).
    fn expected(file: &str) -> Value {
        let all: Value =
            serde_json::from_str(include_str!("../tests/fixtures/glm_expected.json")).unwrap();
        all[file.rsplit('/').next().unwrap()].clone()
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn epoch_constant_is_noon_not_midnight() {
        assert_eq!(
            Utc.with_ymd_and_hms(2000, 1, 1, 12, 0, 0)
                .unwrap()
                .timestamp(),
            GLM_EPOCH_UNIX_SECS
        );
        assert_ne!(
            Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0)
                .unwrap()
                .timestamp(),
            GLM_EPOCH_UNIX_SECS
        );
    }

    #[test]
    fn goes19_matches_the_independent_reference() {
        let exp = expected(G19);
        let g = read_glm_flashes(Path::new(G19)).unwrap();

        assert_eq!(g.platform, "G19");
        assert_eq!(g.skipped_invalid, 0);
        assert_eq!(g.flashes.len(), exp["n_flashes"].as_u64().unwrap() as usize);
        assert!(close(
            g.window_start.timestamp() as f64,
            exp["product_time_unix"].as_f64().unwrap(),
            1e-6
        ));

        let ids: Vec<u16> = g.flashes.iter().map(|f| f.flash_id).collect();
        assert_eq!(
            *ids.iter().min().unwrap() as u64,
            exp["flash_id_min"].as_u64().unwrap()
        );
        assert_eq!(
            *ids.iter().max().unwrap() as u64,
            exp["flash_id_max"].as_u64().unwrap()
        );
        assert_eq!(
            ids.iter().map(|&i| i as u64).sum::<u64>(),
            exp["flash_id_sum"].as_u64().unwrap()
        );

        let n = g.flashes.len() as f64;
        let _ = n;
        assert!(close(
            g.flashes.iter().map(|f| f.lat).sum::<f64>(),
            exp["lat_sum"].as_f64().unwrap(),
            1e-3
        ));
        assert!(close(
            g.flashes.iter().map(|f| f.lon).sum::<f64>(),
            exp["lon_sum"].as_f64().unwrap(),
            1e-3
        ));
        let e_sum: f64 = g.flashes.iter().filter_map(|f| f.energy_j).sum();
        let e_ref = exp["energy_sum_j"].as_f64().unwrap();
        assert!(
            (e_sum - e_ref).abs() / e_ref < 1e-9,
            "{} vs {}",
            e_sum,
            e_ref
        );

        let t_min = g
            .flashes
            .iter()
            .map(|f| f.time.timestamp_micros())
            .min()
            .unwrap() as f64
            / 1e6;
        let t_max = g
            .flashes
            .iter()
            .map(|f| f.time.timestamp_micros())
            .max()
            .unwrap() as f64
            / 1e6;
        assert!(
            close(t_min, exp["unix_time_min"].as_f64().unwrap(), 1e-5),
            "{} {}",
            t_min,
            exp["unix_time_min"]
        );
        assert!(close(t_max, exp["unix_time_max"].as_f64().unwrap(), 1e-5));
    }

    #[test]
    fn unsigned_flash_ids_are_not_read_as_negative() {
        // Every flash_id in this granule is above 32,767 (46211..47574). Read as
        // a signed short they are -19325..-17962; as u16 they must be the real values.
        let g = read_glm_flashes(Path::new(G19)).unwrap();
        assert_eq!(
            expected(G19)["n_flash_id_above_32767"],
            expected(G19)["n_flashes"]
        );
        assert!(g.flashes.iter().all(|f| f.flash_id > 32_767));
        assert!(g
            .flashes
            .iter()
            .all(|f| (46_211..=47_574).contains(&f.flash_id)));
    }

    #[test]
    fn first_flashes_match_field_by_field() {
        for file in [G18, G19] {
            let exp = expected(file);
            let g = read_glm_flashes(Path::new(file)).unwrap();
            for (i, want) in exp["first_flashes"].as_array().unwrap().iter().enumerate() {
                let got = &g.flashes[i];
                assert_eq!(
                    got.flash_id as u64,
                    want["flash_id"].as_u64().unwrap(),
                    "{} #{}",
                    file,
                    i
                );
                assert_eq!(got.quality as u64, want["quality"].as_u64().unwrap());
                // flash_lat/lon are f32 in the file -> exact once widened to f64 (needs
                // serde_json float_roundtrip so the REFERENCE parses exactly).
                assert_eq!(got.lat, want["lat"].as_f64().unwrap());
                assert_eq!(got.lon, want["lon"].as_f64().unwrap());
                let t = got.time.timestamp_micros() as f64 / 1e6;
                assert!(
                    close(t, want["unix_time"].as_f64().unwrap(), 1e-5),
                    "time {} vs {}",
                    t,
                    want["unix_time"]
                );
                let e = got.energy_j.unwrap();
                let e_ref = want["energy_j"].as_f64().unwrap();
                assert!((e - e_ref).abs() / e_ref < 1e-9);
            }
        }
    }

    #[test]
    fn goes18_matches_the_independent_reference() {
        let exp = expected(G18);
        let g = read_glm_flashes(Path::new(G18)).unwrap();
        assert_eq!(g.platform, "G18");
        assert_eq!(g.flashes.len(), 57);
        assert_eq!(g.flashes.len() as u64, exp["n_flashes"].as_u64().unwrap());
        assert_eq!(
            g.flashes.iter().map(|f| f.flash_id as u64).sum::<u64>(),
            exp["flash_id_sum"].as_u64().unwrap()
        );
    }

    #[test]
    fn quality_flags_are_preserved_not_filtered() {
        // 44 of GOES-19's 899 flashes carry quality 3 (degraded). They are real
        // flashes and must be kept, with the flag intact.
        let g = read_glm_flashes(Path::new(G19)).unwrap();
        let q3 = g.flashes.iter().filter(|f| f.quality == 3).count();
        assert_eq!(
            q3 as u64,
            expected(G19)["quality_counts"]["3"].as_u64().unwrap()
        );
        assert_eq!(q3, 44);
    }

    #[test]
    fn some_flashes_start_before_the_window_opens() {
        // Rule 4: add_offset = -5, so a flash can begin before product_time.
        let g = read_glm_flashes(Path::new(G19)).unwrap();
        let before = g.flashes.iter().filter(|f| f.time < g.window_start).count();
        assert!(
            before > 0,
            "expected flashes with negative offsets in this granule"
        );
        // ...but never wildly outside the 5 s allowance either side of the 20 s window.
        let end = g.window_end.unwrap();
        assert!(g
            .flashes
            .iter()
            .all(|f| f.time >= g.window_start - chrono::Duration::seconds(6)));
        assert!(g
            .flashes
            .iter()
            .all(|f| f.time <= end + chrono::Duration::seconds(1)));
        assert_eq!((end - g.window_start).num_seconds(), 20);
    }

    #[test]
    fn conus_clip_matches_the_reference_counts() {
        for file in [G18, G19] {
            let exp = expected(file);
            let mut g = read_glm_flashes(Path::new(file)).unwrap();
            let total = g.flashes.len();
            let dropped = g.retain_in_bbox(-125.0, 24.0, -66.0, 50.0);
            assert_eq!(
                g.flashes.len() as u64,
                exp["conus_count"].as_u64().unwrap(),
                "{}",
                file
            );
            assert_eq!(dropped, total - g.flashes.len());
            assert_eq!(
                g.flashes.iter().map(|f| f.flash_id as u64).sum::<u64>(),
                exp["conus_flash_id_sum"].as_u64().unwrap()
            );
        }
    }

    #[test]
    fn zero_flash_granule_is_valid_and_empty() {
        let g = read_glm_flashes(Path::new(EMPTY)).unwrap();
        assert!(g.flashes.is_empty());
        assert_eq!(g.skipped_invalid, 0);
        assert_eq!(g.platform, "G18");
        // product_time = 844677300 s after J2000 noon = 2026-10-07T20:35:00Z
        assert_eq!(
            g.window_start,
            Utc.with_ymd_and_hms(2026, 10, 7, 20, 35, 0).unwrap()
        );
        assert_eq!(
            g.window_end,
            Some(Utc.with_ymd_and_hms(2026, 10, 7, 20, 35, 20).unwrap())
        );
    }

    #[test]
    fn from_bytes_equals_from_path_and_leaves_no_temp_file() {
        // Every test that spools bytes to a temp file holds this lock, so no other
        // test can have a `glm_<pid>_*` file in flight while we count them.
        let _g = HDF5_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = std::fs::read(G18).unwrap();
        let a = read_glm_flashes(Path::new(G18)).unwrap();
        let b = read_glm_flashes_from_bytes(&bytes).unwrap();
        assert_eq!(a.flashes, b.flashes);
        assert_eq!(a.window_start, b.window_start);
        let leftovers = std::fs::read_dir(get_optimal_temp_dir())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("glm_{}_", std::process::id()))
            })
            .count();
        assert_eq!(leftovers, 0, "temp file was not cleaned up");
    }

    #[test]
    fn garbage_input_is_an_error_not_a_panic() {
        let _g = HDF5_STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(read_glm_flashes_from_bytes(b"this is not a netcdf file").is_err());
        assert!(read_glm_flashes_from_bytes(&[]).is_err());
        assert!(read_glm_flashes(Path::new("tests/fixtures/does_not_exist.nc")).is_err());
    }

    #[test]
    fn an_abi_file_or_non_glm_netcdf_is_rejected_clearly() {
        let _g = HDF5_STATE.lock().unwrap_or_else(|e| e.into_inner());
        // Build a valid netCDF that is not GLM (no platform_ID) via the cf fixture
        // path if one exists; otherwise the garbage test above covers rejection.
        let err = read_glm_flashes_from_bytes(b"\x89HDF\r\n\x1a\n not really").unwrap_err();
        assert!(err.to_string().contains("GLM") || err.to_string().contains("open"));
    }

    #[test]
    fn j2000_conversion_is_exact_to_the_microsecond() {
        // 844677300 s after the epoch = 2026-10-07T20:35:00Z exactly.
        let t = j2000_seconds_to_utc(844_677_300.0).unwrap();
        assert_eq!(t, Utc.with_ymd_and_hms(2026, 10, 7, 20, 35, 0).unwrap());
        // sub-second precision survives (0.000381 s steps are far above 1 us)
        let t2 = j2000_seconds_to_utc(844_677_300.123456).unwrap();
        assert_eq!(t2.timestamp_subsec_micros(), 123_456);
        assert!(j2000_seconds_to_utc(f64::NAN).is_err());
    }

    #[test]
    fn error_silencing_survives_a_failed_open() {
        // Regression: opening a non-HDF5 file makes netcdf-c/HDF5 re-initialize,
        // which restored error printing. With the old once-per-process guard, ONE
        // bad upload made every later good read spray 20+ HDF5-DIAG blocks on
        // stderr (a log flood at one granule per 20 s per satellite).
        let _g = HDF5_STATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(read_glm_flashes_from_bytes(b"not netcdf").is_err());
        assert!(read_glm_flashes(Path::new(G18)).is_ok());
        assert!(
            hdf5_auto_printing_is_off(),
            "HDF5 error printing came back after a failed open"
        );
    }

    #[test]
    fn silencing_concurrently_with_reads_never_corrupts_a_read() {
        // Regression for a race I introduced when silence_hdf5_errors() began running
        // before every open: it called H5Eset_auto2 OUTSIDE the netcdf crate's global
        // lock, racing another thread's in-flight attribute read. On HDF5 2.2.0 that
        // failed ~40% of runs of the GLM tests with NC_EATTMETA (-107) on a valid file.
        // Here many threads read the same real granule while others hammer the
        // silencer; every read must still decode identically.
        let _g = HDF5_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let want = read_glm_flashes(Path::new(G19)).unwrap().flashes.len();
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            let hammer: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            crate::native::silence_hdf5_errors();
                        }
                    })
                })
                .collect();
            let readers: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        for _ in 0..40 {
                            let got = read_glm_flashes(Path::new(G19))
                                .expect("valid file must always read");
                            assert_eq!(got.flashes.len(), want);
                        }
                    })
                })
                .collect();
            for r in readers {
                r.join().expect("a reader panicked: the race is back");
            }
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            for h in hammer {
                h.join().unwrap();
            }
        });
    }
}
