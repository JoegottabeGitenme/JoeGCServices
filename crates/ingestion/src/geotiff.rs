//! GeoTIFF file ingestion logic.
//!
//! Handles GeoTIFF files (VIIRS light pollution data) and converts them to
//! Zarr format. Uses streaming to handle very large files without loading
//! the entire dataset into memory.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;
use tracing::{info, warn};
use zarrs::array::codec::bytes_to_bytes::blosc::{
    BloscCodec, BloscCompressionLevel, BloscCompressor, BloscShuffleMode,
};
use zarrs::array::{Array, ArrayBuilder, DataType, FillValue};
use zarrs::array_subset::ArraySubset;
use zarrs_filesystem::FilesystemStore;

use storage::{Catalog, CatalogEntry, ObjectStorage};
use wms_common::BoundingBox;

use crate::error::{IngestionError, Result};
use crate::upload::upload_zarr_directory;
use crate::{IngestOptions, IngestionResult};

/// GeoTIFF tag IDs for geographic metadata.
const GEOTIFF_MODEL_PIXEL_SCALE_TAG: u16 = 33550;
const GEOTIFF_MODEL_TIEPOINT_TAG: u16 = 33922;
const GDAL_NODATA_TAG: u16 = 42113;

/// Zarr chunk size for output (512x512 = 1MB per chunk at f32).
const ZARR_CHUNK_SIZE: u64 = 512;

/// Maximum pixels to process in memory at once (~256MB buffer).
const MAX_BUFFER_PIXELS: usize = 64 * 1024 * 1024;

/// Metadata extracted from a GeoTIFF file.
#[derive(Debug, Clone)]
pub struct GeoTiffMetadata {
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// Geographic bounding box (EPSG:4326)
    pub bbox: BoundingBox,
    /// Pixel scale in degrees (x, y)
    pub pixel_scale: (f64, f64),
    /// NoData value (if specified)
    pub no_data_value: Option<f64>,
}

