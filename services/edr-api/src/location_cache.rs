//! In-memory cache for EDR location query responses.
//!
//! Caches full CoverageJSON responses for named location queries,
//! enabling fast repeated queries at the same location.
//!
//! ## Cache Key Structure
//! Keys are composed of:
//! collection_id:location_id:instance_id:datetime:params:z:format:run:forecast_hour
//!
//! `run` and `forecast_hour` are in the key even though `location_query`
//! currently ignores them: a key must name every input that can change the
//! body, so honouring them later cannot serve one run's answer for another.
//!
//! ## Eviction Strategy
//! - Memory-based LRU eviction when size limit is exceeded
//! - TTL-based expiration on read (lazy)
//! - A per-entry TTL shorter than the default for responses about the latest
//!   data (see [`LocationCache::put`])
//!
//! TODO: Add background pre-caching for configured high-traffic locations
//! when new data is ingested. This would involve:
//! 1. Adding a precaching config section to locations.yaml
//! 2. Creating a LocationWarmer background task (similar to ChunkWarmer)
//! 3. Triggering pre-computation on data ingest events

use bytes::Bytes;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cache key for location queries.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct LocationCacheKey {
    /// Collection ID (e.g., "hrrr-surface")
    pub collection_id: String,
    /// Location ID (e.g., "KJFK")
    pub location_id: String,
    /// Instance ID if specified (e.g., "2024-12-29T12:00:00Z")
    pub instance_id: Option<String>,
    /// Datetime parameter
    pub datetime: Option<String>,
    /// Parameter names (sorted, comma-separated)
    pub parameters: Option<String>,
    /// Z parameter (vertical levels)
    pub z: Option<String>,
    /// Output format (e.g., "geojson", "covjson")
    pub format: Option<String>,
    /// `run` parameter (see [`Self::with_run`]).
    pub run: Option<String>,
    /// `forecast-hour` parameter (see [`Self::with_run`]).
    pub forecast_hour: Option<String>,
}

impl LocationCacheKey {
    /// Create a new cache key.
    pub fn new(
        collection_id: impl Into<String>,
        location_id: impl Into<String>,
        instance_id: Option<String>,
        datetime: Option<String>,
        parameters: Option<String>,
        z: Option<String>,
        format: Option<String>,
    ) -> Self {
        Self {
            collection_id: collection_id.into(),
            location_id: location_id.into(),
            instance_id,
            datetime,
            parameters,
            z,
            format,
            run: None,
            forecast_hour: None,
        }
    }

    /// Add the `run` and `forecast-hour` the query was made with, so two URLs
    /// that differ only in them never share an entry.
    pub fn with_run(mut self, run: Option<String>, forecast_hour: Option<String>) -> Self {
        self.run = run;
        self.forecast_hour = forecast_hour;
        self
    }

    /// Convert to a string key for the LRU cache.
    fn to_string_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.collection_id,
            self.location_id,
            self.instance_id.as_deref().unwrap_or("-"),
            self.datetime.as_deref().unwrap_or("-"),
            self.parameters.as_deref().unwrap_or("-"),
            self.z.as_deref().unwrap_or("-"),
            self.format.as_deref().unwrap_or("-"),
            self.run.as_deref().unwrap_or("-"),
            self.forecast_hour.as_deref().unwrap_or("-"),
        )
    }
}

/// Cached response entry.
struct CachedResponse {
    /// The serialized JSON response.
    data: Bytes,
    /// Content-Type header value.
    content_type: String,
    /// When this entry was inserted.
    inserted_at: Instant,
    /// Time-to-live for this entry.
    ttl: Duration,
}

impl CachedResponse {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }

    fn size(&self) -> usize {
        self.data.len() + self.content_type.len()
    }
}

/// Statistics for the location cache.
#[derive(Default)]
pub struct LocationCacheStats {
    /// Total cache hits.
    pub hits: AtomicU64,
    /// Total cache misses.
    pub misses: AtomicU64,
    /// Total entries evicted.
    pub evictions: AtomicU64,
    /// Total entries expired via TTL.
    pub expired: AtomicU64,
    /// Current cache size in bytes.
    pub size_bytes: AtomicU64,
    /// Current number of entries.
    pub entry_count: AtomicU64,
}

impl LocationCacheStats {
    /// Calculate cache hit rate as a percentage (0-100).
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            (hits as f64 / total as f64) * 100.0
        }
    }
}

/// In-memory LRU cache for location query responses.
pub struct LocationCache {
    cache: Arc<RwLock<LruCache<String, CachedResponse>>>,
    max_bytes: u64,
    default_ttl: Duration,
    stats: Arc<LocationCacheStats>,
    current_bytes: Arc<AtomicU64>,
}

