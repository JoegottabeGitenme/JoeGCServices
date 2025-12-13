//! Ingestion pipeline for processing weather data.

use anyhow::{anyhow, Result};
use bytes::Bytes;
use chrono::{Duration, Utc};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, instrument, warn};

use grid_processor::{GridProcessorConfig, ZarrWriter};
use storage::{Catalog, CatalogEntry, ObjectStorage};
use wms_common::BoundingBox;
use zarrs_filesystem::FilesystemStore;

use crate::config::{IngesterConfig, ModelConfig, ParameterConfig};
use crate::sources::{
    create_fetcher, cycles_to_check, DataSourceFetcher, RemoteFile,
};

/// Main ingestion pipeline.
pub struct IngestionPipeline {
    config: IngesterConfig,
    storage: ObjectStorage,
    catalog: Catalog,
    download_semaphore: Arc<Semaphore>,
}

impl IngestionPipeline {
    /// Create a new ingestion pipeline.
    pub async fn new(config: &IngesterConfig) -> Result<Self> {
        let storage = ObjectStorage::new(&config.storage)?;
        let catalog = Catalog::connect(&config.database_url).await?;

        // Run migrations
        catalog.migrate().await?;

        let download_semaphore = Arc::new(Semaphore::new(config.parallel_downloads));

        Ok(Self {
            config: config.clone(),
            storage,
            catalog,
            download_semaphore,
        })
    }

    /// Run the ingestion pipeline forever.
    pub async fn run_forever(&self) -> Result<()> {
        loop {
            info!("Starting ingestion cycle");

            if let Err(e) = self.ingest_all().await {
                error!(error = %e, "Ingestion cycle failed");
            }

            // Clean up old data
            if let Err(e) = self.cleanup_old_data().await {
                warn!(error = %e, "Cleanup failed");
            }

            info!(
                interval_secs = self.config.poll_interval_secs,
                "Sleeping until next cycle"
            );
            tokio::time::sleep(std::time::Duration::from_secs(
                self.config.poll_interval_secs,
            ))
            .await;
        }
    }

    /// Ingest all configured models.
    pub async fn ingest_all(&self) -> Result<()> {
        for model_id in self.config.models.keys() {
            if let Err(e) = self.ingest_model(model_id).await {
                error!(model = %model_id, error = %e, "Model ingestion failed");
            }
        }
        Ok(())
    }

    /// Ingest a specific model.
    #[instrument(skip(self), fields(model = %model_id))]
    pub async fn ingest_model(&self, model_id: &str) -> Result<()> {
        let model_config = self
            .config
            .models
            .get(model_id)
            .ok_or_else(|| anyhow!("Unknown model: {}", model_id))?;

        info!(model = %model_config.name, "Starting model ingestion");

        let fetcher = create_fetcher(&model_config.source, model_id, &model_config.resolution);

        // Get cycles to check (look back 24 hours)
        let cycles = cycles_to_check(&model_config.cycles, 24);

        for (date, cycle) in cycles {
            if let Err(e) = self
                .ingest_cycle(model_id, model_config, &*fetcher, &date, cycle)
                .await
            {
                warn!(
                    model = %model_id,
                    date = %date,
                    cycle = cycle,
                    error = %e,
                    "Cycle ingestion failed"
                );
            }
        }

        Ok(())
    }

    /// Ingest a specific model cycle.
    #[instrument(skip(self, model_config, fetcher), fields(model = %model_id, date = %date, cycle = cycle))]
    async fn ingest_cycle(
        &self,
        model_id: &str,
        model_config: &ModelConfig,
        fetcher: &dyn DataSourceFetcher,
        date: &str,
        cycle: u32,
    ) -> Result<()> {
        info!("Checking cycle for new data");

        // List available files
        let available_files = fetcher.list_files(date, cycle).await?;

        if available_files.is_empty() {
            debug!("No files found for this cycle");
            return Ok(());
        }

        // Process each forecast hour
        let tasks: Vec<_> = model_config
            .forecast_hours
            .iter()
            .map(|&fhr| {
                let sem = self.download_semaphore.clone();
                let model_id = model_id.to_string();
                let _date = date.to_string();
                let file_pattern = model_config.source.file_pattern(
                    &model_id,
                    &model_config.resolution,
                    cycle,
                    fhr,
                    model_config.file_pattern.as_deref(),
                );

                async move {
                    let _permit = sem.acquire().await?;
                    // Return file pattern for matching
                    Ok::<_, anyhow::Error>((fhr, file_pattern))
                }
            })
            .collect();

        let file_patterns: Vec<_> = stream::iter(tasks)
            .buffer_unordered(self.config.parallel_downloads)
            .filter_map(|r| async { r.ok() })
            .collect()
            .await;

        // Match and download files
        for (fhr, pattern) in file_patterns {
            let matching_file = available_files.iter().find(|f| f.path.contains(&pattern));

            if let Some(file) = matching_file {
                if let Err(e) = self
                    .process_file(model_id, model_config, fetcher, file, date, cycle, fhr)
                    .await
                {
                    warn!(
                        file = %file.path,
                        error = %e,
                        "File processing failed"
                    );
                }
            }
        }

        Ok(())
    }

