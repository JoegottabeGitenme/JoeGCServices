use renderer::barbs::{
    calculate_barb_positions, render_wind_barbs, uv_to_speed_direction, BarbConfig,
};

fn main() {
    let width = 512;
    let height = 256;

    let u_data = vec![5.0f32; width * height];
    let v_data = vec![-5.0f32; width * height];

    println!("Canvas size: {}x{}", width, height);

    let config = BarbConfig {
        size: 40,
        spacing: 50,
        color: "#000000".to_string(),
    };

    let positions = calculate_barb_positions(width, height, config.spacing);
    println!("Number of barb positions: {}", positions.len());

    // Check first position manually
    if let Some((x, y)) = positions.first() {
        let idx = y * width + x;
        let u = u_data[idx];
        let v = v_data[idx];
        println!("Position ({}, {}), idx={}", x, y, idx);
        println!("U={}, V={}", u, v);

        let (speed, dir) = uv_to_speed_direction(u, v);
        println!("Speed={} m/s, Direction={} rad", speed, dir);

        // Check what SVG would be selected
        // speed = sqrt(5^2 + 5^2) = 7.07 m/s
        // Looking at SPEED_RANGES_MS: 5.0-7.5 m/s -> "10" (10 knots)
        println!("Expected SVG: 10.svg (for 5-7.5 m/s range)");
    }

    // Try rendering a single barb manually
    let svg_10 = include_str!("../assets/wind-barbs/10.svg");
    println!("SVG 10 content length: {} bytes", svg_10.len());

    let opt = usvg::Options::default();
    match usvg::Tree::from_str(svg_10, &opt) {
        Ok(tree) => {
            println!("SVG parsed successfully");
            let size = tree.size();
            println!("SVG size: {}x{}", size.width(), size.height());

            // Create pixmap
            let mut pixmap = tiny_skia::Pixmap::new(40, 40).unwrap();

            // Apply transform
            let scale = 40.0 / size.width().max(size.height());
            let transform = tiny_skia::Transform::from_scale(scale, scale);

            println!("Scale factor: {}", scale);

            resvg::render(&tree, transform, &mut pixmap.as_mut());

            let mut non_zero = 0;
            for i in (0..pixmap.data().len()).step_by(4) {
                if pixmap.data()[i + 3] > 0 {
                    non_zero += 1;
                }
            }
            println!("Manual render non-transparent: {}", non_zero);
        }
        Err(e) => {
            println!("SVG parse error: {}", e);
        }
    }

    // Now try the full render
    let pixels = render_wind_barbs(&u_data, &v_data, width, height, &config);

    let mut non_transparent = 0;
    for i in (0..pixels.len()).step_by(4) {
        if pixels[i + 3] > 0 {
            non_transparent += 1;
        }
    }
    println!("Full render non-transparent pixels: {}", non_transparent);
}
