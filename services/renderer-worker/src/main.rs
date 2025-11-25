//! Renderer worker service.
//!
//! Consumes render jobs from Redis queue and produces tile images.

use anyhow::Result;
use clap::Parser;
use std::env;
use tracing::{info, warn, error, Level};
use tracing_subscriber::FmtSubscriber;
use uuid::Uuid;

use storage::{ObjectStorage, ObjectStorageConfig, JobQueue, RenderJob, TileCache, CacheKey};

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

    let worker_name = args.name
        .unwrap_or_else(|| format!("worker-{}", Uuid::new_v4()));

    info!(name = %worker_name, "Starting renderer worker");

    // Connect to services
    let redis_url = env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://redis:6379".to_string());

    let mut queue = JobQueue::connect(&redis_url).await?;
    let mut cache = TileCache::connect(&redis_url).await?;

    let storage_config = ObjectStorageConfig {
        endpoint: env::var("S3_ENDPOINT").unwrap_or_else(|_| "http://minio:9000".to_string()),
        bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "weather-data".to_string()),
        access_key_id: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
        secret_access_key: env::var("S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".to_string()),
        region: "us-east-1".to_string(),
        allow_http: true,
    };

    let storage = ObjectStorage::new(&storage_config)?;

    info!("Connected to Redis and object storage");

    // Main worker loop
    loop {
        match queue.claim_next(&worker_name).await {
            Ok(Some(job)) => {
                info!(job_id = %job.id, layer = %job.layer, "Processing render job");

                match render_tile(&storage, &job).await {
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
            }
            Err(e) => {
                error!(error = %e, "Error claiming job");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

/// Render a tile for the given job.
async fn render_tile(storage: &ObjectStorage, job: &RenderJob) -> Result<Vec<u8>> {
    // TODO: Implement actual rendering
    // 1. Load grid data from storage
    // 2. Apply coordinate transformation
    // 3. Render using appropriate style
    // 4. Encode as PNG

    // For now, generate a test pattern
    let width = job.width as usize;
    let height = job.height as usize;

    let mut pixels = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            
            // Simple gradient pattern
            pixels[idx] = (x * 255 / width) as u8;     // R
            pixels[idx + 1] = (y * 255 / height) as u8; // G
            pixels[idx + 2] = 128;                       // B
            pixels[idx + 3] = 255;                       // A
        }
    }

    // Encode as PNG (placeholder - using a simple format)
    // TODO: Use proper PNG encoder from renderer crate
    Ok(encode_test_png(width, height, &pixels))
}

/// Simple PNG encoder for test patterns.
fn encode_test_png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    // This is a placeholder - real implementation would use the renderer crate
    // For now, return a minimal valid PNG
    
    let mut png = Vec::new();
    
    // PNG signature
    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    
    // IHDR chunk
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.push(8);  // bit depth
    ihdr.push(6);  // color type (RGBA)
    ihdr.push(0);  // compression
    ihdr.push(0);  // filter
    ihdr.push(0);  // interlace
    
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    
    // IDAT chunk (compressed image data)
    // For simplicity, using uncompressed deflate
    let mut raw_data = Vec::new();
    for y in 0..height {
        raw_data.push(0); // filter type: none
        for x in 0..width {
            let idx = (y * width + x) * 4;
            raw_data.extend_from_slice(&rgba[idx..idx + 4]);
        }
    }
    
    // Compress with deflate
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw_data).unwrap();
    let compressed = encoder.finish().unwrap();
    
    write_png_chunk(&mut png, b"IDAT", &compressed);
    
    // IEND chunk
    write_png_chunk(&mut png, b"IEND", &[]);
    
    png
}

fn write_png_chunk(output: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(chunk_type);
    output.extend_from_slice(data);
    
    let crc = crc32fast::hash(&[chunk_type.as_slice(), data].concat());
    output.extend_from_slice(&crc.to_be_bytes());
}

use flate2;
use crc32fast;
