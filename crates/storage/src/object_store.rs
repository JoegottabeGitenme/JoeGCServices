//! Object storage interface for grid data (MinIO/S3 compatible).

use bytes::Bytes;
use object_store::{aws::AmazonS3Builder, path::Path, ObjectStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, instrument};

use wms_common::{WmsError, WmsResult};

/// Configuration for object storage connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectStorageConfig {
    /// S3/MinIO endpoint URL
    pub endpoint: String,
    /// Bucket name
    pub bucket: String,
    /// Access key ID
    pub access_key_id: String,
    /// Secret access key
    pub secret_access_key: String,
    /// AWS region (use "us-east-1" for MinIO)
    pub region: String,
    /// Allow HTTP (for local MinIO)
    pub allow_http: bool,
}

impl Default for ObjectStorageConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://minio:9000".to_string(),
            bucket: "weather-data".to_string(),
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
            allow_http: true,
        }
    }
}

/// Keys per bulk-delete request issued by [`ObjectStorage::delete_prefix`].
///
/// S3 allows 1000 keys per `DeleteObjects` call, but RustFS 1.0.3 works through
/// the keys of one call at roughly 25-35 ms each (measured), so a full
/// 1000-key batch takes ~25 s and, under any concurrency, exceeds
/// object_store's 30 s default request timeout. 100 keys keeps every request
/// to a few seconds without costing throughput (the server is the bottleneck,
/// not the request count) and is far above the 7-11 objects of a typical zarr
/// directory, which still go out as a single request.
const DELETE_BATCH_KEYS: usize = 100;

/// Object storage client for weather data.
pub struct ObjectStorage {
    store: Arc<dyn ObjectStore>,
    bucket: String,
}

impl ObjectStorage {
    /// Create a new object storage client from config.
    pub fn new(config: &ObjectStorageConfig) -> WmsResult<Self> {
        let mut builder = AmazonS3Builder::new()
            .with_endpoint(&config.endpoint)
            .with_bucket_name(&config.bucket)
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key)
            .with_region(&config.region);

        if config.allow_http {
            builder = builder.with_allow_http(true);
        }

        let store = builder
            .build()
            .map_err(|e| WmsError::StorageError(format!("Failed to create S3 client: {}", e)))?;

