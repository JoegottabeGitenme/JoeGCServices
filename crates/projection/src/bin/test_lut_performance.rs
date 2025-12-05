//! Integration test to verify LUT performance gains.
//!
//! This test:
//! 1. Generates synthetic GOES data
//! 2. Computes LUTs for several test tiles
//! 3. Compares on-the-fly projection vs LUT-based resampling
//! 4. Reports performance statistics
//!
//! Run with: cargo run --release --bin test-lut-performance

use projection::{
    compute_tile_lut, resample_with_lut, Geostationary,
};
use std::time::Instant;

fn main() {
    println!("LUT Performance Test");
    println!("====================\n");

    // GOES CONUS grid size (typical for 2km resolution)
    let goes_width = 2500;
    let goes_height = 1500;
    let proj = Geostationary::goes16_conus();

    // Generate synthetic GOES data (brightness temperatures)
    println!("Generating synthetic GOES data ({} x {})...", goes_width, goes_height);
    let goes_data = generate_goes_data(goes_width, goes_height);
    println!("  Data size: {} values\n", goes_data.len());

    // Test tiles at different zoom levels
    let test_tiles = [
        (5, 7, 11, "z5_central_conus"),
        (5, 8, 11, "z5_midwest"),
        (6, 14, 22, "z6_kansas"),
        (6, 15, 23, "z6_oklahoma"),
        (7, 28, 44, "z7_detailed_1"),
        (7, 30, 46, "z7_detailed_2"),
    ];

    println!("Test Tiles:");
    println!("{:-<70}", "");
    println!(
        "{:<25} {:>12} {:>12} {:>12}",
        "Tile", "On-the-fly", "With LUT", "Speedup"
    );
    println!("{:-<70}", "");

    let mut total_on_the_fly = 0u128;
    let mut total_with_lut = 0u128;
    let mut tile_count = 0;

    for (z, x, y, name) in test_tiles {
        // Calculate tile bbox
        let n = 2u32.pow(z) as f64;
        let lon_min = x as f64 / n * 360.0 - 180.0;
        let lon_max = (x + 1) as f64 / n * 360.0 - 180.0;
        let lat_max = (std::f64::consts::PI * (1.0 - 2.0 * y as f64 / n))
            .sinh()
            .atan()
            .to_degrees();
        let lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (y + 1) as f64 / n))
            .sinh()
            .atan()
            .to_degrees();
        let bbox = [lon_min as f32, lat_min as f32, lon_max as f32, lat_max as f32];

        // Pre-compute LUT
        let lut = compute_tile_lut(&proj, z, x, y, goes_width, goes_height);
        let valid_pixels = lut.valid_count();

        if valid_pixels == 0 {
            println!("{:<25} (outside GOES coverage)", name);
            continue;
        }

        // Benchmark on-the-fly projection (10 iterations)
        let iterations = 10;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = resample_goes_to_mercator(
                &goes_data,
                goes_width,
                goes_height,
                256,
                256,
                bbox,
                &proj,
            );
        }
        let on_the_fly_us = start.elapsed().as_micros() / iterations as u128;

        // Benchmark LUT-based resampling (10 iterations)
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = resample_with_lut(&goes_data, goes_width, &lut);
        }
        let with_lut_us = start.elapsed().as_micros() / iterations as u128;

        let speedup = on_the_fly_us as f64 / with_lut_us as f64;

        println!(
            "{:<25} {:>10} µs {:>10} µs {:>11.1}x",
            name, on_the_fly_us, with_lut_us, speedup
        );

        total_on_the_fly += on_the_fly_us;
        total_with_lut += with_lut_us;
        tile_count += 1;
    }

    println!("{:-<70}", "");

    if tile_count > 0 {
        let avg_on_the_fly = total_on_the_fly / tile_count as u128;
        let avg_with_lut = total_with_lut / tile_count as u128;
        let avg_speedup = avg_on_the_fly as f64 / avg_with_lut as f64;

        println!(
            "{:<25} {:>10} µs {:>10} µs {:>11.1}x",
            "AVERAGE", avg_on_the_fly, avg_with_lut, avg_speedup
        );
        println!("{:-<70}", "");

        println!("\nSummary:");
        println!("  Tiles tested: {}", tile_count);
        println!("  Average on-the-fly: {} µs ({:.2} ms)", avg_on_the_fly, avg_on_the_fly as f64 / 1000.0);
        println!("  Average with LUT: {} µs ({:.2} ms)", avg_with_lut, avg_with_lut as f64 / 1000.0);
        println!("  Average speedup: {:.1}x", avg_speedup);

        // Verify significant speedup
        if avg_speedup >= 10.0 {
            println!("\n✓ PASS: LUT provides significant speedup (>= 10x)");
        } else if avg_speedup >= 5.0 {
            println!("\n⚠ WARN: LUT speedup is moderate ({:.1}x). Expected >= 10x", avg_speedup);
        } else {
            println!("\n✗ FAIL: LUT speedup is low ({:.1}x). Something may be wrong.", avg_speedup);
            std::process::exit(1);
        }
    }
}

