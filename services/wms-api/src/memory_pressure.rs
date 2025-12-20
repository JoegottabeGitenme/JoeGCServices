//! Memory pressure monitoring and cache eviction.
//!
//! This module monitors system memory usage and triggers cache evictions
//! when memory pressure exceeds configured thresholds.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{info, warn, debug};

use crate::state::AppState;

/// Memory pressure monitor that runs in the background.
pub struct MemoryPressureMonitor {
    state: Arc<AppState>,
    /// Memory limit in bytes (0 = auto-detect)
    memory_limit_bytes: u64,
    /// Threshold at which to start evicting (0.0-1.0)
    threshold: f64,
    /// Target memory usage after eviction (0.0-1.0)
    target: f64,
    /// Check interval
    check_interval: Duration,
}

impl MemoryPressureMonitor {
    /// Create a new memory pressure monitor from configuration.
    pub fn new(state: Arc<AppState>) -> Self {
        // Read config values first to avoid borrow conflicts
        let memory_limit_mb = state.optimization_config.memory_limit_mb;
        let threshold = state.optimization_config.memory_pressure_threshold;
        let target = state.optimization_config.memory_pressure_target;
        let check_interval_secs = state.optimization_config.memory_check_interval_secs;
        
        // Determine memory limit
        let memory_limit_bytes = if memory_limit_mb > 0 {
            memory_limit_mb as u64 * 1024 * 1024
        } else {
            // Auto-detect from cgroup or system
            detect_memory_limit()
        };
        
        Self {
            state,
            memory_limit_bytes,
            threshold,
            target,
            check_interval: Duration::from_secs(check_interval_secs),
        }
    }
    
    /// Run the memory pressure monitor forever.
    pub async fn run_forever(&self) {
        info!(
            memory_limit_mb = self.memory_limit_bytes / (1024 * 1024),
            threshold_percent = self.threshold * 100.0,
            target_percent = self.target * 100.0,
            check_interval_secs = self.check_interval.as_secs(),
            "Memory pressure monitor started"
        );
        
        let mut ticker = interval(self.check_interval);
        
        loop {
            ticker.tick().await;
            
            if let Err(e) = self.check_and_evict().await {
                warn!(error = %e, "Memory pressure check failed");
            }
        }
    }
    
    /// Check memory pressure and evict if necessary.
    async fn check_and_evict(&self) -> Result<(), String> {
        let current_rss = get_process_rss();
        let usage_ratio = current_rss as f64 / self.memory_limit_bytes as f64;
        
        debug!(
            current_rss_mb = current_rss / (1024 * 1024),
            limit_mb = self.memory_limit_bytes / (1024 * 1024),
            usage_percent = usage_ratio * 100.0,
            threshold_percent = self.threshold * 100.0,
            "Memory pressure check"
        );
        
        // Record metrics
        metrics::gauge!("memory_pressure_usage_ratio").set(usage_ratio);
        metrics::gauge!("memory_pressure_limit_bytes").set(self.memory_limit_bytes as f64);
        
        if usage_ratio > self.threshold {
            warn!(
                current_rss_mb = current_rss / (1024 * 1024),
                limit_mb = self.memory_limit_bytes / (1024 * 1024),
                usage_percent = format!("{:.1}%", usage_ratio * 100.0),
                threshold_percent = format!("{:.1}%", self.threshold * 100.0),
                "Memory pressure threshold exceeded, starting eviction"
            );
            
            self.evict_to_target().await?;
        }
        
        Ok(())
    }
    