        Ok(Self {
            store: Arc::new(store),
            bucket: config.bucket.clone(),
        })
    }

    /// Wrap an existing [`ObjectStore`] (e.g. `InMemory` in tests).
    pub fn from_store(store: Arc<dyn ObjectStore>, bucket: impl Into<String>) -> Self {
        Self {
            store,
            bucket: bucket.into(),
        }
    }

    /// Write bytes to a path in the bucket.
    #[instrument(skip(self, data), fields(bucket = %self.bucket, path = %path))]
    pub async fn put(&self, path: &str, data: Bytes) -> WmsResult<()> {
        let location = Path::from(path);
        debug!(size = data.len(), "Writing object");

        self.store
            .put(&location, data.into())
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to write {}: {}", path, e)))?;

        Ok(())
    }

    /// Read bytes from a path.
    #[instrument(skip(self), fields(bucket = %self.bucket, path = %path))]
    pub async fn get(&self, path: &str) -> WmsResult<Bytes> {
        let location = Path::from(path);

        let result = self
            .store
            .get(&location)
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to read {}: {}", path, e)))?;

        let bytes = result
            .bytes()
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to read bytes: {}", e)))?;

        debug!(size = bytes.len(), "Read object");
        Ok(bytes)
    }

    /// Read a byte range from a path.
    #[instrument(skip(self), fields(bucket = %self.bucket, path = %path))]
    pub async fn get_range(&self, path: &str, start: usize, end: usize) -> WmsResult<Bytes> {
        let location = Path::from(path);

        let result = self
            .store
            .get_range(&location, start..end)
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to read range {}: {}", path, e)))?;

        Ok(result)
    }

    /// Check if an object exists.
    pub async fn exists(&self, path: &str) -> WmsResult<bool> {
        let location = Path::from(path);

        match self.store.head(&location).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(WmsError::StorageError(format!(
                "Failed to check {}: {}",
                path, e
            ))),
        }
    }

    /// List objects with a given prefix.
    pub async fn list(&self, prefix: &str) -> WmsResult<Vec<String>> {
        use futures::TryStreamExt;

        let prefix_path = Path::from(prefix);
        let mut paths = Vec::new();

        let mut stream = self.store.list(Some(&prefix_path));
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| WmsError::StorageError(format!("List failed: {}", e)))?
        {
            paths.push(meta.location.to_string());
        }

        Ok(paths)
    }

    /// List objects with their last-modified timestamps.
    ///
    /// Used by the sync/orphan-reconciliation task to apply a grace period:
    /// objects written recently must not be treated as orphans because the
    /// corresponding catalog row may not have been registered yet (uploads
    /// happen before registration).
    pub async fn list_with_modified(
        &self,
        prefix: &str,
    ) -> WmsResult<Vec<(String, chrono::DateTime<chrono::Utc>)>> {
        use futures::TryStreamExt;

        let prefix_path = Path::from(prefix);
        let mut results = Vec::new();

        let mut stream = self.store.list(Some(&prefix_path));
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| WmsError::StorageError(format!("List failed: {}", e)))?
        {
            results.push((meta.location.to_string(), meta.last_modified));
        }

        Ok(results)
    }

    /// List objects with metadata (path and size).
    pub async fn list_with_sizes(&self, prefix: &str) -> WmsResult<Vec<(String, u64)>> {
        use futures::TryStreamExt;

        let prefix_path = Path::from(prefix);
        let mut results = Vec::new();

        let mut stream = self.store.list(Some(&prefix_path));
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| WmsError::StorageError(format!("List failed: {}", e)))?
        {
            results.push((meta.location.to_string(), meta.size as u64));
        }

        Ok(results)
    }

    /// Get object metadata (size).
    pub async fn head(&self, path: &str) -> WmsResult<u64> {
        let location = Path::from(path);

        let meta = self.store.head(&location).await.map_err(|e| {
            WmsError::StorageError(format!("Failed to get metadata for {}: {}", path, e))
        })?;

        Ok(meta.size as u64)
    }

    /// Delete an object.
    #[instrument(skip(self), fields(bucket = %self.bucket, path = %path))]
    pub async fn delete(&self, path: &str) -> WmsResult<()> {
        let location = Path::from(path);

        self.store
            .delete(&location)
            .await
            .map_err(|e| WmsError::StorageError(format!("Failed to delete {}: {}", path, e)))?;

        Ok(())
    }

    /// Delete all objects with a given prefix (for Zarr directories).
    /// Returns the number of objects deleted.
    ///
    /// The prefix is listed once and the keys are removed with the bulk API
    /// ([`ObjectStore::delete_stream`], which on S3 sends `DeleteObjects`
    /// requests), [`DELETE_BATCH_KEYS`] keys per request. A zarr directory
    /// (7-11 objects for HRRR/GOES) is therefore one LIST plus one DELETE
    /// request instead of one DELETE per object.
    ///
    /// Every key is attempted even if some fail. If any key could not be
    /// deleted (or the listing failed part-way) this returns an error naming
    /// how many failed and the first few causes, so the caller must treat the
    /// prefix as *not* fully deleted and retry it later.
    ///
    /// An empty prefix is rejected: it would list and delete the whole bucket.
    #[instrument(skip(self), fields(bucket = %self.bucket, prefix = %prefix))]
    pub async fn delete_prefix(&self, prefix: &str) -> WmsResult<u64> {
        use futures::{stream, StreamExt};

        /// How many distinct failure causes to quote in the returned error.
        const MAX_QUOTED_ERRORS: usize = 3;

        let prefix_path = Path::from(prefix);
        if prefix_path.parts().next().is_none() {
            return Err(WmsError::StorageError(format!(
                "Refusing to delete empty prefix {:?}",
                prefix
            )));
        }

        let mut listing = self.store.list(Some(&prefix_path));
        let mut deleted: u64 = 0;
        let mut failed: u64 = 0;
        let mut causes: Vec<String> = Vec::new();
        let mut record_failure = |cause: String| {
            failed += 1;
            if causes.len() < MAX_QUOTED_ERRORS {
                causes.push(cause);
            }
        };

        let mut listing_done = false;
        while !listing_done {
            // Pull the next batch of keys off the (lazy, paginated) listing.
            let mut batch: Vec<Path> = Vec::with_capacity(DELETE_BATCH_KEYS);
            while batch.len() < DELETE_BATCH_KEYS {
                match listing.next().await {
                    Some(Ok(meta)) => batch.push(meta.location),
                    Some(Err(e)) => {
                        // Can't trust the rest of the listing; delete what we
                        // have, then report the prefix as incomplete.
                        record_failure(format!("listing failed: {}", e));
                        listing_done = true;
                        break;
                    }
                    None => {
                        listing_done = true;
                        break;
                    }
                }
            }
            if batch.is_empty() {
                break;
            }

            // At most DELETE_BATCH_KEYS keys: delete_stream sends exactly one
            // DeleteObjects request and reports a result per key.
            let mut results = self
                .store
                .delete_stream(stream::iter(batch.into_iter().map(Ok)).boxed());
            while let Some(result) = results.next().await {
                match result {
                    Ok(_) => deleted += 1,
                    Err(e) => record_failure(e.to_string()),
                }
            }
        }

        if failed > 0 {
            return Err(WmsError::StorageError(format!(
                "Failed to delete {} object(s) under prefix {} ({} deleted): {}",
                failed,
                prefix,
                deleted,
                causes.join("; ")
            )));
        }

        debug!(deleted = deleted, "Deleted objects with prefix");
        Ok(deleted)
    }

    /// Get storage statistics (total size and object count).
    pub async fn stats(&self) -> WmsResult<StorageStats> {
        self.stats_with_prefix(None).await
    }

    /// Get storage statistics for objects matching a prefix.
    pub async fn stats_with_prefix(&self, prefix: Option<&str>) -> WmsResult<StorageStats> {
        use futures::TryStreamExt;

        let mut total_size: u64 = 0;
        let mut object_count: u64 = 0;

        let prefix_path = prefix.map(object_store::path::Path::from);
        let mut stream = self.store.list(prefix_path.as_ref());
        while let Some(meta) = stream
            .try_next()
            .await
            .map_err(|e| WmsError::StorageError(format!("List failed: {}", e)))?
        {
            total_size += meta.size as u64;
            object_count += 1;
        }

        Ok(StorageStats {
            total_size,
            object_count,
            bucket: self.bucket.clone(),
        })
    }

    /// Get detailed storage statistics with breakdown by storage type (raw vs shredded).
    pub async fn detailed_stats(&self) -> WmsResult<DetailedStorageStats> {
        // Query raw/ and shredded/ prefixes in parallel
        let raw_stats = self.stats_with_prefix(Some("raw/")).await?;
        let shredded_stats = self.stats_with_prefix(Some("shredded/")).await?;

        Ok(DetailedStorageStats {
            raw_size_bytes: raw_stats.total_size,
            raw_object_count: raw_stats.object_count,
            shredded_size_bytes: shredded_stats.total_size,
            shredded_object_count: shredded_stats.object_count,
            total_size_bytes: raw_stats.total_size + shredded_stats.total_size,
            total_object_count: raw_stats.object_count + shredded_stats.object_count,
            bucket: self.bucket.clone(),
        })
    }
}

