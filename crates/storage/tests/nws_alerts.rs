//! Real-Postgres/PostGIS tests for `storage::nws_alerts`: geometry resolution
//! from lazily fetched zones, supersede / disappearance / cancel lifecycle,
//! failure handling, retention, and the point/bbox/id queries the EDR layer uses.
//!
//! `#[ignore]`d like the other integration tests (`-- --ignored`, Docker via
//! `test_utils`). To use a local database instead:
//!
//! ```text
//! docker run -d --name pg-nws -p 15435:5432 -e POSTGRES_PASSWORD=t postgis/postgis:16-3.4
//! NWS_TEST_DATABASE_URL=postgres://postgres:t@localhost:15435/postgres \
//!   cargo test -p storage --test nws_alerts -- --ignored --test-threads=1
//! ```

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{json, Value};
use storage::nws_alerts::{
    parse_feature, AlertRegion, AlertSearch, ParsedAlert, ZoneRecord, ZoneStatus,
};
use storage::{Catalog, NwsAlertCatalog};

const FIXTURE: &str =
    include_str!("../../../services/downloader/tests/fixtures/nws_alerts_active.json");

struct Env {
    catalog: Catalog,
    nws: NwsAlertCatalog,
    _infra: Option<test_utils::containers::TestInfrastructure>,
}

async fn env() -> Env {
    let (url, infra) = match std::env::var("NWS_TEST_DATABASE_URL") {
        Ok(url) => (url, None),
        Err(_) => {
            let infra = test_utils::containers::TestInfrastructure::start().await;
            (infra.postgres_url(), Some(infra))
        }
    };
    let catalog = Catalog::connect(&url).await.expect("connect");
    catalog.migrate().await.expect("migrate");
    catalog.migrate_observations().await.expect("PostGIS");
    catalog
        .migrate_nws_alerts()
        .await
        .expect("migrate_nws_alerts");
    // idempotent
    catalog.migrate_nws_alerts().await.expect("migrate twice");
    sqlx::query("TRUNCATE nws_alerts, nws_zones, nws_alerts_status")
        .execute(catalog.pool())
        .await
        .expect("truncate");
    // (left behind in a shared database by a test that installs it, see install_poison_trigger
    // and install_write_log)
    for trigger in ["nws_test_poison", "nws_test_write_log"] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger} ON nws_alerts"))
            .execute(catalog.pool())
            .await
            .expect("drop test trigger");
    }
    let nws = NwsAlertCatalog::new(catalog.pool_clone());
    Env {
        catalog,
        nws,
        _infra: infra,
    }
}

/// 2026-11-01 23:30Z: every fixture alert that is not already terminal is in force.
fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 23, 30, 0).unwrap()
}

fn features() -> Vec<Value> {
    let v: Value = serde_json::from_str(FIXTURE).unwrap();
    v["features"].as_array().unwrap().clone()
}

fn fixture_alerts() -> Vec<ParsedAlert> {
    features()
        .iter()
        .map(|f| parse_feature(f).unwrap())
        .collect()
}

fn id_of(alerts: &[ParsedAlert], event: &str, pick: impl Fn(&ParsedAlert) -> bool) -> String {
    alerts
        .iter()
        .find(|a| a.event == event && pick(a))
        .unwrap()
        .id
        .clone()
}

