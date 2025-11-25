//! Redis-based tile cache for rendered images.

use bytes::Bytes;
use redis::{aio::MultiplexedConnection, AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use wms_common::{BoundingBox, CrsCode, WmsError, WmsResult};

/// Redis tile cache client.
pub struct TileCache {
    conn: MultiplexedConnection,
    default_ttl: Duration,
}

impl TileCache {
    /// Connect to Redis.
    pub async fn connect(redis_url: &str) -> WmsResult<Self> {
        let client = Client::open(redis_url)
            .map_err(|e| WmsError::CacheError(format!("Redis connection failed: {}", e)))?;

        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| WmsError::CacheError(format!("Redis connection failed: {}", e)))?;

        Ok(Self {
            conn,
            default_ttl: Duration::from_secs(3600), // 1 hour default
        })
    }

    /// Get a cached tile.
    pub async fn get(&mut self, key: &CacheKey) -> WmsResult<Option<Bytes>> {
        let key_str = key.to_string();

        let result: Option<Vec<u8>> = self.conn
            .get(&key_str)
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache get failed: {}", e)))?;

        Ok(result.map(Bytes::from))
    }

    /// Store a tile in cache.
    pub async fn set(&mut self, key: &CacheKey, data: &[u8], ttl: Option<Duration>) -> WmsResult<()> {
        let key_str = key.to_string();
        let ttl = ttl.unwrap_or(self.default_ttl);

        self.conn
            .set_ex(&key_str, data, ttl.as_secs())
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache set failed: {}", e)))?;

        Ok(())
    }

    /// Check if a key exists.
    pub async fn exists(&mut self, key: &CacheKey) -> WmsResult<bool> {
        let key_str = key.to_string();

        let exists: bool = self.conn
            .exists(&key_str)
            .await
            .map_err(|e| WmsError::CacheError(format!("Cache exists check failed: {}", e)))?;

        Ok(exists)
    }

    /// Delete a specific key.
    pub async fn delete(&mut self, key: &CacheKey) -> WmsResult<()> {
        let key_str = key.to_string();

        self.conn
            .del(&key_str)
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

    /// Delete keys matching a pattern.
    async fn delete_by_pattern(&mut self, pattern: &str) -> WmsResult<u64> {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(pattern)
            .query_async(&mut self.conn)
            .await
            .map_err(|e| WmsError::CacheError(format!("Pattern search failed: {}", e)))?;

        if keys.is_empty() {
            return Ok(0);
        }

        let count = keys.len() as u64;
        
        for key in keys {
            let _: () = self.conn
                .del(&key)
                .await
                .map_err(|e| WmsError::CacheError(format!("Delete failed: {}", e)))?;
        }

        Ok(count)
    }

    /// Get cache statistics.
    pub async fn stats(&mut self) -> WmsResult<CacheStats> {
        let info: String = redis::cmd("INFO")
            .arg("memory")
            .query_async(&mut self.conn)
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
            .query_async(&mut self.conn)
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
}