/// Storage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    /// Total size in bytes
    pub total_size: u64,
    /// Number of objects
    pub object_count: u64,
    /// Bucket name
    pub bucket: String,
}

/// Detailed storage statistics with breakdown by storage type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedStorageStats {
    /// Statistics for raw ingested files (raw/ prefix)
    pub raw_size_bytes: u64,
    pub raw_object_count: u64,
    /// Statistics for shredded/processed files (shredded/ prefix)
    pub shredded_size_bytes: u64,
    pub shredded_object_count: u64,
    /// Total across all storage
    pub total_size_bytes: u64,
    pub total_object_count: u64,
    /// Bucket name
    pub bucket: String,
}

/// Path builder for consistent storage layout.
pub struct StoragePath;

impl StoragePath {
    /// Build path for raw ingested files.
    /// Format: raw/{model}/{date}/{cycle}/{filename}
    pub fn raw_file(model: &str, date: &str, cycle: &str, filename: &str) -> String {
        format!("raw/{}/{}/{}/{}", model, date, cycle, filename)
    }

    /// Build path for processed grid chunks.
    /// Format: grids/{model}/{parameter}/{date}/{cycle}/{fhr}/{chunk_id}.bin
    pub fn grid_chunk(
        model: &str,
        parameter: &str,
        date: &str,
        cycle: &str,
        forecast_hour: u32,
        chunk_id: u32,
    ) -> String {
        format!(
            "grids/{}/{}/{}/{}/{:03}/{}.bin",
            model, parameter, date, cycle, forecast_hour, chunk_id
        )
    }