fn square(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> String {
    json!({"type": "Polygon", "coordinates": [[
        [min_lon, min_lat], [max_lon, min_lat], [max_lon, max_lat], [min_lon, max_lat], [min_lon, min_lat]
    ]]})
    .to_string()
}

fn zone(key: &str, status: ZoneStatus, geom: Option<String>) -> ZoneRecord {
    ZoneRecord {
        zone_key: key.to_string(),
        name: Some(format!("zone {key}")),
        state: Some("XX".into()),
        kind: Some("public".into()),
        status,
        geometry_json: geom,
    }
}

async fn source_of(env: &Env, id: &str) -> (String, bool) {
    let r: (String, bool) =
        sqlx::query_as("SELECT geometry_source, geom IS NOT NULL FROM nws_alerts WHERE id = $1")
            .bind(id)
            .fetch_one(env.catalog.pool())
            .await
            .unwrap();
    r
}

fn ids(v: &[storage::StoredAlert]) -> Vec<String> {
    let mut x: Vec<String> = v.iter().map(|a| a.id.clone()).collect();
    x.sort();
    x
}

// Zone squares used below.
const CO: [f64; 4] = [-107.5, 37.0, -106.5, 38.0]; // forecast/COZ068
const MD: [f64; 4] = [-77.3, 39.0, -76.8, 39.4]; // county/MDC031
const PZ_A: [f64; 4] = [-125.0, 40.0, -124.0, 41.0]; // forecast/PZZ455
const PZ_B: [f64; 4] = [-125.0, 41.0, -124.0, 42.0]; // forecast/PZZ475

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn zone_only_alerts_get_geometry_as_zones_arrive_and_point_query_finds_them() {
    let env = env().await;
    let alerts = fixture_alerts();
    let r = env.nws.apply_feed(&alerts, None, t0()).await.unwrap();
    assert_eq!(r.received, alerts.len());
    assert_eq!(r.ended, 0);

    // Alerts are stored immediately; zone-only ones wait for geometry.
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |_| true);
    let test_msg = id_of(&alerts, "Test Message", |_| true);
    let sca = id_of(&alerts, "Small Craft Advisory", |a| a.zone_keys.len() == 2);
    assert_eq!(source_of(&env, &svr).await, ("polygon".into(), true));
    assert_eq!(source_of(&env, &winter).await, ("pending".into(), false));

    // Polygon alerts never ask for their zones; the rest are all listed once.
    assert!(r.missing_zones.contains(&"forecast/COZ068".to_string()));
    assert!(r.missing_zones.contains(&"county/MDC031".to_string()));
    assert!(
        !r.missing_zones.contains(&"county/NMC001".to_string()),
        "svr has a polygon"
    );
    let mut sorted = r.missing_zones.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), r.missing_zones.len(), "no duplicates");

    // Before the zones arrive, the point is in nobody's geometry.
    assert!(env
        .nws
        .active_at_point(-107.0, 37.5, t0(), false)
        .await
        .unwrap()
        .is_empty());

    // Zones arrive.
    let n = env
        .nws
        .upsert_zones(
            &[
                zone(
                    "forecast/COZ068",
                    ZoneStatus::Ok,
                    Some(square(CO[0], CO[1], CO[2], CO[3])),
                ),
                zone(
                    "county/MDC031",
                    ZoneStatus::Ok,
                    Some(square(MD[0], MD[1], MD[2], MD[3])),
                ),
                // one of the marine alert's two zones
                zone(
                    "forecast/PZZ455",
                    ZoneStatus::Ok,
                    Some(square(PZ_A[0], PZ_A[1], PZ_A[2], PZ_A[3])),
                ),
            ],
            t0(),
        )
        .await
        .unwrap();
    assert!(
        n.alerts_updated >= 3,
        "at least winter, test message, marine updated (got {n:?})"
    );
    assert_eq!((n.stored, n.invalid_geometry, n.failed), (3, 0, 0));
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    assert_eq!(source_of(&env, &test_msg).await, ("zones".into(), true));
    // Marine alert has one of two zones: partial geometry, still pending.
    assert_eq!(source_of(&env, &sca).await, ("pending".into(), true));

    // (The fixture's Avalanche Advisory also lists COZ068 among four zones: it has partial geometry.)
    let avalanche = id_of(&alerts, "Avalanche Advisory", |_| true);
    let hit = env
        .nws
        .active_at_point(-107.0, 37.5, t0(), true)
        .await
        .unwrap();
    let mut want = vec![winter.clone(), avalanche.clone()];
    want.sort();
    assert_eq!(ids(&hit), want);
    let w = hit.iter().find(|a| a.id == winter).unwrap();
    assert_eq!(w.event, "Winter Storm Warning");
    assert_eq!(w.color, "#FF69B4");
    assert!(w
        .geometry_geojson
        .as_deref()
        .unwrap()
        .contains("MultiPolygon"));
    assert!(w.sections["what"].is_string());
    assert!(w.is_active(t0()));
    assert_eq!(source_of(&env, &avalanche).await, ("pending".into(), true));
    let miss = env
        .nws
        .active_at_point(-100.0, 37.5, t0(), false)
        .await
        .unwrap();
    assert!(miss.is_empty());
    assert!(env
        .nws
        .active_at_point(-107.0, 37.5, t0(), false)
        .await
        .unwrap()
        .iter()
        .all(|a| a.geometry_geojson.is_none()));

    // Second marine zone arrives: union of both squares, now final.
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/PZZ475",
                ZoneStatus::Ok,
                Some(square(PZ_B[0], PZ_B[1], PZ_B[2], PZ_B[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    assert_eq!(source_of(&env, &sca).await, ("zones".into(), true));
    for (lon, lat) in [(-124.5, 40.5), (-124.5, 41.5)] {
        let h = env
            .nws
            .active_at_point(lon, lat, t0(), false)
            .await
            .unwrap();
        assert!(
            ids(&h).contains(&sca),
            "({lon},{lat}) should be inside the union"
        );
    }
    assert!(env
        .nws
        .active_at_point(-124.5, 42.5, t0(), false)
        .await
        .unwrap()
        .is_empty());

    // Polygon alert: point inside its own polygon.
    let (px, py): (f64, f64) = sqlx::query_as(
        "SELECT ST_X(ST_PointOnSurface(geom)), ST_Y(ST_PointOnSurface(geom)) FROM nws_alerts WHERE id = $1",
    )
    .bind(&svr)
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    let h = env.nws.active_at_point(px, py, t0(), false).await.unwrap();
    assert!(ids(&h).contains(&svr));

    // A 404 zone is "resolved without geometry": an alert whose zones all 404 settles on 'none'.
    let gale = id_of(&alerts, "Gale Warning", |_| true);
    let gale_zones = alerts
        .iter()
        .find(|a| a.id == gale)
        .unwrap()
        .zone_keys
        .clone();
    let recs: Vec<ZoneRecord> = gale_zones
        .iter()
        .map(|k| zone(k, ZoneStatus::NotFound, None))
        .collect();
    env.nws.upsert_zones(&recs, t0()).await.unwrap();
    assert_eq!(source_of(&env, &gale).await, ("none".into(), false));
    // ...and is not stuck there: when one of its zones later comes back with geometry (the 24 h
    // retry after a 404), the alert picks it up.
    env.nws
        .upsert_zones(
            &[zone(
                &gale_zones[0],
                ZoneStatus::Ok,
                Some(square(-125.0, 40.0, -124.0, 41.0)),
            )],
            t0(),
        )
        .await
        .unwrap();
    assert_eq!(source_of(&env, &gale).await, ("zones".into(), true));
    // A 404 on a refresh never wipes a zone we already hold.
    env.nws
        .upsert_zones(&[zone("forecast/COZ068", ZoneStatus::NotFound, None)], t0())
        .await
        .unwrap();
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    let st: String =
        sqlx::query_scalar("SELECT fetch_status FROM nws_zones WHERE zone_key = 'forecast/COZ068'")
            .fetch_one(env.catalog.pool())
            .await
            .unwrap();
    assert_eq!(st, "ok");

    // Fresh zones are not re-requested; stale ones are; failed ones after a day.
    let all_zones: Vec<String> = alerts
        .iter()
        .filter(|a| a.geometry_json.is_none())
        .flat_map(|a| a.zone_keys.clone())
        .collect();
    let recs: Vec<ZoneRecord> = all_zones
        .iter()
        .map(|k| match k.as_str() {
            "forecast/COZ068" => zone(k, ZoneStatus::Ok, Some(square(CO[0], CO[1], CO[2], CO[3]))),
            _ => zone(k, ZoneStatus::NotFound, None),
        })
        .collect();
    env.nws.upsert_zones(&recs, t0()).await.unwrap();
    assert!(env.nws.missing_zones(t0()).await.unwrap().is_empty());
    let day_later = env
        .nws
        .missing_zones(t0() + Duration::hours(25))
        .await
        .unwrap();
    assert!(
        !day_later.is_empty() && !day_later.contains(&"forecast/COZ068".to_string()),
        "ok zones keep for 30 d; failed retry after 24 h"
    );
    let month_later = env
        .nws
        .missing_zones(t0() + Duration::days(31))
        .await
        .unwrap();
    assert!(month_later.contains(&"forecast/COZ068".to_string()));

    // Re-polling the same feed keeps the resolved geometry (zones unchanged).
    env.nws
        .apply_feed(&alerts, None, t0() + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    assert_eq!(source_of(&env, &svr).await.0, "polygon");
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn lifecycle_supersede_disappear_cancel_and_failure() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, None, t0()).await.unwrap();
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |_| true);
    let ffw = id_of(&alerts, "Flash Flood Warning", |_| true);

    // A VTEC EXP message ("will expire at 23:30Z", sent 23:26Z) is in force until its end time:
    // not ended on arrival, active at 23:28Z, over at 23:30Z.
    let ffw_row = env.nws.get(&ffw, false).await.unwrap().unwrap();
    assert!(ffw_row.ended_at.is_none());
    let before_end = t0() - Duration::minutes(2);
    assert!(ffw_row.is_active(before_end) && !ffw_row.is_active(t0()));
    let (fx, fy): (f64, f64) = sqlx::query_as(
        "SELECT ST_X(ST_PointOnSurface(geom)), ST_Y(ST_PointOnSurface(geom)) FROM nws_alerts WHERE id = $1",
    )
    .bind(&ffw)
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    assert!(
        ids(&env
            .nws
            .active_at_point(fx, fy, before_end, false)
            .await
            .unwrap())
        .contains(&ffw),
        "the SQL predicate agrees: in force at 23:28Z"
    );
    assert!(
        !ids(&env.nws.active_at_point(fx, fy, t0(), false).await.unwrap()).contains(&ffw),
        "...and over at its end time"
    );

    // 1. Update: references the winter warning -> it is superseded, the update is live.
    let mut upd = features()
        .into_iter()
        .find(|f| f["properties"]["id"] == winter.as_str())
        .unwrap();
    upd["properties"]["id"] = json!("urn:test:winter-update");
    upd["properties"]["messageType"] = json!("Update");
    upd["properties"]["references"] =
        json!([{"identifier": winter, "sent": "2026-11-01T10:27:00-07:00"}]);
    upd["properties"]["headline"] = json!("Updated headline");
    let upd = parse_feature(&upd).unwrap();
    let mut feed2: Vec<ParsedAlert> = alerts.iter().filter(|a| a.id != winter).cloned().collect();
    feed2.push(upd.clone());
    let r = env
        .nws
        .apply_feed(&feed2, None, t0() + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(r.superseded, 1);
    let old = env.nws.get(&winter, false).await.unwrap().unwrap();
    assert_eq!(old.superseded_by.as_deref(), Some("urn:test:winter-update"));
    assert!(!old.is_active(t0()));
    assert_eq!(old.ended_at, None, "superseded is not 'ended'");
    // The update shares the zone, so it picks up the cached geometry immediately.
    let hit = env
        .nws
        .active_at_point(-107.0, 37.5, t0(), false)
        .await
        .unwrap();
    let avalanche = id_of(&alerts, "Avalanche Advisory", |_| true); // also lists COZ068
    let mut want = vec!["urn:test:winter-update".to_string(), avalanche.clone()];
    want.sort();
    assert_eq!(ids(&hit), want);
    let u = hit
        .iter()
        .find(|a| a.id == "urn:test:winter-update")
        .unwrap();
    assert_eq!(u.headline.as_deref(), Some("Updated headline"));
    assert_eq!(u.references_ids, vec![winter.clone()]);

    // 2. Disappearance: a poll without the SVR ends it; others stay live.
    let feed3: Vec<ParsedAlert> = feed2.iter().filter(|a| a.id != svr).cloned().collect();
    let r = env
        .nws
        .apply_feed(&feed3, None, t0() + Duration::minutes(2))
        .await
        .unwrap();
    assert_eq!(r.ended, 1);
    let svr_row = env.nws.get(&svr, false).await.unwrap().unwrap();
    assert_eq!(svr_row.ended_at, Some(t0() + Duration::minutes(2)));
    assert!(!svr_row.is_active(t0()));
    // ...and its polygon no longer answers point queries.
    let (px, py): (f64, f64) = sqlx::query_as(
        "SELECT ST_X(ST_PointOnSurface(geom)), ST_Y(ST_PointOnSurface(geom)) FROM nws_alerts WHERE id = $1",
    )
    .bind(&svr)
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    assert!(!ids(&env.nws.active_at_point(px, py, t0(), false).await.unwrap()).contains(&svr));
    // ...but a bbox/time (history) query still sees it, flagged as ended.
    let hist = env
        .nws
        .in_bbox(
            [px - 0.1, py - 0.1, px + 0.1, py + 0.1],
            t0() - Duration::hours(6),
            t0() + Duration::hours(6),
            false,
            false,
            t0(),
            50,
            false,
        )
        .await
        .unwrap();
    assert!(hist.iter().any(|a| a.id == svr && a.ended_at.is_some()));
    let active_only = env
        .nws
        .in_bbox(
            [px - 0.1, py - 0.1, px + 0.1, py + 0.1],
            t0() - Duration::hours(6),
            t0() + Duration::hours(6),
            true,
            false,
            t0(),
            50,
            false,
        )
        .await
        .unwrap();
    assert!(!active_only.iter().any(|a| a.id == svr));

    // 3. It comes back in the feed: live again.
    env.nws
        .apply_feed(&feed2, None, t0() + Duration::minutes(3))
        .await
        .unwrap();
    assert_eq!(
        env.nws.get(&svr, false).await.unwrap().unwrap().ended_at,
        None
    );
    // ...and the EXP message was never ended: NWS keeps listing it, its end time decides.
    assert!(env
        .nws
        .get(&ffw, false)
        .await
        .unwrap()
        .unwrap()
        .ended_at
        .is_none());

    // 4. A failed poll changes nothing but the status row.
    let before = env.nws.status().await.unwrap().unwrap();
    env.nws
        .record_poll_failure(
            "GET /alerts/active returned 503",
            t0() + Duration::minutes(4),
        )
        .await
        .unwrap();
    let after = env.nws.status().await.unwrap().unwrap();
    assert_eq!(
        after.last_success_at, before.last_success_at,
        "success time is not advanced by a failure"
    );
    assert_eq!(
        after.last_error.as_deref(),
        Some("GET /alerts/active returned 503")
    );
    assert!(env
        .nws
        .get(&svr, false)
        .await
        .unwrap()
        .unwrap()
        .ended_at
        .is_none());
    // An empty feed must not end anything either.
    let r = env
        .nws
        .apply_feed(&[], None, t0() + Duration::minutes(5))
        .await
        .unwrap();
    assert_eq!(r.ended, 0);
    assert!(env
        .nws
        .get(&svr, false)
        .await
        .unwrap()
        .unwrap()
        .ended_at
        .is_none());
    // 304: success, data age moves.
    env.nws
        .record_not_modified(t0() + Duration::minutes(6))
        .await
        .unwrap();
    let st = env.nws.status().await.unwrap().unwrap();
    assert_eq!(st.last_success_at, Some(t0() + Duration::minutes(6)));
    assert_eq!(st.last_alert_count, Some(feed2.len() as i32));

    // 5. A Cancel supersedes what it references and is itself over.
    let mut cancel = features()
        .into_iter()
        .find(|f| f["properties"]["id"] == winter.as_str())
        .unwrap();
    cancel["properties"]["id"] = json!("urn:test:winter-cancel");
    cancel["properties"]["messageType"] = json!("Cancel");
    cancel["properties"]["references"] = json!([{"identifier": "urn:test:winter-update"}]);
    let cancel = parse_feature(&cancel).unwrap();
    let mut feed4: Vec<ParsedAlert> = feed2
        .iter()
        .filter(|a| a.id != "urn:test:winter-update")
        .cloned()
        .collect();
    feed4.push(cancel);
    env.nws
        .apply_feed(&feed4, None, t0() + Duration::minutes(7))
        .await
        .unwrap();
    assert_eq!(
        ids(&env
            .nws
            .active_at_point(-107.0, 37.5, t0(), false)
            .await
            .unwrap()),
        vec![avalanche.clone()]
    );
    let c = env
        .nws
        .get("urn:test:winter-cancel", false)
        .await
        .unwrap()
        .unwrap();
    assert!(c.ended_at.is_some() && !c.is_active(t0()));
    assert_eq!(
        env.nws
            .get("urn:test:winter-update", false)
            .await
            .unwrap()
            .unwrap()
            .superseded_by
            .as_deref(),
        Some("urn:test:winter-cancel")
    );

    // 6. Time alone ends an alert even if the feed is stale: past `ends`, not active.
    let late = Utc.with_ymd_and_hms(2026, 11, 5, 0, 0, 0).unwrap();
    let (px, py): (f64, f64) = (px, py);
    assert!(env
        .nws
        .active_at_point(px, py, late, false)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn retention_deletes_week_old_ended_alerts_and_keeps_zones() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, None, t0()).await.unwrap();
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |_| true);

    // Nothing is old enough yet.
    assert_eq!(
        env.nws
            .delete_retired_before(t0() - Duration::days(7))
            .await
            .unwrap(),
        0
    );

    // Ten days on, NWS lists only the SVR: everything else ended 10 days ago.
    let later = t0() + Duration::days(10);
    let only_svr: Vec<ParsedAlert> = alerts.iter().filter(|a| a.id == svr).cloned().collect();
    let r = env.nws.apply_feed(&only_svr, None, later).await.unwrap();
    assert_eq!(
        r.ended as usize,
        alerts.len() - 1,
        "everything but the SVR (the EXP message is no longer ended on arrival)"
    );

    let deleted = env
        .nws
        .delete_retired_before(later - Duration::days(7))
        .await
        .unwrap();
    assert_eq!(deleted as usize, alerts.len() - 1);
    assert!(
        env.nws.get(&svr, false).await.unwrap().is_some(),
        "still listed: kept"
    );
    let zones: i64 = sqlx::query_scalar("SELECT count(*) FROM nws_zones")
        .fetch_one(env.catalog.pool())
        .await
        .unwrap();
    assert_eq!(zones, 1, "zones are never swept");

    // A recently ended alert is kept: end it now, sweep with a 7-day window.
    env.nws
        .apply_feed(&alerts, None, later + Duration::hours(1))
        .await
        .unwrap();
    env.nws
        .apply_feed(&only_svr, None, later + Duration::hours(2))
        .await
        .unwrap();
    assert_eq!(
        env.nws
            .delete_retired_before(later + Duration::hours(2) - Duration::days(7))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn one_alert_by_id_carries_the_predigested_fields() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |_| true);

    let a = env.nws.get(&svr, true).await.unwrap().unwrap();
    assert_eq!(a.event, "Severe Thunderstorm Warning");
    assert_eq!(a.vtec[0]["action"], "NEW");
    assert_eq!(a.vtec[0]["office"], "KABQ");
    assert_eq!(a.vtec_event_key.as_deref(), Some("KABQ.SV.W.0428"));
    assert_eq!(a.sections["hazard"], "60 mph wind gusts.");
    assert_eq!(a.color, "#FFA500");
    assert_eq!(a.geometry_source, "polygon");
    assert!(a
        .geometry_geojson
        .unwrap()
        .starts_with("{\"type\":\"MultiPolygon\""));
    assert!(a.ugc.is_empty() || a.ugc.iter().all(|u| u.len() == 6));
    assert!(a.end_time.is_some());
    assert!(env.nws.get("urn:nope", false).await.unwrap().is_none());

    let st = env.nws.status().await.unwrap().unwrap();
    assert_eq!(st.last_success_at, Some(t0()));
    assert_eq!(st.feed_updated_at, Some(t0()));
    assert_eq!(st.last_alert_count, Some(alerts.len() as i32));

    // The point query uses the GIST index (plan check on a table big enough to prefer it).
    // One connection for both statements: a session setting does not follow a pool.
    let mut conn = env.catalog.pool().acquire().await.unwrap();
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .unwrap();
    let plan: Vec<(String,)> = sqlx::query_as(
        "EXPLAIN SELECT id FROM nws_alerts a WHERE a.superseded_by IS NULL AND a.ended_at IS NULL \
         AND a.geom && ST_SetSRID(ST_MakePoint(-107, 37.5), 4326) \
         AND ST_Intersects(a.geom, ST_SetSRID(ST_MakePoint(-107, 37.5), 4326))",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    let plan = plan.into_iter().map(|r| r.0).collect::<Vec<_>>().join("\n");
    assert!(plan.contains("idx_nws_alerts_geom"), "{plan}");
}

fn search(region: AlertRegion, now: DateTime<Utc>) -> AlertSearch {
    AlertSearch {
        region,
        now,
        window: None,
        include_superseded: false,
        severities: vec![],
        events: vec![],
        limit: 100,
        with_geometry: false,
        simplify: None,
        include_non_actual: false,
        max_geometry_bytes: None,
    }
}

/// A copy of the fixture's Severe Thunderstorm Warning under a new id, with a CAP `status`
/// (`None` removes the field).
fn svr_with_status(id: &str, status: Option<&str>) -> ParsedAlert {
    let mut f = features()
        .into_iter()
        .find(|f| f["properties"]["event"] == "Severe Thunderstorm Warning")
        .unwrap();
    f["properties"]["id"] = json!(id);
    match status {
        Some(s) => f["properties"]["status"] = json!(s),
        None => {
            f["properties"].as_object_mut().unwrap().remove("status");
        }
    }
    parse_feature(&f).unwrap()
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn test_exercise_and_draft_alerts_are_stored_but_only_actual_ones_are_served_by_default() {
    let env = env().await;
    let mut alerts = fixture_alerts();
    for (id, st) in [
        ("urn:test:exercise", Some("Exercise")),
        ("urn:test:draft", Some("Draft")),
        ("urn:test:system", Some("System")),
        ("urn:test:nostatus", None),
    ] {
        alerts.push(svr_with_status(id, st));
    }
    env.nws.apply_feed(&alerts, None, t0()).await.unwrap();
    env.nws
        .upsert_zones(
            &[zone(
                "county/MDC031",
                ZoneStatus::Ok,
                Some(square(MD[0], MD[1], MD[2], MD[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |a| {
        a.id.starts_with("urn:oid")
    });
    let test_msg = id_of(&alerts, "Test Message", |_| true);

    // Everything NWS offers is stored, whatever its status.
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM nws_alerts")
        .fetch_one(env.catalog.pool())
        .await
        .unwrap();
    assert_eq!(stored as usize, alerts.len());
    let (px, py): (f64, f64) = sqlx::query_as(
        "SELECT ST_X(ST_PointOnSurface(geom)), ST_Y(ST_PointOnSurface(geom)) FROM nws_alerts WHERE id = $1",
    )
    .bind(&svr)
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    let md = AlertRegion::Point {
        lon: (MD[0] + MD[2]) / 2.0,
        lat: (MD[1] + MD[3]) / 2.0,
    };
    let svr_ids = ["urn:test:exercise", "urn:test:draft", "urn:test:system"];

    // Point search: Actual (and status-less) only by default.
    let point = AlertRegion::Point { lon: px, lat: py };
    let mut q = search(point.clone(), t0() - Duration::minutes(10));
    let got = ids(&env.nws.search(&q).await.unwrap());
    let mut want = vec![svr.clone(), "urn:test:nostatus".to_string()];
    want.sort();
    assert_eq!(got, want, "Exercise / Draft / System are hidden by default");
    q.include_non_actual = true;
    let got = ids(&env.nws.search(&q).await.unwrap());
    assert!(
        svr_ids.iter().all(|i| got.contains(&i.to_string())) && got.contains(&svr),
        "include_non_actual returns them too: {got:?}"
    );
    assert_eq!(got.len(), 5);

    // The county-zone "Test Message" (CAP status Test) is hidden from a point search...
    let mut q = search(md.clone(), t0() - Duration::minutes(1));
    assert!(env.nws.search(&q).await.unwrap().is_empty());
    q.include_non_actual = true;
    let got = env.nws.search(&q).await.unwrap();
    assert_eq!(ids(&got), vec![test_msg.clone()]);
    assert_eq!(got[0].status.as_deref(), Some("Test"));

    // ...and from every other region type and from history windows.
    let bbox = AlertRegion::Bbox([MD[0] - 0.1, MD[1] - 0.1, MD[2] + 0.1, MD[3] + 0.1]);
    let poly = AlertRegion::Polygon {
        wkt: format!(
            "POLYGON(({w} {s}, {e} {s}, {e} {n}, {w} {n}, {w} {s}))",
            w = MD[0] - 0.1,
            s = MD[1] - 0.1,
            e = MD[2] + 0.1,
            n = MD[3] + 0.1
        ),
    };
    let radius = AlertRegion::Radius {
        lon: (MD[0] + MD[2]) / 2.0,
        lat: (MD[1] + MD[3]) / 2.0,
        meters: 50_000.0,
    };
    for region in [bbox, poly, radius] {
        let mut q = search(region, t0() - Duration::minutes(1));
        assert!(
            env.nws.search(&q).await.unwrap().is_empty(),
            "{:?}",
            q.region
        );
        q.include_non_actual = true;
        assert_eq!(
            ids(&env.nws.search(&q).await.unwrap()),
            vec![test_msg.clone()]
        );
        // history window
        let mut h = search(q.region.clone(), t0());
        h.window = Some((t0() - Duration::hours(2), t0()));
        assert!(env.nws.search(&h).await.unwrap().is_empty());
        h.include_non_actual = true;
        assert_eq!(
            ids(&env.nws.search(&h).await.unwrap()),
            vec![test_msg.clone()]
        );
    }

    // The older single-purpose reads are Actual-only too.
    assert!(env
        .nws
        .active_at_point(
            md_lonlat().0,
            md_lonlat().1,
            t0() - Duration::minutes(1),
            false
        )
        .await
        .unwrap()
        .is_empty());

    // An alert by id comes back whatever its status.
    let one = env.nws.get(&test_msg, false).await.unwrap().unwrap();
    assert_eq!(one.status.as_deref(), Some("Test"));
}

fn md_lonlat() -> (f64, f64) {
    ((MD[0] + MD[2]) / 2.0, (MD[1] + MD[3]) / 2.0)
}

/// GeoJSON that `ST_GeomFromGeoJSON` raises an error for (not just an empty or degenerate shape).
const POISON_GEOJSON: &str = r#"{"type":"Polygon","coordinates":"not an array"}"#;
const POISON_GEOJSON_2: &str = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0,5],[1],[0,0]]]}"#;

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn an_alert_with_a_polygon_postgis_rejects_does_not_poison_the_feed() {
    let env = env().await;
    let mut alerts = fixture_alerts();
    // One poisoned alert that has zones to fall back on, one with nothing to fall back on.
    let mut with_zone = svr_with_status("urn:test:bad-with-zone", Some("Actual"));
    with_zone.geometry_json = Some(POISON_GEOJSON.into());
    with_zone.zone_keys = vec!["forecast/COZ068".into()];
    let mut no_zone = svr_with_status("urn:test:bad-no-zone", Some("Actual"));
    no_zone.geometry_json = Some(POISON_GEOJSON_2.into());
    no_zone.zone_keys = vec![];
    alerts.push(with_zone);
    alerts.push(no_zone);

    let r = env
        .nws
        .apply_feed(&alerts, Some(t0()), t0())
        .await
        .expect("one bad polygon must not fail the whole feed");
    assert_eq!(r.received, alerts.len());
    assert_eq!(r.upserted, alerts.len(), "every alert is stored");
    assert_eq!((r.geometry_rejected, r.failed), (2, 0));
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM nws_alerts")
        .fetch_one(env.catalog.pool())
        .await
        .unwrap();
    assert_eq!(stored as usize, alerts.len());

    // The valid ones are untouched, the bad ones are stored without their polygon.
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |a| {
        a.id.starts_with("urn:oid")
    });
    assert_eq!(source_of(&env, &svr).await, ("polygon".into(), true));
    assert_eq!(
        source_of(&env, "urn:test:bad-with-zone").await,
        ("pending".into(), false),
        "falls back to its zones"
    );
    assert_eq!(
        source_of(&env, "urn:test:bad-no-zone").await,
        ("none".into(), false),
        "nothing to fall back on"
    );
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    assert_eq!(
        source_of(&env, "urn:test:bad-with-zone").await,
        ("zones".into(), true)
    );

    // The poll counts as a success: the status moved on (so the downloader keeps its ETag).
    let st = env.nws.status().await.unwrap().unwrap();
    assert_eq!(st.last_success_at, Some(t0()));
    assert_eq!(st.feed_updated_at, Some(t0()));
    assert_eq!(st.last_alert_count, Some(alerts.len() as i32));
    assert!(st.last_error.is_none());

    // The same feed next minute is just as fine (the bad row does not recur as a failure).
    let r = env
        .nws
        .apply_feed(
            &alerts,
            Some(t0() + Duration::minutes(1)),
            t0() + Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!((r.ended, r.geometry_rejected, r.failed), (0, 2, 0));
    assert_eq!(
        (r.upserted, r.unchanged),
        (2, alerts.len() - 2),
        "the two stored without their polygon are tried with it again, the rest are as they were"
    );
    assert_eq!(
        source_of(&env, "urn:test:bad-with-zone").await,
        ("zones".into(), true),
        "geometry from the zones is kept across polls"
    );

    // An alert that was fine and then arrives with a bad polygon stays live.
    let mut next = alerts.clone();
    let changed = next.iter_mut().find(|a| a.id == svr).unwrap();
    changed.geometry_json = Some(POISON_GEOJSON.into());
    changed.content_hash = None;
    env.nws
        .apply_feed(
            &next,
            Some(t0() + Duration::minutes(2)),
            t0() + Duration::minutes(2),
        )
        .await
        .unwrap();
    let row = env.nws.get(&svr, false).await.unwrap().unwrap();
    assert!(row.ended_at.is_none() && row.superseded_by.is_none());
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn one_bad_zone_does_not_poison_the_batch() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    let test_msg = id_of(&alerts, "Test Message", |_| true);
    let sca = id_of(&alerts, "Small Craft Advisory", |a| a.zone_keys.len() == 2);

    let r = env
        .nws
        .upsert_zones(
            &[
                zone(
                    "forecast/COZ068",
                    ZoneStatus::Ok,
                    Some(square(CO[0], CO[1], CO[2], CO[3])),
                ),
                // the county zone's geometry makes PostGIS raise
                zone("county/MDC031", ZoneStatus::Ok, Some(POISON_GEOJSON.into())),
                zone(
                    "forecast/PZZ455",
                    ZoneStatus::Ok,
                    Some(square(PZ_A[0], PZ_A[1], PZ_A[2], PZ_A[3])),
                ),
                zone(
                    "forecast/PZZ475",
                    ZoneStatus::Ok,
                    Some(square(PZ_B[0], PZ_B[1], PZ_B[2], PZ_B[3])),
                ),
            ],
            t0(),
        )
        .await
        .expect("one bad zone must not fail the batch");
    assert_eq!((r.stored, r.invalid_geometry, r.failed), (4, 1, 0));

    // The good zones landed and their alerts have geometry...
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    assert_eq!(source_of(&env, &sca).await, ("zones".into(), true));
    // ...the bad one is recorded as such, with no geometry, so its alert settles on none.
    let (status, has_geom): (String, bool) = sqlx::query_as(
        "SELECT fetch_status, geom IS NOT NULL FROM nws_zones WHERE zone_key = 'county/MDC031'",
    )
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    assert_eq!((status.as_str(), has_geom), ("invalid_geometry", false));
    assert_eq!(source_of(&env, &test_msg).await, ("none".into(), false));

    // It is retried after a day like any other zone without usable geometry, not every poll.
    assert!(!env
        .nws
        .missing_zones(t0())
        .await
        .unwrap()
        .contains(&"county/MDC031".to_string()));
    assert!(env
        .nws
        .missing_zones(t0() + Duration::hours(25))
        .await
        .unwrap()
        .contains(&"county/MDC031".to_string()));

    // A later refresh that returns garbage never wipes a geometry we hold.
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(POISON_GEOJSON_2.into()),
            )],
            t0() + Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
}

/// A trigger that raises (a stand-in for any per-row failure: a GEOS error in a union, text the
/// database refuses) whenever the poison alert's geometry columns are written with `zones` or
/// `pending`.
async fn install_poison_trigger(env: &Env) {
    sqlx::query(
        "CREATE OR REPLACE FUNCTION nws_test_poison() RETURNS trigger LANGUAGE plpgsql AS $f$ \
         BEGIN \
           IF NEW.id = 'urn:test:poison' AND NEW.geometry_source IN ('zones', 'pending') THEN \
             RAISE EXCEPTION 'poisoned row' USING ERRCODE = 'XX000'; \
           END IF; \
           RETURN NEW; \
         END $f$",
    )
    .execute(env.catalog.pool())
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER IF EXISTS nws_test_poison ON nws_alerts")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER nws_test_poison BEFORE UPDATE ON nws_alerts \
         FOR EACH ROW EXECUTE FUNCTION nws_test_poison()",
    )
    .execute(env.catalog.pool())
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_row_the_database_refuses_is_skipped_in_the_feed_and_in_the_zone_recompute() {
    let env = env().await;
    let mut alerts = fixture_alerts();
    let mut poison = svr_with_status("urn:test:poison", Some("Actual"));
    poison.geometry_json = None;
    poison.zone_keys = vec!["forecast/COZ068".into()];
    alerts.push(poison);
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |a| {
        a.id.starts_with("urn:oid")
    });
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    assert_eq!(
        source_of(&env, "urn:test:poison").await,
        ("zones".into(), true)
    );

    install_poison_trigger(&env).await;

    // Feed: the poison row's update fails, the rest of the feed goes through.
    let mut next = alerts.clone();
    for a in &mut next {
        if a.id == "urn:test:poison" || a.id == svr {
            a.headline = Some("changed headline".into());
            a.description = Some("changed description".into());
            a.content_hash = None;
        }
    }
    let t1 = t0() + Duration::minutes(1);
    let r = env
        .nws
        .apply_feed(&next, Some(t1), t1)
        .await
        .expect("a refused row must not fail the feed");
    assert_eq!(
        (r.failed, r.upserted, r.unchanged),
        (1, 1, alerts.len() - 2),
        "the poison row fails alone, the changed one is rewritten, the rest only touched"
    );
    assert_eq!(
        env.nws
            .get(&svr, false)
            .await
            .unwrap()
            .unwrap()
            .headline
            .as_deref(),
        Some("changed headline"),
        "the rest of the feed was applied"
    );
    let p = env
        .nws
        .get("urn:test:poison", false)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(p.headline.as_deref(), Some("changed headline"));
    assert!(
        p.ended_at.is_none() && p.superseded_by.is_none(),
        "an alert that is still listed stays live even though its update was refused"
    );
    let st = env.nws.status().await.unwrap().unwrap();
    assert_eq!(st.last_success_at, Some(t1));
    assert_eq!(st.feed_updated_at, Some(t1));

    // Zones: the poison alert's recomputation fails, the others are recomputed.
    let z = env
        .nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0] - 0.1, CO[1], CO[2], CO[3])),
            )],
            t1,
        )
        .await
        .expect("a refused recomputation must not fail the zone batch");
    assert_eq!((z.stored, z.alerts_failed), (1, 1));
    assert!(z.alerts_updated >= 2, "{z:?}");
    assert_eq!(
        source_of(&env, "urn:test:poison").await,
        ("none".into(), false)
    );
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    let hit = env
        .nws
        .active_at_point(CO[0] - 0.05, 37.5, t0() + Duration::hours(7), false)
        .await
        .unwrap();
    assert!(
        ids(&hit).contains(&winter),
        "the refreshed zone geometry reached the other alerts"
    );
}

