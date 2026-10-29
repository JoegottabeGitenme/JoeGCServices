//! Benchmark / compatibility check for `ObjectStorage::delete_prefix` against a real RustFS (or any
//! S3-compatible server). Ignored by default.
//!
//! ```text
//! docker run -d --name rustfs-bench -p 19100:9000 \
//!     -e RUSTFS_ACCESS_KEY=bench -e RUSTFS_SECRET_KEY=benchbench rustfs/rustfs:1.0.3
//! curl -X PUT --aws-sigv4 aws:amz:us-east-1:s3 --user bench:benchbench http://127.0.0.1:19100/bench-bucket
//! cargo test -p storage --test rustfs_bench -- --ignored --nocapture
//! ```
//!
//! Environment (all optional): `BENCH_ENDPOINT` (default `http://127.0.0.1:19100`), `BENCH_BUCKET`
//! (`bench-bucket`), `BENCH_ACCESS_KEY` (`bench`), `BENCH_SECRET_KEY` (`benchbench`),
//! `BENCH_PREFIXES` (300 zarr-like prefixes per variant), `BENCH_OBJECTS` (10 objects each),
//! `BENCH_CONCURRENCY` (16, what `CLEANUP_DELETE_CONCURRENCY` defaults to).
//!
//! Seeds three identical sets of `.zarr`-style prefixes plus a `keep/` set, then times deleting them:
//!
//! 1. the OLD `delete_prefix` (list, then one DELETE per object) one prefix at a time, as
//!    `run_once` used to;
//! 2. the NEW bulk `delete_prefix` one prefix at a time (isolates the DeleteObjects effect);
//! 3. the NEW bulk `delete_prefix` with `BENCH_CONCURRENCY` prefixes in flight, as `run_once` now
//!    does.
//!
//! After each variant it asserts every object under that variant's root is gone and that `keep/`
//! is untouched.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::stream::{self, StreamExt, TryStreamExt};
use object_store::aws::AmazonS3Builder;
use object_store::path::Path;
use object_store::ObjectStore;

use storage::{ObjectStorage, ObjectStorageConfig};

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The pre-fix `ObjectStorage::delete_prefix`, verbatim apart from error types.
async fn old_delete_prefix(store: &dyn ObjectStore, prefix: &str) -> object_store::Result<u64> {
    let prefix_path = Path::from(prefix);
    let mut deleted: u64 = 0;
    let objects: Vec<_> = store.list(Some(&prefix_path)).try_collect().await?;
    for meta in objects {
        store.delete(&meta.location).await?;
        deleted += 1;
    }
    Ok(deleted)
}

/// `{root}/hrrr/run{n}/field{i}.zarr`, `n` prefixes.
fn zarr_paths(root: &str, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("{}/hrrr/run{:02}/field{:04}.zarr", root, i / 50, i))
        .collect()
}

