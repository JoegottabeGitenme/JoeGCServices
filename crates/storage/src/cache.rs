//! Redis-based tile cache for rendered images.

use bytes::Bytes;
use redis::{aio::MultiplexedConnection, AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use wms_common::{BoundingBox, CrsCode, WmsError, WmsResult};

/// Redis tile cache client.
///
/// # Degraded mode
///
/// Redis is an **L2 cache** — it accelerates responses but is not required for
/// correctness. `conn` is therefore optional: when Redis is unreachable at
/// startup, [`TileCache::connect`] returns a *disabled* cache instead of an
/// error. All operations then become no-ops (gets miss, sets are dropped), so
/// the service still starts, serves from the L1 in-memory cache and object
/// storage, and — critically — keeps running its background retention/cleanup
/// tasks.
///
/// This exists because a Redis outage once prevented wms-api from starting at
/// all, which silently stopped data retention and filled the disk.
pub struct TileCache {
    conn: Option<MultiplexedConnection>,
    default_ttl: Duration,
}

impl TileCache {
    /// Connect to Redis with a specified TTL for cached tiles.
    ///
    /// Never fails on connection problems: if Redis is unreachable, a disabled
    /// cache is returned (see the type-level docs) and the caller should log a
    /// warning. Check [`TileCache::is_enabled`] to report status.
    ///
    /// # Arguments
    /// - `redis_url`: Redis connection URL (e.g., "redis://redis:6379")
    /// - `ttl_secs`: Time-to-live for cached tiles in seconds (default: 3600 = 1 hour)
    pub async fn connect(redis_url: &str, ttl_secs: u64) -> WmsResult<Self> {
        let default_ttl = Duration::from_secs(ttl_secs);

        let client = match Client::open(redis_url) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    url = redis_url,
                    "Invalid Redis URL - continuing with L2 tile cache DISABLED"
                );
                return Ok(Self {
                    conn: None,
                    default_ttl,
                });
            }
        };

        match client.get_multiplexed_async_connection().await {
            Ok(conn) => Ok(Self {
                conn: Some(conn),
                default_ttl,
            }),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    url = redis_url,
                    "Redis unreachable - continuing with L2 tile cache DISABLED \
                     (service still serves from L1 + object storage, and background \
                     retention keeps running)"
                );
                Ok(Self {
                    conn: None,
                    default_ttl,
                })
            }
        }
    }

    /// Whether the Redis L2 cache is connected and in use.
    pub fn is_enabled(&self) -> bool {
        self.conn.is_some()
    }

    /// Get a cached tile. Always a miss when Redis is disabled.
    pub async fn get(&mut self, key: &CacheKey) -> WmsResult<Option<Bytes>> {
        let Some(conn) = self.conn.as_mut() else {
            return Ok(None);
        };
        let key_str = key.to_string();

        let result: Option<Vec<u8>> = conn
            .get(&key_str)
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache get failed: {}", e)))?;

        Ok(result.map(Bytes::from))
    }

    /// Store a tile in cache. No-op when Redis is disabled.
    pub async fn set(
        &mut self,
        key: &CacheKey,
        data: &[u8],
        ttl: Option<Duration>,
    ) -> WmsResult<()> {
        let ttl = ttl.unwrap_or(self.default_ttl);
        let Some(conn) = self.conn.as_mut() else {
            return Ok(());
        };
        let key_str = key.to_string();

        conn.set_ex::<_, _, ()>(&key_str, data, ttl.as_secs())
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache set failed: {}", e)))?;

        Ok(())
    }

    /// Check if a key exists. Always false when Redis is disabled.
    pub async fn exists(&mut self, key: &CacheKey) -> WmsResult<bool> {
        let Some(conn) = self.conn.as_mut() else {
            return Ok(false);
        };
        let key_str = key.to_string();

        let exists: bool = conn
            .exists(&key_str)
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache exists check failed: {}", e)))?;

        Ok(exists)
    }

    /// Delete a specific key. No-op when Redis is disabled.
    pub async fn delete(&mut self, key: &CacheKey) -> WmsResult<()> {
        let Some(conn) = self.conn.as_mut() else {
            return Ok(());
        };
        let key_str = key.to_string();

        conn.del::<_, ()>(&key_str)
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache delete failed: {}", e)))?;

        Ok(())
    }

    /// Invalidate all tiles for a layer.
    pub async fn invalidate_layer(&mut self, layer: &str) -> WmsResult<u64> {
        let pattern = format!("wms:{}:*", layer);
        self.delete_by_pattern(&pattern).await
    }

    /// Invalidate all tiles for a layer/time combination.
    pub async fn invalidate_layer_time(&mut self, layer: &str, time: &str) -> WmsResult<u64> {
        let pattern = format!("wms:{}:*:*:*:*:{}:*", layer, time);
        self.delete_by_pattern(&pattern).await
    }

    /// Get keys matching a pattern. Empty when Redis is disabled.
    pub async fn keys(&mut self, pattern: &str) -> WmsResult<Vec<String>> {
        let Some(conn) = self.conn.as_mut() else {
            return Ok(Vec::new());
        };
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(pattern)
            .query_async(conn)
            .await
            .map_err(|e| WmsError::CacheError(format!("Pattern search failed: {}", e)))?;

        Ok(keys)
    }

    /// Delete keys matching a pattern. No-op when Redis is disabled.
    async fn delete_by_pattern(&mut self, pattern: &str) -> WmsResult<u64> {
        let keys = self.keys(pattern).await?;

        if keys.is_empty() {
            return Ok(0);
        }

        let count = keys.len() as u64;
        let Some(conn) = self.conn.as_mut() else {
            return Ok(0);
        };

        for key in keys {
            let _: () = conn
                .del(&key)
                .await
                .map_err(|e| WmsError::CacheError(format!("Delete failed: {}", e)))?;
        }

        Ok(count)
    }

    /// Get cache statistics. Zeroed when Redis is disabled.
    pub async fn stats(&mut self) -> WmsResult<CacheStats> {
        let Some(conn) = self.conn.as_mut() else {
            return Ok(CacheStats {
                key_count: 0,
                memory_used: 0,
            });
        };
        let info: String = redis::cmd("INFO")
            .arg("memory")
            .query_async(&mut *conn)
            .await
            .map_err(|e| WmsError::CacheError(format!("Info failed: {}", e)))?;

        // Parse basic stats from INFO output
        let mut used_memory = 0u64;
        for line in info.lines() {
            if line.starts_with("used_memory:") {
                if let Some(val) = line.strip_prefix("used_memory:") {
                    used_memory = val.parse().unwrap_or(0);
                }
            }
        }

        let db_size: u64 = redis::cmd("DBSIZE")
            .query_async(&mut *conn)
            .await
            .map_err(|e| WmsError::CacheError(format!("DBSIZE failed: {}", e)))?;

        Ok(CacheStats {
            key_count: db_size,
            memory_used: used_memory,
        })
    }
}

