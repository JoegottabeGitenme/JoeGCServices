use std::fs;
use grib2_parser::Grib2Reader;
use bytes::Bytes;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grib_path = "testdata/gfs_sample.grib2";
    
    println!("Reading GRIB2 file: {}", grib_path);
    let grib_data = fs::read(grib_path)?;
    println!("File size: {} bytes", grib_data.len());
    
    let mut reader = Grib2Reader::new(Bytes::from(grib_data));
    let mut msg_count = 0;
    
    while let Some(msg) = reader.next_message()? {
        msg_count += 1;
        
        println!("\n=== Message {} ===", msg_count);
        println!("Parameter: {}", msg.parameter());
        println!("Level: {}", msg.level());
        println!("Grid dimensions: {:?}", msg.grid_dims());
        
        // Get the raw grid data
        match msg.unpack_data() {
            Ok(data) => {
                println!("Data points: {}", data.len());
                
                // Calculate min/max
                let (min_val, max_val) = data.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
                    (min.min(val), max.max(val))
                });
                
                println!("Data range: {:.2} to {:.2}", min_val, max_val);
                
                // Show first 10 values
                println!("First 10 values: {:?}", &data[0..10.min(data.len())]);
                
                // Analyze if it looks like Kelvin
                if min_val > 200.0 && max_val < 320.0 {
                    println!("✓ Looks like KELVIN (typical range 200-320K)");
                    
                    // Show what it would be in Celsius
                    let sample_k = data[data.len()/2];
                    let sample_c = sample_k - 273.15;
                    println!("  Sample value: {:.2}K = {:.2}°C", sample_k, sample_c);
                } else if min_val > -60.0 && max_val < 60.0 {
                    println!("✓ Looks like CELSIUS (typical range -60 to 60°C)");
                } else {
                    println!("? Unknown unit - range {:.2} to {:.2}", min_val, max_val);
                }
            }
            Err(e) => {
                println!("Error unpacking data: {}", e);
            }
        }
    }
    
    println!("\nTotal messages: {}", msg_count);
    Ok(())
}
