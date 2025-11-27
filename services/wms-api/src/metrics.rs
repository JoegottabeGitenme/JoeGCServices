//! Application metrics collection and reporting.

use metrics::{counter, gauge, histogram};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Metrics collector for the WMS API.
#[derive(Debug)]
pub struct MetricsCollector {
    /// Request counts
    pub wms_requests: AtomicU64,
    pub wmts_requests: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub minio_reads: AtomicU64,
    pub minio_read_bytes: AtomicU64,
    
    /// Render stats
    pub renders_total: AtomicU64,
    pub render_errors: AtomicU64,
    
    /// Timing stats (stored as microseconds for atomic ops)
    render_times: RwLock<TimingStats>,
    minio_times: RwLock<TimingStats>,
    
    /// Start time for uptime calculation
    start_time: Instant,
}

#[derive(Debug, Default)]
struct TimingStats {
    count: u64,
    total_us: u64,
    min_us: u64,
    max_us: u64,
    last_us: u64,
}

impl TimingStats {
    fn record(&mut self, duration_us: u64) {
        self.count += 1;
        self.total_us += duration_us;
        self.last_us = duration_us;
        if self.min_us == 0 || duration_us < self.min_us {
            self.min_us = duration_us;
        }
        if duration_us > self.max_us {
            self.max_us = duration_us;
        }
    }
    
    fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            (self.total_us as f64 / self.count as f64) / 1000.0
        }
    }
    
    fn last_ms(&self) -> f64 {
        self.last_us as f64 / 1000.0
    }
    
    fn min_ms(&self) -> f64 {
        self.min_us as f64 / 1000.0
    }
    
    fn max_ms(&self) -> f64 {
        self.max_us as f64 / 1000.0
    }
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            wms_requests: AtomicU64::new(0),
            wmts_requests: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            minio_reads: AtomicU64::new(0),
            minio_read_bytes: AtomicU64::new(0),
            renders_total: AtomicU64::new(0),
            render_errors: AtomicU64::new(0),
            render_times: RwLock::new(TimingStats::default()),
            minio_times: RwLock::new(TimingStats::default()),
            start_time: Instant::now(),
        }
    }
    
    /// Record a WMS request
    pub fn record_wms_request(&self) {
        self.wms_requests.fetch_add(1, Ordering::Relaxed);
        counter!("wms_requests_total").increment(1);
    }
    
    /// Record a WMTS request
    pub fn record_wmts_request(&self) {
        self.wmts_requests.fetch_add(1, Ordering::Relaxed);
        counter!("wmts_requests_total").increment(1);
    }
    
    /// Record a cache hit
    pub async fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        counter!("cache_hits_total").increment(1);
    }
    
    /// Record a cache miss
    pub async fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        counter!("cache_misses_total").increment(1);
    }
    
    /// Record a MinIO read operation
    pub async fn record_minio_read(&self, bytes: u64, duration_us: u64) {
        self.minio_reads.fetch_add(1, Ordering::Relaxed);
        self.minio_read_bytes.fetch_add(bytes, Ordering::Relaxed);
        counter!("minio_reads_total").increment(1);
        counter!("minio_read_bytes_total").increment(bytes);
        histogram!("minio_read_duration_ms").record(duration_us as f64 / 1000.0);
        
        let mut times = self.minio_times.write().await;
        times.record(duration_us);
    }
    
    /// Record a render operation
    pub async fn record_render(&self, duration_us: u64, success: bool) {
        self.renders_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.render_errors.fetch_add(1, Ordering::Relaxed);
        }
        counter!("renders_total").increment(1);
        histogram!("render_duration_ms").record(duration_us as f64 / 1000.0);
        
        let mut times = self.render_times.write().await;
        times.record(duration_us);
    }
    
    /// Get current metrics snapshot
    pub async fn snapshot(&self) -> MetricsSnapshot {
        let render_times = self.render_times.read().await;
        let minio_times = self.minio_times.read().await;
        
        let cache_hits = self.cache_hits.load(Ordering::Relaxed);
        let cache_misses = self.cache_misses.load(Ordering::Relaxed);
        let cache_total = cache_hits + cache_misses;
        let cache_hit_rate = if cache_total > 0 {
            (cache_hits as f64 / cache_total as f64) * 100.0
        } else {
            0.0
        };
        
        MetricsSnapshot {
            uptime_secs: self.start_time.elapsed().as_secs(),
            
            wms_requests: self.wms_requests.load(Ordering::Relaxed),
            wmts_requests: self.wmts_requests.load(Ordering::Relaxed),
            
            cache_hits,
            cache_misses,
            cache_hit_rate,
            
            minio_reads: self.minio_reads.load(Ordering::Relaxed),
            minio_read_bytes: self.minio_read_bytes.load(Ordering::Relaxed),
            minio_avg_ms: minio_times.avg_ms(),
            minio_last_ms: minio_times.last_ms(),
            
            renders_total: self.renders_total.load(Ordering::Relaxed),
            render_errors: self.render_errors.load(Ordering::Relaxed),
            render_avg_ms: render_times.avg_ms(),
            render_last_ms: render_times.last_ms(),
            render_min_ms: render_times.min_ms(),
            render_max_ms: render_times.max_ms(),
        }
    }
    
    /// Reset all counters (useful for testing)
    pub async fn reset(&self) {
        self.wms_requests.store(0, Ordering::Relaxed);
        self.wmts_requests.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.minio_reads.store(0, Ordering::Relaxed);
        self.minio_read_bytes.store(0, Ordering::Relaxed);
        self.renders_total.store(0, Ordering::Relaxed);
        self.render_errors.store(0, Ordering::Relaxed);
        
        *self.render_times.write().await = TimingStats::default();
        *self.minio_times.write().await = TimingStats::default();
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of current metrics for JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub uptime_secs: u64,
    
    // Request counts
    pub wms_requests: u64,
    pub wmts_requests: u64,
    
    // Cache stats
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_hit_rate: f64,
    
    // MinIO stats
    pub minio_reads: u64,
    pub minio_read_bytes: u64,
    pub minio_avg_ms: f64,
    pub minio_last_ms: f64,
    
    // Render stats
    pub renders_total: u64,
    pub render_errors: u64,
    pub render_avg_ms: f64,
    pub render_last_ms: f64,
    pub render_min_ms: f64,
    pub render_max_ms: f64,
}

