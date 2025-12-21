//! Health checks, metrics, and monitoring endpoints.

use axum::{
    extract::Extension,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
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
pub async fn metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Response {
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
    if let Some(mem_used) = container_stats.get("memory_used_bytes").and_then(|v| v.as_u64()) {
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
#[instrument(skip(state))]
pub async fn api_metrics_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let chunk_stats = state.grid_processor_factory.cache_stats().await;
    let l1_stats = state.tile_memory_cache.stats();
    let l2_hits = state.metrics.cache_hits.load(Ordering::Relaxed);
    let l2_misses = state.metrics.cache_misses.load(Ordering::Relaxed);
    
    let chunk_hit_rate = if chunk_stats.hits + chunk_stats.misses > 0 {
        (chunk_stats.hits as f64 / (chunk_stats.hits + chunk_stats.misses) as f64) * 100.0
    } else {
        0.0
    };
    
    Json(serde_json::json!({
        "requests": {
            "wms": state.metrics.wms_requests.load(Ordering::Relaxed),
            "wmts": state.metrics.wmts_requests.load(Ordering::Relaxed),
            "total": state.metrics.wms_requests.load(Ordering::Relaxed) + state.metrics.wmts_requests.load(Ordering::Relaxed)
        },
        "cache": {
            "l1_hits": l1_stats.hits.load(Ordering::Relaxed),
            "l1_misses": l1_stats.misses.load(Ordering::Relaxed),
            "l2_hits": l2_hits,
            "l2_misses": l2_misses
        },
        "chunk_cache": {
            "entries": chunk_stats.entries,
            "bytes": chunk_stats.memory_bytes,
            "memory_mb": chunk_stats.memory_bytes as f64 / 1024.0 / 1024.0,
            "hits": chunk_stats.hits,
            "misses": chunk_stats.misses,
            "hit_rate": chunk_hit_rate,
            "evictions": chunk_stats.evictions
        },
        "l1_cache": {
            "size_bytes": l1_stats.size_bytes.load(Ordering::Relaxed),
            "hits": l1_stats.hits.load(Ordering::Relaxed),
            "misses": l1_stats.misses.load(Ordering::Relaxed)
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
pub async fn container_stats_handler() -> impl IntoResponse {
    Json(read_container_stats())
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
    let (mem_total, mem_free, mem_available) = read_proc_meminfo();
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
            if s == "max" { None } else { s.parse().ok() }
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
