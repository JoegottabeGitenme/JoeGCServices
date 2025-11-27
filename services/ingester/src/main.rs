//! Weather data ingester service.
//!
//! Polls NOAA data sources (NOMADS, AWS Open Data) and ingests
//! GRIB2/NetCDF files into object storage with catalog updates.

mod config;
mod ingest;
mod sources;

use anyhow::Result;
use clap::Parser;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use config::IngesterConfig;
use ingest::IngestionPipeline;
use std::fs;
use std::collections::HashSet;
use bytes::Bytes;
use chrono::Utc;

#[derive(Parser, Debug)]
#[command(name = "ingester")]
#[command(about = "Weather data ingester for WMS services")]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/ingester/config.yaml")]
    config: String,

    /// Run once and exit (vs continuous polling)
    #[arg(long)]
    once: bool,

    /// Specific model to ingest (default: all configured)
    #[arg(short, long)]
    model: Option<String>,

    /// Test with local GRIB2 file
    #[arg(long)]
    test_file: Option<String>,

    /// Forecast hour for test file (overrides GRIB metadata)
    #[arg(long)]
    forecast_hour: Option<u32>,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(true)
        .with_thread_ids(true)
        .json()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting weather data ingester");

    // Load configuration
    let config = IngesterConfig::from_env()?;
    info!(models = ?config.models.keys().collect::<Vec<_>>(), "Loaded configuration");

    // Handle test file mode
    if let Some(test_file) = &args.test_file {
        return test_file_ingestion(&config, test_file, args.forecast_hour).await;
    }

    // Create ingestion pipeline
    let pipeline = IngestionPipeline::new(&config).await?;

    if args.once {
        // Single run mode
        info!("Running single ingestion cycle");

        if let Some(model) = &args.model {
            pipeline.ingest_model(model).await?;
        } else {
            pipeline.ingest_all().await?;
        }
    } else {
        // Continuous polling mode
        info!("Starting continuous polling");
        pipeline.run_forever().await?;
    }

    Ok(())
}