    /// Process and store a single file.
    #[instrument(skip(self, model_config, fetcher, file), fields(path = %file.path))]
    async fn process_file(
        &self,
        model_id: &str,
        model_config: &ModelConfig,
        fetcher: &dyn DataSourceFetcher,
        file: &RemoteFile,
        date: &str,
        cycle: u32,
        fhr: u32,
    ) -> Result<()> {
        // Check if already ingested
        let storage_path = format!(
            "raw/{}/{}/{:02}/{}",
            model_id,
            date,
            cycle,
            file.path.split('/').next_back().unwrap_or(&file.path)
        );

        if self.storage.exists(&storage_path).await? {
            debug!("File already ingested, skipping");
            return Ok(());
        }

        // Download file
        info!("Downloading file");
        let data = fetcher.fetch_file(file).await?;
        let file_size = data.len() as u64;

        // Store raw file
        self.storage.put(&storage_path, data.clone()).await?;
        info!(size = file_size, "Stored raw file");

        // Check if this is GOES data (NetCDF) or GRIB2
        let is_goes = model_config.source.is_goes();

        if is_goes {
            // Process GOES NetCDF file
            if let Err(e) = self
                .extract_goes_parameter(
                    model_id,
                    date,
                    cycle,
                    file,
                    &storage_path,
                    file_size,
                )
                .await
            {
                warn!(
                    error = %e,
                    "GOES parameter extraction failed"
                );
            }
        } else {
            // Parse and extract parameters from GRIB2
            for param_config in &model_config.parameters {
                if let Err(e) = self
                    .extract_parameter(
                        model_id,
                        date,
                        cycle,
                        fhr,
                        &data,
                        param_config,
                        &storage_path,
                        file_size,
                    )
                    .await
                {
                    warn!(
                        parameter = %param_config.name,
                        error = %e,
                        "Parameter extraction failed"
                    );
                }
            }
        }

        Ok(())
    }

    /// Extract parameter from GOES NetCDF file.
    /// 
    /// GOES files are self-describing - the filename contains the band and timestamp info.
    /// Format: OR_ABI-L2-CMIPC-M6C{band:02}_G{sat}_s{start}_e{end}_c{created}.nc
    #[instrument(skip(self), fields(path = %file.path))]
    async fn extract_goes_parameter(
        &self,
        model_id: &str,
        date: &str,
        cycle: u32,
        file: &RemoteFile,
        storage_path: &str,
        file_size: u64,
    ) -> Result<()> {
        // Parse filename to extract band and time info
        let filename = file.path.split('/').next_back().unwrap_or(&file.path);
        
        // Extract band number from filename (e.g., "C02" from "...M6C02_G16...")
        let band = filename
            .find("M6C")
            .or_else(|| filename.find("M3C"))
            .and_then(|pos| {
                let band_str = &filename[pos + 3..pos + 5];
                band_str.parse::<u8>().ok()
            })
            .unwrap_or(0);

        // Extract satellite ID (G16 or G18)
        let satellite = if filename.contains("_G16_") {
            "G16"
        } else if filename.contains("_G18_") {
            "G18"
        } else {
            "GOES"
        };

        // Extract observation time from filename (format: s20250500001170)
        // Time is in format: YYYYDDDHHMMSSt (year, day-of-year, hour, min, sec, tenths)
        let observation_time = filename
            .find("_s")
            .and_then(|pos| {
                let time_str = &filename[pos + 2..pos + 15];
                // Parse YYYYDDDHHMMSS
                let year: i32 = time_str[0..4].parse().ok()?;
                let doy: u32 = time_str[4..7].parse().ok()?;
                let hour: u32 = time_str[7..9].parse().ok()?;
                let min: u32 = time_str[9..11].parse().ok()?;
                let sec: u32 = time_str[11..13].parse().ok()?;
                
                // Convert to DateTime
                let date = chrono::NaiveDate::from_yo_opt(year, doy)?;
                let time = chrono::NaiveTime::from_hms_opt(hour, min, sec)?;
                Some(chrono::Utc.from_utc_datetime(&date.and_time(time)))
            })
            .unwrap_or_else(|| {
                // Fallback: use date and cycle
                let date = chrono::NaiveDate::parse_from_str(date, "%Y%m%d")
                    .unwrap_or_else(|_| chrono::Utc::now().date_naive());
                let time = chrono::NaiveTime::from_hms_opt(cycle, 0, 0).unwrap();
                chrono::Utc.from_utc_datetime(&date.and_time(time))
            });

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
            _ => ("CMI", "unknown"),
        };

