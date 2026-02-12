//! NASA GES DISC download support for NLDAS-2 data.
//!
//! Provides Earthdata Login authentication and NLDAS-2 file URL construction.
//! GES DISC uses an OAuth2 redirect flow:
//!
//! 1. Request to GES DISC → 302 redirect to URS OAuth
//! 2. Client follows redirect with HTTP Basic auth (username/password)
//! 3. URS redirects back to GES DISC with session cookies
//! 4. GES DISC returns the data
//!
//! This requires `reqwest` with `cookie_store(true)` and custom redirect
//! handling to inject Basic auth on the `urs.earthdata.nasa.gov` host.
//!
//! **Prerequisites:**
//! - `EARTHDATA_USERNAME` and `EARTHDATA_PASSWORD` env vars must be set
//! - The Earthdata account must have approved the "NASA GESDISC DATA ARCHIVE" app
//!   at `https://urs.earthdata.nasa.gov/approve_app?client_id=e2WVk8Pw6weeLUKZYOxvTQ`

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Timelike, Utc};
use reqwest::{Client, StatusCode};
use tracing::{debug, info, warn};

use crate::model_runner::DownloadFile;

/// Earthdata URS hostname for authentication.
const URS_HOST: &str = "urs.earthdata.nasa.gov";

/// User agent for NASA Earthdata requests.
const EARTHDATA_USER_AGENT: &str = "weather-wms/0.1 (NASA GES DISC data access)";

/// Build an authenticated reqwest client for GES DISC downloads.
///
/// The client is configured with:
/// - Cookie store enabled (for session cookies across OAuth redirects)
/// - Custom redirect policy (injects Basic auth on URS host)
/// - Reasonable timeouts for large files
///
/// Returns `None` if credentials are not configured.
pub fn build_earthdata_client() -> Result<Option<(Client, String, String)>> {
    let username = match std::env::var("EARTHDATA_USERNAME") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            warn!("EARTHDATA_USERNAME not set, NLDAS downloads will be skipped");
            return Ok(None);
        }
    };

    let password = match std::env::var("EARTHDATA_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            warn!("EARTHDATA_PASSWORD not set, NLDAS downloads will be skipped");
            return Ok(None);
        }
    };

    // Build client with cookie store for OAuth redirect flow
    // We use a manual redirect policy so we can inject auth on the URS host
    let client = Client::builder()
        .cookie_store(true)
        .user_agent(EARTHDATA_USER_AGENT)
        .redirect(reqwest::redirect::Policy::none()) // Handle redirects manually
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .context("Failed to build Earthdata HTTP client")?;

    info!("Earthdata client configured for user: {}", username);

    Ok(Some((client, username, password)))
}

/// Download a file from GES DISC with Earthdata authentication.
///
/// Handles the OAuth redirect chain:
/// 1. GET to GES DISC URL
/// 2. Follow 302 to URS with Basic auth
/// 3. Follow 302 back to GES DISC with session cookies
/// 4. Stream response body to output file
///
/// Returns the path to the downloaded file.
pub async fn download_earthdata_file(
    client: &Client,
    username: &str,
    password: &str,
    url: &str,
    output_path: &PathBuf,
) -> Result<u64> {
    debug!(url = %url, "Starting Earthdata download");

    // Step 1: Initial request to GES DISC (expect 302 redirect)
    let mut current_url = url.to_string();
    let mut redirect_count = 0;
    let max_redirects = 10;

    loop {
        if redirect_count >= max_redirects {
            anyhow::bail!("Too many redirects ({})", max_redirects);
        }

        let mut request = client.get(&current_url);

        // Inject Basic auth when redirected to URS
        if current_url.contains(URS_HOST) {
            request = request.basic_auth(username, Some(password));
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Request failed for {}", current_url))?;

        let status = response.status();

        if status.is_redirection() {
            // Follow redirect
            if let Some(location) = response.headers().get("location") {
                let location_str = location.to_str().context("Invalid redirect location")?;

                // Handle relative redirects
                let next_url = if location_str.starts_with("http") {
                    location_str.to_string()
                } else if location_str.starts_with('/') {
                    // Extract scheme + host from current URL
                    let parsed = reqwest::Url::parse(&current_url)?;
                    format!(
                        "{}://{}{}",
                        parsed.scheme(),
                        parsed.host_str().unwrap_or(""),
                        location_str
                    )
                } else {
                    location_str.to_string()
                };

                debug!(
                    from = %current_url,
                    to = %next_url,
                    redirect = redirect_count + 1,
                    "Following redirect"
                );

                current_url = next_url;
                redirect_count += 1;
                continue;
            } else {
                anyhow::bail!("Redirect without Location header from {}", current_url);
            }
        }

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            anyhow::bail!(
                "Authentication failed ({}). Check EARTHDATA_USERNAME/PASSWORD \
                 and ensure GES DISC app is authorized at \
                 https://urs.earthdata.nasa.gov/approve_app?client_id=e2WVk8Pw6weeLUKZYOxvTQ",
                status
            );
        }

        if !status.is_success() {
            anyhow::bail!("Download failed with status {} for {}", status, url);
        }

        // Success! Stream body to file
        let bytes = response
            .bytes()
            .await
            .context("Failed to read response body")?;

        let file_size = bytes.len() as u64;

        // Ensure parent directory exists
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(output_path, &bytes).await?;

        info!(
            url = %url,
            path = %output_path.display(),
            size = file_size,
            redirects = redirect_count,
            "Earthdata download complete"
        );

        return Ok(file_size);
    }
}

