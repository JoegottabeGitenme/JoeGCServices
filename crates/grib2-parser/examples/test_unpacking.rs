use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing GRIB2 unpacking with grib crate integration...\n");
    
    // Read the first extracted message
    let data = fs::read("/tmp/first_gfs_message.grib2")?;
    let bytes = bytes::Bytes::from(data);
    
    println!("File size: {} bytes", bytes.len());
    
    // Create reader
    let mut reader = grib2_parser::Grib2Reader::new(bytes);
    
    // Read first message
    if let Some(message) = reader.next_message()? {
        println!("✓ Successfully parsed GRIB2 message");
        println!("  Parameter: {}", message.parameter());
        println!("  Level: {}", message.level());
        println!("  Grid dimensions: {:?}", message.grid_dims());
        println!("  Packing method: {}", message.data_representation.packing_method);
        
        println!("\nAttempting to unpack data with grib crate...");
        
        let values = message.unpack_data()?;
        
        println!("✓ Successfully unpacked {} values!", values.len());
        println!("  First 10 values: {:?}", &values[0..10.min(values.len())]);
        
        // Calculate statistics
        let min = values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = values.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let sum: f32 = values.iter().sum();
        let mean = sum / values.len() as f32;
        
        println!("\nStatistics:");
        println!("  Min: {:.2}", min);
        println!("  Max: {:.2}", max);
        println!("  Mean: {:.2}", mean);
        println!("  Count: {}", values.len());
        
        // Verify grid size matches
        let expected_size = message.grid_dims().0 * message.grid_dims().1;
        if values.len() == expected_size as usize {
            println!("\n✓ Grid size matches! {} = {} × {}", 
                values.len(), message.grid_dims().0, message.grid_dims().1);
        } else {
            println!("\n✗ WARNING: Grid size mismatch! {} != {}", 
                values.len(), expected_size);
        }
    } else {
        println!("✗ No message found");
    }
    
    Ok(())
}