/// The race the advisory lock closes, reproduced by hand: a zone batch committed (and
/// recomputed only the alerts it could see) while the poll that stores this alert was still
/// in flight. The alert is left `pending` with a partial geometry even though all its zones
/// exist; the next poll must finish it, not skip it because its geometry is not NULL.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_pending_alert_with_partial_geometry_is_completed_by_the_next_poll() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    let sca = id_of(&alerts, "Small Craft Advisory", |a| a.zone_keys.len() == 2);

    // One of its two zones arrives: partial geometry, pending.
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/PZZ455",
                ZoneStatus::Ok,
                Some(square(PZ_A[0], PZ_A[1], PZ_A[2], PZ_A[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    assert_eq!(source_of(&env, &sca).await, ("pending".into(), true));

    // The second zone is committed behind the alert's back (no recompute of this alert).
    sqlx::query(
        "INSERT INTO nws_zones (zone_key, zone_type, zone_id, geom, fetch_status, fetched_at) \
         VALUES ('forecast/PZZ475', 'forecast', 'PZZ475', \
                 ST_Multi(ST_GeomFromText('POLYGON((-125 41, -124 41, -124 42, -125 42, -125 41))', 4326)), 'ok', $1)",
    )
    .bind(t0())
    .execute(env.catalog.pool())
    .await
    .unwrap();
    assert_eq!(
        source_of(&env, &sca).await,
        ("pending".into(), true),
        "still stuck"
    );

    // The next poll (nothing about this alert changed) completes it.
    let t1 = t0() + Duration::minutes(1);
    env.nws.apply_feed(&alerts, Some(t1), t1).await.unwrap();
    assert_eq!(source_of(&env, &sca).await, ("zones".into(), true));
    for (lon, lat) in [(-124.5, 40.5), (-124.5, 41.5)] {
        let h = env
            .nws
            .active_at_point(lon, lat, t0(), false)
            .await
            .unwrap();
        assert!(
            ids(&h).contains(&sca),
            "({lon},{lat}) is inside the union of both zones"
        );
    }
}

/// Feed polls and zone batches take the same transaction-scoped advisory lock first, so
/// neither can commit while the other is mid-flight (and they cannot deadlock on row locks).
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn feed_polls_and_zone_batches_wait_for_each_other() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();

    for writer in ["feed", "zones"] {
        // Somebody else holds the writers' lock...
        let mut holder = env.catalog.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(storage::nws_alerts::NWS_WRITE_LOCK_KEY)
            .execute(&mut *holder)
            .await
            .unwrap();

        let nws = env.nws.clone();
        let t1 = t0() + Duration::minutes(1);
        let alerts = alerts.clone();
        let task = tokio::spawn(async move {
            match writer {
                "feed" => nws.apply_feed(&alerts, Some(t1), t1).await.map(|_| ()),
                _ => nws
                    .upsert_zones(
                        &[zone(
                            "forecast/COZ068",
                            ZoneStatus::Ok,
                            Some(square(CO[0], CO[1], CO[2], CO[3])),
                        )],
                        t1,
                    )
                    .await
                    .map(|_| ()),
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(!task.is_finished(), "{writer} must wait for the lock");

        // ...and the writer goes ahead as soon as it is released.
        holder.commit().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .unwrap_or_else(|_| panic!("{writer} never got the lock"))
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn an_older_feed_counts_as_a_poll_but_changes_no_alert() {
    let env = env().await;
    let alerts = fixture_alerts();
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |a| {
        a.id.starts_with("urn:oid")
    });
    let fresh = t0(); // the feed's own `updated`
    env.nws
        .apply_feed(&alerts, Some(fresh), t0())
        .await
        .unwrap();

    // A lagging cache node answers with a feed from three minutes earlier: it does not list the
    // SVR (issued since) and lists an alert that has since gone.
    let mut old: Vec<ParsedAlert> = alerts.iter().filter(|a| a.id != svr).cloned().collect();
    old.push(svr_with_status("urn:test:old-only", Some("Actual")));
    let t1 = t0() + Duration::minutes(1);
    let r = env
        .nws
        .apply_feed(&old, Some(fresh - Duration::minutes(3)), t1)
        .await
        .unwrap();
    assert!(r.stale_ignored);
    assert_eq!((r.upserted, r.ended, r.superseded), (0, 0, 0));
    assert!(
        env.nws
            .get(&svr, false)
            .await
            .unwrap()
            .unwrap()
            .ended_at
            .is_none(),
        "the stale feed must not end an alert it does not list"
    );
    assert!(
        env.nws
            .get("urn:test:old-only", false)
            .await
            .unwrap()
            .is_none(),
        "...nor store what it lists"
    );
    // It was a valid answer, so the data age moves; nothing else about the feed does.
    let st = env.nws.status().await.unwrap().unwrap();
    assert_eq!(st.last_attempt_at, Some(t1));
    assert_eq!(st.last_success_at, Some(t1));
    assert_eq!(st.last_change_at, Some(t0()), "no change was applied");
    assert_eq!(
        st.feed_updated_at,
        Some(fresh),
        "the held stamp does not go backwards"
    );
    assert_eq!(st.last_alert_count, Some(alerts.len() as i32));

    // An empty feed is a fault, never a poll that counts, stale or not.
    let r = env
        .nws
        .apply_feed(
            &[],
            Some(fresh - Duration::minutes(3)),
            t1 + Duration::seconds(30),
        )
        .await
        .unwrap();
    assert!(!r.stale_ignored);
    assert_eq!(
        env.nws.status().await.unwrap().unwrap().last_success_at,
        Some(t1)
    );

    // The same stamp is processed normally (a 200 with unchanged `updated` is still a real answer).
    let t2 = t0() + Duration::minutes(2);
    let r = env.nws.apply_feed(&old, Some(fresh), t2).await.unwrap();
    assert!(!r.stale_ignored);
    assert_eq!((r.upserted, r.unchanged, r.ended), (1, old.len() - 1, 1));
    assert!(env
        .nws
        .get(&svr, false)
        .await
        .unwrap()
        .unwrap()
        .ended_at
        .is_some());
    assert!(env
        .nws
        .get("urn:test:old-only", false)
        .await
        .unwrap()
        .is_some());

    // A newer stamp is processed and moves the held one forward; so is a feed with no stamp.
    let t3 = t0() + Duration::minutes(3);
    let newer = fresh + Duration::minutes(2);
    let r = env.nws.apply_feed(&alerts, Some(newer), t3).await.unwrap();
    assert!(!r.stale_ignored);
    assert_eq!(
        env.nws.status().await.unwrap().unwrap().feed_updated_at,
        Some(newer)
    );
    assert!(env
        .nws
        .get(&svr, false)
        .await
        .unwrap()
        .unwrap()
        .ended_at
        .is_none());
    let r = env
        .nws
        .apply_feed(&alerts, None, t0() + Duration::minutes(4))
        .await
        .unwrap();
    assert!(!r.stale_ignored);

    // Nothing held yet (first poll ever): nothing to be older than.
    sqlx::query("TRUNCATE nws_alerts, nws_alerts_status")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    let r = env
        .nws
        .apply_feed(&alerts, Some(fresh - Duration::days(1)), t0())
        .await
        .unwrap();
    assert!(!r.stale_ignored && r.upserted == alerts.len());
}

async fn zone_row(env: &Env, key: &str) -> (String, bool, DateTime<Utc>) {
    sqlx::query_as(
        "SELECT fetch_status, geom IS NOT NULL, fetched_at FROM nws_zones WHERE zone_key = $1",
    )
    .bind(key)
    .fetch_one(env.catalog.pool())
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_zone_whose_fetch_failed_is_recorded_retried_in_15_minutes_and_never_wipes_geometry() {
    let env = env().await;
    let alerts = fixture_alerts();
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    assert!(env
        .nws
        .missing_zones(t0())
        .await
        .unwrap()
        .contains(&"forecast/COZ068".to_string()));

    // The first fetch of COZ068 times out / 500s: recorded, so the next polls do not ask again...
    let r = env
        .nws
        .upsert_zones(&[zone("forecast/COZ068", ZoneStatus::Error, None)], t0())
        .await
        .unwrap();
    assert_eq!((r.stored, r.invalid_geometry, r.failed), (1, 0, 0));
    assert_eq!(zone_row(&env, "forecast/COZ068").await.0, "error");
    for minute in [0, 1, 5, 14] {
        assert!(
            !env.nws
                .missing_zones(t0() + Duration::minutes(minute))
                .await
                .unwrap()
                .contains(&"forecast/COZ068".to_string()),
            "not asked for again after {minute} min"
        );
    }
    // ...until the retry delay has passed (15 minutes), long before the 24 h of a 404.
    assert!(env
        .nws
        .missing_zones(t0() + Duration::minutes(16))
        .await
        .unwrap()
        .contains(&"forecast/COZ068".to_string()));
    // The alert is still waiting for that zone, not settled on "no geometry".
    assert_eq!(source_of(&env, &winter).await, ("pending".into(), false));

    // The retry succeeds: geometry, status ok, alert resolved.
    let t1 = t0() + Duration::minutes(16);
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t1,
        )
        .await
        .unwrap();
    assert_eq!(zone_row(&env, "forecast/COZ068").await.0, "ok");
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));

    // A refresh (a month on) that fails keeps the geometry we hold and the alerts that use it,
    // and is retried after 15 minutes, not after another month.
    let t2 = t1 + Duration::days(31);
    assert!(env
        .nws
        .missing_zones(t2)
        .await
        .unwrap()
        .contains(&"forecast/COZ068".to_string()));
    env.nws
        .upsert_zones(&[zone("forecast/COZ068", ZoneStatus::Error, None)], t2)
        .await
        .unwrap();
    let (status, has_geom, fetched_at) = zone_row(&env, "forecast/COZ068").await;
    assert_eq!((status.as_str(), has_geom, fetched_at), ("error", true, t2));
    assert_eq!(
        source_of(&env, &winter).await,
        ("zones".into(), true),
        "the stale geometry still serves the alert"
    );
    let hit = env
        .nws
        .active_at_point(-107.0, 37.5, t0() + Duration::hours(7), false)
        .await
        .unwrap();
    assert!(ids(&hit).contains(&winter));
    assert!(!env
        .nws
        .missing_zones(t2 + Duration::minutes(14))
        .await
        .unwrap()
        .contains(&"forecast/COZ068".to_string()));
    assert!(env
        .nws
        .missing_zones(t2 + Duration::minutes(16))
        .await
        .unwrap()
        .contains(&"forecast/COZ068".to_string()));

    // Other outcomes keep their own schedule: 404 waits a day, ok waits a month.
    env.nws
        .upsert_zones(&[zone("county/MDC031", ZoneStatus::NotFound, None)], t2)
        .await
        .unwrap();
    assert!(!env
        .nws
        .missing_zones(t2 + Duration::hours(23))
        .await
        .unwrap()
        .contains(&"county/MDC031".to_string()));
    assert!(env
        .nws
        .missing_zones(t2 + Duration::hours(25))
        .await
        .unwrap()
        .contains(&"county/MDC031".to_string()));
}

/// Every read the EDR API uses is cut off at the catalog's read timeout and fails as
/// `ServiceUnavailable` (SQLSTATE 57014), the limit ends with the read instead of staying on the
/// pooled connection, and the writers never see it.
/// The pool is shared with the API's other collections. A read that waits on a table lock holds a
/// connection, and so does one whose client hung up (Postgres is not told, the statement runs on
/// until the time limit): enough of them hold every connection. With the read slots the burst
/// stops short of the whole pool and leaves connections for everyone else, and the slots are
/// given back once the abandoned reads have ended.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_burst_of_alert_reads_cannot_take_every_connection_of_the_pool() {
    let env = env().await;
    // a pool of four stands for the API's ten
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with((*env.catalog.pool().connect_options()).clone())
        .await
        .unwrap();
    let limit = std::time::Duration::from_secs(2);
    let ms = std::time::Duration::from_millis;

    // Another session holds the alert tables, so every read queues in Postgres until the limit.
    let mut holder = env.catalog.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE nws_alerts, nws_alerts_status IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();

    // (read slots, whether the rest of the pool is starved). Four slots on four connections is
    // what there was before the slots; the default for this pool is two.
    let mut nws = NwsAlertCatalog::new(pool.clone());
    for (slots, starved) in [(4, true), (2, false)] {
        nws = NwsAlertCatalog::new(pool.clone())
            .with_read_timeout(limit)
            .with_read_slots(slots);
        let spawn_reads = |n: usize| -> Vec<tokio::task::JoinHandle<_>> {
            (0..n)
                .map(|_| {
                    let nws = nws.clone();
                    tokio::spawn(async move { nws.status().await })
                })
                .collect()
        };
        // eight reads, from clients that hang up after a moment...
        let abandoned = spawn_reads(8);
        tokio::time::sleep(ms(300)).await;
        for read in &abandoned {
            read.abort();
        }
        // ...and four more that arrive after that, while Postgres is still on the first ones
        let waiting = spawn_reads(4);
        tokio::time::sleep(ms(300)).await;
        let other = tokio::time::timeout(ms(1000), pool.acquire()).await;
        assert_eq!(other.is_err(), starved, "{slots} read slots");
        drop(other);
        for read in &waiting {
            read.abort();
        }
        // Postgres cancels the abandoned statements at the limit; their slots come back then
        tokio::time::sleep(limit + ms(1000)).await;
    }
    holder.rollback().await.unwrap();
    nws.status()
        .await
        .expect("the slots were given back once the abandoned reads ended");

    // The time limit covers the wait for a slot: with the one slot taken, the read gives up
    // after the limit, as a 503 (ServiceUnavailable), instead of queueing for ever.
    let nws = NwsAlertCatalog::new(pool.clone())
        .with_read_timeout(ms(400))
        .with_read_slots(1);
    let mut holder = env.catalog.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE nws_alerts_status IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let first = tokio::spawn({
        let nws = nws.clone();
        async move { nws.status().await }
    });
    tokio::time::sleep(ms(100)).await;
    let started = std::time::Instant::now();
    let second = nws.status().await.unwrap_err();
    assert!(
        matches!(second, wms_common::WmsError::ServiceUnavailable(_)),
        "{second:?}"
    );
    assert!(started.elapsed() < ms(1500), "{:?}", started.elapsed());
    assert!(matches!(
        first.await.unwrap().unwrap_err(),
        wms_common::WmsError::ServiceUnavailable(_)
    ));
    holder.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn reads_are_cut_off_at_the_read_timeout_and_the_limit_does_not_leak() {
    use wms_common::WmsError;

    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    let id = alerts[0].id.clone();
    let reader = NwsAlertCatalog::new(env.catalog.pool_clone())
        .with_read_timeout(std::time::Duration::from_millis(300));

    // Somebody holds both tables exclusively: every read queues behind the lock until the
    // timeout cancels it.
    let mut holder = env.catalog.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE nws_alerts, nws_alerts_status IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let region = |r| search(r, t0());
    let bbox = [-108.0, 36.0, -106.0, 39.0];
    let ring = "POLYGON((-108 36, -106 36, -106 39, -108 39, -108 36))".to_string();
    let started = std::time::Instant::now();
    let results = [
        ("status", reader.status().await.err()),
        (
            "point",
            reader
                .active_at_point(-107.0, 37.5, t0(), false)
                .await
                .err(),
        ),
        (
            "bbox",
            reader
                .in_bbox(bbox, t0(), t0(), true, false, t0(), 10, false)
                .await
                .err(),
        ),
        (
            "search point",
            reader
                .search(&region(AlertRegion::Point {
                    lon: -107.0,
                    lat: 37.5,
                }))
                .await
                .err(),
        ),
        (
            "search bbox",
            reader.search(&region(AlertRegion::Bbox(bbox))).await.err(),
        ),
        (
            "search polygon",
            reader
                .search(&region(AlertRegion::Polygon { wkt: ring.clone() }))
                .await
                .err(),
        ),
        (
            "search radius",
            reader
                .search(&region(AlertRegion::Radius {
                    lon: -107.0,
                    lat: 37.5,
                    meters: 50_000.0,
                }))
                .await
                .err(),
        ),
        ("get", reader.get(&id, true).await.err()),
    ];
    for (what, err) in results {
        assert!(
            matches!(err, Some(WmsError::ServiceUnavailable(_))),
            "{what}: {err:?}"
        );
    }
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(8 * 250),
        "each read waited out its limit"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "and none of them hung"
    );
    holder.rollback().await.unwrap();

    // The lock is gone: the same reads answer, and the limit left no trace on any connection.
    assert!(
        reader.status().await.unwrap().is_some(),
        "the poll was recorded"
    );
    assert!(reader.get(&id, false).await.unwrap().is_some());
    let hits = reader
        .search(&region(AlertRegion::Polygon { wkt: ring }))
        .await
        .unwrap();
    assert!(hits.len() <= 100);
    let mut held = Vec::new();
    for _ in 0..10 {
        held.push(env.catalog.pool().acquire().await.unwrap());
    }
    for conn in &mut held {
        let t: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        assert_eq!(t, "0", "no pooled connection kept the read timeout");
    }
    drop(held);

    // Writers share the pool but not the limit: a feed poll under a 1 ms read timeout is fine.
    let strict = NwsAlertCatalog::new(env.catalog.pool_clone())
        .with_read_timeout(std::time::Duration::from_millis(1));
    let t1 = t0() + Duration::minutes(1);
    let applied = strict.apply_feed(&alerts, Some(t1), t1).await.unwrap();
    assert_eq!(applied.failed, 0);
    assert_eq!(applied.received, alerts.len());
}

/// A bowtie is not a valid polygon; the guard (`ST_MakeValid`, once per query) repairs it and the
/// search still finds what overlaps either lobe.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_polygon_search_repairs_a_bowtie_and_matches_its_lobes() {
    let env = env().await;
    let alerts = fixture_alerts();
    env.nws.apply_feed(&alerts, Some(t0()), t0()).await.unwrap();
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);

    // Colorado zone is lon -107.5..-106.5, lat 37..38. The lobes meet at (-107, 38.5) and
    // cover its upper-left and lower-right.
    let bowtie = "POLYGON((-108 37, -107 38.5, -108 40, -106 40, -107 38.5, -106 37, -108 37))";
    let hit = env
        .nws
        .search(&search(AlertRegion::Polygon { wkt: bowtie.into() }, t0()))
        .await
        .unwrap();
    assert!(ids(&hit).contains(&winter), "{:?}", ids(&hit));
    let miss = env
        .nws
        .search(&search(
            AlertRegion::Polygon {
                wkt: "POLYGON((10 10, 11 10, 11 11, 10 10))".into(),
            },
            t0(),
        ))
        .await
        .unwrap();
    assert!(miss.is_empty());
}

