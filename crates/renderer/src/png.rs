//! PNG encoding for RGBA image data.
//!
//! Supports two encoding modes:
//! - **Indexed PNG (color type 3)**: Used when image has ≤256 unique colors.
//!   Produces smaller files and encodes faster.
//! - **RGBA PNG (color type 6)**: Fallback for images with >256 colors.
//!
//! Use `create_png_auto` for automatic mode selection, or `create_png` for
//! explicit RGBA encoding.

use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Write;

/// Maximum colors for indexed PNG (PNG8)
const MAX_PALETTE_SIZE: usize = 256;

/// Minimum pixels to benefit from parallel palette extraction
const PARALLEL_THRESHOLD: usize = 4096; // 64x64 or larger

/// Create a PNG image with automatic format selection.
///
/// Analyzes the pixel data and chooses the most efficient encoding:
/// - If ≤256 unique colors: uses indexed PNG (smaller, faster)
/// - Otherwise: uses RGBA PNG (full color)
///
/// # Arguments
/// - `pixels`: RGBA pixel data (4 bytes per pixel)
/// - `width`: Image width in pixels
/// - `height`: Image height in pixels
pub fn create_png_auto(pixels: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    let num_pixels = pixels.len() / 4;
    
    // Try to extract a palette (use parallel version for larger images)
    let palette_result = if num_pixels >= PARALLEL_THRESHOLD {
        extract_palette_parallel(pixels)
    } else {
        extract_palette_sequential(pixels)
    };
    
    match palette_result {
        Some((palette, indices)) => {
            // Can use indexed PNG
            create_png_indexed(width, height, &palette, &indices)
        }
        None => {
            // Too many colors, fall back to RGBA
            create_png(pixels, width, height)
        }
    }
}

