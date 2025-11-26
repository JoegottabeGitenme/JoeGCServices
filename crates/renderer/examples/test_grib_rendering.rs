use renderer::barbs::{BarbConfig, render_wind_barbs};
use renderer::gradient::resample_grid;
use renderer::png::create_png;

fn main() {
    // Test with U=5, V=-3 (this was failing before)
    let u: Vec<f32> = vec![5.0; 256 * 256];
    let v: Vec<f32> = vec![-3.0; 256 * 256];
    let config = BarbConfig { size: 40, spacing: 50, color: "#000000".to_string() };
    
    println!("Test: 256x256 with U=5 V=-3");
    let pixels = render_wind_barbs(&u, &v, 256, 256, &config);
    let nt = pixels.chunks(4).filter(|c| c[3] > 0).count();
    println!("  Non-transparent pixels: {}", nt);
    
    if nt > 0 {
        println!("SUCCESS!");
        let png = create_png(&pixels, 256, 256).unwrap();
        std::fs::write("test_wind_barbs_fixed.png", &png).unwrap();
        println!("Saved to test_wind_barbs_fixed.png");
    } else {
        println!("FAILED - still no pixels!");
    }
}