impl LocationCache {
    /// Create a new location cache.
    ///
    /// # Arguments
    /// * `max_mb` - Maximum cache size in megabytes
    /// * `ttl_secs` - Default time-to-live in seconds
    pub fn new(max_mb: usize, ttl_secs: u64) -> Self {
        let max_entries = NonZeroUsize::new(10000).unwrap();
        let max_bytes = (max_mb * 1024 * 1024) as u64;

        tracing::info!(
            "LocationCache initialized: max_mb={}, ttl_secs={}, max_entries={}",
            max_mb,
            ttl_secs,
            max_entries
        );

        Self {
            cache: Arc::new(RwLock::new(LruCache::new(max_entries))),
            max_bytes,
            default_ttl: Duration::from_secs(ttl_secs),
            stats: Arc::new(LocationCacheStats::default()),
            current_bytes: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get a cached response.
    pub async fn get(&self, key: &LocationCacheKey) -> Option<(Bytes, String)> {
        let string_key = key.to_string_key();
        let mut cache: tokio::sync::RwLockWriteGuard<'_, LruCache<String, CachedResponse>> =
            self.cache.write().await;

        if let Some(entry) = cache.get(&string_key) {
            if entry.is_expired() {
                // Remove expired entry
                if let Some(removed) = cache.pop(&string_key) {
                    self.current_bytes
                        .fetch_sub(removed.size() as u64, Ordering::Relaxed);
                    self.stats.expired.fetch_add(1, Ordering::Relaxed);
                    self.stats.entry_count.fetch_sub(1, Ordering::Relaxed);
                }
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                None
            } else {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                Some((entry.data.clone(), entry.content_type.clone()))
            }
        } else {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Store a response in the cache.
    ///
    /// `ttl` shortens this entry's lifetime; `None` uses the default. It can
    /// only shorten, never extend, the default. WHY: a response about the
    /// latest data (no datetime / instance) goes to the client with a short
    /// `max-age`; if the entry behind it lived for the full default (5 min) a
    /// cache HIT would hand out a body far older than that header admits, and
    /// the CDN would hold it on top.
    pub async fn put(
        &self,
        key: &LocationCacheKey,
        data: Bytes,
        content_type: String,
        ttl: Option<Duration>,
    ) {
        let string_key = key.to_string_key();
        let entry = CachedResponse {
            data,
            content_type,
            inserted_at: Instant::now(),
            ttl: ttl.map_or(self.default_ttl, |t| t.min(self.default_ttl)),
        };
        let entry_size = entry.size() as u64;

        let mut cache: tokio::sync::RwLockWriteGuard<'_, LruCache<String, CachedResponse>> =
            self.cache.write().await;

        // Check if we need to evict entries to make room
        let current = self.current_bytes.load(Ordering::Relaxed);
        if current + entry_size > self.max_bytes {
            // Evict oldest entries until we have room
            let target = self.max_bytes.saturating_sub(entry_size);
            let mut evicted_bytes = 0u64;
            let mut evicted_count = 0u64;

            while self.current_bytes.load(Ordering::Relaxed) > target {
                if let Some((_, removed)) = cache.pop_lru() {
                    evicted_bytes += removed.size() as u64;
                    evicted_count += 1;
                    self.current_bytes
                        .fetch_sub(removed.size() as u64, Ordering::Relaxed);
                } else {
                    break;
                }
            }

            if evicted_count > 0 {
                self.stats
                    .evictions
                    .fetch_add(evicted_count, Ordering::Relaxed);
                self.stats
                    .entry_count
                    .fetch_sub(evicted_count, Ordering::Relaxed);
                tracing::debug!(
                    "LocationCache evicted {} entries ({} bytes)",
                    evicted_count,
                    evicted_bytes
                );
            }
        }

        // If replacing an existing entry, subtract its size first
        if let Some(old) = cache.push(string_key, entry) {
            self.current_bytes
                .fetch_sub(old.1.size() as u64, Ordering::Relaxed);
        } else {
            self.stats.entry_count.fetch_add(1, Ordering::Relaxed);
        }

        self.current_bytes.fetch_add(entry_size, Ordering::Relaxed);
        self.stats.size_bytes.store(
            self.current_bytes.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &LocationCacheStats {
        &self.stats
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) {
        let mut cache: tokio::sync::RwLockWriteGuard<'_, LruCache<String, CachedResponse>> =
            self.cache.write().await;
        let count = cache.len() as u64;
        cache.clear();
        self.current_bytes.store(0, Ordering::Relaxed);
        self.stats.size_bytes.store(0, Ordering::Relaxed);
        self.stats.entry_count.store(0, Ordering::Relaxed);
        tracing::info!("LocationCache cleared {} entries", count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_put_get() {
        let cache = LocationCache::new(10, 300);

        let key = LocationCacheKey::new("hrrr-surface", "KJFK", None, None, None, None, None);
        let data = Bytes::from(r#"{"type":"Coverage"}"#);
        let content_type = "application/vnd.cov+json".to_string();

        cache
            .put(&key, data.clone(), content_type.clone(), None)
            .await;

        let result = cache.get(&key).await;
        assert!(result.is_some());
        let (cached_data, cached_ct) = result.unwrap();
        assert_eq!(cached_data, data);
        assert_eq!(cached_ct, content_type);
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = LocationCache::new(10, 300);
        let key = LocationCacheKey::new("hrrr-surface", "UNKNOWN", None, None, None, None, None);

        let result = cache.get(&key).await;
        assert!(result.is_none());
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_cache_key_with_params() {
        let key = LocationCacheKey::new(
            "hrrr-isobaric",
            "KLAX",
            Some("2024-12-29T12:00:00Z".to_string()),
            Some("2024-12-29T12:00:00Z".to_string()),
            Some("TMP,UGRD,VGRD".to_string()),
            Some("850".to_string()),
            Some("geojson".to_string()),
        );

        let string_key = key.to_string_key();
        assert!(string_key.contains("hrrr-isobaric"));
        assert!(string_key.contains("KLAX"));
        assert!(string_key.contains("TMP,UGRD,VGRD"));
        assert!(string_key.contains("850"));
        assert!(string_key.contains("geojson"));
    }

    #[test]
    fn run_and_forecast_hour_are_part_of_the_key() {
        let base = || LocationCacheKey::new("hrrr-surface", "KJFK", None, None, None, None, None);
        let plain = base().to_string_key();
        let run = base().with_run(Some("2024-12-29T12:00:00Z".to_string()), None);
        let hour = base().with_run(None, Some("6".to_string()));
        assert_ne!(run.to_string_key(), plain);
        assert_ne!(hour.to_string_key(), plain);
        assert_ne!(run.to_string_key(), hour.to_string_key());
        // ...and an absent run is not the same key as a run called "-".
        assert_eq!(base().with_run(None, None).to_string_key(), plain);
    }

    /// The finding being fixed: a latest response was cached for the default
    /// 5 minutes while the header said 60 s.
    #[tokio::test]
    async fn an_entry_with_a_short_ttl_expires_before_the_default() {
        let cache = LocationCache::new(10, 300);
        let latest = LocationCacheKey::new("mrms", "KOKC", None, None, None, None, None);
        let pinned = LocationCacheKey::new(
            "mrms",
            "KOKC",
            None,
            Some("2024-12-29T12:00:00Z".to_string()),
            None,
            None,
            None,
        );
        cache
            .put(
                &latest,
                Bytes::from("latest"),
                "text/plain".to_string(),
                Some(Duration::from_millis(5)),
            )
            .await;
        cache
            .put(
                &pinned,
                Bytes::from("pinned"),
                "text/plain".to_string(),
                None,
            )
            .await;

        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(
            cache.get(&latest).await.is_none(),
            "a latest entry must not outlive its short TTL"
        );
        assert!(
            cache.get(&pinned).await.is_some(),
            "an entry without an override keeps the default TTL"
        );
    }

    #[tokio::test]
    async fn a_ttl_override_never_extends_the_default() {
        let cache = LocationCache::new(10, 0);
        let key = LocationCacheKey::new("col", "loc", None, None, None, None, None);
        cache
            .put(
                &key,
                Bytes::from("data"),
                "text/plain".to_string(),
                Some(Duration::from_secs(3600)),
            )
            .await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache = LocationCache::new(10, 300);

        let key1 = LocationCacheKey::new("col", "loc1", None, None, None, None, None);
        let key2 = LocationCacheKey::new("col", "loc2", None, None, None, None, None);

        // Miss
        cache.get(&key1).await;
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 1);

        // Put
        cache
            .put(&key1, Bytes::from("data"), "text/plain".to_string(), None)
            .await;
        assert_eq!(cache.stats().entry_count.load(Ordering::Relaxed), 1);

        // Hit
        cache.get(&key1).await;
        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 1);

        // Another miss
        cache.get(&key2).await;
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 2);

        // Hit rate should be 33% (1 hit, 2 misses)
        let hit_rate = cache.stats().hit_rate();
        assert!((hit_rate - 33.33).abs() < 1.0);
    }
}