/// Pack RGBA bytes into a u32 for faster hashing and comparison
#[inline(always)]
fn pack_color(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

/// Unpack u32 back to RGBA tuple
#[inline(always)]
fn unpack_color(packed: u32) -> (u8, u8, u8, u8) {
    (
        packed as u8,
        (packed >> 8) as u8,
        (packed >> 16) as u8,
        (packed >> 24) as u8,
    )
}

/// Sequential palette extraction for small images.
fn extract_palette_sequential(pixels: &[u8]) -> Option<(Vec<(u8, u8, u8, u8)>, Vec<u8>)> {
    // Use u32 keys for faster hashing
    let mut color_to_index: HashMap<u32, u8> = HashMap::with_capacity(MAX_PALETTE_SIZE);
    let mut palette: Vec<(u8, u8, u8, u8)> = Vec::with_capacity(MAX_PALETTE_SIZE);
    let mut indices: Vec<u8> = Vec::with_capacity(pixels.len() / 4);

    for chunk in pixels.chunks_exact(4) {
        let packed = pack_color(chunk[0], chunk[1], chunk[2], chunk[3]);

        let index = match color_to_index.get(&packed) {
            Some(&idx) => idx,
            None => {
                if palette.len() >= MAX_PALETTE_SIZE {
                    return None;
                }
                let idx = palette.len() as u8;
                palette.push((chunk[0], chunk[1], chunk[2], chunk[3]));
                color_to_index.insert(packed, idx);
                idx
            }
        };
        indices.push(index);
    }

    Some((palette, indices))
}

/// Parallel palette extraction for larger images.
/// 
/// Strategy:
/// 1. Parallel pass: collect unique colors from chunks using thread-local sets
/// 2. Merge unique colors and check if ≤256
/// 3. Build final palette and color-to-index map
/// 4. Parallel pass: map each pixel to its palette index
fn extract_palette_parallel(pixels: &[u8]) -> Option<(Vec<(u8, u8, u8, u8)>, Vec<u8>)> {
    // Step 1: Parallel collection of unique colors using thread-local HashSets
    // Each chunk processes a portion of pixels and returns its unique colors
    let chunk_size = (pixels.len() / 4 / rayon::current_num_threads()).max(256) * 4;
    
    let unique_colors: Vec<u32> = pixels
        .par_chunks(chunk_size)
        .flat_map(|chunk| {
            let mut local_colors: HashMap<u32, ()> = HashMap::with_capacity(MAX_PALETTE_SIZE);
            for pixel in chunk.chunks_exact(4) {
                let packed = pack_color(pixel[0], pixel[1], pixel[2], pixel[3]);
                local_colors.insert(packed, ());
                // Early exit if we definitely have too many colors
                if local_colors.len() > MAX_PALETTE_SIZE {
                    break;
                }
            }
            local_colors.into_keys().collect::<Vec<_>>()
        })
        .collect();

    // Step 2: Deduplicate and check count
    let mut global_colors: HashMap<u32, u8> = HashMap::with_capacity(MAX_PALETTE_SIZE);
    let mut palette: Vec<(u8, u8, u8, u8)> = Vec::with_capacity(MAX_PALETTE_SIZE);
    
    for packed in unique_colors {
        if !global_colors.contains_key(&packed) {
            if palette.len() >= MAX_PALETTE_SIZE {
                return None; // Too many colors
            }
            let idx = palette.len() as u8;
            global_colors.insert(packed, idx);
            palette.push(unpack_color(packed));
        }
    }

    // Step 3: Parallel mapping of pixels to indices
    let num_pixels = pixels.len() / 4;
    let mut indices = vec![0u8; num_pixels];
    
    indices
        .par_chunks_mut(chunk_size / 4)
        .enumerate()
        .for_each(|(chunk_idx, idx_chunk)| {
            let pixel_start = chunk_idx * (chunk_size / 4) * 4;
            for (i, idx) in idx_chunk.iter_mut().enumerate() {
                let pixel_offset = pixel_start + i * 4;
                if pixel_offset + 3 < pixels.len() {
                    let packed = pack_color(
                        pixels[pixel_offset],
                        pixels[pixel_offset + 1],
                        pixels[pixel_offset + 2],
                        pixels[pixel_offset + 3],
                    );
                    *idx = *global_colors.get(&packed).unwrap_or(&0);
                }
            }
        });

    Some((palette, indices))
}

use crate::style::PrecomputedPalette;

/// Create an indexed PNG from a pre-computed palette and indices.
///
/// This is the fastest path for weather tile rendering:
/// - Palette was computed once at style load time
/// - Indices were generated during rendering (1 byte/pixel)
/// - No palette extraction needed at encoding time
///
/// # Arguments
/// * `indices` - Palette indices from `apply_style_gradient_indexed()`
/// * `width` - Image width
/// * `height` - Image height
/// * `palette` - Pre-computed palette from `StyleDefinition::compute_palette()`
pub fn create_png_from_precomputed(
    indices: &[u8],
    width: usize,
    height: usize,
    palette: &PrecomputedPalette,
) -> Result<Vec<u8>, String> {
    create_png_indexed(width, height, &palette.colors, indices)
}

/// Create an indexed PNG (color type 3) from palette and indices.
///
/// This is more efficient than RGBA when the image has few unique colors:
/// - 1 byte per pixel instead of 4
/// - Less data to compress
/// - Smaller output file
pub fn create_png_indexed(
    width: usize,
    height: usize,
    palette: &[(u8, u8, u8, u8)],
    indices: &[u8],
) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();

    // PNG signature
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR chunk
    let mut ihdr_data = Vec::with_capacity(13);
    ihdr_data.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr_data.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr_data.push(8); // bit depth (8 bits per palette index)
    ihdr_data.push(3); // color type 3 = indexed
    ihdr_data.push(0); // compression method
    ihdr_data.push(0); // filter method
    ihdr_data.push(0); // interlace method
    write_chunk(&mut png, b"IHDR", &ihdr_data);

    // PLTE chunk (palette)
    let mut plte_data = Vec::with_capacity(palette.len() * 3);
    for (r, g, b, _) in palette {
        plte_data.push(*r);
        plte_data.push(*g);
        plte_data.push(*b);
    }
    write_chunk(&mut png, b"PLTE", &plte_data);

    // tRNS chunk (transparency) - only if any color has alpha < 255
    let has_transparency = palette.iter().any(|(_, _, _, a)| *a < 255);
    if has_transparency {
        // tRNS contains alpha value for each palette entry
        let trns_data: Vec<u8> = palette.iter().map(|(_, _, _, a)| *a).collect();
        write_chunk(&mut png, b"tRNS", &trns_data);
    }

    // IDAT chunk (image data)
    let idat_data = deflate_idat_indexed(indices, width, height)
        .map_err(|e| format!("IDAT compression failed: {}", e))?;
    write_chunk(&mut png, b"IDAT", &idat_data);

    // IEND chunk
    write_chunk(&mut png, b"IEND", &[]);

    Ok(png)
}

/// Deflate indexed image data for IDAT chunk.
fn deflate_idat_indexed(
    indices: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Add filter byte (0 = no filter) to each scanline
    // For indexed, each row is: filter_byte + width index bytes
    let mut uncompressed = Vec::with_capacity(height * (1 + width));

    for y in 0..height {
        uncompressed.push(0); // filter type: none
        let row_start = y * width;
        let row_end = row_start + width;
        uncompressed.extend_from_slice(&indices[row_start..row_end]);
    }

    // Compress with flate2
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&uncompressed)?;
    let compressed = encoder.finish()?;

    Ok(compressed)
}

