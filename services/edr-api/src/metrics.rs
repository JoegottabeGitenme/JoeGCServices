//! Metrics collection and reporting for the EDR API.

use metrics::{counter, histogram};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::RwLock;

/// EDR query endpoint types for metrics labeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointType {
    Position,
    Area,
    Radius,
    Cube,
    Corridor,
    Trajectory,
    Locations,
    Collections,
    Instances,
}

impl EndpointType {
    pub fn label(&self) -> &'static str {
        match self {
            EndpointType::Position => "position",
            EndpointType::Area => "area",
            EndpointType::Radius => "radius",
            EndpointType::Cube => "cube",
            EndpointType::Corridor => "corridor",
            EndpointType::Trajectory => "trajectory",
            EndpointType::Locations => "locations",
            EndpointType::Collections => "collections",
            EndpointType::Instances => "instances",
        }
    }
}

/// Output format for response metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatType {
    CoverageJson,
    GeoJson,
    Png,
}

impl FormatType {
    pub fn label(&self) -> &'static str {
        match self {
            FormatType::CoverageJson => "coverage_json",
            FormatType::GeoJson => "geojson",
            FormatType::Png => "png",
        }
    }
}

/// Tracks request timestamps for calculating rates over time windows.
#[derive(Debug)]
struct RateTracker {
    timestamps: Vec<u64>,
    start: Instant,
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
            let cutoff = now.saturating_sub(300);
            self.timestamps.retain(|&t| t >= cutoff);
        }
    }

    fn rate_1m(&self) -> f64 {
        let now = self.start.elapsed().as_secs();
        let cutoff = now.saturating_sub(60);
        let count = self.timestamps.iter().filter(|&&t| t >= cutoff).count();
        count as f64 / 60.0
    }

    fn rate_5m(&self) -> f64 {
        let now = self.start.elapsed().as_secs();
        let cutoff = now.saturating_sub(300);
        let count = self.timestamps.iter().filter(|&&t| t >= cutoff).count();
        count as f64 / 300.0
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

impl Default for RateTracker {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

/// Timing statistics for a category of operations.
#[derive(Debug, Default, Clone)]
struct TimingStats {
    count: u64,
    total_us: u64,
    min_us: u64,
    max_us: u64,
    last_us: u64,
    /// Store recent values for percentile calculations (circular buffer)
    recent_values: Vec<u64>,
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
        // Keep last 1000 values for percentile calculations
        if self.recent_values.len() >= 1000 {
            self.recent_values.remove(0);
        }
        self.recent_values.push(duration_us);
    }

    fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            (self.total_us as f64 / self.count as f64) / 1000.0
        }
    }

    #[allow(dead_code)] // May be useful for future metrics display
    fn last_ms(&self) -> f64 {
        self.last_us as f64 / 1000.0
    }

    #[allow(dead_code)] // May be useful for future metrics display
    fn min_ms(&self) -> f64 {
        self.min_us as f64 / 1000.0
    }

    fn max_ms(&self) -> f64 {
        self.max_us as f64 / 1000.0
    }

    /// Calculate percentile (0-100) from recent values.
    fn percentile_ms(&self, p: f64) -> f64 {
        if self.recent_values.is_empty() {
            return 0.0;
        }
        let mut sorted = self.recent_values.clone();
        sorted.sort_unstable();
        let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx.min(sorted.len() - 1)] as f64 / 1000.0
    }
}

/// Geographic query extent for heatmap visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExtent {
    /// Bounding box [min_lon, min_lat, max_lon, max_lat]
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
    /// Center point for Grafana geomap compatibility
    pub latitude: f64,
    pub longitude: f64,
    /// Query type that generated this extent
    pub query_type: String,
    /// Collection queried
    pub collection: String,
    /// Request count for this extent (aggregated)
    pub count: u64,
}

/// Tracks query extents for geographic heatmap visualization.
#[derive(Debug)]
pub struct QueryHeatmap {
    /// Map of "min_lon,min_lat,max_lon,max_lat" -> query extent cell
    cells: HashMap<String, QueryExtent>,
    /// Maximum number of cells to track
    max_cells: usize,
    /// Timestamp of last clear
    last_clear: Instant,
}

