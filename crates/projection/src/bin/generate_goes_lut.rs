//! CLI tool to pre-compute GOES projection LUTs.
//!
//! Generates lookup tables for GOES-16 and GOES-18 CONUS sectors,
//! saving them to binary files that can be loaded at runtime.
//!
//! Usage:
//!   cargo run --release --bin generate-goes-lut -- --output ./data/luts
//!
//! This will create:
//!   - ./data/luts/goes16_conus_z0-7.lut
//!   - ./data/luts/goes18_conus_z0-7.lut

use projection::{compute_all_luts, Geostationary, ProjectionLutCache};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse arguments
    let mut output_dir = PathBuf::from("./data/luts");
    let mut max_zoom = 7u32;
    let mut satellites = vec!["goes16", "goes18"];

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                if i < args.len() {
                    output_dir = PathBuf::from(&args[i]);
                }
            }
            "--max-zoom" | "-z" => {
                i += 1;
                if i < args.len() {
                    max_zoom = args[i].parse().expect("Invalid max zoom");
                }
            }
            "--satellite" | "-s" => {
                i += 1;
                if i < args.len() {
                    satellites = args[i].split(',').collect();
                }
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Create output directory
    fs::create_dir_all(&output_dir).expect("Failed to create output directory");

    println!("GOES Projection LUT Generator");
    println!("==============================");
    println!("Output directory: {}", output_dir.display());
    println!("Max zoom level: {}", max_zoom);
    println!("Satellites: {:?}", satellites);
    println!();

    for satellite in satellites {
        generate_lut_for_satellite(satellite, max_zoom, &output_dir);
    }

    println!("\nDone! LUT files are ready for deployment.");
}

fn generate_lut_for_satellite(satellite: &str, max_zoom: u32, output_dir: &PathBuf) {
    println!("Generating LUT for {}...", satellite);

    let proj = match satellite {
        "goes16" => Geostationary::goes16_conus(),
        "goes18" => Geostationary::goes18_conus(),
        _ => {
            eprintln!("Unknown satellite: {}", satellite);
            return;
        }
    };

    // GOES CONUS grid dimensions
    let data_width = 5000;
    let data_height = 3000;

    let start = Instant::now();
    let mut last_print = Instant::now();

    let cache = compute_all_luts(
        satellite,
        &proj,
        max_zoom,
        data_width,
        data_height,
        Some(|z, x, y, total| {
            // Print progress every 100ms
            if last_print.elapsed().as_millis() > 100 {
                print!("\r  Computing tile {}/{}/{} (total: {})...", z, x, y, total);
                std::io::Write::flush(&mut std::io::stdout()).ok();
                last_print = Instant::now();
            }
        }),
    );

    let elapsed = start.elapsed();
    println!(
        "\r  Computed {} tiles in {:.2}s                    ",
        cache.len(),
        elapsed.as_secs_f64()
    );

    // Save to file
    let filename = format!("{}_conus_z0-{}.lut", satellite, max_zoom);
    let filepath = output_dir.join(&filename);

    println!("  Saving to {}...", filepath.display());
    let file = File::create(&filepath).expect("Failed to create output file");
    let mut writer = BufWriter::new(file);
    cache.save(&mut writer).expect("Failed to save LUT");

    let file_size = fs::metadata(&filepath).map(|m| m.len()).unwrap_or(0);
    println!(
        "  Saved {} ({:.2} MB)",
        filename,
        file_size as f64 / 1024.0 / 1024.0
    );

    // Print statistics
    print_cache_stats(&cache);
}

fn print_cache_stats(cache: &ProjectionLutCache) {
    println!("  Statistics:");
    println!("    Total tiles: {}", cache.len());
    println!(
        "    Memory usage: {:.2} MB",
        cache.memory_usage() as f64 / 1024.0 / 1024.0
    );
}

fn print_help() {
    println!(
        r#"GOES Projection LUT Generator

Generates pre-computed lookup tables for fast GOES satellite tile rendering.

USAGE:
    generate-goes-lut [OPTIONS]

OPTIONS:
    -o, --output <DIR>       Output directory for LUT files [default: ./data/luts]
    -z, --max-zoom <LEVEL>   Maximum zoom level to compute [default: 7]
    -s, --satellite <LIST>   Comma-separated list of satellites [default: goes16,goes18]
    -h, --help               Print this help message

EXAMPLES:
    # Generate LUTs for both satellites, zoom 0-7
    generate-goes-lut --output ./data/luts

    # Generate only GOES-16 LUT up to zoom 8
    generate-goes-lut -s goes16 -z 8 -o ./luts

OUTPUT:
    Creates binary LUT files that can be loaded at runtime:
    - <output>/goes16_conus_z0-7.lut
    - <output>/goes18_conus_z0-7.lut

    Each file is approximately 300-350 MB and contains pre-computed
    projection indices for all tiles within the satellite's coverage area.
"#
    );
}
