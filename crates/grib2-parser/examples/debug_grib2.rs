use std::fs;
use grib2_parser::Grib2Reader;
use bytes::Bytes;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grib_path = "testdata/gfs_sample.grib2";
    let grib_data = fs::read(grib_path)?;
    
    println!("File size: {} bytes\n", grib_data.len());
    
    // Find GRIB section 3 (Grid Definition)
    for (i, window) in grib_data.windows(4).enumerate() {
        if window == b"3\x00\x00" || (window[0] == 0 && window[1] == 0 && window[2] == 0 && window[3] == 3) {
            println!("Found section 3 marker at byte offset {}", i);
            
            // Print surrounding bytes
            let start = i.saturating_sub(20);
            let end = (i + 100).min(grib_data.len());
            println!("Context (bytes {}-{}):", start, end);
            
            for j in start..end {
                if j % 16 == 0 {
                    print!("\n{:08x}: ", j);
                }
                print!("{:02x} ", grib_data[j]);
            }
            println!("\n");
        }
    }
    
    // Also try to parse and see what we get
    let mut reader = Grib2Reader::new(Bytes::from(grib_data));
    if let Ok(Some(msg)) = reader.next_message() {
        println!("First message parsed:");
        println!("  Parameter: {}", msg.parameter());
        println!("  Grid dims: {:?}", msg.grid_dims());
    }
    
    Ok(())
}
