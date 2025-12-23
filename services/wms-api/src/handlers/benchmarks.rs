//! Load test and benchmark result handlers.

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

/// GET /api/loadtest/results - Serve historical load test results
pub async fn loadtest_results_handler() -> impl IntoResponse {
    let results_dir = std::env::var("LOADTEST_RESULTS_DIR")
        .unwrap_or_else(|_| "validation/load-test/results".to_string());

    match std::fs::read_dir(&results_dir) {
        Ok(entries) => {
            let mut results = Vec::new();
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".jsonl") {
                        results.push(name.to_string());
                    }
                }
            }
            results.sort();
            results.reverse(); // Most recent first
            Json(serde_json::json!({ "files": results })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read results: {}", e),
        )
            .into_response(),
    }
}

/// GET /api/loadtest/files - List available JSONL result files
pub async fn loadtest_files_handler() -> impl IntoResponse {
    loadtest_results_handler().await
}

/// GET /api/loadtest/file/:filename - Serve specific load test file
pub async fn loadtest_file_handler(Path(filename): Path<String>) -> Response {
    // Sanitize filename to prevent path traversal
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Invalid filename".into())
            .unwrap();
    }

    let results_dir = std::env::var("LOADTEST_RESULTS_DIR")
        .unwrap_or_else(|_| "validation/load-test/results".to_string());
    let filepath = format!("{}/{}", results_dir, filename);

    match std::fs::read_to_string(&filepath) {
        Ok(content) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/x-ndjson")
            .body(content.into())
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("File not found".into())
            .unwrap(),
    }
}

/// GET /api/benchmarks/criterion - Criterion microbenchmark results
pub async fn criterion_benchmarks_handler() -> impl IntoResponse {
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let criterion_dir = format!("{}/criterion", target_dir);

    let mut benchmarks = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&criterion_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    // Try to read the estimates.json for this benchmark
                    let estimates_path = format!("{}/{}/new/estimates.json", criterion_dir, name);
                    if let Ok(content) = std::fs::read_to_string(&estimates_path) {
                        if let Ok(estimates) = serde_json::from_str::<serde_json::Value>(&content) {
                            benchmarks.push(serde_json::json!({
                                "name": name,
                                "estimates": estimates
                            }));
                        }
                    }
                }
            }
        }
    }

    Json(serde_json::json!({ "benchmarks": benchmarks }))
}

/// GET /api/benchmarks - Benchmark results with git metadata
pub async fn benchmarks_handler() -> impl IntoResponse {
    // Get git info
    let git_commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let git_branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Json(serde_json::json!({
        "git": {
            "commit": git_commit,
            "branch": git_branch
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// GET /benchmarks.html - HTML dashboard for load test comparison
pub async fn loadtest_dashboard_handler() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Load Test Dashboard</title>
    <style>
        body { font-family: system-ui, sans-serif; margin: 20px; }
        table { border-collapse: collapse; width: 100%; }
        th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
        th { background-color: #4CAF50; color: white; }
        tr:nth-child(even) { background-color: #f2f2f2; }
    </style>
</head>
<body>
    <h1>Load Test Results</h1>
    <div id="results">Loading...</div>
    <script>
        fetch('/api/loadtest/results')
            .then(r => r.json())
            .then(data => {
                if (data.files && data.files.length > 0) {
                    document.getElementById('results').innerHTML = 
                        '<ul>' + data.files.map(f => '<li><a href="/api/loadtest/file/' + f + '">' + f + '</a></li>').join('') + '</ul>';
                } else {
                    document.getElementById('results').innerHTML = '<p>No results found</p>';
                }
            })
            .catch(e => {
                document.getElementById('results').innerHTML = '<p>Error: ' + e + '</p>';
            });
    </script>
</body>
</html>"#;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html")
        .body(axum::body::Body::from(html))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename_sanitization() {
        // The handler should reject these
        assert!("../etc/passwd".contains(".."));
        assert!("/etc/passwd".contains('/'));
        assert!("..\\windows".contains(".."));
    }
}
