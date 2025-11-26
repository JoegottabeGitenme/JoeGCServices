use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  GRIB2 → PNG RENDERING PIPELINE TEST                    ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");
    
    // Step 1: Read GRIB2 file
    println!("[1/5] Reading GRIB2 file...");
    let grib_data = fs::read("/tmp/gfs_message_000.grib2")?;
    let bytes = bytes::Bytes::from(grib_data);
    println!("      ✓ Read {} bytes", bytes.len());
    
    // Step 2: Parse GRIB2 message
    println!("\n[2/5] Parsing GRIB2 message...");
    let mut reader = grib2_parser::Grib2Reader::new(bytes);
    let message = reader.next_message()?.expect("Should have a message");
    println!("      ✓ Parameter: {}", message.parameter());
    println!("      ✓ Level: {}", message.level());
    println!("      ✓ Grid: {} × {}", message.grid_dims().0, message.grid_dims().1);
    println!("      ✓ Valid time: {}", message.valid_time());
    
    // Step 3: Unpack PNG-compressed data
    println!("\n[3/5] Unpacking PNG-compressed GRIB data...");
    let values = message.unpack_data()?;
    let min_val = values.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_val = values.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    println!("      ✓ Unpacked {} values", values.len());
    println!("      ✓ Range: {:.2} - {:.2} Pa", min_val, max_val);
    println!("      ✓ Range: {:.2} - {:.2} hPa", min_val / 100.0, max_val / 100.0);
    
    // Step 4: Render to image
    println!("\n[4/5] Rendering pressure field to image...");
    let (height, width) = message.grid_dims();
    let width = width as usize;
    let height = height as usize;
    
    // Convert Pa to hPa for color mapping
    let values_hpa: Vec<f32> = values.iter().map(|v| v / 100.0).collect();
    let min_hpa = min_val / 100.0;
    let max_hpa = max_val / 100.0;
    
    let rgba_pixels = renderer::gradient::render_pressure(
        &values_hpa,
        width,
        height,
        min_hpa,
        max_hpa
    );
    println!("      ✓ Rendered {}x{} image", width, height);
    println!("      ✓ Generated {} RGBA pixels", rgba_pixels.len() / 4);
    
    // Step 5: Encode as PNG
    println!("\n[5/5] Encoding PNG...");
    let png_bytes = renderer::png::create_png(&rgba_pixels, width, height)?;
    fs::write("/tmp/grib_render.png", &png_bytes)?;
    println!("      ✓ Encoded {} bytes", png_bytes.len());
    println!("      ✓ Saved to /tmp/grib_render.png");
    
    // Summary
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║  ✅ FULL PIPELINE SUCCESS!                              ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("\n✨ Successfully rendered GFS pressure field from PNG-compressed GRIB2 data!");
    println!("📊 Grid: {}x{} ({} points)", width, height, values.len());
    println!("🎨 Pressure range: {:.1} - {:.1} hPa", min_hpa, max_hpa);
    println!("💾 Output: /tmp/grib_render.png ({} KB)", png_bytes.len() / 1024);
    
    Ok(())
}
