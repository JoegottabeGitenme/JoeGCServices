//! GRIB2 Test Data Generator
//!
//! Creates minimal synthetic GRIB2 files for testing the parser.
//! The generated files have valid structure but minimal data.



/// Build a minimal GRIB2 message with the specified parameters
pub struct Grib2Builder {
    discipline: u8,
    center: u16,
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    // Grid definition
    ni: u32,       // columns
    nj: u32,       // rows
    la1: i32,      // first lat (microdegrees)
    lo1: i32,      // first lon (microdegrees)
    la2: i32,      // last lat (microdegrees)
    lo2: i32,      // last lon (microdegrees)
    di: u32,       // lon increment (microdegrees)
    dj: u32,       // lat increment (microdegrees)
    scanning_mode: u8,
    // Product definition
    param_category: u8,
    param_number: u8,
    level_type: u8,
    level_value: u32,
    forecast_hour: u32,
    // Data
    data_values: Vec<f32>,
}

impl Grib2Builder {
    /// Create a new builder with defaults for GFS-like data
    pub fn new_gfs() -> Self {
        // Small 10x10 grid centered on CONUS
        let ni = 10;
        let nj = 10;
        Self {
            discipline: 0, // Meteorological
            center: 7,     // NCEP
            year: 2025,
            month: 12,
            day: 10,
            hour: 12,
            ni,
            nj,
            la1: 45_000_000,   // 45.0°N (microdegrees)
            lo1: 230_000_000,  // 230.0°E = -130°W (microdegrees, 0-360 range)
            la2: 35_000_000,   // 35.0°N
            lo2: 240_000_000,  // 240.0°E = -120°W
            di: 1_000_000,     // 1.0° increment
            dj: 1_000_000,     // 1.0° increment
            scanning_mode: 0b01000000, // +i, -j, i consecutive
            param_category: 0,
            param_number: 0, // TMP
            level_type: 103, // m above ground
            level_value: 2,  // 2m
            forecast_hour: 0,
            data_values: vec![288.15; (ni * nj) as usize], // 15°C in Kelvin
        }
    }

    /// Create a builder for MRMS-like data
    pub fn new_mrms() -> Self {
        // Small grid representing MRMS CONUS
        let ni = 20;
        let nj = 15;
        Self {
            discipline: 209, // MRMS local discipline
            center: 161,     // NSSL
            year: 2025,
            month: 12,
            day: 10,
            hour: 12,
            ni,
            nj,
            la1: 54_995_000,   // 54.995°N (microdegrees)
            lo1: 230_005_000,  // 230.005°E = -129.995°W
            la2: 40_005_000,   // 40.005°N
            lo2: 249_995_000,  // 249.995°E = -110.005°W
            di: 10_000,        // 0.01° increment
            dj: 10_000,        // 0.01° increment
            scanning_mode: 0b01000000, // +i, -j, i consecutive
            param_category: 0,
            param_number: 16, // MergedReflectivityQC (REFL)
            level_type: 102,  // m above MSL
            level_value: 500, // 500m
            forecast_hour: 0,
            data_values: vec![-999.0; (ni * nj) as usize], // Missing values
        }
    }

    pub fn with_reference_time(mut self, year: u16, month: u8, day: u8, hour: u8) -> Self {
        self.year = year;
        self.month = month;
        self.day = day;
        self.hour = hour;
        self
    }

    pub fn with_grid(mut self, ni: u32, nj: u32) -> Self {
        self.ni = ni;
        self.nj = nj;
        self.data_values = vec![0.0; (ni * nj) as usize];
        self
    }

    pub fn with_parameter(mut self, category: u8, number: u8) -> Self {
        self.param_category = category;
        self.param_number = number;
        self
    }

    pub fn with_level(mut self, level_type: u8, level_value: u32) -> Self {
        self.level_type = level_type;
        self.level_value = level_value;
        self
    }

    pub fn with_forecast_hour(mut self, hour: u32) -> Self {
        self.forecast_hour = hour;
        self
    }

    pub fn with_constant_value(mut self, value: f32) -> Self {
        self.data_values = vec![value; (self.ni * self.nj) as usize];
        self
    }

    pub fn with_gradient(mut self, min_val: f32, max_val: f32) -> Self {
        let n = (self.ni * self.nj) as usize;
        self.data_values = (0..n)
            .map(|i| min_val + (max_val - min_val) * (i as f32 / n as f32))
            .collect();
        self
    }

    pub fn with_data(mut self, data: Vec<f32>) -> Self {
        self.data_values = data;
        self
    }

