//! A single-value, stale-while-revalidate cache for an expensive response body.
//!
//! Built for `GET /edr/collections`, which costs ~575 database queries per request
//! (3 of them `COUNT(*)` over millions of observation rows) and took 60-90 s on
//! production. Nothing about that listing needs to be computed per request, but it
//! is only called by a few internal dashboards, so a fixed-interval refresh timer
//! would burn database time all day for almost no readers. Instead:
//!
//! - **Warm at startup** ([`SnapshotCache::refresh`]), so the first real request is fast.
//! - **Serve instantly** whatever is cached, however old.
//! - If the cached value is older than `ttl`, **kick off one background refresh**
//!   (single-flight: concurrent requests never start a second) and still serve the
//!   stale value. No request ever waits on the expensive build once a value exists.
//! - **Cold** (nothing cached, e.g. startup warm-up not finished): requests wait on
//!   one shared build rather than each starting their own.
//! - A **failed or panicking** refresh keeps the previous value and releases the
//!   single-flight flag, so one bad build can neither blank the endpoint nor wedge
//!   it into never refreshing again.
//! - [`SnapshotCache::invalidate`] drops the value (used when config reloads, since
//!   the cached body was built from the old config).

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};

struct Entry {
    body: Arc<String>,
    built_at: Instant,
}

/// What a lookup returned and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Served {
    /// Fresh enough; served from cache.
    Fresh,
    /// Served from cache, but older than the TTL: a background refresh was started
    /// (or one was already running).
    Stale,
    /// Nothing was cached; this request waited for a build.
    Built,
}

pub struct SnapshotCache {
    ttl: Duration,
    entry: RwLock<Option<Entry>>,
    /// Held for the duration of a build so cold callers share one.
    build_lock: Mutex<()>,
    /// True while a background refresh is running (single-flight).
    refreshing: AtomicBool,
}

