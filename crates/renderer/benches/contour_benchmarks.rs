//! Benchmarks for contour line generation and rendering.
//!
//! Run with: cargo bench --package renderer --bench contour_benchmarks

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::Rng;
use renderer::contour::{
    connect_segments, generate_all_contours, generate_contour_levels, march_squares,
    render_contours, render_contours_to_canvas, smooth_contour, Contour, ContourConfig, Point,
};
use renderer::png;

/// Generate a smooth temperature-like field with hills and valleys.
fn generate_smooth_field(width: usize, height: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; width * height];

    for y in 0..height {
        for x in 0..width {
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;

            // Create multiple overlapping sine waves for a realistic pattern
            let v1 = (fx * std::f32::consts::PI * 4.0).sin() * 20.0;
            let v2 = (fy * std::f32::consts::PI * 4.0).sin() * 20.0;
            let v3 = ((fx + fy) * std::f32::consts::PI * 2.0).sin() * 10.0;

            data[y * width + x] = 50.0 + v1 + v2 + v3;
        }
    }
    data
}

/// Generate a field with random noise (more contour segments).
fn generate_noisy_field(width: usize, height: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    let base = generate_smooth_field(width, height);
    base.iter().map(|&v| v + rng.gen_range(-5.0..5.0)).collect()
}

/// Generate a simple linear gradient field.
fn generate_linear_field(width: usize, height: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            data[y * width + x] = (x as f32 / width as f32) * 100.0;
        }
    }
    data
}

// =============================================================================
// CONTOUR LEVEL GENERATION BENCHMARKS
// =============================================================================

fn bench_generate_contour_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_contour_levels");

    let ranges = [
        (0.0f32, 100.0, 10.0, "0-100_by_10"),
        (0.0f32, 100.0, 5.0, "0-100_by_5"),
        (0.0f32, 100.0, 2.0, "0-100_by_2"),
        (-50.0f32, 50.0, 5.0, "neg50-50_by_5"),
        (900.0f32, 1100.0, 4.0, "pressure_4hPa"),
    ];

    for (min, max, interval, name) in ranges {
        group.bench_with_input(
            BenchmarkId::new("levels", name),
            &(min, max, interval),
            |b, &(min, max, interval)| {
                b.iter(|| generate_contour_levels(black_box(min), black_box(max), black_box(interval)));
            },
        );
    }

    group.finish();
}

// =============================================================================
// MARCHING SQUARES BENCHMARKS
// =============================================================================

fn bench_march_squares(c: &mut Criterion) {
    let mut group = c.benchmark_group("march_squares");

    // Test different grid sizes
    let sizes = [(64, 64), (128, 128), (256, 256), (512, 512)];

    for (width, height) in sizes {
        let smooth_data = generate_smooth_field(width, height);
        let noisy_data = generate_noisy_field(width, height);

        group.throughput(Throughput::Elements((width * height) as u64));

        // Single level, smooth data
        group.bench_with_input(
            BenchmarkId::new("smooth_single_level", format!("{}x{}", width, height)),
            &smooth_data,
            |b, data| {
                b.iter(|| march_squares(black_box(data), width, height, black_box(50.0)));
            },
        );

        // Single level, noisy data (more segments)
        group.bench_with_input(
            BenchmarkId::new("noisy_single_level", format!("{}x{}", width, height)),
            &noisy_data,
            |b, data| {
                b.iter(|| march_squares(black_box(data), width, height, black_box(50.0)));
            },
        );
    }

    group.finish();
}

// =============================================================================
// SEGMENT CONNECTION BENCHMARKS
// =============================================================================