/// Ingest a GeoTIFF file into Zarr format.
///
/// For large files (like VIIRS global data), this uses streaming to avoid
/// loading the entire dataset into memory. The TIFF is read chunk-by-chunk
/// and written directly to Zarr chunks.
pub async fn ingest_geotiff(
    storage: &Arc<ObjectStorage>,
    catalog: &Catalog,
    data: Bytes,
    file_path: &str,
    options: &IngestOptions,
) -> Result<IngestionResult> {
    let model = options
        .model
        .clone()
        .unwrap_or_else(|| extract_model_from_geotiff_path(file_path));

    info!(
        model = %model,
        file_size = data.len(),
        file_path = %file_path,
        "Ingesting GeoTIFF file"
    );

    // For large gzip files (>50MB), decompress to temp file to avoid memory pressure
    let is_gzip = file_path.to_lowercase().ends_with(".gz");
    let is_large = data.len() > 50 * 1024 * 1024; // 50MB threshold

    // Create limits for TIFF decoder
    let mut limits = Limits::default();
    limits.decoding_buffer_size = 256 * 1024 * 1024; // 256MB for strip decoding
    limits.intermediate_buffer_size = 256 * 1024 * 1024;

    if is_gzip && is_large {
        // Large gzip file: decompress to temp file to avoid memory pressure
        info!(
            compressed_size_mb = data.len() / (1024 * 1024),
            "Large gzip file detected, decompressing to temp file..."
        );
        return ingest_geotiff_from_gzip(
            storage, catalog, data, file_path, &model, limits, options,
        )
        .await;
    }

    // Handle small gzip-compressed GeoTIFF files in memory
    let data = if is_gzip {
        info!("Decompressing small gzip file in memory...");
        decompress_gzip(&data)?
    } else {
        data
    };

    info!(decompressed_size = data.len(), "Decompressed data ready");

    // Parse GeoTIFF header (doesn't load pixel data)
    let cursor = Cursor::new(data.as_ref());

    let mut decoder = Decoder::new(cursor)
        .map_err(|e| IngestionError::GeoTiffParse(format!("Failed to decode TIFF: {}", e)))?;
    decoder = decoder.with_limits(limits);

    // Extract metadata from GeoTIFF tags
    let metadata = extract_geotiff_metadata(&mut decoder)?;
    let total_pixels = metadata.width as u64 * metadata.height as u64;

    info!(
        width = metadata.width,
        height = metadata.height,
        total_pixels = total_pixels,
        bbox = ?metadata.bbox,
        pixel_scale = ?metadata.pixel_scale,
        no_data = ?metadata.no_data_value,
        "Extracted GeoTIFF metadata"
    );

    // Determine parameter name
    let param = determine_parameter(&model, file_path);
    let reference_time = Utc::now();
    let zarr_storage_path = build_storage_path(&model, &param);

    // Create temporary directory for Zarr output
    let temp_dir = tempfile::tempdir()?;
    let zarr_path = temp_dir.path().join("grid.zarr");
    std::fs::create_dir_all(&zarr_path)?;

    // Check if we should use streaming or direct approach
    let is_large = total_pixels > MAX_BUFFER_PIXELS as u64;

    if is_large {
        info!(
            "Large GeoTIFF detected ({:.1}M pixels), using streaming ingestion",
            total_pixels as f64 / 1_000_000.0
        );
        stream_geotiff_to_zarr(&mut decoder, &metadata, &model, &param, &zarr_path)?;
    } else {
        info!("Small GeoTIFF, using direct ingestion");
        direct_geotiff_to_zarr(&mut decoder, &metadata, &model, &param, &zarr_path)?;
    }

    // Upload to object storage
    let zarr_file_size = upload_zarr_directory(storage, &zarr_path, &zarr_storage_path).await?;

    info!(
        path = %zarr_storage_path,
        size_mb = zarr_file_size / (1024 * 1024),
        "Uploaded Zarr to storage"
    );

    // Build zarr metadata for catalog
    let zarr_metadata = build_zarr_metadata(&metadata, &model, &param, reference_time);

    // Register in catalog
    let entry = CatalogEntry {
        model: model.clone(),
        parameter: param.clone(),
        level: "surface".to_string(),
        reference_time,
        forecast_hour: 0,
        bbox: metadata.bbox,
        storage_path: zarr_storage_path,
        file_size: zarr_file_size,
        zarr_metadata: Some(zarr_metadata),
    };

    match catalog.register_dataset(&entry).await {
        Ok(id) => {
            info!(id = %id, param = %param, model = %model, "Registered GeoTIFF dataset");
        }
        Err(e) => {
            warn!(
                param = %param,
                model = %model,
                error = %e,
                "Could not register dataset (may already exist)"
            );
        }
    }

    info!(
        model = %model,
        param = %param,
        size_mb = zarr_file_size / (1024 * 1024),
        "GeoTIFF ingestion complete"
    );

    Ok(IngestionResult {
        datasets_registered: 1,
        model,
        reference_time,
        parameters: vec![param],
        bytes_written: zarr_file_size,
    })
}

