//! Real-Postgres/PostGIS tests for `storage::lightning`.
//!
//! These exercise behaviour that SQL-text assertions cannot: array binds with
//! NULLs, the dedup key, writer ordering under concurrency, and spatial
//! correctness. `#[ignore]`d like the other integration tests (CI runs them with
//! `-- --ignored`, using Docker via `test_utils`).
//!
//! To run against a local database instead of Docker containers:
//!
//! ```text
//! docker run -d --name pg-glm -p 15434:5432 -e POSTGRES_PASSWORD=t postgis/postgis:16-3.4
//! LIGHTNING_TEST_DATABASE_URL=postgres://postgres:t@localhost:15434/postgres \
//!   cargo test -p storage --test lightning -- --ignored --test-threads=1
//! ```

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::PgPool;
use storage::{Catalog, FlashArea, FlashQuery, LightningCatalog, NewFlash};

/// Keeps the Docker infrastructure alive for the duration of a test.
struct Env {
    catalog: Catalog,
    _infra: Option<test_utils::containers::TestInfrastructure>,
}

async fn env() -> Env {
    let (url, infra) = match std::env::var("LIGHTNING_TEST_DATABASE_URL") {
        Ok(url) => (url, None),
        Err(_) => {
            let infra = test_utils::containers::TestInfrastructure::start().await;
            (infra.postgres_url(), Some(infra))
        }
    };
    let catalog = Catalog::connect(&url).await.expect("connect");
    catalog.migrate().await.expect("migrate");
    catalog
        .migrate_observations()
        .await
        .expect("migrate_observations (enables PostGIS)");
    catalog
        .migrate_lightning()
        .await
        .expect("migrate_lightning");
    sqlx::query("TRUNCATE lightning_flashes, lightning_ingest_progress RESTART IDENTITY")
        .execute(catalog.pool())
        .await
        .expect("truncate");
    Env {
        catalog,
        _infra: infra,
    }
}

fn lc(pool: &PgPool) -> LightningCatalog {
    LightningCatalog::new(pool.clone())
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 20, 0, 0).unwrap()
}

fn flash(sat: &str, secs: i64, id: i32, lon: f64, lat: f64) -> NewFlash {
    NewFlash {
        satellite: sat.to_string(),
        flash_time: t0() + Duration::seconds(secs),
        flash_id: id,
        lon,
        lat,
        energy_j: Some(4.7e-14),
        quality: 0,
    }
}

fn query(area: FlashArea) -> FlashQuery {
    FlashQuery {
        satellites: vec![],
        since: t0() - Duration::hours(1),
        until: None,
        after_id: None,
        area,
        limit: 10_000,
    }
}

#[tokio::test]
#[ignore]
async fn insert_round_trips_every_field_including_null_energy() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    let mut a = flash("goes-east", 1, 7, -105.25, 40.01);
    a.energy_j = None;
    a.quality = 3;
    let b = flash("goes-west", 2, 8, -104.5, 39.5);

    let ids = c.insert_flashes(&[a.clone(), b.clone()]).await.unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids[0] < ids[1], "ids come back ascending");

    let got = c.query_flashes(&query(FlashArea::Anywhere)).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(
        (got[0].satellite.as_str(), got[0].flash_id, got[0].quality),
        ("goes-east", 7, 3)
    );
    assert_eq!(
        got[0].energy_j, None,
        "NULL energy must survive the array bind"
    );
    assert_eq!(got[0].flash_time, a.flash_time);
    assert!((got[0].lon - -105.25).abs() < 1e-9 && (got[0].lat - 40.01).abs() < 1e-9);
    assert_eq!(got[1].energy_j, Some(4.7e-14_f32));
}

#[tokio::test]
#[ignore]
async fn reingesting_a_granule_is_a_no_op() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    let batch: Vec<NewFlash> = (0..20)
        .map(|i| flash("goes-east", i, 100 + i as i32, -105.0, 40.0))
        .collect();
    assert_eq!(c.insert_flashes(&batch).await.unwrap().len(), 20);
    assert!(
        c.insert_flashes(&batch).await.unwrap().is_empty(),
        "second insert must add nothing"
    );
    assert_eq!(c.count().await.unwrap(), 20);
}

#[tokio::test]
#[ignore]
async fn dedup_key_is_satellite_time_and_id_together() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    // flash_id is a rolling u16 counter: the SAME id legitimately recurs ~25 min later.
    // Same id + different time  -> two flashes.
    // Same id + same time, different satellite -> two flashes (E and W both saw it).
    // Same everything -> one flash.
    let rows = [
        flash("goes-east", 0, 5000, -100.0, 40.0),
        flash("goes-east", 1500, 5000, -100.0, 40.0),
        flash("goes-west", 0, 5000, -100.0, 40.0),
        flash("goes-east", 0, 5000, -100.0, 40.0),
    ];
    let ids = c.insert_flashes(&rows).await.unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(c.count().await.unwrap(), 3);
}