impl QueryHeatmap {
    pub fn new() -> Self {
        Self {
            cells: HashMap::new(),
            max_cells: 10000,
            last_clear: Instant::now(),
        }
    }

    /// Record a query extent from a position query (single point).
    pub fn record_point(&mut self, lon: f64, lat: f64, collection: &str) {
        // For points, create a small box around the point (0.01 degree = ~1km)
        let delta = 0.01;
        self.record_bbox(
            lon - delta,
            lat - delta,
            lon + delta,
            lat + delta,
            "position",
            collection,
        );
    }

    /// Record a query extent from a radius query.
    pub fn record_radius(&mut self, lon: f64, lat: f64, radius_m: f64, collection: &str) {
        // Convert radius to degrees (approximate, 1 degree ≈ 111km at equator)
        let radius_deg = radius_m / 111_000.0;
        self.record_bbox(
            lon - radius_deg,
            lat - radius_deg,
            lon + radius_deg,
            lat + radius_deg,
            "radius",
            collection,
        );
    }

    /// Record a query extent from an area/cube query.
    pub fn record_bbox(
        &mut self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        query_type: &str,
        collection: &str,
    ) {
        // Round to 1 decimal place for aggregation (~11km cells)
        let min_lon = (min_lon * 10.0).round() / 10.0;
        let min_lat = (min_lat * 10.0).round() / 10.0;
        let max_lon = (max_lon * 10.0).round() / 10.0;
        let max_lat = (max_lat * 10.0).round() / 10.0;

        let key = format!(
            "{:.1},{:.1},{:.1},{:.1}",
            min_lon, min_lat, max_lon, max_lat
        );

        if let Some(cell) = self.cells.get_mut(&key) {
            cell.count += 1;
        } else if self.cells.len() < self.max_cells {
            // Calculate center point for Grafana geomap
            let latitude = (min_lat + max_lat) / 2.0;
            let longitude = (min_lon + max_lon) / 2.0;

            self.cells.insert(
                key,
                QueryExtent {
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    latitude,
                    longitude,
                    query_type: query_type.to_string(),
                    collection: collection.to_string(),
                    count: 1,
                },
            );
        }
    }

    /// Get a snapshot of all recorded extents.
    pub fn snapshot(&self) -> Vec<QueryExtent> {
        self.cells.values().cloned().collect()
    }

    /// Clear all heatmap data.
    pub fn clear(&mut self) {
        self.cells.clear();
        self.last_clear = Instant::now();
    }
}