// --- an explicit instant: what was in force then ---------------------------------------------

/// 2026-11-01 `h:m`Z, the day of the fixture.
fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, h, m, 0).unwrap()
}

/// A copy of the fixture's Severe Thunderstorm Warning under `id` with its own polygon
/// (`[west, south, east, north]`) and times: issued at `sent`, hazard from `onset` to `ends`,
/// replacing the messages in `replaces` (then an `Update`).
fn timed_alert(
    id: &str,
    bbox: [f64; 4],
    sent: DateTime<Utc>,
    onset: DateTime<Utc>,
    ends: DateTime<Utc>,
    replaces: &[&str],
) -> ParsedAlert {
    parse_feature(&timed_feature(id, bbox, sent, onset, ends, replaces)).unwrap()
}

/// [`timed_alert`] as the NWS feature, for a test that changes more of it.
fn timed_feature(
    id: &str,
    bbox: [f64; 4],
    sent: DateTime<Utc>,
    onset: DateTime<Utc>,
    ends: DateTime<Utc>,
    replaces: &[&str],
) -> Value {
    let mut f = features()
        .into_iter()
        .find(|f| f["properties"]["event"] == "Severe Thunderstorm Warning")
        .unwrap();
    f["geometry"] = serde_json::from_str(&square(bbox[0], bbox[1], bbox[2], bbox[3])).unwrap();
    let p = &mut f["properties"];
    p["id"] = json!(id);
    p["sent"] = json!(sent.to_rfc3339());
    p["effective"] = json!(sent.to_rfc3339());
    p["onset"] = json!(onset.to_rfc3339());
    p["ends"] = json!(ends.to_rfc3339());
    p["expires"] = json!(ends.to_rfc3339());
    p["messageType"] = json!(if replaces.is_empty() {
        "Alert"
    } else {
        "Update"
    });
    p["references"] = json!(replaces
        .iter()
        .map(|r| json!({"identifier": r, "sent": sent.to_rfc3339()}))
        .collect::<Vec<_>>());
    f
}

/// A `Cancel` of `cancels`, issued at `sent` and good until `ends`, as NWS sends one: with the
/// cancelled alert's polygon.
fn timed_cancel(
    id: &str,
    bbox: [f64; 4],
    sent: DateTime<Utc>,
    ends: DateTime<Utc>,
    cancels: &str,
) -> ParsedAlert {
    let mut f = timed_feature(id, bbox, sent, sent, ends, &[cancels]);
    f["properties"]["messageType"] = json!("Cancel");
    parse_feature(&f).unwrap()
}

// Both squares lie in lon -107.5..-106.5; the small one covers only the eastern half.
const BIG: [f64; 4] = [-107.5, 37.0, -106.5, 38.0];
const SMALL: [f64; 4] = [-107.0, 37.0, -106.5, 38.0];
/// In `BIG` only.
const P: (f64, f64) = (-107.25, 37.5);
/// In both.
const Q: (f64, f64) = (-106.75, 37.5);

struct Timeline {
    /// Sent 10:00, hazard from 10:30 until 18:00, replaced at 11:00 by `a2` (smaller polygon).
    a: String,
    a2: String,
    /// Sent 10:00 until 18:00, dropped from the feed at the 12:00 poll.
    b: String,
}

