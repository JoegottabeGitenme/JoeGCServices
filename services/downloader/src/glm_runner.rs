//! GOES GLM (Geostationary Lightning Mapper) L2 LCFA runner.
//!
//! Polls the public NOAA GOES S3 bucket for new 20-second `GLM-L2-LCFA`
//! granules, downloads each one and POSTs the raw netCDF to the ingester's
//! `/ingest/lightning`, which parses, CONUS-clips and stores the flashes.
//!
//! This is a dedicated runner (like NDBC and storm events) rather than a
//! `ModelRunner` source because GLM is point-event data and wants a very
//! different cadence from gridded imagery: a new file every 20 s per satellite
//! (the quickest gridded source here polls every 120 s) and a small rolling
//! window, not a retention-sized backfill.
//!
//! ## Delivery semantics
//!
//! The ingester is idempotent (`UNIQUE(satellite, flash_time, flash_id)`), so this
//! runner only needs to be *at-least-once*:
//!
//! - It remembers processed keys in memory. After a restart it simply re-sends the
//!   last `backfill_minutes` of granules; the ingester stores nothing twice.
//! - `2xx` -> done. `422` -> the granule itself is unusable, so it is dropped (a
//!   retry cannot fix it, and retrying a poison file forever would starve newer
//!   data). Anything else (network error, `5xx`) -> retried on the next poll.
//! - Granules are sent oldest-first so ids in the database follow time order.
//!
//! ## Listing
//!
//! Keys are `GLM-L2-LCFA/{year}/{doy}/{hour}/OR_GLM-L2-LCFA_{sat}_s{YYYYDDDHHMMSSt}_e..._c....nc`.
//! They sort chronologically, so `start_after` on a truncated `s` timestamp lists
//! only granules from the window start onwards.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, Timelike, Utc};
use reqwest::{Client, StatusCode};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Configuration for one GLM source (one satellite).
#[derive(Debug, Clone)]
pub struct GlmConfig {
    /// Source identifier, e.g. `glm-goes19`.
    pub id: String,
    pub bucket: String,
    /// e.g. `GLM-L2-LCFA`
    pub product: String,
    /// Platform id as it appears in file names, e.g. `G19`.
    pub satellite: String,
    pub poll_interval_secs: u64,
    /// How far back each poll looks.
    pub backfill_minutes: u32,
    /// Base URL of the ingester (with or without a trailing `/ingest`).
    pub ingester_url: String,
    /// Where granules are fetched from. `None` = the bucket's public HTTPS
    /// endpoint; overridden in tests.
    pub download_base_url: Option<String>,
}

impl GlmConfig {
    fn download_url(&self, key: &str) -> String {
        match &self.download_base_url {
            Some(base) => format!("{}/{}", base.trim_end_matches('/'), key),
            None => format!("https://{}.s3.amazonaws.com/{}", self.bucket, key),
        }
    }

    fn ingest_url(&self, source_url: &str) -> String {
        let base = self.ingester_url.trim_end_matches("/ingest");
        format!(
            "{}/ingest/lightning?source_url={}",
            base,
            percent_encode(source_url)
        )
    }
}

/// What to do with a granule after the ingester answered.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Stored (or already stored). Do not send again.
    Done,
    /// The granule is unusable; retrying cannot help. Do not send again.
    Drop,
    /// Transient failure. Try again next poll.
    Retry,
}

