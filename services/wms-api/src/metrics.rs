//! Application metrics collection and reporting.

use metrics::{counter, gauge, histogram};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
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
    
    /// Per-data-source parsing stats (for GRIB2 vs NetCDF comparison)
    data_source_parse_times: RwLock<HashMap<String, DataSourceParseStats>>,
    
    /// Detailed pipeline timing stats
    grib_load_times: RwLock<TimingStats>,
    grib_parse_times: RwLock<TimingStats>,
    resample_times: RwLock<TimingStats>,
    png_encode_times: RwLock<TimingStats>,
    cache_lookup_times: RwLock<TimingStats>,
    
    /// Request rate tracking for 1min/5min windows
    wms_rate_tracker: RwLock<RateTracker>,
    wmts_rate_tracker: RwLock<RateTracker>,
    render_rate_tracker: RwLock<RateTracker>,
    
    /// Tile request heatmap for geographic visualization
    tile_heatmap: RwLock<TileRequestHeatmap>,
    
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

/// Data source type for per-layer parsing metrics
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataSourceType {
    Grib2Gfs,
    Grib2Hrrr,
    Grib2Mrms,
    NetcdfGoes,
    Other(String),
}

/// GOES satellite identifier for per-satellite metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GoesSatellite {
    Goes16,
    Goes18,
}

impl GoesSatellite {
    pub fn from_model(model: &str) -> Option<Self> {
        match model.to_lowercase().as_str() {
            "goes16" => Some(GoesSatellite::Goes16),
            "goes18" => Some(GoesSatellite::Goes18),
            _ => None,
        }
    }
    
    pub fn label(&self) -> &'static str {
        match self {
            GoesSatellite::Goes16 => "goes16",
            GoesSatellite::Goes18 => "goes18",
        }
    }
}

/// Weather model identifier for per-model metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WeatherModel {
    Gfs,
    Hrrr,
    Mrms,
}

impl WeatherModel {
    pub fn from_model(model: &str) -> Option<Self> {
        match model.to_lowercase().as_str() {
            "gfs" => Some(WeatherModel::Gfs),
            "hrrr" => Some(WeatherModel::Hrrr),
            "mrms" => Some(WeatherModel::Mrms),
            _ => None,
        }
    }
    
    pub fn label(&self) -> &'static str {
        match self {
            WeatherModel::Gfs => "gfs",
            WeatherModel::Hrrr => "hrrr",
            WeatherModel::Mrms => "mrms",
        }
    }
    
    pub fn display_name(&self) -> &'static str {
        match self {
            WeatherModel::Gfs => "GFS",
            WeatherModel::Hrrr => "HRRR",
            WeatherModel::Mrms => "MRMS",
        }
    }
}

impl DataSourceType {
    /// Classify data source type based on model name
    pub fn from_model(model: &str) -> Self {
        match model.to_lowercase().as_str() {
            "gfs" => DataSourceType::Grib2Gfs,
            "hrrr" => DataSourceType::Grib2Hrrr,
            "mrms" => DataSourceType::Grib2Mrms,
            "goes16" | "goes18" | "goes" => DataSourceType::NetcdfGoes,
            other => DataSourceType::Other(other.to_string()),
        }
    }
    
    /// Get a string label for metrics
    pub fn label(&self) -> &str {
        match self {
            DataSourceType::Grib2Gfs => "gfs",
            DataSourceType::Grib2Hrrr => "hrrr",
            DataSourceType::Grib2Mrms => "mrms",
            DataSourceType::NetcdfGoes => "goes",
            DataSourceType::Other(name) => name.as_str(),
        }
    }
    
    /// Check if this is a GRIB2 source
    pub fn is_grib2(&self) -> bool {
        matches!(self, DataSourceType::Grib2Gfs | DataSourceType::Grib2Hrrr | DataSourceType::Grib2Mrms)
    }
    