    /// Build path for layer metadata.
    /// Format: meta/{model}/{parameter}/metadata.json
    pub fn layer_metadata(model: &str, parameter: &str) -> String {
        format!("meta/{}/{}/metadata.json", model, parameter)
    }

    /// Build path for pre-rendered tiles.
    /// Format: tiles/{layer}/{style}/{z}/{x}/{y}.png
    pub fn tile(layer: &str, style: &str, z: u32, x: u32, y: u32) -> String {
        format!("tiles/{}/{}/{}/{}/{}.png", layer, style, z, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream::BoxStream;
    use futures::{StreamExt, TryStreamExt};
    use object_store::memory::InMemory;
    use object_store::{
        GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMultipartOpts,
        PutOptions, PutPayload, PutResult,
    };
    use std::ops::Range;

    /// An `InMemory` store that fails `delete` for any key containing `poison`, can fail its
    /// listing after `fail_list_after` entries, and records the size of every bulk delete.
    #[derive(Debug, Default)]
    struct FlakyStore {
        inner: InMemory,
        poison: &'static str,
        fail_list_after: Option<usize>,
        bulk_delete_sizes: std::sync::Mutex<Vec<usize>>,
    }

    impl std::fmt::Display for FlakyStore {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "FlakyStore({})", self.inner)
        }
    }

    #[async_trait]
    impl ObjectStore for FlakyStore {
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

        async fn get_range(
            &self,
            location: &Path,
            range: Range<usize>,
        ) -> object_store::Result<Bytes> {
            self.inner.get_range(location, range).await
        }

        async fn delete(&self, location: &Path) -> object_store::Result<()> {
            if !self.poison.is_empty() && location.as_ref().contains(self.poison) {
                return Err(object_store::Error::Generic {
                    store: "FlakyStore",
                    source: format!("injected delete failure for {}", location).into(),
                });
            }
            self.inner.delete(location).await
        }