/// Generate synthetic GOES IR brightness temperature data.
fn generate_goes_data(width: usize, height: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; width * height];

    for j in 0..height {
        for i in 0..width {
            // Simulate brightness temperatures (200K to 310K)
            // with some spatial variation
            let lat_factor = (j as f32 / height as f32) * 2.0 - 1.0;
            let lon_factor = (i as f32 / width as f32) * 2.0 - 1.0;
            
            let base_temp = 260.0;
            let variation = lat_factor * 30.0 + lon_factor.abs() * 10.0;
            let noise = ((i * 17 + j * 31) % 100) as f32 * 0.1 - 5.0;
            
            data[j * width + i] = base_temp + variation + noise;
        }
    }

    data
}

/// Full GOES-to-Mercator resampling with projection transforms.
fn resample_goes_to_mercator(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4],
    proj: &Geostationary,
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;

    let min_merc_y = lat_to_mercator_y(out_min_lat as f64);
    let max_merc_y = lat_to_mercator_y(out_max_lat as f64);

    let mut output = vec![f32::NAN; output_width * output_height];

    for out_y in 0..output_height {
        for out_x in 0..output_width {
            let x_ratio = (out_x as f32 + 0.5) / output_width as f32;
            let y_ratio = (out_y as f32 + 0.5) / output_height as f32;

            let lon = out_min_lon + x_ratio * (out_max_lon - out_min_lon);
            let merc_y = max_merc_y - y_ratio as f64 * (max_merc_y - min_merc_y);
            let lat = mercator_y_to_lat(merc_y);

            let grid_coords = proj.geo_to_grid(lat, lon as f64);

            let (grid_i, grid_j) = match grid_coords {
                Some((i, j)) => (i, j),
                None => continue,
            };

            if grid_i < 0.0
                || grid_i >= data_width as f64 - 1.0
                || grid_j < 0.0
                || grid_j >= data_height as f64 - 1.0
            {
                continue;
            }

            // Bilinear interpolation
            let i1 = grid_i.floor() as usize;
            let j1 = grid_j.floor() as usize;
            let i2 = (i1 + 1).min(data_width - 1);
            let j2 = (j1 + 1).min(data_height - 1);

            let di = (grid_i - i1 as f64) as f32;
            let dj = (grid_j - j1 as f64) as f32;

            let v11 = data[j1 * data_width + i1];
            let v21 = data[j1 * data_width + i2];
            let v12 = data[j2 * data_width + i1];
            let v22 = data[j2 * data_width + i2];

            if v11.is_nan() || v21.is_nan() || v12.is_nan() || v22.is_nan() {
                continue;
            }

            let v1 = v11 * (1.0 - di) + v21 * di;
            let v2 = v12 * (1.0 - di) + v22 * di;
            output[out_y * output_width + out_x] = v1 * (1.0 - dj) + v2 * dj;
        }
    }

    output
}

fn lat_to_mercator_y(lat_deg: f64) -> f64 {
    let lat_rad = lat_deg.to_radians();
    lat_rad.tan().asinh()
}

fn mercator_y_to_lat(merc_y: f64) -> f64 {
    merc_y.sinh().atan().to_degrees()
}
