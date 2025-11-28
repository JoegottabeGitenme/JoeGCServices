//! Application metrics collection and reporting.

use metrics::{counter, gauge, histogram};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use std::collections::HashMap;
use storage::TileMemoryCacheStats;

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
    
    /// Per-layer-type timing stats
    layer_type_times: RwLock<HashMap<LayerType, TimingStats>>,
    
    /// Detailed pipeline timing stats
    grib_load_times: RwLock<TimingStats>,
    grib_parse_times: RwLock<TimingStats>,
    resample_times: RwLock<TimingStats>,
    png_encode_times: RwLock<TimingStats>,
    cache_lookup_times: RwLock<TimingStats>,
    
    /// Start time for uptime calculation
    start_time: Instant,
}

/// Layer rendering type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerType {
    Gradient,
    WindBarbs,
    Isolines,
}

impl LayerType {
    /// Classify layer type based on layer name and style
    pub fn from_layer_and_style(layer: &str, style: &str) -> Self {
        // Isolines style takes priority
        if style == "isolines" {
            return LayerType::Isolines;
        }
        
        // Wind barb layers
        if layer.contains("WIND_BARBS") || style == "wind_barbs" {
            return LayerType::WindBarbs;
        }
        
        // Everything else is gradient (temperature, pressure, satellite, radar, etc.)
        LayerType::Gradient
    }
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
            layer_type_times: RwLock::new(HashMap::new()),
            grib_load_times: RwLock::new(TimingStats::default()),
            grib_parse_times: RwLock::new(TimingStats::default()),
            resample_times: RwLock::new(TimingStats::default()),
            png_encode_times: RwLock::new(TimingStats::default()),
            cache_lookup_times: RwLock::new(TimingStats::default()),
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
    
