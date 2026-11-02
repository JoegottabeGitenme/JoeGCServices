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
    // (left behind in a shared database by a test that installs it, see install_poison_trigger)
    sqlx::query("DROP TRIGGER IF EXISTS nws_test_poison ON nws_alerts")
        .execute(catalog.pool())
        .await
        .expect("drop test trigger");
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
    assert_eq!((r.ended, r.upserted), (0, alerts.len()));
    assert_eq!(
        source_of(&env, "urn:test:bad-with-zone").await,
        ("zones".into(), true),
        "geometry from the zones is kept across polls"
    );

    // An alert that was fine and then arrives with a bad polygon stays live.
    let mut next = alerts.clone();
    next.iter_mut().find(|a| a.id == svr).unwrap().geometry_json = Some(POISON_GEOJSON.into());
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
        }
    }
    let t1 = t0() + Duration::minutes(1);
    let r = env
        .nws
        .apply_feed(&next, Some(t1), t1)
        .await
        .expect("a refused row must not fail the feed");
    assert_eq!((r.failed, r.upserted), (1, alerts.len() - 1));
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
    assert_eq!((r.upserted, r.ended), (old.len(), 1));
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