/// Three polls: 10:01 shows A and B; 11:01 shows A2 (an update of A, sent 11:00) and B; 12:00
/// shows A2 alone.
async fn timeline(env: &Env) -> Timeline {
    let a = timed_alert("urn:test:a", BIG, at(10, 0), at(10, 30), at(18, 0), &[]);
    let b = timed_alert("urn:test:b", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    let a2 = timed_alert(
        "urn:test:a2",
        SMALL,
        at(11, 0),
        at(11, 0),
        at(18, 0),
        &["urn:test:a"],
    );
    let polls: [(DateTime<Utc>, Vec<ParsedAlert>); 3] = [
        (at(10, 1), vec![a.clone(), b.clone()]),
        (at(11, 1), vec![a2.clone(), b.clone()]),
        (at(12, 0), vec![a2.clone()]),
    ];
    for (now, feed) in polls {
        env.nws.apply_feed(&feed, Some(now), now).await.unwrap();
    }
    Timeline {
        a: a.id,
        a2: a2.id,
        b: b.id,
    }
}

fn point(p: (f64, f64)) -> AlertRegion {
    AlertRegion::Point { lon: p.0, lat: p.1 }
}

fn bbox_around(p: (f64, f64)) -> AlertRegion {
    AlertRegion::Bbox([p.0 - 0.01, p.1 - 0.01, p.0 + 0.01, p.1 + 0.01])
}

fn polygon_around(p: (f64, f64)) -> AlertRegion {
    let (x, y, d) = (p.0, p.1, 0.01);
    AlertRegion::Polygon {
        wkt: format!(
            "POLYGON(({} {}, {} {}, {} {}, {} {}, {} {}))",
            x - d,
            y - d,
            x + d,
            y - d,
            x + d,
            y + d,
            x - d,
            y + d,
            x - d,
            y - d
        ),
    }
}

fn radius_around(p: (f64, f64)) -> AlertRegion {
    AlertRegion::Radius {
        lon: p.0,
        lat: p.1,
        meters: 500.0,
    }
}

fn sorted(mut v: Vec<&str>) -> Vec<String> {
    v.sort();
    v.into_iter().map(str::to_string).collect()
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn an_explicit_instant_returns_the_messages_in_force_then() {
    let env = env().await;
    let Timeline { a, a2, b } = timeline(&env).await;
    let (a, a2, b) = (a.as_str(), a2.as_str(), b.as_str());

    // What the poll records: A is over from the moment its replacement was issued (11:00), not
    // from the poll that first showed it (11:01); B from the poll that no longer listed it.
    let a_row = env.nws.get(a, false).await.unwrap().unwrap();
    assert_eq!(a_row.superseded_by.as_deref(), Some(a2));
    assert_eq!(a_row.superseded_at, Some(at(11, 0)));
    assert_eq!(a_row.ended_at, None);
    let b_row = env.nws.get(b, false).await.unwrap().unwrap();
    assert_eq!(
        (b_row.superseded_by, b_row.ended_at),
        (None, Some(at(12, 0)))
    );

    // (instant, at P (BIG only), at Q (BIG and SMALL)). The row's own life is [issue, end): the
    // instant it is issued counts, the instant it is replaced / ended / expires does not.
    let cases: [(DateTime<Utc>, Vec<&str>, Vec<&str>); 13] = [
        (at(9, 30), vec![], vec![]),          // before anything was issued
        (at(10, 0), vec![a, b], vec![a, b]),  // T1: issued
        (at(10, 15), vec![a, b], vec![a, b]), // A's hazard starts at 10:30
        (at(10, 30), vec![a, b], vec![a, b]), // T2
        (at(10, 45), vec![a, b], vec![a, b]), // before the update: A is still there
        (at(10, 59), vec![a, b], vec![a, b]), // ...and the poll has not even run
        (at(11, 0), vec![b], vec![a2, b]),    // T3: replaced; SMALL no longer covers P
        (at(11, 30), vec![b], vec![a2, b]),   // B is in force until T4
        (at(11, 59), vec![b], vec![a2, b]),   //
        (at(12, 0), vec![], vec![a2]),        // T4: B ended
        (at(12, 30), vec![], vec![a2]),       //
        (at(17, 59), vec![], vec![a2]),       //
        (at(18, 0), vec![], vec![]),          // the end time is exclusive
    ];
    for (t, at_p, at_q) in cases {
        for (name, region) in [
            ("point", point as fn((f64, f64)) -> AlertRegion),
            ("bbox", bbox_around),
            ("polygon", polygon_around),
            ("radius", radius_around),
        ] {
            for (place, xy, want) in [("P", P, &at_p), ("Q", Q, &at_q)] {
                let hits = env
                    .nws
                    .search(&search(region(xy), t0()).in_force_at(t))
                    .await
                    .unwrap();
                let got: Vec<String> = ids(&hits)
                    .into_iter()
                    .map(|i| i.trim_start_matches("urn:test:").to_string())
                    .collect();
                let want: Vec<String> = sorted(
                    want.iter()
                        .map(|i| i.trim_start_matches("urn:test:"))
                        .collect(),
                );
                assert_eq!(got, want, "{name} at {place}, instant {t}");
            }
        }
    }

    // The instant also reaches the status filters unchanged: severity / event / limit.
    let mut only_extreme = search(point(Q), t0()).in_force_at(at(11, 30));
    only_extreme.severities = vec!["extreme".into()];
    assert!(env.nws.search(&only_extreme).await.unwrap().is_empty());
    let mut one = search(point(Q), t0()).in_force_at(at(11, 30));
    one.limit = 1;
    assert_eq!(env.nws.search(&one).await.unwrap().len(), 1);

    // A replacement issued after the poll that shows it (its clock is ahead) cannot make the old
    // message "replaced in the future": the poll time caps it.
    let c = timed_alert("urn:test:c", BIG, at(13, 0), at(13, 0), at(18, 0), &[]);
    let c2 = timed_alert(
        "urn:test:c2",
        SMALL,
        at(14, 0),
        at(14, 0),
        at(18, 0),
        &["urn:test:c"],
    );
    env.nws
        .apply_feed(&[c], Some(at(13, 1)), at(13, 1))
        .await
        .unwrap();
    env.nws
        .apply_feed(&[c2], Some(at(13, 2)), at(13, 2))
        .await
        .unwrap();
    let c_row = env.nws.get("urn:test:c", false).await.unwrap().unwrap();
    assert_eq!(c_row.superseded_at, Some(at(13, 2)));
}

/// A message that is over when it arrives (a `Cancel`, or an update whose every VTEC action is CAN)
/// has no life: issued and over at the same moment, whatever time passes before a poll stores it.
/// The warning it cancels is out of force from that moment too, so until the poll the answer at
/// that instant is nothing, not the cancellation presented as an active alert.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_cancellation_is_never_in_force_not_even_before_the_poll_that_stored_it() {
    let env = env().await;
    let a = timed_alert("urn:test:a", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    let a2 = timed_alert("urn:test:a2", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    // A is cancelled at 11:00 by a message the 11:01:10 poll is the first to list...
    let c = timed_cancel("urn:test:c", BIG, at(11, 0), at(11, 30), "urn:test:a");
    // ...and A2 at 12:00 by an Update whose only VTEC action is CAN, listed at 12:01:10.
    let mut d = timed_feature(
        "urn:test:d",
        BIG,
        at(12, 0),
        at(12, 0),
        at(18, 0),
        &["urn:test:a2"],
    );
    d["properties"]["parameters"]["VTEC"] =
        json!(["/O.CAN.KFGZ.SV.W.0001.000000T0000Z-261101T1800Z/"]);
    let d = parse_feature(&d).unwrap();
    assert!(c.ended_on_arrival && d.ended_on_arrival);

    let late = |h, m| at(h, m) + Duration::seconds(70);
    for (feed, now) in [
        (vec![a, a2.clone()], at(10, 1)),
        (vec![c, a2], late(11, 0)),
        (vec![d], late(12, 0)),
    ] {
        env.nws.apply_feed(&feed, Some(now), now).await.unwrap();
    }

    // They end when issued, not when a poll first showed them.
    let row = |id: &'static str| {
        let nws = env.nws.clone();
        async move { nws.get(id, false).await.unwrap().unwrap() }
    };
    assert_eq!(row("urn:test:c").await.ended_at, Some(at(11, 0)));
    assert_eq!(row("urn:test:d").await.ended_at, Some(at(12, 0)));
    assert_eq!(row("urn:test:a").await.superseded_at, Some(at(11, 0)));
    assert_eq!(row("urn:test:a2").await.superseded_at, Some(at(12, 0)));

    let cases: [(DateTime<Utc>, Vec<&str>); 9] = [
        (at(10, 59), vec!["a", "a2"]),
        (at(11, 0), vec!["a2"]), // A replaced; C is over as it is issued
        (at(11, 0) + Duration::seconds(30), vec!["a2"]), // before the poll that stored C
        (late(11, 0), vec!["a2"]), // the poll itself
        (at(11, 59), vec!["a2"]), // C's end time is long past, A2 remains
        (at(12, 0), vec![]),     // A2 replaced by D, itself over at once
        (at(12, 0) + Duration::seconds(30), vec![]), // before the poll that stored D
        (late(12, 0), vec![]),
        (at(17, 59), vec![]),
    ];
    for (t, want) in cases {
        for (name, region) in [
            ("point", point as fn((f64, f64)) -> AlertRegion),
            ("bbox", bbox_around),
            ("polygon", polygon_around),
            ("radius", radius_around),
        ] {
            let hits = env
                .nws
                .search(&search(region(P), t0()).in_force_at(t))
                .await
                .unwrap();
            let got: Vec<String> = ids(&hits)
                .into_iter()
                .map(|i| i.trim_start_matches("urn:test:").to_string())
                .collect();
            assert_eq!(got, sorted(want.clone()), "{name} at {t}");
        }
    }

    // A window over the cancellation still lists it: it happened, it just was never in force.
    let mut history = search(point(P), t0());
    history.window = Some((at(10, 30), at(12, 30)));
    history.include_superseded = true;
    let hits = env.nws.search(&history).await.unwrap();
    assert_eq!(
        ids(&hits),
        ["urn:test:a", "urn:test:a2", "urn:test:c", "urn:test:d"]
    );
}

/// Each update names the whole chain before it, and NWS lists the newest message first, so a poll
/// after a gap can show two replacements of one message in the wrong order. The one issued first
/// ends the old message; otherwise the old and the first update would both be in force in between.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn the_replacement_issued_first_ends_the_old_message_whatever_the_feed_order() {
    let env = env().await;
    let a = timed_alert("urn:test:a", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    let u1 = timed_alert(
        "urn:test:u1",
        SMALL,
        at(11, 0),
        at(11, 0),
        at(18, 0),
        &["urn:test:a"],
    );
    let u2 = timed_alert(
        "urn:test:u2",
        SMALL,
        at(11, 30),
        at(11, 30),
        at(18, 0),
        &["urn:test:a", "urn:test:u1"],
    );
    env.nws
        .apply_feed(&[a], Some(at(10, 1)), at(10, 1))
        .await
        .unwrap();
    // the ingester was down from 10:01 to 11:45: one poll shows both updates, newest first
    env.nws
        .apply_feed(&[u2, u1], Some(at(11, 45)), at(11, 45))
        .await
        .unwrap();

    let row = |id: &'static str| {
        let nws = env.nws.clone();
        async move { nws.get(id, false).await.unwrap().unwrap() }
    };
    let a = row("urn:test:a").await;
    assert_eq!(a.superseded_by.as_deref(), Some("urn:test:u1"));
    assert_eq!(a.superseded_at, Some(at(11, 0)));
    let u1 = row("urn:test:u1").await;
    assert_eq!(u1.superseded_by.as_deref(), Some("urn:test:u2"));
    assert_eq!(u1.superseded_at, Some(at(11, 30)));

    for (t, want) in [
        (at(10, 59), vec!["a"]),
        (at(11, 0), vec!["u1"]),
        (at(11, 15), vec!["u1"]), // not the old message too
        (at(11, 30), vec!["u2"]),
        (at(11, 44), vec!["u2"]),
    ] {
        let hits = env
            .nws
            .search(&search(point(Q), t0()).in_force_at(t))
            .await
            .unwrap();
        let got: Vec<String> = ids(&hits)
            .into_iter()
            .map(|i| i.trim_start_matches("urn:test:").to_string())
            .collect();
        assert_eq!(got, sorted(want), "at {t}");
    }
}

/// A search reads rows only until their geometry text outgrows `max_geometry_bytes`: the row that
/// crosses the line is the last one returned, so a caller holds the budget plus one row at most.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_search_stops_reading_rows_once_their_geometry_outgrows_the_budget() {
    let env = env().await;
    let feed: Vec<ParsedAlert> = (0..5)
        .map(|i| {
            timed_alert(
                &format!("urn:test:g{i}"),
                BIG,
                at(10, i),
                at(10, i),
                at(23, 59),
                &[],
            )
        })
        .collect();
    env.nws.apply_feed(&feed, Some(t0()), t0()).await.unwrap();
    let with_geometry = |budget: Option<usize>| {
        let mut q = search(bbox_around(P), t0());
        q.with_geometry = true;
        q.max_geometry_bytes = budget;
        q
    };
    let all = env.nws.search(&with_geometry(None)).await.unwrap();
    assert_eq!(all.len(), 5);
    let one = all[0].geometry_geojson.as_ref().unwrap().len();
    assert!(all
        .iter()
        .all(|a| a.geometry_geojson.as_ref().unwrap().len() == one));

    // (budget, rows returned): the row that crosses the budget is returned, the rest are not read
    for (budget, want) in [
        (Some(1), 1),
        (Some(one - 1), 1),
        (Some(one), 2),
        (Some(2 * one), 3),
        (Some(4 * one), 5),
        (Some(5 * one), 5),
        (Some(usize::MAX), 5),
        (None, 5),
    ] {
        let rows = env.nws.search(&with_geometry(budget)).await.unwrap();
        assert_eq!(rows.len(), want, "budget {budget:?}");
        assert_eq!(ids(&rows), ids(&all[..want]), "the first ones in order");
    }
    // the connection that was left with unread rows is usable and clean
    assert_eq!(
        env.nws.search(&with_geometry(Some(1))).await.unwrap().len(),
        1
    );
    assert_eq!(env.nws.search(&with_geometry(None)).await.unwrap().len(), 5);
}

/// A search without a window is what it always was: the lifecycle flags at `now`, on the partial
/// index. The explicit-instant work must not move either.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_search_without_a_datetime_is_the_live_set_on_the_partial_index() {
    let env = env().await;
    // Replaced (A) and ended (B) rows from the timeline, plus one the feed still lists after its
    // end time (D) and one issued whose hazard is hours away (LATER), both from the 12:01 poll.
    let tl = timeline(&env).await;
    let d = timed_alert("urn:test:d", BIG, at(9, 0), at(9, 0), at(11, 15), &[]);
    let later = timed_alert("urn:test:later", BIG, at(12, 0), at(20, 0), at(23, 0), &[]);
    let a2 = timed_alert(&tl.a2, SMALL, at(11, 0), at(11, 0), at(18, 0), &[&tl.a]);
    env.nws
        .apply_feed(&[a2, d, later], Some(at(12, 1)), at(12, 1))
        .await
        .unwrap();

    // The same question asked the way it was asked before: the flags and the end time only.
    for now in [
        at(9, 0),
        at(10, 30),
        at(11, 30),
        at(12, 30),
        at(17, 59),
        at(18, 0),
        at(21, 0),
        t0(),
    ] {
        for xy in [P, Q, (-100.0, 40.0)] {
            let legacy: Vec<String> = sqlx::query_scalar(
                "SELECT a.id FROM nws_alerts a \
                 WHERE a.superseded_by IS NULL AND a.ended_at IS NULL \
                   AND (a.end_time IS NULL OR a.end_time > $1) \
                   AND COALESCE(lower(a.status), 'actual') = 'actual' \
                   AND a.geom && ST_SetSRID(ST_MakePoint($2, $3), 4326) \
                   AND ST_Intersects(a.geom, ST_SetSRID(ST_MakePoint($2, $3), 4326)) \
                 ORDER BY a.id",
            )
            .bind(now)
            .bind(xy.0)
            .bind(xy.1)
            .fetch_all(env.catalog.pool())
            .await
            .unwrap();
            let live = env.nws.search(&search(point(xy), now)).await.unwrap();
            assert_eq!(ids(&live), legacy, "live set at {now} for {xy:?}");
            let by_point = env
                .nws
                .active_at_point(xy.0, xy.1, now, false)
                .await
                .unwrap();
            assert_eq!(
                ids(&by_point),
                legacy,
                "active_at_point at {now} for {xy:?}"
            );
        }
    }
    // Not time travel: at 10:30 the flags already hide A (replaced since) and B (ended since),
    // and show D (over at 11:15, but the 10:30 end-time test passes) and LATER (not issued yet).
    let q_live = env.nws.search(&search(point(Q), at(10, 30))).await.unwrap();
    assert_eq!(
        ids(&q_live),
        vec!["urn:test:a2", "urn:test:d", "urn:test:later"]
    );

    // The plan, as Postgres would run the prepared statement (a generic plan, no parameter
    // values): the wall-clock reads use the small partial GiST index of the live rows; the
    // instant read must also see replaced and ended rows, so it uses the full one.
    for region in [point(Q), bbox_around(Q)] {
        let live = plan_of(&env, &search(region.clone(), t0())).await;
        assert!(live.contains("idx_nws_alerts_geom_live"), "{live}");
        let then = plan_of(&env, &search(region, t0()).in_force_at(at(11, 30))).await;
        assert!(then.contains("idx_nws_alerts_geom"), "{then}");
        assert!(!then.contains("idx_nws_alerts_geom_live"), "{then}");
    }
}