        fn delete_stream<'a>(
            &'a self,
            locations: BoxStream<'a, object_store::Result<Path>>,
        ) -> BoxStream<'a, object_store::Result<Path>> {
            futures::stream::once(async move {
                let locations: Vec<_> = locations.collect().await;
                self.bulk_delete_sizes.lock().unwrap().push(locations.len());
                futures::stream::iter(locations).then(move |location| async move {
                    let location = location?;
                    self.delete(&location).await?;
                    Ok(location)
                })
            })
            .flatten()
            .boxed()
        }

        fn list(&self, prefix: Option<&Path>) -> BoxStream<'_, object_store::Result<ObjectMeta>> {
            match self.fail_list_after {
                None => self.inner.list(prefix),
                Some(n) => self
                    .inner
                    .list(prefix)
                    .take(n)
                    .chain(futures::stream::once(async {
                        Err(object_store::Error::Generic {
                            store: "FlakyStore",
                            source: "injected list failure".into(),
                        })
                    }))
                    .boxed(),
            }
        }

        async fn list_with_delimiter(
            &self,
            prefix: Option<&Path>,
        ) -> object_store::Result<ListResult> {
            self.inner.list_with_delimiter(prefix).await
        }

        async fn copy(&self, from: &Path, to: &Path) -> object_store::Result<()> {
            self.inner.copy(from, to).await
        }

        async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> object_store::Result<()> {
            self.inner.copy_if_not_exists(from, to).await
        }
    }

    async fn put_all(storage: &ObjectStorage, keys: &[&str]) {
        for key in keys {
            storage
                .put(key, Bytes::from_static(b"x"))
                .await
                .expect("put into in-memory store");
        }
    }

    async fn all_keys(storage: &ObjectStorage) -> Vec<String> {
        let mut keys = storage.list("").await.expect("list in-memory store");
        keys.sort();
        keys
    }

    fn in_memory() -> ObjectStorage {
        ObjectStorage::from_store(Arc::new(InMemory::new()), "test-bucket")
    }

    #[tokio::test]
    async fn delete_prefix_removes_every_object_under_the_prefix_and_nothing_else() {
        let storage = in_memory();
        let zarr = [
            "grids/hrrr/20261029_12z/t2m_f001.zarr/zarr.json",
            "grids/hrrr/20261029_12z/t2m_f001.zarr/c/0/0",
            "grids/hrrr/20261029_12z/t2m_f001.zarr/c/0/1",
            "grids/hrrr/20261029_12z/t2m_f001.zarr/c/1/0",
            "grids/hrrr/20261029_12z/t2m_f001.zarr/nested/deep/er/chunk",
        ];
        let others = [
            // sibling zarrs, including ones that merely share a string prefix
            "grids/hrrr/20261029_12z/t2m_f002.zarr/zarr.json",
            "grids/hrrr/20261029_12z/t2m_f0010.zarr/zarr.json",
            "grids/hrrr/20261029_12z/t2m_f001.zarr.bak/zarr.json",
            "grids/hrrr/20261029_12z/t2m_f001.zarrx",
            // a different tree entirely
            "raw/hrrr/t2m_f001.zarr/zarr.json",
        ];
        put_all(&storage, &zarr).await;
        put_all(&storage, &others).await;

        // Trailing slash is how retention calls it.
        let deleted = storage
            .delete_prefix("grids/hrrr/20261029_12z/t2m_f001.zarr/")
            .await
            .expect("delete_prefix");

        assert_eq!(deleted, zarr.len() as u64);
        let mut expected: Vec<String> = others.iter().map(|k| k.to_string()).collect();
        expected.sort();
        assert_eq!(all_keys(&storage).await, expected);

        // Without the trailing slash it means the same thing.
        let deleted = storage
            .delete_prefix("grids/hrrr/20261029_12z/t2m_f002.zarr")
            .await
            .expect("delete_prefix");
        assert_eq!(deleted, 1);
        assert_eq!(all_keys(&storage).await.len(), others.len() - 1);
    }

    #[tokio::test]
    async fn delete_prefix_handles_more_objects_than_one_bulk_request() {
        let storage = in_memory();
        let keys: Vec<String> = (0..2_500)
            .map(|i| format!("grids/big.zarr/c/{}/{}", i / 50, i % 50))
            .collect();
        let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        put_all(&storage, &key_refs).await;
        put_all(&storage, &["grids/keep.zarr/zarr.json"]).await;

        let deleted = storage.delete_prefix("grids/big.zarr/").await.unwrap();

        assert_eq!(deleted, 2_500);
        assert_eq!(all_keys(&storage).await, vec!["grids/keep.zarr/zarr.json"]);
    }

    #[tokio::test]
    async fn delete_prefix_of_a_missing_prefix_deletes_nothing_and_succeeds() {
        let storage = in_memory();
        put_all(&storage, &["grids/keep.zarr/zarr.json"]).await;

        assert_eq!(storage.delete_prefix("grids/gone.zarr/").await.unwrap(), 0);
        assert_eq!(all_keys(&storage).await.len(), 1);
    }

    #[tokio::test]
    async fn delete_prefix_refuses_an_empty_prefix() {
        let storage = in_memory();
        put_all(&storage, &["grids/keep.zarr/zarr.json"]).await;

        for prefix in ["", "/", "//"] {
            let err = storage.delete_prefix(prefix).await.unwrap_err();
            assert!(err.to_string().contains("empty prefix"), "{err}");
        }
        assert_eq!(all_keys(&storage).await.len(), 1, "nothing may be deleted");
    }

    #[tokio::test]
    async fn delete_prefix_attempts_every_key_and_reports_the_ones_that_failed() {
        let store = Arc::new(FlakyStore {
            poison: "c/1/",
            ..Default::default()
        });
        let storage = ObjectStorage::from_store(store.clone(), "test-bucket");
        put_all(
            &storage,
            &[
                "grids/a.zarr/zarr.json",
                "grids/a.zarr/c/0/0",
                "grids/a.zarr/c/1/0",
                "grids/a.zarr/c/1/1",
                "grids/a.zarr/c/2/0",
            ],
        )
        .await;

        let err = storage
            .delete_prefix("grids/a.zarr/")
            .await
            .expect_err("per-key failures must surface");

        let msg = err.to_string();
        assert!(msg.contains("2 object(s)"), "{msg}");
        assert!(msg.contains("(3 deleted)"), "{msg}");
        assert!(msg.contains("injected delete failure"), "{msg}");
        // The healthy keys were still deleted; only the poisoned ones remain.
        let remaining: Vec<String> = store
            .list(None)
            .map_ok(|m| m.location.to_string())
            .try_collect()
            .await
            .unwrap();
        let mut remaining = remaining;
        remaining.sort();
        assert_eq!(remaining, vec!["grids/a.zarr/c/1/0", "grids/a.zarr/c/1/1"]);
    }

    #[tokio::test]
    async fn delete_prefix_sends_a_zarr_as_one_request_and_a_huge_prefix_in_bounded_batches() {
        let store = Arc::new(FlakyStore::default());
        let storage = ObjectStorage::from_store(store.clone(), "test-bucket");

        let zarr: Vec<String> = (0..9).map(|i| format!("grids/z.zarr/c/{i}")).collect();
        put_all(
            &storage,
            &zarr.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .await;
        assert_eq!(storage.delete_prefix("grids/z.zarr/").await.unwrap(), 9);
        assert_eq!(*store.bulk_delete_sizes.lock().unwrap(), vec![9]);

        store.bulk_delete_sizes.lock().unwrap().clear();
        let big: Vec<String> = (0..250).map(|i| format!("grids/big.zarr/c/{i}")).collect();
        put_all(
            &storage,
            &big.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .await;
        assert_eq!(storage.delete_prefix("grids/big.zarr/").await.unwrap(), 250);
        assert_eq!(
            *store.bulk_delete_sizes.lock().unwrap(),
            vec![
                DELETE_BATCH_KEYS,
                DELETE_BATCH_KEYS,
                250 - 2 * DELETE_BATCH_KEYS
            ]
        );
        assert!(all_keys(&storage).await.is_empty());
    }

    #[tokio::test]
    async fn delete_prefix_reports_a_listing_that_fails_part_way() {
        let store = Arc::new(FlakyStore {
            fail_list_after: Some(3),
            ..Default::default()
        });
        let storage = ObjectStorage::from_store(store.clone(), "test-bucket");
        put_all(
            &storage,
            &["a.zarr/0", "a.zarr/1", "a.zarr/2", "a.zarr/3", "a.zarr/4"],
        )
        .await;

        let err = storage.delete_prefix("a.zarr/").await.unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("listing failed"), "{msg}");
        assert!(msg.contains("injected list failure"), "{msg}");
        assert!(msg.contains("(3 deleted)"), "{msg}");
        // The prefix is NOT reported as deleted, so the caller keeps its row and retries.
        let remaining = store.inner.list(None).count().await;
        assert_eq!(remaining, 2);
    }

    #[test]
    fn test_storage_paths() {
        assert_eq!(
            StoragePath::raw_file("gfs", "20240115", "12", "gfs.t12z.pgrb2.0p25.f006"),
            "raw/gfs/20240115/12/gfs.t12z.pgrb2.0p25.f006"
        );

        assert_eq!(
            StoragePath::grid_chunk("gfs", "temperature_2m", "20240115", "12", 6, 42),
            "grids/gfs/temperature_2m/20240115/12/006/42.bin"
        );
    }
}
