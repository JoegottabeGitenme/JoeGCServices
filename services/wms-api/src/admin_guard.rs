//! CSRF guard for state-changing API requests.
//!
//! The API has permissive CORS and sits behind browser-cached credentials. Any
//! non-GET request under `/api/` must carry `X-Admin-Request: 1`. A custom
//! header forces a CORS preflight, which a cross-site page cannot pass.
//! `/admin/ingest` (called by the downloader) is not under `/api/` and is exempt.

use axum::{
    extract::Request,
    http::{header::HeaderName, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

/// Header that state-changing `/api/` requests must carry.
pub const ADMIN_REQUEST_HEADER: HeaderName = HeaderName::from_static("x-admin-request");

/// Reject non-GET/HEAD/OPTIONS requests under `/api/` that lack `X-Admin-Request: 1`.
pub async fn require_admin_header(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && req.uri().path().starts_with("/api/") {
        let ok = req
            .headers()
            .get(ADMIN_REQUEST_HEADER)
            .is_some_and(|v| v.as_bytes() == b"1");
        if !ok {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "missing X-Admin-Request header"})),
            )
                .into_response();
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        middleware::from_fn,
        routing::{get, post},
        Router,
    };
    use tower::ServiceExt;
    use tower_http::cors::CorsLayer;

    fn app() -> Router {
        Router::new()
            .route("/api/thing", post(|| async { "ok" }).get(|| async { "ok" }))
            .route("/admin/ingest", post(|| async { "ok" }))
            .route("/health", get(|| async { "ok" }))
            .layer(from_fn(require_admin_header))
            .layer(CorsLayer::permissive())
    }

    async fn send(method: &str, path: &str, header: Option<&str>) -> Response {
        let mut b = axum::http::Request::builder().method(method).uri(path);
        if let Some(h) = header {
            b = b.header("X-Admin-Request", h);
        }
        app().oneshot(b.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn post_without_header_is_forbidden_with_json_body() {
        let resp = send("POST", "/api/thing", None).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "missing X-Admin-Request header");
    }

    #[tokio::test]
    async fn post_with_wrong_value_is_forbidden() {
        let resp = send("POST", "/api/thing", Some("0")).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn post_with_header_passes() {
        let resp = send("POST", "/api/thing", Some("1")).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_is_unaffected() {
        assert_eq!(
            send("GET", "/api/thing", None).await.status(),
            StatusCode::OK
        );
        assert_eq!(send("GET", "/health", None).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_ingest_is_exempt() {
        let resp = send("POST", "/admin/ingest", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cors_preflight_passes_without_header() {
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/api/thing")
            .header("Origin", "https://example.com")
            .header("Access-Control-Request-Method", "POST")
            .header("Access-Control-Request-Headers", "x-admin-request")
            .body(Body::empty())
            .unwrap();
        let resp = app().oneshot(req).await.unwrap();
        assert_ne!(resp.status(), StatusCode::FORBIDDEN);
        assert!(resp.status().is_success());
    }
}