/// Ingest a large gzip-compressed GeoTIFF by decompressing to a temp file first.
///
/// This avoids holding the entire decompressed data in memory, which is critical
/// for large files like VIIRS global data (~2.5GB decompressed).
async fn ingest_geotiff_from_gzip(
    storage: &Arc<ObjectStorage>,
    catalog: &Catalog,
    compressed_data: Bytes,
    file_path: &str,
    model: &str,
    limits: Limits,
    _options: &IngestOptions,
) -> Result<IngestionResult> {
    // Create a temp file for decompressed data
    let temp_file = tempfile::NamedTempFile::new()?;
    let temp_path = temp_file.path().to_path_buf();

    info!(
        temp_path = %temp_path.display(),
        "Decompressing to temp file..."
    );

    // Decompress to the temp file with progress logging
    {
        let mut decoder = GzDecoder::new(compressed_data.as_ref());
        let file = File::create(&temp_path)?;
        let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file); // 8MB buffer

        let mut buffer = vec![0u8; 8 * 1024 * 1024]; // 8MB read buffer
        let mut total_written = 0u64;
        let mut last_log = 0u64;

        loop {
            let bytes_read = decoder
                .read(&mut buffer)
                .map_err(|e| IngestionError::Decompression(e.to_string()))?;

            if bytes_read == 0 {
                break;
            }

            writer.write_all(&buffer[..bytes_read])?;

            total_written += bytes_read as u64;

            // Log progress every 500MB
            if total_written - last_log >= 500 * 1024 * 1024 {
                info!(
                    decompressed_mb = total_written / (1024 * 1024),
                    "Decompression progress..."
                );
                last_log = total_written;
            }
        }

        writer.flush()?;

        info!(
            total_size_mb = total_written / (1024 * 1024),
            "Decompression complete"
        );
    }

    // Now open the temp file and use file-based TIFF decoding
    let file = File::open(&temp_path)?;
    let reader = BufReader::with_capacity(64 * 1024 * 1024, file); // 64MB read buffer

    let mut decoder = Decoder::new(reader)
        .map_err(|e| IngestionError::GeoTiffParse(format!("Failed to decode TIFF: {}", e)))?;
    decoder = decoder.with_limits(limits);

    // Extract metadata from GeoTIFF tags
    let metadata = extract_geotiff_metadata(&mut decoder)?;
    let total_pixels = metadata.width as u64 * metadata.height as u64;

    info!(
        width = metadata.width,
        height = metadata.height,
        total_pixels = total_pixels,
        bbox = ?metadata.bbox,
        pixel_scale = ?metadata.pixel_scale,
        no_data = ?metadata.no_data_value,
        "Extracted GeoTIFF metadata"
    );

    // Determine parameter name
    let param = determine_parameter(model, file_path);
    let reference_time = Utc::now();
    let zarr_storage_path = build_storage_path(model, &param);

    // Create temporary directory for Zarr output
    let zarr_temp_dir = tempfile::tempdir()?;
    let zarr_path = zarr_temp_dir.path().join("grid.zarr");
    std::fs::create_dir_all(&zarr_path)?;

    // Always use streaming for large files
    info!(
        "Large GeoTIFF ({:.1}M pixels), using streaming ingestion",
        total_pixels as f64 / 1_000_000.0
    );
    stream_geotiff_to_zarr(&mut decoder, &metadata, model, &param, &zarr_path)?;

    // Upload to object storage
    let zarr_file_size = upload_zarr_directory(storage, &zarr_path, &zarr_storage_path).await?;

    info!(
        path = %zarr_storage_path,
        size_mb = zarr_file_size / (1024 * 1024),
        "Uploaded Zarr to storage"
    );

    // Build zarr metadata for catalog
    let zarr_metadata = build_zarr_metadata(&metadata, model, &param, reference_time);

    // Register in catalog
    let entry = CatalogEntry {
        model: model.to_string(),
        parameter: param.clone(),
        level: "surface".to_string(),
        reference_time,
        forecast_hour: 0,
        bbox: metadata.bbox,
        storage_path: zarr_storage_path,
        file_size: zarr_file_size,
        zarr_metadata: Some(zarr_metadata),
    };

    match catalog.register_dataset(&entry).await {
        Ok(id) => {
            info!(id = %id, param = %param, model = %model, "Registered GeoTIFF dataset");
        }
        Err(e) => {
            warn!(
                param = %param,
                model = %model,
                error = %e,
                "Could not register dataset (may already exist)"
            );
        }
    }

    info!(
        model = %model,
        param = %param,
        size_mb = zarr_file_size / (1024 * 1024),
        "GeoTIFF ingestion complete"
    );

    // Temp files are automatically cleaned up when they go out of scope

    Ok(IngestionResult {
        datasets_registered: 1,
        model: model.to_string(),
        reference_time,
        parameters: vec![param],
        bytes_written: zarr_file_size,
    })
}

