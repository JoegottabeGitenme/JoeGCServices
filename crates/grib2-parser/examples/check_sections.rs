use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grib_path = "testdata/gfs_sample.grib2";
    let grib_data = fs::read(grib_path)?;
    
    println!("Scanning for GRIB2 sections...\n");
    
    // GRIB2 format: sections are preceded by a 4-byte length field and 1-byte section number
    // Skip GRIB header (should be first 16 bytes)
    println!("Header (0-16): {:?}", String::from_utf8_lossy(&grib_data[0..16]));
    let mut offset = 16;
    
    let mut section_num = 0;
    while offset < grib_data.len().min(offset + 5000) && section_num < 10 {
        if offset + 5 > grib_data.len() {
            break;
        }
        
        // Read 4-byte big-endian length
        let len_bytes = [grib_data[offset], grib_data[offset+1], grib_data[offset+2], grib_data[offset+3]];
        let length = u32::from_be_bytes(len_bytes) as usize;
        
        // Read section number
        let section = grib_data[offset + 4];
        
        println!("Offset {}: Section {} (length: {} bytes)", offset, section, length);
        
        // Print first few bytes of section content
        if offset + 8 < grib_data.len() {
            println!("  First 8 bytes: {:02x?}", &grib_data[offset+5..offset+5+8.min(grib_data.len()-offset-5)]);
        }
        
        offset += length;
        section_num += 1;
    }
    
    Ok(())
}
