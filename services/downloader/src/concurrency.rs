//! Concurrency control for per-model download slots with shared pool.
//!
//! This module implements a two-tier concurrency system:
//! 1. Each model gets 1 guaranteed slot that's always available
//! 2. Additional slots come from a shared pool (first-come, first-served)
//!
//! This ensures that time-sensitive data (like MRMS radar) is never starved
//! by bulk downloads (like GFS forecast files).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::debug;

/// Manages download concurrency across all models.
///
/// Each model gets:
/// - 1 guaranteed slot (always available to that model)
/// - Access to shared pool slots (first-come, first-served)
#[derive(Debug)]
#[allow(dead_code)]
pub struct ConcurrencyManager {
    /// Shared pool semaphore for extra slots beyond guaranteed minimums
    shared_pool: Arc<Semaphore>,
    /// Total maximum concurrent downloads
    total_max: usize,
    /// Number of models
    num_models: usize,
    /// Size of the shared pool
    shared_pool_size: usize,
    /// Track active downloads for metrics
    active_downloads: Arc<AtomicUsize>,
}

#[allow(dead_code)]
impl ConcurrencyManager {
    /// Create a new concurrency manager.
    ///
    /// # Arguments
    /// * `total_max_concurrent` - Maximum total concurrent downloads across all models
    /// * `num_models` - Number of enabled models (determines guaranteed slots)
    pub fn new(total_max_concurrent: usize, num_models: usize) -> Self {
        // Shared pool = total - guaranteed (1 per model)
        // Ensure at least 0 shared slots
        let shared_pool_size = total_max_concurrent.saturating_sub(num_models);

        debug!(
            total_max = total_max_concurrent,
            num_models = num_models,
            shared_pool_size = shared_pool_size,
            "Creating concurrency manager"
        );

        Self {
            shared_pool: Arc::new(Semaphore::new(shared_pool_size)),
            total_max: total_max_concurrent,
            num_models,
            shared_pool_size,
            active_downloads: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get access to the shared pool semaphore
    pub fn shared_pool(&self) -> Arc<Semaphore> {
        self.shared_pool.clone()
    }

    /// Get the total maximum concurrent downloads
    pub fn total_max(&self) -> usize {
        self.total_max
    }

    /// Get the number of models
    pub fn num_models(&self) -> usize {
        self.num_models
    }

    /// Get the shared pool size
    pub fn shared_pool_size(&self) -> usize {
        self.shared_pool_size
    }

    /// Get current number of active downloads (for metrics)
    pub fn active_downloads(&self) -> usize {
        self.active_downloads.load(Ordering::Relaxed)
    }

    /// Get the active downloads counter (for sharing with model runners)
    pub fn active_downloads_counter(&self) -> Arc<AtomicUsize> {
        self.active_downloads.clone()
    }

    /// Get current number of available shared pool slots
    pub fn available_shared_slots(&self) -> usize {
        self.shared_pool.available_permits()
    }
}

/// Per-model download permit manager.
///
/// Each model has its own permit manager that provides:
/// - 1 guaranteed slot via a dedicated semaphore
/// - Access to additional shared pool slots
#[derive(Clone)]
pub struct ModelDownloadPermit {
    /// Model identifier for logging
    model_id: String,
    /// The model's guaranteed concurrent limit (always 1)
    guaranteed_semaphore: Arc<Semaphore>,
    /// Reference to shared pool for additional slots
    shared_pool: Arc<Semaphore>,
    /// How many extra slots this model can use from shared pool
    max_shared: usize,
    /// Track active downloads globally
    active_downloads: Arc<AtomicUsize>,
}

impl ModelDownloadPermit {
    /// Create a new per-model permit manager.
    ///
    /// # Arguments
    /// * `model_id` - Model identifier for logging
    /// * `shared_pool` - Reference to the shared pool semaphore
    /// * `max_concurrent` - Maximum concurrent downloads for this model
    /// * `active_downloads` - Global counter for active downloads
    pub fn new(
        model_id: String,
        shared_pool: Arc<Semaphore>,
        max_concurrent: usize,
        active_downloads: Arc<AtomicUsize>,
    ) -> Self {
        // Each model has 1 guaranteed slot
        let max_shared = max_concurrent.saturating_sub(1);

        debug!(
            model = %model_id,
            max_concurrent = max_concurrent,
            guaranteed = 1,
            max_shared = max_shared,
            "Creating model download permit"
        );

        Self {
            model_id,
            guaranteed_semaphore: Arc::new(Semaphore::new(1)),
            shared_pool,
            max_shared,
            active_downloads,
        }
    }

    /// Acquire a download slot (guaranteed or shared).
    ///
    /// This will:
    /// 1. Try to acquire the guaranteed slot first
    /// 2. If guaranteed is taken and we can use shared, try shared pool
    /// 3. If all else fails, wait for guaranteed slot (never starve)
    ///
    /// Returns a guard that releases the slot on drop.
    pub async fn acquire(&self) -> DownloadSlotGuard {
        // Try to get guaranteed slot first (non-blocking)
        if let Ok(permit) = self.guaranteed_semaphore.clone().try_acquire_owned() {
            self.active_downloads.fetch_add(1, Ordering::Relaxed);
            debug!(model = %self.model_id, slot = "guaranteed", "Acquired download slot");
            return DownloadSlotGuard::Guaranteed {
                permit,
                active_downloads: self.active_downloads.clone(),
                model_id: self.model_id.clone(),
            };
        }

        // If guaranteed is taken and we can use shared slots, try shared pool
        if self.max_shared > 0 {
            if let Ok(permit) = self.shared_pool.clone().try_acquire_owned() {
                self.active_downloads.fetch_add(1, Ordering::Relaxed);
                debug!(model = %self.model_id, slot = "shared", "Acquired download slot");
                return DownloadSlotGuard::Shared {
                    permit,
                    active_downloads: self.active_downloads.clone(),
                    model_id: self.model_id.clone(),
                };
            }
        }

        // Wait for guaranteed slot (never starve)
        let permit = self
            .guaranteed_semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("Semaphore closed unexpectedly");

        self.active_downloads.fetch_add(1, Ordering::Relaxed);
        debug!(model = %self.model_id, slot = "guaranteed", waited = true, "Acquired download slot after waiting");

        DownloadSlotGuard::Guaranteed {
            permit,
            active_downloads: self.active_downloads.clone(),
            model_id: self.model_id.clone(),
        }
    }

    /// Try to acquire a slot without waiting.
    ///
    /// Returns `Some(guard)` if a slot was acquired, `None` if all slots are busy.
    #[allow(dead_code)]
    pub fn try_acquire(&self) -> Option<DownloadSlotGuard> {
        // Try guaranteed first
        if let Ok(permit) = self.guaranteed_semaphore.clone().try_acquire_owned() {
            self.active_downloads.fetch_add(1, Ordering::Relaxed);
            debug!(model = %self.model_id, slot = "guaranteed", "Acquired download slot (try)");
            return Some(DownloadSlotGuard::Guaranteed {
                permit,
                active_downloads: self.active_downloads.clone(),
                model_id: self.model_id.clone(),
            });
        }

        // Try shared pool
        if self.max_shared > 0 {
            if let Ok(permit) = self.shared_pool.clone().try_acquire_owned() {
                self.active_downloads.fetch_add(1, Ordering::Relaxed);
                debug!(model = %self.model_id, slot = "shared", "Acquired download slot (try)");
                return Some(DownloadSlotGuard::Shared {
                    permit,
                    active_downloads: self.active_downloads.clone(),
                    model_id: self.model_id.clone(),
                });
            }
        }

        None
    }

    /// Get the maximum concurrent downloads for this model
    pub fn max_concurrent(&self) -> usize {
        1 + self.max_shared
    }

    /// Get the model ID
    #[allow(dead_code)]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// Guard that holds a download slot and releases it on drop.
#[allow(dead_code)]
pub enum DownloadSlotGuard {
    /// Holding the guaranteed slot
    Guaranteed {
        permit: OwnedSemaphorePermit,
        active_downloads: Arc<AtomicUsize>,
        model_id: String,
    },
    /// Holding a shared pool slot
    Shared {
        permit: OwnedSemaphorePermit,
        active_downloads: Arc<AtomicUsize>,
        model_id: String,
    },
}

impl Drop for DownloadSlotGuard {
    fn drop(&mut self) {
        match self {
            DownloadSlotGuard::Guaranteed {
                active_downloads,
                model_id,
                ..
            } => {
                active_downloads.fetch_sub(1, Ordering::Relaxed);
                debug!(model = %model_id, slot = "guaranteed", "Released download slot");
            }
            DownloadSlotGuard::Shared {
                active_downloads,
                model_id,
                ..
            } => {
                active_downloads.fetch_sub(1, Ordering::Relaxed);
                debug!(model = %model_id, slot = "shared", "Released download slot");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[test]
    fn test_concurrency_manager_creation() {
        let manager = ConcurrencyManager::new(10, 6);
        assert_eq!(manager.total_max(), 10);
        assert_eq!(manager.num_models(), 6);
        assert_eq!(manager.shared_pool_size(), 4); // 10 - 6 = 4
        assert_eq!(manager.active_downloads(), 0);
    }

    #[test]
    fn test_shared_pool_size_edge_cases() {
        // More models than total max
        let manager = ConcurrencyManager::new(4, 6);
        assert_eq!(manager.shared_pool_size(), 0); // saturating_sub

        // Equal models and total max
        let manager = ConcurrencyManager::new(6, 6);
        assert_eq!(manager.shared_pool_size(), 0);

        // Single model
        let manager = ConcurrencyManager::new(10, 1);
        assert_eq!(manager.shared_pool_size(), 9);
    }

    #[tokio::test]
    async fn test_model_permit_guaranteed_slot() {
        let manager = ConcurrencyManager::new(10, 2);
        let permit = ModelDownloadPermit::new(
            "test".to_string(),
            manager.shared_pool(),
            2,
            manager.active_downloads_counter(),
        );

        // First acquire should get guaranteed slot immediately
        let guard1 = permit.acquire().await;
        assert!(matches!(guard1, DownloadSlotGuard::Guaranteed { .. }));
        assert_eq!(manager.active_downloads(), 1);

        drop(guard1);
        assert_eq!(manager.active_downloads(), 0);
    }

    #[tokio::test]
    async fn test_model_permit_shared_slot() {
        let manager = ConcurrencyManager::new(10, 2);
        let permit = ModelDownloadPermit::new(
            "test".to_string(),
            manager.shared_pool(),
            3, // Can use 2 shared slots
            manager.active_downloads_counter(),
        );

        // First acquire gets guaranteed
        let guard1 = permit.acquire().await;
        assert!(matches!(guard1, DownloadSlotGuard::Guaranteed { .. }));

        // Second acquire should get shared slot
        let guard2 = permit.acquire().await;
        assert!(matches!(guard2, DownloadSlotGuard::Shared { .. }));

        assert_eq!(manager.active_downloads(), 2);

        drop(guard1);
        drop(guard2);
        assert_eq!(manager.active_downloads(), 0);
    }

    #[tokio::test]
    async fn test_model_permit_try_acquire() {
        let manager = ConcurrencyManager::new(10, 2);
        let permit = ModelDownloadPermit::new(
            "test".to_string(),
            manager.shared_pool(),
            1, // Only guaranteed slot
            manager.active_downloads_counter(),
        );

        // First try_acquire should succeed
        let guard1 = permit.try_acquire();
        assert!(guard1.is_some());

        // Second try_acquire should fail (only 1 slot, no shared access)
        let guard2 = permit.try_acquire();
        assert!(guard2.is_none());

        drop(guard1);

        // Now it should succeed again
        let guard3 = permit.try_acquire();
        assert!(guard3.is_some());
    }

    #[tokio::test]
    async fn test_guaranteed_slot_never_starves() {
        let manager = ConcurrencyManager::new(2, 2); // 0 shared slots
        let permit = ModelDownloadPermit::new(
            "test".to_string(),
            manager.shared_pool(),
            1,
            manager.active_downloads_counter(),
        );

        // Take the guaranteed slot
        let guard1 = permit.acquire().await;

        // Second acquire should wait for guaranteed slot
        let acquire_future = permit.acquire();

        // Should timeout since slot is taken
        let result = timeout(Duration::from_millis(50), acquire_future).await;
        assert!(result.is_err(), "Should timeout waiting for slot");

        // Release first slot
        drop(guard1);

        // Now acquire should succeed
        let guard2 = timeout(Duration::from_millis(50), permit.acquire()).await;
        assert!(guard2.is_ok(), "Should acquire after slot released");
    }

    #[tokio::test]
    async fn test_multiple_models_independence() {
        let manager = ConcurrencyManager::new(4, 2); // 2 shared slots

        let permit1 = ModelDownloadPermit::new(
            "model1".to_string(),
            manager.shared_pool(),
            2,
            manager.active_downloads_counter(),
        );

        let permit2 = ModelDownloadPermit::new(
            "model2".to_string(),
            manager.shared_pool(),
            2,
            manager.active_downloads_counter(),
        );

        // Both models can acquire their guaranteed slots simultaneously
        let guard1a = permit1.acquire().await;
        let guard2a = permit2.acquire().await;

        assert!(matches!(guard1a, DownloadSlotGuard::Guaranteed { .. }));
        assert!(matches!(guard2a, DownloadSlotGuard::Guaranteed { .. }));
        assert_eq!(manager.active_downloads(), 2);

        // Both can also acquire from shared pool
        let guard1b = permit1.acquire().await;
        let guard2b = permit2.acquire().await;

        assert!(matches!(guard1b, DownloadSlotGuard::Shared { .. }));
        assert!(matches!(guard2b, DownloadSlotGuard::Shared { .. }));
        assert_eq!(manager.active_downloads(), 4);

        // Shared pool should be exhausted now
        assert_eq!(manager.available_shared_slots(), 0);
    }
}
