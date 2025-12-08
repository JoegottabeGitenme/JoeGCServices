//! Load test CLI for Weather WMS/WMTS service.

use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "load-test")]
#[command(about = "Load testing tool for Weather WMS/WMTS service", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compare two request log files to analyze cache behavior
    Compare {
        /// First request log file (e.g., from cold cache run)
        #[arg(short = '1', long)]
        first: PathBuf,

        /// Second request log file (e.g., from warm cache run)
        #[arg(short = '2', long)]
        second: PathBuf,
    },

    /// Run a load test from a scenario file
    Run {
        /// Path to scenario YAML file
        #[arg(short, long)]
        scenario: PathBuf,

        /// Override concurrency level
        #[arg(short, long)]
        concurrency: Option<u32>,

        /// Override test duration in seconds
        #[arg(short, long)]
        duration: Option<u64>,

        /// Output format: table (default), json, csv
        #[arg(short, long, default_value = "table")]
        output: String,

        /// Log all requests to a JSONL file for analysis/visualization
        #[arg(long)]
        log_requests: bool,
    },

    /// Run a quick smoke test
    Quick {
        /// Layer to test (e.g., gfs_TMP)
        #[arg(short, long, default_value = "gfs_TMP")]
        layer: String,

        /// Number of requests
        #[arg(short, long, default_value = "100")]
        requests: u64,

        /// Base URL
        #[arg(short, long, default_value = "http://localhost:8080")]
        url: String,
    },

    /// List available scenarios
    List {
        /// Scenarios directory
        #[arg(short, long, default_value = "scenarios")]
        dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compare { first, second } => {
            println!("Comparing request logs:");
            println!("  First:  {}", first.display());
            println!("  Second: {}", second.display());
            println!();

            // Parse first file
            let first_content = std::fs::read_to_string(&first)?;
            let first_requests: Vec<serde_json::Value> = first_content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();

            // Parse second file
            let second_content = std::fs::read_to_string(&second)?;
            let second_requests: Vec<serde_json::Value> = second_content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();

            // Extract unique tiles from each
            let first_tiles: HashSet<String> = first_requests
                .iter()
                .map(|r| format!("{}/{}/{}", r["z"], r["x"], r["y"]))
                .collect();

            let second_tiles: HashSet<String> = second_requests
                .iter()
                .map(|r| format!("{}/{}/{}", r["z"], r["x"], r["y"]))
                .collect();

            // Analyze
            let common: HashSet<_> = first_tiles.intersection(&second_tiles).collect();
            let only_first: HashSet<_> = first_tiles.difference(&second_tiles).collect();
            let only_second: HashSet<_> = second_tiles.difference(&first_tiles).collect();

            // Cache stats for second run
            let second_hits = second_requests
                .iter()
                .filter(|r| r["cache_status"].as_str() == Some("HIT"))
                .count();
            let second_misses = second_requests.len() - second_hits;

            println!("Request Counts:");
            println!("  First run:  {} requests", first_requests.len());
            println!("  Second run: {} requests", second_requests.len());
            println!();

            println!("Unique Tiles:");
            println!("  First run:  {} unique tiles", first_tiles.len());
            println!("  Second run: {} unique tiles", second_tiles.len());
            println!();

            println!("Tile Overlap Analysis:");
            println!("  Common tiles:        {} ({:.1}% of first run)", 
                common.len(), 
                (common.len() as f64 / first_tiles.len() as f64) * 100.0);
            println!("  Only in first run:   {}", only_first.len());
            println!("  Only in second run:  {} (NEW tiles causing cache misses)", only_second.len());
            println!();

            println!("Second Run Cache Performance:");
            println!("  Cache hits:   {} ({:.1}%)", 
                second_hits, 
                (second_hits as f64 / second_requests.len() as f64) * 100.0);
            println!("  Cache misses: {} ({:.1}%)", 
                second_misses,
                (second_misses as f64 / second_requests.len() as f64) * 100.0);
            println!();

            // Expected vs actual
            let expected_hits = common.len();
            let expected_misses = only_second.len();
            println!("Expected (if all common tiles were cached):");
            println!("  Expected hits from common tiles: ~{}", expected_hits);
            println!("  Expected misses from new tiles:  ~{}", expected_misses);
            println!();

            if !only_second.is_empty() {
                println!("Why aren't you seeing 100% cache hits?");
                println!("  The second run made {} more requests than the first run,", 
                    second_requests.len() as i64 - first_requests.len() as i64);
                println!("  accessing {} tiles that were never requested in the first run.", 
                    only_second.len());
                println!();
                println!("Suggestions:");
                println!("  1. Use a fixed number of requests instead of duration-based testing");
                println!("  2. Constrain the tile space with a bbox in your scenario");
                println!("  3. Use a smaller zoom range to reduce the tile space");
                println!("  4. Use TileSelection::Fixed with specific tiles for exact reproducibility");
            }

            Ok(())
        }
        Commands::Run {
            scenario,
            concurrency,
            duration,
            output,
            log_requests,
        } => {
            println!("Loading scenario: {}", scenario.display());
            
            // Load and validate configuration
            let mut config = load_test::TestConfig::from_file(&scenario)?;
            
            // Apply overrides
            if let Some(c) = concurrency {
                config.concurrency = c;
            }
            if let Some(d) = duration {
                config.duration_secs = d;
            }
            if log_requests {
                config.log_requests = true;
            }
            
            config.validate()?;
            
            println!("✓ Configuration loaded successfully");
            println!("  Name: {}", config.name);
            println!("  Description: {}", config.description);
            println!("  Duration: {}s", config.duration_secs);
            println!("  Concurrency: {}", config.concurrency);
            println!("  Layers: {}", config.layers.len());
            println!();
            
            // Run the load test
            let mut runner = load_test::LoadRunner::new(config);
            let results = runner.run().await?;
            
            // Output results
            match output.as_str() {
                "json" => {
                    println!("{}", load_test::ResultsReport::format_json(&results)?);
                }
                "csv" => {
                    println!("{}", load_test::ResultsReport::csv_header());
                    println!("{}", load_test::ResultsReport::format_csv(&results));
                }
                _ => {
                    println!("{}", load_test::ResultsReport::format_table(&results));
                }
            }
            
            Ok(())
        }
        Commands::Quick { layer, requests, url } => {
            println!("Running quick test:");
            println!("  Layer: {}", layer);
            println!("  Requests: {}", requests);
            println!("  URL: {}", url);
            println!();
            
            // Calculate duration based on requests (rough estimate: 10 req/s)
            let estimated_duration = (requests as f64 / 10.0).max(5.0) as u64;
            
            // Create a simple config
            let config = load_test::TestConfig {
                name: "quick".to_string(),
                description: "Quick smoke test".to_string(),
                base_url: url,
                duration_secs: estimated_duration,
                concurrency: 5,
                requests_per_second: Some(requests as f64 / estimated_duration as f64),
                warmup_secs: 0,
                layers: vec![load_test::LayerConfig {
                    name: layer.clone(),
                    style: Some("default".to_string()),
                    weight: 1.0,
                }],
                tile_selection: load_test::TileSelection::Random {
                    zoom_range: (4, 6),
                    bbox: None,
                },
                seed: None,
                time_selection: None,
                log_requests: false,
            };
            
            // Run the load test
            let mut runner = load_test::LoadRunner::new(config);
            let results = runner.run().await?;
            
            // Display results as table
            println!("{}", load_test::ResultsReport::format_table(&results));
            
            Ok(())
        }
        Commands::List { dir } => {
            println!("Available scenarios in {}:", dir.display());
            println!();
            
            // Read directory
            match std::fs::read_dir(&dir) {
                Ok(entries) => {
                    let mut scenarios = Vec::new();
                    
                    for entry in entries {
                        if let Ok(entry) = entry {
                            let path = entry.path();
                            if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
                                // Try to load the config to get name and description
                                if let Ok(config) = load_test::TestConfig::from_file(&path) {
                                    scenarios.push((
                                        path.file_name().unwrap().to_string_lossy().to_string(),
                                        config.name,
                                        config.description,
                                    ));
                                }
                            }
                        }
                    }
                    
                    scenarios.sort_by(|a, b| a.0.cmp(&b.0));
                    
                    if scenarios.is_empty() {
                        println!("No scenario files found");
                    } else {
                        for (filename, name, desc) in scenarios {
                            println!("  {} - {}", filename, name);
                            println!("    {}", desc);
                            println!();
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error reading directory: {}", e);
                    eprintln!("Make sure the directory exists and is readable");
                }
            }
            
            Ok(())
        }
    }
}
