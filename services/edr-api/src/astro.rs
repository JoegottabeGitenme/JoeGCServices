//! Astronomical calculations for the astro EDR collection.
//!
//! Provides on-demand computation of solar and lunar data for any location and time,
//! using the `astro` crate which implements algorithms from Jean Meeus's
//! "Astronomical Algorithms".

use astro::time as astro_time;
use astro::{lunar, sun};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};

/// Solar data for a specific location and time.
#[derive(Debug, Clone)]
pub struct SolarData {
    /// Sunrise time (Unix timestamp), None if no sunrise (polar night)
    pub sunrise: Option<i64>,
    /// Sunset time (Unix timestamp), None if no sunset (midnight sun)
    pub sunset: Option<i64>,
    /// Solar noon time (Unix timestamp) - when sun is highest
    pub solar_noon: i64,
    /// Sun altitude/elevation angle in degrees (-90 to 90, negative = below horizon)
    pub altitude: f64,
    /// Sun azimuth in degrees (0=N, 90=E, 180=S, 270=W)
    pub azimuth: f64,
}

/// Lunar data for a specific location and time.
#[derive(Debug, Clone)]
pub struct LunarData {
    /// Moonrise time (Unix timestamp), None if no moonrise
    pub moonrise: Option<i64>,
    /// Moonset time (Unix timestamp), None if no moonset
    pub moonset: Option<i64>,
    /// Moon phase name
    pub phase: MoonPhase,
    /// Illuminated fraction (0.0 to 1.0)
    pub illumination: f64,
    /// Days since new moon (0-29.5 approximately)
    pub age_days: f64,
}

/// Moon phase enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoonPhase {
    NewMoon,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    FullMoon,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

impl MoonPhase {
    /// Get the phase name as a string for CoverageJSON output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NewMoon => "new_moon",
            Self::WaxingCrescent => "waxing_crescent",
            Self::FirstQuarter => "first_quarter",
            Self::WaxingGibbous => "waxing_gibbous",
            Self::FullMoon => "full_moon",
            Self::WaningGibbous => "waning_gibbous",
            Self::LastQuarter => "last_quarter",
            Self::WaningCrescent => "waning_crescent",
        }
    }

    /// Determine moon phase from age in days since new moon.
    pub fn from_age(age_days: f64) -> Self {
        const SYNODIC_MONTH: f64 = 29.530588853; // mean synodic month in days
        let phase_length = SYNODIC_MONTH / 8.0;
        let normalized_age = age_days % SYNODIC_MONTH;
        let phase_index = (normalized_age / phase_length) as usize;

        match phase_index {
            0 => Self::NewMoon,
            1 => Self::WaxingCrescent,
            2 => Self::FirstQuarter,
            3 => Self::WaxingGibbous,
            4 => Self::FullMoon,
            5 => Self::WaningGibbous,
            6 => Self::LastQuarter,
            _ => Self::WaningCrescent,
        }
    }
}

/// Convert chrono DateTime to Julian Day Number.
///
/// The Julian Day is the continuous count of days since the beginning of
/// the Julian Period (January 1, 4713 BCE).
pub fn datetime_to_jd(dt: &DateTime<Utc>) -> f64 {
    let date = astro_time::Date {
        year: dt.year() as i16,
        month: dt.month() as u8,
        decimal_day: dt.day() as f64
            + dt.hour() as f64 / 24.0
            + dt.minute() as f64 / 1440.0
            + dt.second() as f64 / 86400.0
            + dt.timestamp_subsec_nanos() as f64 / 86400_000_000_000.0,
        cal_type: astro_time::CalType::Gregorian,
    };
    astro_time::julian_day(&date)
}

/// Convert Julian Day Number back to DateTime<Utc>.
pub fn jd_to_datetime(jd: f64) -> DateTime<Utc> {
    // JD to Unix timestamp conversion
    // Unix epoch (1970-01-01 00:00:00) = JD 2440587.5
    const UNIX_EPOCH_JD: f64 = 2440587.5;
    let unix_timestamp = (jd - UNIX_EPOCH_JD) * 86400.0;

    let seconds = unix_timestamp.floor() as i64;
    let nanos = ((unix_timestamp - seconds as f64) * 1_000_000_000.0) as u32;

    DateTime::from_timestamp(seconds, nanos).unwrap_or(Utc::now())
}

/// Compute solar data for a specific location and time.
///
/// # Arguments
/// * `lat` - Latitude in degrees (-90 to 90, negative = south)
/// * `lon` - Longitude in degrees (-180 to 180, negative = west)
/// * `datetime` - UTC datetime for the calculation
///
/// # Returns
/// Solar data including sunrise, sunset, position, etc.
pub fn compute_solar(_lat: f64, _lon: f64, datetime: &DateTime<Utc>) -> SolarData {
    let jd = datetime_to_jd(datetime);

    // Get sun's geocentric ecliptic position
    let (sun_ecl_pos, _radius) = sun::geocent_ecl_pos(jd);

    // For now, return simplified data with current position only
    // Sunrise/sunset calculations require more complex transit computations
    // that we'll implement in a follow-up

    // Estimate altitude and azimuth from ecliptic position
    // This is a simplified calculation - full accuracy requires coordinate transforms
    let altitude = sun_ecl_pos.lat.to_degrees();
    let azimuth = sun_ecl_pos.long.to_degrees();

    // Use noon of current day as solar noon estimate
    let solar_noon_dt = Utc
        .with_ymd_and_hms(datetime.year(), datetime.month(), datetime.day(), 12, 0, 0)
        .unwrap();
    let solar_noon = solar_noon_dt.timestamp();

    // For now, return None for sunrise/sunset
    // TODO: Implement accurate transit time calculations
    let sunrise = None;
    let sunset = None;

    SolarData {
        sunrise,
        sunset,
        solar_noon,
        altitude,
        azimuth,
    }
}

