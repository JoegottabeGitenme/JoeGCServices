//! OpenAPI definition handler.

use std::sync::LazyLock;

use axum::{
    extract::{rejection::QueryRejection, Query},
    http::{header, StatusCode},
    response::Response,
};
use serde::Deserialize;

/// OpenAPI 3.0 specification for the EDR API, as written (YAML).
const OPENAPI_SPEC: &str = include_str!("../openapi.yaml");

/// Media type of the JSON form, the one the landing page's `service-desc` link advertises.
const OPENAPI_JSON_TYPE: &str = "application/vnd.oai.openapi+json;version=3.0";
/// Media type of the YAML form (`?f=yaml`).
const OPENAPI_YAML_TYPE: &str = "application/vnd.oai.openapi;version=3.0";

/// [`OPENAPI_SPEC`] as JSON, in the document's own order, made once: a client that is told the
/// answer is `+json` may hand it straight to a JSON parser.
static OPENAPI_JSON: LazyLock<String> = LazyLock::new(|| {
    yaml_to_json(OPENAPI_SPEC).expect("openapi.yaml converts to JSON (see the unit test)")
});

/// The YAML document as compact JSON. Keys keep their order, and a key that is not a string
/// (an unquoted `200:`) becomes one.
fn yaml_to_json(yaml: &str) -> Result<String, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

/// Convert the specification now, so a document that cannot be converted fails the start of the
/// service, not the first request for it.
pub fn init() {
    LazyLock::force(&OPENAPI_JSON);
}

#[derive(Debug, Default, Deserialize)]
pub struct ApiQuery {
    /// `yaml` for the document as written; anything else (`json`, none) is JSON.
    f: Option<String>,
}

/// GET /edr/api - OpenAPI definition: JSON (`application/vnd.oai.openapi+json`), or with
/// `?f=yaml` (`yml`) the YAML source (`application/vnd.oai.openapi`).
pub async fn api_handler(query: Result<Query<ApiQuery>, QueryRejection>) -> Response {
    let yaml = query.is_ok_and(|Query(q)| {
        q.f.as_deref()
            .is_some_and(|f| f.eq_ignore_ascii_case("yaml") || f.eq_ignore_ascii_case("yml"))
    });
    let (content_type, body) = if yaml {
        (OPENAPI_YAML_TYPE, OPENAPI_SPEC)
    } else {
        (OPENAPI_JSON_TYPE, OPENAPI_JSON.as_str())
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "max-age=3600")
        .body(body.into())
        .unwrap()
}

/// ReDoc page for the spec at `api` (relative, so it resolves to /edr/api).
///
/// The bundle is pinned to an exact release. It used to load `redoc/latest`, and when
/// `latest` moved to Redoc 3 (an ES module: the bundle ends in `export {...}`) the classic
/// `<script>` tag threw a SyntaxError and the page went blank.
const API_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
    <title>Weather WMS EDR API Documentation</title>
    <meta charset="utf-8"/>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <link href="https://fonts.googleapis.com/css?family=Montserrat:300,400,700|Roboto:300,400,700" rel="stylesheet">
    <style>
        body { margin: 0; padding: 0; }
    </style>
</head>
<body>
    <redoc spec-url='api'></redoc>
    <script src="https://cdn.redoc.ly/redoc/v2.5.4/bundles/redoc.standalone.js"></script>
</body>
</html>"#;

