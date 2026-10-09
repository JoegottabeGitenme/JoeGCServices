//! Cache management and configuration reload handlers.

use axum::{extract::Extension, http::StatusCode, response::IntoResponse, Json};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{error, info, instrument};

use crate::layer_config::LayerConfigRegistry;
use crate::state::AppState;

/// Load the layer registry from `config_dir` (the directory that *contains*
/// `layers/` and `styles/`, i.e. `CONFIG_DIR`), refusing an empty result.
///
/// `LayerConfigRegistry::load_from_directory` appends `layers/` itself. These
/// handlers used to pass `"{CONFIG_DIR}/layers"`, which resolved to
/// `config/layers/layers`: nothing was found and an empty registry replaced the
/// live one, so every layer vanished from GetCapabilities and GetMap until the
/// service was restarted. A reload that finds no layers is now an error and the
/// current configuration is kept.
fn load_layer_registry(config_dir: &str) -> Result<LayerConfigRegistry, String> {
    let registry = LayerConfigRegistry::load_from_directory(config_dir);
    if registry.models().is_empty() {
        return Err(format!(
            "No layer configurations found under '{config_dir}/layers'; keeping the current configuration"
        ));
    }
    Ok(registry)
}

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

    let new_registry = match load_layer_registry(&config_dir) {
        Ok(r) => r,
        Err(msg) => {
            error!("{msg}");
            return (StatusCode::INTERNAL_SERVER_ERROR, msg);
        }
    };
    let mut configs = state.layer_configs.write().await;
    *configs = new_registry;

    // Invalidate capabilities cache when layer configs change
    state.capabilities_cache.invalidate().await;

    info!("Layer configurations reloaded successfully");
    (StatusCode::OK, "Layer configurations reloaded".to_string())
}

/// POST /api/config/reload - Full config reload and cache clear
#[instrument(skip(state))]
pub async fn config_reload_handler(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    info!("Full configuration reload");

    // Reload layer configs
    let config_dir = std::env::var("CONFIG_DIR").unwrap_or_else(|_| "config".to_string());

    let new_registry = match load_layer_registry(&config_dir) {
        Ok(r) => r,
        Err(msg) => {
            error!("{msg}");
            return (StatusCode::INTERNAL_SERVER_ERROR, msg);
        }
    };
    let mut configs = state.layer_configs.write().await;
    *configs = new_registry;

    // Clear caches
    state.tile_memory_cache.clear().await;
    state.grid_processor_factory.clear_chunk_cache().await;

    // Invalidate capabilities cache when config changes
    state.capabilities_cache.invalidate().await;

    (
        StatusCode::OK,
        "Configuration reloaded and caches cleared".to_string(),
    )
}

/// GET /api/config - Show current optimization settings
#[instrument(skip(state))]
pub async fn config_handler(Extension(state): Extension<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = &state.optimization_config;

    Json(serde_json::json!({
        "l1_cache": {
            "enabled": config.l1_cache_enabled,
            "size_mb": config.l1_cache_size_mb,
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
    use super::*;

    #[test]
    fn test_cache_module_compiles() {
        assert!(true);
    }

    fn repo_config_dir() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../config")
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn reload_loads_layers_from_the_config_dir() {
        let registry = load_layer_registry(&repo_config_dir()).expect("repo config must load");
        assert!(registry.get_model("hrrr").is_some());
        assert!(registry.models().len() > 10);
    }

    #[test]
    fn reload_never_replaces_the_live_registry_with_an_empty_one() {
        // The old handlers effectively did this: pointed at `<config>/layers`,
        // so `layers/` was looked up inside it and nothing was found.
        let wrong = format!("{}/layers", repo_config_dir());
        let err = load_layer_registry(&wrong).expect_err("empty result must be an error");
        assert!(err.contains("keeping the current configuration"), "{err}");
        assert!(load_layer_registry("/definitely/not/a/dir").is_err());
    }
}
