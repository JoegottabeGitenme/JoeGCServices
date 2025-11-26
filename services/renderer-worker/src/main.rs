//! Renderer worker service.
//!
//! Consumes render jobs from Redis queue and produces tile images.

use anyhow::Result;
use clap::Parser;
use std::env;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;
use uuid::Uuid;

use storage::{CacheKey, JobQueue, ObjectStorage, ObjectStorageConfig, RenderJob, TileCache, Catalog};

#[derive(Parser, Debug)]
#[command(name = "renderer-worker")]
#[command(about = "Render worker for WMS tile generation")]
struct Args {
    /// Worker name (for consumer group)
    #[arg(short, long, env = "WORKER_NAME")]
    name: Option<String>,

    /// Number of concurrent render tasks
    #[arg(short, long, default_value = "4")]
    concurrency: usize,

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
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .json()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    let worker_name = args
        .name
        .unwrap_or_else(|| format!("worker-{}", Uuid::new_v4()));

    info!(name = %worker_name, "Starting renderer worker");

    // Connect to services
    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());
    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://user:password@db:5432/weather".to_string());

    let mut queue = JobQueue::connect(&redis_url).await?;
    let mut cache = TileCache::connect(&redis_url).await?;
    let catalog = Catalog::connect(&db_url).await?;

    let storage_config = ObjectStorageConfig {
        endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
        bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
        access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
        secret_access_key: env::var("S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    let storage = ObjectStorage::new(&storage_config)?;

    info!("Connected to Redis, database, and object storage");

    // Main worker loop
    loop {
        match queue.claim_next(&worker_name).await {
            Ok(Some(job)) => {
                info!(job_id = %job.id, layer = %job.layer, "Processing render job");

                match render_tile(&storage, &catalog, &job).await {
                    Ok(image_data) => {
                        // Store in cache
                        let cache_key = CacheKey::new(
                            &job.layer,
                            &job.style,
                            job.crs,
                            job.bbox,
                            job.width,
                            job.height,
                            job.time.clone(),
                            &job.format,
                        );

                        if let Err(e) = cache.set(&cache_key, &image_data, None).await {
                            warn!(error = %e, "Failed to cache tile");
                        }

                        // Complete the job
                        if let Err(e) = queue.complete(&job.id, &image_data).await {
                            error!(error = %e, "Failed to complete job");
                        }

                        info!(job_id = %job.id, size = image_data.len(), "Render complete");
                    }
                    Err(e) => {
                        error!(job_id = %job.id, error = %e, "Render failed");
                        if let Err(e) = queue.fail(&job.id, &e.to_string()).await {
                            error!(error = %e, "Failed to mark job as failed");
                        }
                    }
                }
            }
            Ok(None) => {
                // No jobs available, continue polling
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => {
                error!(error = %e, "Error claiming job");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

/// Render a tile for the given job.
async fn render_tile(storage: &ObjectStorage, catalog: &Catalog, job: &RenderJob) -> Result<Vec<u8>> {
    // Parse layer name (format: "model_parameter")
    let parts: Vec<&str> = job.layer.split('_').collect();
    if parts.len() < 2 {
        return Err(anyhow::anyhow!("Invalid layer format: {}", job.layer));
    }

    let model = parts[0];
    let parameter = parts[1..].join("_");

    // Parse TIME parameter (format: "H" for forecast hour)
    let forecast_hour: Option<u32> = job.time.as_ref().and_then(|t| t.parse().ok());
    
    info!(forecast_hour = ?forecast_hour, "Parsed TIME parameter");

    // Get dataset for this parameter
    let entry = if let Some(hour) = forecast_hour {
        // Find dataset with matching forecast hour
        catalog
            .find_by_forecast_hour(model, &parameter, hour)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No data found for {}/{} at hour {}", model, parameter, hour))?
    } else {
        // Get latest dataset
        catalog
            .get_latest(model, &parameter)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No data found for {}/{}", model, parameter))?
    };

    // Load GRIB2 file from storage
    let grib_data = storage
        .get(&entry.storage_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load GRIB2 file: {}", e))?;

    // Parse GRIB2 and find matching message
    // Strategy: Look for exact match first, then substring match
    // If multiple messages match, prefer the one with the lowest level (e.g., surface over upper-level)
    let mut reader = grib2_parser::Grib2Reader::new(grib_data);
    let mut exact_match = None;
    let mut substring_matches = Vec::new();

    while let Some(msg) = reader.next_message()? {
        let msg_param = msg.parameter();
        
        // Look for exact match
        if msg_param == &parameter[..] {
            exact_match = Some(msg);
            break; // Exact match takes priority
        }
        
        // Look for substring matches
        if msg_param.contains(&parameter[..]) || parameter.contains(msg_param) {
            substring_matches.push(msg);
        }
    }

    let msg = if let Some(msg) = exact_match {
        msg
    } else if !substring_matches.is_empty() {
        // Use the first substring match
        substring_matches.into_iter().next().unwrap()
    } else {
        return Err(anyhow::anyhow!("Parameter {} not found in GRIB2", parameter));
    };

    // Unpack grid data
    let grid_data = msg.unpack_data()?;

    let (grid_height, grid_width) = msg.grid_dims();
    let grid_width = grid_width as usize;
    let grid_height = grid_height as usize;

    info!(
        "Grid dimensions: {}x{}, data points: {}",
        grid_width,
        grid_height,
        grid_data.len()
    );

    if grid_data.len() != grid_width * grid_height {
        return Err(anyhow::anyhow!(
            "Grid data size mismatch: {} vs {}x{}",
            grid_data.len(),
            grid_width,
            grid_height
        ));
    }

    // Find data min/max for scaling
    let (min_val, max_val) = grid_data
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &val| {
            (min.min(val), max.max(val))
        });

    info!("Data range: {} to {}", min_val, max_val);
    info!("Sample values: {:?}", &grid_data[0..10.min(grid_data.len())]);

    // Resample grid to match request dimensions
    let rendered_width = job.width as usize;
    let rendered_height = job.height as usize;

    let resampled_data = if grid_width != rendered_width || grid_height != rendered_height {
        info!(
            "Resampling grid from {}x{} to {}x{}",
            grid_width, grid_height, rendered_width, rendered_height
        );
        renderer::gradient::resample_grid(&grid_data, grid_width, grid_height, rendered_width, rendered_height)
    } else {
        grid_data.clone()
    };

    // Render based on parameter type
    let rgba_data = if parameter.contains("TMP") || parameter.contains("TEMP") {
        // Temperature in Kelvin, convert to Celsius for rendering
        let celsius_data: Vec<f32> = resampled_data.iter().map(|k| k - 273.15).collect();
        let min_c = min_val - 273.15;
        let max_c = max_val - 273.15;
        
        info!(
            "Temperature range: {:.2}°C to {:.2}°C",
            min_c, max_c
        );
        
        renderer::gradient::render_temperature(&celsius_data, rendered_width, rendered_height, min_c, max_c)
    } else if parameter.contains("WIND") || parameter.contains("GUST") || parameter.contains("SPEED") {
        // Wind speed in m/s
        info!(
            "Wind speed range: {:.2} to {:.2} m/s",
            min_val, max_val
        );
        
        renderer::gradient::render_wind_speed(&resampled_data, rendered_width, rendered_height, min_val, max_val)
    } else if parameter.contains("PRES") || parameter.contains("PRESS") {
        // Pressure in Pa, convert to hPa
        let hpa_data: Vec<f32> = resampled_data.iter().map(|pa| pa / 100.0).collect();
        let min_hpa = min_val / 100.0;
        let max_hpa = max_val / 100.0;
        
        info!(
            "Pressure range: {:.2} to {:.2} hPa",
            min_hpa, max_hpa
        );
        
        renderer::gradient::render_pressure(&hpa_data, rendered_width, rendered_height, min_hpa, max_hpa)
    } else if parameter.contains("RH") || parameter.contains("HUMID") {
        // Relative humidity in percent (0-100)
        info!(
            "Humidity range: {:.2}% to {:.2}%",
            min_val, max_val
        );
        
        renderer::gradient::render_humidity(&resampled_data, rendered_width, rendered_height, min_val, max_val)
    } else {
        // Generic gradient rendering
        renderer::gradient::render_grid(
            &resampled_data,
            rendered_width,
            rendered_height,
            min_val,
            max_val,
            |norm| {
                // Generic blue-red gradient
                let hue = (1.0 - norm) * 240.0; // Blue to red
                let rgb = hsv_to_rgb(hue, 1.0, 1.0);
                renderer::gradient::Color::new(rgb.0, rgb.1, rgb.2, 255)
            },
        )
    };

    info!(
        "Rendered PNG from {}x{} grid, output {}x{}",
        grid_width, grid_height, rendered_width, rendered_height
    );

    // Convert to PNG 
    let png = renderer::png::create_png(&rgba_data, rendered_width, rendered_height)
        .map_err(|e| anyhow::anyhow!("PNG encoding failed: {}", e))?;

    Ok(png)
}

/// Convert HSV to RGB (simplified version)
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
