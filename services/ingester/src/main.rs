//! Weather data ingester service.
//!
//! Polls NOAA data sources (NOMADS, AWS Open Data) and ingests
//! GRIB2/NetCDF files into object storage with catalog updates.

mod config;
mod config_loader;
mod ingest;
mod sources;

use anyhow::Result;
use clap::Parser;
use tracing::{info, warn, Level};
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
    
    /// Model name for test file (e.g., "gfs", "hrrr")
    #[arg(long)]
    test_model: Option<String>,

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

    // Load configuration (try YAML first, fall back to env vars)
    let config = if std::path::Path::new("config/ingestion.yaml").exists() {
        info!("Loading configuration from YAML files");
        match IngesterConfig::from_yaml(".") {
            Ok(cfg) => {
                info!(models = ?cfg.models.keys().collect::<Vec<_>>(), "Loaded configuration from YAML");
                cfg
            }
            Err(e) => {
                warn!(error = %e, "Failed to load YAML config, falling back to environment variables");
                IngesterConfig::from_env()?
            }
        }
    } else {
        info!("No YAML config found, loading from environment variables");
        IngesterConfig::from_env()?
    };
    
    info!(models = ?config.models.keys().collect::<Vec<_>>(), "Configuration loaded");

    // Handle test file mode
    if let Some(test_file) = &args.test_file {
        return test_file_ingestion(&config, test_file, args.forecast_hour, args.test_model.as_deref()).await;
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

/// Ingest a local GRIB2 or NetCDF test file with shredding (extract individual parameters)
async fn test_file_ingestion(
    config: &IngesterConfig, 
    test_file: &str, 
    forecast_hour_override: Option<u32>,
    model_override: Option<&str>
) -> Result<()> {
    use storage::{Catalog, CatalogEntry, ObjectStorage};
    use wms_common::BoundingBox;

    info!(file = %test_file, forecast_hour = ?forecast_hour_override, model = ?model_override, "Testing ingestion with local file (shredded mode)");

    // Check if this is a NetCDF file (GOES satellite data)
    let is_netcdf = test_file.ends_with(".nc") || test_file.ends_with(".nc4");
    
    if is_netcdf {
        return test_goes_file_ingestion(config, test_file, model_override).await;
    }

    // Read file
    let data = fs::read(test_file)?;
    let data_bytes = Bytes::from(data);
    let original_file_size = data_bytes.len() as u64;

    // Setup storage and catalog
    let storage = ObjectStorage::new(&config.storage)?;
    let catalog = Catalog::connect(&config.database_url).await?;
    catalog.migrate().await?;

    // Determine model name (use override or extract from filename or default to "gfs")
    let model = model_override.map(String::from).or_else(|| {
        // Try to extract from filename like "hrrr.t00z.wrfsfcf00.grib2" -> "hrrr"
        // or "gfs.t00z.pgrb2.0p25.f003" -> "gfs"
        // or "MRMS_MergedReflectivityComposite..." -> "mrms"
        std::path::Path::new(test_file)
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| {
                if s.starts_with("hrrr.") {
                    Some("hrrr".to_string())
                } else if s.starts_with("gfs.") || s.starts_with("gfs_") {
                    Some("gfs".to_string())
                } else if s.starts_with("MRMS_") || s.contains("_latest") {
                    Some("mrms".to_string())
                } else {
                    None
                }
            })
    }).unwrap_or_else(|| "gfs".to_string());

    // Determine forecast hour (use override or extract from filename or default to 0)
    let forecast_hour = forecast_hour_override.or_else(|| {
        let filename = std::path::Path::new(test_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        
        // Try different patterns:
        // 1. GFS: "gfs_f003.grib2" or "f003"
        // 2. HRRR: "hrrr.t00z.wrfsfcf03.grib2" -> wrfsfcf03 -> 3
        if let Some(stripped) = filename.strip_prefix("gfs_f") {
            stripped.parse::<u32>().ok()
        } else if filename.contains("wrfsfcf") {
            // Extract hour from wrfsfcf##
            filename.split("wrfsfcf")
                .nth(1)
                .and_then(|s| s.get(..2))
                .and_then(|s| s.parse::<u32>().ok())
        } else if filename.starts_with('f') {
            filename.get(1..).and_then(|s| s.parse::<u32>().ok())
        } else {
            None
        }
    }).unwrap_or(0);

    // Extract MRMS parameter name from filename (since GRIB2 uses local tables)
    // Examples: "MRMS_MergedReflectivityComposite_..." -> "REFL"
    //           "PrecipRate_latest.grib2" -> "PRECIP_RATE"
    //           "MultiSensor_QPE_01H_Pass2_..." -> "QPE_01H"
    let mrms_param_name: Option<String> = if model == "mrms" {
        std::path::Path::new(test_file)
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| {
                let lower = s.to_lowercase();
                if lower.contains("reflectivity") || lower.contains("refl") {
                    Some("REFL".to_string())
                } else if lower.contains("preciprate") || lower.contains("precip_rate") {
                    Some("PRECIP_RATE".to_string())
                } else if lower.contains("qpe_01h") {
                    Some("QPE_01H".to_string())
                } else if lower.contains("qpe_03h") {
                    Some("QPE_03H".to_string())
                } else if lower.contains("qpe_06h") {
                    Some("QPE_06H".to_string())
                } else if lower.contains("qpe_24h") {
                    Some("QPE_24H".to_string())
                } else if lower.contains("qpe") {
                    Some("QPE".to_string())
                } else {
                    // Fall back to extracting the product name from MRMS_ prefix
                    if s.starts_with("MRMS_") {
                        s.strip_prefix("MRMS_")
                            .and_then(|rest| rest.split('_').next())
                            .map(|p| p.to_uppercase())
                    } else {
                        // Use the first part of the filename
                        s.split('_').next().map(|p| p.to_uppercase())
                    }
                }
            })
    } else {
        None
    };

    info!(forecast_hour = forecast_hour, model = %model, mrms_param = ?mrms_param_name, original_size = original_file_size, "Using forecast hour and model");

    // Parse GRIB2
    let mut reader = grib2_parser::Grib2Reader::new(data_bytes);
    let mut message_count = 0;
    let mut registered_params = HashSet::new();
    let mut grib_reference_time: Option<chrono::DateTime<Utc>> = None;

    // Parameters to ingest with their accepted level types
    // GRIB2 Code Table 4.5 level type codes:
    // - 1 = Ground/water surface
    // - 100 = Isobaric surface (pressure level in Pa, we convert to mb)
    // - 101 = Mean sea level
    // - 103 = Specified height above ground (value in meters)
    // - 104 = Sigma level
    // - 106 = Depth below land surface
    // - 200 = Entire atmosphere (column)
    // - 214 = Low cloud layer
    // - 224 = Middle cloud layer
    // - 234 = High cloud layer
    // - 108 = Specified height level above ground (layer)
    
    // Standard pressure levels to ingest (in mb)
    let pressure_levels: HashSet<u32> = [
        1000, 975, 950, 925, 900, 850, 800, 750, 700, 650, 
        600, 550, 500, 450, 400, 350, 300, 250, 200, 150, 
        100, 70, 50, 30, 20, 10
    ].into_iter().collect();
    
    // Parameters and the level types we accept for each
    // Format: (param, Vec<(level_type, optional_specific_value)>)
    let target_params: Vec<(&str, Vec<(u8, Option<u32>)>)> = vec![
        // ==========================================================================
        // Existing Parameters
        // ==========================================================================
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
        
        // ==========================================================================
        // Phase 1: Surface & Near-Surface Parameters
        // ==========================================================================
        ("DPT", vec![
            (103, Some(2)),                              // 2m dew point temperature
        ]),
        
        // ==========================================================================
        // Phase 1: Precipitation Parameters
        // ==========================================================================
        ("APCP", vec![
            (1, None),                                   // Surface total precipitation (accumulated)
        ]),
        ("PWAT", vec![
            (200, None),                                 // Entire atmosphere precipitable water
        ]),
        
        // ==========================================================================
        // Phase 1: Convective/Stability Parameters
        // ==========================================================================
        ("CAPE", vec![
            (1, None),                                   // Surface-based CAPE
            (108, None),                                 // CAPE in specified layer (for MUCAPE, SBCAPE)
        ]),
        ("CIN", vec![
            (1, None),                                   // Surface-based CIN
            (108, None),                                 // CIN in specified layer
        ]),
        
        // ==========================================================================
        // Phase 1: Cloud Parameters
        // ==========================================================================
        ("TCDC", vec![
            (200, None),                                 // Total cloud cover (entire atmosphere)
            (10, None),                                  // Entire atmosphere (alternative code)
        ]),
        ("LCDC", vec![
            (214, None),                                 // Low cloud layer
        ]),
        ("MCDC", vec![
            (224, None),                                 // Middle cloud layer
        ]),
        ("HCDC", vec![
            (234, None),                                 // High cloud layer
        ]),
        
        // ==========================================================================
        // Phase 1: Visibility
        // ==========================================================================
        ("VIS", vec![
            (1, None),                                   // Surface visibility
        ]),
        
        // ==========================================================================
        // HRRR-specific: Radar & Reflectivity
        // ==========================================================================
        ("REFC", vec![
            (200, None),                                 // Composite reflectivity (entire atmosphere)
            (10, None),                                  // Entire atmosphere (alternative)
            (103, Some(1000)),                           // 1000m above ground (alternative)
        ]),
        ("RETOP", vec![
            (3, None),                                   // Echo top (cloud top level)
            (200, None),                                 // Entire atmosphere
        ]),
        
        // ==========================================================================
        // HRRR-specific: Severe Weather Parameters
        // ==========================================================================
        ("MXUPHL", vec![
            (103, None),                                 // Height above ground layer (2-5km)
            (108, None),                                 // Specified height layer
        ]),
        ("LTNG", vec![
            (200, None),                                 // Lightning threat (entire atmosphere)
            (10, None),                                  // Entire atmosphere (alternative)
        ]),
        ("HLCY", vec![
            (106, None),                                 // Storm-relative helicity (0-1km, 0-3km layers)
            (108, None),                                 // Specified height layer
        ]),
    ];

    while let Some(message) = reader.next_message().ok().flatten() {
        message_count += 1;

        // Extract reference time from the first message (it's the same for all messages in a file)
        if grib_reference_time.is_none() {
            grib_reference_time = Some(message.identification.reference_time);
            info!(reference_time = %message.identification.reference_time, "Extracted reference time from GRIB2");
        }

        // For MRMS, use the parameter name extracted from filename (more reliable than local GRIB2 tables)
        // For other models, use the GRIB2 parameter name
        let grib_param = &message.product_definition.parameter_short_name;
        let param = if model == "mrms" {
            mrms_param_name.as_ref().unwrap_or(grib_param)
        } else {
            grib_param
        };
        let level = &message.product_definition.level_description;
        let level_type = message.product_definition.level_type;
        let level_value = message.product_definition.level_value;
        
        // Create a unique key for param+level combination
        let param_level_key = format!("{}:{}", param, level);

        // For MRMS, accept all parameters and derive parameter name from filename
        // For other models, check against the target parameter list
        let should_register = if model == "mrms" {
            // Accept any parameter that hasn't been registered yet
            !registered_params.contains(&param_level_key)
        } else {
            // Check if this matches one of our target parameter:level combinations
            target_params.iter().any(|(p, level_specs)| {
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
            })
        };

        if should_register {
            // Use reference time from GRIB2 file, fallback to now if not available
            let reference_time = grib_reference_time.unwrap_or_else(Utc::now);
            
            // Create a sanitized level string for the path (replace spaces and special chars)
            let level_sanitized = level
                .replace([' ', '/'], "_")
                .to_lowercase();
            
            // New shredded storage path structure:
            // shredded/{model}/{run_date}/{param}_{level}/f{fhr:03}.grib2
            // For observation data like MRMS (updates every ~2 minutes), use minute-level paths
            // For forecast models like GFS/HRRR, use hourly paths (they have hourly forecast cycles)
            let run_date = if model == "mrms" {
                reference_time.format("%Y%m%d_%H%Mz").to_string()
            } else {
                reference_time.format("%Y%m%d_%Hz").to_string()
            };
            let storage_path = format!(
                "shredded/{}/{}/{}_{}/f{:03}.grib2",
                model,
                run_date,
                param.to_lowercase(),
                level_sanitized,
                forecast_hour
            );
            
            // Extract just this message's raw data and store it
            let shredded_data = message.raw_data.clone();
            let shredded_size = shredded_data.len() as u64;
            
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
            
            // Get model-specific bounding box
            // TODO: Parse from GRIB2 Section 3 (Grid Definition) instead of hardcoding
            let bbox = match model.as_str() {
                "hrrr" => {
                    // HRRR CONUS domain (Lambert Conformal 3km)
                    // Extracted using wgrib2 -ijlat from actual HRRR files
                    // First point: (237.280472, 21.138123) -> (-122.719528, 21.138123)
                    // Last point: (299.082807, 47.842195) -> (-60.917193, 47.842195)
                    BoundingBox::new(-122.719528, 21.138123, -60.917193, 47.842195)
                },
                "mrms" => {
                    // MRMS CONUS domain (regular lat-lon 0.01 degree, ~1km)
                    // Grid: 7000 x 3500 points
                    // lat 54.995 to 20.005 by 0.01
                    // lon 230.005 to 299.995 by 0.01 (= -130.0 to -60.0)
                    BoundingBox::new(-130.0, 20.0, -60.0, 55.0)
                },
                "gfs" => {
                    // GFS global 0.25 degree
                    BoundingBox::new(0.0, -90.0, 360.0, 90.0)
                },
                _ => {
                    // Default to global for unknown models
                    warn!(model = %model, "Unknown model, using global bbox");
                    BoundingBox::new(0.0, -90.0, 360.0, 90.0)
                }
            };
            
            let entry = CatalogEntry {
                model: model.clone(),
                parameter: param.clone(),
                level: level.clone(),
                reference_time,
                forecast_hour,
                bbox,
                storage_path,
                file_size: shredded_size,
                zarr_metadata: None,
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

/// Ingest a local GOES NetCDF test file
async fn test_goes_file_ingestion(
    config: &IngesterConfig,
    test_file: &str,
    model_override: Option<&str>,
) -> Result<()> {
    use storage::{Catalog, CatalogEntry, ObjectStorage};
    use wms_common::BoundingBox;
    use chrono::TimeZone;

    info!(file = %test_file, model = ?model_override, "Testing GOES NetCDF ingestion");

    // Read file
    let data = fs::read(test_file)?;
    let data_bytes = Bytes::from(data);
    let file_size = data_bytes.len() as u64;

    // Setup storage and catalog
    let storage = ObjectStorage::new(&config.storage)?;
    let catalog = Catalog::connect(&config.database_url).await?;
    catalog.migrate().await?;

    // Parse filename to extract band and time info
    let filename = std::path::Path::new(test_file)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.nc");

    // Extract band number from filename (e.g., "C02" from "...M6C02_G16...")
    let band = filename
        .find("M6C")
        .or_else(|| filename.find("M3C"))
        .and_then(|pos| {
            let band_str = &filename[pos + 3..pos + 5];
            band_str.parse::<u8>().ok()
        })
        .unwrap_or(2); // Default to band 2 (visible red)

    // Determine model from filename or override
    let model = model_override.map(String::from).unwrap_or_else(|| {
        if filename.contains("_G16_") || filename.to_lowercase().contains("goes16") {
            "goes16".to_string()
        } else if filename.contains("_G18_") || filename.to_lowercase().contains("goes18") {
            "goes18".to_string()
        } else {
            "goes16".to_string() // Default to GOES-16
        }
    });

    // Extract observation time from filename (format: s20250500001170)
    // Time is in format: YYYYDDDHHMMSSt (year, day-of-year, hour, min, sec, tenths)
    let observation_time = filename
        .find("_s")
        .and_then(|pos| {
            if pos + 15 > filename.len() {
                return None;
            }
            let time_str = &filename[pos + 2..pos + 15];
            // Parse YYYYDDDHHMMSS
            let year: i32 = time_str.get(0..4)?.parse().ok()?;
            let doy: u32 = time_str.get(4..7)?.parse().ok()?;
            let hour: u32 = time_str.get(7..9)?.parse().ok()?;
            let min: u32 = time_str.get(9..11)?.parse().ok()?;
            let sec: u32 = time_str.get(11..13)?.parse().ok()?;

            // Convert to DateTime
            let date = chrono::NaiveDate::from_yo_opt(year, doy)?;
            let time = chrono::NaiveTime::from_hms_opt(hour, min, sec)?;
            Some(Utc.from_utc_datetime(&date.and_time(time)))
        })
        .unwrap_or_else(Utc::now);

    // Determine parameter name based on band
    let (parameter, level) = match band {
        1 => ("CMI_C01", "visible_blue"),       // 0.47µm Blue
        2 => ("CMI_C02", "visible_red"),        // 0.64µm Red (most common visible)
        3 => ("CMI_C03", "visible_veggie"),     // 0.86µm Vegetation
        4 => ("CMI_C04", "cirrus"),             // 1.37µm Cirrus
        5 => ("CMI_C05", "snow_ice"),           // 1.6µm Snow/Ice
        6 => ("CMI_C06", "cloud_particle"),     // 2.2µm Cloud Particle Size
        7 => ("CMI_C07", "shortwave_ir"),       // 3.9µm Shortwave Window
        8 => ("CMI_C08", "upper_vapor"),        // 6.2µm Upper-Level Water Vapor
        9 => ("CMI_C09", "mid_vapor"),          // 6.9µm Mid-Level Water Vapor
        10 => ("CMI_C10", "low_vapor"),         // 7.3µm Lower-Level Water Vapor
        11 => ("CMI_C11", "cloud_phase"),       // 8.4µm Cloud-Top Phase
        12 => ("CMI_C12", "ozone"),             // 9.6µm Ozone
        13 => ("CMI_C13", "clean_ir"),          // 10.3µm "Clean" Longwave IR
        14 => ("CMI_C14", "ir"),                // 11.2µm Longwave IR
        15 => ("CMI_C15", "dirty_ir"),          // 12.3µm "Dirty" Longwave IR
        16 => ("CMI_C16", "co2"),               // 13.3µm CO2
        _ => ("CMI_C02", "visible_red"),        // Default to visible red
    };

    info!(
        band = band,
        model = %model,
        parameter = parameter,
        observation_time = %observation_time,
        file_size = file_size,
        "Parsed GOES file metadata"
    );

    // Create storage path - include hour and minute for GOES (5-minute intervals)
    let run_datetime = observation_time.format("%Y%m%d_%H%Mz").to_string();
    let storage_path = format!(
        "raw/{}/{}/{}.nc",
        model,
        run_datetime,
        parameter.to_lowercase()
    );

    // Store the NetCDF file
    storage.put(&storage_path, data_bytes).await?;
    info!(path = %storage_path, "Stored GOES NetCDF file");

    // Get model-specific bounding box
    let bbox = match model.as_str() {
        "goes16" => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
        "goes18" => BoundingBox::new(-165.0, 14.5, -90.0, 55.5),
        _ => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
    };

    // Create catalog entry
    let entry = CatalogEntry {
        model: model.clone(),
        parameter: parameter.to_string(),
        level: level.to_string(),
        reference_time: observation_time,
        forecast_hour: 0, // Observational data, no forecast
        bbox,
        storage_path: storage_path.clone(),
        file_size,
        zarr_metadata: None,
    };

    match catalog.register_dataset(&entry).await {
        Ok(id) => {
            info!(id = %id, parameter = %parameter, model = %model, "Registered GOES dataset");
        }
        Err(e) => {
            warn!(error = %e, "Could not register (may already exist)");
        }
    }

    info!(
        model = %model,
        parameter = %parameter,
        band = band,
        path = %storage_path,
        "GOES file ingestion completed"
    );

    Ok(())
}
