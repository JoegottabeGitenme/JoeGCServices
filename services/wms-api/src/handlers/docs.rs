//! API documentation handlers - Swagger UI and OpenAPI spec.

use axum::{
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};

/// OpenAPI 3.0 specification YAML content
const OPENAPI_YAML: &str = include_str!("../../../../docs/api/openapi.yaml");

/// Swagger UI HTML template
const SWAGGER_UI_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Weather WMS/WMTS API Documentation</title>
    <link rel="stylesheet" type="text/css" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
    <style>
        html { box-sizing: border-box; overflow-y: scroll; }
        *, *:before, *:after { box-sizing: inherit; }
        body { margin: 0; background: #fafafa; }
        .swagger-ui .topbar { display: none; }
        .swagger-ui .info .title { font-size: 2rem; }
        .swagger-ui .info { margin: 30px 0; }
        /* Custom styling for weather API */
        .swagger-ui .opblock-tag { font-size: 1.1rem; }
        .swagger-ui .opblock.opblock-get .opblock-summary-method { background: #61affe; }
        .swagger-ui .opblock.opblock-get { border-color: #61affe; background: rgba(97,175,254,.1); }
        /* Header banner */
        .api-header {
            background: linear-gradient(135deg, #1a365d 0%, #2d4a6f 100%);
            color: white;
            padding: 20px 40px;
            display: flex;
            align-items: center;
            gap: 20px;
        }
        .api-header h1 { margin: 0; font-size: 1.5rem; font-weight: 600; }
        .api-header .subtitle { opacity: 0.8; font-size: 0.9rem; margin-top: 4px; }
        .api-header .badges { display: flex; gap: 8px; margin-top: 8px; }
        .api-header .badge {
            background: rgba(255,255,255,0.2);
            padding: 4px 10px;
            border-radius: 4px;
            font-size: 0.75rem;
            font-weight: 500;
        }
        .api-links {
            margin-left: auto;
            display: flex;
            gap: 15px;
        }
        .api-links a {
            color: white;
            text-decoration: none;
            opacity: 0.8;
            font-size: 0.85rem;
            transition: opacity 0.2s;
        }
        .api-links a:hover { opacity: 1; }
    </style>
</head>
<body>
    <div class="api-header">
        <div>
            <h1>Weather WMS/WMTS API</h1>
            <div class="subtitle">OGC-compliant Web Map Service and Web Map Tile Service</div>
            <div class="badges">
                <span class="badge">WMS 1.3.0</span>
                <span class="badge">WMTS 1.0.0</span>
                <span class="badge">OpenAPI 3.0</span>
            </div>
        </div>
        <div class="api-links">
            <a href="/api/docs/openapi.yaml" download>Download OpenAPI Spec</a>
            <a href="/">Back to Dashboard</a>
        </div>
    </div>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-standalone-preset.js"></script>
    <script>
        window.onload = function() {
            window.ui = SwaggerUIBundle({
                url: "/api/docs/openapi.yaml",
                dom_id: '#swagger-ui',
                deepLinking: true,
                presets: [
                    SwaggerUIBundle.presets.apis,
                    SwaggerUIStandalonePreset
                ],
                plugins: [
                    SwaggerUIBundle.plugins.DownloadUrl
                ],
                layout: "StandaloneLayout",
                defaultModelsExpandDepth: 1,
                defaultModelExpandDepth: 1,
                docExpansion: "list",
                filter: true,
                showExtensions: true,
                showCommonExtensions: true,
                tryItOutEnabled: true,
                supportedSubmitMethods: ['get'],
                validatorUrl: null,
            });
        };
    </script>
</body>
</html>"#;

/// Serve the Swagger UI HTML page
/// 
/// GET /api/docs
pub async fn swagger_ui_handler() -> Html<&'static str> {
    Html(SWAGGER_UI_HTML)
}

/// Serve the OpenAPI YAML specification
/// 
/// GET /api/docs/openapi.yaml
pub async fn openapi_yaml_handler() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/yaml")],
        OPENAPI_YAML,
    )
        .into_response()
}

/// Serve the OpenAPI JSON specification (converted from YAML)
/// 
/// GET /api/docs/openapi.json
pub async fn openapi_json_handler() -> Response {
    // Parse YAML and convert to JSON
    match serde_yaml::from_str::<serde_json::Value>(OPENAPI_YAML) {
        Ok(value) => {
            match serde_json::to_string_pretty(&value) {
                Ok(json) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    json,
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize to JSON: {}", e),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to parse OpenAPI YAML: {}", e),
        )
            .into_response(),
    }
}
