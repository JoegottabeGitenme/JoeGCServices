//! Test the exact scenario from WMS request

use renderer::barbs::{render_wind_barbs, BarbConfig};
use renderer::png::create_png;

fn main() {
    // Simulates render_wind_barbs_layer with a 256x256 output and real data
    let width = 256usize;
    let height = 256usize;

    // Simulated resampled data - should have width*height elements
    // Let's use 10 m/s northerly wind
    let u_data: Vec<f32> = vec![0.0; width * height];
    let v_data: Vec<f32> = vec![-10.0; width * height];

    println!("Simulating WMS scenario:");
    println!("  Output size: {}x{}", width, height);
    println!("  U data len: {}", u_data.len());
    println!("  V data len: {}", v_data.len());

    let config = BarbConfig {
        size: 40,
        spacing: 50,
        color: "#000000".to_string(),
    };

    println!(
        "  Barb config: size={}, spacing={}",
        config.size, config.spacing
    );

    let pixels = render_wind_barbs(&u_data, &v_data, width, height, &config);

    println!("  Pixel buffer size: {} bytes", pixels.len());
    println!("  Expected: {} bytes", width * height * 4);

    // Count non-transparent
    let mut non_transparent = 0;
    for i in (0..pixels.len()).step_by(4) {
        if pixels[i + 3] > 0 {
            non_transparent += 1;
        }
    }
    println!("  Non-transparent pixels: {}", non_transparent);

    if non_transparent == 0 {
        println!("ERROR: No barbs rendered!");
    } else {
        println!("SUCCESS: Barbs rendered");

        // Save to file
        let png = create_png(&pixels, width, height).unwrap();
        std::fs::write("test_wms_256x256.png", &png).unwrap();
        println!("  Saved to test_wms_256x256.png ({} bytes)", png.len());
    }
}
