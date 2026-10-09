//! HTTP-level tests for the GetCapabilities layer filter (`layer=` / `layers=`),
//! through the real `wms_handler` and a real `AppState` on throwaway
//! Postgres/Redis/MinIO containers.
//!
//! Requires Docker; run with `cargo test -p wms-api --test capabilities_filter -- --ignored`.
//! One test function on purpose: `AppState::new()` reads process-wide env vars.

use std::sync::Arc;

use axum::{body::Body, http::Request, routing::get, Extension, Router};
use chrono::{DateTime, Duration, Utc};
use storage::{Catalog, CatalogEntry};
use test_utils::containers::{TestConfig, TestInfrastructure};
use tower::ServiceExt;
use wms_api::{handlers, state::AppState};
use wms_common::BoundingBox;

fn entry(model: &str, parameter: &str, level: &str, run: DateTime<Utc>, hour: u32) -> CatalogEntry {
    CatalogEntry {
        model: model.to_string(),
        parameter: parameter.to_string(),
        level: level.to_string(),
        reference_time: run,
        forecast_hour: hour,
        bbox: BoundingBox::new(-135.0, 20.0, -60.0, 55.0),
        storage_path: format!("{model}/{parameter}/{hour}.zarr"),
        file_size: 1,
        zarr_metadata: None,
    }
}

async fn get_path(app: &Router, path_and_query: &str) -> (u16, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path_and_query)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn data_layers(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    let needle = r#"<Layer queryable="1"><Name>"#;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        let end = after.find("</Name>").unwrap();
        out.push(after[..end].to_string());
        rest = &after[end..];
    }
    out.sort();
    // config/layers/gfs.yaml has a second layer (gfs_SST) whose parameter is also
    // TMP, and capabilities name layers `{model}_{parameter}`, so the document
    // (already, in production) advertises `gfs_TMP` twice. Compare as a set.
    out.dedup();
    out
}

