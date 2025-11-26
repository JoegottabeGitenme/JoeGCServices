use renderer::barbs::{BarbConfig, render_wind_barbs};
use renderer::png::create_png;
use std::fs;

fn main() {
    // Create test data with varying wind patterns
    let width = 512;
    let height = 512;
    
    // Create a grid with different wind directions
    let mut u_data = vec![0.0f32; width * height];
    let mut v_data = vec![0.0f32; width * height];
    
    // Fill with 10 m/s north wind everywhere
    for i in 0..u_data.len() {
        u_data[i] = 0.0;
        v_data[i] = -10.0;
    }
    
    let config = BarbConfig {
        size: 40,
        spacing: 80,
        color: "#000000".to_string(),
    };
    
    println!("Rendering wind barbs...");
    let pixels = render_wind_barbs(&u_data, &v_data, width, height, &config);
    
    // Count non-transparent pixels
    let mut non_transparent = 0;
    for i in (0..pixels.len()).step_by(4) {
        if pixels[i+3] > 0 {
            non_transparent += 1;
        }
    }
    println!("Non-transparent pixels: {}", non_transparent);
    
    // Save as PNG
    let png_data = create_png(&pixels, width, height).expect("Failed to create PNG");
    fs::write("test_wind_barbs.png", png_data).expect("Failed to write file");
    println!("Saved to test_wind_barbs.png");
}
