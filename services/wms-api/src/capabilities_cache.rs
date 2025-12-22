//! Capabilities document caching.
//!
//! Caches generated WMS and WMTS capabilities XML documents with a configurable TTL.
//! This reduces database queries for frequently requested capabilities documents
//! while ensuring data remains reasonably fresh.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Cached capabilities document with generation timestamp.
struct CachedCapabilities {
    xml: String,
    generated_at: Instant,
}

/// Cache for WMS and WMTS capabilities documents.
/// 
/// Both capabilities documents are cached independently with a shared TTL.
/// Cache is invalidated on:
/// - TTL expiration
/// - Data ingestion
/// - Data cleanup/expiration
/// - Configuration reload
pub struct CapabilitiesCache {
    wms_xml: RwLock<Option<CachedCapabilities>>,
    wmts_xml: RwLock<Option<CachedCapabilities>>,
    ttl: Duration,
}

impl CapabilitiesCache {
    /// Create a new capabilities cache with the specified TTL in seconds.
    pub fn new(ttl_secs: u64) -> Self {
        info!(ttl_secs = ttl_secs, "Initializing capabilities cache");
        Self {
            wms_xml: RwLock::new(None),
            wmts_xml: RwLock::new(None),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get cached WMS capabilities if still valid.
    pub async fn get_wms(&self) -> Option<String> {
        let guard = self.wms_xml.read().await;
        if let Some(cached) = guard.as_ref() {
            if cached.generated_at.elapsed() < self.ttl {
                debug!("WMS capabilities cache hit");
                return Some(cached.xml.clone());
            }
            debug!("WMS capabilities cache expired");
        }
        None
    }

    /// Store WMS capabilities in cache.
    pub async fn set_wms(&self, xml: String) {
        let mut guard = self.wms_xml.write().await;
        *guard = Some(CachedCapabilities {
            xml,
            generated_at: Instant::now(),
        });
        debug!("WMS capabilities cached");
    }

    /// Get cached WMTS capabilities if still valid.
    pub async fn get_wmts(&self) -> Option<String> {
        let guard = self.wmts_xml.read().await;
        if let Some(cached) = guard.as_ref() {
            if cached.generated_at.elapsed() < self.ttl {
                debug!("WMTS capabilities cache hit");
                return Some(cached.xml.clone());
            }
            debug!("WMTS capabilities cache expired");
        }
        None
    }

    /// Store WMTS capabilities in cache.
    pub async fn set_wmts(&self, xml: String) {
        let mut guard = self.wmts_xml.write().await;
        *guard = Some(CachedCapabilities {
            xml,
            generated_at: Instant::now(),
        });
        debug!("WMTS capabilities cached");
    }

    /// Invalidate both caches.
    /// Called when data changes (ingestion, cleanup, config reload).
    pub async fn invalidate(&self) {
        let mut wms_guard = self.wms_xml.write().await;
        let mut wmts_guard = self.wmts_xml.write().await;
        *wms_guard = None;
        *wmts_guard = None;
        debug!("Capabilities cache invalidated");
    }

    /// Get the configured TTL.
    pub fn ttl_secs(&self) -> u64 {
        self.ttl.as_secs()
    }
}

/// Create a shared capabilities cache from environment configuration.
/// 
/// Environment variable: CAPABILITIES_CACHE_TTL_SECS (default: 120)
pub fn create_capabilities_cache() -> Arc<CapabilitiesCache> {
    let ttl_secs = std::env::var("CAPABILITIES_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    
    Arc::new(CapabilitiesCache::new(ttl_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_hit_within_ttl() {
        let cache = CapabilitiesCache::new(60);
        cache.set_wms("test xml".to_string()).await;
        
        let result = cache.get_wms().await;
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "test xml");
    }

    #[tokio::test]
    async fn test_cache_miss_when_empty() {
        let cache = CapabilitiesCache::new(60);
        let result = cache.get_wms().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_invalidate_clears_cache() {
        let cache = CapabilitiesCache::new(60);
        cache.set_wms("wms xml".to_string()).await;
        cache.set_wmts("wmts xml".to_string()).await;
        
        cache.invalidate().await;
        
        assert!(cache.get_wms().await.is_none());
        assert!(cache.get_wmts().await.is_none());
    }

    #[tokio::test]
    async fn test_cache_expires_after_ttl() {
        let cache = CapabilitiesCache::new(0); // 0 second TTL = immediate expiration
        cache.set_wms("test xml".to_string()).await;
        
        // Even with 0 TTL, there's a tiny window, so we sleep a tiny bit
        tokio::time::sleep(Duration::from_millis(10)).await;
        
        let result = cache.get_wms().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_wms_and_wmts_are_independent() {
        let cache = CapabilitiesCache::new(60);
        
        // Set only WMS
        cache.set_wms("wms content".to_string()).await;
        
        // WMTS should still be empty
        assert!(cache.get_wms().await.is_some());
        assert!(cache.get_wmts().await.is_none());
        
        // Set WMTS
        cache.set_wmts("wmts content".to_string()).await;
        
        // Both should be present
        assert_eq!(cache.get_wms().await.unwrap(), "wms content");
        assert_eq!(cache.get_wmts().await.unwrap(), "wmts content");
    }

    #[tokio::test]
    async fn test_cache_overwrites_on_set() {
        let cache = CapabilitiesCache::new(60);
        
        cache.set_wms("first".to_string()).await;
        assert_eq!(cache.get_wms().await.unwrap(), "first");
        
        cache.set_wms("second".to_string()).await;
        assert_eq!(cache.get_wms().await.unwrap(), "second");
    }

    #[tokio::test]
    async fn test_ttl_secs_returns_configured_value() {
        let cache = CapabilitiesCache::new(120);
        assert_eq!(cache.ttl_secs(), 120);
        
        let cache2 = CapabilitiesCache::new(300);
        assert_eq!(cache2.ttl_secs(), 300);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        use std::sync::Arc;
        
        let cache = Arc::new(CapabilitiesCache::new(60));
        
        // Spawn multiple readers and writers
        let mut handles = vec![];
        
        for i in 0..10 {
            let cache_clone = cache.clone();
            let handle = tokio::spawn(async move {
                if i % 2 == 0 {
                    cache_clone.set_wms(format!("content_{}", i)).await;
                } else {
                    let _ = cache_clone.get_wms().await;
                }
            });
            handles.push(handle);
        }
        
        // All should complete without panic
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Cache should have some content
        assert!(cache.get_wms().await.is_some());
    }
}