    /// Record a cache hit (L2 Redis cache)
    pub async fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        counter!("cache_hits_total").increment(1);
    }
    
    /// Record a cache miss (L2 Redis cache)
    pub async fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        counter!("cache_misses_total").increment(1);
    }
    
    /// Record L1 cache hit
    pub fn record_l1_cache_hit(&self) {
        counter!("tile_memory_cache_hits_total").increment(1);
    }
    
    /// Record L1 cache miss
    pub fn record_l1_cache_miss(&self) {
        counter!("tile_memory_cache_misses_total").increment(1);
    }
    
    /// Update L1 cache statistics from TileMemoryCache
    pub fn record_tile_memory_cache_stats(&self, stats: &TileMemoryCacheStats) {
        let hits = stats.hits.load(Ordering::Relaxed);
        let misses = stats.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        
        gauge!("tile_memory_cache_hits_total").set(hits as f64);
        gauge!("tile_memory_cache_misses_total").set(misses as f64);
        gauge!("tile_memory_cache_hit_rate_percent").set(hit_rate);
        gauge!("tile_memory_cache_evictions_total").set(stats.evictions.load(Ordering::Relaxed) as f64);
        gauge!("tile_memory_cache_expired_total").set(stats.expired.load(Ordering::Relaxed) as f64);
        gauge!("tile_memory_cache_size_bytes").set(stats.size_bytes.load(Ordering::Relaxed) as f64);
    }
    
    /// Record container resource statistics
    pub fn record_container_stats(
        &self,
        memory_used_bytes: u64,
        memory_total_bytes: u64,
        memory_percent: f64,
        process_rss_bytes: u64,
        cpu_load_1m: f64,
        cpu_load_5m: f64,
        cpu_load_15m: f64,
        cpu_count: usize,
    ) {
        // Memory metrics
        gauge!("container_memory_used_bytes").set(memory_used_bytes as f64);
        gauge!("container_memory_total_bytes").set(memory_total_bytes as f64);
        gauge!("container_memory_percent").set(memory_percent);
        gauge!("process_rss_bytes").set(process_rss_bytes as f64);
        
        // CPU metrics
        gauge!("container_cpu_load_1m").set(cpu_load_1m);
        gauge!("container_cpu_load_5m").set(cpu_load_5m);
        gauge!("container_cpu_load_15m").set(cpu_load_15m);
        gauge!("container_cpu_count").set(cpu_count as f64);
        
        // CPU load percentage (load / cpu_count * 100)
        let cpu_load_percent = if cpu_count > 0 {
            (cpu_load_1m / cpu_count as f64) * 100.0
        } else {
            0.0
        };
        gauge!("container_cpu_load_percent").set(cpu_load_percent);
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
    
    /// Record a render operation with layer type classification
    pub async fn record_render_with_type(&self, duration_us: u64, success: bool, layer_type: LayerType) {
        // Record general render stats
        self.record_render(duration_us, success).await;
        
        // Record layer-type specific stats
        if success {
            let mut layer_times = self.layer_type_times.write().await;
            layer_times.entry(layer_type).or_insert_with(TimingStats::default).record(duration_us);
            
            // Record to metrics crate with label
            let type_label = match layer_type {
                LayerType::Gradient => "gradient",
                LayerType::WindBarbs => "wind_barbs",
                LayerType::Isolines => "isolines",
            };
            histogram!("render_duration_by_type_ms", "layer_type" => type_label)
                .record(duration_us as f64 / 1000.0);
            counter!("renders_by_type_total", "layer_type" => type_label).increment(1);
        }
    }
    
    /// Record GRIB file load time (from MinIO/cache)
    pub async fn record_grib_load(&self, duration_us: u64) {
        let mut times = self.grib_load_times.write().await;
        times.record(duration_us);
        histogram!("grib_load_duration_ms").record(duration_us as f64 / 1000.0);
    }
    
    /// Record GRIB parsing time (decompression + parsing)
    pub async fn record_grib_parse(&self, duration_us: u64) {
        let mut times = self.grib_parse_times.write().await;
        times.record(duration_us);
        histogram!("grib_parse_duration_ms").record(duration_us as f64 / 1000.0);
    }
    
    /// Record grid resampling time (projection + interpolation)
    pub async fn record_resample(&self, duration_us: u64) {
        let mut times = self.resample_times.write().await;
        times.record(duration_us);
        histogram!("resample_duration_ms").record(duration_us as f64 / 1000.0);
    }
    
    /// Record PNG encoding time
    pub async fn record_png_encode(&self, duration_us: u64) {
        let mut times = self.png_encode_times.write().await;
        times.record(duration_us);
        histogram!("png_encode_duration_ms").record(duration_us as f64 / 1000.0);
    }
    
    /// Record cache lookup time
    pub async fn record_cache_lookup(&self, duration_us: u64) {
        let mut times = self.cache_lookup_times.write().await;
        times.record(duration_us);
        histogram!("cache_lookup_duration_ms").record(duration_us as f64 / 1000.0);
    }
    
    /// Record GRIB cache statistics
    pub fn record_grib_cache_stats(&self, hits: u64, misses: u64, evictions: u64, size: usize, capacity: usize) {
        // Record GRIB cache hit rate
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        
        gauge!("grib_cache_hit_rate_percent").set(hit_rate);
        gauge!("grib_cache_hits_total").set(hits as f64);
        gauge!("grib_cache_misses_total").set(misses as f64);
        gauge!("grib_cache_evictions_total").set(evictions as f64);
        gauge!("grib_cache_size").set(size as f64);
        gauge!("grib_cache_capacity").set(capacity as f64);
        gauge!("grib_cache_utilization_percent").set((size as f64 / capacity as f64) * 100.0);
    }
    
    /// Get current metrics snapshot
    pub async fn snapshot(&self) -> MetricsSnapshot {
        let render_times = self.render_times.read().await;
        let minio_times = self.minio_times.read().await;
        let layer_times = self.layer_type_times.read().await;
        let grib_load_times = self.grib_load_times.read().await;
        let grib_parse_times = self.grib_parse_times.read().await;
        let resample_times = self.resample_times.read().await;
        let png_encode_times = self.png_encode_times.read().await;
        let cache_lookup_times = self.cache_lookup_times.read().await;
        
        let cache_hits = self.cache_hits.load(Ordering::Relaxed);
        let cache_misses = self.cache_misses.load(Ordering::Relaxed);
        let cache_total = cache_hits + cache_misses;
        let cache_hit_rate = if cache_total > 0 {
            (cache_hits as f64 / cache_total as f64) * 100.0
        } else {
            0.0
        };
        
        // Build per-layer-type stats
        let mut layer_type_stats = HashMap::new();
        for (layer_type, stats) in layer_times.iter() {
            layer_type_stats.insert(*layer_type, LayerTypeStats {
                count: stats.count,
                avg_ms: stats.avg_ms(),
                min_ms: stats.min_ms(),
                max_ms: stats.max_ms(),
                last_ms: stats.last_ms(),
            });
        }
        
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
            
            layer_type_stats,
            
            // Pipeline metrics
            grib_load_avg_ms: grib_load_times.avg_ms(),
            grib_load_last_ms: grib_load_times.last_ms(),
            grib_load_count: grib_load_times.count,
            
            grib_parse_avg_ms: grib_parse_times.avg_ms(),
            grib_parse_last_ms: grib_parse_times.last_ms(),
            grib_parse_count: grib_parse_times.count,
            
            resample_avg_ms: resample_times.avg_ms(),
            resample_last_ms: resample_times.last_ms(),
            resample_count: resample_times.count,
            
            png_encode_avg_ms: png_encode_times.avg_ms(),
            png_encode_last_ms: png_encode_times.last_ms(),
            png_encode_count: png_encode_times.count,
            
            cache_lookup_avg_ms: cache_lookup_times.avg_ms(),
            cache_lookup_last_ms: cache_lookup_times.last_ms(),
            cache_lookup_count: cache_lookup_times.count,
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
        self.layer_type_times.write().await.clear();
        *self.grib_load_times.write().await = TimingStats::default();
        *self.grib_parse_times.write().await = TimingStats::default();
        *self.resample_times.write().await = TimingStats::default();
        *self.png_encode_times.write().await = TimingStats::default();
        *self.cache_lookup_times.write().await = TimingStats::default();
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
    
    // Per-layer-type stats
    pub layer_type_stats: HashMap<LayerType, LayerTypeStats>,
    
    // Pipeline timing breakdown
    pub grib_load_avg_ms: f64,
    pub grib_load_last_ms: f64,
    pub grib_load_count: u64,
    
    pub grib_parse_avg_ms: f64,
    pub grib_parse_last_ms: f64,
    pub grib_parse_count: u64,
    
    pub resample_avg_ms: f64,
    pub resample_last_ms: f64,
    pub resample_count: u64,
    
    pub png_encode_avg_ms: f64,
    pub png_encode_last_ms: f64,
    pub png_encode_count: u64,
    
    pub cache_lookup_avg_ms: f64,
    pub cache_lookup_last_ms: f64,
    pub cache_lookup_count: u64,
}

/// Per-layer-type performance statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerTypeStats {
    pub count: u64,
    pub avg_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub last_ms: f64,
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