/// How Postgres plans `search_sql(q)` with sequential scans off (the tables here are tiny).
async fn plan_of(env: &Env, q: &AlertSearch) -> String {
    use sqlx::Row;
    let sql = format!(
        "BEGIN; SET LOCAL enable_seqscan = off; EXPLAIN (GENERIC_PLAN) {}; ROLLBACK",
        storage::nws_alerts::catalog::search_sql(q)
    );
    let mut conn = env.catalog.pool().acquire().await.unwrap();
    let rows = sqlx::raw_sql(&sql).fetch_all(&mut *conn).await.unwrap();
    rows.iter()
        .map(|r| r.get::<String, _>(0))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A copy of [`timed_alert`] (in force 10:00-18:00) with its own GeoJSON geometry.
fn alert_with_geometry(id: &str, geometry: Value) -> ParsedAlert {
    let mut a = timed_alert(id, BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    a.geometry_json = Some(geometry.to_string());
    a
}

/// NWS splits zones that touch the antimeridian at +-180: the Bering Sea offshore zones are one
/// polygon on the 171E-180 side (PKZ411) and another on the -180 to 171W side (PKZ412), and an
/// alert for both is a MultiPolygon with a part on each side.
async fn dateline_alerts(env: &Env) -> Vec<String> {
    let polygons: [(&str, [f64; 4]); 9] = [
        ("pkz411", [171.5, 56.0, 180.0, 59.0]),
        ("pkz412", [-180.0, 56.0, -171.0, 59.0]),
        ("adak-w", [172.0, 51.0, 180.0, 54.0]),
        ("fiji-e", [176.0, -20.0, 180.0, -15.0]),
        ("fiji-w", [-180.0, -20.0, -175.0, -15.0]),
        ("florida", [-82.0, 25.0, -80.0, 27.0]),
        ("pole-0", [0.0, 86.0, 10.0, 89.0]),
        ("pole-180", [170.0, 86.0, 175.0, 89.0]),
        ("pole-w", [-100.0, 86.0, -90.0, 89.0]),
    ];
    let mut feed: Vec<ParsedAlert> = polygons
        .iter()
        .map(|(name, b)| {
            alert_with_geometry(
                &format!("urn:test:{name}"),
                serde_json::from_str(&square(b[0], b[1], b[2], b[3])).unwrap(),
            )
        })
        .collect();
    // one alert for the union of two zones, split at the line
    let part = |b: [f64; 4]| {
        let g: Value = serde_json::from_str(&square(b[0], b[1], b[2], b[3])).unwrap();
        g["coordinates"].clone()
    };
    feed.push(alert_with_geometry(
        "urn:test:saint-matthew",
        json!({"type": "MultiPolygon", "coordinates": [
            part([176.0, 60.0, 180.0, 62.0]), part([-180.0, 60.0, -172.0, 62.0])
        ]}),
    ));
    env.nws
        .apply_feed(&feed, Some(at(10, 1)), at(10, 1))
        .await
        .unwrap();
    feed.into_iter().map(|a| a.id).collect()
}

/// Ids the way the database itself answers a distance question, with no index prefilter.
async fn within_oracle(env: &Env, lon: f64, lat: f64, meters: f64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT id FROM nws_alerts WHERE ST_DWithin(geom::geography, \
         ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography, $3) ORDER BY id",
    )
    .bind(lon)
    .bind(lat)
    .bind(meters)
    .fetch_all(env.catalog.pool())
    .await
    .unwrap()
}

/// A radius search at the antimeridian misses nothing: alerts on the far side of +-180 used to be
/// dropped by the planar prefilter before the geodetic distance was ever asked.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_radius_search_finds_alerts_on_both_sides_of_the_antimeridian() {
    let env = env().await;
    dateline_alerts(&env).await;
    let now = at(12, 0);
    let radius = |lon: f64, lat: f64, km: f64| {
        search(
            AlertRegion::Radius {
                lon,
                lat,
                meters: km * 1000.0,
            },
            now,
        )
    };

    // 3 km from the line, 50 km: PKZ411 holds the centre and PKZ412 starts at -180
    for (lon, expect) in [
        (179.95, true),
        (-179.95, true),
        (180.0, true),
        (-180.0, true),
    ] {
        let got = ids(&env.nws.search(&radius(lon, 57.5, 50.0)).await.unwrap());
        assert_eq!(
            got,
            vec!["urn:test:pkz411", "urn:test:pkz412"],
            "centre {lon}: both zones, whichever side the centre is on ({expect})"
        );
    }
    // Adak (-176.64, 51.88) is on the east side; adak-w ends at the line, 231 km west of it
    let got = ids(&env
        .nws
        .search(&radius(-176.64, 51.88, 300.0))
        .await
        .unwrap());
    assert_eq!(got, vec!["urn:test:adak-w"]);
    // 100 km is not far enough for it
    let got = ids(&env
        .nws
        .search(&radius(-176.64, 51.88, 100.0))
        .await
        .unwrap());
    assert!(got.is_empty(), "{got:?}");
    // the union alert with a part on each side is found from either
    for lon in [179.0, -179.0] {
        let got = ids(&env.nws.search(&radius(lon, 61.0, 20.0)).await.unwrap());
        assert_eq!(got, vec!["urn:test:saint-matthew"], "{lon}");
    }
    // the southern hemisphere too
    let got = ids(&env.nws.search(&radius(-179.9, -17.0, 30.0)).await.unwrap());
    assert_eq!(got, vec!["urn:test:fiji-e", "urn:test:fiji-w"]);
    // and nothing that is not within the radius is added
    assert!(env
        .nws
        .search(&radius(179.95, 40.0, 50.0))
        .await
        .unwrap()
        .is_empty());

    // Against the database's own distance, for centres and radii all around the line, in the
    // far south, and at the poles.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = |lo: f64, hi: f64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        lo + (state >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    };
    let (mut both_sides, mut found) = (0, 0);
    for i in 0..400 {
        let lon = if i % 2 == 0 {
            next(165.0, 180.0)
        } else {
            next(-180.0, -160.0)
        };
        let lat = match i % 4 {
            0 => next(-25.0, -10.0),
            1 => next(48.0, 64.0),
            2 => next(80.0, 90.0),
            _ => next(-60.0, 70.0),
        };
        let km = [1.0, 10.0, 50.0, 100.0, 300.0, 500.0][i % 6];
        let want = within_oracle(&env, lon, lat, km * 1000.0).await;
        let got = ids(&env.nws.search(&radius(lon, lat, km)).await.unwrap());
        assert_eq!(got, want, "({lon}, {lat}) within {km} km");
        found += !want.is_empty() as usize;
        both_sides += (want
            .iter()
            .any(|i| i.contains("pkz411") || i.contains("adak-w"))
            && want.iter().any(|i| i.contains("pkz412"))) as usize;
    }
    assert!(
        found > 100 && both_sides > 5,
        "{found} {both_sides}: the sweep should hit things"
    );

    // and the pole: a circle around it reaches every longitude
    let want = within_oracle(&env, 0.0, 89.9, 500_000.0).await;
    assert!(
        want.contains(&"urn:test:pole-180".to_string())
            && want.contains(&"urn:test:pole-w".to_string())
    );
    let got = ids(&env.nws.search(&radius(0.0, 89.9, 500.0)).await.unwrap());
    assert_eq!(got, want);
}

/// A bbox with west > east is the two boxes either side of +-180.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_bbox_with_west_above_east_covers_both_sides_of_the_antimeridian() {
    let env = env().await;
    dateline_alerts(&env).await;
    let now = at(12, 0);
    let bbox = |w, s, e, n| search(AlertRegion::Bbox([w, s, e, n]), now);

    let got = ids(&env
        .nws
        .search(&bbox(170.0, 50.0, -170.0, 59.5))
        .await
        .unwrap());
    assert_eq!(
        got,
        vec!["urn:test:adak-w", "urn:test:pkz411", "urn:test:pkz412"],
        "both sides, not Florida, not the far-south zones, not the 60-62N union"
    );
    // each half alone sees only its side; the ordinary boxes are unchanged
    assert_eq!(
        ids(&env
            .nws
            .search(&bbox(170.0, 50.0, 180.0, 59.5))
            .await
            .unwrap()),
        vec!["urn:test:adak-w", "urn:test:pkz411"]
    );
    assert_eq!(
        ids(&env
            .nws
            .search(&bbox(-180.0, 50.0, -170.0, 59.5))
            .await
            .unwrap()),
        vec!["urn:test:pkz412"]
    );
    // the union alert is returned once, whichever parts the box meets
    assert_eq!(
        ids(&env
            .nws
            .search(&bbox(178.0, 59.5, -178.0, 63.0))
            .await
            .unwrap()),
        vec!["urn:test:saint-matthew"]
    );
    assert_eq!(
        env.nws
            .search(&bbox(170.0, 55.0, -170.0, 62.0))
            .await
            .unwrap()
            .iter()
            .filter(|a| a.id == "urn:test:saint-matthew")
            .count(),
        1
    );
    // a box that crosses the line in the other hemisphere
    assert_eq!(
        ids(&env
            .nws
            .search(&bbox(170.0, -30.0, -170.0, -10.0))
            .await
            .unwrap()),
        vec!["urn:test:fiji-e", "urn:test:fiji-w"]
    );

    // the database's own answer for random boxes
    let mut state = 0xD1B5_4A32_D192_ED03u64;
    let mut next = |lo: f64, hi: f64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        lo + (state >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    };
    for _ in 0..100 {
        let (w, e) = (next(150.0, 180.0), next(-180.0, -150.0));
        let (s, n) = {
            let a = next(-30.0, 80.0);
            (a, a + next(1.0, 30.0))
        };
        let want: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM nws_alerts WHERE ST_Intersects(geom, ST_MakeEnvelope($1, $2, 180, $4, 4326)) \
             OR ST_Intersects(geom, ST_MakeEnvelope(-180, $2, $3, $4, 4326)) ORDER BY id",
        )
        .bind(w)
        .bind(s)
        .bind(e)
        .bind(n)
        .fetch_all(env.catalog.pool())
        .await
        .unwrap();
        let got = ids(&env.nws.search(&bbox(w, s, e, n)).await.unwrap());
        assert_eq!(got, want, "bbox {w},{s},{e},{n}");
    }

    // both still use the GiST index (one bitmap-OR over its two arms), as the planner sees them on
    // a table of some size rather than the dozen rows above
    let mut state = 0x1234_5678_9ABC_DEF1u64;
    let mut next = |lo: f64, hi: f64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        lo + (state >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    };
    let filler: Vec<ParsedAlert> = (0..1500)
        .map(|i| {
            let (x, y) = (next(-170.0, 170.0), next(-60.0, 70.0));
            timed_alert(
                &format!("urn:test:filler-{i}"),
                [x, y, x + 0.5, y + 0.5],
                at(10, 0),
                at(10, 0),
                at(18, 0),
                &[],
            )
        })
        .collect();
    env.nws
        .apply_feed(&filler, Some(at(10, 2)), at(10, 2))
        .await
        .unwrap();
    sqlx::query("ANALYZE nws_alerts")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    for region in [
        AlertRegion::Bbox([170.0, 50.0, -170.0, 60.0]),
        AlertRegion::Radius {
            lon: 179.95,
            lat: 57.5,
            meters: 50_000.0,
        },
    ] {
        let plan = plan_of(&env, &search(region.clone(), now)).await;
        assert!(
            plan.contains("idx_nws_alerts_geom_live"),
            "{region:?}\n{plan}"
        );
        assert!(plan.contains("BitmapOr"), "{region:?}\n{plan}");
        assert!(!plan.contains("Seq Scan"), "{region:?}\n{plan}");
    }
}

/// Valid in full precision; the hole is 4e-6 degrees inside the shell, so rounding to 5 decimals
/// puts its edge on the shell's (like NWS zones AMZ364 and PZZ745, which come out of a plain
/// `ST_AsGeoJSON(geom, 5)` self-intersecting).
const THIN_HOLE: &str = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]],[[0.000004,0.3],[0.5,0.3],[0.5,0.5],[0.000004,0.5],[0.000004,0.3]]]}"#;
/// A shell with a 0.1 degree bump and a small island (hole) inside the bump. Simplified by more
/// than the bump's height, the shell edge skips over the island, which GEOS before 3.12 allows.
const BUMP_AND_ISLAND: &str = r#"{"type":"Polygon","coordinates":[[[10,0],[20,0],[20,10],[15,10.1],[10,10],[10,0]],[[14.99,10.04],[15.01,10.04],[15.01,10.06],[14.99,10.06],[14.99,10.04]]]}"#;

async fn is_valid_geojson(env: &Env, geometry: &str) -> bool {
    sqlx::query_scalar("SELECT ST_IsValid(ST_GeomFromGeoJSON($1))")
        .bind(geometry)
        .fetch_one(env.catalog.pool())
        .await
        .unwrap()
}

/// What the SQL says a plain rendering of the stored geometry is worth.
async fn naive_is_valid(env: &Env, id: &str, simplify: Option<f64>) -> bool {
    let geom = match simplify {
        Some(t) => format!("ST_SimplifyPreserveTopology(geom, {t})"),
        None => "geom".to_string(),
    };
    sqlx::query_scalar(&format!(
        "SELECT ST_IsValid(ST_GeomFromGeoJSON(ST_AsGeoJSON({geom}, 5))) FROM nws_alerts WHERE id = $1"
    ))
    .bind(id)
    .fetch_one(env.catalog.pool())
    .await
    .unwrap()
}

async fn geos_before_3_12(env: &Env) -> bool {
    let v: String = sqlx::query_scalar("SELECT postgis_geos_version()")
        .fetch_one(env.catalog.pool())
        .await
        .unwrap();
    let mut it = v.split(|c: char| !c.is_ascii_digit());
    let (major, minor): (u32, u32) = (
        it.next().unwrap().parse().unwrap(),
        it.next().unwrap().parse().unwrap(),
    );
    (major, minor) < (3, 12)
}

