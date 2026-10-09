//! HTTP-level tests for the WMS and WMTS GetCapabilities layer filter (`layer=` / `layers=`),
//! through the real `wms_handler` and a real `AppState` on throwaway
//! Postgres/Redis/MinIO containers.
//!
//! Requires Docker; run with `cargo test -p wms-api --test capabilities_filter -- --ignored`.
//! One test function on purpose: `AppState::new()` reads process-wide env vars.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request},
    routing::{get, post},
    Extension, Router,
};
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

async fn post_path(app: &Router, path: &str) -> u16 {
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
        .as_u16()
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

/// Identifiers of the data layers in a WMTS capabilities document.
fn wmts_layers(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    // data layers: `<Layer>` ... `<ows:Identifier>X</ows:Identifier>` (the tile
    // matrix sets use `<TileMatrixSet>`, not `<Layer>`)
    while let Some(i) = rest.find("<Layer>") {
        let after = &rest[i..];
        let id_at = after.find("<ows:Identifier>").unwrap() + "<ows:Identifier>".len();
        let end = after[id_at..].find("</ows:Identifier>").unwrap();
        out.push(after[id_at..id_at + end].to_string());
        rest = &after[id_at + end..];
    }
    out.sort();
    out.dedup();
    out
}

fn wmts_layer_element(xml: &str, id: &str) -> String {
    let at = xml
        .find(&format!("<ows:Identifier>{id}</ows:Identifier>"))
        .unwrap_or_else(|| panic!("{id} not in WMTS document"));
    let start = xml[..at].rfind("<Layer>").unwrap();
    let end = xml[at..].find("</Layer>").unwrap() + at + "</Layer>".len();
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
        .route("/wmts", get(handlers::wmts_kvp_handler))
        .route(
            "/api/config/reload/layers",
            post(handlers::config_reload_layers_handler),
        )
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

    // --- config reload must not wipe the layer registry ---------------------
    // (the handlers used to look in `<CONFIG_DIR>/layers/layers`, find nothing and
    // swap in an empty registry; this also drops the capabilities cache)
    assert_eq!(post_path(&app, "/api/config/reload/layers").await, 200);
    let (st, after_reload) = get_path(&app, BASE).await;
    assert_eq!(st, 200);
    assert_eq!(
        data_layers(&after_reload),
        data_layers(&full),
        "reload lost layers"
    );
    let (_, one_after) = get_path(&app, &format!("{BASE}&layer=hrrr_DPT")).await;
    assert_eq!(data_layers(&one_after), ["hrrr_DPT"]);
    // a reload that finds no layers is refused and changes nothing
    let good = std::env::var("CONFIG_DIR").unwrap();
    std::env::set_var("CONFIG_DIR", "/definitely/not/a/config/dir");
    assert_eq!(post_path(&app, "/api/config/reload/layers").await, 500);
    std::env::set_var("CONFIG_DIR", good);
    let (_, still) = get_path(&app, &format!("{BASE}&layer=hrrr_DPT")).await;
    assert_eq!(
        data_layers(&still),
        ["hrrr_DPT"],
        "failed reload emptied the registry"
    );

    // ======================= WMTS GetCapabilities ==========================
    const W: &str = "/wmts?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0";
    const ALL: [&str; 6] = [
        "gfs_TMP",
        "hrrr_DPT",
        "hrrr_TMP",
        "hrrr_UGRD",
        "hrrr_VGRD",
        "hrrr_WIND_BARBS",
    ];

    // filtered on a cold WMTS cache must not populate it
    state.capabilities_cache.invalidate().await;
    let (st, w_one) = get_path(&app, &format!("{W}&layer=hrrr_DPT")).await;
    assert_eq!(st, 200, "{w_one}");
    assert!(well_formed(&w_one));
    assert_eq!(wmts_layers(&w_one), ["hrrr_DPT"]);

    let (st, w_full) = get_path(&app, W).await;
    assert_eq!(st, 200);
    assert!(well_formed(&w_full));
    assert_eq!(
        wmts_layers(&w_full),
        ALL,
        "full WMTS document poisoned or changed"
    );
    // ...and once the full document is cached a filtered request must not be served from it
    let (_, w_again) = get_path(&app, &format!("{W}&layer=hrrr_DPT")).await;
    assert_eq!(
        wmts_layers(&w_again),
        ["hrrr_DPT"],
        "filtered request served from the cache"
    );
    assert!(w_again.len() < w_full.len());

    // markup identical to the full document's entry; boilerplate intact
    assert_eq!(
        wmts_layer_element(&w_one, "hrrr_DPT"),
        wmts_layer_element(&w_full, "hrrr_DPT")
    );
    assert!(w_one.contains("<ows:Title>Weather WMTS Service</ows:Title>"));
    assert!(w_one.contains("<ows:Identifier>WebMercatorQuad</ows:Identifier>"));
    assert!(w_one.contains("<ows:Identifier>WorldCRS84Quad</ows:Identifier>"));

    // plural spelling, any key case, URL-encoded commas, both params combined
    let (st, w_two) = get_path(&app, &format!("{W}&LAYERS=HRRR_dpt,hrrr_TMP")).await;
    assert_eq!(st, 200);
    assert_eq!(wmts_layers(&w_two), ["hrrr_DPT", "hrrr_TMP"]);
    let (st, w_enc) = get_path(&app, &format!("{W}&layers=hrrr_DPT%2Chrrr_TMP")).await;
    assert_eq!(st, 200, "{w_enc}");
    assert_eq!(wmts_layers(&w_enc), ["hrrr_DPT", "hrrr_TMP"]);
    let (_, w_both) = get_path(&app, &format!("{W}&layer=hrrr_DPT&layers=gfs_TMP,hrrr_DPT")).await;
    assert_eq!(wmts_layers(&w_both), ["gfs_TMP", "hrrr_DPT"]);

    // empty value = no filter
    let (_, w_empty) = get_path(&app, &format!("{W}&layer=")).await;
    assert_eq!(wmts_layers(&w_empty), ALL);

    // composite built from components, components not emitted
    let (st, w_wind) = get_path(&app, &format!("{W}&layer=hrrr_WIND_BARBS")).await;
    assert_eq!(st, 200, "{w_wind}");
    assert_eq!(wmts_layers(&w_wind), ["hrrr_WIND_BARBS"]);
    assert_eq!(
        wmts_layer_element(&w_wind, "hrrr_WIND_BARBS"),
        wmts_layer_element(&w_full, "hrrr_WIND_BARBS")
    );

    // errors: OWS InvalidParameterValue + locator=layer (the GetTile precedent)
    for (q, expect) in [
        ("layer=nope_TMP", "nope_TMP"),
        ("layers=hrrr_DPT,nope_TMP", "nope_TMP"),
        ("layer=hrrr_GUST", "hrrr_GUST"),
        ("layer=gfs_WIND_BARBS", "gfs_WIND_BARBS"),
    ] {
        let (st, body) = get_path(&app, &format!("{W}&{q}")).await;
        assert_eq!(st, 400, "{q}: {body}");
        assert!(body.contains("ExceptionReport"), "{q}: {body}");
        assert!(
            body.contains(r#"exceptionCode="InvalidParameterValue""#),
            "{q}: {body}"
        );
        assert!(body.contains(r#"locator="layer""#), "{q}: {body}");
        assert!(body.contains(expect), "{q}: {body}");
        assert!(!body.contains("<Contents>"), "{q}: no partial document");
    }

    // LAYER keeps its normal meaning in GetTile: the filter reads it on
    // GetCapabilities only. (The tile itself cannot render here - no data
    // in object storage - but the layer must be accepted, not rejected as
    // a capabilities filter would reject an unknown one.)
    let (_, tile_unknown) = get_path(
        &app,
        "/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=nope_TMP&STYLE=default&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=3&TILEROW=2&TILECOL=2&FORMAT=image/png",
    )
    .await;
    assert!(
        tile_unknown.contains("nope_TMP"),
        "GetTile unknown-layer path changed: {tile_unknown}"
    );

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

    // WMTS: same two behaviors.
    let (st, body) = get_path(&app, &format!("{W}&layer=nope_TMP")).await;
    assert_eq!(
        st, 400,
        "unknown WMTS layer must be rejected without a DB: {body}"
    );
    assert!(body.contains(r#"exceptionCode="InvalidParameterValue""#));
    let (st, body) = get_path(&app, &format!("{W}&layer=hrrr_DPT")).await;
    assert_eq!(st, 500, "{body}");
    assert!(
        body.contains(r#"exceptionCode="NoApplicableCode""#),
        "{body}"
    );
}
