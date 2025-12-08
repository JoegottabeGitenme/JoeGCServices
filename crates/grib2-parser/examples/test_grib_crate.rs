use std::env;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_path = env::args().nth(1).unwrap_or_else(|| "testdata/gfs_sample.grib2".to_string());
    
    println!("Opening GRIB2 file: {}", file_path);
    
    // Check file size
    let metadata = std::fs::metadata(&file_path)?;
    println!("File size: {} bytes ({:.2} MB)", metadata.len(), metadata.len() as f64 / 1024.0 / 1024.0);
    
    let f = File::open(&file_path)?;
    let f = BufReader::new(f);
    
    println!("Parsing GRIB file with grib crate...");
    let grib_file = match grib::from_reader(f) {
        Ok(f) => {
            println!("✓ Successfully parsed GRIB file!");
            f
        }
        Err(e) => {
            eprintln!("\n✗ ERROR parsing GRIB file: {:?}", e);
            eprintln!("\nThis could mean:");
            eprintln!("  1. The file contains unsupported sections");
            eprintln!("  2. The file structure doesn't match what the grib crate expects");
            eprintln!("  3. There's a mismatch in GRIB2 format implementation");
            return Err(e.into());
        }
    };
    
    // Iterate through submessages
    let mut count = 0;
    for ((section_num, submessage_num), submsg) in grib_file.iter() {
        count += 1;
        
        if count <= 3 {
            println!("\n=== Submessage {} (Section {}, Submessage {}) ===", count, section_num, submessage_num);
            println!("Discipline: {:?}", submsg.indicator().discipline);
            
            // Get grid information
            let grid_def = submsg.grid_def();
            println!("Grid template number: {}", grid_def.grid_tmpl_num());
            
            // Get product information
            let prod_def = submsg.prod_def();
            println!("Product template number: {}", prod_def.prod_tmpl_num());
            
            // Try to decode values - this is the key test for PNG unpacking!
            println!("Attempting to decode values...");
            match grib::Grib2SubmessageDecoder::from(submsg) {
                Ok(decoder) => {
                    match decoder.dispatch() {
                        Ok(values) => {
                            let vals: Vec<_> = values.collect();
                            println!("✓ Successfully decoded {} values!", vals.len());
                            if !vals.is_empty() {
                                println!("  First 10 values: {:?}", &vals[0..10.min(vals.len())]);
                                // Calculate some stats
                                let min = vals.iter().cloned().fold(f32::INFINITY, f32::min);
                                let max = vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                                println!("  Min: {:.2}, Max: {:.2}", min, max);
                            }
                        }
                        Err(e) => {
                            println!("✗ ERROR dispatching decoder: {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("✗ ERROR creating decoder: {}", e);
                }
            }
        }
    }
    
    println!("\n========================================");
    println!("Total submessages in file: {}", count);
    println!("========================================");
    
    Ok(())
}
