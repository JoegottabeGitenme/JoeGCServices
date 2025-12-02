use grib2_parser::Grib2Reader;
use std::fs;
use bytes::Bytes;

#[test]
fn test_mrms_bounds() {
    let path = "/tmp/mrms_refl_full.grib2";
    let data = fs::read(path).expect("Failed to read test file");
    let mut reader = Grib2Reader::new(Bytes::from(data));

    match reader.next_message() {
        Ok(Some(msg)) => {
            let gd = &msg.grid_definition;
            
            // Convert millidegrees to degrees
            let first_lat = gd.first_latitude_millidegrees as f64 / 1000.0;
            let first_lon = gd.first_longitude_millidegrees as f64 / 1000.0;
            let last_lat = gd.last_latitude_millidegrees as f64 / 1000.0;
            let last_lon = gd.last_longitude_millidegrees as f64 / 1000.0;
            let lat_inc = gd.latitude_increment_millidegrees as f64 / 1000.0;
            let lon_inc = gd.longitude_increment_millidegrees as f64 / 1000.0;
            
            println!("\n=== MRMS Grid Bounds (degrees) ===");
            println!("Grid: {} x {}", gd.num_points_longitude, gd.num_points_latitude);
            println!("First point (NW): lat={:.6}°, lon={:.6}° (={:.6}° in -180/180)", 
                     first_lat, first_lon, 
                     if first_lon > 180.0 { first_lon - 360.0 } else { first_lon });
            println!("Last point (SE): lat={:.6}°, lon={:.6}° (={:.6}° in -180/180)", 
                     last_lat, last_lon,
                     if last_lon > 180.0 { last_lon - 360.0 } else { last_lon });
            println!("Increment: lat={:.6}°, lon={:.6}°", lat_inc, lon_inc);
            println!("Scan mode: {}", gd.scanning_mode);
            
            // For catalog bbox, we need:
            // min_x = westmost = first_lon in -180/180
            // max_x = eastmost = last_lon in -180/180
            // min_y = southmost = last_lat
            // max_y = northmost = first_lat
            let min_x = if first_lon > 180.0 { first_lon - 360.0 } else { first_lon };
            let max_x = if last_lon > 180.0 { last_lon - 360.0 } else { last_lon };
            let min_y = last_lat;
            let max_y = first_lat;
            
            println!("\n=== Correct catalog bbox ===");
            println!("min_x (west): {:.6}°", min_x);
            println!("min_y (south): {:.6}°", min_y);
            println!("max_x (east): {:.6}°", max_x);
            println!("max_y (north): {:.6}°", max_y);
            
            // What we have in catalog
            println!("\n=== Current catalog bbox (approximate) ===");
            println!("BoundingBox::new(-130.0, 20.0, -60.0, 55.0)");
        }
        Ok(None) => println!("No messages"),
        Err(e) => println!("Error: {}", e),
    }
}