/// Cache key for WMS tiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheKey {
    pub layer: String,
    pub style: String,
    pub crs: CrsCode,
    pub bbox: BoundingBox,
    pub width: u32,
    pub height: u32,
    pub time: Option<String>,
    pub format: String,
}

impl CacheKey {
    pub fn new(
        layer: impl Into<String>,
        style: impl Into<String>,
        crs: CrsCode,
        bbox: BoundingBox,
        width: u32,
        height: u32,
        time: Option<String>,
        format: impl Into<String>,
    ) -> Self {
        Self {
            layer: layer.into(),
            style: style.into(),
            crs,
            bbox,
            width,
            height,
            time,
            format: format.into(),
        }
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "wms:{}:{}:{}:{}:{}x{}:{}:{}",
            self.layer,
            self.style,
            self.crs,
            self.bbox.cache_key(),
            self.width,
            self.height,
            self.time.as_deref().unwrap_or("current"),
            self.format
        )
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub key_count: u64,
    pub memory_used: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Redis outage must not prevent startup: connect() returns a disabled
    /// cache whose operations are no-ops. Regression guard for the incident
    /// where Redis being down stopped wms-api and therefore data retention.
    #[tokio::test]
    async fn test_connect_unreachable_redis_returns_disabled_cache() {
        // Port 1 is reserved/unused, so this connection always fails fast.
        let cache = TileCache::connect("redis://127.0.0.1:1", 3600).await;
        let mut cache = cache.expect("connect must not error when Redis is down");
        assert!(!cache.is_enabled(), "cache should report disabled");

        let key = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 1.0, 1.0),
            256,
            256,
            None,
            "png",
        );

        // All operations degrade gracefully instead of erroring.
        assert!(cache.get(&key).await.unwrap().is_none());
        assert!(cache.set(&key, b"data", None).await.is_ok());
        assert!(!cache.exists(&key).await.unwrap());
        assert!(cache.delete(&key).await.is_ok());
        assert!(cache.keys("wms:*").await.unwrap().is_empty());
        let stats = cache.stats().await.unwrap();
        assert_eq!(stats.key_count, 0);
        assert_eq!(stats.memory_used, 0);
    }

    #[test]
    fn test_cache_key_format() {
        let key = CacheKey::new(
            "gfs:temperature_2m",
            "gradient",
            CrsCode::Epsg3857,
            BoundingBox::new(-125.0, 24.0, -66.0, 50.0),
            512,
            512,
            Some("2024-01-15T12:00:00Z".to_string()),
            "png",
        );

        let key_str = key.to_string();
        assert!(key_str.starts_with("wms:gfs:temperature_2m:gradient:EPSG:3857"));
        assert!(key_str.contains("512x512"));
    }

    #[test]
    fn test_cache_key_no_time() {
        let key = CacheKey::new(
            "metar:temperature",
            "default",
            CrsCode::Epsg4326,
            BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
            256,
            256,
            None,
            "png",
        );

        let key_str = key.to_string();
        assert!(key_str.contains(":current:"));
    }

    #[test]
    fn test_cache_key_different_crs() {
        let key_3857 = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg3857,
            BoundingBox::new(0.0, 0.0, 100.0, 100.0),
            256,
            256,
            None,
            "png",
        );

        let key_4326 = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 100.0, 100.0),
            256,
            256,
            None,
            "png",
        );

        assert_ne!(key_3857.to_string(), key_4326.to_string());
        assert!(key_3857.to_string().contains("EPSG:3857"));
        assert!(key_4326.to_string().contains("EPSG:4326"));
    }

    #[test]
    fn test_cache_key_different_dimensions() {
        let key_256 = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 100.0, 100.0),
            256,
            256,
            None,
            "png",
        );

        let key_512 = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 100.0, 100.0),
            512,
            512,
            None,
            "png",
        );

        assert_ne!(key_256.to_string(), key_512.to_string());
        assert!(key_256.to_string().contains("256x256"));
        assert!(key_512.to_string().contains("512x512"));
    }

    #[test]
    fn test_cache_key_different_formats() {
        let key_png = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 100.0, 100.0),
            256,
            256,
            None,
            "png",
        );

        let key_webp = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 100.0, 100.0),
            256,
            256,
            None,
            "webp",
        );

        assert_ne!(key_png.to_string(), key_webp.to_string());
        assert!(key_png.to_string().ends_with(":png"));
        assert!(key_webp.to_string().ends_with(":webp"));
    }

    #[test]
    fn test_cache_key_deterministic() {
        let key1 = CacheKey::new(
            "gfs:wind",
            "arrows",
            CrsCode::Epsg3857,
            BoundingBox::new(-100.0, 30.0, -90.0, 40.0),
            512,
            512,
            Some("2024-01-15T00:00:00Z".to_string()),
            "png",
        );

        let key2 = CacheKey::new(
            "gfs:wind",
            "arrows",
            CrsCode::Epsg3857,
            BoundingBox::new(-100.0, 30.0, -90.0, 40.0),
            512,
            512,
            Some("2024-01-15T00:00:00Z".to_string()),
            "png",
        );

        assert_eq!(key1.to_string(), key2.to_string());
    }

    #[test]
    fn test_cache_key_different_bbox() {
        let key1 = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(0.0, 0.0, 10.0, 10.0),
            256,
            256,
            None,
            "png",
        );

        let key2 = CacheKey::new(
            "layer",
            "style",
            CrsCode::Epsg4326,
            BoundingBox::new(10.0, 10.0, 20.0, 20.0),
            256,
            256,
            None,
            "png",
        );

        assert_ne!(key1.to_string(), key2.to_string());
    }
}