/// Geometry is valid GeoJSON as the client reads it: after rounding to a metre, and after
/// simplification, on every read that returns one.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn returned_geometry_is_valid_after_rounding_and_simplification() {
    let env = env().await;
    let thin = alert_with_geometry("urn:test:thin", serde_json::from_str(THIN_HOLE).unwrap());
    let bump = alert_with_geometry(
        "urn:test:bump",
        serde_json::from_str(BUMP_AND_ISLAND).unwrap(),
    );
    let tiny = alert_with_geometry(
        "urn:test:tiny",
        serde_json::from_str(&square(30.0, 30.0, 30.000001, 30.000001)).unwrap(),
    );
    let plain = alert_with_geometry(
        "urn:test:plain",
        serde_json::from_str(&square(40.0, 40.0, 41.5, 41.25)).unwrap(),
    );
    let mut pending = timed_alert(
        "urn:test:pending",
        BIG,
        at(10, 0),
        at(10, 0),
        at(18, 0),
        &[],
    );
    pending.geometry_json = None;
    pending.zone_keys = vec!["forecast/NOPE".into()];
    env.nws
        .apply_feed(
            &[thin, bump, tiny, plain, pending],
            Some(at(10, 1)),
            at(10, 1),
        )
        .await
        .unwrap();
    let now = at(12, 0);

    // the stored geometry is valid; the plain renderings are not (the test is sensitive)
    for id in ["urn:test:thin", "urn:test:bump"] {
        let stored: bool =
            sqlx::query_scalar("SELECT ST_IsValid(geom) FROM nws_alerts WHERE id = $1")
                .bind(id)
                .fetch_one(env.catalog.pool())
                .await
                .unwrap();
        assert!(stored, "{id}");
    }
    assert!(
        !naive_is_valid(&env, "urn:test:thin", None).await,
        "5-decimal rounding breaks the thin hole"
    );
    if geos_before_3_12(&env).await {
        assert!(
            !naive_is_valid(&env, "urn:test:bump", Some(0.2)).await,
            "this GEOS lets the simplified shell skip the island"
        );
    }

    let geometry_of = |rows: &[storage::StoredAlert], id: &str| -> Option<String> {
        rows.iter()
            .find(|a| a.id == id)
            .unwrap_or_else(|| panic!("{id} not returned"))
            .geometry_geojson
            .clone()
    };
    let with_geometry = |mut q: AlertSearch, simplify: Option<f64>| {
        q.with_geometry = true;
        q.simplify = simplify;
        q
    };
    let mut reads: Vec<(&str, Vec<storage::StoredAlert>)> = Vec::new();
    for (name, simplify) in [
        ("search", None),
        ("search simplify=0.2", Some(0.2)),
        ("search simplify=0.00001", Some(0.00001)),
        ("search simplify=3", Some(3.0)),
    ] {
        let q = with_geometry(
            search(AlertRegion::Bbox([-1.0, -1.0, 50.0, 50.0]), now),
            simplify,
        );
        reads.push((name, env.nws.search(&q).await.unwrap()));
    }
    reads.push((
        "in_bbox",
        env.nws
            .in_bbox(
                [-1.0, -1.0, 50.0, 50.0],
                now,
                now,
                true,
                false,
                now,
                100,
                true,
            )
            .await
            .unwrap(),
    ));
    reads.push((
        "active_at_point",
        env.nws.active_at_point(0.9, 0.9, now, true).await.unwrap(),
    ));
    for (name, rows) in &reads {
        for id in ["urn:test:thin", "urn:test:bump", "urn:test:plain"] {
            if *name == "active_at_point" && id != "urn:test:thin" {
                continue;
            }
            let g = geometry_of(rows, id).unwrap_or_else(|| panic!("{name}: {id} has a geometry"));
            let v: Value = serde_json::from_str(&g).unwrap();
            assert_eq!(v["type"], "MultiPolygon", "{name} {id}");
            assert!(is_valid_geojson(&env, &g).await, "{name} {id}: {g}");
        }
    }
    // one alert in full, simplified or not
    for id in ["urn:test:thin", "urn:test:bump"] {
        let g = env
            .nws
            .get(id, true)
            .await
            .unwrap()
            .unwrap()
            .geometry_geojson
            .unwrap();
        assert!(is_valid_geojson(&env, &g).await, "get {id}: {g}");
    }

    // what stays: an ordinary shape is the same shape, a pending one has none, one smaller than
    // the grid collapses to none instead of failing the read
    let rows = &reads[0].1;
    let plain_json: Value =
        serde_json::from_str(&geometry_of(rows, "urn:test:plain").unwrap()).unwrap();
    assert_eq!(
        plain_json["coordinates"],
        serde_json::from_str::<Value>("[[[[40,40],[41.5,40],[41.5,41.25],[40,41.25],[40,40]]]]")
            .unwrap()
    );
    let (name, rows) = &reads[0];
    assert!(
        rows.iter().all(|a| a.id != "urn:test:pending"),
        "{name}: a pending alert has no geometry, so no box finds it"
    );
    assert_eq!(geometry_of(rows, "urn:test:tiny"), None);
    let pending = env
        .nws
        .get("urn:test:pending", true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.geometry_geojson, None);
    // the area of the repaired shapes is the stored one's, to a metre
    let (stored, served): (f64, f64) = sqlx::query_as(
        "SELECT ST_Area(geom), ST_Area(ST_GeomFromGeoJSON($1)) FROM nws_alerts WHERE id = 'urn:test:thin'",
    )
    .bind(geometry_of(&reads[0].1, "urn:test:thin").unwrap())
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    assert!((stored - served).abs() < 1e-4, "{stored} vs {served}");
}

// --- a poll rewrites only what changed -----------------------------------------------------------

/// Log every write of an alert's content: an insert, or an upsert's `DO UPDATE SET event = ...`
/// (`UPDATE OF event` fires whenever `event` is in the SET list, changed or not). The
/// `last_seen_at` bump, the supersede, the ended and the geometry updates do not name `event`,
/// so the log is exactly the rows a poll rewrote.
async fn install_write_log(env: &Env) {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS nws_test_write_log (id TEXT NOT NULL, op TEXT NOT NULL)",
    )
    .execute(env.catalog.pool())
    .await
    .unwrap();
    sqlx::query(
        "CREATE OR REPLACE FUNCTION nws_test_log_write() RETURNS trigger LANGUAGE plpgsql AS $f$ \
         BEGIN INSERT INTO nws_test_write_log VALUES (NEW.id, TG_OP); RETURN NEW; END $f$",
    )
    .execute(env.catalog.pool())
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER IF EXISTS nws_test_write_log ON nws_alerts")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER nws_test_write_log AFTER INSERT OR UPDATE OF event ON nws_alerts \
         FOR EACH ROW EXECUTE FUNCTION nws_test_log_write()",
    )
    .execute(env.catalog.pool())
    .await
    .unwrap();
    sqlx::query("TRUNCATE nws_test_write_log")
        .execute(env.catalog.pool())
        .await
        .unwrap();
}

/// The ids written since the log was last drained (sorted, one entry per write), then empties it.
async fn drain_write_log(env: &Env) -> Vec<(String, String)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, op FROM nws_test_write_log ORDER BY id, op")
            .fetch_all(env.catalog.pool())
            .await
            .unwrap();
    sqlx::query("TRUNCATE nws_test_write_log")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    rows
}

/// One poll at `t` (its own `updated` stamp too).
async fn poll(
    env: &Env,
    alerts: &[ParsedAlert],
    t: DateTime<Utc>,
) -> storage::nws_alerts::ApplyResult {
    env.nws.apply_feed(alerts, Some(t), t).await.unwrap()
}

/// Every stored alert as JSON, bar the columns a quiet poll is allowed to move (`last_seen_at`,
/// `hashed_seen_at`) and the hash.
async fn alert_rows(env: &Env) -> Vec<(String, Value)> {
    sqlx::query_as(
        "SELECT id, to_jsonb(a) - 'last_seen_at' - 'hashed_seen_at' - 'content_hash' \
         FROM nws_alerts a ORDER BY id",
    )
    .fetch_all(env.catalog.pool())
    .await
    .unwrap()
}

async fn last_seen(env: &Env) -> Vec<DateTime<Utc>> {
    sqlx::query_scalar("SELECT last_seen_at FROM nws_alerts ORDER BY id")
        .fetch_all(env.catalog.pool())
        .await
        .unwrap()
}

/// A square for each zone the alerts list, so that none is left pending.
async fn resolve_all_zones(env: &Env, alerts: &[ParsedAlert], t: DateTime<Utc>) {
    let mut keys: Vec<&String> = alerts.iter().flat_map(|a| &a.zone_keys).collect();
    keys.sort();
    keys.dedup();
    let zones: Vec<ZoneRecord> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let x = i as f64;
            zone(k, ZoneStatus::Ok, Some(square(x, 10.0, x + 0.5, 10.5)))
        })
        .collect();
    env.nws.upsert_zones(&zones, t).await.unwrap();
}

