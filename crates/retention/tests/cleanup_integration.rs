//! `CleanupTask::run_once` against a real Postgres catalog (Docker) and an in-memory object
//! store whose deletes can be made to fail.
//!
//! Run with `cargo test -p retention --test cleanup_integration -- --ignored`.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{Duration, Utc};
use futures::stream::BoxStream;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{
    GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore, PutMultipartOpts,
    PutOptions, PutPayload, PutResult,
};

use retention::{
    CleanupConfig, CleanupTask, ModelRetentionConfig, ModelType, RetentionContext,
    DEFAULT_DELETE_CONCURRENCY,
};
use storage::{Catalog, CatalogEntry, ObjectStorage};
use test_utils::containers::TestInfrastructure;
use wms_common::BoundingBox;

/// An `InMemory` store whose `delete` fails for keys containing "poison" while `armed`.
#[derive(Debug)]
struct PoisonedStore {
    inner: InMemory,
    armed: AtomicBool,
}

impl std::fmt::Display for PoisonedStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PoisonedStore")
    }
}

#[async_trait]
impl ObjectStore for PoisonedStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOpts,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.inner.get_opts(location, options).await
    }

    async fn get_range(&self, location: &Path, range: Range<usize>) -> object_store::Result<Bytes> {
        self.inner.get_range(location, range).await
    }

    async fn delete(&self, location: &Path) -> object_store::Result<()> {
        if self.armed.load(Ordering::SeqCst) && location.as_ref().contains("poison") {
            return Err(object_store::Error::Generic {
                store: "PoisonedStore",
                source: format!("injected delete failure for {}", location).into(),
            });
        }
        self.inner.delete(location).await
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'_, object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy(&self, from: &Path, to: &Path) -> object_store::Result<()> {
        self.inner.copy(from, to).await
    }

    async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> object_store::Result<()> {
        self.inner.copy_if_not_exists(from, to).await
    }
}

fn entry(model: &str, parameter: &str, age_hours: i64, storage_path: &str) -> CatalogEntry {
    CatalogEntry {
        model: model.to_string(),
        parameter: parameter.to_string(),
        level: "surface".to_string(),
        reference_time: Utc::now() - Duration::hours(age_hours),
        forecast_hour: 0,
        forecast_minute: 0,
        bbox: BoundingBox::new(-180.0, -90.0, 180.0, 90.0),
        storage_path: storage_path.to_string(),
        file_size: 1024,
        zarr_metadata: None,
    }
}

fn model_config(model_type: ModelType, hours: u32) -> ModelRetentionConfig {
    ModelRetentionConfig {
        hours,
        model_type,
        keep_latest_runs: 1,
        keep_latest_observations: 1,
        expected_forecast_hours: None,
    }
}

/// Seven objects, like a small zarr directory.
async fn put_zarr(storage: &ObjectStorage, zarr_path: &str) {
    for key in [
        "zarr.json",
        "lat/zarr.json",
        "lon/zarr.json",
        "lat/c/0",
        "lon/c/0",
        "data/zarr.json",
        "data/c/0/0",
    ] {
        storage
            .put(&format!("{}/{}", zarr_path, key), Bytes::from_static(b"x"))
            .await
            .unwrap();
    }
}

async fn objects_under(storage: &ObjectStorage, prefix: &str) -> usize {
    storage.list(prefix).await.unwrap().len()
}

async fn status_of(catalog: &Catalog, storage_path: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM datasets WHERE storage_path = $1")
        .bind(storage_path)
        .fetch_optional(&catalog.pool_clone())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore] // Requires Docker