    /// Check if this is a NetCDF source
    pub fn is_netcdf(&self) -> bool {
        matches!(self, DataSourceType::NetcdfGoes)
    }
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

/// Per-data-source parsing statistics
#[derive(Debug, Default, Clone)]
pub struct DataSourceParseStats {
    /// Total parse operations
    pub parse_count: u64,
    /// Cache hits (parsed data reused)
    pub cache_hits: u64,
    /// Cache misses (had to parse)
    pub cache_misses: u64,
    /// Total parse time in microseconds
    pub total_parse_us: u64,
    /// Minimum parse time
    pub min_parse_us: u64,
    /// Maximum parse time
    pub max_parse_us: u64,
    /// Last parse time
    pub last_parse_us: u64,
}

impl DataSourceParseStats {
    pub fn record_parse(&mut self, duration_us: u64) {
        self.parse_count += 1;
        self.cache_misses += 1;
        self.total_parse_us += duration_us;
        self.last_parse_us = duration_us;
        if self.min_parse_us == 0 || duration_us < self.min_parse_us {
            self.min_parse_us = duration_us;
        }
        if duration_us > self.max_parse_us {
            self.max_parse_us = duration_us;
        }
    }
    
    pub fn record_cache_hit(&mut self) {
        self.cache_hits += 1;
    }
    
    pub fn avg_parse_ms(&self) -> f64 {
        if self.cache_misses == 0 {
            0.0
        } else {
            (self.total_parse_us as f64 / self.cache_misses as f64) / 1000.0
        }
    }
    
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            (self.cache_hits as f64 / total as f64) * 100.0
        }
    }
}

/// Tracks request timestamps for calculating rates over time windows
#[derive(Debug)]
struct RateTracker {
    /// Timestamps of requests (as seconds since start)
    timestamps: Vec<u64>,
    /// Reference start time
    start: Instant,
}

/// Tracks tile request locations for heatmap visualization
/// Cache status for a tile request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileCacheStatus {
    /// L1 in-memory cache hit
    L1Hit,
    /// L2 Redis cache hit  
    L2Hit,
    /// Cache miss - had to render
    Miss,
}

#[derive(Debug)]
pub struct TileRequestHeatmap {
    /// Map of "min_lon,min_lat,max_lon,max_lat" -> tile cell
    /// Stores actual tile bounds for accurate visualization
    cells: HashMap<String, TileHeatmapCell>,
    /// Maximum number of cells to track (prevents unbounded growth)
    max_cells: usize,
    /// Timestamp of last clear
    last_clear: Instant,
}

/// A single cell in the tile heatmap - stores full tile bounds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileHeatmapCell {
    /// Bounding box [min_lon, min_lat, max_lon, max_lat]
    pub min_lon: f32,
    pub min_lat: f32,
    pub max_lon: f32,
    pub max_lat: f32,
    /// Total request count
    pub count: u64,
    /// L1 cache hits
    pub l1_hits: u64,
    /// L2 cache hits
    pub l2_hits: u64,
    /// Cache misses (full renders)
    pub misses: u64,
}

/// Snapshot of heatmap data for JSON serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileHeatmapSnapshot {
    pub cells: Vec<TileHeatmapCell>,
    pub total_requests: u64,
}

impl TileRequestHeatmap {
    pub fn new() -> Self {
        Self {
            cells: HashMap::new(),
            max_cells: 10000, // Limit to prevent memory bloat
            last_clear: Instant::now(),
        }
    }
    