/// Stream large GeoTIFF directly to Zarr, chunk by chunk.
///
/// This reads the TIFF strip-by-strip and writes to Zarr chunks without
/// ever holding the entire dataset in memory.
fn stream_geotiff_to_zarr<R: Read + std::io::Seek>(
    decoder: &mut Decoder<R>,
    metadata: &GeoTiffMetadata,
    model: &str,
    _param: &str,
    zarr_path: &Path,
) -> Result<()> {
    let width = metadata.width as usize;
    let height = metadata.height as usize;

    // Create Zarr array structure
    let store = FilesystemStore::new(zarr_path)
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create store: {}", e)))?;

    // Use Blosc compression (same as other ingestion pipelines)
    // BloscCodec::new(compressor, level, blocksize, shuffle, typesize)
    // typesize=4 for f32 data when shuffle is enabled
    let blosc_codec = BloscCodec::new(
        BloscCompressor::Zstd,
        BloscCompressionLevel::try_from(5).unwrap(),
        None, // auto blocksize
        BloscShuffleMode::Shuffle,
        Some(4), // f32 = 4 bytes
    )
    .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create Blosc codec: {}", e)))?;

    let array = ArrayBuilder::new(
        vec![height as u64, width as u64],
        DataType::Float32,
        vec![ZARR_CHUNK_SIZE, ZARR_CHUNK_SIZE].try_into().unwrap(),
        FillValue::from(f32::NAN),
    )
    .bytes_to_bytes_codecs(vec![Arc::new(blosc_codec)])
    .build(Arc::new(store), "/0") // Write as level 0 of pyramid (matching grid-processor expectation)
    .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create array: {}", e)))?;

    // Get TIFF chunk (strip/tile) info
    let chunk_dims = decoder.chunk_dimensions();
    let tiff_chunk_width = chunk_dims.0 as usize;
    let tiff_chunk_height = chunk_dims.1 as usize;
    let chunks_across = (width + tiff_chunk_width - 1) / tiff_chunk_width;
    let chunks_down = (height + tiff_chunk_height - 1) / tiff_chunk_height;
    let total_tiff_chunks = chunks_across * chunks_down;

    info!(
        tiff_chunks = total_tiff_chunks,
        tiff_chunk_size = format!("{}x{}", tiff_chunk_width, tiff_chunk_height),
        zarr_chunk_size = ZARR_CHUNK_SIZE,
        "Starting streaming ingestion"
    );

    // Valid range for filtering (VIIRS specific)
    let (min_valid, max_valid) = if model == "viirs" {
        (0.0_f32, 300.0_f32)
    } else {
        (f32::NEG_INFINITY, f32::INFINITY)
    };
    let no_data = metadata.no_data_value.map(|v| v as f32);

    // Track which Zarr chunks we've accumulated data for
    let _zarr_chunks_x = (width as u64 + ZARR_CHUNK_SIZE - 1) / ZARR_CHUNK_SIZE;
    let _zarr_chunks_y = (height as u64 + ZARR_CHUNK_SIZE - 1) / ZARR_CHUNK_SIZE;

    // Process row-by-row to accumulate full Zarr chunk rows before writing
    // This is more memory efficient than tracking partial chunks
    let rows_per_zarr_chunk = ZARR_CHUNK_SIZE as usize;
    let mut current_chunk_row = 0usize;
    let mut row_buffer: Vec<Vec<f32>> = Vec::with_capacity(rows_per_zarr_chunk);

    // We'll process TIFF strips and accumulate rows
    let _tiff_row = 0usize;
    let mut chunks_processed = 0u32;
    let mut last_progress = 0u32;

    for tiff_chunk_idx in 0..total_tiff_chunks as u32 {
        let chunk_x = (tiff_chunk_idx as usize) % chunks_across;
        let chunk_y = (tiff_chunk_idx as usize) / chunks_across;

        let start_x = chunk_x * tiff_chunk_width;
        let start_y = chunk_y * tiff_chunk_height;
        let actual_width = tiff_chunk_width.min(width - start_x);
        let actual_height = tiff_chunk_height.min(height - start_y);

        // Read this TIFF chunk
        let chunk_data = match decoder.read_chunk(tiff_chunk_idx) {
            Ok(result) => decode_chunk_to_f32(result),
            Err(e) => {
                warn!("Failed to read TIFF chunk {}: {}", tiff_chunk_idx, e);
                // Fill with NaN for failed chunks
                vec![f32::NAN; tiff_chunk_width * tiff_chunk_height]
            }
        };

        // Process each row in this TIFF chunk
        for local_row in 0..actual_height {
            let img_row = start_y + local_row;

            // Ensure row_buffer has this row
            while row_buffer.len() <= img_row % rows_per_zarr_chunk {
                row_buffer.push(vec![f32::NAN; width]);
            }

            let buffer_row = img_row % rows_per_zarr_chunk;

            // Copy pixels from TIFF chunk to row buffer, applying filtering
            for local_col in 0..actual_width {
                let img_col = start_x + local_col;
                let tiff_idx = local_row * tiff_chunk_width + local_col;

                if tiff_idx < chunk_data.len() {
                    let mut value = chunk_data[tiff_idx];

                    // Apply NoData filtering
                    if let Some(nd) = no_data {
                        if (value - nd).abs() < f32::EPSILON || value == nd {
                            value = f32::NAN;
                        }
                    }

                    // Apply valid range filtering
                    if !value.is_nan() && (value < min_valid || value > max_valid) {
                        value = f32::NAN;
                    }

                    row_buffer[buffer_row][img_col] = value;
                }
            }

            // Check if we've completed a full Zarr chunk row
            if img_row > 0 && (img_row + 1) % rows_per_zarr_chunk == 0 {
                // Write all Zarr chunks for this row of chunks
                write_zarr_chunk_row(
                    &array,
                    &row_buffer,
                    current_chunk_row,
                    width,
                    ZARR_CHUNK_SIZE as usize,
                )?;

                current_chunk_row += 1;
                row_buffer.clear();
            }
        }

        chunks_processed += 1;

        // Progress logging every 5%
        let progress = (chunks_processed * 100 / total_tiff_chunks as u32) / 5 * 5;
        if progress > last_progress {
            info!(
                "Streaming progress: {}% ({}/{} TIFF chunks)",
                progress, chunks_processed, total_tiff_chunks
            );
            last_progress = progress;
        }
    }

    // Write any remaining rows (final partial chunk row)
    if !row_buffer.is_empty() {
        write_zarr_chunk_row(
            &array,
            &row_buffer,
            current_chunk_row,
            width,
            ZARR_CHUNK_SIZE as usize,
        )?;
    }

    // Store array metadata
    array
        .store_metadata()
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to store metadata: {}", e)))?;

    info!(
        "Streaming complete: wrote {} Zarr chunk rows",
        current_chunk_row + 1
    );

    Ok(())
}