/// Classify the ingester's HTTP status. See the module docs.
pub fn classify_status(status: StatusCode) -> Outcome {
    if status.is_success() {
        Outcome::Done
    } else if status == StatusCode::UNPROCESSABLE_ENTITY {
        Outcome::Drop
    } else {
        Outcome::Retry
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// The `{product}/{year}/{doy}/{hour}/` folders covering `[since, now]`,
/// oldest first. Spans at most a few hours for any sane `backfill_minutes`.
pub fn hour_prefixes(product: &str, since: DateTime<Utc>, now: DateTime<Utc>) -> Vec<String> {
    let mut out = Vec::new();
    let mut t = since
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(since);
    while t <= now {
        out.push(format!(
            "{}/{}/{:03}/{:02}/",
            product,
            t.year(),
            t.ordinal(),
            t.hour()
        ));
        t += ChronoDuration::hours(1);
    }
    out
}

/// Parse the observation start from a GLM key:
/// `..._s{YYYY}{DDD}{HH}{MM}{SS}{t}_e...` (the final digit is tenths of a second).
pub fn parse_key_start(key: &str) -> Option<DateTime<Utc>> {
    let name = key.rsplit('/').next()?;
    let i = name.find("_s")? + 2;
    let ts = name.get(i..i + 14)?;
    if !ts.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = ts[0..4].parse().ok()?;
    let doy: u32 = ts[4..7].parse().ok()?;
    let (h, m, s): (u32, u32, u32) = (
        ts[7..9].parse().ok()?,
        ts[9..11].parse().ok()?,
        ts[11..13].parse().ok()?,
    );
    let date = NaiveDate::from_yo_opt(year, doy)?;
    Some(DateTime::from_naive_utc_and_offset(
        date.and_hms_opt(h, m, s)?,
        Utc,
    ))
}

/// S3 `start_after` key that lists granules starting at or after `since`.
///
/// `start_after` is exclusive, so the timestamp is truncated *before* the tenths
/// digit: a key sharing that prefix sorts after it, so the granule that starts
/// exactly at `since` is still included.
pub fn start_after_key(
    prefix: &str,
    product: &str,
    satellite: &str,
    since: DateTime<Utc>,
) -> String {
    format!(
        "{}OR_{}_{}_s{}{:03}{:02}{:02}{:02}",
        prefix,
        product,
        satellite,
        since.year(),
        since.ordinal(),
        since.hour(),
        since.minute(),
        since.second()
    )
}

/// From a listing, the granules to send: right satellite, `.nc`, inside the
/// window, not already processed. Oldest first.
pub fn select_new(
    keys: &[String],
    satellite: &str,
    since: DateTime<Utc>,
    seen: &HashSet<String>,
) -> Vec<String> {
    let needle = format!("_{}_s", satellite);
    let mut out: Vec<(DateTime<Utc>, String)> = keys
        .iter()
        .filter(|k| k.ends_with(".nc") && k.contains(&needle) && !seen.contains(*k))
        .filter_map(|k| {
            let t = parse_key_start(k)?;
            (t >= since).then(|| (t, k.clone()))
        })
        .collect();
    out.sort();
    out.into_iter().map(|(_, k)| k).collect()
}

/// Forget processed keys that fell out of the window, so the set stays bounded.
pub fn prune_seen(seen: &mut HashSet<String>, since: DateTime<Utc>) {
    seen.retain(|k| parse_key_start(k).is_none_or(|t| t >= since));
}

/// Minimal percent-encoding for a query-string value (the source URL is only for
/// the ingester's logs, but it must not be able to break the request line).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Polls one satellite's GLM granules forever.
pub struct GlmRunner {
    config: GlmConfig,
    client: Client,
    s3: Option<aws_sdk_s3::Client>,
}

impl GlmRunner {
    pub fn new(config: GlmConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Self {
            config,
            client,
            s3: None,
        })
    }

    /// Test constructor that never touches AWS.
    #[cfg(test)]
    fn for_test(config: GlmConfig) -> Self {
        Self {
            config,
            client: Client::new(),
            s3: None,
        }
    }

    async fn s3(&mut self) -> &aws_sdk_s3::Client {
        if self.s3.is_none() {
            // Public bucket: unsigned requests (same as the model runners).
            let aws = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new("us-east-1"))
                .no_credentials()
                .load()
                .await;
            self.s3 = Some(aws_sdk_s3::Client::new(&aws));
        }
        self.s3.as_ref().expect("just initialised")
    }

    pub async fn run_forever(mut self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let interval = Duration::from_secs(self.config.poll_interval_secs.max(1));
        info!(
            source = %self.config.id,
            bucket = %self.config.bucket,
            satellite = %self.config.satellite,
            poll_interval_secs = self.config.poll_interval_secs,
            backfill_minutes = self.config.backfill_minutes,
            "Starting GLM lightning runner"
        );

        let mut seen: HashSet<String> = HashSet::new();
        loop {
            if let Err(e) = self.poll_once(&mut seen).await {
                error!(source = %self.config.id, error = %e, "GLM poll failed");
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown.recv() => {
                    info!(source = %self.config.id, "GLM runner shutting down");
                    return Ok(());
                }
            }
        }
    }

    /// One poll: list the window, send whatever is new.
    async fn poll_once(&mut self, seen: &mut HashSet<String>) -> Result<()> {
        let now = Utc::now();
        let since = now - ChronoDuration::minutes(self.config.backfill_minutes as i64);

        // Only the folder containing `since` needs a lower bound; later folders
        // are listed whole (they are entirely inside the window).
        let since_folder = hour_prefixes(&self.config.product, since, since).remove(0);
        let mut keys = Vec::new();
        for prefix in hour_prefixes(&self.config.product, since, now) {
            let start_after = (prefix == since_folder).then(|| {
                start_after_key(&prefix, &self.config.product, &self.config.satellite, since)
            });
            keys.extend(self.list(&prefix, start_after.as_deref()).await?);
        }

        let fresh = select_new(&keys, &self.config.satellite, since, seen);
        let sent = self.send_batch(&fresh, seen).await;
        prune_seen(seen, since);
        if sent > 0 {
            debug!(source = %self.config.id, sent, listed = keys.len(), "GLM poll complete");
        }
        Ok(())
    }

    /// Send granules oldest-first, recording the ones that need not be sent again.
    /// Returns how many were stored.
    ///
    /// On a transient failure it **stops**: sending later granules first would
    /// store them ahead of the one that failed, and the database ids (which are a
    /// change-feed cursor) would no longer follow time order. The failed granule
    /// and everything after it are simply retried on the next poll.
    async fn send_batch(&self, keys: &[String], seen: &mut HashSet<String>) -> usize {
        let mut sent = 0usize;
        for key in keys {
            match self.send_one(key).await {
                Outcome::Done => {
                    seen.insert(key.clone());
                    sent += 1;
                }
                Outcome::Drop => {
                    warn!(source = %self.config.id, key = %key, "Dropping unusable GLM granule");
                    seen.insert(key.clone());
                }
                Outcome::Retry => {
                    debug!(source = %self.config.id, key = %key, "Will retry GLM granule next poll");
                    break;
                }
            }
        }
        sent
    }

    async fn list(&mut self, prefix: &str, start_after: Option<&str>) -> Result<Vec<String>> {
        let bucket = self.config.bucket.clone();
        let client = self.s3().await.clone();
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut req = client
                .list_objects_v2()
                .bucket(&bucket)
                .prefix(prefix)
                .max_keys(1000);
            match &token {
                Some(t) => req = req.continuation_token(t.clone()),
                None => {
                    if let Some(s) = start_after {
                        req = req.start_after(s);
                    }
                }
            }
            let resp = req.send().await.context("S3 list_objects_v2 failed")?;
            keys.extend(
                resp.contents()
                    .iter()
                    .filter_map(|o| o.key().map(String::from)),
            );
            if resp.is_truncated() == Some(true) {
                token = resp.next_continuation_token().map(String::from);
            } else {
                return Ok(keys);
            }
        }
    }

    /// Download one granule and hand it to the ingester.
    async fn send_one(&self, key: &str) -> Outcome {
        let url = self.config.download_url(key);
        let bytes = match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    warn!(source = %self.config.id, key, error = %e, "GLM download body failed");
                    return Outcome::Retry;
                }
            },
            Ok(r) => {
                warn!(source = %self.config.id, key, status = %r.status(), "GLM download refused");
                return Outcome::Retry;
            }
            Err(e) => {
                warn!(source = %self.config.id, key, error = %e, "GLM download failed");
                return Outcome::Retry;
            }
        };

        match self
            .client
            .post(self.config.ingest_url(&url))
            .header("content-type", "application/x-netcdf")
            .body(bytes)
            .send()
            .await
        {
            Ok(r) => {
                let status = r.status();
                let outcome = classify_status(status);
                if outcome != Outcome::Done {
                    warn!(source = %self.config.id, key, %status, ?outcome, "Ingester did not accept GLM granule");
                }
                outcome
            }
            Err(e) => {
                warn!(source = %self.config.id, key, error = %e, "Could not reach ingester");
                Outcome::Retry
            }
        }
    }
}