    /// Record a tile request with its bounding box and cache status
    pub fn record(&mut self, bbox: &[f32; 4], cache_status: TileCacheStatus) {
        // bbox format: [min_lon, min_lat, max_lon, max_lat]
        // Round to 2 decimal places to aggregate nearby tiles while preserving tile shapes
        let min_lon = (bbox[0] * 100.0).round() / 100.0;
        let min_lat = (bbox[1] * 100.0).round() / 100.0;
        let max_lon = (bbox[2] * 100.0).round() / 100.0;
        let max_lat = (bbox[3] * 100.0).round() / 100.0;
        
        // Key uniquely identifies this tile's bounds
        let key = format!("{:.2},{:.2},{:.2},{:.2}", min_lon, min_lat, max_lon, max_lat);
        
        // Update or insert cell
        if let Some(cell) = self.cells.get_mut(&key) {
            cell.count += 1;
            match cache_status {
                TileCacheStatus::L1Hit => cell.l1_hits += 1,
                TileCacheStatus::L2Hit => cell.l2_hits += 1,
                TileCacheStatus::Miss => cell.misses += 1,
            }
        } else if self.cells.len() < self.max_cells {
            let (l1_hits, l2_hits, misses) = match cache_status {
                TileCacheStatus::L1Hit => (1, 0, 0),
                TileCacheStatus::L2Hit => (0, 1, 0),
                TileCacheStatus::Miss => (0, 0, 1),
            };
            self.cells.insert(key, TileHeatmapCell {
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                count: 1,
                l1_hits,
                l2_hits,
                misses,
            });
        }
        // If at max capacity, just ignore new cells (existing cells still get incremented)
    }
    
    /// Get a snapshot of the current heatmap state
    pub fn snapshot(&self) -> TileHeatmapSnapshot {
        let cells: Vec<TileHeatmapCell> = self.cells.values().cloned().collect();
        let total_requests: u64 = cells.iter().map(|c| c.count).sum();
        TileHeatmapSnapshot {
            cells,
            total_requests,
        }
    }
    
    /// Clear all heatmap data
    pub fn clear(&mut self) {
        self.cells.clear();
        self.last_clear = Instant::now();
    }
}

impl Default for TileRequestHeatmap {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for RateTracker {
    fn default() -> Self {
        Self {
            timestamps: Vec::with_capacity(10000),
            start: Instant::now(),
        }
    }
}

impl RateTracker {
    fn new(start: Instant) -> Self {
        Self {
            timestamps: Vec::with_capacity(10000),
            start,
        }
    }
    
    fn record(&mut self) {
        let now = self.start.elapsed().as_secs();
        self.timestamps.push(now);
        
        // Prune old entries (older than 5 minutes) periodically
        if self.timestamps.len() > 5000 {
            let cutoff = now.saturating_sub(300); // 5 minutes
            self.timestamps.retain(|&t| t >= cutoff);
        }
    }
    
    fn rate_1m(&self) -> f64 {
        let now = self.start.elapsed().as_secs();
        let cutoff = now.saturating_sub(60);
        let count = self.timestamps.iter().filter(|&&t| t >= cutoff).count();
        count as f64 / 60.0 // requests per second
    }
    
    fn rate_5m(&self) -> f64 {
        let now = self.start.elapsed().as_secs();
        let cutoff = now.saturating_sub(300);
        let count = self.timestamps.iter().filter(|&&t| t >= cutoff).count();
        count as f64 / 300.0 // requests per second
    }
    
    fn count_1m(&self) -> u64 {
        let now = self.start.elapsed().as_secs();
        let cutoff = now.saturating_sub(60);
        self.timestamps.iter().filter(|&&t| t >= cutoff).count() as u64
    }
    
    fn count_5m(&self) -> u64 {
        let now = self.start.elapsed().as_secs();
        let cutoff = now.saturating_sub(300);
        self.timestamps.iter().filter(|&&t| t >= cutoff).count() as u64
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
        let start_time = Instant::now();
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
            data_source_parse_times: RwLock::new(HashMap::new()),
            grib_load_times: RwLock::new(TimingStats::default()),
            grib_parse_times: RwLock::new(TimingStats::default()),
            resample_times: RwLock::new(TimingStats::default()),
            png_encode_times: RwLock::new(TimingStats::default()),
            cache_lookup_times: RwLock::new(TimingStats::default()),
            wms_rate_tracker: RwLock::new(RateTracker::new(start_time)),
            wmts_rate_tracker: RwLock::new(RateTracker::new(start_time)),
            render_rate_tracker: RwLock::new(RateTracker::new(start_time)),
            tile_heatmap: RwLock::new(TileRequestHeatmap::new()),
            start_time,
        }
    }
    