/// Ingest a local GRIB2 test file with shredding (extract individual parameters)
async fn test_file_ingestion(config: &IngesterConfig, test_file: &str, forecast_hour_override: Option<u32>) -> Result<()> {
    use storage::{Catalog, CatalogEntry, ObjectStorage};
    use wms_common::BoundingBox;

    info!(file = %test_file, forecast_hour = ?forecast_hour_override, "Testing ingestion with local file (shredded mode)");

    // Read file
    let data = fs::read(test_file)?;
    let data_bytes = Bytes::from(data);
    let original_file_size = data_bytes.len() as u64;

    // Setup storage and catalog
    let storage = ObjectStorage::new(&config.storage)?;
    let catalog = Catalog::connect(&config.database_url).await?;
    catalog.migrate().await?;

    // Determine forecast hour (use override or extract from filename or default to 0)
    let forecast_hour = forecast_hour_override.or_else(|| {
        // Try to extract from filename like "gfs_f003.grib2" -> 3
        std::path::Path::new(test_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("gfs_f"))
            .and_then(|s| s.parse::<u32>().ok())
    }).unwrap_or(0);

    info!(forecast_hour = forecast_hour, original_size = original_file_size, "Using forecast hour");

    // Parse GRIB2
    let mut reader = grib2_parser::Grib2Reader::new(data_bytes);
    let mut message_count = 0;
    let mut registered_params = HashSet::new();
    let mut grib_reference_time: Option<chrono::DateTime<Utc>> = None;
    let mut total_shredded_size: u64 = 0;

    // Parameters to ingest with their accepted level types
    // GRIB2 Code Table 4.5 level type codes:
    // - 1 = Ground/water surface
    // - 100 = Isobaric surface (pressure level in Pa, we convert to mb)
    // - 101 = Mean sea level
    // - 103 = Specified height above ground (value in meters)
    // - 104 = Sigma level
    // - 106 = Depth below land surface
    
    // Standard pressure levels to ingest (in mb)
    let pressure_levels: HashSet<u32> = [
        1000, 975, 950, 925, 900, 850, 800, 750, 700, 650, 
        600, 550, 500, 450, 400, 350, 300, 250, 200, 150, 
        100, 70, 50, 30, 20, 10
    ].into_iter().collect();
    
    // Parameters and the level types we accept for each
    // Format: (param, Vec<(level_type, optional_specific_value)>)
    let target_params: Vec<(&str, Vec<(u8, Option<u32>)>)> = vec![
        ("PRMSL", vec![(101, None)]),                    // Mean sea level pressure only
        ("TMP", vec![
            (103, Some(2)),                              // 2m above ground
            (100, None),                                 // All pressure levels
        ]),
        ("UGRD", vec![
            (103, Some(10)),                             // 10m above ground
            (100, None),                                 // All pressure levels
        ]),
        ("VGRD", vec![
            (103, Some(10)),                             // 10m above ground
            (100, None),                                 // All pressure levels
        ]),
        ("RH", vec![
            (103, Some(2)),                              // 2m above ground
            (100, None),                                 // All pressure levels
        ]),
        ("HGT", vec![
            (100, None),                                 // All pressure levels (geopotential height)
        ]),
        ("GUST", vec![(1, None)]),                       // Surface wind gust
    ];

    while let Some(message) = reader.next_message().ok().flatten() {
        message_count += 1;

        // Extract reference time from the first message (it's the same for all messages in a file)
        if grib_reference_time.is_none() {
            grib_reference_time = Some(message.identification.reference_time);
            info!(reference_time = %message.identification.reference_time, "Extracted reference time from GRIB2");
        }

        let param = &message.product_definition.parameter_short_name;
        let level = &message.product_definition.level_description;
        let level_type = message.product_definition.level_type;
        let level_value = message.product_definition.level_value;
        
        // Create a unique key for param+level combination
        let param_level_key = format!("{}:{}", param, level);

        // Check if this matches one of our target parameter:level combinations
        let should_register = target_params.iter().any(|(p, level_specs)| {
            if param != p || registered_params.contains(&param_level_key) {
                return false;
            }
            
            // Check if any of the level specs match
            level_specs.iter().any(|(lt, lv)| {
                if level_type != *lt {
                    return false;
                }
                
                // For isobaric levels (type 100), check if it's in our target pressure levels
                // GRIB2 stores pressure in Pa, level_value is in 100*mb (so 850mb = 85000 Pa / 100 = 850)
                if level_type == 100 {
                    return pressure_levels.contains(&level_value);
                }
                
                // For other level types, check specific value if required
                if let Some(required_value) = lv {
                    level_value == *required_value
                } else {
                    true
                }
            })
        });

        if should_register {
            // Use reference time from GRIB2 file, fallback to now if not available
            let reference_time = grib_reference_time.unwrap_or_else(Utc::now);
            
            // Create a sanitized level string for the path (replace spaces and special chars)
            let level_sanitized = level
                .replace(' ', "_")
                .replace('/', "_")
                .to_lowercase();
            
            // New shredded storage path structure:
            // shredded/{model}/{run_date}/{param}_{level}/f{fhr:03}.grib2
            let run_date = reference_time.format("%Y%m%d_%Hz").to_string();
            let storage_path = format!(
                "shredded/gfs/{}/{}_{}/f{:03}.grib2",
                run_date,
                param.to_lowercase(),
                level_sanitized,
                forecast_hour
            );
            
            // Extract just this message's raw data and store it
            let shredded_data = message.raw_data.clone();
            let shredded_size = shredded_data.len() as u64;
            total_shredded_size += shredded_size;
            
            // Store the shredded (individual parameter) GRIB2 file
            storage.put(&storage_path, shredded_data).await?;
            
            info!(
                message = message_count, 
                param = %param, 
                level = %level, 
                path = %storage_path,
                size = shredded_size,
                "Stored shredded GRIB message"
            );
            
            let entry = CatalogEntry {
                model: "gfs".to_string(),
                parameter: param.clone(),
                level: level.clone(),
                reference_time,
                forecast_hour,
                bbox: BoundingBox::new(0.0, -90.0, 360.0, 90.0),
                storage_path,
                file_size: shredded_size,
            };

                match catalog.register_dataset(&entry).await {
                Ok(id) => {
                    info!(id = %id, param = %param, level = %level, "Registered dataset");
                    registered_params.insert(param_level_key.clone());
                }
                Err(e) => {
                    info!(param = %param, level = %level, error = %e, "Could not register (may already exist)");
                }
            }
        }
    }

    info!(
        messages = message_count,
        datasets = registered_params.len(),
        "Test file ingestion completed"
    );

    Ok(())
}