#[tokio::test]
#[ignore]
async fn bbox_is_an_exact_lon_lat_rectangle_not_a_great_circle_polygon() {
    // This is the property that motivated geometry over geography. On a wide
    // bbox a geography envelope's northern edge is a great-circle arc that bulges
    // poleward of the 50N parallel (by ~1 degree at the middle of 59 degrees of
    // longitude), so a point at 50.3N would wrongly count as inside. A map
    // viewport is a lon/lat rectangle: 50.3N is outside it.
    let e = env().await;
    let c = lc(e.catalog.pool());
    c.insert_flashes(&[
        flash("goes-east", 0, 1, -95.5, 50.3), // just north of the 50N edge, mid-box
        flash("goes-east", 1, 2, -95.5, 49.9), // inside
        flash("goes-east", 2, 3, -125.0, 24.0), // exactly on the SW corner (inclusive)
        flash("goes-east", 3, 4, -66.0, 50.0), // exactly on the NE corner (inclusive)
        flash("goes-east", 4, 5, -66.01, 24.0 - 0.01), // just outside to the south
    ])
    .await
    .unwrap();
    let area = FlashArea::Bbox {
        min_lon: -125.0,
        min_lat: 24.0,
        max_lon: -66.0,
        max_lat: 50.0,
    };
    let got: Vec<i32> = c
        .query_flashes(&query(area))
        .await
        .unwrap()
        .iter()
        .map(|f| f.flash_id)
        .collect();
    assert_eq!(got, vec![2, 3, 4]);
}

#[tokio::test]
#[ignore]
async fn radius_prefilter_never_drops_a_true_match() {
    // The planar prefilter box is an optimisation and must be lossless: for any
    // centre/radius the result must equal the exact geodesic test with NO
    // prefilter at all. Checked against PostGIS itself on random points from the
    // equator-ish to near the northern edge of CONUS, at radii from 1 km to 600 km.
    let e = env().await;
    let pool = e.catalog.pool().clone();
    let c = lc(&pool);

    // Deterministic pseudo-random points (no rand dependency).
    let mut state: u64 = 0x2545F4914F6CDD1D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let rows: Vec<NewFlash> = (0..4000)
        .map(|i| {
            flash(
                "goes-east",
                i,
                (i % 60000) as i32,
                -125.0 + 59.0 * next(),
                24.0 + 26.0 * next(),
            )
        })
        .collect();
    c.insert_flashes(&rows).await.unwrap();

    let mut checked = 0;
    let mut nonempty = 0;
    for &(lon, lat, meters) in &[
        (-105.2, 39.7, 1_000.0),
        (-105.2, 39.7, 50_000.0),
        (-105.2, 39.7, 300_000.0),
        (-95.0, 49.9, 100_000.0), // high latitude: a degree of longitude is short
        (-95.0, 49.9, 600_000.0),
        (-120.0, 25.0, 600_000.0),
        (-66.5, 45.0, 250_000.0),
        (-110.0, 30.0, 5_000.0),
    ] {
        let got: Vec<i64> = c
            .query_flashes(&query(FlashArea::Radius { lon, lat, meters }))
            .await
            .unwrap()
            .iter()
            .map(|f| f.id)
            .collect();
        let truth: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM lightning_flashes \
             WHERE ST_DWithin(location::geography, ST_SetSRID(ST_MakePoint($1,$2),4326)::geography, $3) ORDER BY id",
        )
        .bind(lon)
        .bind(lat)
        .bind(meters)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            got, truth,
            "prefilter changed the result for ({lon},{lat}) r={meters}"
        );
        checked += 1;
        nonempty += (!truth.is_empty()) as usize;
    }
    assert_eq!(checked, 8);
    assert!(
        nonempty >= 6,
        "the comparison must not be vacuous (only {nonempty} non-empty)"
    );
}

#[tokio::test]
#[ignore]
async fn cursor_paging_returns_everything_once_in_order() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    for batch in 0..3 {
        let rows: Vec<NewFlash> = (0..9)
            .map(|i| {
                flash(
                    "goes-east",
                    batch * 100 + i,
                    (batch * 100 + i) as i32,
                    -105.0,
                    40.0,
                )
            })
            .collect();
        c.insert_flashes(&rows).await.unwrap();
    }
    let mut seen = Vec::new();
    let mut cursor = Some(0i64);
    let mut pages = 0;
    loop {
        let mut q = query(FlashArea::Anywhere);
        q.after_id = cursor;
        q.limit = 10;
        let page = c.query_flashes(&q).await.unwrap();
        pages += 1;
        assert!(pages < 10, "paging did not terminate");
        if page.is_empty() {
            break;
        }
        assert!(
            page.windows(2).all(|w| w[0].id < w[1].id),
            "page is ascending"
        );
        cursor = Some(page.last().unwrap().id);
        let truncated = page.len() as i64 == q.limit;
        seen.extend(page.iter().map(|f| f.id));
        if !truncated {
            break;
        }
    }
    assert_eq!(seen.len(), 27);
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted, seen, "no duplicates, no gaps in order");
}