impl Default for QueryHeatmap {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-endpoint statistics.
#[derive(Debug)]
struct EndpointStats {
    requests: AtomicU64,
    errors: AtomicU64,
}

impl Default for EndpointStats {
    fn default() -> Self {
        Self {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

/// Per-collection statistics.
#[derive(Debug, Default)]
struct CollectionStats {
    requests: AtomicU64,
}

/// Metrics collector for the EDR API.
#[derive(Debug)]
pub struct MetricsCollector {
    /// Per-endpoint request stats
    endpoint_stats: RwLock<HashMap<EndpointType, EndpointStats>>,

    /// Per-endpoint timing stats (separate to avoid nested locks)
    endpoint_timing: RwLock<HashMap<EndpointType, TimingStats>>,

    /// Per-collection request stats
    collection_stats: RwLock<HashMap<String, CollectionStats>>,

    /// Per-parameter request stats
    parameter_stats: RwLock<HashMap<String, AtomicU64>>,

    /// Per-format response stats
    format_stats: RwLock<HashMap<FormatType, AtomicU64>>,

    /// Total request counter
    total_requests: AtomicU64,

    /// Total error counter
    total_errors: AtomicU64,

    /// Cache statistics
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,

    /// Request rate tracker
    rate_tracker: RwLock<RateTracker>,

    /// Overall timing stats
    overall_timing: RwLock<TimingStats>,

    /// Geographic query heatmap
    query_heatmap: RwLock<QueryHeatmap>,

    /// Client tracking (IP -> request count)
    client_requests: RwLock<HashMap<String, u64>>,

    /// User-Agent tracking
    user_agent_requests: RwLock<HashMap<String, u64>>,

    /// Start time for uptime calculation
    start_time: Instant,
}

impl MetricsCollector {
    pub fn new() -> Self {
        let start_time = Instant::now();
        Self {
            endpoint_stats: RwLock::new(HashMap::new()),
            endpoint_timing: RwLock::new(HashMap::new()),
            collection_stats: RwLock::new(HashMap::new()),
            parameter_stats: RwLock::new(HashMap::new()),
            format_stats: RwLock::new(HashMap::new()),
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            rate_tracker: RwLock::new(RateTracker::new(start_time)),
            overall_timing: RwLock::new(TimingStats::default()),
            query_heatmap: RwLock::new(QueryHeatmap::new()),
            client_requests: RwLock::new(HashMap::new()),
            user_agent_requests: RwLock::new(HashMap::new()),
            start_time,
        }
    }

    /// Record an EDR request with all relevant metadata.
    pub async fn record_request(
        &self,
        endpoint: EndpointType,
        collection: Option<&str>,
        parameters: &[String],
        format: FormatType,
        duration_us: u64,
        success: bool,
        client_ip: Option<&str>,
        user_agent: Option<&str>,
    ) {
        // Update total counters
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.total_errors.fetch_add(1, Ordering::Relaxed);
        }

        // Update Prometheus counters
        counter!("edr_requests_total", "endpoint" => endpoint.label().to_string()).increment(1);
        if !success {
            counter!("edr_errors_total", "endpoint" => endpoint.label().to_string()).increment(1);
        }

        // Record duration
        histogram!("edr_request_duration_seconds", "endpoint" => endpoint.label().to_string())
            .record(duration_us as f64 / 1_000_000.0);

        // Update rate tracker
        if let Ok(mut tracker) = self.rate_tracker.try_write() {
            tracker.record();
        }

        // Update overall timing
        if let Ok(mut timing) = self.overall_timing.try_write() {
            timing.record(duration_us);
        }

        // Update endpoint-specific stats
        {
            let mut stats = self.endpoint_stats.write().await;
            let endpoint_stat = stats.entry(endpoint).or_default();
            endpoint_stat.requests.fetch_add(1, Ordering::Relaxed);
            if !success {
                endpoint_stat.errors.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Update endpoint-specific timing (separate lock to avoid nesting)
        {
            let mut timing = self.endpoint_timing.write().await;
            timing.entry(endpoint).or_default().record(duration_us);
        }

        // Update collection stats
        if let Some(coll) = collection {
            counter!("edr_requests_total",
                "endpoint" => endpoint.label().to_string(),
                "collection" => coll.to_string()
            )
            .increment(1);

            let mut stats = self.collection_stats.write().await;
            stats
                .entry(coll.to_string())
                .or_default()
                .requests
                .fetch_add(1, Ordering::Relaxed);
        }

        // Update parameter stats
        for param in parameters {
            counter!("edr_parameter_requests_total", "parameter" => param.to_string()).increment(1);

            let mut stats = self.parameter_stats.write().await;
            stats
                .entry(param.clone())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }

        // Update format stats
        counter!("edr_format_requests_total", "format" => format.label().to_string()).increment(1);
        {
            let mut stats = self.format_stats.write().await;
            stats
                .entry(format)
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }

        // Update client tracking
        if let Some(ip) = client_ip {
            let mut clients = self.client_requests.write().await;
            *clients.entry(ip.to_string()).or_insert(0) += 1;
        }

        // Update user-agent tracking
        if let Some(ua) = user_agent {
            // Simplify user-agent to just the first part (browser/client name)
            let ua_simple = ua.split('/').next().unwrap_or(ua);
            let ua_simple = ua_simple.split(' ').next().unwrap_or(ua_simple);
            let mut agents = self.user_agent_requests.write().await;
            *agents.entry(ua_simple.to_string()).or_insert(0) += 1;
        }
    }

    /// Record a query extent for geographic heatmap.
    pub async fn record_query_extent(
        &self,
        min_lon: f64,
        min_lat: f64,
        max_lon: f64,
        max_lat: f64,
        query_type: &str,
        collection: &str,
    ) {
        let mut heatmap = self.query_heatmap.write().await;
        heatmap.record_bbox(min_lon, min_lat, max_lon, max_lat, query_type, collection);
    }

    /// Record a point query location.
    pub async fn record_point_query(&self, lon: f64, lat: f64, collection: &str) {
        let mut heatmap = self.query_heatmap.write().await;
        heatmap.record_point(lon, lat, collection);
    }

    /// Record a radius query location.
    pub async fn record_radius_query(&self, lon: f64, lat: f64, radius_m: f64, collection: &str) {
        let mut heatmap = self.query_heatmap.write().await;
        heatmap.record_radius(lon, lat, radius_m, collection);
    }

    /// Record a cache hit.
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        counter!("edr_cache_hits_total").increment(1);
    }

    /// Record a cache miss.
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        counter!("edr_cache_misses_total").increment(1);
    }

    /// Get a snapshot of current metrics.
    pub async fn snapshot(&self) -> MetricsSnapshot {
        let overall_timing = self.overall_timing.read().await;
        let rate_tracker = self.rate_tracker.read().await;

        let cache_hits = self.cache_hits.load(Ordering::Relaxed);
        let cache_misses = self.cache_misses.load(Ordering::Relaxed);
        let cache_total = cache_hits + cache_misses;
        let cache_hit_rate = if cache_total > 0 {
            (cache_hits as f64 / cache_total as f64) * 100.0
        } else {
            0.0
        };

        // Build per-endpoint stats
        let endpoint_stats = self.endpoint_stats.read().await;
        let endpoint_timing = self.endpoint_timing.read().await;
        let mut endpoints = HashMap::new();
        for (endpoint, stats) in endpoint_stats.iter() {
            let timing = endpoint_timing.get(endpoint);
            endpoints.insert(
                *endpoint,
                EndpointMetrics {
                    requests: stats.requests.load(Ordering::Relaxed),
                    errors: stats.errors.load(Ordering::Relaxed),
                    avg_ms: timing.map(|t| t.avg_ms()).unwrap_or(0.0),
                    p50_ms: timing.map(|t| t.percentile_ms(50.0)).unwrap_or(0.0),
                    p90_ms: timing.map(|t| t.percentile_ms(90.0)).unwrap_or(0.0),
                    p99_ms: timing.map(|t| t.percentile_ms(99.0)).unwrap_or(0.0),
                    max_ms: timing.map(|t| t.max_ms()).unwrap_or(0.0),
                },
            );
        }

        // Build per-collection stats
        let collection_stats = self.collection_stats.read().await;
        let mut collections = HashMap::new();
        for (name, stats) in collection_stats.iter() {
            collections.insert(name.clone(), stats.requests.load(Ordering::Relaxed));
        }

        // Build per-parameter stats
        let parameter_stats = self.parameter_stats.read().await;
        let mut parameters = HashMap::new();
        for (name, count) in parameter_stats.iter() {
            parameters.insert(name.clone(), count.load(Ordering::Relaxed));
        }

        // Build per-format stats
        let format_stats = self.format_stats.read().await;
        let mut formats = HashMap::new();
        for (format, count) in format_stats.iter() {
            formats.insert(*format, count.load(Ordering::Relaxed));
        }

        // Get top clients
        let client_requests = self.client_requests.read().await;
        let mut top_clients: Vec<_> = client_requests.iter().collect();
        top_clients.sort_by(|a, b| b.1.cmp(a.1));
        let top_clients: Vec<(String, u64)> = top_clients
            .into_iter()
            .take(10)
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        // Get query heatmap
        let heatmap = self.query_heatmap.read().await;
        let query_extents = heatmap.snapshot();

        MetricsSnapshot {
            uptime_secs: self.start_time.elapsed().as_secs(),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
            rate_1m: rate_tracker.rate_1m(),
            rate_5m: rate_tracker.rate_5m(),
            count_1m: rate_tracker.count_1m(),
            count_5m: rate_tracker.count_5m(),
            cache_hits,
            cache_misses,
            cache_hit_rate,
            avg_ms: overall_timing.avg_ms(),
            p50_ms: overall_timing.percentile_ms(50.0),
            p90_ms: overall_timing.percentile_ms(90.0),
            p99_ms: overall_timing.percentile_ms(99.0),
            max_ms: overall_timing.max_ms(),
            endpoints,
            collections,
            parameters,
            formats,
            top_clients,
            query_extents,
        }
    }

    /// Get the query heatmap data for Grafana geomap.
    pub async fn get_query_heatmap(&self) -> Vec<QueryExtent> {
        self.query_heatmap.read().await.snapshot()
    }

    /// Clear the query heatmap.
    pub async fn clear_query_heatmap(&self) {
        self.query_heatmap.write().await.clear();
    }

    /// Reset all metrics (useful for testing).
    pub async fn reset(&self) {
        self.total_requests.store(0, Ordering::Relaxed);
        self.total_errors.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.endpoint_stats.write().await.clear();
        self.endpoint_timing.write().await.clear();
        self.collection_stats.write().await.clear();
        self.parameter_stats.write().await.clear();
        self.format_stats.write().await.clear();
        *self.rate_tracker.write().await = RateTracker::new(Instant::now());
        *self.overall_timing.write().await = TimingStats::default();
        self.query_heatmap.write().await.clear();
        self.client_requests.write().await.clear();
        self.user_agent_requests.write().await.clear();
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-endpoint metrics for snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointMetrics {
    pub requests: u64,
    pub errors: u64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// Snapshot of current metrics for JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub uptime_secs: u64,
    pub total_requests: u64,
    pub total_errors: u64,
    pub rate_1m: f64,
    pub rate_5m: f64,
    pub count_1m: u64,
    pub count_5m: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_hit_rate: f64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub endpoints: HashMap<EndpointType, EndpointMetrics>,
    pub collections: HashMap<String, u64>,
    pub parameters: HashMap<String, u64>,
    pub formats: HashMap<FormatType, u64>,
    pub top_clients: Vec<(String, u64)>,
    pub query_extents: Vec<QueryExtent>,
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

/// Extract client IP from request headers.
/// Checks X-Forwarded-For, X-Real-IP, then falls back to connection info.
pub fn extract_client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    // Try X-Forwarded-For first (may contain multiple IPs, take the first)
    if let Some(xff) = headers.get("x-forwarded-for") {
        if let Ok(xff_str) = xff.to_str() {
            if let Some(first_ip) = xff_str.split(',').next() {
                return Some(first_ip.trim().to_string());
            }
        }
    }

    // Try X-Real-IP
    if let Some(xri) = headers.get("x-real-ip") {
        if let Ok(ip) = xri.to_str() {
            return Some(ip.to_string());
        }
    }

    None
}

/// Extract User-Agent from request headers.
pub fn extract_user_agent(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|ua| ua.to_str().ok())
        .map(|s| s.to_string())
}

/// Convert OutputFormat to FormatType for metrics.
pub fn format_from_output(format: &crate::content_negotiation::OutputFormat) -> FormatType {
    match format {
        crate::content_negotiation::OutputFormat::CoverageJson => FormatType::CoverageJson,
        crate::content_negotiation::OutputFormat::GeoJson => FormatType::GeoJson,
        crate::content_negotiation::OutputFormat::Png => FormatType::Png,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_collector_basic() {
        let collector = MetricsCollector::new();

        collector
            .record_request(
                EndpointType::Position,
                Some("gfs_temperature"),
                &["temperature".to_string()],
                FormatType::CoverageJson,
                1000,
                true,
                Some("127.0.0.1"),
                Some("curl/7.68.0"),
            )
            .await;

        let snapshot = collector.snapshot().await;
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.total_errors, 0);
        assert!(snapshot.endpoints.contains_key(&EndpointType::Position));
    }

    #[tokio::test]
    async fn test_query_heatmap() {
        let collector = MetricsCollector::new();

        collector
            .record_point_query(-97.5, 35.2, "gfs_temperature")
            .await;
        collector
            .record_point_query(-97.5, 35.2, "gfs_temperature")
            .await;

        let heatmap = collector.get_query_heatmap().await;
        assert!(!heatmap.is_empty());
        assert!(heatmap.iter().any(|e| e.count >= 2));
    }

    #[test]
    fn test_timing_percentiles() {
        let mut timing = TimingStats::default();
        for i in 1..=100 {
            timing.record(i * 1000); // 1-100ms
        }

        assert!((timing.percentile_ms(50.0) - 50.0).abs() < 2.0);
        assert!((timing.percentile_ms(90.0) - 90.0).abs() < 2.0);
        assert!((timing.percentile_ms(99.0) - 99.0).abs() < 2.0);
    }
}
