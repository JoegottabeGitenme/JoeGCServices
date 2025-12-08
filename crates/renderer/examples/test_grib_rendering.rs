use renderer::barbs::{BarbConfig, render_wind_barbs, uv_to_speed_direction};
use renderer::png::create_png;

fn main() {
    let width = 300usize;
    let height = 300usize;
    
    // Create a 3x3 grid of barbs with different wind directions
    // Spacing of 100 gives us barbs at (50,50), (150,50), (250,50), etc.
    let mut u_data = vec![0.0f32; width * height];
    let mut v_data = vec![0.0f32; width * height];
    
    // Set specific wind patterns at the 9 barb positions
    // Wind FROM the named direction (so barb points that way)
    let patterns: [(usize, usize, f32, f32, &str); 9] = [
        // Row 1: North, NE, East
        (50, 50, 0.0, -10.0, "N"),   // North wind
        (150, 50, -7.0, -7.0, "NE"), // Northeast wind
        (250, 50, -10.0, 0.0, "E"),  // East wind
        // Row 2: NW, Calm, SE
        (50, 150, 7.0, -7.0, "NW"),  // Northwest wind
        (150, 150, 0.5, 0.5, "Calm"),// Calm
        (250, 150, -7.0, 7.0, "SE"), // Southeast wind
        // Row 3: West, SW, South
        (50, 250, 10.0, 0.0, "W"),   // West wind
        (150, 250, 7.0, 7.0, "SW"),  // Southwest wind  
        (250, 250, 0.0, 10.0, "S"),  // South wind
    ];
    
    // Fill in a 20x20 pixel area around each position with the wind values
    for (cx, cy, u, v, name) in &patterns {
        for dy in 0..100 {
            for dx in 0..100 {
                let x = cx.saturating_sub(50) + dx;
                let y = cy.saturating_sub(50) + dy;
                if x < width && y < height {
                    let idx = y * width + x;
                    u_data[idx] = *u;
                    v_data[idx] = *v;
                }
            }
        }
        let (_, dir) = uv_to_speed_direction(*u, *v);
        println!("{:4} wind at ({:3},{:3}): U={:5.1}, V={:5.1} -> dir={:5.1}°", 
            name, cx, cy, u, v, dir.to_degrees());
    }
    
    let config = BarbConfig { size: 40, spacing: 100, color: "#000000".to_string() };
    
    println!("\nRendering...");
    let pixels = render_wind_barbs(&u_data, &v_data, width, height, &config);
    
    let non_transparent = pixels.chunks(4).filter(|c| c[3] > 0).count();
    println!("Non-transparent pixels: {}", non_transparent);
    
    let png = create_png(&pixels, width, height).unwrap();
    std::fs::write("test_wind_directions.png", &png).unwrap();
    println!("Saved to test_wind_directions.png");
    println!("\nExpected layout:");
    println!("  N   NE   E");
    println!("  NW  C   SE"); 
    println!("  W   SW   S");
}
