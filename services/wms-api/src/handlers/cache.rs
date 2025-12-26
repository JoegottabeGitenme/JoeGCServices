//! Cache management and configuration reload handlers.

use axum::{extract::Extension, http::StatusCode, response::IntoResponse, Json};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{info, instrument};

use crate::state::AppState;

/// POST /api/cache/clear - Clear all in-memory caches
#[instrument(skip(state))]
pub async fn cache_clear_handler(Extension(state): Extension<Arc<AppState>>) -> impl IntoResponse {
    info!("Clearing all caches");

    // Clear L1 tile cache
    state.tile_memory_cache.clear().await;

    // Clear chunk cache
    state.grid_processor_factory.clear_chunk_cache().await;

    (StatusCode::OK, "All caches cleared")
}

/// GET /api/cache/list - List all cached tiles
#[instrument(skip(state))]
pub async fn cache_list_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let l1_stats = state.tile_memory_cache.stats();
    let chunk_stats = state.grid_processor_factory.cache_stats().await;

    Json(serde_json::json!({
        "l1_cache": {
            "size_bytes": l1_stats.size_bytes.load(Ordering::Relaxed),
            "hits": l1_stats.hits.load(Ordering::Relaxed),
            "misses": l1_stats.misses.load(Ordering::Relaxed)
        },
        "chunk_cache": {
            "entries": chunk_stats.entries,
            "bytes": chunk_stats.memory_bytes,
            "hits": chunk_stats.hits,
            "misses": chunk_stats.misses
        }
    }))
}

/// POST /api/config/reload/layers - Hot reload layer configurations
#[instrument(skip(state))]
pub async fn config_reload_layers_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Reloading layer configurations");

    let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "config".to_string());
    let layer_config_dir = format!("{}/layers", config_dir);

    let new_registry =
        crate::layer_config::LayerConfigRegistry::load_from_directory(&layer_config_dir);
    let mut configs = state.layer_configs.write().await;
    *configs = new_registry;

    // Invalidate capabilities cache when layer configs change
    state.capabilities_cache.invalidate().await;

    info!("Layer configurations reloaded successfully");
    (StatusCode::OK, "Layer configurations reloaded")
}

/// POST /api/config/reload - Full config reload and cache clear
#[instrument(skip(state))]
pub async fn config_reload_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Full configuration reload");

    // Reload layer configs
    let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "config".to_string());
    let layer_config_dir = format!("{}/layers", config_dir);

    let new_registry =
        crate::layer_config::LayerConfigRegistry::load_from_directory(&layer_config_dir);
    let mut configs = state.layer_configs.write().await;
    *configs = new_registry;

    // Clear caches
    state.tile_memory_cache.clear().await;
    state.grid_processor_factory.clear_chunk_cache().await;

    // Invalidate capabilities cache when config changes
    state.capabilities_cache.invalidate().await;

    (StatusCode::OK, "Configuration reloaded and caches cleared")
}

/// GET /api/config - Show current optimization settings
#[instrument(skip(state))]
pub async fn config_handler(Extension(state): Extension<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = &state.optimization_config;

    Json(serde_json::json!({
        "l1_cache": {
            "enabled": config.l1_cache_enabled,
            "size": config.l1_cache_size,
            "ttl_secs": config.l1_cache_ttl_secs
        },
        "prefetch": {
            "enabled": config.prefetch_enabled,
            "min_zoom": config.prefetch_min_zoom,
            "max_zoom": config.prefetch_max_zoom,
            "rings": state.prefetch_rings
        },
        "chunk_cache": {
            "max_memory_mb": config.chunk_cache_size_mb
        }
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_cache_module_compiles() {
        assert!(true);
    }
}