/// Create a PNG image from RGBA pixel data (color type 6).
///
/// This is the fallback for images with >256 unique colors.
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
    let idat_data =
        deflate_idat_rgba(pixels, width, height).map_err(|e| format!("IDAT compression failed: {}", e))?;
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

/// Deflate RGBA image data for IDAT chunk.
fn deflate_idat_rgba(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Add filter byte (0 = no filter) to each scanline
    let mut uncompressed = Vec::with_capacity(height * (1 + width * 4));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_palette_simple() {
        // 4 pixels: red, green, blue, red (3 unique colors)
        let pixels = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 0, 0, 255, // red again
        ];

        let result = extract_palette_sequential(&pixels);
        assert!(result.is_some());

        let (palette, indices) = result.unwrap();
        assert_eq!(palette.len(), 3);
        assert_eq!(indices.len(), 4);
        assert_eq!(indices[0], indices[3]); // both red pixels have same index
    }

    #[test]
    fn test_extract_palette_with_transparency() {
        // 2 pixels: one opaque, one transparent
        let pixels = [
            255, 0, 0, 255, // red, opaque
            0, 0, 0, 0,     // transparent
        ];

        let result = extract_palette_sequential(&pixels);
        assert!(result.is_some());

        let (palette, _) = result.unwrap();
        assert_eq!(palette.len(), 2);
        // Check that we captured the alpha values
        assert!(palette.iter().any(|(_, _, _, a)| *a == 0));
        assert!(palette.iter().any(|(_, _, _, a)| *a == 255));
    }
    
    #[test]
    fn test_extract_palette_parallel() {
        // Generate a larger image that triggers parallel extraction
        // 128x128 = 16384 pixels, above PARALLEL_THRESHOLD
        let mut pixels = Vec::with_capacity(128 * 128 * 4);
        for y in 0..128 {
            for x in 0..128 {
                // Limited color palette (~50 colors)
                let color_idx = ((x / 8) + (y / 8)) % 50;
                let r = (color_idx * 5) as u8;
                let g = (100 + color_idx * 3) as u8;
                let b = (200 - color_idx * 2) as u8;
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        
        let result = extract_palette_parallel(&pixels);
        assert!(result.is_some());
        
        let (palette, indices) = result.unwrap();
        assert!(palette.len() <= 50); // Should have ~50 unique colors
        assert_eq!(indices.len(), 128 * 128);
    }

    #[test]
    fn test_create_png_indexed() {
        // Simple 2x2 image with 2 colors
        let pixels = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 255, 0, 255, // green
            255, 0, 0, 255, // red
        ];

        let result = create_png_auto(&pixels, 2, 2);
        assert!(result.is_ok());

        let png = result.unwrap();
        // Check PNG signature
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);

        // Should be smaller than RGBA version
        let rgba_png = create_png(&pixels, 2, 2).unwrap();
        // Note: for very small images, indexed might not always be smaller
        // due to palette overhead, but for typical tiles it will be
        println!(
            "Indexed: {} bytes, RGBA: {} bytes",
            png.len(),
            rgba_png.len()
        );
    }

    #[test]
    fn test_create_png_rgba_fallback() {
        // Create image with >256 unique colors
        let mut pixels = Vec::with_capacity(300 * 4);
        for i in 0..300 {
            pixels.push((i % 256) as u8); // R
            pixels.push(((i / 2) % 256) as u8); // G
            pixels.push(((i / 3) % 256) as u8); // B
            pixels.push(255); // A
        }

        // Should fall back to RGBA
        let result = create_png_auto(&pixels, 300, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_png_auto_weather_like() {
        // Simulate weather tile: gradient with limited colors
        // 16x16 tile with temperature-like gradient (maybe 20-30 unique colors)
        let mut pixels = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16 {
            for x in 0..16 {
                // Quantized gradient (step of 16 gives ~16 unique colors per channel)
                let r = ((x * 16) as u8).wrapping_mul(16);
                let g = 128;
                let b = ((y * 16) as u8).wrapping_mul(16);
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }

        let indexed = create_png_auto(&pixels, 16, 16).unwrap();
        let rgba = create_png(&pixels, 16, 16).unwrap();

        println!(
            "Weather-like 16x16: Indexed {} bytes, RGBA {} bytes, savings: {:.1}%",
            indexed.len(),
            rgba.len(),
            (1.0 - indexed.len() as f64 / rgba.len() as f64) * 100.0
        );

        // Indexed should be smaller for weather-like data
        assert!(indexed.len() < rgba.len());
    }

    #[test]
    fn test_file_size_comparison_256x256() {
        // Simulate realistic 256x256 weather tile with quantized colors
        // Real weather tiles use discrete color stops from style JSON
        let mut pixels = Vec::with_capacity(256 * 256 * 4);
        
        // Define a realistic weather color palette (like temperature)
        let palette: [(u8, u8, u8); 20] = [
            (102, 0, 214),   // -40C purple
            (0, 51, 255),    // -30C blue
            (0, 128, 255),   // -20C light blue
            (0, 191, 255),   // -10C cyan
            (0, 255, 255),   // 0C cyan
            (0, 255, 191),   // 5C teal
            (0, 255, 128),   // 10C green
            (0, 255, 0),     // 15C bright green
            (128, 255, 0),   // 20C yellow-green
            (191, 255, 0),   // 22C
            (255, 255, 0),   // 25C yellow
            (255, 220, 0),   // 27C
            (255, 191, 0),   // 30C orange
            (255, 128, 0),   // 33C
            (255, 64, 0),    // 36C red-orange
            (255, 0, 0),     // 40C red
            (214, 0, 0),     // 42C dark red
            (178, 0, 0),     // 45C
            (139, 0, 0),     // 48C maroon
            (100, 0, 0),     // 50C dark maroon
        ];
        
        for y in 0..256 {
            for x in 0..256 {
                // Map position to palette index (quantized like real weather data)
                let temp_idx = ((x as f32 / 256.0 * 0.3 + y as f32 / 256.0 * 0.7) * 19.0) as usize;
                let (r, g, b) = palette[temp_idx.min(19)];
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        
        // Count unique colors
        let mut unique: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for chunk in pixels.chunks_exact(4) {
            unique.insert(pack_color(chunk[0], chunk[1], chunk[2], chunk[3]));
        }
        println!("\n=== 256x256 Weather Tile ===");
        println!("Unique colors: {}", unique.len());

        let indexed = create_png_auto(&pixels, 256, 256).unwrap();
        let rgba = create_png(&pixels, 256, 256).unwrap();

        println!("RGBA PNG:    {:>6} bytes ({:.1} KB)", rgba.len(), rgba.len() as f64 / 1024.0);
        println!("Indexed PNG: {:>6} bytes ({:.1} KB)", indexed.len(), indexed.len() as f64 / 1024.0);
        println!(
            "Savings:     {:.1}%",
            (1.0 - indexed.len() as f64 / rgba.len() as f64) * 100.0
        );

        // Both should work; check if indexed is used when colors fit
        if unique.len() <= 256 {
            assert!(indexed.len() <= rgba.len(), "Indexed should be <= RGBA when colors fit");
        }
    }

    #[test]
    fn test_file_size_comparison_512x512() {
        // 512x512 tile with quantized weather palette
        let palette: [(u8, u8, u8); 20] = [
            (102, 0, 214), (0, 51, 255), (0, 128, 255), (0, 191, 255),
            (0, 255, 255), (0, 255, 191), (0, 255, 128), (0, 255, 0),
            (128, 255, 0), (191, 255, 0), (255, 255, 0), (255, 220, 0),
            (255, 191, 0), (255, 128, 0), (255, 64, 0), (255, 0, 0),
            (214, 0, 0), (178, 0, 0), (139, 0, 0), (100, 0, 0),
        ];
        
        let mut pixels = Vec::with_capacity(512 * 512 * 4);
        for y in 0..512 {
            for x in 0..512 {
                let temp_idx = ((x as f32 / 512.0 * 0.3 + y as f32 / 512.0 * 0.7) * 19.0) as usize;
                let (r, g, b) = palette[temp_idx.min(19)];
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        
        let mut unique: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for chunk in pixels.chunks_exact(4) {
            unique.insert(pack_color(chunk[0], chunk[1], chunk[2], chunk[3]));
        }
        
        println!("\n=== 512x512 Weather Tile ===");
        println!("Unique colors: {}", unique.len());

        let indexed = create_png_auto(&pixels, 512, 512).unwrap();
        let rgba = create_png(&pixels, 512, 512).unwrap();

        println!("RGBA PNG:    {:>6} bytes ({:.1} KB)", rgba.len(), rgba.len() as f64 / 1024.0);
        println!("Indexed PNG: {:>6} bytes ({:.1} KB)", indexed.len(), indexed.len() as f64 / 1024.0);
        println!(
            "Savings:     {:.1}%",
            (1.0 - indexed.len() as f64 / rgba.len() as f64) * 100.0
        );

        if unique.len() <= 256 {
            assert!(indexed.len() <= rgba.len(), "Indexed should be <= RGBA when colors fit");
        }
    }
}
