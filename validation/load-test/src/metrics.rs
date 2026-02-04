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
    pub fn results(
        &self,
        config_name: String,
        scenario_name: String,
        layers: Vec<String>,
        concurrency: u32,
        system_config: Option<SystemConfig>,
    ) -> TestResults {
        let duration = self
            .last_request_time
            .and_then(|last| {
                self.first_request_time
                    .map(|first| last.duration_since(first))
            })
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
            timestamp: chrono::Utc::now().to_rfc3339(),
            scenario_name,
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
            layers,
            concurrency,
            system_config,
            git_info: GitInfo::capture(),
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
    pub timestamp: String,
    pub scenario_name: String,
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

    // Test configuration
    pub layers: Vec<String>,
    pub concurrency: u32,

    // System configuration at test time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_config: Option<SystemConfig>,

    // Git metadata for tracking code changes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_info: Option<GitInfo>,
}

/// Git repository information captured at test time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    pub commit_hash: String,
    pub commit_short: String,
    pub branch: String,
    pub commit_message: String,
    pub commit_author: String,
    pub commit_date: String,
    pub is_dirty: bool,
}

/// System configuration captured at test time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub l1_cache_enabled: bool,
    pub l1_cache_size: usize,
    pub l1_cache_ttl_secs: u64,
    #[serde(default)]
    pub l2_cache_enabled: bool,
    #[serde(default)]
    pub chunk_cache_enabled: bool,
    #[serde(default)]
    pub chunk_cache_size_mb: usize,
    pub prefetch_enabled: bool,
    pub prefetch_rings: u32,
    pub prefetch_min_zoom: u32,
    pub prefetch_max_zoom: u32,
    pub cache_warming_enabled: bool,
}

impl GitInfo {
    /// Capture current git repository state
    pub fn capture() -> Option<Self> {
        use std::process::Command;

        // Get commit hash
        let commit_hash = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())?;

        let commit_short = commit_hash.chars().take(7).collect();