fn bench_connect_segments(c: &mut Criterion) {
    let mut group = c.benchmark_group("connect_segments");

    // Generate segments from different field types
    let sizes = [(128, 128), (256, 256)];

    for (width, height) in sizes {
        let smooth_data = generate_smooth_field(width, height);
        let segments = march_squares(&smooth_data, width, height, 50.0);

        group.throughput(Throughput::Elements(segments.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("smooth", format!("{}x{}_{}seg", width, height, segments.len())),
            &segments,
            |b, segs| {
                b.iter(|| connect_segments(black_box(segs.clone())));
            },
        );
    }

    group.finish();
}

// =============================================================================
// CONTOUR SMOOTHING BENCHMARKS
// =============================================================================

fn bench_smooth_contour(c: &mut Criterion) {
    let mut group = c.benchmark_group("smooth_contour");

    // Create test contours of varying complexity
    let point_counts = [10, 50, 100, 500];

    for count in point_counts {
        let points: Vec<Point> = (0..count)
            .map(|i| {
                let angle = (i as f32 / count as f32) * std::f32::consts::PI * 2.0;
                let radius = 100.0 + (angle * 5.0).sin() * 20.0;
                Point::new(128.0 + radius * angle.cos(), 128.0 + radius * angle.sin())
            })
            .collect();

        let contour = Contour {
            level: 50.0,
            points,
            closed: true,
        };

        for passes in [1, 2, 3] {
            group.bench_with_input(
                BenchmarkId::new(format!("{}_passes", passes), format!("{}_points", count)),
                &contour,
                |b, c| {
                    b.iter(|| smooth_contour(black_box(c), black_box(passes)));
                },
            );
        }
    }

    group.finish();
}

// =============================================================================
// FULL CONTOUR GENERATION BENCHMARKS
// =============================================================================

fn bench_generate_all_contours(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_all_contours");
    group.sample_size(20); // Slower benchmark

    let sizes = [(128, 128), (256, 256)];

    for (width, height) in sizes {
        let data = generate_smooth_field(width, height);

        // Few levels
        let config_few = ContourConfig {
            levels: vec![20.0, 40.0, 60.0, 80.0],
            line_width: 2.0,
            line_color: [0, 0, 0, 255],
            smoothing_passes: 1,
        };

        // Many levels
        let config_many = ContourConfig {
            levels: (0..20).map(|i| 10.0 + i as f32 * 5.0).collect(),
            line_width: 2.0,
            line_color: [0, 0, 0, 255],
            smoothing_passes: 1,
        };

        group.bench_with_input(
            BenchmarkId::new("4_levels", format!("{}x{}", width, height)),
            &(data.clone(), config_few.clone()),
            |b, (data, config)| {
                b.iter(|| generate_all_contours(black_box(data), width, height, black_box(config)));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("20_levels", format!("{}x{}", width, height)),
            &(data.clone(), config_many.clone()),
            |b, (data, config)| {
                b.iter(|| generate_all_contours(black_box(data), width, height, black_box(config)));
            },
        );
    }

    group.finish();
}

// =============================================================================
// CONTOUR RENDERING BENCHMARKS
// =============================================================================

fn bench_render_contours_to_canvas(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_contours_to_canvas");
    group.sample_size(20);

    let sizes = [(256, 256), (512, 512)];

    for (width, height) in sizes {
        let data = generate_smooth_field(width, height);
        let config = ContourConfig {
            levels: vec![20.0, 40.0, 60.0, 80.0],
            line_width: 2.0,
            line_color: [0, 0, 0, 255],
            smoothing_passes: 1,
        };
        let contours = generate_all_contours(&data, width, height, &config);

        group.throughput(Throughput::Elements((width * height) as u64));

        group.bench_with_input(
            BenchmarkId::new("4_levels", format!("{}x{}", width, height)),
            &(contours.clone(), config.clone()),
            |b, (contours, config)| {
                b.iter(|| {
                    render_contours_to_canvas(black_box(contours), width, height, black_box(config))
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// FULL CONTOUR PIPELINE BENCHMARKS
// =============================================================================

fn bench_full_contour_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_contour_pipeline");
    group.sample_size(20);

    let data = generate_smooth_field(256, 256);

    // Standard temperature isolines
    let config = ContourConfig {
        levels: vec![-20.0, -10.0, 0.0, 10.0, 20.0, 30.0, 40.0],
        line_width: 2.0,
        line_color: [50, 50, 50, 255],
        smoothing_passes: 1,
    };

    group.bench_function("contour_256x256_7levels", |b| {
        b.iter(|| {
            // Generate contours
            let rgba = render_contours(black_box(&data), 256, 256, black_box(&config));

            // Encode as PNG
            png::create_png(&rgba, 256, 256)
        });
    });

    // More levels
    let config_dense = ContourConfig {
        levels: generate_contour_levels(0.0, 100.0, 2.0),
        line_width: 1.0,
        line_color: [0, 0, 0, 200],
        smoothing_passes: 1,
    };

    group.bench_function("contour_256x256_dense", |b| {
        b.iter(|| {
            let rgba = render_contours(black_box(&data), 256, 256, black_box(&config_dense));
            png::create_png(&rgba, 256, 256)
        });
    });

    group.finish();
}

// =============================================================================
// LINE WIDTH IMPACT BENCHMARKS
// =============================================================================

fn bench_line_width_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_width_impact");
    group.sample_size(20);

    let data = generate_smooth_field(256, 256);

    for line_width in [1.0f32, 2.0, 4.0, 8.0] {
        let config = ContourConfig {
            levels: vec![30.0, 50.0, 70.0],
            line_width,
            line_color: [0, 0, 0, 255],
            smoothing_passes: 1,
        };

        group.bench_with_input(BenchmarkId::new("width", line_width), &config, |b, config| {
            b.iter(|| render_contours(black_box(&data), 256, 256, black_box(config)));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_generate_contour_levels,
    bench_march_squares,
    bench_connect_segments,
    bench_smooth_contour,
    bench_generate_all_contours,
    bench_render_contours_to_canvas,
    bench_full_contour_pipeline,
    bench_line_width_impact,
);
criterion_main!(benches);
