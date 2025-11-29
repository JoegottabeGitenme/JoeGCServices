//! Benchmarks for wind barb rendering.
//!
//! Run with: cargo bench --package renderer --bench barbs_benchmarks

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::Rng;
use renderer::barbs::{
    calculate_barb_positions, calculate_barb_positions_geographic, render_wind_barbs,
    render_wind_barbs_aligned, uv_to_speed_direction, BarbConfig,
};
use renderer::png;

/// Generate U/V wind component grids with realistic patterns.
fn generate_wind_components(width: usize, height: usize) -> (Vec<f32>, Vec<f32>) {
    let mut rng = rand::thread_rng();
    let mut u_data = vec![0.0f32; width * height];
    let mut v_data = vec![0.0f32; width * height];

    for y in 0..height {
        for x in 0..width {
            // Create a flow pattern with some variation
            let base_u = ((x as f32 / width as f32) * std::f32::consts::PI * 2.0).sin() * 15.0;
            let base_v = ((y as f32 / height as f32) * std::f32::consts::PI * 2.0).cos() * 15.0;

            u_data[y * width + x] = base_u + rng.gen_range(-5.0..5.0);
            v_data[y * width + x] = base_v + rng.gen_range(-5.0..5.0);
        }
    }

    (u_data, v_data)
}

/// Generate uniform wind field for consistent benchmark timing.
fn generate_uniform_wind(width: usize, height: usize, u: f32, v: f32) -> (Vec<f32>, Vec<f32>) {
    let u_data = vec![u; width * height];
    let v_data = vec![v; width * height];
    (u_data, v_data)
}

// =============================================================================
// UV TO SPEED/DIRECTION BENCHMARKS
// =============================================================================

fn bench_uv_to_speed_direction(c: &mut Criterion) {
    let mut group = c.benchmark_group("uv_to_speed_direction");

    // Test with various wind speeds
    let test_cases: Vec<(f32, f32)> = (0..100)
        .map(|i| {
            let angle = (i as f32 / 100.0) * std::f32::consts::PI * 2.0;
            let speed = (i as f32 / 10.0).min(20.0);
            (speed * angle.cos(), speed * angle.sin())
        })
        .collect();

    group.bench_function("100_conversions", |b| {
        b.iter(|| {
            for &(u, v) in &test_cases {
                black_box(uv_to_speed_direction(u, v));
            }
        });
    });

    group.finish();
}

// =============================================================================
// BARB POSITION CALCULATION BENCHMARKS
// =============================================================================

fn bench_calculate_positions(c: &mut Criterion) {
    let mut group = c.benchmark_group("barb_positions");

    let configs = [
        (256, 256, 30, "tile_256"),
        (512, 512, 30, "tile_512"),
        (1024, 1024, 30, "tile_1024"),
        (256, 256, 50, "tile_256_sparse"),
        (256, 256, 20, "tile_256_dense"),
    ];

    for (width, height, spacing, name) in configs {
        group.bench_with_input(
            BenchmarkId::new("pixel_grid", name),
            &(width, height, spacing),
            |b, &(w, h, s)| {
                b.iter(|| calculate_barb_positions(black_box(w), black_box(h), black_box(s)));
            },
        );
    }

    group.finish();
}

