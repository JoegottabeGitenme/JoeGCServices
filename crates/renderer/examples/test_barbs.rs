use renderer::barbs::{render_wind_barbs, uv_to_speed_direction, BarbConfig};

fn main() {
    // Create test data with a known wind pattern
    // 10 m/s wind from the north (U=0, V=-10)
    let width = 100;
    let height = 100;
    let u_data = vec![0.0f32; width * height];
    let v_data = vec![-10.0f32; width * height];

    let config = BarbConfig {
        size: 40,
        spacing: 50,
        color: "#000000".to_string(),
    };

    println!("Testing wind barb rendering...");
    println!("Data size: {}x{}", width, height);
    println!("Sample U/V: {}, {}", u_data[0], v_data[0]);

    let (speed, dir) = uv_to_speed_direction(u_data[0], v_data[0]);
    println!("Converted to speed={} m/s, direction={} rad", speed, dir);

    let pixels = render_wind_barbs(&u_data, &v_data, width, height, &config);

    // Check if any pixels are non-transparent
    let mut non_transparent = 0;
    for i in (0..pixels.len()).step_by(4) {
        if pixels[i + 3] > 0 {
            non_transparent += 1;
        }
    }

    println!("Total pixels: {}", width * height);
    println!("Non-transparent pixels: {}", non_transparent);
    println!("Pixels buffer size: {} bytes", pixels.len());

    if non_transparent == 0 {
        println!("WARNING: All pixels are transparent!");
    } else {
        println!("SUCCESS: Wind barbs are rendering");
    }
}