/// Load a GLM source from a model config file. `None` if it is not an
/// `aws_s3_glm` source.
pub fn load_glm_config(
    config_path: &std::path::Path,
    ingester_url: &str,
) -> Result<Option<GlmConfig>> {
    use crate::config::ModelConfig;

    let model_config = ModelConfig::load(config_path)?;
    if model_config.source.source_type != "aws_s3_glm" {
        return Ok(None);
    }
    let s = &model_config.source;
    if s.bucket.is_empty() {
        anyhow::bail!("aws_s3_glm source {} has no bucket", model_config.model.id);
    }
    let satellite = s.satellite.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "aws_s3_glm source {} needs `satellite` (e.g. G19)",
            model_config.model.id
        )
    })?;
    Ok(Some(GlmConfig {
        id: model_config.model.id.clone(),
        bucket: s.bucket.clone(),
        product: s
            .product
            .clone()
            .unwrap_or_else(|| "GLM-L2-LCFA".to_string()),
        satellite,
        poll_interval_secs: model_config.schedule.poll_interval_secs,
        backfill_minutes: s.backfill_minutes.unwrap_or(30),
        ingester_url: ingester_url.to_string(),
        download_base_url: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    const KEY19: &str =
        "GLM-L2-LCFA/2026/280/20/OR_GLM-L2-LCFA_G19_s20262802033000_e20262802033200_c20262802033219.nc";
    const KEY18: &str =
        "GLM-L2-LCFA/2026/280/20/OR_GLM-L2-LCFA_G18_s20262802033000_e20262802033200_c20262802033222.nc";

    fn cfg(download: Option<String>, ingester: &str) -> GlmConfig {
        GlmConfig {
            id: "glm-goes19".into(),
            bucket: "noaa-goes19".into(),
            product: "GLM-L2-LCFA".into(),
            satellite: "G19".into(),
            poll_interval_secs: 15,
            backfill_minutes: 30,
            ingester_url: ingester.into(),
            download_base_url: download,
        }
    }

    #[test]
    fn parses_the_start_time_from_a_real_key() {
        // 2026 day 280 = Oct 7; the 14th digit is tenths of a second.
        assert_eq!(parse_key_start(KEY19), Some(ts(2026, 10, 7, 20, 33, 0)));
        assert_eq!(parse_key_start(KEY18), Some(ts(2026, 10, 7, 20, 33, 0)));
    }

    #[test]
    fn unparseable_keys_yield_none_not_a_panic() {
        for bad in [
            "",
            "GLM-L2-LCFA/",
            "x_s.nc",
            "OR_GLM-L2-LCFA_G19_sABCDEFGHIJKLMN_e.nc",
            "OR_GLM_G19_s2026",
        ] {
            assert_eq!(parse_key_start(bad), None, "{bad:?}");
        }
        // day 400 does not exist
        assert_eq!(
            parse_key_start("OR_GLM-L2-LCFA_G19_s20264002033000_e.nc"),
            None
        );
    }

    #[test]
    fn hour_prefixes_cover_the_window_across_hour_and_day_boundaries() {
        let p = hour_prefixes(
            "GLM-L2-LCFA",
            ts(2026, 10, 7, 19, 45, 0),
            ts(2026, 10, 7, 20, 10, 0),
        );
        assert_eq!(
            p,
            vec!["GLM-L2-LCFA/2026/280/19/", "GLM-L2-LCFA/2026/280/20/"]
        );
        // across midnight and into a new day-of-year
        let p = hour_prefixes(
            "GLM-L2-LCFA",
            ts(2026, 10, 7, 23, 50, 0),
            ts(2026, 10, 8, 0, 5, 0),
        );
        assert_eq!(
            p,
            vec!["GLM-L2-LCFA/2026/280/23/", "GLM-L2-LCFA/2026/281/00/"]
        );
        // across New Year (day 365 -> day 001 of the next year)
        let p = hour_prefixes(
            "GLM-L2-LCFA",
            ts(2026, 12, 31, 23, 40, 0),
            ts(2027, 1, 1, 0, 10, 0),
        );
        assert_eq!(
            p,
            vec!["GLM-L2-LCFA/2026/365/23/", "GLM-L2-LCFA/2027/001/00/"]
        );
    }

    #[test]
    fn a_window_inside_one_hour_is_one_folder() {
        assert_eq!(
            hour_prefixes(
                "GLM-L2-LCFA",
                ts(2026, 10, 7, 20, 5, 0),
                ts(2026, 10, 7, 20, 35, 0)
            )
            .len(),
            1
        );
    }

    #[test]
    fn start_after_includes_the_granule_that_starts_exactly_at_since() {
        // start_after is exclusive: the truncated timestamp must sort BEFORE the
        // real key that begins with it.
        let sa = start_after_key(
            "GLM-L2-LCFA/2026/280/20/",
            "GLM-L2-LCFA",
            "G19",
            ts(2026, 10, 7, 20, 33, 0),
        );
        assert_eq!(
            sa,
            "GLM-L2-LCFA/2026/280/20/OR_GLM-L2-LCFA_G19_s2026280203300"
        );
        assert!(
            KEY19 > sa.as_str(),
            "the granule starting at `since` must be listed"
        );
        // ...and an earlier granule must not be.
        let earlier = "GLM-L2-LCFA/2026/280/20/OR_GLM-L2-LCFA_G19_s20262802032400_e.nc";
        assert!(earlier < sa.as_str());
    }

    #[test]
    fn select_new_filters_satellite_suffix_window_and_seen_and_sorts() {
        let since = ts(2026, 10, 7, 20, 0, 0);
        let k = |sat: &str, hms: &str, ext: &str| {
            format!("GLM-L2-LCFA/2026/280/20/OR_GLM-L2-LCFA_{sat}_s2026280{hms}0_e.{ext}")
        };
        let keys = vec![
            k("G19", "203320", "nc"),  // ok
            k("G19", "203300", "nc"),  // ok, older -> must sort first
            k("G18", "203300", "nc"),  // wrong satellite
            k("G19", "203340", "txt"), // wrong suffix
            k("G19", "195900", "nc"),  // before the window
            k("G19", "203400", "nc"),  // already seen
        ];
        let seen: HashSet<String> = [k("G19", "203400", "nc")].into_iter().collect();
        let got = select_new(&keys, "G19", since, &seen);
        assert_eq!(
            got,
            vec![k("G19", "203300", "nc"), k("G19", "203320", "nc")]
        );
    }

    #[test]
    fn prune_seen_drops_only_keys_older_than_the_window() {
        let mut seen: HashSet<String> = [KEY19.to_string(), "weird-unparseable-key".to_string()]
            .into_iter()
            .collect();
        prune_seen(&mut seen, ts(2026, 10, 7, 20, 40, 0)); // KEY19 starts 20:33 -> expired
        assert!(!seen.contains(KEY19));
        assert!(
            seen.contains("weird-unparseable-key"),
            "unparseable keys are kept, never silently lost"
        );
        prune_seen(&mut seen, ts(2026, 10, 7, 20, 0, 0));
    }

    #[test]
    fn status_classification_drives_retry_behaviour() {
        assert_eq!(classify_status(StatusCode::OK), Outcome::Done);
        assert_eq!(
            classify_status(StatusCode::UNPROCESSABLE_ENTITY),
            Outcome::Drop
        );
        for code in [500, 502, 503, 504, 404, 413, 429, 400] {
            assert_eq!(
                classify_status(StatusCode::from_u16(code).unwrap()),
                Outcome::Retry,
                "{code}"
            );
        }
    }

    #[test]
    fn ingest_url_tolerates_both_ingester_url_styles_and_encodes_the_source() {
        let c = cfg(None, "http://ingester:8082/ingest");
        assert!(c
            .ingest_url("https://b.s3.amazonaws.com/a b?x=1&y=2")
            .starts_with("http://ingester:8082/ingest/lightning?source_url="));
        let c2 = cfg(None, "http://ingester:8082");
        assert!(c2
            .ingest_url("u")
            .starts_with("http://ingester:8082/ingest/lightning?"));
        let u = c.ingest_url("https://x/y z?a=b&c=d");
        assert!(!u.contains(' ') && !u.contains("&c=d"), "{u}");
    }

    #[test]
    fn download_url_uses_the_public_endpoint_unless_overridden() {
        assert_eq!(
            cfg(None, "i").download_url("a/b.nc"),
            "https://noaa-goes19.s3.amazonaws.com/a/b.nc"
        );
        assert_eq!(
            cfg(Some("http://127.0.0.1:9/".into()), "i").download_url("a/b.nc"),
            "http://127.0.0.1:9/a/b.nc"
        );
    }

    // ---- the I/O shell, against mock servers ----

    async fn mock_s3(body: &'static [u8]) -> MockServer {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"\.nc$"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&s)
            .await;
        s
    }

    #[tokio::test]
    async fn a_2xx_from_the_ingester_means_done_and_the_raw_bytes_are_forwarded() {
        let s3 = mock_s3(b"GRANULE-BYTES").await;
        let ing = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ingest/lightning"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&ing)
            .await;
        let r = GlmRunner::for_test(cfg(Some(s3.uri()), &ing.uri()));
        assert_eq!(r.send_one(KEY19).await, Outcome::Done);
        let req = &ing.received_requests().await.unwrap()[0];
        assert_eq!(
            req.body, b"GRANULE-BYTES",
            "the granule must be forwarded byte-for-byte"
        );
        assert_eq!(
            req.headers.get("content-type").unwrap(),
            "application/x-netcdf"
        );
        assert!(req.url.query().unwrap().contains("source_url="));
    }

    #[tokio::test]
    async fn a_422_drops_the_granule_and_a_503_retries_it() {
        let s3 = mock_s3(b"x").await;
        let bad = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(422))
            .mount(&bad)
            .await;
        assert_eq!(
            GlmRunner::for_test(cfg(Some(s3.uri()), &bad.uri()))
                .send_one(KEY19)
                .await,
            Outcome::Drop
        );

        let down = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&down)
            .await;
        assert_eq!(
            GlmRunner::for_test(cfg(Some(s3.uri()), &down.uri()))
                .send_one(KEY19)
                .await,
            Outcome::Retry
        );
    }

    #[tokio::test]
    async fn download_failures_and_an_unreachable_ingester_are_retried_not_dropped() {
        let ing = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&ing)
            .await;

        let s3_404 = MockServer::start().await; // no mocks -> 404 for everything
        assert_eq!(
            GlmRunner::for_test(cfg(Some(s3_404.uri()), &ing.uri()))
                .send_one(KEY19)
                .await,
            Outcome::Retry
        );

        let s3 = mock_s3(b"x").await;
        // nothing is listening on port 9
        assert_eq!(
            GlmRunner::for_test(cfg(Some(s3.uri()), "http://127.0.0.1:9"))
                .send_one(KEY19)
                .await,
            Outcome::Retry
        );
    }

    // ---- batch ordering / retry semantics ----

    fn key_at(hms: &str) -> String {
        format!("GLM-L2-LCFA/2026/280/20/OR_GLM-L2-LCFA_G19_s2026280{hms}0_e.nc")
    }

    /// An ingester that answers 503 to the Nth POST (1-based) and 200 otherwise.
    async fn ingester_failing_on(n: usize, code: u16) -> MockServer {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let ing = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |_: &wiremock::Request| {
                let i = calls.fetch_add(1, Ordering::SeqCst) + 1;
                ResponseTemplate::new(if i == n { code } else { 200 })
            })
            .mount(&ing)
            .await;
        ing
    }

    #[tokio::test]
    async fn a_transient_failure_stops_the_batch_so_later_granules_are_never_stored_first() {
        let s3 = mock_s3(b"x").await;
        let ing = ingester_failing_on(2, 503).await;
        let r = GlmRunner::for_test(cfg(Some(s3.uri()), &ing.uri()));
        let keys = vec![key_at("203300"), key_at("203320"), key_at("203340")];
        let mut seen = HashSet::new();

        let sent = r.send_batch(&keys, &mut seen).await;

        assert_eq!(sent, 1);
        assert_eq!(
            seen,
            [keys[0].clone()].into_iter().collect(),
            "only the first is done"
        );
        assert_eq!(
            ing.received_requests().await.unwrap().len(),
            2,
            "the third must not even be attempted"
        );
    }

    #[tokio::test]
    async fn a_poison_granule_is_dropped_without_blocking_the_ones_behind_it() {
        let s3 = mock_s3(b"x").await;
        let ing = ingester_failing_on(2, 422).await;
        let r = GlmRunner::for_test(cfg(Some(s3.uri()), &ing.uri()));
        let keys = vec![key_at("203300"), key_at("203320"), key_at("203340")];
        let mut seen = HashSet::new();

        let sent = r.send_batch(&keys, &mut seen).await;

        assert_eq!(sent, 2, "first and third stored");
        assert_eq!(
            seen.len(),
            3,
            "the poison one is remembered too, so it is not retried forever"
        );
        assert_eq!(ing.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn already_seen_granules_are_not_resent_on_the_next_poll() {
        let s3 = mock_s3(b"x").await;
        let ing = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ing)
            .await;
        let r = GlmRunner::for_test(cfg(Some(s3.uri()), &ing.uri()));
        let keys = vec![key_at("203300"), key_at("203320")];
        let mut seen = HashSet::new();
        assert_eq!(r.send_batch(&keys, &mut seen).await, 2);

        // The next poll lists the same two plus one new granule.
        let listing = vec![key_at("203300"), key_at("203320"), key_at("203340")];
        let fresh = select_new(&listing, "G19", ts(2026, 10, 7, 20, 0, 0), &seen);
        assert_eq!(fresh, vec![key_at("203340")]);
    }

    // ---- the real config files ----

    fn models_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/models")
    }

    #[test]
    fn the_real_glm_configs_load_through_the_downloaders_own_loader() {
        let east = load_glm_config(
            &models_dir().join("glm-goes19.yaml"),
            "http://ingester:8082/ingest",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (
                east.bucket.as_str(),
                east.satellite.as_str(),
                east.product.as_str()
            ),
            ("noaa-goes19", "G19", "GLM-L2-LCFA")
        );
        let west = load_glm_config(
            &models_dir().join("glm-goes18.yaml"),
            "http://ingester:8082/ingest",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (west.bucket.as_str(), west.satellite.as_str()),
            ("noaa-goes18", "G18")
        );
        for c in [&east, &west] {
            assert_eq!(c.poll_interval_secs, 15, "granules arrive every 20 s");
            assert_eq!(
                c.backfill_minutes, 30,
                "must stay far below 24 h of retention"
            );
            assert!(
                c.download_base_url.is_none(),
                "production downloads from the public bucket"
            );
        }
    }

    #[test]
    fn only_glm_sources_are_picked_up_and_each_satellite_exactly_once() {
        // Scan every model config: non-GLM sources must yield None (not an error),
        // and the GLM ones must be exactly the two satellites.
        let mut found = Vec::new();
        for entry in std::fs::read_dir(models_dir()).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml") {
                if let Ok(Some(c)) = load_glm_config(&path, "http://i") {
                    found.push((c.id, c.satellite));
                }
            }
        }
        found.sort();
        assert_eq!(
            found,
            vec![
                ("glm-goes18".to_string(), "G18".to_string()),
                ("glm-goes19".to_string(), "G19".to_string())
            ]
        );
    }

    #[test]
    fn a_glm_source_without_a_satellite_is_a_config_error_not_a_silent_default() {
        let dir = tempfile::tempdir().unwrap();
        let bad = std::fs::read_to_string(models_dir().join("glm-goes19.yaml"))
            .unwrap()
            .replace("  satellite: G19", "  # satellite removed");
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, bad).unwrap();
        let err = load_glm_config(&path, "http://i").unwrap_err().to_string();
        assert!(err.contains("satellite"), "{err}");
    }
}