#[tokio::test]
#[ignore]
async fn without_a_cursor_the_newest_n_are_returned_oldest_first() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    let rows: Vec<NewFlash> = (0..30)
        .map(|i| flash("goes-east", i, i as i32, -105.0, 40.0))
        .collect();
    c.insert_flashes(&rows).await.unwrap();
    let mut q = query(FlashArea::Anywhere);
    q.limit = 5;
    let got = c.query_flashes(&q).await.unwrap();
    assert_eq!(
        got.iter().map(|f| f.flash_id).collect::<Vec<_>>(),
        vec![25, 26, 27, 28, 29]
    );
}

#[tokio::test]
#[ignore]
async fn satellite_and_time_filters() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    c.insert_flashes(&[
        flash("goes-east", 10, 1, -105.0, 40.0),
        flash("goes-west", 20, 2, -105.0, 40.0),
        flash("goes-east", 30, 3, -105.0, 40.0),
    ])
    .await
    .unwrap();

    let mut q = query(FlashArea::Anywhere);
    q.satellites = vec!["goes-east".into()];
    assert_eq!(c.query_flashes(&q).await.unwrap().len(), 2);

    q.satellites = vec![];
    q.since = t0() + Duration::seconds(15);
    q.until = Some(t0() + Duration::seconds(25));
    let got = c.query_flashes(&q).await.unwrap();
    assert_eq!(got.iter().map(|f| f.flash_id).collect::<Vec<_>>(), vec![2]);
}

#[tokio::test]
#[ignore]
async fn delete_before_removes_only_older_rows() {
    let e = env().await;
    let c = lc(e.catalog.pool());
    c.insert_flashes(&[
        flash("goes-east", -7200, 1, -105.0, 40.0),
        flash("goes-east", -60, 2, -105.0, 40.0),
        flash("goes-east", 60, 3, -105.0, 40.0),
    ])
    .await
    .unwrap();
    assert_eq!(c.delete_before(t0()).await.unwrap(), 2);
    assert_eq!(c.count().await.unwrap(), 1);
    assert_eq!(c.delete_before(t0()).await.unwrap(), 0);
}

#[tokio::test]
#[ignore]
async fn concurrent_writers_get_contiguous_non_interleaved_id_blocks() {
    // The change-feed cursor is only gap-free if ids become visible in order.
    // insert_flashes serializes writers with an advisory lock, so each batch's
    // ids form one contiguous block. Without the lock, concurrent INSERTs
    // interleave their nextval() calls and the blocks interleave.
    let e = env().await;
    let pool = e.catalog.pool().clone();
    let mut tasks = Vec::new();
    for w in 0..8 {
        let c = lc(&pool);
        tasks.push(tokio::spawn(async move {
            let rows: Vec<NewFlash> = (0..200)
                .map(|i| {
                    flash(
                        "goes-east",
                        w * 1000 + i,
                        (w * 1000 + i) as i32,
                        -105.0,
                        40.0,
                    )
                })
                .collect();
            c.insert_flashes(&rows).await.unwrap()
        }));
    }
    let mut blocks = Vec::new();
    for t in tasks {
        blocks.push(t.await.unwrap());
    }
    for ids in &blocks {
        assert_eq!(ids.len(), 200);
        assert_eq!(
            ids.last().unwrap() - ids.first().unwrap() + 1,
            200,
            "a writer's ids must be one contiguous block, got {}..{}",
            ids.first().unwrap(),
            ids.last().unwrap()
        );
    }
}

#[tokio::test]
#[ignore]
async fn granule_progress_is_recorded_even_with_no_flashes_and_never_moves_backwards() {
    // Monitoring depends on this: a quiet sky (granules with zero CONUS flashes)
    // must still advance the marker, and a late older granule must not rewind it.
    let e = env().await;
    let c = lc(e.catalog.pool());
    assert!(c.ingest_progress().await.unwrap().is_empty());

    c.record_granule("goes-east", t0() + Duration::seconds(40))
        .await
        .unwrap();
    c.record_granule("goes-east", t0() + Duration::seconds(20))
        .await
        .unwrap(); // late, older
    c.record_granule("goes-west", t0() + Duration::seconds(10))
        .await
        .unwrap();

    let p = c.ingest_progress().await.unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].0, "goes-east");
    assert_eq!(
        p[0].1,
        t0() + Duration::seconds(40),
        "must not move backwards"
    );
    assert_eq!(p[1].0, "goes-west");
    assert_eq!(c.count().await.unwrap(), 0, "no flashes were stored");
}
