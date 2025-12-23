//! Health checks, metrics, and monitoring endpoints.

use axum::{
    extract::Extension,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::instrument;

use crate::state::AppState;

// ============================================================================
// Health Checks
// ============================================================================

/// GET /health - Basic health check
pub async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// GET /ready - Readiness check (verifies database connectivity)
pub async fn ready_handler(Extension(state): Extension<Arc<AppState>>) -> impl IntoResponse {
    match state.catalog.list_models().await {
        Ok(_) => (StatusCode::OK, "Ready"),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "Not ready"),
    }
}

// ============================================================================
// Prometheus Metrics
// ============================================================================

/// GET /metrics - Prometheus metrics endpoint
#[instrument(skip(state))]
pub async fn metrics_handler(Extension(state): Extension<Arc<AppState>>) -> Response {
    // Get cache statistics (async calls)
    let chunk_stats = state.grid_processor_factory.cache_stats().await;
    let l1_stats = state.tile_memory_cache.stats();
    let container_stats = read_container_stats();

    let mut output = String::new();

    // WMS/WMTS request counts (fields on metrics struct)
    output.push_str(&format!(
        "# HELP wms_requests_total Total WMS requests\n# TYPE wms_requests_total counter\nwms_requests_total {}\n",
        state.metrics.wms_requests.load(Ordering::Relaxed)
    ));
    output.push_str(&format!(
        "# HELP wmts_requests_total Total WMTS requests\n# TYPE wmts_requests_total counter\nwmts_requests_total {}\n",
        state.metrics.wmts_requests.load(Ordering::Relaxed)
    ));

    // Chunk cache metrics
    output.push_str(&format!(
        "# HELP chunk_cache_entries Current chunk cache entries\n# TYPE chunk_cache_entries gauge\nchunk_cache_entries {}\n",
        chunk_stats.entries
    ));
    output.push_str(&format!(
        "# HELP chunk_cache_bytes Current chunk cache size in bytes\n# TYPE chunk_cache_bytes gauge\nchunk_cache_bytes {}\n",
        chunk_stats.memory_bytes
    ));
    output.push_str(&format!(
        "# HELP chunk_cache_hits Total chunk cache hits\n# TYPE chunk_cache_hits counter\nchunk_cache_hits {}\n",
        chunk_stats.hits
    ));
    output.push_str(&format!(
        "# HELP chunk_cache_misses Total chunk cache misses\n# TYPE chunk_cache_misses counter\nchunk_cache_misses {}\n",
        chunk_stats.misses
    ));

    // L1 tile cache metrics
    output.push_str(&format!(
        "# HELP l1_cache_size_bytes Current L1 tile cache size in bytes\n# TYPE l1_cache_size_bytes gauge\nl1_cache_size_bytes {}\n",
        l1_stats.size_bytes.load(Ordering::Relaxed)
    ));
    output.push_str(&format!(
        "# HELP l1_cache_hits Total L1 cache hits\n# TYPE l1_cache_hits counter\nl1_cache_hits {}\n",
        l1_stats.hits.load(Ordering::Relaxed)
    ));
    output.push_str(&format!(
        "# HELP l1_cache_misses Total L1 cache misses\n# TYPE l1_cache_misses counter\nl1_cache_misses {}\n",
        l1_stats.misses.load(Ordering::Relaxed)
    ));

    // Container memory metrics
    if let Some(mem_used) = container_stats
        .get("memory_used_bytes")
        .and_then(|v| v.as_u64())
    {
        output.push_str(&format!(
            "# HELP process_memory_bytes Memory used by process\n# TYPE process_memory_bytes gauge\nprocess_memory_bytes {}\n",
            mem_used
        ));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; version=0.0.4")
        .body(output.into())
        .unwrap()
}

// ============================================================================
// JSON Metrics API
// ============================================================================

/// GET /api/metrics - JSON metrics for web UI
///
/// Returns comprehensive metrics in a format expected by the dashboard:
/// - metrics: request counts, render stats, rates, per-data-source stats
/// - l1_cache: tile memory cache stats
/// - l2_cache: Redis cache stats
/// - chunk_cache: Zarr chunk cache stats
/// - system: system resource stats
#[instrument(skip(state))]
pub async fn api_metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    // Get the comprehensive metrics snapshot
    let snapshot = state.metrics.snapshot().await;

    // Get cache statistics
    let chunk_stats = state.grid_processor_factory.cache_stats().await;
    let l1_stats = state.tile_memory_cache.stats();

    // Calculate L1 hit rate
    let l1_hits = l1_stats.hits.load(Ordering::Relaxed);
    let l1_misses = l1_stats.misses.load(Ordering::Relaxed);
    let l1_total = l1_hits + l1_misses;
    let l1_hit_rate = if l1_total > 0 {
        (l1_hits as f64 / l1_total as f64) * 100.0
    } else {
        0.0
    };

    // Calculate chunk cache hit rate
    let chunk_hit_rate = if chunk_stats.hits + chunk_stats.misses > 0 {
        (chunk_stats.hits as f64 / (chunk_stats.hits + chunk_stats.misses) as f64) * 100.0
    } else {
        0.0
    };

    // Get container/system stats
    let container_stats = read_container_stats();

    // Get Redis (L2) cache stats
    let (l2_connected, l2_key_count, l2_memory_used) = {
        let mut cache = state.cache.lock().await;
        match cache.stats().await {
            Ok(stats) => (true, stats.key_count, stats.memory_used),
            Err(_) => (false, 0, 0),
        }
    };

    // Build data_source_stats with defaults for known sources
    // This ensures GFS, GOES, HRRR, MRMS always appear in the dashboard
    let mut data_source_stats = serde_json::json!({
        "gfs": { "cache_hit_rate": 0.0, "avg_parse_ms": 0.0, "parse_count": 0, "cache_hits": 0, "cache_misses": 0 },
        "goes": { "cache_hit_rate": 0.0, "avg_parse_ms": 0.0, "parse_count": 0, "cache_hits": 0, "cache_misses": 0 },
        "hrrr": { "cache_hit_rate": 0.0, "avg_parse_ms": 0.0, "parse_count": 0, "cache_hits": 0, "cache_misses": 0 },
        "mrms": { "cache_hit_rate": 0.0, "avg_parse_ms": 0.0, "parse_count": 0, "cache_hits": 0, "cache_misses": 0 }
    });
    // Merge in actual stats from snapshot
    if let serde_json::Value::Object(ref mut map) = data_source_stats {
        for (key, value) in &snapshot.data_source_stats {
            map.insert(key.clone(), serde_json::to_value(value).unwrap_or_default());
        }
    }

    Json(serde_json::json!({
        // Main metrics object - all request/render stats from snapshot
        "metrics": {
            "uptime_secs": snapshot.uptime_secs,

            // Request counts
            "wms_requests": snapshot.wms_requests,
            "wmts_requests": snapshot.wmts_requests,

            // Request rates
            "wms_rate_1m": snapshot.wms_rate_1m,
            "wms_rate_5m": snapshot.wms_rate_5m,
            "wms_count_1m": snapshot.wms_count_1m,
            "wms_count_5m": snapshot.wms_count_5m,
            "wmts_rate_1m": snapshot.wmts_rate_1m,
            "wmts_rate_5m": snapshot.wmts_rate_5m,
            "wmts_count_1m": snapshot.wmts_count_1m,
            "wmts_count_5m": snapshot.wmts_count_5m,

            // Cache stats (L2/Redis - from snapshot)
            "cache_hits": snapshot.cache_hits,
            "cache_misses": snapshot.cache_misses,
            "cache_hit_rate": snapshot.cache_hit_rate,

            // Render stats
            "renders_total": snapshot.renders_total,
            "render_errors": snapshot.render_errors,
            "render_avg_ms": snapshot.render_avg_ms,
            "render_last_ms": snapshot.render_last_ms,
            "render_min_ms": snapshot.render_min_ms,
            "render_max_ms": snapshot.render_max_ms,
            "render_rate_1m": snapshot.render_rate_1m,
            "render_rate_5m": snapshot.render_rate_5m,
            "render_count_1m": snapshot.render_count_1m,
            "render_count_5m": snapshot.render_count_5m,

            // Per-data-source stats (GFS, HRRR, GOES, MRMS)
            "data_source_stats": data_source_stats,

            // Pipeline timing breakdown
            "grib_load_avg_ms": snapshot.grib_load_avg_ms,
            "grib_parse_avg_ms": snapshot.grib_parse_avg_ms,
            "resample_avg_ms": snapshot.resample_avg_ms,
            "png_encode_avg_ms": snapshot.png_encode_avg_ms,
            "cache_lookup_avg_ms": snapshot.cache_lookup_avg_ms
        },

        // L1 tile cache (in-memory)
        "l1_cache": {
            "hits": l1_hits,
            "misses": l1_misses,
            "hit_rate": l1_hit_rate,
            "size_bytes": l1_stats.size_bytes.load(Ordering::Relaxed),
            "evictions": l1_stats.evictions.load(Ordering::Relaxed),
            "expired": l1_stats.expired.load(Ordering::Relaxed)
        },

        // L2 cache (Redis)
        "l2_cache": {
            "connected": l2_connected,
            "key_count": l2_key_count,
            "memory_used": l2_memory_used
        },

        // Chunk cache (Zarr data)
        "chunk_cache": {
            "entries": chunk_stats.entries,
            "bytes": chunk_stats.memory_bytes,
            "memory_mb": chunk_stats.memory_bytes as f64 / 1024.0 / 1024.0,
            "hits": chunk_stats.hits,
            "misses": chunk_stats.misses,
            "hit_rate": chunk_hit_rate,
            "evictions": chunk_stats.evictions
        },

        // System stats from container
        "system": {
            "memory_used_bytes": container_stats.get("memory_used_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
            "memory_total_bytes": container_stats.get("memory_total_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
            "load_1m": container_stats.get("load_average").and_then(|v| v.get("1m")).and_then(|v| v.as_f64()).unwrap_or(0.0),
            "load_5m": container_stats.get("load_average").and_then(|v| v.get("5m")).and_then(|v| v.as_f64()).unwrap_or(0.0)
        }
    }))
}

// ============================================================================
// Storage Stats
// ============================================================================

/// GET /api/storage/stats - MinIO bucket statistics
#[instrument(skip(state))]
pub async fn storage_stats_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.storage.detailed_stats().await {
        Ok(stats) => Ok(Json(serde_json::json!({
            // Fields expected by web dashboard
            "object_count": stats.raw_object_count + stats.shredded_object_count,
            "total_size": stats.total_size_bytes,
            // Additional breakdown fields
            "total_objects": stats.raw_object_count + stats.shredded_object_count,
            "total_bytes": stats.total_size_bytes,
            "raw_size_bytes": stats.raw_size_bytes,
            "shredded_size_bytes": stats.shredded_size_bytes
        }))),
        Err(e) => {
            tracing::error!(error = %e, "Failed to get storage stats");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ============================================================================
// Container Stats
// ============================================================================

/// GET /api/container/stats - Container resource statistics
///
/// Returns stats in the format expected by the web dashboard:
/// - container: hostname, in_container flag
/// - memory: used_bytes, host_total_bytes, cgroup_limit_bytes, percent_used
/// - process: rss_bytes, vms_bytes
/// - cpu: count, load_1m, load_5m, load_15m
pub async fn container_stats_handler() -> impl IntoResponse {
    let (mem_total, _mem_free, _mem_available) = read_proc_meminfo();
    let (load_1, load_5, load_15) = read_load_average();
    let (cgroup_used, cgroup_limit) = read_cgroup_memory();
    let (vm_rss, vm_size) = read_proc_self_status();
    let cpu_count = read_cpu_count();

    // Calculate memory percent used
    let mem_used_bytes = vm_rss * 1024;
    let mem_total_bytes = mem_total * 1024;
    let effective_limit = if cgroup_limit > 0 {
        cgroup_limit
    } else {
        mem_total_bytes
    };
    let percent_used = if effective_limit > 0 {
        (mem_used_bytes as f64 / effective_limit as f64) * 100.0
    } else {
        0.0
    };

    // Check if running in container
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_string();
    let in_container = std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|s| s.contains("docker") || s.contains("kubepods"))
            .unwrap_or(false);

    Json(serde_json::json!({
        "container": {
            "hostname": hostname,
            "in_container": in_container
        },
        "memory": {
            "used_bytes": mem_used_bytes,
            "host_total_bytes": mem_total_bytes,
            "cgroup_limit_bytes": cgroup_limit,
            "cgroup_used_bytes": cgroup_used,
            "percent_used": percent_used
        },
        "process": {
            "rss_bytes": vm_rss * 1024,
            "vms_bytes": vm_size * 1024
        },
        "cpu": {
            "count": cpu_count,
            "load_1m": load_1,
            "load_5m": load_5,
            "load_15m": load_15
        }
    }))
}

/// GET /api/grid-processor/stats - Zarr grid processor statistics
#[instrument(skip(state))]
pub async fn grid_processor_stats_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let stats = state.grid_processor_factory.cache_stats().await;

    let hit_rate = if stats.hits + stats.misses > 0 {
        (stats.hits as f64 / (stats.hits + stats.misses) as f64) * 100.0
    } else {
        0.0
    };

    let memory_mb = stats.memory_bytes as f64 / 1024.0 / 1024.0;

    Json(serde_json::json!({
        "chunk_cache": {
            "entries": stats.entries,
            "bytes": stats.memory_bytes,
            "memory_bytes": stats.memory_bytes,
            "mb": memory_mb,
            "memory_mb": memory_mb,
            "hits": stats.hits,
            "misses": stats.misses,
            "hit_rate": hit_rate,
            "hit_rate_percent": hit_rate,
            "evictions": stats.evictions,
            "total_requests": stats.hits + stats.misses
        }
    }))
}

// ============================================================================
// Tile Heatmap
// ============================================================================

/// GET /api/tile-heatmap - Geographic distribution of tile requests
#[instrument(skip(state))]
pub async fn tile_heatmap_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<crate::metrics::TileHeatmapSnapshot> {
    let heatmap = state.metrics.get_tile_heatmap().await;
    Json(heatmap)
}

/// POST /api/tile-heatmap/clear - Clear the tile request heatmap
#[instrument(skip(state))]
pub async fn tile_heatmap_clear_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.clear_tile_heatmap().await;
    (StatusCode::OK, "Heatmap cleared")
}

// ============================================================================
// Container Stats Helpers
// ============================================================================

fn read_container_stats() -> serde_json::Value {
    let (mem_total, _mem_free, mem_available) = read_proc_meminfo();
    let (load_1, load_5, load_15) = read_load_average();
    let (cgroup_used, cgroup_limit) = read_cgroup_memory();
    let (vm_rss, vm_size) = read_proc_self_status();

    serde_json::json!({
        "memory_used_bytes": vm_rss * 1024,
        "memory_total_bytes": mem_total * 1024,
        "memory_available_bytes": mem_available * 1024,
        "cgroup_memory_used": cgroup_used,
        "cgroup_memory_limit": cgroup_limit,
        "vm_rss_kb": vm_rss,
        "vm_size_kb": vm_size,
        "load_average": {
            "1m": load_1,
            "5m": load_5,
            "15m": load_15
        }
    })
}

fn read_proc_meminfo() -> (u64, u64, u64) {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total = 0u64;
    let mut free = 0u64;
    let mut available = 0u64;

    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = parse_kb_value(line);
        } else if line.starts_with("MemFree:") {
            free = parse_kb_value(line);
        } else if line.starts_with("MemAvailable:") {
            available = parse_kb_value(line);
        }
    }

    (total, free, available)
}

