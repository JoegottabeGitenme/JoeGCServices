//! GOES satellite rendering pipeline benchmarks.
//!
//! This benchmark suite tests the GOES rendering pipeline, specifically:
//! 1. Temp file I/O overhead (the primary bottleneck!)
//! 2. Geostationary projection coordinate transforms
//! 3. Grid resampling from GOES to Mercator tiles
//! 4. Color mapping for IR and visible bands
//! 5. Full pipeline simulation
//!
//! Run with: cargo bench --package renderer --bench goes_benchmarks
//!
//! For flamegraph profiling:
//! CARGO_PROFILE_BENCH_DEBUG=true cargo flamegraph --bench goes_benchmarks -- --bench

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use projection::geostationary::Geostationary;
use projection::{compute_tile_lut, resample_with_lut};
use rand::Rng;
use renderer::png;
use std::io::{Read, Write};

// =============================================================================
// TEMP FILE I/O BENCHMARKS (PRIMARY BOTTLENECK)
// =============================================================================

/// Simulate the temp file write/read/delete cycle that happens in NetCDF parsing.
/// This is the PRIMARY bottleneck identified in GOES_RENDERING_PERFORMANCE_ANALYSIS.md
fn bench_temp_file_io(c: &mut Criterion) {
    let mut group = c.benchmark_group("temp_file_io");
    
    // GOES CONUS NetCDF files are typically 2.5-3.0 MB
    let file_sizes = [
        (1_000_000, "1MB"),
        (2_800_000, "2.8MB_typical"),
        (5_000_000, "5MB"),
        (10_000_000, "10MB"),
    ];
    
    // Generate random data to simulate NetCDF file contents
    let mut rng = rand::thread_rng();
    
    for (size, name) in file_sizes {
        let data: Vec<u8> = (0..size).map(|_| rng.gen()).collect();
        
        group.throughput(Throughput::Bytes(size as u64));
        
        // Benchmark 1: Write to system temp dir (current behavior)
        group.bench_with_input(
            BenchmarkId::new("system_temp_write_read_delete", name),
            &data,
            |b, data| {
                b.iter(|| {
                    let temp_dir = std::env::temp_dir();
                    let temp_file = temp_dir.join(format!("bench_goes_{}.nc", std::process::id()));
                    
                    // Write (simulates netcdf temp file creation)
                    let mut file = std::fs::File::create(&temp_file).unwrap();
                    file.write_all(black_box(data)).unwrap();
                    drop(file);
                    
                    // Read back (simulates netcdf library opening file)
                    let mut file = std::fs::File::open(&temp_file).unwrap();
                    let mut buf = Vec::with_capacity(data.len());
                    file.read_to_end(&mut buf).unwrap();
                    drop(file);
                    
                    // Delete (cleanup)
                    std::fs::remove_file(&temp_file).unwrap();
                    
                    black_box(buf.len())
                });
            },
        );
        
        // Benchmark 2: Write only (to measure write overhead)
        group.bench_with_input(
            BenchmarkId::new("system_temp_write_only", name),
            &data,
            |b, data| {
                b.iter(|| {
                    let temp_dir = std::env::temp_dir();
                    let temp_file = temp_dir.join(format!("bench_goes_{}.nc", std::process::id()));
                    
                    let mut file = std::fs::File::create(&temp_file).unwrap();
                    file.write_all(black_box(data)).unwrap();
                    drop(file);
                    
                    std::fs::remove_file(&temp_file).unwrap();
                    
                    black_box(data.len())
                });
            },
        );
        
        // Benchmark 3: /dev/shm if available (Linux memory-backed filesystem)
        #[cfg(target_os = "linux")]
        {
            let shm_path = Path::new("/dev/shm");
            if shm_path.exists() {
                group.bench_with_input(
                    BenchmarkId::new("dev_shm_write_read_delete", name),
                    &data,
                    |b, data| {
                        b.iter(|| {
                            let temp_file = shm_path.join(format!("bench_goes_{}.nc", std::process::id()));
                            
                            // Write to memory-backed filesystem
                            let mut file = std::fs::File::create(&temp_file).unwrap();
                            file.write_all(black_box(data)).unwrap();
                            drop(file);
                            
                            // Read back
                            let mut file = std::fs::File::open(&temp_file).unwrap();
                            let mut buf = Vec::with_capacity(data.len());
                            file.read_to_end(&mut buf).unwrap();
                            drop(file);
                            
                            // Delete
                            std::fs::remove_file(&temp_file).unwrap();
                            
                            black_box(buf.len())
                        });
                    },
                );
            }
        }
        
        // Benchmark 4: Memory-only baseline (no file I/O - theoretical minimum)
        group.bench_with_input(
            BenchmarkId::new("memory_copy_baseline", name),
            &data,
            |b, data| {
                b.iter(|| {
                    // Just copy data in memory (simulates what we'd do without temp files)
                    let buf: Vec<u8> = black_box(data).to_vec();
                    black_box(buf.len())
                });
            },
        );
        
        // Benchmark 5: Optimized path (uses /dev/shm on Linux, system temp elsewhere)
        // This simulates the actual code path after optimization
        group.bench_with_input(
            BenchmarkId::new("optimized_temp_path", name),
            &data,
            |b, data| {
                b.iter(|| {
                    // Use the same logic as get_optimal_temp_dir()
                    let temp_dir = {
                        #[cfg(target_os = "linux")]
                        {
                            use std::path::Path;
                            let shm = Path::new("/dev/shm");
                            if shm.exists() && shm.is_dir() {
                                shm.to_path_buf()
                            } else {
                                std::env::temp_dir()
                            }
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            std::env::temp_dir()
                        }
                    };
                    
                    // Generate unique filename like the optimized code
                    use std::sync::atomic::{AtomicU64, Ordering};
                    static COUNTER: AtomicU64 = AtomicU64::new(0);
                    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
                    let temp_file = temp_dir.join(format!("bench_opt_{}_{}.nc", std::process::id(), count));
                    
                    // Write
                    let mut file = std::fs::File::create(&temp_file).unwrap();
                    file.write_all(black_box(data)).unwrap();
                    drop(file);
                    
                    // Read back
                    let mut file = std::fs::File::open(&temp_file).unwrap();
                    let mut buf = Vec::with_capacity(data.len());
                    file.read_to_end(&mut buf).unwrap();
                    drop(file);
                    
                    // Delete
                    std::fs::remove_file(&temp_file).unwrap();
                    
                    black_box(buf.len())
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark to simulate the full NetCDF parsing pattern with sync points
fn bench_netcdf_io_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("netcdf_io_pattern");
    
    // Typical GOES file size
    let size = 2_800_000;
    let mut rng = rand::thread_rng();
    let data: Vec<u8> = (0..size).map(|_| rng.gen()).collect();
    
    group.throughput(Throughput::Bytes(size as u64));
    
    // Pattern 1: Current implementation (write, sync, read, delete)
    group.bench_function("current_pattern_with_sync", |b| {
        b.iter(|| {
            let temp_dir = std::env::temp_dir();
            let temp_file = temp_dir.join(format!("bench_goes_{}.nc", std::process::id()));
            
            // Write with explicit sync (ensures data hits disk)
            let mut file = std::fs::File::create(&temp_file).unwrap();
            file.write_all(black_box(&data)).unwrap();
            file.sync_all().unwrap(); // Force flush to disk
            drop(file);
            
            // Read back
            let mut file = std::fs::File::open(&temp_file).unwrap();
            let mut buf = Vec::with_capacity(data.len());
            file.read_to_end(&mut buf).unwrap();
            drop(file);
            
            // Delete
            std::fs::remove_file(&temp_file).unwrap();
            
            black_box(buf.len())
        });
    });
    
    // Pattern 2: Without explicit sync (OS may buffer)
    group.bench_function("no_sync_pattern", |b| {
        b.iter(|| {
            let temp_dir = std::env::temp_dir();
            let temp_file = temp_dir.join(format!("bench_goes_{}.nc", std::process::id()));
            
            // Write without sync
            let mut file = std::fs::File::create(&temp_file).unwrap();
            file.write_all(black_box(&data)).unwrap();
            drop(file);
            
            // Read back
            let mut file = std::fs::File::open(&temp_file).unwrap();
            let mut buf = Vec::with_capacity(data.len());
            file.read_to_end(&mut buf).unwrap();
            drop(file);
            
            // Delete
            std::fs::remove_file(&temp_file).unwrap();
            
            black_box(buf.len())
        });
    });
    
    // Pattern 3: Multiple sequential operations (simulates concurrent requests)
    group.bench_function("sequential_3x_operations", |b| {
        b.iter(|| {
            let temp_dir = std::env::temp_dir();
            let mut total_bytes = 0;
            
            for i in 0..3 {
                let temp_file = temp_dir.join(format!("bench_goes_{}_{}.nc", std::process::id(), i));
                
                let mut file = std::fs::File::create(&temp_file).unwrap();
                file.write_all(black_box(&data)).unwrap();
                drop(file);
                
                let mut file = std::fs::File::open(&temp_file).unwrap();
                let mut buf = Vec::with_capacity(data.len());
                file.read_to_end(&mut buf).unwrap();
                total_bytes += buf.len();
                drop(file);
                
                std::fs::remove_file(&temp_file).unwrap();
            }
            
            black_box(total_bytes)
        });
    });
    
    group.finish();
}

// =============================================================================
// TEST DATA GENERATORS
// =============================================================================

/// Generate synthetic GOES CONUS data (2500 x 1500 grid).
/// Values simulate brightness temperatures in Kelvin for IR bands.
fn generate_goes_ir_data(width: usize, height: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    let mut data = vec![0.0f32; width * height];

    for y in 0..height {
        for x in 0..width {
            // Simulate cloud-top temperatures (200K to 310K)
            // Colder at edges (higher clouds), warmer in center (ground)
            let dist_from_center = {
                let dx = (x as f32 / width as f32 - 0.5).abs();
                let dy = (y as f32 / height as f32 - 0.5).abs();
                (dx * dx + dy * dy).sqrt()
            };
            
            // Base temp varies with distance from center
            let base_temp = 310.0 - dist_from_center * 150.0;
            // Add noise
            let noise = rng.gen_range(-5.0..5.0);
            
            data[y * width + x] = (base_temp + noise).clamp(200.0, 320.0);
        }
    }
    data
}

/// Generate synthetic GOES visible band data (reflectance 0.0 to 1.2).
fn generate_goes_visible_data(width: usize, height: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    let mut data = vec![0.0f32; width * height];

    for y in 0..height {
        for x in 0..width {
            // Simulate reflectance patterns (clouds bright, ocean dark)
            let x_norm = x as f32 / width as f32;
            let y_norm = y as f32 / height as f32;
            
            // Some cloud-like patterns
            let cloud = ((x_norm * 10.0).sin() * (y_norm * 8.0).cos()).abs() * 0.4;
            let base = 0.3 + cloud;
            let noise = rng.gen_range(-0.05..0.05);
            
            data[y * width + x] = (base + noise).clamp(0.0, 1.2);
        }
    }
    data
}

/// Generate RGBA data for PNG encoding benchmarks.
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
// GEOSTATIONARY PROJECTION BENCHMARKS
// =============================================================================

fn bench_geo_to_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("goes_projection");
    
    let proj = Geostationary::goes16_conus();
    
    // Test different numbers of coordinate transforms
    let counts = [256 * 256, 512 * 512, 1024 * 1024];
    
    for count in counts {
        // Pre-generate test points within CONUS bounds
        let mut rng = rand::thread_rng();
        let test_points: Vec<(f64, f64)> = (0..count)
            .map(|_| {
                let lat = rng.gen_range(25.0..50.0);  // CONUS latitude range
                let lon = rng.gen_range(-125.0..-65.0); // CONUS longitude range
                (lat, lon)
            })
            .collect();
        
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("geo_to_grid", count),
            &test_points,
            |b, points| {
                b.iter(|| {
                    let mut sum = 0.0f64;
                    for &(lat, lon) in points {
                        if let Some((i, j)) = proj.geo_to_grid(black_box(lat), black_box(lon)) {
                            sum += i + j;
                        }
                    }
                    black_box(sum)
                });
            },
        );
    }
    
    group.finish();
}

fn bench_geo_to_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("goes_projection");
    
    let proj = Geostationary::goes16_conus();
    
    // Test the lower-level geo_to_scan which has the heavy trig
    let count = 256 * 256;
    
    let mut rng = rand::thread_rng();
    let test_points: Vec<(f64, f64)> = (0..count)
        .map(|_| {
            let lat = rng.gen_range(25.0..50.0);
            let lon = rng.gen_range(-125.0..-65.0);
            (lon, lat) // Note: geo_to_scan takes (lon, lat)
        })
        .collect();
    
    group.throughput(Throughput::Elements(count as u64));
    group.bench_with_input(
        BenchmarkId::new("geo_to_scan", count),
        &test_points,
        |b, points| {
            b.iter(|| {
                let mut sum = 0.0f64;
                for &(lon, lat) in points {
                    if let Some((x, y)) = proj.geo_to_scan(black_box(lon), black_box(lat)) {
                        sum += x + y;
                    }
                }
                black_box(sum)
            });
        },
    );
    
    group.finish();
}

// =============================================================================
// GOES RESAMPLING BENCHMARKS
// =============================================================================

/// Simulate GOES-to-Mercator resampling without the actual projection calls.
/// This isolates the bilinear interpolation overhead.
fn resample_bilinear_only(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
) -> Vec<f32> {
    let mut output = vec![f32::NAN; output_width * output_height];
    
    for out_y in 0..output_height {
        for out_x in 0..output_width {
            // Simple linear mapping (no projection)
            let grid_i = out_x as f64 * (data_width as f64 - 1.0) / output_width as f64;
            let grid_j = out_y as f64 * (data_height as f64 - 1.0) / output_height as f64;
            
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
            
            let v1 = v11 * (1.0 - di) + v21 * di;
            let v2 = v12 * (1.0 - di) + v22 * di;
            output[out_y * output_width + out_x] = v1 * (1.0 - dj) + v2 * dj;
        }
    }
    
    output
}

/// Full GOES-to-Mercator resampling with projection transforms.
fn resample_goes_to_mercator(
    data: &[f32],
    data_width: usize,
    data_height: usize,
    output_width: usize,
    output_height: usize,
    output_bbox: [f32; 4], // [min_lon, min_lat, max_lon, max_lat]
    proj: &Geostationary,
) -> Vec<f32> {
    let [out_min_lon, out_min_lat, out_max_lon, out_max_lat] = output_bbox;
    
    // Convert lat bounds to Mercator Y coordinates
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
            
            // This is the expensive part - projection transform
            let grid_coords = proj.geo_to_grid(lat, lon as f64);
            
            let (grid_i, grid_j) = match grid_coords {
                Some((i, j)) => (i, j),
                None => continue,
            };
            
            if grid_i < 0.0 || grid_i >= data_width as f64 - 1.0 ||
               grid_j < 0.0 || grid_j >= data_height as f64 - 1.0 {
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

// Mercator conversion helpers
fn lat_to_mercator_y(lat_deg: f64) -> f64 {
    let lat_rad = lat_deg.to_radians();
    lat_rad.tan().asinh()
}

fn mercator_y_to_lat(merc_y: f64) -> f64 {
    merc_y.sinh().atan().to_degrees()
}

fn bench_goes_resampling(c: &mut Criterion) {
    let mut group = c.benchmark_group("goes_resample");
    
    // GOES CONUS grid size (typical for 2km resolution)
    let goes_width = 2500;
    let goes_height = 1500;
    let goes_data = generate_goes_ir_data(goes_width, goes_height);
    let proj = Geostationary::goes16_conus();
    
    // Common tile sizes and zoom scenarios
    let scenarios = [
        (256, 256, [-125.0f32, 25.0, -65.0, 50.0], "full_conus_z4"),
        (256, 256, [-100.0f32, 35.0, -90.0, 45.0], "central_us_z7"),
        (256, 256, [-95.0f32, 38.0, -94.0, 39.0], "kansas_city_z10"),
        (512, 512, [-125.0f32, 25.0, -65.0, 50.0], "full_conus_z4_512"),
    ];
    
    for (out_w, out_h, bbox, name) in scenarios {
        group.throughput(Throughput::Elements((out_w * out_h) as u64));
        
        // Bilinear only (no projection)
        group.bench_with_input(
            BenchmarkId::new("bilinear_only", name),
            &goes_data,
            |b, data| {
                b.iter(|| {
                    resample_bilinear_only(
                        black_box(data),
                        goes_width,
                        goes_height,
                        out_w,
                        out_h,
                    )
                });
            },
        );
        
        // Full resampling with projection
        group.bench_with_input(
            BenchmarkId::new("with_projection", name),
            &(&goes_data, &proj, bbox),
            |b, (data, proj, bbox)| {
                b.iter(|| {
                    resample_goes_to_mercator(
                        black_box(data),
                        goes_width,
                        goes_height,
                        out_w,
                        out_h,
                        *bbox,
                        proj,
                    )
                });
            },
        );
    }
    
    group.finish();
}

// =============================================================================
// GOES COLOR MAPPING BENCHMARKS
// =============================================================================

/// Map GOES IR brightness temperature to enhanced IR colorscale.
/// This mimics the GOES IR rendering in the actual WMS service.
fn render_goes_ir(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    
    for (i, &temp) in data.iter().enumerate() {
        let px = i * 4;
        
        if temp.is_nan() {
            // Transparent for missing data
            pixels[px..px + 4].copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        
        // Enhanced IR colorscale: cold (high clouds) = white/cyan, warm (ground) = gray/black
        // Typical range: 200K to 310K
        let norm = ((temp - 200.0) / 110.0).clamp(0.0, 1.0);
        
        // Color mapping (simplified enhanced IR)
        let (r, g, b) = if norm < 0.2 {
            // Very cold (high clouds): white to cyan
            let t = norm / 0.2;
            (
                (255.0 * (1.0 - t * 0.3)) as u8,
                255,
                255,
            )
        } else if norm < 0.5 {
            // Cold: cyan to green
            let t = (norm - 0.2) / 0.3;
            (
                (180.0 * (1.0 - t)) as u8,
                255,
                (255.0 * (1.0 - t)) as u8,
            )
        } else if norm < 0.7 {
            // Mid: green to yellow
            let t = (norm - 0.5) / 0.2;
            (
                (255.0 * t) as u8,
                255,
                0,
            )
        } else if norm < 0.85 {
            // Warm: yellow to red
            let t = (norm - 0.7) / 0.15;
            (
                255,
                (255.0 * (1.0 - t)) as u8,
                0,
            )
        } else {
            // Very warm (ground): red to gray
            let t = (norm - 0.85) / 0.15;
            let gray = (255.0 * (1.0 - t) + 100.0 * t) as u8;
            (gray, (100.0 * t) as u8, (100.0 * t) as u8)
        };
        
        pixels[px] = r;
        pixels[px + 1] = g;
        pixels[px + 2] = b;
        pixels[px + 3] = 255;
    }
    
    pixels
}

/// Map GOES visible reflectance to grayscale.
fn render_goes_visible(data: &[f32], width: usize, height: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    
    for (i, &refl) in data.iter().enumerate() {
        let px = i * 4;
        
        if refl.is_nan() {
            pixels[px..px + 4].copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        
        // Reflectance to grayscale (0.0 = black, 1.0 = white)
        // Apply gamma correction for better visualization
        let gamma = 0.8;
        let norm = refl.clamp(0.0, 1.2) / 1.2;
        let corrected = norm.powf(gamma);
        let gray = (corrected * 255.0) as u8;
        
        pixels[px] = gray;
        pixels[px + 1] = gray;
        pixels[px + 2] = gray;
        pixels[px + 3] = 255;
    }
    
    pixels
}

fn bench_goes_color_mapping(c: &mut Criterion) {
    let mut group = c.benchmark_group("goes_color");
    
    let sizes = [(256, 256), (512, 512), (1024, 1024)];
    
    for (width, height) in sizes {
        let ir_data = generate_goes_ir_data(width, height);
        let vis_data = generate_goes_visible_data(width, height);
        
        group.throughput(Throughput::Elements((width * height) as u64));
        
        group.bench_with_input(
            BenchmarkId::new("ir_enhanced", format!("{}x{}", width, height)),
            &ir_data,
            |b, data| {
                b.iter(|| render_goes_ir(black_box(data), width, height));
            },
        );
        
        group.bench_with_input(
            BenchmarkId::new("visible_grayscale", format!("{}x{}", width, height)),
            &vis_data,
            |b, data| {
                b.iter(|| render_goes_visible(black_box(data), width, height));
            },
        );
    }
    
    group.finish();
}

// =============================================================================
// FULL PIPELINE BENCHMARKS
// =============================================================================

fn bench_goes_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("goes_pipeline");
    
    // Simulate full GOES rendering pipeline
    let goes_width = 2500;
    let goes_height = 1500;
    let goes_ir_data = generate_goes_ir_data(goes_width, goes_height);
    let goes_vis_data = generate_goes_visible_data(goes_width, goes_height);
    let proj = Geostationary::goes16_conus();
    
    let tile_width = 256;
    let tile_height = 256;
    
    // CONUS bbox for zoom ~6
    let bbox = [-105.0f32, 30.0, -85.0, 45.0];
    
    group.throughput(Throughput::Elements((tile_width * tile_height) as u64));
    
    // Full IR pipeline: resample -> color map -> PNG
    group.bench_function("ir_tile_256x256", |b| {
        b.iter(|| {
            // Step 1: Resample from GOES grid to tile
            let resampled = resample_goes_to_mercator(
                black_box(&goes_ir_data),
                goes_width,
                goes_height,
                tile_width,
                tile_height,
                bbox,
                &proj,
            );
            
            // Step 2: Apply IR colorscale
            let rgba = render_goes_ir(&resampled, tile_width, tile_height);
            
            // Step 3: Encode as PNG
            png::create_png(&rgba, tile_width, tile_height)
        });
    });
    
    // Full visible pipeline
    group.bench_function("visible_tile_256x256", |b| {
        b.iter(|| {
            let resampled = resample_goes_to_mercator(
                black_box(&goes_vis_data),
                goes_width,
                goes_height,
                tile_width,
                tile_height,
                bbox,
                &proj,
            );
            
            let rgba = render_goes_visible(&resampled, tile_width, tile_height);
            png::create_png(&rgba, tile_width, tile_height)
        });
    });
    
    // Compare: resampling only (no color/PNG)
    group.bench_function("resample_only_256x256", |b| {
        b.iter(|| {
            resample_goes_to_mercator(
                black_box(&goes_ir_data),
                goes_width,
                goes_height,
                tile_width,
                tile_height,
                bbox,
                &proj,
            )
        });
    });
    
    // Compare: color + PNG only (skip resampling)
    let pre_resampled = resample_goes_to_mercator(
        &goes_ir_data, goes_width, goes_height, tile_width, tile_height, bbox, &proj
    );
    group.bench_function("color_and_png_only_256x256", |b| {
        b.iter(|| {
            let rgba = render_goes_ir(black_box(&pre_resampled), tile_width, tile_height);
            png::create_png(&rgba, tile_width, tile_height)
        });
    });
    
    group.finish();
}

// =============================================================================
// PNG ENCODING BENCHMARKS (GOES-specific sizes)
// =============================================================================

fn bench_goes_png_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("goes_png");
    
    let sizes = [(256, 256), (512, 512)];
    
    for (width, height) in sizes {
        let rgba_data = generate_rgba_data(width, height);
        
        group.throughput(Throughput::Bytes((width * height * 4) as u64));
        
        group.bench_with_input(
            BenchmarkId::new("encode", format!("{}x{}", width, height)),
            &rgba_data,
            |b, data| {
                b.iter(|| png::create_png(black_box(data), width, height));
            },
        );
    }
    
    group.finish();
}

// =============================================================================
// PROJECTION LUT BENCHMARKS
// =============================================================================

/// Benchmark the projection LUT computation and lookup vs on-the-fly projection.
fn bench_projection_lut(c: &mut Criterion) {
    let mut group = c.benchmark_group("projection_lut");
    
    // GOES CONUS grid size (typical for 2km resolution)
    let goes_width = 2500;
    let goes_height = 1500;
    let goes_data = generate_goes_ir_data(goes_width, goes_height);
    let proj = Geostationary::goes16_conus();
    
    // Test a few representative tiles
    let tiles = [
        (5, 7, 11, "z5_central_conus"),
        (6, 14, 22, "z6_midwest"),
        (7, 28, 44, "z7_detailed"),
    ];
    
    for (z, x, y, name) in tiles {
        group.throughput(Throughput::Elements(256 * 256));
        
        // Pre-compute the LUT for this tile
        let lut = compute_tile_lut(&proj, z, x, y, goes_width, goes_height);
        let valid_count = lut.valid_count();
        
        // Calculate tile bbox for on-the-fly comparison
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
        
        // Benchmark: On-the-fly projection (current method)
        group.bench_with_input(
            BenchmarkId::new("on_the_fly", name),
            &(&goes_data, &proj, bbox),
            |b, (data, proj, bbox)| {
                b.iter(|| {
                    resample_goes_to_mercator(
                        black_box(data),
                        goes_width,
                        goes_height,
                        256,
                        256,
                        *bbox,
                        proj,
                    )
                });
            },
        );
        
        // Benchmark: LUT-based resampling (optimized method)
        group.bench_with_input(
            BenchmarkId::new("with_lut", name),
            &(&goes_data, &lut),
            |b, (data, lut)| {
                b.iter(|| {
                    resample_with_lut(black_box(data), goes_width, lut)
                });
            },
        );
        
        // Print info about this tile
        println!("Tile {}: {} valid pixels out of {}", name, valid_count, 256 * 256);
    }
    
    // Benchmark: LUT computation time (one-time cost)
    group.bench_function("compute_lut_z5", |b| {
        b.iter(|| {
            compute_tile_lut(black_box(&proj), 5, 7, 11, goes_width, goes_height)
        });
    });
    
    group.bench_function("compute_lut_z7", |b| {
        b.iter(|| {
            compute_tile_lut(black_box(&proj), 7, 28, 44, goes_width, goes_height)
        });
    });
    
    group.finish();
}

// =============================================================================
// BENCHMARK GROUPS
// =============================================================================

criterion_group!(
    io_benches,
    bench_temp_file_io,
    bench_netcdf_io_pattern,
);

criterion_group!(
    projection_benches,
    bench_geo_to_grid,
    bench_geo_to_scan,
);

criterion_group!(
    resample_benches,
    bench_goes_resampling,
);

criterion_group!(
    color_benches,
    bench_goes_color_mapping,
);

criterion_group!(
    lut_benches,
    bench_projection_lut,
);

criterion_group!(
    pipeline_benches,
    bench_goes_full_pipeline,
    bench_goes_png_encoding,
);

criterion_main!(io_benches, projection_benches, resample_benches, color_benches, lut_benches, pipeline_benches);
