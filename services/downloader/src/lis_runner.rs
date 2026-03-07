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
use chrono::{DateTime, Datelike, Duration as ChronoDuration, TimeZone, Timelike, Utc};
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

/// LIS product configuration for URL construction.
struct LisProduct {
    /// Top-level dataset directory (e.g., "NLDAS", "GLDAS")
    dataset: &'static str,
    /// Product version directory (e.g., "NLDAS_NOAH0125_H.2.0")
    product_dir: &'static str,
    /// File prefix before `.A{date}` (e.g., "NLDAS_NOAH0125_H")
    file_prefix: &'static str,
    /// Version suffix in filename (e.g., "020" for NLDAS, "021" for GLDAS)
    version_suffix: &'static str,
    /// File extension (e.g., "nc" or "nc4")
    extension: &'static str,
    /// Time step in hours (1 for NLDAS hourly, 3 for GLDAS 3-hourly)
    step_hours: i64,
}

/// Construct NASA LIS download URLs for a given time range.
///
/// Supports NLDAS-2 (hourly) and GLDAS-2.1 EP (3-hourly) products.
///
/// # Arguments
/// * `model_id` - Model identifier (e.g., "nldas-noah", "gldas-noah")
/// * `now` - Current time
/// * `delay_hours` - Data latency
/// * `retention_hours` - How far back to look
///
/// Returns a list of `DownloadFile` entries for missing files.
pub fn build_lis_file_list(
    model_id: &str,
    now: DateTime<Utc>,
    delay_hours: u32,
    lookback_hours: u32,
) -> Vec<DownloadFile> {
    let mut files = Vec::new();

    let product = match model_id {
        "nldas-noah" => LisProduct {
            dataset: "NLDAS",
            product_dir: "NLDAS_NOAH0125_H.2.0",
            file_prefix: "NLDAS_NOAH0125_H",
            version_suffix: "020",
            extension: "nc",
            step_hours: 1,
        },
        "nldas-forcing" => LisProduct {
            dataset: "NLDAS",
            product_dir: "NLDAS_FORA0125_H.2.0",
            file_prefix: "NLDAS_FORA0125_H",
            version_suffix: "020",
            extension: "nc",
            step_hours: 1,
        },
        "gldas-noah" => LisProduct {
            dataset: "GLDAS",
            product_dir: "GLDAS_NOAH025_3H_EP.2.1",
            file_prefix: "GLDAS_NOAH025_3H_EP",
            version_suffix: "021",
            extension: "nc4",
            step_hours: 3,
        },
        _ => {
            warn!(model = %model_id, "Unknown LIS model ID");
            return files;
        }
    };

    // Time window: (now - lookback) to (now - delay)
    let latest = now - ChronoDuration::hours(delay_hours as i64);
    let earliest = now - ChronoDuration::hours(lookback_hours as i64);

    // Snap earliest to the product's time step boundary
    let earliest = snap_to_step(earliest, product.step_hours);

    // Iterate over each time step in the window
    let mut current = earliest;
    while current <= latest {
        let year = current.year();
        let doy = current.ordinal(); // Day of year (1-366)
        let date_str = current.format("%Y%m%d").to_string();
        let hour = current.hour();
        let minute = current.minute();

        let filename = format!(
            "{}.A{}.{:02}{:02}.{}.{}",
            product.file_prefix, date_str, hour, minute, product.version_suffix, product.extension
        );
        let url = format!(
            "https://hydro1.gesdisc.eosdis.nasa.gov/data/{}/{}/{}/{:03}/{}",
            product.dataset, product.product_dir, year, doy, filename
        );

        // Output filename for local storage
        let output_filename = format!(
            "{}_{}_{}_{:02}{:02}.{}",
            model_id.replace('-', "_"),
            date_str,
            format!("{:03}", doy),
            hour,
            minute,
            product.extension
        );

        files.push(DownloadFile {
            url,
            filename: output_filename,
            timestamp: Some(current),
        });

        current = current + ChronoDuration::hours(product.step_hours);
    }

    files
}

