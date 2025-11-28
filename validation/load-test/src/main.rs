//! Load test CLI for Weather WMS/WMTS service.

use clap::{Parser, Subcommand};
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
        Commands::Run {
            scenario,
            concurrency,
            duration,
            output,
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