/// A copy of the fixture's Severe Thunderstorm Warning under `id` that NWS sent as a Cancel.
fn cancel_message(id: &str) -> ParsedAlert {
    let mut f = features()
        .into_iter()
        .find(|f| f["properties"]["event"] == "Severe Thunderstorm Warning")
        .unwrap();
    f["properties"]["id"] = json!(id);
    f["properties"]["messageType"] = json!("Cancel");
    parse_feature(&f).unwrap()
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_poll_of_an_unchanged_feed_rewrites_no_alert_but_advances_last_seen() {
    let env = env().await;
    install_write_log(&env).await;
    let mut alerts = fixture_alerts();
    alerts.push(cancel_message("urn:test:cancel"));
    let n = alerts.len();

    let r = poll(&env, &alerts, t0()).await;
    assert_eq!((r.upserted, r.unchanged, r.failed), (n, 0, 0));
    let first = drain_write_log(&env).await;
    assert_eq!(first.len(), n, "a new alert is written once: {first:?}");
    resolve_all_zones(&env, &alerts, t0()).await;
    let before = alert_rows(&env).await;
    let cancel_ended = env
        .nws
        .get("urn:test:cancel", false)
        .await
        .unwrap()
        .unwrap()
        .ended_at;
    assert_eq!(
        cancel_ended,
        alerts.last().unwrap().effective,
        "a Cancel is over from the moment it was issued"
    );

    // NWS' `updated` moves every minute; the alerts in it do not. Before alerts were hashed every
    // one of them was rewritten by every poll (n of n, measured on this feed).
    for k in 1..=3 {
        let t = t0() + Duration::minutes(k);
        let r = poll(&env, &alerts, t).await;
        assert_eq!(
            (r.upserted, r.unchanged, r.failed, r.ended, r.superseded),
            (0, n, 0, 0, 0),
            "poll {k}"
        );
        assert_eq!(
            drain_write_log(&env).await,
            vec![],
            "poll {k} wrote an alert row"
        );
        assert_eq!(
            last_seen(&env).await,
            vec![t; n],
            "poll {k}: everything was seen"
        );
    }
    assert_eq!(
        alert_rows(&env).await,
        before,
        "nothing but last_seen_at moved"
    );
    let a = env
        .nws
        .get("urn:test:cancel", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((a.ended_at, a.first_seen_at), (cancel_ended, t0()));

    // The skipped rewrite is the one that would have changed nothing: rewriting the same feed in
    // full (what every poll used to do) leaves every column but last_seen_at as it is.
    let unhashed: Vec<ParsedAlert> = alerts
        .iter()
        .cloned()
        .map(|mut a| {
            a.content_hash = None;
            a
        })
        .collect();
    let r = poll(&env, &unhashed, t0() + Duration::minutes(4)).await;
    assert_eq!((r.upserted, r.unchanged), (n, 0));
    assert_eq!(drain_write_log(&env).await.len(), n);
    assert_eq!(alert_rows(&env).await, before);
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn only_the_alert_that_changed_is_rewritten() {
    let env = env().await;
    install_write_log(&env).await;
    let alerts = fixture_alerts();
    let n = alerts.len();
    poll(&env, &alerts, t0()).await;
    resolve_all_zones(&env, &alerts, t0()).await;
    drain_write_log(&env).await;
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    let svr = id_of(&alerts, "Severe Thunderstorm Warning", |a| {
        a.id.starts_with("urn:oid")
    });
    let before = env.nws.get(&winter, false).await.unwrap().unwrap();

    // NWS rewords one alert and moves another's polygon.
    let t1 = t0() + Duration::minutes(1);
    let next: Vec<ParsedAlert> = features()
        .into_iter()
        .map(|mut f| {
            if f["properties"]["id"] == winter.as_str() {
                f["properties"]["description"] =
                    json!("Heavy snow now expected. Travel is unwise.");
            }
            if f["properties"]["id"] == svr.as_str() {
                f["geometry"] = serde_json::from_str(&square(-100.0, 30.0, -99.0, 31.0)).unwrap();
            }
            parse_feature(&f).unwrap()
        })
        .collect();
    let r = poll(&env, &next, t1).await;
    assert_eq!((r.upserted, r.unchanged), (2, n - 2));
    let mut want = vec![
        (winter.clone(), "UPDATE".to_string()),
        (svr.clone(), "UPDATE".to_string()),
    ];
    want.sort();
    assert_eq!(drain_write_log(&env).await, want);

    let w = env.nws.get(&winter, false).await.unwrap().unwrap();
    assert_eq!(
        w.description.as_deref(),
        Some("Heavy snow now expected. Travel is unwise.")
    );
    assert_eq!(w.updated_at, t1, "its content changed");
    assert_eq!(w.first_seen_at, before.first_seen_at);
    let moved: bool = sqlx::query_scalar(
        "SELECT ST_Equals(geom, ST_Multi(ST_GeomFromGeoJSON($2))) FROM nws_alerts WHERE id = $1",
    )
    .bind(&svr)
    .bind(square(-100.0, 30.0, -99.0, 31.0))
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    assert!(moved, "the new polygon was stored");

    // ...and once rewritten they are quiet again.
    let r = poll(&env, &next, t1 + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_changed_alert_resolves_its_zones_again_and_zones_arriving_later_reach_one_nobody_rewrites(
) {
    let env = env().await;
    install_write_log(&env).await;
    let alerts = fixture_alerts();
    let n = alerts.len();
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);

    // No zone is known yet: the zone-based alerts wait, and a second poll leaves them waiting.
    poll(&env, &alerts, t0()).await;
    assert_eq!(source_of(&env, &winter).await, ("pending".into(), false));
    drain_write_log(&env).await;
    let r = poll(&env, &alerts, t0() + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
    assert!(drain_write_log(&env).await.is_empty());
    assert_eq!(source_of(&env, &winter).await, ("pending".into(), false));
    assert!(
        r.missing_zones.contains(&"forecast/COZ068".to_string()),
        "{:?}",
        r.missing_zones
    );

    // Its zone arrives: the alert gets its geometry though no poll writes it.
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ068",
                ZoneStatus::Ok,
                Some(square(CO[0], CO[1], CO[2], CO[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    let r = poll(&env, &alerts, t0() + Duration::minutes(2)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
    assert!(drain_write_log(&env).await.is_empty());
    assert_eq!(
        source_of(&env, &winter).await,
        ("zones".into(), true),
        "kept across quiet polls"
    );

    // NWS moves the warning to another zone, which is already known: the alert is rewritten and
    // its geometry follows in the same poll.
    env.nws
        .upsert_zones(
            &[zone(
                "forecast/COZ069",
                ZoneStatus::Ok,
                Some(square(MD[0], MD[1], MD[2], MD[3])),
            )],
            t0(),
        )
        .await
        .unwrap();
    let t3 = t0() + Duration::minutes(3);
    let next: Vec<ParsedAlert> = features()
        .into_iter()
        .map(|mut f| {
            if f["properties"]["id"] == winter.as_str() {
                f["properties"]["affectedZones"] =
                    json!(["https://api.weather.gov/zones/forecast/COZ069"]);
            }
            parse_feature(&f).unwrap()
        })
        .collect();
    let r = poll(&env, &next, t3).await;
    assert_eq!((r.upserted, r.unchanged), (1, n - 1));
    assert_eq!(
        drain_write_log(&env).await,
        vec![(winter.clone(), "UPDATE".to_string())]
    );
    assert_eq!(source_of(&env, &winter).await, ("zones".into(), true));
    let (lon, lat) = md_lonlat();
    let hit = env.nws.active_at_point(lon, lat, t3, false).await.unwrap();
    assert!(ids(&hit).contains(&winter), "found where its new zone is");
    let old = env
        .nws
        .active_at_point(-107.0, 37.5, t3, false)
        .await
        .unwrap();
    assert!(
        !ids(&old).contains(&winter),
        "and no longer where the old one was"
    );
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn an_alert_that_left_the_feed_and_came_back_is_live_again_and_rewritten_once() {
    let env = env().await;
    install_write_log(&env).await;
    let mut alerts = fixture_alerts();
    alerts.push(cancel_message("urn:test:cancel"));
    let n = alerts.len();
    let winter = id_of(&alerts, "Winter Storm Warning", |_| true);
    poll(&env, &alerts, t0()).await;
    resolve_all_zones(&env, &alerts, t0()).await;
    let first = env.nws.get(&winter, false).await.unwrap().unwrap();
    drain_write_log(&env).await;

    // Gone for a poll: ended, and nothing was rewritten to say so.
    let without: Vec<ParsedAlert> = alerts.iter().filter(|a| a.id != winter).cloned().collect();
    let t1 = t0() + Duration::minutes(1);
    let r = poll(&env, &without, t1).await;
    assert_eq!((r.ended, r.upserted, r.unchanged), (1, 0, n - 1));
    assert!(drain_write_log(&env).await.is_empty());
    let gone = env.nws.get(&winter, false).await.unwrap().unwrap();
    assert_eq!(gone.ended_at, Some(t1));
    assert_eq!(gone.last_seen_at, t0(), "not listed, so not seen");
    assert!(!gone.is_active(t1));

    // Back, byte for byte the same message: revived, which takes a rewrite, and only of it.
    let t2 = t0() + Duration::minutes(2);
    let r = poll(&env, &alerts, t2).await;
    assert_eq!((r.ended, r.upserted, r.unchanged), (0, 1, n - 1));
    assert_eq!(
        drain_write_log(&env).await,
        vec![(winter.clone(), "UPDATE".to_string())]
    );
    let back = env.nws.get(&winter, false).await.unwrap().unwrap();
    assert_eq!(back.ended_at, None);
    assert!(back.is_active(t2));
    assert_eq!(
        (back.first_seen_at, back.last_seen_at),
        (first.first_seen_at, t2)
    );
    assert_eq!(
        back.updated_at, first.updated_at,
        "its content did not change"
    );
    assert_eq!(back.geometry_source, "zones");
    let live = env
        .nws
        .search(&search(AlertRegion::Bbox([-179.0, -89.0, 179.0, 89.0]), t2))
        .await
        .unwrap();
    assert!(ids(&live).contains(&winter));

    // Then quiet again; the Cancel, over since it arrived, was never rewritten for being ended.
    let r = poll(&env, &alerts, t2 + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
    assert!(drain_write_log(&env).await.is_empty());
    let cancel = env
        .nws
        .get("urn:test:cancel", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cancel.ended_at, alerts.last().unwrap().effective);
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_new_derivation_version_rewrites_every_alert_once() {
    let env = env().await;
    install_write_log(&env).await;
    let alerts = fixture_alerts();
    let n = alerts.len();
    poll(&env, &alerts, t0()).await;
    drain_write_log(&env).await;

    // The rows were stored by a build with another DERIVATION_VERSION: same features, other hashes.
    let other = storage::nws_alerts::DERIVATION_VERSION + 1;
    for f in features() {
        let old = storage::nws_alerts::feature_hash(&f, other);
        assert_ne!(Some(old.clone()), parse_feature(&f).unwrap().content_hash);
        sqlx::query("UPDATE nws_alerts SET content_hash = $2 WHERE id = $1")
            .bind(f["properties"]["id"].as_str().unwrap())
            .bind(old)
            .execute(env.catalog.pool())
            .await
            .unwrap();
    }
    let t1 = t0() + Duration::minutes(1);
    let r = poll(&env, &alerts, t1).await;
    assert_eq!((r.upserted, r.unchanged), (n, 0));
    assert_eq!(drain_write_log(&env).await.len(), n);
    let r = poll(&env, &alerts, t1 + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
    assert!(drain_write_log(&env).await.is_empty());
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn rows_stored_before_the_hash_existed_are_migrated_and_rewritten_once() {
    let env = env().await;
    install_write_log(&env).await;
    let alerts = fixture_alerts();
    let n = alerts.len();
    poll(&env, &alerts, t0()).await;
    resolve_all_zones(&env, &alerts, t0()).await;
    let before = alert_rows(&env).await;

    // The table as the previous release left it: no hash columns, the last_seen_at index.
    sqlx::query("ALTER TABLE nws_alerts DROP COLUMN content_hash, DROP COLUMN hashed_seen_at")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    sqlx::query("CREATE INDEX idx_nws_alerts_last_seen ON nws_alerts(last_seen_at)")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    env.catalog
        .migrate_nws_alerts()
        .await
        .expect("migrates a populated table");
    env.catalog.migrate_nws_alerts().await.expect("and again");
    let (rows, hashed): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(content_hash) FROM nws_alerts")
            .fetch_one(env.catalog.pool())
            .await
            .unwrap();
    assert_eq!(
        (rows as usize, hashed),
        (n, 0),
        "the rows survive, hash unknown"
    );
    assert_eq!(hash_columns(&env).await, 2, "and both columns are back");
    let index: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes WHERE tablename = 'nws_alerts' AND indexname = 'idx_nws_alerts_last_seen'",
    )
    .fetch_one(env.catalog.pool())
    .await
    .unwrap();
    assert_eq!(index, 0, "last_seen_at is no longer indexed");
    drain_write_log(&env).await;

    // Unknown means changed: the first poll rewrites everything, and says nothing else changed.
    let t1 = t0() + Duration::minutes(1);
    let r = poll(&env, &alerts, t1).await;
    assert_eq!((r.upserted, r.unchanged, r.failed), (n, 0, 0));
    assert_eq!(drain_write_log(&env).await.len(), n);
    assert_eq!(alert_rows(&env).await, before);
    let hashed: i64 = sqlx::query_scalar("SELECT count(content_hash) FROM nws_alerts")
        .fetch_one(env.catalog.pool())
        .await
        .unwrap();
    assert_eq!(hashed as usize, n);

    // ...once.
    let r = poll(&env, &alerts, t1 + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
    assert!(drain_write_log(&env).await.is_empty());
}

/// How many of the hash columns `nws_alerts` has (2 when migrated).
async fn hash_columns(env: &Env) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_attribute WHERE attrelid = 'nws_alerts'::regclass \
         AND attname IN ('content_hash', 'hashed_seen_at') AND NOT attisdropped",
    )
    .fetch_one(env.catalog.pool())
    .await
    .unwrap()
}

/// What a poll of a build that predates the hash leaves in the rows `filter` selects: its own
/// colour and sections, `last_seen_at` moved to `t`, and the hash as it was.
async fn older_build_polls(env: &Env, t: DateTime<Utc>, filter: &str) {
    sqlx::query(&format!(
        "UPDATE nws_alerts SET color = '#808080', sections = '{{}}', last_seen_at = $1 {filter}"
    ))
    .bind(t)
    .execute(env.catalog.pool())
    .await
    .unwrap();
}

/// An older build rewrites every listed alert on every poll with its own parsers and moves
/// `last_seen_at`, but knows nothing of the hash, so the hash stays what it was. After a rollback
/// and a roll forward the hash still equals the feature's while the row is the old build's: the
/// poll has to see that, and rewrite the row once.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn rows_an_older_build_rewrote_are_rewritten_by_the_next_poll_whatever_their_hash() {
    let env = env().await;
    install_write_log(&env).await;
    let alerts = fixture_alerts();
    let n = alerts.len();
    poll(&env, &alerts, t0()).await;
    resolve_all_zones(&env, &alerts, t0()).await;
    let before = alert_rows(&env).await;
    drain_write_log(&env).await;

    // The older build polls, and rewrites one alert its way.
    let t1 = t0() + Duration::minutes(1);
    older_build_polls(&env, t1, "WHERE event = 'Avalanche Advisory'").await;
    assert_ne!(alert_rows(&env).await, before);
    let t2 = t1 + Duration::minutes(1);
    let r = poll(&env, &alerts, t2).await;
    assert_eq!((r.upserted, r.unchanged, r.failed), (1, n - 1, 0));
    assert_eq!(drain_write_log(&env).await.len(), 1);
    assert_eq!(
        alert_rows(&env).await,
        before,
        "derived the current way again"
    );

    // Then quiet; and the whole table written by the older build is rewritten, once.
    let r = poll(&env, &alerts, t2 + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
    older_build_polls(&env, t2 + Duration::minutes(2), "").await;
    let t4 = t2 + Duration::minutes(2);
    let r = poll(&env, &alerts, t4).await;
    assert_eq!((r.upserted, r.unchanged, r.failed), (n, 0, 0));
    assert_eq!(alert_rows(&env).await, before);
    let r = poll(&env, &alerts, t4 + Duration::minutes(1)).await;
    assert_eq!((r.upserted, r.unchanged), (0, n));
}

/// Looking whether the columns exist is what the migration does at every start; the `ALTER TABLE
/// ... ADD COLUMN IF NOT EXISTS` it replaced queues for an `ACCESS EXCLUSIVE` lock even when
/// there is nothing to add, and every read of the table queues behind it.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn the_migration_takes_no_lock_on_the_alerts_when_nothing_is_missing() {
    let env = env().await;
    // a transaction that has read the table and is still open: a long read, a psql session
    let mut open = env.catalog.pool().begin().await.unwrap();
    sqlx::query("SELECT count(*) FROM nws_alerts")
        .fetch_one(&mut *open)
        .await
        .unwrap();

    // the old statement would have waited for it (so the test is sensitive)...
    let mut probe = env.catalog.pool().begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '300ms'")
        .execute(&mut *probe)
        .await
        .unwrap();
    let e = sqlx::query("ALTER TABLE nws_alerts ADD COLUMN IF NOT EXISTS content_hash TEXT")
        .execute(&mut *probe)
        .await
        .unwrap_err();
    assert_eq!(
        e.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("55P03"),
        "{e}"
    );
    probe.rollback().await.unwrap();

    // ...the migration does not
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        env.catalog.migrate_nws_alerts(),
    )
    .await
    .expect("the migration waited for a lock on nws_alerts")
    .unwrap();
    open.rollback().await.unwrap();
}

/// A column that is missing is added under a short `lock_timeout` and tried again, so the reads
/// queued behind its lock request wait for that long at most, not for as long as the transaction
/// it waits for stays open.
#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_missing_column_is_added_without_holding_reads_up_behind_a_lock() {
    let env = env().await;
    sqlx::query("ALTER TABLE nws_alerts DROP COLUMN hashed_seen_at")
        .execute(env.catalog.pool())
        .await
        .unwrap();
    assert_eq!(hash_columns(&env).await, 1);
    let mut open = env.catalog.pool().begin().await.unwrap();
    sqlx::query("SELECT count(*) FROM nws_alerts")
        .fetch_one(&mut *open)
        .await
        .unwrap();

    let catalog = env.catalog.clone();
    let migrating = tokio::spawn(async move { catalog.migrate_nws_alerts().await });
    // the ALTER is now waiting for `open`; a read arriving queues behind it
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let started = std::time::Instant::now();
    tokio::time::timeout(std::time::Duration::from_secs(15), env.nws.status())
        .await
        .expect("a read was held up behind the migration's lock request")
        .unwrap();
    let waited = started.elapsed();
    assert!(waited < std::time::Duration::from_secs(5), "{waited:?}");

    open.rollback().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), migrating)
        .await
        .expect("the migration never finished")
        .unwrap()
        .expect("the retry succeeds once the lock is free");
    assert_eq!(hash_columns(&env).await, 2);
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn a_row_the_database_refuses_to_touch_is_skipped_alone_and_the_feed_still_applies() {
    let env = env().await;
    let mut alerts = fixture_alerts();
    let mut poison = svr_with_status("urn:test:poison", Some("Actual"));
    poison.geometry_json = None;
    poison.zone_keys = vec!["forecast/COZ068".into()];
    alerts.push(poison);
    let n = alerts.len();
    poll(&env, &alerts, t0()).await;
    install_poison_trigger(&env).await;

    // The feed is unchanged, but the database will not update the poison row at all: the quiet
    // path cannot touch it, so every alert goes the one-at-a-time way, where only it fails.
    let t1 = t0() + Duration::minutes(1);
    let r = poll(&env, &alerts, t1).await;
    assert_eq!((r.failed, r.upserted, r.unchanged), (1, n - 1, 0));
    assert_eq!(
        last_seen(&env).await.iter().filter(|t| **t == t1).count(),
        n - 1
    );
    let st = env.nws.status().await.unwrap().unwrap();
    assert_eq!(st.last_success_at, Some(t1));
    let p = env
        .nws
        .get("urn:test:poison", false)
        .await
        .unwrap()
        .unwrap();
    assert!(p.ended_at.is_none(), "still listed, so still live");
}

#[tokio::test]
#[ignore = "requires docker (PostGIS)"]
async fn replacements_supersede_whether_listed_before_or_after_the_message_and_in_quiet_polls() {
    let env = env().await;
    let a = timed_alert("urn:test:a", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    let a2 = timed_alert(
        "urn:test:a2",
        SMALL,
        at(11, 0),
        at(11, 0),
        at(18, 0),
        &["urn:test:a"],
    );
    let a3 = timed_alert(
        "urn:test:a3",
        SMALL,
        at(11, 30),
        at(11, 30),
        at(18, 0),
        &["urn:test:a"],
    );
    let b = timed_alert("urn:test:b", BIG, at(10, 0), at(10, 0), at(18, 0), &[]);
    let b2 = timed_alert(
        "urn:test:b2",
        SMALL,
        at(11, 0),
        at(11, 0),
        at(18, 0),
        &["urn:test:b"],
    );

    // The replacement is listed first, the first issued of two replacements wins (a3, listed
    // first, is the later), and the replaced message is stored by this very poll.
    let feed = vec![a3.clone(), a2.clone(), a.clone(), b2.clone()];
    let r = poll(&env, &feed, at(12, 0)).await;
    assert_eq!((r.superseded, r.upserted), (1, 4));
    let a_row = env.nws.get("urn:test:a", false).await.unwrap().unwrap();
    assert_eq!(a_row.superseded_by.as_deref(), Some("urn:test:a2"));
    assert_eq!(a_row.superseded_at, Some(at(11, 0)));

    // b2 is unchanged in the next poll, in which b turns up: b2 supersedes it though nothing
    // rewrites b2.
    let feed = vec![a3, a2, a, b2, b];
    let r = poll(&env, &feed, at(12, 1)).await;
    assert_eq!((r.superseded, r.upserted, r.unchanged), (1, 1, 4));
    let b_row = env.nws.get("urn:test:b", false).await.unwrap().unwrap();
    assert_eq!(b_row.superseded_by.as_deref(), Some("urn:test:b2"));
    assert_eq!(b_row.superseded_at, Some(at(11, 0)));
    let r = poll(&env, &feed, at(12, 2)).await;
    assert_eq!((r.superseded, r.upserted, r.unchanged), (0, 0, 5));
}
