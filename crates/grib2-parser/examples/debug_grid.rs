use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grib_data = fs::read("testdata/gfs_sample.grib2")?;
    
    // Section 3 is at offset 37
    let section_3_offset = 37;
    let section_data = &grib_data[section_3_offset..];
    
    println!("Section 3 starts at byte offset: {}", section_3_offset);
    println!("Section 3 data (first 40 bytes hex):");
    for i in 0..40.min(section_data.len()) {
        if i % 16 == 0 {
            print!("\n{:04x}: ", i);
        }
        print!("{:02x} ", section_data[i]);
    }
    println!("\n");
    
    // Parse header
    let length = u32::from_be_bytes([
        section_data[0], section_data[1], section_data[2], section_data[3]
    ]);
    let section_num = section_data[4];
    let template = u16::from_be_bytes([section_data[5], section_data[6]]);
    
    println!("Section 3 header:");
    println!("  Length: {}", length);
    println!("  Section number: {}", section_num);
    println!("  Template: {}", template);
    
    // Grid data starts after header
    let grid_data = &section_data[7..];
    println!("\nGrid data (template-specific, first 40 bytes):");
    for i in 0..40.min(grid_data.len()) {
        if i % 16 == 0 {
            print!("\n[{:02}]: ", i);
        }
        print!("{:02x} ", grid_data[i]);
    }
    println!("\n");
    
    // Check what the current code is reading
    if grid_data.len() >= 30 {
        println!("Current code reads:");
        println!("  grid_data[24-26] for longitude: {:02x}{:02x} = {}",
            grid_data[24], grid_data[25],
            u16::from_be_bytes([grid_data[24], grid_data[25]])
        );
        println!("  grid_data[28-30] for latitude: {:02x}{:02x} = {}",
            grid_data[28], grid_data[29],
            u16::from_be_bytes([grid_data[28], grid_data[29]])
        );
    }
    
    Ok(())
}