    /// Evict cache entries until we're below the target memory usage.
    async fn evict_to_target(&self) -> Result<(), String> {
        let target_bytes = (self.memory_limit_bytes as f64 * self.target) as u64;
        let mut total_evicted = 0usize;
        
        // Get current cache stats
        let chunk_stats = self.state.grid_processor_factory.cache_stats().await;
        let l1_stats = self.state.tile_memory_cache.stats();
        
        let chunk_cache_mb = chunk_stats.memory_bytes as f64 / (1024.0 * 1024.0);
        
        info!(
            chunk_cache_mb = chunk_cache_mb,
            l1_cache_bytes = l1_stats.size_bytes.load(std::sync::atomic::Ordering::Relaxed),
            target_rss_mb = target_bytes / (1024 * 1024),
            "Starting cache eviction"
        );
        
        // Calculate how much we need to free
        let current_rss = get_process_rss();
        if current_rss <= target_bytes {
            info!("Memory already below target after GC");
            return Ok(());
        }
        
        let bytes_to_free = current_rss - target_bytes;
        
        // Strategy: Evict in order of data size (largest first)
        // 1. Chunk cache (Zarr chunks, variable size)
        // 2. L1 tile cache (tiles are ~30KB each)
        
        // Evict from chunk cache first (largest entries)
        if chunk_stats.memory_bytes > 0 {
            // Calculate percentage to evict based on how much we need to free
            // If we need to free more than the chunk cache has, evict all of it
            let evict_ratio = (bytes_to_free as f64 / chunk_stats.memory_bytes as f64).min(0.5);
            if evict_ratio > 0.1 {
                // Clear chunk cache completely (no partial eviction API)
                let (evicted, _bytes) = self.state.grid_processor_factory.clear_chunk_cache().await;
                total_evicted += evicted;
                info!(
                    evicted_entries = evicted,
                    evict_ratio = format!("{:.0}%", evict_ratio * 100.0),
                    "Cleared chunk cache"
                );
                metrics::counter!("memory_pressure_chunk_evictions").increment(evicted as u64);
            }
        }
        
        // Check if we freed enough
        let current_rss_after = get_process_rss();
        if current_rss_after <= target_bytes {
            info!(
                total_evicted = total_evicted,
                new_rss_mb = current_rss_after / (1024 * 1024),
                "Memory pressure relieved after chunk cache eviction"
            );
            return Ok(());
        }
        
        // Evict from L1 tile cache if still under pressure
        let l1_size = l1_stats.size_bytes.load(std::sync::atomic::Ordering::Relaxed);
        if l1_size > 0 {
            let remaining_to_free = current_rss_after - target_bytes;
            let evict_ratio = (remaining_to_free as f64 / l1_size as f64).min(0.3);
            if evict_ratio > 0.05 {
                let evicted = self.state.tile_memory_cache.evict_percentage(evict_ratio).await;
                total_evicted += evicted;
                info!(
                    evicted_entries = evicted,
                    evict_ratio = format!("{:.0}%", evict_ratio * 100.0),
                    "Evicted from L1 tile cache"
                );
                metrics::counter!("memory_pressure_l1_evictions").increment(evicted as u64);
            }
        }
        
        let final_rss = get_process_rss();
        info!(
            total_evicted = total_evicted,
            initial_rss_mb = current_rss / (1024 * 1024),
            final_rss_mb = final_rss / (1024 * 1024),
            freed_mb = (current_rss.saturating_sub(final_rss)) / (1024 * 1024),
            "Memory pressure eviction complete"
        );
        
        metrics::counter!("memory_pressure_eviction_runs").increment(1);
        
        Ok(())
    }
}

/// Detect memory limit from cgroup (for containerized environments) or system.
fn detect_memory_limit() -> u64 {
    // Try cgroup v2 first
    if let Ok(limit) = std::fs::read_to_string("/sys/fs/cgroup/memory.max") {
        if let Ok(bytes) = limit.trim().parse::<u64>() {
            if bytes < u64::MAX / 2 {
                return bytes;
            }
        }
    }
    
    // Try cgroup v1
    if let Ok(limit) = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
        if let Ok(bytes) = limit.trim().parse::<u64>() {
            if bytes < u64::MAX / 2 {
                return bytes;
            }
        }
    }
    
    // Fall back to system memory
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        return kb * 1024;
                    }
                }
            }
        }
    }
    
    // Default to 16GB if we can't detect
    16 * 1024 * 1024 * 1024
}

/// Get the current process RSS (Resident Set Size) in bytes.
fn get_process_rss() -> u64 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        return kb * 1024;
                    }
                }
            }
        }
    }
    0
}
