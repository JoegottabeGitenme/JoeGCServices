//! Benchmarks for the renderer crate - gradient rendering and grid operations.
//!
//! Run with: cargo bench --package renderer -- gradient
//! Or: cargo bench --package renderer --bench render_benchmarks

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use rand::Rng;
use renderer::{gradient, png, style};

/// Generate a test temperature grid with realistic patterns.
/// Values are in Kelvin (typical surface temps: 220K to 320K).
fn generate_temperature_grid(width: usize, height: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    let mut data = vec![0.0f32; width * height];

    for y in 0..height {
        for x in 0..width {
            // Base temperature varies with latitude (y position)
            let lat_factor = (y as f32 / height as f32 - 0.5) * 60.0;
            // Add some longitudinal variation
            let lon_factor = ((x as f32 / width as f32) * std::f32::consts::PI * 4.0).sin() * 5.0;
            // Add noise
            let noise = rng.gen_range(-3.0..3.0);

            data[y * width + x] = 273.15 + lat_factor + lon_factor + noise;
        }
    }
    data
}

/// Generate a simple linear gradient grid for consistent benchmarks.
fn generate_linear_grid(width: usize, height: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = (x as f32 + y as f32) / (width + height) as f32 * 100.0;
        }
    }
    data
}

/// Generate random RGBA pixel data for PNG encoding benchmarks.
fn generate_rgba_data(width: usize, height: usize) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let mut data = vec![0u8; width * height * 4];
    for chunk in data.chunks_mut(4) {
        chunk[0] = rng.gen(); // R
        chunk[1] = rng.gen(); // G
        chunk[2] = rng.gen(); // B
        chunk[3] = 255; // A (fully opaque)
    }
    data
}

/// Create a simple temperature style for benchmarking
fn create_temperature_style() -> style::StyleDefinition {
    style::StyleDefinition {
        name: "Temperature".to_string(),
        description: Some("Temperature gradient".to_string()),
        style_type: "gradient".to_string(),
        default: true,
        units: Some("K".to_string()),
        range: Some(style::ValueRange { min: 233.15, max: 313.15 }),
        transform: None,
        stops: vec![
            style::ColorStop { value: 233.15, color: "#1E0082".to_string(), label: Some("-40°C".to_string()) },
            style::ColorStop { value: 253.15, color: "#0096FF".to_string(), label: Some("-20°C".to_string()) },
            style::ColorStop { value: 273.15, color: "#96FFC8".to_string(), label: Some("0°C".to_string()) },
            style::ColorStop { value: 293.15, color: "#FF9600".to_string(), label: Some("20°C".to_string()) },
            style::ColorStop { value: 313.15, color: "#960000".to_string(), label: Some("40°C".to_string()) },
        ],
        interpolation: Some("linear".to_string()),
        out_of_range: Some("clamp".to_string()),
        legend: None,
    }
}

/// Create a simple wind speed style for benchmarking
fn create_wind_speed_style() -> style::StyleDefinition {
    style::StyleDefinition {
        name: "Wind Speed".to_string(),
        description: Some("Wind speed gradient".to_string()),
        style_type: "gradient".to_string(),
        default: true,
        units: Some("m/s".to_string()),
        range: Some(style::ValueRange { min: 0.0, max: 40.0 }),
        transform: None,
        stops: vec![
            style::ColorStop { value: 0.0, color: "#C8C8C8".to_string(), label: Some("0".to_string()) },
            style::ColorStop { value: 10.0, color: "#00C8FF".to_string(), label: Some("10".to_string()) },
            style::ColorStop { value: 20.0, color: "#FFFF00".to_string(), label: Some("20".to_string()) },
            style::ColorStop { value: 30.0, color: "#FFA500".to_string(), label: Some("30".to_string()) },
            style::ColorStop { value: 40.0, color: "#8B0000".to_string(), label: Some("40".to_string()) },
        ],
        interpolation: Some("linear".to_string()),
        out_of_range: Some("clamp".to_string()),
        legend: None,
    }
}

