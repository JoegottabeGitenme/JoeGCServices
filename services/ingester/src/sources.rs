//! Data source implementations for fetching weather data.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use reqwest::Client;
use tracing::{debug, info, instrument};

use crate::config::DataSource;

/// Trait for data sources that can list and fetch files.
#[async_trait]
pub trait DataSourceFetcher: Send + Sync {
    /// List available files for a date/cycle.
    async fn list_files(&self, date: &str, cycle: u32) -> Result<Vec<RemoteFile>>;

    /// Download a specific file.
    async fn fetch_file(&self, file: &RemoteFile) -> Result<Bytes>;

    /// Check if a file exists.
    #[allow(dead_code)]
    async fn file_exists(&self, file: &RemoteFile) -> Result<bool>;
}

/// Information about a remote file.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub path: String,
    pub size: Option<u64>,
    pub last_modified: Option<DateTime<Utc>>,
}

/// AWS S3 data source fetcher (for NOAA Open Data).
#[allow(dead_code)]
pub struct AwsDataSource {
    client: Client,
    bucket: String,
    prefix_template: String,
    model: String,
    resolution: String,
}

impl AwsDataSource {
    pub fn new(bucket: String, prefix_template: String, model: String, resolution: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            bucket,
            prefix_template,
            model,
            resolution,
        }
    }

    fn build_s3_url(&self, path: &str) -> String {
        format!("https://{}.s3.amazonaws.com/{}", self.bucket, path)
    }
}

#[async_trait]
impl DataSourceFetcher for AwsDataSource {
    #[instrument(skip(self), fields(bucket = %self.bucket))]
    async fn list_files(&self, date: &str, cycle: u32) -> Result<Vec<RemoteFile>> {
        let prefix = self
            .prefix_template
            .replace("{date}", date)
            .replace("{cycle}", &format!("{:02}", cycle));

        // Use S3 list API via HTTP
        let list_url = format!(
            "https://{}.s3.amazonaws.com/?list-type=2&prefix={}",
            self.bucket, prefix
        );

        debug!(url = %list_url, "Listing S3 bucket");

        let response = self.client.get(&list_url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("S3 list failed: {}", response.status()));
        }

        let body = response.text().await?;

        // Simple XML parsing for S3 ListObjectsV2 response
        let mut files = Vec::new();
        for key_match in body.split("<Key>").skip(1) {
            if let Some(end) = key_match.find("</Key>") {
                let key = &key_match[..end];
                files.push(RemoteFile {
                    path: key.to_string(),
                    size: None,
                    last_modified: None,
                });
            }
        }

        info!(count = files.len(), prefix = %prefix, "Listed files");
        Ok(files)
    }

    #[instrument(skip(self), fields(path = %file.path))]
    async fn fetch_file(&self, file: &RemoteFile) -> Result<Bytes> {
        let url = self.build_s3_url(&file.path);

        debug!(url = %url, "Downloading file");

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Download failed: {}", response.status()));
        }

        let bytes = response.bytes().await?;
        info!(size = bytes.len(), path = %file.path, "Downloaded file");

        Ok(bytes)
    }

    async fn file_exists(&self, file: &RemoteFile) -> Result<bool> {
        let url = self.build_s3_url(&file.path);

        let response = self.client.head(&url).send().await?;

        Ok(response.status().is_success())
    }
}

/// NOMADS HTTP data source fetcher.
pub struct NomadsDataSource {
    client: Client,
    base_url: String,
    path_template: String,
}

impl NomadsDataSource {
    pub fn new(base_url: String, path_template: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url,
            path_template,
        }
    }
}

#[async_trait]
impl DataSourceFetcher for NomadsDataSource {
    async fn list_files(&self, date: &str, cycle: u32) -> Result<Vec<RemoteFile>> {
        let path = self
            .path_template
            .replace("{date}", date)
            .replace("{cycle}", &format!("{:02}", cycle));

        let url = format!("{}/{}", self.base_url, path);

        // NOMADS directory listing
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("NOMADS list failed: {}", response.status()));
        }

        let body = response.text().await?;

        // Parse HTML directory listing (basic implementation)
        let mut files = Vec::new();
        for line in body.lines() {
            if line.contains("href=") && line.contains(".grib2") {
                if let Some(start) = line.find("href=\"") {
                    let rest = &line[start + 6..];
                    if let Some(end) = rest.find('"') {
                        let filename = &rest[..end];
                        files.push(RemoteFile {
                            path: format!("{}/{}", path, filename),
                            size: None,
                            last_modified: None,
                        });
                    }
                }
            }
        }

        Ok(files)
    }

    async fn fetch_file(&self, file: &RemoteFile) -> Result<Bytes> {
        let url = format!("{}/{}", self.base_url, file.path);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("NOMADS download failed: {}", response.status()));
        }

        Ok(response.bytes().await?)
    }

    async fn file_exists(&self, file: &RemoteFile) -> Result<bool> {
        let url = format!("{}/{}", self.base_url, file.path);

        let response = self.client.head(&url).send().await?;

        Ok(response.status().is_success())
    }
}