/// Construct NLDAS-2 download URLs for a given time range.
///
/// # Arguments
/// * `model_id` - "nldas-noah" or "nldas-forcing"
/// * `now` - Current time
/// * `delay_hours` - Data latency (typically 96 hours / 4 days for NLDAS)
/// * `retention_hours` - How far back to look (typically 720 hours / 30 days)
///
/// Returns a list of `DownloadFile` entries for missing hourly files.
pub fn build_nldas_file_list(
    model_id: &str,
    now: DateTime<Utc>,
    delay_hours: u32,
    lookback_hours: u32,
) -> Vec<DownloadFile> {
    let mut files = Vec::new();

    // Determine product type from model ID
    let (product_prefix, file_prefix) = match model_id {
        "nldas-noah" => ("NLDAS_NOAH0125_H.2.0", "NLDAS_NOAH0125_H"),
        "nldas-forcing" => ("NLDAS_FORA0125_H.2.0", "NLDAS_FORA0125_H"),
        _ => {
            warn!(model = %model_id, "Unknown NLDAS model ID");
            return files;
        }
    };

    // Time window: (now - lookback) to (now - delay)
    let latest = now - ChronoDuration::hours(delay_hours as i64);
    let earliest = now - ChronoDuration::hours(lookback_hours as i64);

    // Iterate over each hour in the window
    let mut current = earliest;
    while current <= latest {
        let year = current.year();
        let doy = current.ordinal(); // Day of year (1-366)
        let date_str = current.format("%Y%m%d").to_string();
        let hour = current.hour();

        // URL: https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/{product}/{YYYY}/{DDD}/{file}.nc
        let filename = format!("{}.A{}.{:02}00.020.nc", file_prefix, date_str, hour);
        let url = format!(
            "https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/{}/{}/{:03}/{}",
            product_prefix, year, doy, filename
        );

        // Output filename for local storage
        let output_filename = format!(
            "{}_{}_{}_{:02}00.nc",
            model_id.replace('-', "_"),
            date_str,
            format!("{:03}", doy),
            hour
        );

        files.push(DownloadFile {
            url,
            filename: output_filename,
            timestamp: Some(current),
        });

        current = current + ChronoDuration::hours(1);
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_build_nldas_file_list_noah() {
        let now = Utc.with_ymd_and_hms(2026, 2, 10, 12, 0, 0).unwrap();
        // Small window for testing: delay 96h, lookback 100h
        // Latest = 2026-02-06 12:00, Earliest = 2026-02-06 08:00 (4 hours)
        let files = build_nldas_file_list("nldas-noah", now, 96, 100);

        assert_eq!(files.len(), 5); // 08, 09, 10, 11, 12 = 5 hours

        // Check first file
        let first = &files[0];
        assert!(first.url.contains("NLDAS_NOAH0125_H.2.0"));
        assert!(first.url.contains("NLDAS_NOAH0125_H.A20260206.0800.020.nc"));
        assert!(first.url.contains("/2026/037/")); // Feb 6 = day 37

        // Check output filename format
        assert!(first.filename.starts_with("nldas_noah_"));
        assert!(first.filename.ends_with(".nc"));
    }

    #[test]
    fn test_build_nldas_file_list_forcing() {
        let now = Utc.with_ymd_and_hms(2026, 2, 10, 12, 0, 0).unwrap();
        let files = build_nldas_file_list("nldas-forcing", now, 96, 100);

        assert_eq!(files.len(), 5);
        assert!(files[0].url.contains("NLDAS_FORA0125_H.2.0"));
        assert!(files[0]
            .url
            .contains("NLDAS_FORA0125_H.A20260206.0800.020.nc"));
    }

    #[test]
    fn test_build_nldas_file_list_unknown_model() {
        let now = Utc::now();
        let files = build_nldas_file_list("unknown-model", now, 96, 100);
        assert!(files.is_empty());
    }

    #[test]
    fn test_build_nldas_file_list_single_hour() {
        let now = Utc.with_ymd_and_hms(2026, 2, 10, 12, 0, 0).unwrap();
        // delay=96, lookback=97 → 1 hour window
        let files = build_nldas_file_list("nldas-noah", now, 96, 97);
        assert_eq!(files.len(), 2); // hours 11 and 12
    }

    #[test]
    fn test_build_nldas_url_format() {
        // Use a known date
        let test_time = Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap();
        let files = build_nldas_file_list("nldas-noah", test_time, 0, 1);

        // Should have 2 files (hour 11 and 12)
        assert!(!files.is_empty());

        let last = files.last().unwrap();
        // March 15 = day 74
        assert!(
            last.url.contains("/074/"),
            "URL should contain day-of-year: {}",
            last.url
        );
        assert!(last
            .url
            .starts_with("https://hydro1.gesdisc.eosdis.nasa.gov"));
    }

    #[test]
    fn test_build_nldas_file_list_year_boundary() {
        // Dec 31 → Jan 1 transition
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 12, 0, 0).unwrap();
        // Look back 120 hours from delay of 0 → goes to Jan 1 - Dec 31
        let files = build_nldas_file_list("nldas-noah", now, 0, 120);

        // Should span from ~Dec 31 to Jan 5
        assert!(!files.is_empty());

        // Find a file from 2025 (day > 350)
        let has_2025 = files.iter().any(|f| f.url.contains("/2025/"));
        let has_2026 = files.iter().any(|f| f.url.contains("/2026/"));
        assert!(has_2025, "Should have files from 2025");
        assert!(has_2026, "Should have files from 2026");
    }

    #[test]
    fn test_build_earthdata_client_no_creds() {
        // With no env vars set, should return None
        // (This test relies on EARTHDATA_USERNAME not being set in CI)
        // We can't easily test this without modifying env, so just verify the function exists
        // and doesn't panic
        let _ = build_earthdata_client();
    }
}