/// Write a row of Zarr chunks from the row buffer.
fn write_zarr_chunk_row(
    array: &Array<FilesystemStore>,
    row_buffer: &[Vec<f32>],
    chunk_row: usize,
    width: usize,
    chunk_size: usize,
) -> Result<()> {
    let chunks_x = (width + chunk_size - 1) / chunk_size;

    for chunk_x in 0..chunks_x {
        let start_col = chunk_x * chunk_size;
        let end_col = (start_col + chunk_size).min(width);
        let chunk_width = end_col - start_col;
        let chunk_height = row_buffer.len();

        // Extract chunk data
        let mut chunk_data = Vec::with_capacity(chunk_height * chunk_width);
        for row in row_buffer {
            for col in start_col..end_col {
                chunk_data.push(if col < row.len() { row[col] } else { f32::NAN });
            }
        }

        // Pad to full chunk size if needed
        let full_chunk_size = chunk_size * chunk_size;
        while chunk_data.len() < full_chunk_size {
            chunk_data.push(f32::NAN);
        }

        // Write chunk
        let chunk_indices = vec![chunk_row as u64, chunk_x as u64];
        array
            .store_chunk_elements(&chunk_indices, &chunk_data)
            .map_err(|e| {
                IngestionError::ZarrWrite(format!(
                    "Failed to write chunk [{}, {}]: {}",
                    chunk_row, chunk_x, e
                ))
            })?;
    }

    Ok(())
}

