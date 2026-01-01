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
        .header(header::CONTENT_TYPE, "application/openapi+json;version=3.0")
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