        info!(
            band = band,
            satellite = satellite,
            parameter = parameter,
            observation_time = %observation_time,
            "Processing GOES file"
        );

        // Create catalog entry
        // For GOES, forecast_hour is 0 (observational data)
        let entry = CatalogEntry {
            model: model_id.to_string(),
            parameter: parameter.to_string(),
            level: level.to_string(),
            reference_time: observation_time,
            forecast_hour: 0, // Observational data
            bbox: get_model_bbox(model_id),
            storage_path: storage_path.to_string(),
            file_size,
            zarr_metadata: None,
        };

        self.catalog.register_dataset(&entry).await?;
        info!(parameter = parameter, band = band, "Registered GOES parameter in catalog");

        Ok(())
    }

    /// Extract a parameter from GRIB2 data and write as Zarr.
    #[instrument(skip(self, data), fields(parameter = %param_config.name))]
    async fn extract_parameter(
        &self,
        model_id: &str,
        date: &str,
        cycle: u32,
        fhr: u32,
        data: &Bytes,
        param_config: &ParameterConfig,
        _storage_path: &str,
        _file_size: u64,
    ) -> Result<()> {
        // Parse reference time
        let reference_time = chrono::NaiveDate::parse_from_str(date, "%Y%m%d")?
            .and_hms_opt(cycle, 0, 0)
            .ok_or_else(|| anyhow!("Invalid time"))?;
        let reference_time = chrono::Utc.from_utc_datetime(&reference_time);

        // Parse GRIB2 file and find matching parameter
        let mut reader = grib2_parser::Grib2Reader::new(data.clone());

        while let Some(message) = reader.next_message().ok().flatten() {
            // Check if this message matches the parameter we're looking for
            if message.product_definition.parameter_short_name == param_config.grib_filter.parameter
                && message.product_definition.level_description.contains(&param_config.grib_filter.level)
            {
                debug!(
                    "Found matching parameter message: {} at level {}",
                    param_config.grib_filter.parameter,
                    param_config.grib_filter.level
                );

                // Extract grid dimensions
                let width = message.grid_definition.num_points_longitude as usize;
                let height = message.grid_definition.num_points_latitude as usize;
                
                // Unpack the grid data
                let grid_data = match message.unpack_data() {
                    Ok(data) => data,
                    Err(e) => {
                        warn!(error = %e, "Failed to unpack GRIB2 data, skipping");
                        continue;
                    }
                };
                
                if grid_data.len() != width * height {
                    warn!(
                        expected = width * height,
                        actual = grid_data.len(),
                        "Grid data size mismatch, skipping"
                    );
                    continue;
                }
                
                // Calculate bounding box from grid definition
                let bbox = get_bbox_from_grid(&message.grid_definition);
                
                // Create Zarr storage path: grids/{model}/{date}/{param}_f{fhr:03}.zarr
                let zarr_storage_path = format!(
                    "grids/{}/{}/{:02}/{}_f{:03}.zarr",
                    model_id, date, cycle, param_config.name, fhr
                );
                
                // Create a temporary directory for Zarr output
                let temp_dir = tempfile::tempdir()?;
                let zarr_path = temp_dir.path().join("grid.zarr");
                std::fs::create_dir_all(&zarr_path)?;
                
                // Create Zarr writer with default config
                let config = GridProcessorConfig::default();
                let writer = ZarrWriter::new(config);
                
                // Create filesystem store for the temp directory
                let store = FilesystemStore::new(&zarr_path)
                    .map_err(|e| anyhow!("Failed to create filesystem store: {}", e))?;
                
                // Get units from parameter config or use default
                let units = param_config.units.as_deref().unwrap_or("unknown");
                
                // Convert grid-processor BoundingBox to match the writer's expected type
                let gp_bbox = grid_processor::BoundingBox::new(
                    bbox.min_x, bbox.min_y, bbox.max_x, bbox.max_y
                );
                
                // Write Zarr data using sharded format (single file)
                let write_result = writer.write_sharded(
                    store,
                    "/",
                    &grid_data,
                    width,
                    height,
                    &gp_bbox,
                    model_id,
                    &param_config.name,
                    &param_config.grib_filter.level,
                    units,
                    reference_time,
                    fhr,
                ).map_err(|e| anyhow!("Failed to write Zarr: {}", e))?;
                
                info!(
                    path = %zarr_storage_path,
                    width = width,
                    height = height,
                    bytes = write_result.bytes_written,
                    "Wrote Zarr grid"
                );
                
                // Upload Zarr files to object storage
                let zarr_file_size = self.upload_zarr_directory(&zarr_path, &zarr_storage_path).await?;
                
                // Create catalog entry with Zarr metadata
                let entry = CatalogEntry {
                    model: model_id.to_string(),
                    parameter: param_config.name.clone(),
                    level: param_config.grib_filter.level.clone(),
                    reference_time,
                    forecast_hour: fhr,
                    bbox,
                    storage_path: zarr_storage_path.clone(),
                    file_size: zarr_file_size,
                    zarr_metadata: Some(write_result.metadata.to_json()),
                };

                self.catalog.register_dataset(&entry).await?;
                info!(path = %zarr_storage_path, "Registered Zarr dataset in catalog");
                
                // Only process the first matching message
                return Ok(());
            }
        }

        debug!(
            "Parameter {} not found in GRIB2 file at level {}",
            param_config.grib_filter.parameter, param_config.grib_filter.level
        );

        Ok(())
    }
    
    /// Upload a Zarr directory to object storage.
    async fn upload_zarr_directory(&self, local_path: &std::path::Path, storage_prefix: &str) -> Result<u64> {
        let mut total_size = 0u64;
        
        for entry in walkdir::WalkDir::new(local_path) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let relative_path = entry.path().strip_prefix(local_path)?;
                let storage_path = format!("{}/{}", storage_prefix, relative_path.display());
                
                let file_data = tokio::fs::read(entry.path()).await?;
                let file_size = file_data.len() as u64;
                total_size += file_size;
                
                self.storage.put(&storage_path, Bytes::from(file_data)).await?;
                debug!(path = %storage_path, size = file_size, "Uploaded Zarr file");
            }
        }
        
        Ok(total_size)
    }

    /// Clean up old data.
    async fn cleanup_old_data(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::hours(self.config.retention_hours as i64);

        let expired = self.catalog.mark_expired(cutoff).await?;
        if expired > 0 {
            info!(count = expired, "Marked expired datasets");

            // Get storage paths for expired datasets
            let paths = self.catalog.get_expired_storage_paths().await?;
            if !paths.is_empty() {
                info!(count = paths.len(), "Deleting expired files from object storage");

                // Delete files from object storage
                let mut deleted = 0;
                let mut failed = 0;
                for path in paths {
                    match self.storage.delete(&path).await {
                        Ok(_) => {
                            debug!(path = %path, "Deleted expired file");
                            deleted += 1;
                        }
                        Err(e) => {
                            warn!(path = %path, error = %e, "Failed to delete expired file");
                            failed += 1;
                        }
                    }
                }

                info!(
                    deleted = deleted,
                    failed = failed,
                    "Completed deletion of expired files"
                );

                // Delete expired dataset records from database
                let deleted_records = self.catalog.delete_expired().await?;
                info!(count = deleted_records, "Deleted expired dataset records from catalog");
            }
        }

        Ok(())
    }
}