async fn seed(
    store: &Arc<dyn ObjectStore>,
    paths: &[String],
    objects_per_path: usize,
    object_bytes: usize,
    concurrency: usize,
) {
    let keys: Vec<Path> = paths
        .iter()
        .flat_map(|p| {
            (0..objects_per_path).map(move |o| {
                if o == 0 {
                    Path::from(format!("{}/zarr.json", p))
                } else {
                    Path::from(format!("{}/c/{}/0", p, o))
                }
            })
        })
        .collect();
    stream::iter(keys)
        .map(|key| {
            let store = Arc::clone(store);
            async move {
                store
                    .put(&key, Bytes::from(vec![7u8; object_bytes]).into())
                    .await
                    .expect("seed put")
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
}

async fn count(store: &Arc<dyn ObjectStore>, prefix: &str) -> usize {
    store
        .list(Some(&Path::from(prefix)))
        .try_collect::<Vec<_>>()
        .await
        .expect("list")
        .len()
}

fn report(label: &str, paths: usize, objects: usize, took: Duration) {
    let secs = took.as_secs_f64();
    println!(
        "{:<44} {:>7.2}s  {:>7.1} prefixes/s  {:>8.1} objects/s  ({:.0} ms/prefix)",
        label,
        secs,
        paths as f64 / secs,
        objects as f64 / secs,
        secs * 1000.0 / paths as f64
    );
}

#[tokio::test]
#[ignore] // Requires a running RustFS/S3 server, see module docs
async fn rustfs_delete_prefix_old_vs_new() {
    let endpoint = env_or("BENCH_ENDPOINT", "http://127.0.0.1:19100");
    let bucket = env_or("BENCH_BUCKET", "bench-bucket");
    let access = env_or("BENCH_ACCESS_KEY", "bench");
    let secret = env_or("BENCH_SECRET_KEY", "benchbench");
    let n_paths = env_usize("BENCH_PREFIXES", 300);
    let n_objects = env_usize("BENCH_OBJECTS", 10);
    let concurrency = env_usize("BENCH_CONCURRENCY", 16);
    let total_objects = n_paths * n_objects;

    // Raw store for seeding / verification / the old algorithm; ObjectStorage is the code under test.
    let raw: Arc<dyn ObjectStore> = Arc::new(
        AmazonS3Builder::new()
            .with_endpoint(&endpoint)
            .with_bucket_name(&bucket)
            .with_access_key_id(&access)
            .with_secret_access_key(&secret)
            .with_region("us-east-1")
            .with_allow_http(true)
            .build()
            .expect("s3 client"),
    );
    let storage = Arc::new(
        ObjectStorage::new(&ObjectStorageConfig {
            endpoint,
            bucket,
            access_key_id: access,
            secret_access_key: secret,
            region: "us-east-1".to_string(),
            allow_http: true,
        })
        .expect("object storage"),
    );

    // A unique run id so reruns never collide with leftovers.
    let run = format!(
        "bench{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let old_paths = zarr_paths(&format!("{run}/old-sequential"), n_paths);
    let bulk_seq_paths = zarr_paths(&format!("{run}/bulk-sequential"), n_paths);
    let bulk_conc_paths = zarr_paths(&format!("{run}/bulk-concurrent"), n_paths);
    let keep_paths = zarr_paths(&format!("{run}/keep"), 20.min(n_paths));

    let seeding = Instant::now();
    for paths in [&old_paths, &bulk_seq_paths, &bulk_conc_paths, &keep_paths] {
        seed(&raw, paths, n_objects, 256, 64).await;
    }
    println!(
        "\nseeded {} prefixes x {} objects x 3 variants (+{} keep prefixes) in {:.1}s",
        n_paths,
        n_objects,
        keep_paths.len(),
        seeding.elapsed().as_secs_f64()
    );
    for (name, root) in [
        ("old-sequential", format!("{run}/old-sequential")),
        ("bulk-sequential", format!("{run}/bulk-sequential")),
        ("bulk-concurrent", format!("{run}/bulk-concurrent")),
    ] {
        assert_eq!(count(&raw, &root).await, total_objects, "{name} seeded");
    }
    let keep_expected = keep_paths.len() * n_objects;
    assert_eq!(count(&raw, &format!("{run}/keep")).await, keep_expected);

    println!("\n--- deleting {n_paths} prefixes x {n_objects} objects = {total_objects} objects per variant ---");

    // 1. OLD: sequential list + per-object DELETE, one prefix at a time.
    let started = Instant::now();
    let mut deleted = 0;
    for path in &old_paths {
        deleted += old_delete_prefix(raw.as_ref(), &format!("{path}/"))
            .await
            .expect("old delete_prefix");
    }
    let old_took = started.elapsed();
    assert_eq!(deleted as usize, total_objects);
    assert_eq!(count(&raw, &format!("{run}/old-sequential")).await, 0);
    report(
        "OLD  list + per-object DELETE, sequential",
        n_paths,
        total_objects,
        old_took,
    );

    // 2. NEW bulk delete_prefix, one prefix at a time.
    let started = Instant::now();
    let mut deleted = 0;
    for path in &bulk_seq_paths {
        deleted += storage
            .delete_prefix(&format!("{path}/"))
            .await
            .expect("new delete_prefix");
    }
    let seq_took = started.elapsed();
    assert_eq!(deleted as usize, total_objects);
    assert_eq!(
        count(&raw, &format!("{run}/bulk-sequential")).await,
        0,
        "DeleteObjects must remove every object"
    );
    report(
        "NEW  DeleteObjects delete_prefix, sequential",
        n_paths,
        total_objects,
        seq_took,
    );

    // 3. NEW bulk delete_prefix, `concurrency` prefixes in flight (what run_once does).
    let started = Instant::now();
    let deleted: u64 = stream::iter(bulk_conc_paths.iter())
        .map(|path| {
            let storage = Arc::clone(&storage);
            async move {
                storage
                    .delete_prefix(&format!("{path}/"))
                    .await
                    .expect("new delete_prefix")
            }
        })
        .buffer_unordered(concurrency)
        .fold(0u64, |acc, n| async move { acc + n })
        .await;
    let conc_took = started.elapsed();
    assert_eq!(deleted as usize, total_objects);
    assert_eq!(
        count(&raw, &format!("{run}/bulk-concurrent")).await,
        0,
        "DeleteObjects must remove every object"
    );
    report(
        &format!("NEW  DeleteObjects delete_prefix, x{concurrency} concurrent"),
        n_paths,
        total_objects,
        conc_took,
    );

    println!(
        "\nspeedup vs OLD: bulk sequential {:.1}x, bulk x{} concurrent {:.1}x",
        old_took.as_secs_f64() / seq_took.as_secs_f64(),
        concurrency,
        old_took.as_secs_f64() / conc_took.as_secs_f64()
    );

    // Nothing outside the deleted prefixes was touched.
    assert_eq!(
        count(&raw, &format!("{run}/keep")).await,
        keep_expected,
        "keep/ must be untouched"
    );

    // DeleteObjects edge cases on this server: keys that do not exist must not be reported as
    // failures, and a prefix larger than one 100-key batch must delete everything.
    let missing: Vec<Path> = (0..5)
        .map(|i| Path::from(format!("{run}/never-existed/{i}")))
        .collect();
    let results: Vec<_> = raw
        .delete_stream(stream::iter(missing).map(Ok).boxed())
        .collect()
        .await;
    assert_eq!(results.len(), 5);
    assert!(
        results.iter().all(|r| r.is_ok()),
        "missing keys must not fail DeleteObjects: {results:?}"
    );
    let big: Vec<String> = (0..450)
        .map(|i| format!("{run}/big.zarr/c/{}/{}", i / 50, i % 50))
        .collect();
    stream::iter(big)
        .map(|key| {
            let raw = Arc::clone(&raw);
            async move {
                raw.put(&Path::from(key), Bytes::from_static(b"x").into())
                    .await
                    .unwrap()
            }
        })
        .buffer_unordered(64)
        .collect::<Vec<_>>()
        .await;
    assert_eq!(
        storage
            .delete_prefix(&format!("{run}/big.zarr/"))
            .await
            .unwrap(),
        450
    );
    assert_eq!(count(&raw, &format!("{run}/big.zarr")).await, 0);
    println!("DeleteObjects: missing keys OK, 450-key (5 batches) prefix OK");

    // Clean up the keep set.
    for path in &keep_paths {
        storage.delete_prefix(&format!("{path}/")).await.unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// Same small-object workload against different S3 servers (RustFS vs MinIO), for per-object costs.
// ---------------------------------------------------------------------------------------------

/// `(writes completed, sectors written)` of block device `dev` from /proc/diskstats.
fn disk_writes(dev: &str) -> Option<(u64, u64)> {
    let stats = std::fs::read_to_string("/proc/diskstats").ok()?;
    for line in stats.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.get(2) == Some(&dev) && f.len() > 9 {
            return Some((f[7].parse().ok()?, f[9].parse().ok()?));
        }
    }
    None
}

/// Cumulative CPU time (microseconds) of a container cgroup, from its `cpu.stat`.
fn cgroup_cpu_usec(cpu_stat: &str) -> Option<u64> {
    std::fs::read_to_string(cpu_stat)
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("usage_usec ")?.trim().parse().ok())
}

struct Probe {
    at: Instant,
    disk: Option<(u64, u64)>,
    cpu_usec: Option<u64>,
}

/// Wall time, objects/s, server CPU and host disk writes between two probes.
fn step_report(server: &str, step: &str, objects: usize, start: &Probe, end: &Probe) {
    let secs = end.at.duration_since(start.at).as_secs_f64();
    let cpu = match (start.cpu_usec, end.cpu_usec) {
        (Some(a), Some(b)) => format!("{:>6.0}% cpu", (b - a) as f64 / 1e6 / secs * 100.0),
        _ => "   n/a cpu".to_string(),
    };
    let disk = match (start.disk, end.disk) {
        (Some(a), Some(b)) => format!(
            "{:>7.0} write IOPS {:>6.1} MB/s written",
            (b.0 - a.0) as f64 / secs,
            (b.1 - a.1) as f64 * 512.0 / 1e6 / secs
        ),
        _ => "n/a disk".to_string(),
    };
    println!(
        "RESULT {:<8} {:<34} {:>7.2}s {:>8.1} objects/s | {} | {}",
        server,
        step,
        secs,
        objects as f64 / secs,
        cpu,
        disk
    );
}

/// 300 zarr-like prefixes x 10 ~16 KB objects: PUT (8 in flight), then OLD sequential delete of
/// the first half and NEW `delete_prefix` x16 concurrent of the second half. Run it once per
/// server, each on fresh storage; `BENCH_SERVER` only labels the output, `BENCH_DISK_DEV`
/// (e.g. `sdb`) and `BENCH_CPU_STAT` (the container's cgroup `cpu.stat`) enable the IOPS / CPU
/// columns.
#[tokio::test]
#[ignore] // Requires a running S3 server, see module docs
async fn s3_server_small_object_workload() {
    let server = env_or("BENCH_SERVER", "s3");
    let endpoint = env_or("BENCH_ENDPOINT", "http://127.0.0.1:19100");
    let bucket = env_or("BENCH_BUCKET", "bench-bucket");
    let access = env_or("BENCH_ACCESS_KEY", "bench");
    let secret = env_or("BENCH_SECRET_KEY", "benchbench");
    let n_paths = env_usize("BENCH_PREFIXES", 300);
    let n_objects = env_usize("BENCH_OBJECTS", 10);
    let object_bytes = env_usize("BENCH_OBJECT_BYTES", 16 * 1024);
    let upload_concurrency = env_usize("BENCH_UPLOAD_CONCURRENCY", 8);
    let concurrency = env_usize("BENCH_CONCURRENCY", 16);
    let disk_dev = std::env::var("BENCH_DISK_DEV").ok();
    let cpu_stat = std::env::var("BENCH_CPU_STAT").ok();

    let raw: Arc<dyn ObjectStore> = Arc::new(
        AmazonS3Builder::new()
            .with_endpoint(&endpoint)
            .with_bucket_name(&bucket)
            .with_access_key_id(&access)
            .with_secret_access_key(&secret)
            .with_region("us-east-1")
            .with_allow_http(true)
            .build()
            .expect("s3 client"),
    );
    let storage = Arc::new(
        ObjectStorage::new(&ObjectStorageConfig {
            endpoint,
            bucket,
            access_key_id: access,
            secret_access_key: secret,
            region: "us-east-1".to_string(),
            allow_http: true,
        })
        .expect("object storage"),
    );
    let probe = || Probe {
        at: Instant::now(),
        disk: disk_dev.as_deref().and_then(disk_writes),
        cpu_usec: cpu_stat.as_deref().and_then(cgroup_cpu_usec),
    };

    let all = zarr_paths("cmp", n_paths);
    let (old_half, new_half) = all.split_at(n_paths / 2);
    let half_objects = old_half.len() * n_objects;
    let total_objects = n_paths * n_objects;

    println!(
        "\n[{server}] {n_paths} prefixes x {n_objects} objects x {object_bytes} B, PUT concurrency \
         {upload_concurrency}, NEW delete concurrency {concurrency}"
    );

    // 1. PUT everything.
    let (a, ()) = (
        probe(),
        seed(&raw, &all, n_objects, object_bytes, upload_concurrency).await,
    );
    let b = probe();
    step_report(&server, "1 PUT all", total_objects, &a, &b);
    assert_eq!(count(&raw, "cmp").await, total_objects);

    // 2. OLD delete on the first half.
    let a = probe();
    let mut deleted = 0;
    for path in old_half {
        deleted += old_delete_prefix(raw.as_ref(), &format!("{path}/"))
            .await
            .expect("old delete_prefix");
    }
    let b = probe();
    assert_eq!(deleted as usize, half_objects);
    step_report(
        &server,
        "2 OLD list + per-object DELETE",
        half_objects,
        &a,
        &b,
    );
    assert_eq!(count(&raw, "cmp").await, total_objects - half_objects);

    // 3. NEW delete on the other half.
    let a = probe();
    let deleted: u64 = stream::iter(new_half.iter())
        .map(|path| {
            let storage = Arc::clone(&storage);
            async move {
                storage
                    .delete_prefix(&format!("{path}/"))
                    .await
                    .expect("new delete_prefix")
            }
        })
        .buffer_unordered(concurrency)
        .fold(0u64, |acc, n| async move { acc + n })
        .await;
    let b = probe();
    assert_eq!(deleted as usize, half_objects);
    step_report(
        &server,
        &format!("3 NEW delete_prefix x{concurrency}"),
        half_objects,
        &a,
        &b,
    );
    assert_eq!(count(&raw, "cmp").await, 0, "everything deleted");
}