/// GET /edr/api.html - API documentation (ReDoc)
pub async fn api_html_handler() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "max-age=3600")
        .body(API_HTML.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A floating CDN version (`redoc/latest`) once blanked this page when Redoc 3 shipped
    /// as an ES module. Keep the bundle pinned.
    #[test]
    fn api_html_pins_the_redoc_version() {
        assert!(
            API_HTML.contains("cdn.redoc.ly/redoc/v2."),
            "ReDoc bundle is not pinned to 2.x"
        );
        assert!(
            !API_HTML.contains("/latest/"),
            "api.html loads a floating `latest` asset"
        );
    }

    async fn get(f: Option<&str>) -> (String, String) {
        let q = ApiQuery {
            f: f.map(str::to_string),
        };
        let r = api_handler(Ok(Query(q))).await;
        let (parts, body) = r.into_parts();
        let bytes = axum::body::to_bytes(body, 8 << 20).await.unwrap();
        (
            parts.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .to_string(),
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    /// `/edr/api` said `application/vnd.oai.openapi+json` and sent YAML, so a client that parsed
    /// it as the type said failed on the first character.
    #[tokio::test]
    async fn the_json_type_comes_with_json() {
        for f in [None, Some("json"), Some("JSON"), Some("html"), Some("")] {
            let (ty, body) = get(f).await;
            assert_eq!(ty, "application/vnd.oai.openapi+json;version=3.0", "{f:?}");
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| {
                panic!("{f:?}: not JSON ({e}): {}", &body[..body.len().min(80)])
            });
            assert!(v["openapi"].as_str().unwrap().starts_with("3.0"));
            assert!(v["paths"].is_object() && v["components"].is_object());
        }
    }

    #[tokio::test]
    async fn f_yaml_sends_the_document_as_written_with_the_yaml_type() {
        for f in ["yaml", "YAML", "yml"] {
            let (ty, body) = get(Some(f)).await;
            assert_eq!(ty, "application/vnd.oai.openapi;version=3.0", "{f}");
            assert_eq!(body, OPENAPI_SPEC, "{f}");
        }
        // a query string that does not parse is the default, not an error
        let r = api_handler(Query::<ApiQuery>::try_from_uri(
            &"/edr/api?f=a&f=b".parse().unwrap(),
        ))
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            r.headers()[header::CONTENT_TYPE],
            "application/vnd.oai.openapi+json;version=3.0"
        );
    }

    #[test]
    fn the_json_is_the_yaml_document_in_its_own_order() {
        init();
        let from_json: serde_json::Value = serde_json::from_str(&OPENAPI_JSON).unwrap();
        let from_yaml: serde_json::Value = serde_yaml::from_str(OPENAPI_SPEC).unwrap();
        assert_eq!(from_json, from_yaml, "nothing lost or changed");
        // the response codes are quoted in the file, and strings in the JSON
        let responses = &from_json["paths"]["/collections/{collectionId}/items/{featureId}"]["get"]
            ["responses"];
        assert!(responses["200"].is_object(), "{responses}");
        // the document's own order: `openapi` first, not sorted away
        assert!(
            OPENAPI_JSON.starts_with(r#"{"openapi":"3.0"#),
            "{}",
            &OPENAPI_JSON[..40]
        );
    }

    #[test]
    fn yaml_keys_that_are_not_strings_become_strings() {
        let json = yaml_to_json("responses:\n  200:\n    description: ok\n  404:\n    description: no\nflag:\n  true: 1\n").unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["responses"]["200"]["description"], "ok");
        assert_eq!(v["responses"]["404"]["description"], "no");
        assert_eq!(v["flag"]["true"], 1);
        assert!(yaml_to_json("a: [").is_err());
    }

    /// The served OpenAPI spec once had no mention of the trails collection
    /// at all, even though `/items`, `conditions=latest` and the timeseries
    /// endpoint were live. Keep the contract a frontend team reads in step
    /// with the routes `main.rs` actually registers.
    #[test]
    fn spec_documents_the_trail_conditions_contract() {
        for needle in [
            "/collections/{collectionId}/items:",
            "/collections/{collectionId}/items/{featureId}/conditions:",
            "operationId: getTrailConditionsTimeseries",
            "/collections/{collectionId}/conditions:",
            "operationId: getTrailConditionsBatch",
            "TrailConditionsBatch:",
            "unknown_ids:",
            "trailConditions:",
            "TrailConditions:",
            "TrailConditionsTimeseries:",
            "saturation:",
            "frozen_fraction:",
            "confidence:",
            "model_version:",
        ] {
            assert!(
                OPENAPI_SPEC.contains(needle),
                "openapi.yaml is missing `{}`",
                needle
            );
        }
    }

    #[test]
    fn area_and_radius_document_limit_and_class() {
        // They used to be silently ignored; keep the published contract honest.
        let v: serde_yaml::Value = serde_yaml::from_str(OPENAPI_SPEC).unwrap();
        for op in [
            "/collections/{collectionId}/area",
            "/collections/{collectionId}/radius",
        ] {
            let names: Vec<String> = v["paths"][op]["get"]["parameters"]
                .as_sequence()
                .unwrap()
                .iter()
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect();
            assert!(names.contains(&"limit".to_string()), "{} lacks limit", op);
            assert!(names.contains(&"class".to_string()), "{} lacks class", op);
        }
    }

    #[test]
    fn spec_documents_the_lightning_contract() {
        let v: serde_yaml::Value = serde_yaml::from_str(OPENAPI_SPEC).unwrap();
        // the three shared parameters are referenced from items, area and radius
        for op in ["items", "area", "radius"] {
            let path = format!("/collections/{{collectionId}}/{}", op);
            let refs: Vec<String> = v["paths"][path.as_str()]["get"]["parameters"]
                .as_sequence()
                .unwrap()
                .iter()
                .filter_map(|p| p.get("$ref").and_then(|r| r.as_str()).map(String::from))
                .collect();
            for name in ["lightningWindow", "lightningAfter", "lightningSatellite"] {
                assert!(refs.iter().any(|r| r.ends_with(name)), "{op} lacks {name}");
            }
        }
        // the freshness fields exist AND are required: they are what lets a client
        // tell a quiet sky from a dead feed, so they must not silently go missing.
        let coll = &v["components"]["schemas"]["LightningFeatureCollection"];
        let required: Vec<&str> = coll["required"]
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        for field in [
            "dataThrough",
            "dataAgeSeconds",
            "lastId",
            "timeStamp",
            "numberReturned",
        ] {
            assert!(required.contains(&field), "{field} must be required");
            assert!(
                coll["properties"].get(field).is_some(),
                "{field} undocumented"
            );
        }
        assert!(v["components"]["schemas"].get("LightningFeature").is_some());
    }

    #[test]
    fn spec_documents_nws_alerts() {
        for needle in [
            "/collections/{collectionId}/items/{featureId}:",
            "operationId: getItem",
            "urn:oid:2.49.0.1.840.0",
            "severity=Severe,Extreme",
            "status=active|all",
            "include_non_actual=true",
        ] {
            assert!(
                OPENAPI_SPEC.contains(needle),
                "openapi.yaml is missing `{}`",
                needle
            );
        }
    }

    #[test]
    fn spec_is_valid_yaml_with_a_paths_section() {
        let value: serde_yaml::Value =
            serde_yaml::from_str(OPENAPI_SPEC).expect("openapi.yaml must parse");
        assert!(value.get("paths").is_some());
        assert!(value.get("components").is_some());
    }
}