/// Direct ingestion for smaller GeoTIFFs (loads entire image into memory).
fn direct_geotiff_to_zarr<R: Read + std::io::Seek>(
    decoder: &mut Decoder<R>,
    metadata: &GeoTiffMetadata,
    model: &str,
    _param: &str,
    zarr_path: &Path,
) -> Result<()> {
    let width = metadata.width as usize;
    let height = metadata.height as usize;

    // Read entire image
    let result = decoder
        .read_image()
        .map_err(|e| IngestionError::GeoTiffParse(format!("Failed to read TIFF image: {}", e)))?;

    let mut data = decode_chunk_to_f32(result);

    // Apply filtering
    let (min_valid, max_valid) = if model == "viirs" {
        (0.0_f32, 300.0_f32)
    } else {
        (f32::NEG_INFINITY, f32::INFINITY)
    };
    let no_data = metadata.no_data_value.map(|v| v as f32);

    for value in &mut data {
        if let Some(nd) = no_data {
            if (*value - nd).abs() < f32::EPSILON || *value == nd {
                *value = f32::NAN;
                continue;
            }
        }
        if !value.is_nan() && (*value < min_valid || *value > max_valid) {
            *value = f32::NAN;
        }
    }

    // Create Zarr array and write all at once
    let store = FilesystemStore::new(zarr_path)
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create store: {}", e)))?;

    // Use Blosc compression (same as other ingestion pipelines)
    // BloscCodec::new(compressor, level, blocksize, shuffle, typesize)
    // typesize=4 for f32 data when shuffle is enabled
    let blosc_codec = BloscCodec::new(
        BloscCompressor::Zstd,
        BloscCompressionLevel::try_from(5).unwrap(),
        None, // auto blocksize
        BloscShuffleMode::Shuffle,
        Some(4), // f32 = 4 bytes
    )
    .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create Blosc codec: {}", e)))?;

    let array = ArrayBuilder::new(
        vec![height as u64, width as u64],
        DataType::Float32,
        vec![ZARR_CHUNK_SIZE, ZARR_CHUNK_SIZE].try_into().unwrap(),
        FillValue::from(f32::NAN),
    )
    .bytes_to_bytes_codecs(vec![Arc::new(blosc_codec)])
    .build(Arc::new(store), "/0") // Write as level 0 of pyramid (matching grid-processor expectation)
    .map_err(|e| IngestionError::ZarrWrite(format!("Failed to create array: {}", e)))?;

    // Write all data
    array
        .store_array_subset_elements::<f32>(
            &ArraySubset::new_with_ranges(&[0..height as u64, 0..width as u64]),
            &data,
        )
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to write array: {}", e)))?;

    array
        .store_metadata()
        .map_err(|e| IngestionError::ZarrWrite(format!("Failed to store metadata: {}", e)))?;

    Ok(())
}