fn parse_kb_value(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn read_load_average() -> (f64, f64, f64) {
    let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let parts: Vec<&str> = content.split_whitespace().collect();

    let load_1 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let load_5 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let load_15 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);

    (load_1, load_5, load_15)
}

fn read_cgroup_memory() -> (u64, u64) {
    let used = std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .or_else(|_| std::fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let limit = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .or_else(|_| std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes"))
        .ok()
        .and_then(|s| {
            let s = s.trim();
            if s == "max" {
                None
            } else {
                s.parse().ok()
            }
        })
        .unwrap_or(0);

    (used, limit)
}

fn read_proc_self_status() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut vm_rss = 0u64;
    let mut vm_size = 0u64;

    for line in content.lines() {
        if line.starts_with("VmRSS:") {
            vm_rss = parse_kb_value(line);
        } else if line.starts_with("VmSize:") {
            vm_size = parse_kb_value(line);
        }
    }

    (vm_rss, vm_size)
}

fn read_cpu_count() -> u32 {
    // Try to get CPU count from /proc/cpuinfo
    let content = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let count = content
        .lines()
        .filter(|line| line.starts_with("processor"))
        .count();

    if count > 0 {
        count as u32
    } else {
        // Fallback to available parallelism
        std::thread::available_parallelism()
            .map(|p| p.get() as u32)
            .unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kb_value() {
        assert_eq!(parse_kb_value("MemTotal:        8168456 kB"), 8168456);
        assert_eq!(parse_kb_value("VmRSS:     123456 kB"), 123456);
        assert_eq!(parse_kb_value("invalid"), 0);
    }
}
