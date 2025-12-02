use grib2_parser::Grib2Reader;
use std::fs;
use bytes::Bytes;

#[test]
fn test_sample_point() {
    let path = "/tmp/mrms_refl_full.grib2";
    let data = fs::read(path).expect("Failed to read test file");
    let mut reader = Grib2Reader::new(Bytes::from(data));

    match reader.next_message() {
        Ok(Some(msg)) => {
            let gd = &msg.grid_definition;
            let values = msg.unpack_data().expect("Failed to unpack");
            
            // Grid info (from earlier analysis)
            let first_lat: f64 = 54.995;
            let first_lon: f64 = -129.995;
            let lat_step: f64 = 0.01;
            let lon_step: f64 = 0.01;
            let ni: usize = 7000;
            let nj: usize = 3500;
            
            // Query point from your request
            let query_lon: f64 = -89.75280761718751;
            let query_lat: f64 = 35.24908941727283;
            
            // Convert to grid indices
            let col = ((query_lon - first_lon) / lon_step).round() as usize;
            let row = ((first_lat - query_lat) / lat_step).round() as usize;
            
            println!("Query: lon={:.6}°, lat={:.6}°", query_lon, query_lat);
            println!("Grid index: row={}, col={}", row, col);
            
            if row < nj && col < ni {
                let idx = row * ni + col;
                let value = values[idx];
                println!("Value at index {}: {:.2}", idx, value);
                
                // Also show surrounding values
                println!("\nSurrounding 5x5 grid:");
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        let r = (row as i32 + dy) as usize;
                        let c = (col as i32 + dx) as usize;
                        if r < nj && c < ni {
                            let v = values[r * ni + c];
                            if v > -90.0 {
                                print!("{:6.1} ", v);
                            } else {
                                print!("  ---  ");
                            }
                        }
                    }
                    println!();
                }
            } else {
                println!("Point outside grid bounds!");
            }
            
            // Check parsed grid definition
            println!("\n=== Parsed Grid Definition ===");
            println!("Ni: {}, Nj: {}", gd.num_points_longitude, gd.num_points_latitude);
            println!("First lat (mdeg): {} -> {:.3}°", gd.first_latitude_millidegrees, gd.first_latitude_millidegrees as f64 / 1000.0);
            println!("First lon (mdeg): {} -> {:.3}°", gd.first_longitude_millidegrees, gd.first_longitude_millidegrees as f64 / 1000.0);
        }
        Ok(None) => println!("No messages"),
        Err(e) => println!("Error: {}", e),
    }
}