/// Timer guard for measuring operation duration.
pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }
    
    pub fn elapsed_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }
    
    pub fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_micros() as f64 / 1000.0
    }
}

/// System resource statistics (from /proc on Linux).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemStats {
    /// Memory used in bytes
    pub memory_used_bytes: u64,
    /// Memory total in bytes
    pub memory_total_bytes: u64,
    /// Memory usage percentage
    pub memory_percent: f64,
    /// CPU usage percentage (approximate)
    pub cpu_percent: f64,
    /// Number of threads
    pub num_threads: u32,
}

impl SystemStats {
    /// Read current process stats from /proc (Linux only).
    pub fn read() -> Self {
        let mut stats = SystemStats::default();
        
        // Read /proc/self/statm for memory info (values are in pages)
        if let Ok(content) = std::fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 2 {
                let page_size = 4096u64; // typical page size
                let _total_pages: u64 = parts[0].parse().unwrap_or(0);
                let resident_pages: u64 = parts[1].parse().unwrap_or(0);
                stats.memory_used_bytes = resident_pages * page_size;
            }
        }
        
        // Read /proc/meminfo for total system memory
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let kb: u64 = parts[1].parse().unwrap_or(0);
                        stats.memory_total_bytes = kb * 1024;
                    }
                    break;
                }
            }
        }
        
        if stats.memory_total_bytes > 0 {
            stats.memory_percent = (stats.memory_used_bytes as f64 / stats.memory_total_bytes as f64) * 100.0;
        }
        
        // Read /proc/self/stat for thread count
        if let Ok(content) = std::fs::read_to_string("/proc/self/stat") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            // Field 20 (0-indexed 19) is num_threads
            if parts.len() > 19 {
                stats.num_threads = parts[19].parse().unwrap_or(0);
            }
        }
        
        stats
    }
}