/// Build Zarr metadata JSON for the catalog.
fn build_zarr_metadata(
    metadata: &GeoTiffMetadata,
    model: &str,
    param: &str,
    reference_time: DateTime<Utc>,
) -> serde_json::Value {
    let width = metadata.width as usize;
    let height = metadata.height as usize;
    let chunk_size = ZARR_CHUNK_SIZE as usize;
    let num_chunks_x = (width + chunk_size - 1) / chunk_size;
    let num_chunks_y = (height + chunk_size - 1) / chunk_size;

    // Build metadata compatible with ZarrMetadata struct in grid-processor
    // ZarrMetadata.shape is (width, height), so we output [width, height]
    // The Zarr array itself stores data as [rows, cols] = [height, width] (row-major)
    serde_json::json!({
        "shape": [width, height],
        "chunk_shape": [chunk_size, chunk_size],
        "num_chunks": [num_chunks_x, num_chunks_y],
        "dtype": "float32",
        "bbox": {
            "min_lon": metadata.bbox.min_x,
            "min_lat": metadata.bbox.min_y,
            "max_lon": metadata.bbox.max_x,
            "max_lat": metadata.bbox.max_y
        },
        "compression": "blosc",
        "model": model,
        "parameter": param,
        "level": "surface",
        "units": get_units_for_param(model, param),
        "reference_time": reference_time.to_rfc3339(),
        "forecast_hour": 0,
        "projection": "geographic",
        "row_origin": "North"
    })
}

/// Extract model name from GeoTIFF file path.
fn extract_model_from_geotiff_path(file_path: &str) -> String {
    let lower = file_path.to_lowercase();

    if lower.contains("viirs") || lower.contains("vnl") || lower.contains("nighttime") {
        "viirs".to_string()
    } else {
        "geotiff".to_string()
    }
}

/// Determine parameter name from model and file path.
fn determine_parameter(model: &str, file_path: &str) -> String {
    let lower = file_path.to_lowercase();

    if model == "viirs" {
        if lower.contains("median") {
            "radiance_median".to_string()
        } else if lower.contains("average") || lower.contains("avg") {
            "radiance_average".to_string()
        } else {
            "radiance".to_string()
        }
    } else {
        "value".to_string()
    }
}

/// Get units for a parameter.
fn get_units_for_param(model: &str, param: &str) -> &'static str {
    if model == "viirs" && param.starts_with("radiance") {
        "nW/cm^2/sr"
    } else {
        "unknown"
    }
}

/// Build storage path for static datasets.
fn build_storage_path(model: &str, param: &str) -> String {
    format!("grids/{}/{}.zarr", model, param.to_lowercase())
}

/// Extract metadata from GeoTIFF tags.
fn extract_geotiff_metadata<R: Read + std::io::Seek>(
    decoder: &mut Decoder<R>,
) -> Result<GeoTiffMetadata> {
    let (width, height) = decoder.dimensions().map_err(|e| {
        IngestionError::GeoTiffParse(format!("Failed to get TIFF dimensions: {}", e))
    })?;

    let pixel_scale = extract_pixel_scale(decoder)?;
    let bbox = extract_bounding_box(decoder, width, height, &pixel_scale)?;
    let no_data_value = extract_no_data_value(decoder);

    Ok(GeoTiffMetadata {
        width,
        height,
        bbox,
        pixel_scale,
        no_data_value,
    })
}

/// Extract pixel scale from ModelPixelScaleTag (33550).
fn extract_pixel_scale<R: Read + std::io::Seek>(decoder: &mut Decoder<R>) -> Result<(f64, f64)> {
    match decoder.get_tag_f64_vec(Tag::Unknown(GEOTIFF_MODEL_PIXEL_SCALE_TAG)) {
        Ok(scale) if scale.len() >= 2 => Ok((scale[0], scale[1])),
        _ => {
            warn!("ModelPixelScaleTag not found, using default VIIRS resolution (15 arc-sec)");
            Ok((15.0 / 3600.0, 15.0 / 3600.0))
        }
    }
}

