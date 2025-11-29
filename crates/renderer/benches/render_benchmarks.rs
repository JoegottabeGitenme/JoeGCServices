//! Benchmarks for the renderer crate - gradient rendering and grid operations.
//!
//! Run with: cargo bench --package renderer -- gradient
//! Or: cargo bench --package renderer --bench render_benchmarks

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use rand::Rng;
use renderer::{gradient, png};

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
// TEMPERATURE RENDERING BENCHMARKS
// =============================================================================

fn bench_render_temperature(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_temperature");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];

    for (width, height) in sizes {
        // Generate temperature data in Celsius range
        let data: Vec<f32> = generate_linear_grid(width, height)
            .iter()
            .map(|v| v - 40.0) // Shift to -40 to +60 range
            .collect();

        group.throughput(Throughput::Elements((width * height) as u64));

        group.bench_with_input(
            BenchmarkId::new("celsius", format!("{}x{}", width, height)),
            &data,
            |b, data| {
                b.iter(|| {
                    gradient::render_temperature(black_box(data), width, height, -40.0, 60.0)
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// OTHER RENDER TYPES BENCHMARKS
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

    // Humidity (0-100%)
    let humidity_data = generate_linear_grid(width, height);

    group.throughput(Throughput::Elements((width * height) as u64));

    group.bench_function("wind_speed", |b| {
        b.iter(|| {
            gradient::render_wind_speed(black_box(&wind_data), width, height, 0.0, 40.0)
        });
    });

    group.bench_function("pressure", |b| {
        b.iter(|| {
            gradient::render_pressure(black_box(&pressure_data), width, height, 950.0, 1050.0)
        });
    });

    group.bench_function("humidity", |b| {
        b.iter(|| {
            gradient::render_humidity(black_box(&humidity_data), width, height, 0.0, 100.0)
        });
    });

    group.finish();
}

// =============================================================================
// PNG ENCODING BENCHMARKS
// =============================================================================

fn bench_png_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("png_encoding");

    let sizes = [(256, 256), (512, 512), (1024, 1024)];

    for (width, height) in sizes {
        let rgba_data = generate_rgba_data(width, height);

        group.throughput(Throughput::Bytes((width * height * 4) as u64));

        group.bench_with_input(
            BenchmarkId::new("create_png", format!("{}x{}", width, height)),
            &rgba_data,
            |b, data| {
                b.iter(|| png::create_png(black_box(data), width, height));
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

    // Simulate a complete tile render: resample -> color map -> PNG encode
    let gfs_data = generate_temperature_grid(1440, 721);

    group.throughput(Throughput::Elements(256 * 256));

    group.bench_function("temperature_tile_256x256", |b| {
        b.iter(|| {
            // Step 1: Resample from GFS grid to tile size
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                256,
                256,
            );

            // Step 2: Convert to Celsius and render
            let celsius: Vec<f32> = resampled.iter().map(|k| k - 273.15).collect();
            let rgba = gradient::render_temperature(&celsius, 256, 256, -40.0, 50.0);

            // Step 3: Encode as PNG
            png::create_png(&rgba, 256, 256)
        });
    });

    // Larger tile
    group.bench_function("temperature_tile_512x512", |b| {
        b.iter(|| {
            let resampled = gradient::resample_grid(
                black_box(&gfs_data),
                1440,
                721,
                512,
                512,
            );
            let celsius: Vec<f32> = resampled.iter().map(|k| k - 273.15).collect();
            let rgba = gradient::render_temperature(&celsius, 512, 512, -40.0, 50.0);
            png::create_png(&rgba, 512, 512)
        });
    });

    group.finish();
}

// =============================================================================
// COLOR FUNCTION BENCHMARKS
// =============================================================================

fn bench_color_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("color_functions");

    // Benchmark individual color mapping functions
    let temps: Vec<f32> = (-50..=50).map(|t| t as f32).collect();
    let speeds: Vec<f32> = (0..=40).map(|s| s as f32).collect();
    let pressures: Vec<f32> = (950..=1050).map(|p| p as f32).collect();

    group.bench_function("temperature_color_100", |b| {
        b.iter(|| {
            for &t in &temps {
                black_box(gradient::temperature_color(t));
            }
        });
    });

    group.bench_function("wind_speed_color_40", |b| {
        b.iter(|| {
            for &s in &speeds {
                black_box(gradient::wind_speed_color(s));
            }
        });
    });

    group.bench_function("pressure_color_100", |b| {
        b.iter(|| {
            for &p in &pressures {
                black_box(gradient::pressure_color(p));
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_resample_grid,
    bench_subset_grid,
    bench_render_grid,
    bench_render_temperature,
    bench_render_other_types,
    bench_png_encoding,
    bench_full_pipeline,
    bench_color_functions,
);
criterion_main!(benches);