/// Get the default bounding box for a model.
fn get_model_bbox(model_id: &str) -> BoundingBox {
    match model_id {
        "gfs" => BoundingBox::new(0.0, -90.0, 360.0, 90.0),
        "hrrr" => BoundingBox::new(-134.1, 21.1, -60.9, 52.6),
        "nam" => BoundingBox::new(-152.9, 12.2, -49.4, 57.3),
        // MRMS CONUS domain (regular lat-lon 0.01 degree, ~1km)
        // Grid: 7000 x 3500 points
        // lat 54.995 to 20.005 by 0.01
        // lon 230.005 to 299.995 by 0.01 (= -129.995 to -60.005)
        "mrms" => BoundingBox::new(-130.0, 20.0, -60.0, 55.0),
        // GOES-16 CONUS bounds (approximate)
        "goes16" => BoundingBox::new(-143.0, 14.5, -53.0, 55.5),
        // GOES-18 CONUS bounds (approximate)  
        "goes18" => BoundingBox::new(-165.0, 14.5, -90.0, 55.5),
        _ => BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
    }
}

/// Extract bounding box from GRIB2 grid definition.
fn get_bbox_from_grid(grid: &grib2_parser::sections::GridDefinition) -> BoundingBox {
    // Convert millidegrees to degrees
    let first_lat = grid.first_latitude_millidegrees as f64 / 1_000_000.0;
    let first_lon = grid.first_longitude_millidegrees as f64 / 1_000_000.0;
    let last_lat = grid.last_latitude_millidegrees as f64 / 1_000_000.0;
    let last_lon = grid.last_longitude_millidegrees as f64 / 1_000_000.0;
    
    // Determine min/max (grid might scan in different directions)
    let min_lat = first_lat.min(last_lat);
    let max_lat = first_lat.max(last_lat);
    let min_lon = first_lon.min(last_lon);
    let max_lon = first_lon.max(last_lon);
    
    BoundingBox::new(min_lon, min_lat, max_lon, max_lat)
}

// Re-export chrono::TimeZone for use in from_utc_datetime
use chrono::TimeZone;