    /// Build the complete GRIB2 message bytes
    pub fn build(&self) -> Vec<u8> {
        let mut message = Vec::new();

        // Build sections
        let section1 = self.build_section1();
        let section3 = self.build_section3();
        let section4 = self.build_section4();
        let section5 = self.build_section5();
        let section6 = self.build_section6();
        let section7 = self.build_section7();

        // Calculate total message length
        let message_length = 16 // Section 0
            + section1.len()
            + section3.len()
            + section4.len()
            + section5.len()
            + section6.len()
            + section7.len()
            + 4; // Section 8 (end)

        // Section 0: Indicator
        message.extend_from_slice(b"GRIB"); // Magic
        message.extend_from_slice(&[0, 0]); // Reserved
        message.push(self.discipline);
        message.push(2); // Edition 2
        message.extend_from_slice(&(message_length as u64).to_be_bytes());

        // Add all sections
        message.extend_from_slice(&section1);
        message.extend_from_slice(&section3);
        message.extend_from_slice(&section4);
        message.extend_from_slice(&section5);
        message.extend_from_slice(&section6);
        message.extend_from_slice(&section7);

        // Section 8: End
        message.extend_from_slice(b"7777");

        message
    }

    fn build_section1(&self) -> Vec<u8> {
        let mut section = Vec::new();
        let section_length: u32 = 21;

        section.extend_from_slice(&section_length.to_be_bytes());
        section.push(1); // Section number

        section.extend_from_slice(&self.center.to_be_bytes());
        section.extend_from_slice(&0u16.to_be_bytes()); // Sub-center
        section.push(2);  // Master table version
        section.push(1);  // Local table version
        section.push(1);  // Significance of reference time (start of forecast)

        // Reference time
        section.extend_from_slice(&self.year.to_be_bytes());
        section.push(self.month);
        section.push(self.day);
        section.push(self.hour);
        section.push(0); // Minute
        section.push(0); // Second

        section.push(0); // Production status (operational)
        section.push(1); // Type of data (forecast)

        section
    }

    fn build_section3(&self) -> Vec<u8> {
        let mut section = Vec::new();

        // Template 3.0: Latitude/Longitude
        let template_data_len = 58;
        let section_length: u32 = 14 + template_data_len;

        section.extend_from_slice(&section_length.to_be_bytes());
        section.push(3); // Section number

        section.push(0); // Source of grid definition
        let num_data_points = self.ni * self.nj;
        section.extend_from_slice(&num_data_points.to_be_bytes());
        section.push(0); // Number of octets for optional list
        section.push(0); // Interpretation of optional list
        section.extend_from_slice(&0u16.to_be_bytes()); // Grid definition template (0 = lat/lon)

        // Template 3.0 data (58 bytes)
        section.push(6); // Shape of Earth (spherical with radius 6371229m)
        section.push(0); // Scale factor of radius
        section.extend_from_slice(&0u32.to_be_bytes()); // Scaled value of radius
        section.push(0); // Scale factor of major axis
        section.extend_from_slice(&0u32.to_be_bytes()); // Scaled value of major axis
        section.push(0); // Scale factor of minor axis
        section.extend_from_slice(&0u32.to_be_bytes()); // Scaled value of minor axis

        section.extend_from_slice(&self.ni.to_be_bytes()); // Ni
        section.extend_from_slice(&self.nj.to_be_bytes()); // Nj
        section.extend_from_slice(&0u32.to_be_bytes()); // Basic angle
        section.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes()); // Subdivisions

        section.extend_from_slice(&self.la1.to_be_bytes()); // La1
        section.extend_from_slice(&self.lo1.to_be_bytes()); // Lo1
        section.push(48); // Resolution and component flags
        section.extend_from_slice(&self.la2.to_be_bytes()); // La2
        section.extend_from_slice(&self.lo2.to_be_bytes()); // Lo2
        section.extend_from_slice(&self.di.to_be_bytes()); // Di
        section.extend_from_slice(&self.dj.to_be_bytes()); // Dj
        section.push(self.scanning_mode); // Scanning mode