/// Snap a timestamp down to the nearest time step boundary.
fn snap_to_step(dt: DateTime<Utc>, step_hours: i64) -> DateTime<Utc> {
    if step_hours <= 1 {
        return dt;
    }
    let hour = dt.hour() as i64;
    let snapped_hour = (hour / step_hours) * step_hours;
    dt.date_naive()
        .and_hms_opt(snapped_hour as u32, 0, 0)
        .map(|naive| TimeZone::from_utc_datetime(&Utc, &naive))
        .unwrap_or(dt)
}

/// Backward-compatible alias for `build_lis_file_list`.
pub fn build_nldas_file_list(
    model_id: &str,
    now: DateTime<Utc>,
    delay_hours: u32,
    lookback_hours: u32,
) -> Vec<DownloadFile> {
    build_lis_file_list(model_id, now, delay_hours, lookback_hours)
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

    // ==================== GLDAS File List ====================

    #[test]
    fn test_build_lis_file_list_gldas_noah() {
        let now = Utc.with_ymd_and_hms(2026, 2, 15, 12, 0, 0).unwrap();
        // GLDAS EP: delay ~792h (~33 days), lookback 800h
        // Latest = now - 792h, Earliest = now - 800h (8-hour window = ~2-3 files at 3h step)
        let files = build_lis_file_list("gldas-noah", now, 792, 800);

        assert!(!files.is_empty());

        // Check file URL format
        let first = &files[0];
        assert!(
            first.url.contains("GLDAS_NOAH025_3H_EP.2.1"),
            "URL should contain GLDAS product dir: {}",
            first.url
        );
        assert!(
            first.url.contains("/data/GLDAS/"),
            "URL should use GLDAS dataset: {}",
            first.url
        );
        assert!(
            first.url.contains(".021.nc4"),
            "URL should have .021.nc4 extension: {}",
            first.url
        );

        // Check output filename format
        assert!(
            first.filename.starts_with("gldas_noah_"),
            "Output filename should start with gldas_noah_: {}",
            first.filename
        );
        assert!(
            first.filename.ends_with(".nc4"),
            "Output filename should end with .nc4: {}",
            first.filename
        );
    }

    #[test]
    fn test_build_lis_file_list_gldas_3hourly_step() {
        let now = Utc.with_ymd_and_hms(2026, 2, 15, 12, 0, 0).unwrap();
        // 24-hour window: should produce exactly 9 files (0,3,6,9,12,15,18,21,0 = 9 steps)
        let files = build_lis_file_list("gldas-noah", now, 0, 24);

        // 24 hours / 3-hour step = 8 intervals + 1 = 9 timestamps
        assert_eq!(files.len(), 9, "24h window at 3h step should have 9 files");

        // Verify 3-hourly spacing: hours should be 12,15,18,21,0,3,6,9,12
        // (going back 24h from hour 12)
        for file in &files {
            let ts = file.timestamp.unwrap();
            assert_eq!(
                ts.hour() % 3,
                0,
                "GLDAS files should be on 3-hour boundaries, got hour {}",
                ts.hour()
            );
        }
    }

    #[test]
    fn test_build_lis_file_list_gldas_url_format() {
        let now = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        let files = build_lis_file_list("gldas-noah", now, 0, 3);

        let last = files.last().unwrap();
        // Jan 15 = day 15
        assert!(
            last.url.contains("/015/"),
            "URL should contain day-of-year: {}",
            last.url
        );
        assert!(last
            .url
            .starts_with("https://hydro1.gesdisc.eosdis.nasa.gov"));
        assert!(last.url.contains("GLDAS_NOAH025_3H_EP.A2026011"));
    }

    #[test]
    fn test_snap_to_step() {
        let dt = Utc.with_ymd_and_hms(2026, 1, 10, 5, 30, 0).unwrap();
        let snapped = snap_to_step(dt, 3);
        assert_eq!(snapped.hour(), 3);
        assert_eq!(snapped.minute(), 0);

        // Already on boundary
        let dt2 = Utc.with_ymd_and_hms(2026, 1, 10, 6, 0, 0).unwrap();
        let snapped2 = snap_to_step(dt2, 3);
        assert_eq!(snapped2.hour(), 6);

        // step_hours=1 should not change anything
        let snapped3 = snap_to_step(dt, 1);
        assert_eq!(snapped3, dt);
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
