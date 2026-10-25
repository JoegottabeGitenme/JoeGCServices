//! OpenAPI definition handler.

use axum::{
    http::{header, StatusCode},
    response::Response,
};

/// OpenAPI 3.0 specification for the EDR API
const OPENAPI_SPEC: &str = include_str!("../openapi.yaml");

/// GET /edr/api - OpenAPI definition
pub async fn api_handler() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "application/vnd.oai.openapi+json;version=3.0",
        )
        .header(header::CACHE_CONTROL, "max-age=3600")
        .body(OPENAPI_SPEC.into())
        .unwrap()
}

/// GET /edr/api.html - API documentation (redirect to ReDoc/Swagger)
pub async fn api_html_handler() -> Response {
    // Return simple HTML with embedded ReDoc
    let html = r#"<!DOCTYPE html>
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
    <script src="https://cdn.redoc.ly/redoc/latest/bundles/redoc.standalone.js"></script>
</body>
</html>"#;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "max-age=3600")
        .body(html.into())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::OPENAPI_SPEC;

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
    fn spec_is_valid_yaml_with_a_paths_section() {
        let value: serde_yaml::Value =
            serde_yaml::from_str(OPENAPI_SPEC).expect("openapi.yaml must parse");
        assert!(value.get("paths").is_some());
        assert!(value.get("components").is_some());
    }
}