/// Create a simple pressure style for benchmarking
fn create_pressure_style() -> style::StyleDefinition {
    style::StyleDefinition {
        name: "Pressure".to_string(),
        description: Some("Pressure gradient".to_string()),
        style_type: "gradient".to_string(),
        default: true,
        units: Some("hPa".to_string()),
        range: Some(style::ValueRange { min: 950.0, max: 1050.0 }),
        transform: None,
        stops: vec![
            style::ColorStop { value: 950.0, color: "#4B0082".to_string(), label: Some("950".to_string()) },
            style::ColorStop { value: 990.0, color: "#0000FF".to_string(), label: Some("990".to_string()) },
            style::ColorStop { value: 1010.0, color: "#00FF00".to_string(), label: Some("1010".to_string()) },
            style::ColorStop { value: 1030.0, color: "#FFFF00".to_string(), label: Some("1030".to_string()) },
            style::ColorStop { value: 1050.0, color: "#FF0000".to_string(), label: Some("1050".to_string()) },
        ],
        interpolation: Some("linear".to_string()),
        out_of_range: Some("clamp".to_string()),
        legend: None,
    }
}

// =============================================================================
// RESAMPLE GRID BENCHMARKS
// =============================================================================

fn bench_resample_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("resample_grid");

    // Common scenarios: source -> destination sizes
    let scenarios = [
        // (src_width, src_height, dst_width, dst_height, name)
        (1440, 721, 256, 256, "GFS_to_tile"),     // Global GFS to single tile
        (7000, 3500, 256, 256, "MRMS_to_tile"),   // MRMS CONUS to tile
        (2500, 1500, 256, 256, "GOES_to_tile"),   // GOES CONUS to tile
        (256, 256, 512, 512, "upscale_2x"),       // Upscale
        (512, 512, 256, 256, "downscale_2x"),     // Downscale
        (1440, 721, 512, 512, "GFS_to_large"),    // Larger output
    ];

    for (src_w, src_h, dst_w, dst_h, name) in scenarios {
        let data = generate_linear_grid(src_w, src_h);

        group.throughput(Throughput::Elements((dst_w * dst_h) as u64));
        group.bench_with_input(BenchmarkId::new(name, "bilinear"), &data, |b, data| {
            b.iter(|| {
                gradient::resample_grid(black_box(data), src_w, src_h, dst_w, dst_h)
            });
        });
    }

    group.finish();
}

// =============================================================================
// SUBSET GRID BENCHMARKS
// =============================================================================

fn bench_subset_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("subset_grid");

    // Test subsetting a global grid to various bboxes
    let gfs_data = generate_linear_grid(1440, 721);

    let bboxes = [
        ([-130.0f32, 20.0, -60.0, 55.0], "conus"),        // Continental US
        ([-180.0f32, -60.0, -60.0, 60.0], "americas"),    // Americas
        ([0.0f32, 30.0, 50.0, 70.0], "europe"),           // Europe
        ([-10.0f32, 40.0, 5.0, 50.0], "small_region"),    // Small area
    ];

    for (bbox, name) in bboxes {
        group.bench_with_input(BenchmarkId::new(name, "GFS"), &bbox, |b, bbox| {
            b.iter(|| gradient::subset_grid(black_box(&gfs_data), 1440, 721, black_box(bbox)));
        });
    }

    group.finish();
}

// =============================================================================
// RENDER GRID BENCHMARKS
// =============================================================================

