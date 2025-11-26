use std::fs;
use grib2_parser::Grib2Reader;
use bytes::Bytes;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grib_path = "testdata/gfs_sample.grib2";
    
    println!("Listing all GRIB2 messages:");
    let grib_data = fs::read(grib_path)?;
    
    let mut reader = Grib2Reader::new(Bytes::from(grib_data));
    let mut msg_count = 0;
    
    loop {
        match reader.next_message() {
            Ok(Some(msg)) => {
                msg_count += 1;
                println!("Message {}: {} at level {} (grid: {:?})", 
                    msg_count, 
                    msg.parameter(),
                    msg.level(),
                    msg.grid_dims()
                );
            }
            Ok(None) => break,
            Err(e) => {
                println!("Message {}: ERROR - {}", msg_count + 1, e);
                break;
            }
        }
    }
    
    println!("\nTotal messages read: {}", msg_count);
    Ok(())
}