async fn run_once_deletes_rows_only_for_paths_that_were_deleted_and_survives_a_failing_model() {
    let infra = TestInfrastructure::start().await;
    let catalog = Catalog::connect(&infra.postgres_url()).await.unwrap();
    catalog.migrate().await.unwrap();

    // Marking "broken" fails (as a production DB error for one model would); other models are fine.
    let pool = catalog.pool_clone();
    sqlx::query(
        "CREATE FUNCTION block_broken() RETURNS trigger AS \
         $$ BEGIN RAISE EXCEPTION 'injected marking failure'; END $$ LANGUAGE plpgsql",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER block_broken BEFORE UPDATE ON datasets FOR EACH ROW \
         WHEN (OLD.model = 'broken') EXECUTE FUNCTION block_broken()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let store = Arc::new(PoisonedStore {
        inner: InMemory::new(),
        armed: AtomicBool::new(true),
    });
    let storage = Arc::new(ObjectStorage::from_store(store.clone(), "test-bucket"));

    // alpha: an old run (3 params, one of which cannot be deleted from storage) and a newer
    // protected run. Sorted so "alpha" < "broken" < "viirs" is irrelevant: each is independent.
    let alpha_old = [
        "grids/alpha/old/t2m.zarr",
        "grids/alpha/old/u10.zarr",
        "grids/alpha/old/poison.zarr",
    ];
    let alpha_new = "grids/alpha/new/t2m.zarr";
    for (i, path) in alpha_old.iter().enumerate() {
        catalog
            .register_dataset(&entry("alpha", &format!("p{}", i), 10, path))
            .await
            .unwrap();
    }
    catalog
        .register_dataset(&entry("alpha", "p0", 0, alpha_new))
        .await
        .unwrap();
    // An expired row left behind by an earlier cycle that never finished.
    let leftover = "grids/alpha/leftover/t2m.zarr";
    catalog
        .register_dataset(&entry("alpha", "leftover", 30, leftover))
        .await
        .unwrap();
    sqlx::query("UPDATE datasets SET status = 'expired' WHERE storage_path = $1")
        .bind(leftover)
        .execute(&pool)
        .await
        .unwrap();

    // broken: old data whose marking fails; it must stay available, untouched.
    let broken_old = "grids/broken/old/t2m.zarr";
    catalog
        .register_dataset(&entry("broken", "p0", 10, broken_old))
        .await
        .unwrap();
    catalog
        .register_dataset(&entry("broken", "p0", 0, "grids/broken/new/t2m.zarr"))
        .await
        .unwrap();

    // viirs: static, old, never expires.
    let viirs = "grids/viirs/old/lights.zarr";
    catalog
        .register_dataset(&entry("viirs", "lights", 1000, viirs))
        .await
        .unwrap();

    for path in alpha_old.iter().copied().chain([
        alpha_new,
        leftover,
        broken_old,
        "grids/broken/new/t2m.zarr",
        viirs,
    ]) {
        put_zarr(&storage, path).await;
    }
    // A sibling that merely shares a string prefix with a path being deleted.
    put_zarr(&storage, "grids/alpha/old/t2m.zarr.keep").await;

    let mut model_configs = HashMap::new();
    model_configs.insert("alpha".to_string(), model_config(ModelType::Forecast, 1));
    model_configs.insert("broken".to_string(), model_config(ModelType::Forecast, 1));
    model_configs.insert("viirs".to_string(), model_config(ModelType::Static, 24));
    let config = CleanupConfig {
        model_configs,
        delete_concurrency: 4,
        ..CleanupConfig::default()
    };
    assert_ne!(config.delete_concurrency, DEFAULT_DELETE_CONCURRENCY);
    let task = CleanupTask::new(
        RetentionContext::new(catalog.clone(), storage.clone()),
        config,
    );

    // ---- Cycle 1: "poison.zarr" cannot be deleted from storage ------------------------------
    let stats = task.run_once().await.expect("cycle must complete");

    assert_eq!(stats.mark_errors, 1, "only the broken model failed to mark");
    assert_eq!(stats.marked_expired, 3, "alpha's three old rows");
    assert_eq!(stats.delete_errors, 1, "poison.zarr");
    // t2m, u10, leftover: 3 zarrs x 7 objects
    assert_eq!(stats.files_deleted, 3);
    assert_eq!(stats.objects_deleted, 21);
    assert_eq!(stats.records_deleted, 3);

    // Rows follow storage: gone where the files are gone, kept where the delete failed.
    assert_eq!(status_of(&catalog, "grids/alpha/old/t2m.zarr").await, None);
    assert_eq!(status_of(&catalog, "grids/alpha/old/u10.zarr").await, None);
    assert_eq!(status_of(&catalog, leftover).await, None);
    assert_eq!(
        status_of(&catalog, "grids/alpha/old/poison.zarr")
            .await
            .as_deref(),
        Some("expired"),
        "failed storage delete keeps its row for the next cycle"
    );
    assert_eq!(
        objects_under(&storage, "grids/alpha/old/t2m.zarr/").await,
        0
    );
    assert_eq!(
        objects_under(&storage, "grids/alpha/old/u10.zarr/").await,
        0
    );
    assert_eq!(objects_under(&storage, "grids/alpha/leftover/").await, 0);

    // Everything that must survive does, in the DB and in storage.
    assert_eq!(
        status_of(&catalog, alpha_new).await.as_deref(),
        Some("available")
    );
    assert_eq!(objects_under(&storage, alpha_new).await, 7);
    assert_eq!(
        status_of(&catalog, broken_old).await.as_deref(),
        Some("available")
    );
    assert_eq!(objects_under(&storage, broken_old).await, 7);
    assert_eq!(
        status_of(&catalog, viirs).await.as_deref(),
        Some("available")
    );
    assert_eq!(objects_under(&storage, viirs).await, 7);
    assert_eq!(
        objects_under(&storage, "grids/alpha/old/t2m.zarr.keep/").await,
        7,
        "a path sharing only a string prefix is not touched"
    );
    assert_eq!(catalog.count_expired().await.unwrap(), 1);

    // ---- Cycle 2: storage recovers; the leftover row is retried and finally removed ----------
    store.armed.store(false, Ordering::SeqCst);
    let stats = task.run_once().await.expect("cycle must complete");

    assert_eq!(
        stats.mark_errors, 1,
        "broken still fails to mark, still skipped"
    );
    assert_eq!(stats.marked_expired, 0);
    assert_eq!(stats.delete_errors, 0);
    assert_eq!(stats.files_deleted, 1);
    assert_eq!(stats.records_deleted, 1);
    assert_eq!(
        status_of(&catalog, "grids/alpha/old/poison.zarr").await,
        None
    );
    assert_eq!(
        objects_under(&storage, "grids/alpha/old/poison.zarr/").await,
        0
    );
    assert_eq!(catalog.count_expired().await.unwrap(), 0);
    assert_eq!(
        status_of(&catalog, alpha_new).await.as_deref(),
        Some("available")
    );
    assert_eq!(objects_under(&storage, alpha_new).await, 7);
}