fn bench_render_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_grid");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];

    for (width, height) in sizes {
        let data = generate_linear_grid(width, height);

        group.throughput(Throughput::Elements((width * height) as u64));

        // Generic render with closure
        group.bench_with_input(
            BenchmarkId::new("generic", format!("{}x{}", width, height)),
            &data,
            |b, data| {
                b.iter(|| {
                    gradient::render_grid(black_box(data), width, height, 0.0, 100.0, |norm| {
                        gradient::Color::new(
                            (norm * 255.0) as u8,
                            ((1.0 - norm) * 255.0) as u8,
                            128,
                            255,
                        )
                    })
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// STYLE-BASED RENDERING BENCHMARKS
// =============================================================================

fn bench_style_rendering(c: &mut Criterion) {
    let mut group = c.benchmark_group("style_rendering");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];
    let temp_style = create_temperature_style();

    for (width, height) in sizes {
        // Generate temperature data in Kelvin
        let data: Vec<f32> = generate_linear_grid(width, height)
            .iter()
            .map(|v| 233.15 + v * 0.8) // Scale to 233K-313K range
            .collect();

        group.throughput(Throughput::Elements((width * height) as u64));

        group.bench_with_input(
            BenchmarkId::new("temperature_style", format!("{}x{}", width, height)),
            &data,
            |b, data| {
                b.iter(|| {
                    style::apply_style_gradient(black_box(data), width, height, &temp_style)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// OTHER RENDER TYPES BENCHMARKS (using styles)
// =============================================================================

fn bench_render_other_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_other");

    let width = 256;
    let height = 256;

    // Wind speed (0-40 m/s)
    let wind_data: Vec<f32> = generate_linear_grid(width, height)
        .iter()
        .map(|v| v * 0.4) // Scale to 0-40 range
        .collect();

    // Pressure (950-1050 hPa)
    let pressure_data: Vec<f32> = generate_linear_grid(width, height)
        .iter()
        .map(|v| 950.0 + v) // Scale to 950-1050 range
        .collect();

    let wind_style = create_wind_speed_style();
    let pressure_style = create_pressure_style();

    group.throughput(Throughput::Elements((width * height) as u64));

    group.bench_function("wind_speed_style", |b| {
        b.iter(|| {
            style::apply_style_gradient(black_box(&wind_data), width, height, &wind_style)
        });
    });

    group.bench_function("pressure_style", |b| {
        b.iter(|| {
            style::apply_style_gradient(black_box(&pressure_data), width, height, &pressure_style)
        });
    });

    group.finish();
}

// =============================================================================
// PNG ENCODING BENCHMARKS
// =============================================================================

/// Generate weather-like RGBA data with limited unique colors.
/// This simulates the output of style-based rendering with gradients.
fn generate_weather_rgba_data(width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    
    // Use a limited palette of ~50 colors (typical for weather gradients)
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            // Quantize to create limited unique colors
            let temp_normalized = (x as f32 / width as f32 + y as f32 / height as f32) / 2.0;
            let color_idx = (temp_normalized * 50.0) as u8;
            
            // Simple color ramp (blue -> cyan -> green -> yellow -> red)
            let (r, g, b) = match color_idx {
                0..=10 => (0, 0, 128 + color_idx * 12),
                11..=20 => (0, (color_idx - 10) * 25, 255),
                21..=30 => (0, 255, 255 - (color_idx - 20) * 25),
                31..=40 => ((color_idx - 30) * 25, 255, 0),
                _ => (255, 255 - (color_idx - 40) * 25, 0),
            };
            
            data[idx] = r;
            data[idx + 1] = g;
            data[idx + 2] = b;
            data[idx + 3] = 255;
        }
    }
    data
}

fn bench_png_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("png_encoding");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];

    // Benchmark with random data (many unique colors - RGBA fallback)
    for (width, height) in sizes {
        let rgba_data = generate_rgba_data(width, height);

        group.throughput(Throughput::Bytes((width * height * 4) as u64));

        group.bench_with_input(
            BenchmarkId::new("rgba_random", format!("{}x{}", width, height)),
            &rgba_data,
            |b, data| {
                b.iter(|| png::create_png(black_box(data), width, height));
            },
        );
    }

    // Benchmark with weather-like data (limited colors - indexed PNG)
    for (width, height) in sizes {
        let weather_data = generate_weather_rgba_data(width, height);

        group.throughput(Throughput::Bytes((width * height * 4) as u64));

        // Auto mode (should choose indexed for weather data)
        group.bench_with_input(
            BenchmarkId::new("auto_weather", format!("{}x{}", width, height)),
            &weather_data,
            |b, data| {
                b.iter(|| png::create_png_auto(black_box(data), width, height));
            },
        );

        // Force RGBA for comparison
        group.bench_with_input(
            BenchmarkId::new("rgba_weather", format!("{}x{}", width, height)),
            &weather_data,
            |b, data| {
                b.iter(|| png::create_png(black_box(data), width, height));
            },
        );
    }

    group.finish();
}

// =============================================================================
// PRE-COMPUTED PALETTE BENCHMARKS
// =============================================================================

fn bench_precomputed_palette(c: &mut Criterion) {
    let mut group = c.benchmark_group("precomputed_palette");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];
    let temp_style = create_temperature_style();
    
    // Pre-compute the palette (this happens once at startup)
    let palette = temp_style.compute_palette().expect("Failed to compute palette");
    
    for (width, height) in sizes {
        // Generate temperature data in Kelvin
        let data: Vec<f32> = generate_linear_grid(width, height)
            .iter()
            .map(|v| 233.15 + v * 0.8) // Scale to 233K-313K range
            .collect();

        group.throughput(Throughput::Elements((width * height) as u64));

        // Benchmark: Pre-computed palette (indexed rendering)
        group.bench_with_input(
            BenchmarkId::new("indexed_render", format!("{}x{}", width, height)),
            &data,
            |b, data| {
                b.iter(|| {
                    style::apply_style_gradient_indexed(
                        black_box(data),
                        width,
                        height,
                        &palette,
                        &temp_style,
                    )
                });
            },
        );

        // Benchmark: Original RGBA rendering (for comparison)
        group.bench_with_input(
            BenchmarkId::new("rgba_render", format!("{}x{}", width, height)),
            &data,
            |b, data| {
                b.iter(|| {
                    style::apply_style_gradient(black_box(data), width, height, &temp_style)
                });
            },
        );
    }

    group.finish();
}

fn bench_precomputed_png_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("precomputed_png");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];
    let temp_style = create_temperature_style();
    let palette = temp_style.compute_palette().expect("Failed to compute palette");

    for (width, height) in sizes {
        // Generate temperature data
        let data: Vec<f32> = generate_linear_grid(width, height)
            .iter()
            .map(|v| 233.15 + v * 0.8)
            .collect();

        // Pre-render to indices
        let indices = style::apply_style_gradient_indexed(&data, width, height, &palette, &temp_style);
        
        // Pre-render to RGBA for comparison
        let rgba = style::apply_style_gradient(&data, width, height, &temp_style);

        group.throughput(Throughput::Bytes((width * height) as u64));

        // Benchmark: PNG from pre-computed palette (skips palette extraction!)
        group.bench_with_input(
            BenchmarkId::new("from_precomputed", format!("{}x{}", width, height)),
            &indices,
            |b, indices| {
                b.iter(|| {
                    png::create_png_from_precomputed(black_box(indices), width, height, &palette)
                });
            },
        );

        // Benchmark: PNG auto (extracts palette at runtime)
        group.bench_with_input(
            BenchmarkId::new("auto_extract", format!("{}x{}", width, height)),
            &rgba,
            |b, rgba| {
                b.iter(|| png::create_png_auto(black_box(rgba), width, height));
            },
        );

        // Benchmark: RGBA PNG (no palette, larger file)
        group.bench_with_input(
            BenchmarkId::new("rgba_direct", format!("{}x{}", width, height)),
            &rgba,
            |b, rgba| {
                b.iter(|| png::create_png(black_box(rgba), width, height));
            },
        );
    }

    group.finish();
}

// =============================================================================
// FULL PIPELINE BENCHMARKS
// =============================================================================

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");

    // Simulate a complete tile render: resample -> style color map -> PNG encode
    let gfs_data = generate_temperature_grid(1440, 721);
    let temp_style = create_temperature_style();
    let palette = temp_style.compute_palette().expect("Failed to compute palette");

    group.throughput(Throughput::Elements(256 * 256));

    // Full pipeline with RGBA PNG (baseline)
    group.bench_function("temp_256_rgba", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                256,
                256,
            );
            let rgba = style::apply_style_gradient(&resampled, 256, 256, &temp_style);
            png::create_png(&rgba, 256, 256)
        });
    });

    // Full pipeline with auto PNG (indexed when possible)
    group.bench_function("temp_256_auto", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                256,
                256,
            );
            let rgba = style::apply_style_gradient(&resampled, 256, 256, &temp_style);
            png::create_png_auto(&rgba, 256, 256)
        });
    });

    // Full pipeline with PRE-COMPUTED palette (fastest path!)
    group.bench_function("temp_256_precomputed", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                256,
                256,
            );
            let indices = style::apply_style_gradient_indexed(&resampled, 256, 256, &palette, &temp_style);
            png::create_png_from_precomputed(&indices, 256, 256, &palette)
        });
    });

    // Larger tile comparisons
    group.throughput(Throughput::Elements(512 * 512));
    
    group.bench_function("temp_512_rgba", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                512,
                512,
            );
            let rgba = style::apply_style_gradient(&resampled, 512, 512, &temp_style);
            png::create_png(&rgba, 512, 512)
        });
    });

    group.bench_function("temp_512_precomputed", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                512,
                512,
            );
            let indices = style::apply_style_gradient_indexed(&resampled, 512, 512, &palette, &temp_style);
            png::create_png_from_precomputed(&indices, 512, 512, &palette)
        });
    });

    group.finish();
}

