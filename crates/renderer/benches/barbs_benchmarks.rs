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

// =============================================================================
// TILE EXPANSION COMPARISON: 3x3 Tiles vs Pixel Buffer
// =============================================================================

/// Simulate 3x3 tile expansion approach (current implementation)
/// Renders 768x768 and crops to center 256x256
fn render_barbs_3x3_expansion(
    u_data_768: &[f32],
    v_data_768: &[f32],
    bbox_768: [f32; 4],
    config: &BarbConfig,
) -> Vec<u8> {
    // Render at 768x768 (3x3 tiles)
    let expanded_pixels = render_wind_barbs_aligned(
        u_data_768,
        v_data_768,
        768,
        768,
        bbox_768,
        config,
    );
    
    // Crop center 256x256 tile
    crop_center_tile(&expanded_pixels, 768, 256)
}

/// Simulate pixel buffer approach (proposed implementation)
/// Renders 376x376 (256 + 60px buffer on each side) and crops to center 256x256
fn render_barbs_pixel_buffer(
    u_data_376: &[f32],
    v_data_376: &[f32],
    bbox_376: [f32; 4],
    config: &BarbConfig,
) -> Vec<u8> {
    // Render at 376x376 (256 + 60*2 buffer)
    let buffered_pixels = render_wind_barbs_aligned(
        u_data_376,
        v_data_376,
        376,
        376,
        bbox_376,
        config,
    );
    
    // Crop center 256x256 tile (60px offset)
    crop_center_with_buffer(&buffered_pixels, 376, 256, 60)
}

/// Crop center tile from 3x3 expanded render (256px offset in 768px image)
fn crop_center_tile(expanded_pixels: &[u8], expanded_width: usize, tile_size: usize) -> Vec<u8> {
    let offset = (expanded_width - tile_size) / 2; // 256 for 768->256
    let mut result = vec![0u8; tile_size * tile_size * 4];
    
    for row in 0..tile_size {
        let src_y = offset + row;
        let src_start = (src_y * expanded_width + offset) * 4;
        let dst_start = row * tile_size * 4;
        result[dst_start..dst_start + tile_size * 4]
            .copy_from_slice(&expanded_pixels[src_start..src_start + tile_size * 4]);
    }
    
    result
}

/// Crop center tile from buffer-expanded render
fn crop_center_with_buffer(
    buffered_pixels: &[u8],
    buffered_width: usize,
    tile_size: usize,
    buffer: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; tile_size * tile_size * 4];
    
    for row in 0..tile_size {
        let src_y = buffer + row;
        let src_start = (src_y * buffered_width + buffer) * 4;
        let dst_start = row * tile_size * 4;
        result[dst_start..dst_start + tile_size * 4]
            .copy_from_slice(&buffered_pixels[src_start..src_start + tile_size * 4]);
    }
    
    result
}

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

// =============================================================================
// TILE EXPANSION COMPARISON BENCHMARKS
// =============================================================================

fn bench_tile_expansion_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("tile_expansion_comparison");
    group.sample_size(20);

    // Generate data for different render sizes
    // 3x3 approach: 768x768 pixels
    let (u_768, v_768) = generate_uniform_wind(768, 768, 10.0, 10.0);
    // Buffer approach: 376x376 pixels (256 + 60*2)
    let (u_376, v_376) = generate_uniform_wind(376, 376, 10.0, 10.0);
    // No expansion: 256x256 pixels (for baseline)
    let (u_256, v_256) = generate_uniform_wind(256, 256, 10.0, 10.0);

    // Sample bbox covering a typical tile (e.g., zoom 6 tile over CONUS)
    // Original tile bbox
    let bbox_256: [f32; 4] = [-100.0, 35.0, -94.375, 40.97];
    
    // 3x3 expanded bbox (3x the geographic area)
    let lon_span = bbox_256[2] - bbox_256[0];
    let lat_span = bbox_256[3] - bbox_256[1];
    let bbox_768: [f32; 4] = [
        bbox_256[0] - lon_span,  // expand left by 1 tile
        bbox_256[1] - lat_span,  // expand down by 1 tile
        bbox_256[2] + lon_span,  // expand right by 1 tile
        bbox_256[3] + lat_span,  // expand up by 1 tile
    ];
    
    // Buffer expanded bbox (60px buffer = 60/256 * tile_span on each side)
    let buffer_ratio = 60.0 / 256.0;
    let lon_buffer = lon_span * buffer_ratio;
    let lat_buffer = lat_span * buffer_ratio;
    let bbox_376: [f32; 4] = [
        bbox_256[0] - lon_buffer,
        bbox_256[1] - lat_buffer,
        bbox_256[2] + lon_buffer,
        bbox_256[3] + lat_buffer,
    ];

    let config = BarbConfig {
        size: 108,  // Default barb size
        spacing: 30,
        color: "#000000".to_string(),
    };

    // Benchmark 1: No expansion (baseline, has edge artifacts)
    group.throughput(Throughput::Elements(1));
    group.bench_function("no_expansion_256", |b| {
        b.iter(|| {
            render_wind_barbs_aligned(
                black_box(&u_256),
                black_box(&v_256),
                256,
                256,
                black_box(bbox_256),
                black_box(&config),
            )
        });
    });

    // Benchmark 2: 3x3 tile expansion (current approach)
    group.bench_function("3x3_expansion_768", |b| {
        b.iter(|| {
            render_barbs_3x3_expansion(
                black_box(&u_768),
                black_box(&v_768),
                black_box(bbox_768),
                black_box(&config),
            )
        });
    });

    // Benchmark 3: Pixel buffer approach (60px buffer)
    group.bench_function("pixel_buffer_376", |b| {
        b.iter(|| {
            render_barbs_pixel_buffer(
                black_box(&u_376),
                black_box(&v_376),
                black_box(bbox_376),
                black_box(&config),
            )
        });
    });

    // Benchmark 4: Just the cropping overhead
    let expanded_pixels_768 = render_wind_barbs_aligned(&u_768, &v_768, 768, 768, bbox_768, &config);
    let buffered_pixels_376 = render_wind_barbs_aligned(&u_376, &v_376, 376, 376, bbox_376, &config);

    group.bench_function("crop_3x3_768_to_256", |b| {
        b.iter(|| crop_center_tile(black_box(&expanded_pixels_768), 768, 256));
    });

    group.bench_function("crop_buffer_376_to_256", |b| {
        b.iter(|| crop_center_with_buffer(black_box(&buffered_pixels_376), 376, 256, 60));
    });

    group.finish();
}