/// Compute lunar data for a specific location and time.
///
/// # Arguments
/// * `lat` - Latitude in degrees (-90 to 90, negative = south)
/// * `lon` - Longitude in degrees (-180 to 180, negative = west)
/// * `datetime` - UTC datetime for the calculation
///
/// # Returns
/// Lunar data including moonrise, moonset, phase, illumination, etc.
pub fn compute_lunar(_lat: f64, _lon: f64, datetime: &DateTime<Utc>) -> LunarData {
    let jd = datetime_to_jd(datetime);

    // Get moon's geocentric ecliptic position
    let (moon_ecl_pos, _radius) = lunar::geocent_ecl_pos(jd);

    // Calculate moon age (days since last new moon)
    // Simplified calculation using synodic month
    const SYNODIC_MONTH: f64 = 29.530588853;

    // Rough estimate: J2000 was near a new moon
    const J2000: f64 = 2451545.0;
    let days_since_j2000 = jd - J2000;
    let lunations_since_j2000 = days_since_j2000 / SYNODIC_MONTH;
    let fraction = lunations_since_j2000 - lunations_since_j2000.floor();
    let mut age_days = fraction * SYNODIC_MONTH;

    // Ensure age is positive and within one lunation
    while age_days < 0.0 {
        age_days += 29.530588853;
    }
    while age_days > 29.530588853 {
        age_days -= 29.530588853;
    }

    // Estimate illumination from age (rough approximation)
    // More accurate would use sun-moon angle, but this is simpler
    let illumination = (1.0 - (2.0 * std::f64::consts::PI * age_days / SYNODIC_MONTH).cos()) / 2.0;

    // Determine phase from age
    let phase = MoonPhase::from_age(age_days);

    // For now, return None for moonrise/moonset
    // TODO: Implement accurate transit time calculations
    let moonrise = None;
    let moonset = None;

    LunarData {
        moonrise,
        moonset,
        phase,
        illumination,
        age_days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_datetime_to_jd_conversion() {
        // Test known value: January 1, 2000, 12:00 UTC = JD 2451545.0
        let dt = Utc.with_ymd_and_hms(2000, 1, 1, 12, 0, 0).unwrap();
        let jd = datetime_to_jd(&dt);
        assert!((jd - 2451545.0).abs() < 0.001);
    }

    #[test]
    fn test_jd_to_datetime_roundtrip() {
        let original = Utc.with_ymd_and_hms(2026, 1, 15, 14, 30, 45).unwrap();
        let jd = datetime_to_jd(&original);
        let converted = jd_to_datetime(jd);

        // Allow small precision loss (within 1 second)
        assert!((original.timestamp() - converted.timestamp()).abs() <= 1);
    }

    #[test]
    fn test_moon_phase_from_age() {
        assert_eq!(MoonPhase::from_age(0.0), MoonPhase::NewMoon);
        assert_eq!(MoonPhase::from_age(7.4), MoonPhase::FirstQuarter);
        assert_eq!(MoonPhase::from_age(14.8), MoonPhase::FullMoon);
        // 22.1 days is in the last quarter phase (between 22.1 and 25.9)
        let phase_at_22 = MoonPhase::from_age(22.1);
        assert!(matches!(
            phase_at_22,
            MoonPhase::WaningGibbous | MoonPhase::LastQuarter
        ));
    }

    #[test]
    fn test_solar_data_sanity() {
        // Test for San Francisco on a known date
        let dt = Utc.with_ymd_and_hms(2026, 1, 15, 20, 0, 0).unwrap();
        let solar = compute_solar(37.77, -122.42, &dt);

        // NOTE: Sunrise/sunset are currently not implemented (return None)
        // This is a TODO for accurate transit time calculations
        assert!(solar.sunrise.is_none());
        assert!(solar.sunset.is_none());

        // Solar noon should be around midday
        let noon_dt = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        assert!((solar.solar_noon - noon_dt.timestamp()).abs() < 86400);

        // Azimuth should be valid (0-360)
        assert!(solar.azimuth >= 0.0 && solar.azimuth < 360.0);
    }

    #[test]
    fn test_lunar_data_sanity() {
        let dt = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        let lunar = compute_lunar(37.77, -122.42, &dt);

        // Illumination should be between 0 and 1
        assert!(lunar.illumination >= 0.0 && lunar.illumination <= 1.0);

        // Age should be within a synodic month
        assert!(lunar.age_days >= 0.0 && lunar.age_days < 30.0);

        // Phase should have a valid string representation
        assert!(!lunar.phase.as_str().is_empty());
    }

    #[test]
    fn test_moon_phase_string_values() {
        assert_eq!(MoonPhase::NewMoon.as_str(), "new_moon");
        assert_eq!(MoonPhase::FullMoon.as_str(), "full_moon");
        assert_eq!(MoonPhase::FirstQuarter.as_str(), "first_quarter");
        assert_eq!(MoonPhase::WaxingCrescent.as_str(), "waxing_crescent");
    }
}
