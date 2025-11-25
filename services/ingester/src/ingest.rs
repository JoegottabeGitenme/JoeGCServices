//! Ingestion pipeline for processing weather data.

use anyhow::{anyhow, Result};
use bytes::Bytes;
use chrono::{Duration, Utc};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, instrument, warn};

use storage::{Catalog, CatalogEntry, ObjectStorage, ObjectStorageConfig};
use wms_common::BoundingBox;

use crate::config::{IngesterConfig, ModelConfig, ParameterConfig};
use crate::sources::{
    create_fetcher, cycles_to_check, latest_available_cycle, DataSourceFetcher, RemoteFile,
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
        for (model_id, model_config) in &self.config.models {
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
                let date = date.to_string();
                let file_pattern = model_config.source.file_pattern(
                    &model_id,
                    &model_config.resolution,
                    cycle,
                    fhr,
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
            file.path.split('/').last().unwrap_or(&file.path)
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

        // Parse and extract parameters
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

        Ok(())
    }

    /// Extract a parameter from GRIB2 data.
    #[instrument(skip(self, data), fields(parameter = %param_config.name))]
    async fn extract_parameter(
        &self,
        model_id: &str,
        date: &str,
        cycle: u32,
        fhr: u32,
        data: &Bytes,
        param_config: &ParameterConfig,
        storage_path: &str,
        file_size: u64,
    ) -> Result<()> {
        // Parse reference time
        let reference_time = chrono::NaiveDate::parse_from_str(date, "%Y%m%d")?
            .and_hms_opt(cycle, 0, 0)
            .ok_or_else(|| anyhow!("Invalid time"))?;
        let reference_time = chrono::Utc.from_utc_datetime(&reference_time);

        // Parse GRIB2 file and find matching parameter
        let mut reader = grib2_parser::Grib2Reader::new(data.clone());
        let mut found_matching_message = false;

        while let Some(message) = reader.next_message().ok().flatten() {
            // Check if this message matches the parameter we're looking for
            if message.product_definition.parameter_short_name == param_config.grib_filter.parameter
                && message.product_definition.level_description.contains(&param_config.grib_filter.level)
            {
                found_matching_message = true;
                
                debug!(
                    "Found matching parameter message: {} at level {}",
                    param_config.grib_filter.parameter,
                    param_config.grib_filter.level
                );

                // For now, just register the raw file in the catalog
                // Full data extraction and unpacking can be added later
            }
        }

        if found_matching_message {
            // Create catalog entry for this parameter
            let entry = CatalogEntry {
                model: model_id.to_string(),
                parameter: param_config.name.clone(),
                level: param_config.grib_filter.level.clone(),
                reference_time,
                forecast_hour: fhr,
                bbox: get_model_bbox(model_id),
                storage_path: storage_path.to_string(),
                file_size,
            };

            self.catalog.register_dataset(&entry).await?;
            info!("Registered parameter in catalog");
        } else {
            debug!(
                "Parameter {} not found in GRIB2 file at level {}",
                param_config.grib_filter.parameter, param_config.grib_filter.level
            );
        }

        Ok(())
    }

    /// Clean up old data.
    async fn cleanup_old_data(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::hours(self.config.retention_hours as i64);

        let expired = self.catalog.mark_expired(cutoff).await?;
        if expired > 0 {
            info!(count = expired, "Marked expired datasets");
        }

        // TODO: Actually delete files from object storage for expired datasets

        Ok(())
    }
}

/// Get the default bounding box for a model.
fn get_model_bbox(model_id: &str) -> BoundingBox {
    match model_id {
        "gfs" => BoundingBox::new(0.0, -90.0, 360.0, 90.0),
        "hrrr" => BoundingBox::new(-134.1, 21.1, -60.9, 52.6),
        "nam" => BoundingBox::new(-152.9, 12.2, -49.4, 57.3),
        _ => BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
    }
}

// Re-export chrono::TimeZone for use in from_utc_datetime
use chrono::TimeZone;