/// GOES satellite data source fetcher (NetCDF on AWS).
pub struct GoesDataSource {
    client: Client,
    bucket: String,
    product: String,
    bands: Vec<u8>,
}

impl GoesDataSource {
    pub fn new(bucket: String, product: String, bands: Vec<u8>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            bucket,
            product,
            bands,
        }
    }

    fn build_s3_url(&self, path: &str) -> String {
        format!("https://{}.s3.amazonaws.com/{}", self.bucket, path)
    }
}

#[async_trait]
impl DataSourceFetcher for GoesDataSource {
    #[instrument(skip(self), fields(bucket = %self.bucket, product = %self.product))]
    async fn list_files(&self, date: &str, cycle: u32) -> Result<Vec<RemoteFile>> {
        // GOES uses year/day_of_year/hour structure
        // Convert date (YYYYMMDD) to year/doy
        let parsed = chrono::NaiveDate::parse_from_str(date, "%Y%m%d")
            .map_err(|e| anyhow!("Invalid date format: {}", e))?;
        let year = parsed.year();
        let doy = parsed.ordinal();

        let prefix = format!("{}/{}/{:03}/{:02}/", self.product, year, doy, cycle);

        // Use S3 list API via HTTP
        let list_url = format!(
            "https://{}.s3.amazonaws.com/?list-type=2&prefix={}",
            self.bucket, prefix
        );

        debug!(url = %list_url, "Listing GOES S3 bucket");

        let response = self.client.get(&list_url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("S3 list failed: {}", response.status()));
        }

        let body = response.text().await?;

        // Parse XML and filter by bands
        let mut files = Vec::new();
        for key_match in body.split("<Key>").skip(1) {
            if let Some(end) = key_match.find("</Key>") {
                let key = &key_match[..end];
                
                // Filter by band - GOES files have pattern like "...C{band:02}_G16..."
                let should_include = self.bands.iter().any(|&band| {
                    let pattern = format!("C{:02}_", band);
                    key.contains(&pattern)
                });

                if should_include && key.ends_with(".nc") {
                    files.push(RemoteFile {
                        path: key.to_string(),
                        size: None,
                        last_modified: None,
                    });
                }
            }
        }

        info!(count = files.len(), prefix = %prefix, "Listed GOES files");
        Ok(files)
    }

    #[instrument(skip(self), fields(path = %file.path))]
    async fn fetch_file(&self, file: &RemoteFile) -> Result<Bytes> {
        let url = self.build_s3_url(&file.path);

        debug!(url = %url, "Downloading GOES file");

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Download failed: {}", response.status()));
        }

        let bytes = response.bytes().await?;
        info!(size = bytes.len(), path = %file.path, "Downloaded GOES file");

        Ok(bytes)
    }

    async fn file_exists(&self, file: &RemoteFile) -> Result<bool> {
        let url = self.build_s3_url(&file.path);

        let response = self.client.head(&url).send().await?;

        Ok(response.status().is_success())
    }
}

/// Create appropriate fetcher for a data source config.
pub fn create_fetcher(
    source: &DataSource,
    model: &str,
    resolution: &str,
) -> Box<dyn DataSourceFetcher> {
    match source {
        DataSource::NoaaAws {
            bucket,
            prefix_template,
        } => Box::new(AwsDataSource::new(
            bucket.clone(),
            prefix_template.clone(),
            model.to_string(),
            resolution.to_string(),
        )),
        DataSource::Nomads {
            base_url,
            path_template,
        } => Box::new(NomadsDataSource::new(
            base_url.clone(),
            path_template.clone(),
        )),
        DataSource::Thredds { .. } => {
            // TODO: Implement THREDDS fetcher
            unimplemented!("THREDDS data source not yet implemented")
        }
        DataSource::GoesAws {
            bucket,
            product,
            bands,
        } => Box::new(GoesDataSource::new(
            bucket.clone(),
            product.clone(),
            bands.clone(),
        )),
    }
}

/// Determine the most recent available model cycle.
#[allow(dead_code)]
pub fn latest_available_cycle(cycles: &[u32], delay_hours: u32) -> (String, u32) {
    let now = Utc::now() - Duration::hours(delay_hours as i64);
    let date = now.format("%Y%m%d").to_string();
    let current_hour = now.hour();

    // Find the most recent cycle that's available
    let cycle = cycles
        .iter()
        .filter(|&&c| c <= current_hour)
        .max()
        .copied()
        .unwrap_or_else(|| {
            // Use previous day's last cycle
            *cycles.last().unwrap_or(&0)
        });

    (date, cycle)
}

/// Generate list of date/cycle combinations to check.
pub fn cycles_to_check(cycles: &[u32], lookback_hours: u32) -> Vec<(String, u32)> {
    let now = Utc::now();
    let mut result = Vec::new();

    for hours_back in 0..lookback_hours {
        let check_time = now - Duration::hours(hours_back as i64);
        let date = check_time.format("%Y%m%d").to_string();

        for &cycle in cycles {
            if check_time.hour() >= cycle || hours_back > 0 {
                result.push((date.clone(), cycle));
            }
        }
    }

    result.sort();
    result.dedup();
    result
}