fn bench_calculate_positions_geographic(c: &mut Criterion) {
    let mut group = c.benchmark_group("barb_positions_geographic");

    // Various bbox sizes and zoom levels
    let bboxes = [
        (256, 256, [-130.0f32, 20.0, -60.0, 55.0], 2.0, "conus_z5"),
        (256, 256, [-100.0f32, 30.0, -90.0, 40.0], 0.5, "region_z8"),
        (256, 256, [-80.0f32, 35.0, -75.0, 40.0], 0.1, "local_z11"),
        (512, 512, [-130.0f32, 20.0, -60.0, 55.0], 2.0, "conus_large"),
    ];

    for (width, height, bbox, spacing_deg, name) in bboxes {
        group.bench_with_input(
            BenchmarkId::new("geographic", name),
            &(width, height, bbox, spacing_deg),
            |b, &(w, h, bbox, spacing)| {
                b.iter(|| {
                    calculate_barb_positions_geographic(
                        black_box(w),
                        black_box(h),
                        black_box(bbox),
                        black_box(spacing),
                    )
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// WIND BARB RENDERING BENCHMARKS
// =============================================================================

fn bench_render_wind_barbs(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_wind_barbs");
    group.sample_size(20); // Reduce sample size for slower benchmarks

    let configs = [
        (256, 256, 50, "tile_256_default"),
        (256, 256, 30, "tile_256_dense"),
        (512, 512, 50, "tile_512"),
    ];

    for (width, height, spacing, name) in configs {
        let (u_data, v_data) = generate_uniform_wind(width, height, 10.0, 5.0);
        let config = BarbConfig {
            size: 40,
            spacing,
            color: "#000000".to_string(),
        };

        // Count expected barbs for throughput
        let positions = calculate_barb_positions(width, height, spacing);
        group.throughput(Throughput::Elements(positions.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("uniform_wind", name),
            &(width, height, &u_data, &v_data, &config),
            |b, (w, h, u, v, cfg)| {
                b.iter(|| render_wind_barbs(black_box(u), black_box(v), *w, *h, black_box(cfg)));
            },
        );
    }

    // Test with variable wind speeds
    let (u_varied, v_varied) = generate_wind_components(256, 256);
    let config = BarbConfig::default();
    group.bench_function("varied_wind_256", |b| {
        b.iter(|| render_wind_barbs(black_box(&u_varied), black_box(&v_varied), 256, 256, &config));
    });

    group.finish();
}

fn bench_render_wind_barbs_aligned(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_wind_barbs_aligned");
    group.sample_size(20);

    let bboxes = [
        ([-100.0f32, 30.0, -90.0, 40.0], "bbox_10deg"),
        ([-130.0f32, 20.0, -60.0, 55.0], "bbox_conus"),
        ([-85.0f32, 38.0, -82.0, 41.0], "bbox_3deg"),
    ];

    for (bbox, name) in bboxes {
        let (u_data, v_data) = generate_uniform_wind(256, 256, 10.0, 10.0);
        let config = BarbConfig::default();

        group.bench_with_input(
            BenchmarkId::new("aligned", name),
            &(bbox, &u_data, &v_data, &config),
            |b, (bbox, u, v, cfg)| {
                b.iter(|| {
                    render_wind_barbs_aligned(
                        black_box(u),
                        black_box(v),
                        256,
                        256,
                        black_box(*bbox),
                        black_box(cfg),
                    )
                });
            },
        );
    }

    group.finish();
}

// =============================================================================
// BARB SIZE IMPACT BENCHMARKS
// =============================================================================

fn bench_barb_size_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("barb_size_impact");
    group.sample_size(20);

    let (u_data, v_data) = generate_uniform_wind(256, 256, 10.0, 10.0);

    // Test different barb sizes
    for size in [24, 40, 64, 108] {
        let config = BarbConfig {
            size,
            spacing: 50,
            color: "#000000".to_string(),
        };

        group.bench_with_input(BenchmarkId::new("size", size), &config, |b, cfg| {
            b.iter(|| render_wind_barbs(black_box(&u_data), black_box(&v_data), 256, 256, cfg));
        });
    }

    group.finish();
}

// =============================================================================
// FULL BARB PIPELINE BENCHMARKS
// =============================================================================

fn bench_barb_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("barb_full_pipeline");
    group.sample_size(20);

    let (u_data, v_data) = generate_wind_components(256, 256);
    let config = BarbConfig::default();

    group.bench_function("render_and_png", |b| {
        b.iter(|| {
            // Render barbs
            let rgba = render_wind_barbs(black_box(&u_data), black_box(&v_data), 256, 256, &config);

            // Encode as PNG
            png::create_png(&rgba, 256, 256)
        });
    });

    group.finish();
}

// =============================================================================
// WIND SPEED DISTRIBUTION BENCHMARKS
// =============================================================================

fn bench_wind_speed_distribution(c: &mut Criterion) {
    let mut group = c.benchmark_group("wind_speed_distribution");
    group.sample_size(20);

    // Test rendering performance at different wind speeds
    // (Different SVG barb graphics are used for different speeds)
    let speeds = [
        (0.5, 0.5, "calm"),      // 0 knots barb
        (5.0, 5.0, "moderate"),  // ~14 knots
        (15.0, 15.0, "strong"),  // ~42 knots
        (30.0, 30.0, "gale"),    // ~85 knots
    ];

    for (u, v, name) in speeds {
        let (u_data, v_data) = generate_uniform_wind(256, 256, u, v);
        let config = BarbConfig::default();

        group.bench_with_input(BenchmarkId::new("speed", name), &(u_data, v_data), |b, (u, v)| {
            b.iter(|| render_wind_barbs(black_box(u), black_box(v), 256, 256, &config));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_uv_to_speed_direction,
    bench_calculate_positions,
    bench_calculate_positions_geographic,
    bench_render_wind_barbs,
    bench_render_wind_barbs_aligned,
    bench_barb_size_impact,
    bench_barb_full_pipeline,
    bench_wind_speed_distribution,
);
criterion_main!(benches);