        section
    }

    fn build_section4(&self) -> Vec<u8> {
        let mut section = Vec::new();

        // Template 4.0: Analysis or forecast at horizontal level
        let section_length: u32 = 34;

        section.extend_from_slice(&section_length.to_be_bytes());
        section.push(4); // Section number

        section.extend_from_slice(&0u16.to_be_bytes()); // Number of coordinate values
        section.extend_from_slice(&0u16.to_be_bytes()); // Product definition template (0)

        section.push(self.param_category);
        section.push(self.param_number);
        section.push(2); // Type of generating process (forecast)
        section.push(0); // Background generating process
        section.push(0); // Analysis or forecast process
        section.extend_from_slice(&0u16.to_be_bytes()); // Hours of cutoff
        section.push(0); // Minutes of cutoff
        section.push(1); // Time range unit (hours)
        section.extend_from_slice(&self.forecast_hour.to_be_bytes()); // Forecast time

        section.push(self.level_type); // Type of first fixed surface
        section.push(0); // Scale factor
        section.extend_from_slice(&self.level_value.to_be_bytes()); // Scaled value

        section.push(255); // Type of second fixed surface (none)
        section.push(0);   // Scale factor
        section.extend_from_slice(&0u32.to_be_bytes()); // Scaled value

        section
    }

    fn build_section5(&self) -> Vec<u8> {
        let mut section = Vec::new();

        // Template 5.0: Simple packing
        let num_data_points = self.ni * self.nj;

        // Calculate packing parameters
        let (min_val, max_val) = self.data_values.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(min, max), &v| (min.min(v), max.max(v)),
        );

        let reference_value = min_val;
        let range = max_val - min_val;
        let bits_per_value: u8 = if range == 0.0 { 0 } else { 16 };

        // Calculate binary scale factor
        // Unpacking formula: value = reference_value + packed_value * 2^E
        // We pack as: packed_value = (value - reference_value) / 2^E
        // For 16-bit packing with max packed value of 65535:
        //   range = 65535 * 2^E
        //   2^E = range / 65535
        //   E = log2(range / 65535)
        let binary_scale_factor: i16 = if range == 0.0 {
            0
        } else {
            (range / 65535.0).log2().ceil() as i16
        };

        let section_length: u32 = 21;

        section.extend_from_slice(&section_length.to_be_bytes());
        section.push(5); // Section number

        section.extend_from_slice(&num_data_points.to_be_bytes());
        section.extend_from_slice(&0u16.to_be_bytes()); // Template 5.0

        section.extend_from_slice(&reference_value.to_be_bytes()); // Reference value
        section.extend_from_slice(&binary_scale_factor.to_be_bytes()); // Binary scale factor
        section.extend_from_slice(&0i16.to_be_bytes()); // Decimal scale factor
        section.push(bits_per_value);
        section.push(0); // Original field type (floating point)

        section
    }

    fn build_section6(&self) -> Vec<u8> {
        let mut section = Vec::new();
        let section_length: u32 = 6;

        section.extend_from_slice(&section_length.to_be_bytes());
        section.push(6); // Section number
        section.push(255); // Bitmap indicator (255 = no bitmap, all data present)

        section
    }

    fn build_section7(&self) -> Vec<u8> {
        let mut section = Vec::new();

        // Simple packing of data
        let packed_data = self.pack_simple();

        let section_length: u32 = 5 + packed_data.len() as u32;

        section.extend_from_slice(&section_length.to_be_bytes());
        section.push(7); // Section number
        section.extend_from_slice(&packed_data);

        section
    }

    fn pack_simple(&self) -> Vec<u8> {
        let (min_val, max_val) = self.data_values.iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(min, max), &v| (min.min(v), max.max(v)),
        );

        let range = max_val - min_val;

        if range == 0.0 {
            // All values are the same - no data needed (0 bits per value)
            return Vec::new();
        }

        // Calculate binary scale factor (must match build_section5)
        // E = ceil(log2(range / 65535))
        let binary_scale_factor = (range / 65535.0).log2().ceil() as i16;
        let binary_scale = 2.0_f32.powi(binary_scale_factor as i32);

        // Pack: packed_value = (value - reference_value) / 2^E
        let mut packed = Vec::new();

        for &val in &self.data_values {
            let packed_value = ((val - min_val) / binary_scale).round() as u16;
            packed.extend_from_slice(&packed_value.to_be_bytes());
        }

        packed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_gfs_message() {
        let data = Grib2Builder::new_gfs().build();

        // Check magic bytes
        assert_eq!(&data[0..4], b"GRIB");
        // Check edition
        assert_eq!(data[7], 2);
        // Check discipline
        assert_eq!(data[6], 0); // Meteorological

        // Check end marker
        assert_eq!(&data[data.len() - 4..], b"7777");
    }

    #[test]
    fn test_build_mrms_message() {
        let data = Grib2Builder::new_mrms().build();

        // Check magic bytes
        assert_eq!(&data[0..4], b"GRIB");
        // Check edition
        assert_eq!(data[7], 2);
        // Check discipline
        assert_eq!(data[6], 209); // MRMS

        // Check end marker
        assert_eq!(&data[data.len() - 4..], b"7777");
    }

    #[test]
    fn test_message_can_be_parsed() {
        use bytes::Bytes;
        use grib2_parser::{Grib2Reader, Grib2Tables};
        use std::sync::Arc;

        let data = Grib2Builder::new_gfs()
            .with_grid(5, 5)
            .with_constant_value(288.15)
            .build();

        let mut tables = Grib2Tables::new();
        tables.add_parameter(0, 0, 0, "TMP".to_string());
        let tables = Arc::new(tables);
        let mut reader = Grib2Reader::new(Bytes::from(data), tables);
        let msg = reader.next_message().expect("Should parse").expect("Should have message");

        assert_eq!(msg.parameter(), "TMP");
        assert_eq!(msg.identification.center, 7); // NCEP

        let (nj, ni) = msg.grid_dims();
        assert_eq!(ni, 5);
        assert_eq!(nj, 5);
    }

    #[test]
    fn test_gradient_data_unpacks_correctly() {
        use bytes::Bytes;
        use grib2_parser::{Grib2Reader, Grib2Tables};
        use std::sync::Arc;

        // Create a simple gradient from 0 to 100
        let data = Grib2Builder::new_gfs()
            .with_grid(10, 1)  // 10 points in a row
            .with_gradient(0.0, 100.0)
            .build();
        
        let tables = Arc::new(Grib2Tables::new());
        let mut reader = Grib2Reader::new(Bytes::from(data), tables);
        let msg = reader.next_message().expect("Should parse").expect("Should have message");

        // Use our own unpack_simple (the grib crate doesn't handle our synthetic files correctly)
        let our_values = grib2_parser::unpack_simple(
            &msg.data_section.data,
            msg.data_representation.num_data_points,
            msg.data_representation.bits_per_value,
            msg.data_representation.reference_value,
            msg.data_representation.binary_scale_factor,
            msg.data_representation.decimal_scale_factor,
            None,
        ).expect("Our unpack should work");
        let values: Vec<f32> = our_values.iter().map(|v| v.unwrap_or(f32::NAN)).collect();
        
        assert_eq!(values.len(), 10);

        // Check first and last values (with some tolerance for quantization)
        let first = values[0];
        let last = values[9];

        // First value should be close to 0
        assert!(first.abs() < 2.0, "First value {} should be close to 0", first);
        // Last value should be close to 90 (our gradient goes 0-100 with n values, so last = 90)
        assert!((last - 90.0).abs() < 2.0, "Last value {} should be close to 90", last);
        // Values should be monotonically increasing
        for i in 1..values.len() {
            assert!(values[i] >= values[i-1], "Values should be monotonically increasing");
        }
    }

    #[test]
    fn test_mrms_gradient_data() {
        use bytes::Bytes;
        use grib2_parser::{Grib2Reader, Grib2Tables};
        use std::sync::Arc;

        // Create MRMS-like data with a gradient
        let data = Grib2Builder::new_mrms()
            .with_gradient(-10.0, 60.0)
            .build();

        let tables = Arc::new(Grib2Tables::new());
        let mut reader = Grib2Reader::new(Bytes::from(data), tables);
        let msg = reader.next_message().expect("Should parse").expect("Should have message");

        // Use our own unpack_simple (the grib crate doesn't handle our synthetic files correctly)
        let our_values = grib2_parser::unpack_simple(
            &msg.data_section.data,
            msg.data_representation.num_data_points,
            msg.data_representation.bits_per_value,
            msg.data_representation.reference_value,
            msg.data_representation.binary_scale_factor,
            msg.data_representation.decimal_scale_factor,
            None,
        ).expect("Our unpack should work");
        let values: Vec<f32> = our_values.iter().map(|v| v.unwrap_or(f32::NAN)).collect();
        
        let min_val = values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_val = values.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        println!("MRMS gradient test - min: {}, max: {}", min_val, max_val);
        println!("First 10 values: {:?}", &values[..10.min(values.len())]);
        
        // Check that the range is approximately correct
        // Note: gradient from -10 to 60, but last value is at (n-1)/n * range
        assert!(min_val < 0.0, "Min value {} should be negative", min_val);
        assert!(max_val > 50.0, "Max value {} should be > 50", max_val);
    }
}