        // Get branch name
        let branch = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Get commit message (first line)
        let commit_message = Command::new("git")
            .args(["log", "-1", "--pretty=%s"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Get commit author
        let commit_author = Command::new("git")
            .args(["log", "-1", "--pretty=%an"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Get commit date
        let commit_date = Command::new("git")
            .args(["log", "-1", "--pretty=%ci"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Check if working directory is dirty
        let is_dirty = Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .ok()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        Some(GitInfo {
            commit_hash,
            commit_short,
            branch,
            commit_message,
            commit_author,
            commit_date,
            is_dirty,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collector_new() {
        let collector = MetricsCollector::new();
        // Verify initial state
        let results = collector.results(
            "test".to_string(),
            "scenario".to_string(),
            vec!["layer1".to_string()],
            1,
            None,
        );
        assert_eq!(results.total_requests, 0);
        assert_eq!(results.successful_requests, 0);
        assert_eq!(results.failed_requests, 0);
    }

    #[test]
    fn test_metrics_collector_default() {
        let collector = MetricsCollector::default();
        let results =
            collector.results("test".to_string(), "scenario".to_string(), vec![], 1, None);
        assert_eq!(results.total_requests, 0);
    }

    #[test]
    fn test_record_success() {
        let mut collector = MetricsCollector::new();

        collector.record_success(1000, 1024, true); // 1ms, 1KB, cache hit
        collector.record_success(2000, 2048, false); // 2ms, 2KB, cache miss

        let results = collector.results(
            "test".to_string(),
            "scenario".to_string(),
            vec!["layer1".to_string()],
            1,
            None,
        );

        assert_eq!(results.total_requests, 2);
        assert_eq!(results.successful_requests, 2);
        assert_eq!(results.failed_requests, 0);
    }

    #[test]
    fn test_record_failure() {
        let mut collector = MetricsCollector::new();

        collector.record_failure();
        collector.record_failure();
        collector.record_success(1000, 1024, false);

        let results =
            collector.results("test".to_string(), "scenario".to_string(), vec![], 1, None);

        assert_eq!(results.total_requests, 3);
        assert_eq!(results.successful_requests, 1);
        assert_eq!(results.failed_requests, 2);
    }

    #[test]
    fn test_cache_hit_rate_calculation() {
        let mut collector = MetricsCollector::new();

        // 3 hits, 1 miss = 75% hit rate
        collector.record_success(1000, 100, true);
        collector.record_success(1000, 100, true);
        collector.record_success(1000, 100, true);
        collector.record_success(1000, 100, false);

        let results =
            collector.results("test".to_string(), "scenario".to_string(), vec![], 1, None);

        assert!((results.cache_hit_rate - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_cache_hit_rate_all_hits() {
        let mut collector = MetricsCollector::new();

        collector.record_success(1000, 100, true);
        collector.record_success(1000, 100, true);

        let results =
            collector.results("test".to_string(), "scenario".to_string(), vec![], 1, None);

        assert!((results.cache_hit_rate - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_cache_hit_rate_no_requests() {
        let collector = MetricsCollector::new();

        let results =
            collector.results("test".to_string(), "scenario".to_string(), vec![], 1, None);

        // No requests = 0% hit rate
        assert_eq!(results.cache_hit_rate, 0.0);
    }

    #[test]
    fn test_results_includes_layers() {
        let collector = MetricsCollector::new();
        let layers = vec!["gfs_TMP".to_string(), "hrrr_WIND".to_string()];

        let results = collector.results(
            "config".to_string(),
            "scenario".to_string(),
            layers.clone(),
            5,
            None,
        );

        assert_eq!(results.layers, layers);
        assert_eq!(results.concurrency, 5);
    }

    #[test]
    fn test_results_includes_system_config() {
        let collector = MetricsCollector::new();
        let sys_config = SystemConfig {
            l1_cache_enabled: true,
            l1_cache_size: 1000,
            l1_cache_ttl_secs: 300,
            l2_cache_enabled: false,
            chunk_cache_enabled: true,
            chunk_cache_size_mb: 512,
            prefetch_enabled: true,
            prefetch_rings: 2,
            prefetch_min_zoom: 4,
            prefetch_max_zoom: 10,
            cache_warming_enabled: false,
        };

        let results = collector.results(
            "config".to_string(),
            "scenario".to_string(),
            vec![],
            1,
            Some(sys_config.clone()),
        );

        let returned_config = results.system_config.unwrap();
        assert!(returned_config.l1_cache_enabled);
        assert_eq!(returned_config.l1_cache_size, 1000);
        assert!(returned_config.chunk_cache_enabled);
    }

    #[test]
    fn test_results_has_timestamp() {
        let collector = MetricsCollector::new();
        let results =
            collector.results("test".to_string(), "scenario".to_string(), vec![], 1, None);

        // Timestamp should be valid RFC3339
        assert!(results.timestamp.contains('T'));
        assert!(results.timestamp.contains('-'));
    }

    #[test]
    fn test_test_results_serialize() {
        let results = TestResults {
            timestamp: "2026-01-15T12:00:00Z".to_string(),
            scenario_name: "test".to_string(),
            config_name: "config".to_string(),
            duration_secs: 60.0,
            total_requests: 1000,
            successful_requests: 990,
            failed_requests: 10,
            requests_per_second: 16.5,
            latency_p50: 50.0,
            latency_p75: 75.0,
            latency_p90: 90.0,
            latency_p95: 95.0,
            latency_p99: 150.0,
            latency_min: 10.0,
            latency_max: 500.0,
            latency_avg: 55.0,
            cache_hit_rate: 80.0,
            bytes_per_second: 1024.0,
            tiles_per_second: 16.5,
            layers: vec!["layer1".to_string()],
            concurrency: 10,
            system_config: None,
            git_info: None,
        };

        let json = serde_json::to_string(&results).unwrap();
        assert!(json.contains("\"total_requests\":1000"));
        assert!(json.contains("\"cache_hit_rate\":80.0"));
    }

    #[test]
    fn test_git_info_capture() {
        // This test should work in a git repository
        let info = GitInfo::capture();
        // In a git repo, this should succeed
        if let Some(git) = info {
            assert!(!git.commit_hash.is_empty());
            assert_eq!(git.commit_short.len(), 7);
            assert!(!git.branch.is_empty());
        }
        // If not in a git repo, the function returns None, which is also valid
    }

    #[test]
    fn test_system_config_serialize() {
        let config = SystemConfig {
            l1_cache_enabled: true,
            l1_cache_size: 100,
            l1_cache_ttl_secs: 60,
            l2_cache_enabled: false,
            chunk_cache_enabled: true,
            chunk_cache_size_mb: 256,
            prefetch_enabled: false,
            prefetch_rings: 1,
            prefetch_min_zoom: 5,
            prefetch_max_zoom: 12,
            cache_warming_enabled: true,
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"l1_cache_enabled\":true"));
        assert!(json.contains("\"chunk_cache_size_mb\":256"));
    }
}