fn layer_element(xml: &str, name: &str) -> String {
    let start = xml
        .find(&format!(r#"<Layer queryable="1"><Name>{name}</Name>"#))
        .unwrap_or_else(|| panic!("{name} not in document"));
    let end = xml[start..].find("</Layer>").unwrap() + start + "</Layer>".len();
    xml[start..end].to_string()
}

fn well_formed(xml: &str) -> bool {
    let mut r = quick_xml::Reader::from_str(xml);
    loop {
        match r.read_event() {
            Ok(quick_xml::events::Event::Eof) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

const BASE: &str = "/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0";

#[tokio::test]
#[ignore] // Requires Docker
async fn capabilities_layer_filter_end_to_end() {
    let infra = TestInfrastructure::start().await;
    let cfg = TestConfig::from_infrastructure(&infra);
    cfg.set_env_vars();
    std::env::set_var(
        "CONFIG_DIR",
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config"),
    );

    // Seed: hrrr DPT/TMP/UGRD/VGRD (two runs each) and one gfs layer.
    let catalog = Catalog::connect(&infra.postgres_url()).await.unwrap();
    catalog.migrate().await.unwrap();
    let run_a = Utc::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        - Duration::days(1);
    let run_b = run_a - Duration::hours(1);
    for (param, level) in [
        ("DPT", "2 m above ground"),
        ("TMP", "2 m above ground"),
        ("UGRD", "10 m above ground"),
        ("VGRD", "10 m above ground"),
    ] {
        for run in [run_a, run_b] {
            for hour in [0u32, 1, 2] {
                catalog
                    .register_dataset(&entry("hrrr", param, level, run, hour))
                    .await
                    .unwrap();
            }
        }
    }
    catalog
        .register_dataset(&entry("gfs", "TMP", "2 m above ground", run_a, 0))
        .await
        .unwrap();

    let state = Arc::new(AppState::new().await.expect("AppState"));
    let app = Router::new()
        .route("/wms", get(handlers::wms_handler))
        .layer(Extension(Arc::clone(&state)));

    // --- A filtered request on a cold cache must not populate it ----------
    let (st, one) = get_path(&app, &format!("{BASE}&layer=hrrr_DPT")).await;
    assert_eq!(st, 200, "{one}");
    assert!(well_formed(&one));
    assert_eq!(data_layers(&one), ["hrrr_DPT"]);

    // --- Full document: still the full document (cache not poisoned) -------
    let (st, full) = get_path(&app, BASE).await;
    assert_eq!(st, 200);
    assert!(well_formed(&full));
    assert_eq!(
        data_layers(&full),
        [
            "gfs_TMP",
            "hrrr_DPT",
            "hrrr_TMP",
            "hrrr_UGRD",
            "hrrr_VGRD",
            "hrrr_WIND_BARBS"
        ]
    );
    // the full document is now cached; a filtered request must not be served from it
    let (_, one_again) = get_path(&app, &format!("{BASE}&layer=hrrr_DPT")).await;
    assert_eq!(
        data_layers(&one_again),
        ["hrrr_DPT"],
        "filtered request served from the full-doc cache"
    );
    assert!(one_again.len() < full.len() / 2);

    // --- The single layer is byte-identical to its entry in the full doc ---
    assert_eq!(
        layer_element(&one, "hrrr_DPT"),
        layer_element(&full, "hrrr_DPT")
    );
    assert!(one.contains("<Title>Weather WMS Service</Title>"));

    // --- layers= (plural), any key case, lists, canonical spelling ---------
    let (st, two) = get_path(&app, &format!("{BASE}&LAYERS=HRRR_dpt,hrrr_TMP")).await;
    assert_eq!(st, 200);
    assert_eq!(data_layers(&two), ["hrrr_DPT", "hrrr_TMP"]);

    // --- both parameters: union --------------------------------------------
    let (st, both) = get_path(
        &app,
        &format!("{BASE}&layer=hrrr_DPT&layers=gfs_TMP,hrrr_DPT"),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(data_layers(&both), ["gfs_TMP", "hrrr_DPT"]);

    // --- empty value = no filter -------------------------------------------
    let (st, empty) = get_path(&app, &format!("{BASE}&layer=")).await;
    assert_eq!(st, 200);
    assert_eq!(data_layers(&empty), data_layers(&full));

    // --- composite: built from components, components not emitted ---------
    let (st, wind) = get_path(&app, &format!("{BASE}&layer=hrrr_WIND_BARBS")).await;
    assert_eq!(st, 200, "{wind}");
    assert_eq!(data_layers(&wind), ["hrrr_WIND_BARBS"]);
    assert_eq!(
        layer_element(&wind, "hrrr_WIND_BARBS"),
        layer_element(&full, "hrrr_WIND_BARBS")
    );

    // --- errors: LayerNotDefined, whole request fails ----------------------
    for (q, expect_in_message) in [
        ("layer=nope_TMP", "nope_TMP"),
        ("layers=hrrr_DPT,nope_TMP", "nope_TMP"),
        ("layer=hrrr_GUST", "hrrr_GUST"), // configured, but no data in the catalog
        ("layer=gfs_WIND_BARBS", "gfs_WIND_BARBS"), // composite whose components have no data
    ] {
        let (st, body) = get_path(&app, &format!("{BASE}&{q}")).await;
        assert_eq!(st, 400, "{q}: {body}");
        assert!(body.contains("ServiceExceptionReport"), "{q}: {body}");
        assert!(body.contains(r#"code="LayerNotDefined""#), "{q}: {body}");
        assert!(body.contains(expect_in_message), "{q}: {body}");
        assert!(!body.contains("<Capability>"), "{q}: no partial document");
    }

    // --- short circuit: unknown names never touch the database -------------
    state.catalog.pool_clone().close().await;
    let (st, body) = get_path(&app, &format!("{BASE}&layer=nope_TMP")).await;
    assert_eq!(
        st, 400,
        "unknown name must be rejected without a DB: {body}"
    );
    assert!(body.contains(r#"code="LayerNotDefined""#));
    // ...while a valid name now hits the dead pool and is reported as a server
    // error, not as a misleading "layer not defined".
    let (st, body) = get_path(&app, &format!("{BASE}&layer=hrrr_DPT")).await;
    assert_eq!(st, 500, "{body}");
    assert!(body.contains(r#"code="NoApplicableCode""#), "{body}");
}