/// Benchmark the full pipeline including PNG encoding for both approaches
fn bench_tile_expansion_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("tile_expansion_full_pipeline");
    group.sample_size(20);

    // Generate data
    let (u_768, v_768) = generate_uniform_wind(768, 768, 10.0, 10.0);
    let (u_376, v_376) = generate_uniform_wind(376, 376, 10.0, 10.0);
    let (u_256, v_256) = generate_uniform_wind(256, 256, 10.0, 10.0);

    let bbox_256: [f32; 4] = [-100.0, 35.0, -94.375, 40.97];
    let lon_span = bbox_256[2] - bbox_256[0];
    let lat_span = bbox_256[3] - bbox_256[1];
    let bbox_768: [f32; 4] = [
        bbox_256[0] - lon_span,
        bbox_256[1] - lat_span,
        bbox_256[2] + lon_span,
        bbox_256[3] + lat_span,
    ];
    let buffer_ratio = 60.0 / 256.0;
    let lon_buffer = lon_span * buffer_ratio;
    let lat_buffer = lat_span * buffer_ratio;
    let bbox_376: [f32; 4] = [
        bbox_256[0] - lon_buffer,
        bbox_256[1] - lat_buffer,
        bbox_256[2] + lon_buffer,
        bbox_256[3] + lat_buffer,
    ];

    let config = BarbConfig {
        size: 108,
        spacing: 30,
        color: "#000000".to_string(),
    };

    // Full pipeline: render + crop + PNG encode
    group.bench_function("full_no_expansion", |b| {
        b.iter(|| {
            let pixels = render_wind_barbs_aligned(&u_256, &v_256, 256, 256, bbox_256, &config);
            png::create_png(&pixels, 256, 256)
        });
    });

    group.bench_function("full_3x3_expansion", |b| {
        b.iter(|| {
            let pixels = render_barbs_3x3_expansion(&u_768, &v_768, bbox_768, &config);
            png::create_png(&pixels, 256, 256)
        });
    });

    group.bench_function("full_pixel_buffer_60px", |b| {
        b.iter(|| {
            let pixels = render_barbs_pixel_buffer(&u_376, &v_376, bbox_376, &config);
            png::create_png(&pixels, 256, 256)
        });
    });

    group.finish();
}

/// Compare different buffer sizes to find optimal balance
fn bench_buffer_size_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("buffer_size_comparison");
    group.sample_size(20);

    let bbox_256: [f32; 4] = [-100.0, 35.0, -94.375, 40.97];
    let lon_span = bbox_256[2] - bbox_256[0];
    let lat_span = bbox_256[3] - bbox_256[1];

    let config = BarbConfig {
        size: 108,
        spacing: 30,
        color: "#000000".to_string(),
    };

    // Test various buffer sizes: 30, 45, 60, 80, 100, 120
    for buffer_px in [30, 45, 60, 80, 100, 120] {
        let render_size = 256 + 2 * buffer_px;
        let (u_data, v_data) = generate_uniform_wind(render_size, render_size, 10.0, 10.0);
        
        let buffer_ratio = buffer_px as f32 / 256.0;
        let lon_buffer = lon_span * buffer_ratio;
        let lat_buffer = lat_span * buffer_ratio;
        let bbox_expanded: [f32; 4] = [
            bbox_256[0] - lon_buffer,
            bbox_256[1] - lat_buffer,
            bbox_256[2] + lon_buffer,
            bbox_256[3] + lat_buffer,
        ];

        group.bench_with_input(
            BenchmarkId::new("buffer_px", buffer_px),
            &(render_size, u_data, v_data, bbox_expanded, buffer_px),
            |b, (size, u, v, bbox, buf)| {
                b.iter(|| {
                    let pixels = render_wind_barbs_aligned(u, v, *size, *size, *bbox, &config);
                    crop_center_with_buffer(&pixels, *size, 256, *buf)
                });
            },
        );
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
    bench_tile_expansion_comparison,
    bench_tile_expansion_full_pipeline,
    bench_buffer_size_comparison,
);
criterion_main!(benches);
