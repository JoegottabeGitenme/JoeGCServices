use grib2_parser::Grib2Reader;
use std::fs;
use bytes::Bytes;

#[test]
fn test_parse_mrms_file() {
    let path = "/tmp/mrms_refl_full.grib2";
    
    let data = fs::read(path).expect("Failed to read test file");
    let data = Bytes::from(data);

    let mut reader = Grib2Reader::new(data);

    match reader.next_message() {
        Ok(Some(msg)) => {
            println!("=== MRMS GRIB2 Analysis ===");
            println!("Parameter: {}", msg.parameter());
            println!("Level: {}", msg.level());
            
            let gd = &msg.grid_definition;
            println!("\n=== Grid Definition (raw millidegrees) ===");
            println!("Grid shape: {}", gd.grid_shape);
            println!("Ni (columns): {}", gd.num_points_longitude);
            println!("Nj (rows): {}", gd.num_points_latitude);
            println!("First lat (mdeg): {}", gd.first_latitude_millidegrees);
            println!("First lon (mdeg): {}", gd.first_longitude_millidegrees);
            println!("Last lat (mdeg): {}", gd.last_latitude_millidegrees);
            println!("Last lon (mdeg): {}", gd.last_longitude_millidegrees);
            println!("Lat increment (mdeg): {}", gd.latitude_increment_millidegrees);
            println!("Lon increment (mdeg): {}", gd.longitude_increment_millidegrees);
            println!("Scan mode: {}", gd.scanning_mode);
            
            // Millidegrees to degrees: divide by 1000
            let first_lat = gd.first_latitude_millidegrees as f64 / 1000.0;
            let first_lon = gd.first_longitude_millidegrees as f64 / 1000.0;
            let last_lat = gd.last_latitude_millidegrees as f64 / 1000.0;
            let last_lon = gd.last_longitude_millidegrees as f64 / 1000.0;
            let lat_inc = gd.latitude_increment_millidegrees as f64 / 1000.0;
            let lon_inc = gd.longitude_increment_millidegrees as f64 / 1000.0;
            
            println!("\n=== In Degrees ===");
            println!("First lat: {:.6}°", first_lat);
            println!("First lon: {:.6}° ({:.6}° when converted to -180/180)", first_lon, if first_lon > 180.0 { first_lon - 360.0 } else { first_lon });
            println!("Last lat: {:.6}°", last_lat);
            println!("Last lon: {:.6}° ({:.6}° when converted to -180/180)", last_lon, if last_lon > 180.0 { last_lon - 360.0 } else { last_lon });
            println!("Lat step: {:.6}°", lat_inc);
            println!("Lon step: {:.6}°", lon_inc);
            
            // Data analysis
            let unpacked = msg.unpack_data().expect("Cannot unpack");
            let missing: Vec<_> = unpacked.iter().filter(|&&v| v <= -90.0 || v > 90.0).collect();
            let valid: Vec<_> = unpacked.iter().filter(|&&v| v > -90.0 && v <= 90.0).collect();
            
            println!("\n=== Data Analysis ===");
            println!("Total values: {}", unpacked.len());
            println!("Missing (no-data): {} ({:.1}%)", missing.len(), 100.0 * missing.len() as f64 / unpacked.len() as f64);
            println!("Valid: {} ({:.1}%)", valid.len(), 100.0 * valid.len() as f64 / unpacked.len() as f64);
            
            if !valid.is_empty() {
                let vmin = valid.iter().cloned().fold(f32::INFINITY, |a, &b| a.min(b));
                let vmax = valid.iter().cloned().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                println!("Valid range: {:.2} to {:.2} dBZ", vmin, vmax);
            }
            
            if !missing.is_empty() {
                use std::collections::HashSet;
                let unique_missing: HashSet<i32> = missing.iter().map(|&&v| v as i32).collect();
                println!("Missing markers: {:?}", unique_missing.iter().take(5).collect::<Vec<_>>());
            }
        }
        Ok(None) => println!("No messages"),
        Err(e) => println!("Error: {}", e),
    }
}
