//! Metrics collection and statistics.

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Collects metrics during load test execution.
pub struct MetricsCollector {
    histogram: Histogram<u64>,
    requests_total: u64,
    requests_success: u64,
    requests_failed: u64,
    cache_hits: u64,
    cache_misses: u64,
    bytes_total: u64,
    _start_time: Instant,
    first_request_time: Option<Instant>,
    last_request_time: Option<Instant>,
}

impl MetricsCollector {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self {
            histogram: Histogram::new(3).expect("Failed to create histogram"),
            requests_total: 0,
            requests_success: 0,
            requests_failed: 0,
            cache_hits: 0,
            cache_misses: 0,
            bytes_total: 0,
            _start_time: Instant::now(),
            first_request_time: None,
            last_request_time: None,
        }
    }

    /// Record a successful request.
    pub fn record_success(&mut self, latency_us: u64, bytes: usize, cache_hit: bool) {
        self.requests_total += 1;
        self.requests_success += 1;
        self.bytes_total += bytes as u64;
        self.histogram.record(latency_us).ok();

        if cache_hit {
            self.cache_hits += 1;
        } else {
            self.cache_misses += 1;
        }

        let now = Instant::now();
        if self.first_request_time.is_none() {
            self.first_request_time = Some(now);
        }
        self.last_request_time = Some(now);
    }

    /// Record a failed request.
    pub fn record_failure(&mut self) {
        self.requests_total += 1;
        self.requests_failed += 1;
    }

    /// Generate final test results.
    pub fn results(&self, config_name: String) -> TestResults {
        let duration = self
            .last_request_time
            .and_then(|last| self.first_request_time.map(|first| last.duration_since(first)))
            .unwrap_or_default();

        let duration_secs = duration.as_secs_f64();
        let rps = if duration_secs > 0.0 {
            self.requests_total as f64 / duration_secs
        } else {
            0.0
        };

        let cache_total = self.cache_hits + self.cache_misses;
        let cache_hit_rate = if cache_total > 0 {
            (self.cache_hits as f64 / cache_total as f64) * 100.0
        } else {
            0.0
        };

        TestResults {
            config_name,
            duration_secs,
            total_requests: self.requests_total,
            successful_requests: self.requests_success,
            failed_requests: self.requests_failed,
            requests_per_second: rps,
            latency_p50: self.histogram.value_at_percentile(50.0) as f64 / 1000.0,
            latency_p75: self.histogram.value_at_percentile(75.0) as f64 / 1000.0,
            latency_p90: self.histogram.value_at_percentile(90.0) as f64 / 1000.0,
            latency_p95: self.histogram.value_at_percentile(95.0) as f64 / 1000.0,
            latency_p99: self.histogram.value_at_percentile(99.0) as f64 / 1000.0,
            latency_min: self.histogram.min() as f64 / 1000.0,
            latency_max: self.histogram.max() as f64 / 1000.0,
            latency_avg: self.histogram.mean() / 1000.0,
            cache_hit_rate,
            bytes_per_second: if duration_secs > 0.0 {
                self.bytes_total as f64 / duration_secs
            } else {
                0.0
            },
            tiles_per_second: rps,
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Final test results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub config_name: String,
    pub duration_secs: f64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub requests_per_second: f64,

    // Latency percentiles (ms)
    pub latency_p50: f64,
    pub latency_p75: f64,
    pub latency_p90: f64,
    pub latency_p95: f64,
    pub latency_p99: f64,
    pub latency_min: f64,
    pub latency_max: f64,
    pub latency_avg: f64,

    // Cache stats
    pub cache_hit_rate: f64,

    // Throughput
    pub bytes_per_second: f64,
    pub tiles_per_second: f64,
}