// =============================================================================
// BUFFER POOLING BENCHMARKS
// =============================================================================

/// Benchmark buffer pool performance vs fresh allocations.
/// 
/// Buffer pooling is designed to improve p99 latency under high load by
/// reducing allocator contention. The benefits are most visible under
/// concurrent access, but we can measure the overhead here.
fn bench_buffer_pooling(c: &mut Criterion) {
    use renderer::buffer_pool;
    
    let mut group = c.benchmark_group("buffer_pooling");
    
    let sizes = [(256, 256), (512, 512), (1024, 1024)];
    let temp_style = create_temperature_style();
    let palette = temp_style.compute_palette().expect("Failed to compute palette");
    
    for (width, height) in sizes {
        let size_name = format!("{}x{}", width, height);
        
        // Generate temperature data
        let data: Vec<f32> = generate_linear_grid(width, height)
            .iter()
            .map(|v| 233.15 + v * 0.8)
            .collect();
        
        group.throughput(Throughput::Elements((width * height) as u64));
        
        // Benchmark: Multiple iterations with buffer pool (simulates reuse)
        group.bench_function(format!("pool_repeated_{}", size_name), |b| {
            b.iter(|| {
                // This uses the buffer pool under the hood
                let indices = style::apply_style_gradient_indexed(
                    black_box(&data),
                    width,
                    height,
                    &palette,
                    &temp_style,
                );
                black_box(indices)
            });
        });
        
        // Benchmark: Direct allocation without pool
        group.bench_function(format!("alloc_direct_{}", size_name), |b| {
            b.iter(|| {
                // Direct allocation (similar to old code path)
                let mut indices = vec![0u8; width * height];
                for (i, val) in data.iter().enumerate() {
                    if !val.is_nan() {
                        let t = (val - palette.min_value) / (palette.max_value - palette.min_value);
                        let lut_idx = (t * 4095.0) as usize;
                        indices[i] = palette.value_to_index[lut_idx.min(4095)];
                    }
                }
                black_box(indices)
            });
        });
        
        // Benchmark: With resampling (tests resample buffer pool)
        let larger_data = generate_temperature_grid(1440, 721);
        group.bench_function(format!("pool_with_resample_{}", size_name), |b| {
            b.iter(|| {
                let resampled = gradient::resample_grid(
                    black_box(&larger_data),
                    1440,
                    721,
                    width,
                    height,
                );
                let indices = style::apply_style_gradient_indexed(
                    &resampled,
                    width,
                    height,
                    &palette,
                    &temp_style,
                );
                black_box(indices)
            });
        });
    }
    
    // Benchmark pool stats access (should be very fast)
    group.bench_function("get_pool_stats", |b| {
        // Warm up the pools
        buffer_pool::with_pixel_buffer(256, 256, |_| {});
        buffer_pool::with_index_buffer(256, 256, |_| {});
        
        b.iter(|| {
            let stats = buffer_pool::get_pool_stats();
            black_box(stats)
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_resample_grid,
    bench_subset_grid,
    bench_render_grid,
    bench_style_rendering,
    bench_render_other_types,
    bench_png_encoding,
    bench_precomputed_palette,
    bench_precomputed_png_encoding,
    bench_full_pipeline,
    bench_buffer_pooling,
);
criterion_main!(benches);