/// Extract bounding box from ModelTiepointTag (33922) and pixel scale.
fn extract_bounding_box<R: Read + std::io::Seek>(
    decoder: &mut Decoder<R>,
    width: u32,
    height: u32,
    pixel_scale: &(f64, f64),
) -> Result<BoundingBox> {
    match decoder.get_tag_f64_vec(Tag::Unknown(GEOTIFF_MODEL_TIEPOINT_TAG)) {
        Ok(tiepoint) if tiepoint.len() >= 6 => {
            let pixel_i = tiepoint[0];
            let pixel_j = tiepoint[1];
            let geo_x = tiepoint[3];
            let geo_y = tiepoint[4];

            let min_lon = geo_x - (pixel_i * pixel_scale.0);
            let max_lat = geo_y + (pixel_j * pixel_scale.1);
            let max_lon = min_lon + (width as f64 * pixel_scale.0);
            let min_lat = max_lat - (height as f64 * pixel_scale.1);

            Ok(BoundingBox::new(min_lon, min_lat, max_lon, max_lat))
        }
        _ => {
            warn!("ModelTiepointTag not found, using default global coverage");
            Ok(BoundingBox::new(-180.0, -65.0, 180.0, 75.0))
        }
    }
}

/// Extract NoData value from GDAL_NODATA tag (42113).
fn extract_no_data_value<R: Read + std::io::Seek>(decoder: &mut Decoder<R>) -> Option<f64> {
    match decoder.get_tag_ascii_string(Tag::Unknown(GDAL_NODATA_TAG)) {
        Ok(s) => s.trim().parse::<f64>().ok(),
        Err(_) => None,
    }
}

/// Convert a decoded chunk to f32 values.
fn decode_chunk_to_f32(result: DecodingResult) -> Vec<f32> {
    match result {
        DecodingResult::U8(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::U16(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::U32(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::U64(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::I8(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::I16(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::I32(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::I64(data) => data.into_iter().map(|v| v as f32).collect(),
        DecodingResult::F32(data) => data,
        DecodingResult::F64(data) => data.into_iter().map(|v| v as f32).collect(),
    }
}

/// Decompress gzip-compressed data.
fn decompress_gzip(data: &Bytes) -> Result<Bytes> {
    let mut decoder = GzDecoder::new(data.as_ref());
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| IngestionError::Decompression(e.to_string()))?;
    Ok(Bytes::from(decompressed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_model_from_geotiff_path() {
        assert_eq!(
            extract_model_from_geotiff_path("VNL_v22_npp_2023_global.tif"),
            "viirs"
        );
        assert_eq!(
            extract_model_from_geotiff_path("/data/viirs/nighttime_lights.tiff"),
            "viirs"
        );
        assert_eq!(
            extract_model_from_geotiff_path("some_other_data.tif"),
            "geotiff"
        );
    }

    #[test]
    fn test_determine_parameter() {
        assert_eq!(
            determine_parameter("viirs", "VNL_median_masked.tif"),
            "radiance_median"
        );
        assert_eq!(
            determine_parameter("viirs", "VNL_average_masked.tif"),
            "radiance_average"
        );
        assert_eq!(determine_parameter("viirs", "VNL_data.tif"), "radiance");
        assert_eq!(determine_parameter("other", "data.tif"), "value");
    }

    #[test]
    fn test_get_units_for_param() {
        assert_eq!(get_units_for_param("viirs", "radiance"), "nW/cm^2/sr");
        assert_eq!(
            get_units_for_param("viirs", "radiance_median"),
            "nW/cm^2/sr"
        );
        assert_eq!(get_units_for_param("other", "value"), "unknown");
    }

    #[test]
    fn test_build_storage_path() {
        assert_eq!(
            build_storage_path("viirs", "radiance"),
            "grids/viirs/radiance.zarr"
        );
        assert_eq!(
            build_storage_path("viirs", "radiance_median"),
            "grids/viirs/radiance_median.zarr"
        );
    }
}
