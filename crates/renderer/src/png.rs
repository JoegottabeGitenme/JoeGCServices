//! Simple PNG encoding for RGBA image data.

use std::io::Write;

/// Create a PNG image from RGBA pixel data
/// 
/// # Arguments
/// - `pixels`: RGBA pixel data (4 bytes per pixel)
/// - `width`: Image width in pixels
/// - `height`: Image height in pixels
pub fn create_png(pixels: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    
    // PNG signature
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    
    // IHDR chunk
    let mut ihdr_data = Vec::new();
    ihdr_data.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr_data.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr_data.push(8); // bit depth
    ihdr_data.push(6); // color type (RGBA)
    ihdr_data.push(0); // compression method
    ihdr_data.push(0); // filter method
    ihdr_data.push(0); // interlace method
    write_chunk(&mut png, b"IHDR", &ihdr_data);
    
    // IDAT chunk (image data)
    let idat_data = deflate_idat(pixels, width, height)
        .map_err(|e| format!("IDAT compression failed: {}", e))?;
    write_chunk(&mut png, b"IDAT", &idat_data);
    
    // IEND chunk
    write_chunk(&mut png, b"IEND", &[]);
    
    Ok(png)
}

/// Write a PNG chunk
fn write_chunk(png: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    // Write length
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    
    // Write chunk type
    png.extend_from_slice(chunk_type);
    
    // Write data
    png.extend_from_slice(data);
    
    // Write CRC
    let crc_data = [chunk_type.as_slice(), data].concat();
    let crc = crc32_checksum(&crc_data);
    png.extend_from_slice(&crc.to_be_bytes());
}

/// Deflate and create IDAT data
fn deflate_idat(pixels: &[u8], width: usize, height: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Add filter byte (0 = no filter) to each scanline
    let mut uncompressed = Vec::new();
    for y in 0..height {
        uncompressed.push(0); // filter type: none
        let row_start = y * width * 4;
        let row_end = row_start + width * 4;
        uncompressed.extend_from_slice(&pixels[row_start..row_end]);
    }
    
    // Compress with flate2
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&uncompressed)?;
    let compressed = encoder.finish()?;
    
    Ok(compressed)
}

/// Simple CRC32 checksum (PNG-style)
fn crc32_checksum(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}