    /// Record a WMS request
    pub fn record_wms_request(&self) {
        self.wms_requests.fetch_add(1, Ordering::Relaxed);
        counter!("wms_requests_total").increment(1);
        // Track for rate calculation (non-blocking)
        if let Ok(mut tracker) = self.wms_rate_tracker.try_write() {
            tracker.record();
        }
    }
    
    /// Record a WMTS request
    pub fn record_wmts_request(&self) {
        self.wmts_requests.fetch_add(1, Ordering::Relaxed);
        counter!("wmts_requests_total").increment(1);
        // Track for rate calculation (non-blocking)
        if let Ok(mut tracker) = self.wmts_rate_tracker.try_write() {
            tracker.record();
        }
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
    
    /// Record a tile request location for heatmap visualization
    /// bbox format: [min_lon, min_lat, max_lon, max_lat]
    pub fn record_tile_request_location(&self, bbox: &[f32; 4], cache_status: TileCacheStatus) {
        if let Ok(mut heatmap) = self.tile_heatmap.try_write() {
            heatmap.record(bbox, cache_status);
        }
    }
    
    /// Get a snapshot of the tile request heatmap
    pub async fn get_tile_heatmap(&self) -> TileHeatmapSnapshot {
        self.tile_heatmap.read().await.snapshot()
    }
    
    /// Clear the tile request heatmap
    pub async fn clear_tile_heatmap(&self) {
        self.tile_heatmap.write().await.clear();
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
        
        // Track for rate calculation (non-blocking)
        if let Ok(mut tracker) = self.render_rate_tracker.try_write() {
            tracker.record();
        }
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
    
    /// Record per-data-source parsing time (for admin dashboard)
    pub async fn record_data_source_parse(&self, source_type: &DataSourceType, duration_us: u64) {
        let label = source_type.label().to_string();
        let mut stats = self.data_source_parse_times.write().await;
        let entry = stats.entry(label.clone()).or_insert_with(DataSourceParseStats::default);
        entry.record_parse(duration_us);
        
        // Also record to prometheus with label
        histogram!("data_source_parse_duration_ms", "source" => label)
            .record(duration_us as f64 / 1000.0);
        counter!("data_source_parse_total", "source" => source_type.label().to_string()).increment(1);
    }
    
    /// Record a grid cache hit for a data source
    pub async fn record_data_source_cache_hit(&self, source_type: &DataSourceType) {
        let label = source_type.label().to_string();
        let mut stats = self.data_source_parse_times.write().await;
        let entry = stats.entry(label.clone()).or_insert_with(DataSourceParseStats::default);
        entry.record_cache_hit();
        
        counter!("data_source_cache_hits_total", "source" => label).increment(1);
    }
    
    // ==================== GOES-Specific Metrics ====================
    
    /// Record a GOES tile request
    pub fn record_goes_request(&self, satellite: GoesSatellite, band: &str) {
        counter!("goes_requests_total", 
            "satellite" => satellite.label().to_string(),
            "band" => band.to_string()
        ).increment(1);
    }
    
    /// Record GOES file fetch from MinIO
    pub fn record_goes_fetch(&self, satellite: GoesSatellite, file_size_bytes: u64, duration_us: u64) {
        let sat_label = satellite.label().to_string();
        counter!("goes_fetch_total", "satellite" => sat_label.clone()).increment(1);
        counter!("goes_fetch_bytes_total", "satellite" => sat_label.clone()).increment(file_size_bytes);
        histogram!("goes_fetch_duration_ms", "satellite" => sat_label.clone())
            .record(duration_us as f64 / 1000.0);
        histogram!("goes_file_size_bytes", "satellite" => sat_label)
            .record(file_size_bytes as f64);
    }
    
    /// Record GOES NetCDF parsing time
    pub fn record_goes_parse(&self, satellite: GoesSatellite, duration_us: u64, width: u32, height: u32) {
        let sat_label = satellite.label().to_string();
        histogram!("goes_parse_duration_ms", "satellite" => sat_label.clone())
            .record(duration_us as f64 / 1000.0);
        gauge!("goes_grid_width", "satellite" => sat_label.clone()).set(width as f64);
        gauge!("goes_grid_height", "satellite" => sat_label.clone()).set(height as f64);
        let pixels = (width as u64) * (height as u64);
        gauge!("goes_grid_pixels", "satellite" => sat_label).set(pixels as f64);
    }
    
    /// Record GOES projection/resampling time
    pub fn record_goes_projection(&self, satellite: GoesSatellite, duration_us: u64, use_lut: bool) {
        let sat_label = satellite.label().to_string();
        let method = if use_lut { "lut" } else { "compute" };
        histogram!("goes_projection_duration_ms", 
            "satellite" => sat_label,
            "method" => method.to_string()
        ).record(duration_us as f64 / 1000.0);
    }
    
    /// Record GOES render completion
    pub fn record_goes_render(&self, satellite: GoesSatellite, band: &str, duration_us: u64, success: bool) {
        let sat_label = satellite.label().to_string();
        let band_label = band.to_string();
        counter!("goes_renders_total", 
            "satellite" => sat_label.clone(),
            "band" => band_label.clone(),
            "success" => success.to_string()
        ).increment(1);
        if success {
            histogram!("goes_render_duration_ms", 
                "satellite" => sat_label,
                "band" => band_label
            ).record(duration_us as f64 / 1000.0);
        }
    }
    
    /// Record GOES cache hit (grid data cache)
    pub fn record_goes_cache_hit(&self, satellite: GoesSatellite) {
        counter!("goes_cache_hits_total", "satellite" => satellite.label().to_string()).increment(1);
    }
    
    /// Record GOES cache miss (grid data cache)
    pub fn record_goes_cache_miss(&self, satellite: GoesSatellite) {
        counter!("goes_cache_misses_total", "satellite" => satellite.label().to_string()).increment(1);
    }
    
    /// Record GOES LUT (look-up table) usage for projection
    pub fn record_goes_lut_status(&self, satellite: GoesSatellite, loaded: bool, generation_ms: Option<f64>) {
        let sat_label = satellite.label().to_string();
        gauge!("goes_lut_loaded", "satellite" => sat_label.clone()).set(if loaded { 1.0 } else { 0.0 });
        if let Some(gen_ms) = generation_ms {
            gauge!("goes_lut_generation_ms", "satellite" => sat_label).set(gen_ms);
        }
    }
    
    /// Record GOES ingestion event
    pub fn record_goes_ingestion(&self, satellite: GoesSatellite, band: &str, file_size_bytes: u64) {
        let sat_label = satellite.label().to_string();
        counter!("goes_ingestion_total", 
            "satellite" => sat_label.clone(),
            "band" => band.to_string()
        ).increment(1);
        counter!("goes_ingestion_bytes_total", "satellite" => sat_label).increment(file_size_bytes);
    }
    
    // ==================== Weather Model Metrics (GFS/HRRR/MRMS) ====================
    
    /// Record a weather model tile request
    pub fn record_model_request(&self, model: WeatherModel, parameter: &str) {
        counter!("model_requests_total", 
            "model" => model.label().to_string(),
            "parameter" => parameter.to_string()
        ).increment(1);
    }
    
    /// Record weather model file fetch from MinIO
    pub fn record_model_fetch(&self, model: WeatherModel, file_size_bytes: u64, duration_us: u64) {
        let model_label = model.label().to_string();
        counter!("model_fetch_total", "model" => model_label.clone()).increment(1);
        counter!("model_fetch_bytes_total", "model" => model_label.clone()).increment(file_size_bytes);
        histogram!("model_fetch_duration_ms", "model" => model_label.clone())
            .record(duration_us as f64 / 1000.0);
        histogram!("model_file_size_bytes", "model" => model_label)
            .record(file_size_bytes as f64);
    }
    
    /// Record weather model GRIB2 parsing time
    pub fn record_model_parse(&self, model: WeatherModel, duration_us: u64, grid_points: u64) {
        let model_label = model.label().to_string();
        histogram!("model_parse_duration_ms", "model" => model_label.clone())
            .record(duration_us as f64 / 1000.0);
        gauge!("model_grid_points", "model" => model_label).set(grid_points as f64);
    }
    
    /// Record weather model render completion
    pub fn record_model_render(&self, model: WeatherModel, parameter: &str, duration_us: u64, success: bool) {
        let model_label = model.label().to_string();
        let param_label = parameter.to_string();
        counter!("model_renders_total", 
            "model" => model_label.clone(),
            "parameter" => param_label.clone(),
            "success" => success.to_string()
        ).increment(1);
        if success {
            histogram!("model_render_duration_ms", 
                "model" => model_label,
                "parameter" => param_label
            ).record(duration_us as f64 / 1000.0);
        }
    }
    
    /// Record weather model cache hit (GRIB cache)
    pub fn record_model_cache_hit(&self, model: WeatherModel) {
        counter!("model_cache_hits_total", "model" => model.label().to_string()).increment(1);
    }
    
    /// Record weather model cache miss (GRIB cache)
    pub fn record_model_cache_miss(&self, model: WeatherModel) {
        counter!("model_cache_misses_total", "model" => model.label().to_string()).increment(1);
    }
    
    /// Record weather model grid cache hit (parsed grid data)
    pub fn record_model_grid_cache_hit(&self, model: WeatherModel) {
        counter!("model_grid_cache_hits_total", "model" => model.label().to_string()).increment(1);
    }
    
    /// Record weather model grid cache miss (parsed grid data)
    pub fn record_model_grid_cache_miss(&self, model: WeatherModel) {
        counter!("model_grid_cache_misses_total", "model" => model.label().to_string()).increment(1);
    }
    
    /// Record weather model resampling/projection time
    pub fn record_model_resample(&self, model: WeatherModel, duration_us: u64) {
        histogram!("model_resample_duration_ms", "model" => model.label().to_string())
            .record(duration_us as f64 / 1000.0);
    }
    
    /// Record weather model PNG encoding time
    pub fn record_model_png_encode(&self, model: WeatherModel, duration_us: u64) {
        histogram!("model_png_encode_duration_ms", "model" => model.label().to_string())
            .record(duration_us as f64 / 1000.0);
    }
    
    /// Record weather model ingestion event
    pub fn record_model_ingestion(&self, model: WeatherModel, parameter: &str, file_size_bytes: u64, forecast_hour: u32) {
        let model_label = model.label().to_string();
        counter!("model_ingestion_total", 
            "model" => model_label.clone(),
            "parameter" => parameter.to_string()
        ).increment(1);
        counter!("model_ingestion_bytes_total", "model" => model_label.clone()).increment(file_size_bytes);
        gauge!("model_latest_forecast_hour", "model" => model_label).set(forecast_hour as f64);
    }
    
    /// Record forecast hour being rendered
    pub fn record_model_forecast_hour(&self, model: WeatherModel, forecast_hour: u32) {
        histogram!("model_forecast_hour_rendered", "model" => model.label().to_string())
            .record(forecast_hour as f64);
    }
    
    /// Record model data age (time since reference time)
    pub fn record_model_data_age(&self, model: WeatherModel, age_minutes: u64) {
        gauge!("model_data_age_minutes", "model" => model.label().to_string()).set(age_minutes as f64);
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
    
    /// Record grid data cache statistics (for parsed GOES/GRIB grids)
    pub fn record_grid_cache_stats(&self, stats: &storage::GridCacheStats) {
        let hit_rate = stats.hit_rate();
        let utilization = stats.utilization();
        let memory_mb = stats.memory_mb();
        
        gauge!("grid_cache_hits_total").set(stats.hits as f64);
        gauge!("grid_cache_misses_total").set(stats.misses as f64);
        gauge!("grid_cache_evictions_total").set(stats.evictions as f64);
        gauge!("grid_cache_entries").set(stats.entries as f64);
        gauge!("grid_cache_capacity").set(stats.capacity as f64);
        gauge!("grid_cache_memory_bytes").set(stats.memory_bytes as f64);
        gauge!("grid_cache_memory_mb").set(memory_mb);
        gauge!("grid_cache_hit_rate_percent").set(hit_rate);
        gauge!("grid_cache_utilization_percent").set(utilization);
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
        
        // Get rate tracking data
        let wms_rate = self.wms_rate_tracker.read().await;
        let wmts_rate = self.wmts_rate_tracker.read().await;
        let render_rate = self.render_rate_tracker.read().await;
        
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
        
        // Build per-data-source stats
        let data_source_times = self.data_source_parse_times.read().await;
        let mut data_source_stats = HashMap::new();
        for (source_name, stats) in data_source_times.iter() {
            data_source_stats.insert(source_name.clone(), DataSourceSnapshotStats {
                source_type: source_name.clone(),
                parse_count: stats.parse_count,
                cache_hits: stats.cache_hits,
                cache_misses: stats.cache_misses,
                cache_hit_rate: stats.cache_hit_rate(),
                avg_parse_ms: stats.avg_parse_ms(),
                min_parse_ms: stats.min_parse_us as f64 / 1000.0,
                max_parse_ms: stats.max_parse_us as f64 / 1000.0,
                last_parse_ms: stats.last_parse_us as f64 / 1000.0,
            });
        }
        
        MetricsSnapshot {
            uptime_secs: self.start_time.elapsed().as_secs(),
            
            wms_requests: self.wms_requests.load(Ordering::Relaxed),
            wmts_requests: self.wmts_requests.load(Ordering::Relaxed),
            
            // Request rates
            wms_rate_1m: wms_rate.rate_1m(),
            wms_rate_5m: wms_rate.rate_5m(),
            wms_count_1m: wms_rate.count_1m(),
            wms_count_5m: wms_rate.count_5m(),
            wmts_rate_1m: wmts_rate.rate_1m(),
            wmts_rate_5m: wmts_rate.rate_5m(),
            wmts_count_1m: wmts_rate.count_1m(),
            wmts_count_5m: wmts_rate.count_5m(),
            
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
            render_rate_1m: render_rate.rate_1m(),
            render_rate_5m: render_rate.rate_5m(),
            render_count_1m: render_rate.count_1m(),
            render_count_5m: render_rate.count_5m(),
            
            layer_type_stats,
            data_source_stats,
            
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
        self.data_source_parse_times.write().await.clear();
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
    
    // Request rates (requests per second)
    pub wms_rate_1m: f64,
    pub wms_rate_5m: f64,
    pub wms_count_1m: u64,
    pub wms_count_5m: u64,
    pub wmts_rate_1m: f64,
    pub wmts_rate_5m: f64,
    pub wmts_count_1m: u64,
    pub wmts_count_5m: u64,
    
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
    pub render_rate_1m: f64,
    pub render_rate_5m: f64,
    pub render_count_1m: u64,
    pub render_count_5m: u64,
    
    // Per-layer-type stats
    pub layer_type_stats: HashMap<LayerType, LayerTypeStats>,
    
    // Per-data-source parsing stats
    pub data_source_stats: HashMap<String, DataSourceSnapshotStats>,
    
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

/// Per-data-source parsing statistics for JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceSnapshotStats {
    pub source_type: String,
    pub parse_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_hit_rate: f64,
    pub avg_parse_ms: f64,
    pub min_parse_ms: f64,
    pub max_parse_ms: f64,
    pub last_parse_ms: f64,
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