impl SnapshotCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entry: RwLock::new(None),
            build_lock: Mutex::new(()),
            refreshing: AtomicBool::new(false),
        }
    }

    /// Age of the cached value, if any.
    pub async fn age(&self) -> Option<Duration> {
        self.entry
            .read()
            .await
            .as_ref()
            .map(|e| e.built_at.elapsed())
    }

    /// Drop the cached value; the next request rebuilds.
    pub async fn invalidate(&self) {
        *self.entry.write().await = None;
    }

    /// Build and store a value now (startup warm-up). Returns whether it succeeded.
    pub async fn refresh<F, Fut>(&self, build: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Option<String>>,
    {
        let _guard = self.build_lock.lock().await;
        match build().await {
            Some(body) => {
                *self.entry.write().await = Some(Entry {
                    body: Arc::new(body),
                    built_at: Instant::now(),
                });
                true
            }
            None => false,
        }
    }

    /// The cached body, refreshing as described in the module docs. `build` returns
    /// `None` on failure. Returns `None` only if nothing is cached AND the build
    /// failed.
    pub async fn get<F, Fut>(self: &Arc<Self>, build: F) -> Option<(Arc<String>, Served)>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Option<String>> + Send + 'static,
    {
        // Fast path.
        // Copy what we need and release the read lock BEFORE spawning, so the
        // refresh task can take the write lock without waiting on us.
        let cached = self
            .entry
            .read()
            .await
            .as_ref()
            .map(|e| (Arc::clone(&e.body), e.built_at.elapsed() <= self.ttl));
        if let Some((body, fresh)) = cached {
            if fresh {
                return Some((body, Served::Fresh));
            }
            self.spawn_refresh(build);
            return Some((body, Served::Stale));
        }

        // Cold: share one build among all callers.
        let _guard = self.build_lock.lock().await;
        if let Some(e) = self.entry.read().await.as_ref() {
            // Someone else finished while we waited.
            return Some((Arc::clone(&e.body), Served::Fresh));
        }
        let body = build().await?;
        let body = Arc::new(body);
        *self.entry.write().await = Some(Entry {
            body: Arc::clone(&body),
            built_at: Instant::now(),
        });
        Some((body, Served::Built))
    }

    fn spawn_refresh<F, Fut>(self: &Arc<Self>, build: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Option<String>> + Send + 'static,
    {
        // Single-flight.
        if self
            .refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            // The flag is cleared by this guard's Drop, so it is released even if
            // `build` panics (the task unwinds) -- otherwise one panic would stop
            // the cache ever refreshing again.
            struct Release(Arc<SnapshotCache>);
            impl Drop for Release {
                fn drop(&mut self) {
                    self.0.refreshing.store(false, Ordering::Release);
                }
            }
            let _release = Release(Arc::clone(&this));
            if let Some(body) = build().await {
                *this.entry.write().await = Some(Entry {
                    body: Arc::new(body),
                    built_at: Instant::now(),
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn cache(ttl_ms: u64) -> Arc<SnapshotCache> {
        Arc::new(SnapshotCache::new(Duration::from_millis(ttl_ms)))
    }

    /// A builder that counts invocations and returns "v<n>".
    fn counting(
        counter: &Arc<AtomicUsize>,
    ) -> impl FnOnce() -> std::future::Ready<Option<String>> + Send + 'static {
        let c = Arc::clone(counter);
        move || {
            let n = c.fetch_add(1, Ordering::SeqCst) + 1;
            std::future::ready(Some(format!("v{n}")))
        }
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(60)).await;
    }

    #[tokio::test]
    async fn the_first_call_builds_and_later_calls_within_the_ttl_do_not() {
        let c = cache(60_000);
        let n = Arc::new(AtomicUsize::new(0));
        let (b, how) = c.get(counting(&n)).await.unwrap();
        assert_eq!((b.as_str(), how), ("v1", Served::Built));
        for _ in 0..5 {
            let (b, how) = c.get(counting(&n)).await.unwrap();
            assert_eq!((b.as_str(), how), ("v1", Served::Fresh));
        }
        assert_eq!(n.load(Ordering::SeqCst), 1, "only one build");
    }

    #[tokio::test]
    async fn a_stale_value_is_served_immediately_and_refreshed_in_the_background() {
        // TTL must comfortably exceed `settle()` (60 ms), or the refreshed value is
        // already stale again by the time we read it.
        let c = cache(200);
        let n = Arc::new(AtomicUsize::new(0));
        c.get(counting(&n)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(260)).await; // now stale

        let (b, how) = c.get(counting(&n)).await.unwrap();
        assert_eq!(
            (b.as_str(), how),
            ("v1", Served::Stale),
            "the OLD value is returned, no waiting"
        );
        settle().await;
        let (b, how) = c.get(counting(&n)).await.unwrap();
        assert_eq!(
            (b.as_str(), how),
            ("v2", Served::Fresh),
            "the refresh landed"
        );
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_stale_request_never_waits_for_a_slow_build() {
        let c = cache(10);
        c.get(|| std::future::ready(Some("old".to_string())))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;

        let started = Instant::now();
        let (b, how) = c
            .get(|| async {
                tokio::time::sleep(Duration::from_millis(800)).await; // an expensive rebuild
                Some("new".to_string())
            })
            .await
            .unwrap();
        assert_eq!((b.as_str(), how), ("old", Served::Stale));
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "request waited {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn concurrent_stale_requests_start_exactly_one_refresh() {
        let c = cache(10);
        let n = Arc::new(AtomicUsize::new(0));
        c.get(counting(&n)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;

        let slow_calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..50 {
            let c = Arc::clone(&c);
            let sc = Arc::clone(&slow_calls);
            tasks.push(tokio::spawn(async move {
                c.get(move || async move {
                    sc.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Some("new".to_string())
                })
                .await
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap().unwrap().1, Served::Stale);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(
            slow_calls.load(Ordering::SeqCst),
            1,
            "single-flight: 50 requests, ONE rebuild"
        );
    }

    #[tokio::test]
    async fn concurrent_cold_requests_share_one_build() {
        let c = cache(60_000);
        let builds = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let c = Arc::clone(&c);
            let b = Arc::clone(&builds);
            tasks.push(tokio::spawn(async move {
                c.get(move || async move {
                    b.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    Some("built".to_string())
                })
                .await
                .unwrap()
                .0
            }));
        }
        for t in tasks {
            assert_eq!(t.await.unwrap().as_str(), "built");
        }
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "20 cold requests, ONE build"
        );
    }

    #[tokio::test]
    async fn a_failed_refresh_keeps_serving_the_previous_value() {
        let c = cache(10);
        c.get(|| std::future::ready(Some("good".to_string())))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        c.get(|| std::future::ready(None::<String>)).await.unwrap(); // refresh fails
        settle().await;
        let (b, _) = c
            .get(|| std::future::ready(Some("never".to_string())))
            .await
            .unwrap();
        assert_eq!(
            b.as_str(),
            "good",
            "a failed rebuild must not blank the endpoint"
        );
    }

    #[tokio::test]
    async fn a_cold_failed_build_returns_none_and_a_later_call_can_succeed() {
        let c = cache(60_000);
        assert!(c.get(|| std::future::ready(None::<String>)).await.is_none());
        let (b, how) = c
            .get(|| std::future::ready(Some("ok".to_string())))
            .await
            .unwrap();
        assert_eq!((b.as_str(), how), ("ok", Served::Built));
    }

    #[tokio::test]
    async fn a_panicking_refresh_does_not_wedge_future_refreshes() {
        let c = cache(10);
        c.get(|| std::future::ready(Some("v1".to_string())))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        c.get(|| async { panic!("boom") }).await.unwrap(); // background refresh panics
        settle().await;
        tokio::time::sleep(Duration::from_millis(30)).await; // stale again
        c.get(|| std::future::ready(Some("v2".to_string())))
            .await
            .unwrap(); // must be allowed to start
        settle().await;
        assert_eq!(
            c.get(|| std::future::ready(None::<String>))
                .await
                .unwrap()
                .0
                .as_str(),
            "v2",
            "the single-flight flag was released after the panic"
        );
    }

    #[tokio::test]
    async fn invalidate_forces_the_next_call_to_rebuild() {
        let c = cache(60_000);
        let n = Arc::new(AtomicUsize::new(0));
        c.get(counting(&n)).await.unwrap();
        c.invalidate().await;
        assert!(c.age().await.is_none());
        let (b, how) = c.get(counting(&n)).await.unwrap();
        assert_eq!((b.as_str(), how), ("v2", Served::Built));
    }

    #[tokio::test]
    async fn refresh_warms_the_cache_so_the_first_request_is_fresh() {
        let c = cache(60_000);
        assert!(
            c.refresh(|| std::future::ready(Some("warm".to_string())))
                .await
        );
        let (b, how) = c
            .get(|| std::future::ready(Some("unused".to_string())))
            .await
            .unwrap();
        assert_eq!((b.as_str(), how), ("warm", Served::Fresh));
        assert!(
            !c.refresh(|| std::future::ready(None::<String>)).await,
            "a failed warm-up reports failure"
        );
        assert_eq!(
            c.get(|| std::future::ready(None::<String>))
                .await
                .unwrap()
                .0
                .as_str(),
            "warm",
            "...and keeps the old value"
        );
    }
}
