//! Benchmarks for the EDR API service layer.
//!
//! Run with: cargo bench --package edr-api
//! Or: cargo bench --package edr-api --bench edr_api_benchmarks

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use axum::http::{header, HeaderMap, HeaderValue};
use edr_api::content_negotiation::{
    check_accept_header, check_data_query_accept, check_metadata_accept, DATA_QUERY_MEDIA_TYPES,
};
use edr_api::limits::ResponseSizeEstimate;

// =============================================================================
// CONTENT NEGOTIATION BENCHMARKS
// =============================================================================

fn bench_content_negotiation(c: &mut Criterion) {
    let mut group = c.benchmark_group("content_negotiation");

    // Simple Accept headers
    group.bench_function("parse_covjson", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/vnd.cov+json"),
        );
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    group.bench_function("parse_json", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    group.bench_function("parse_wildcard", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    // No header (default case)
    group.bench_function("parse_no_header", |b| {
        let headers = HeaderMap::new();
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    // Complex Accept headers (typical browser)
    group.bench_function("parse_browser_default", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    // Multiple quality values
    group.bench_function("parse_with_quality", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static(
                "application/vnd.cov+json;q=1.0, application/json;q=0.9, */*;q=0.1",
            ),
        );
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    // Unsupported type (must check all then fail)
    group.bench_function("parse_unsupported_text_html", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        b.iter(|| {
            let _ = check_data_query_accept(black_box(&headers));
        })
    });

    // Application wildcard
    group.bench_function("parse_application_wildcard", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/*"));
        b.iter(|| check_data_query_accept(black_box(&headers)))
    });

    // Metadata accept check
    group.bench_function("metadata_accept_json", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        b.iter(|| check_metadata_accept(black_box(&headers)))
    });

    // Generic check_accept_header with custom types
    group.bench_function("check_generic_accept", |b| {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/vnd.cov+json"),
        );
        b.iter(|| check_accept_header(black_box(&headers), DATA_QUERY_MEDIA_TYPES))
    });

    group.finish();
}

// =============================================================================
// RESPONSE SIZE ESTIMATION BENCHMARKS
// =============================================================================

fn bench_response_size_estimation(c: &mut Criterion) {
    let mut group = c.benchmark_group("response_size_estimation");

    // Position query estimates
    group.bench_function("estimate_position_simple", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_position(
                black_box(1),  // params
                black_box(1),  // time steps
                black_box(1),  // levels
            )
        })
    });

    group.bench_function("estimate_position_typical", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_position(
                black_box(5),  // params (TMP, UGRD, VGRD, RH, PRMSL)
                black_box(24), // time steps (hourly for a day)
                black_box(7),  // levels (common isobaric levels)
            )
        })
    });

    group.bench_function("estimate_position_large", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_position(
                black_box(10), // params
                black_box(168), // time steps (week hourly)
                black_box(37), // levels (full atmospheric profile)
            )
        })
    });

    // Area query estimates
    group.bench_function("estimate_area_small", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_area(
                black_box(1),   // params
                black_box(1),   // time steps
                black_box(1),   // levels
                black_box(4.0), // 2x2 degree bbox
                black_box(0.03), // HRRR resolution
            )
        })
    });

    group.bench_function("estimate_area_conus", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_area(
                black_box(3),    // params
                black_box(24),   // time steps
                black_box(1),    // levels
                black_box(1534.0), // CONUS area (~59 * 26 degrees)
                black_box(0.25),   // GFS resolution
            )
        })
    });

    // Radius query estimates
    group.bench_function("estimate_radius_50km", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_radius(
                black_box(3),    // params
                black_box(1),    // time steps
                black_box(1),    // levels
                black_box(50.0), // 50 km radius
                black_box(0.03), // HRRR resolution
            )
        })
    });

    group.bench_function("estimate_radius_100km", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_radius(
                black_box(5),     // params
                black_box(24),    // time steps
                black_box(7),     // levels
                black_box(100.0), // 100 km radius
                black_box(0.03),  // HRRR resolution
            )
        })
    });

    // Trajectory query estimates
    group.bench_function("estimate_trajectory_short", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_trajectory(
                black_box(3),  // params
                black_box(10), // 10 waypoints
                black_box(1),  // time steps
                black_box(1),  // levels
            )
        })
    });

    group.bench_function("estimate_trajectory_long", |b| {
        b.iter(|| {
            ResponseSizeEstimate::for_trajectory(
                black_box(5),   // params
                black_box(100), // 100 waypoints (cross-country)
                black_box(1),   // time steps
                black_box(7),   // levels
            )
        })
    });

    // Estimate calculations
    let estimate = ResponseSizeEstimate::for_area(5, 24, 7, 100.0, 0.03);
    group.bench_function("calculate_estimated_mb", |b| {
        b.iter(|| black_box(&estimate).estimated_mb())
    });

    group.finish();
}

// =============================================================================
// LIMIT CHECKING BENCHMARKS
// =============================================================================

fn bench_limit_checking(c: &mut Criterion) {
    let mut group = c.benchmark_group("limit_checking");

    // Use default limits config
    let limits = edr_api::config::LimitsConfig::default();

    // Check limits on small estimate (should pass quickly)
    let small_estimate = ResponseSizeEstimate::for_position(1, 1, 1);
    group.bench_function("check_limits_small_pass", |b| {
        b.iter(|| black_box(&small_estimate).check_limits(black_box(&limits)))
    });

    // Check limits on typical estimate
    let typical_estimate = ResponseSizeEstimate::for_position(5, 24, 7);
    group.bench_function("check_limits_typical_pass", |b| {
        b.iter(|| black_box(&typical_estimate).check_limits(black_box(&limits)))
    });

    // Check limits on large estimate
    let large_estimate = ResponseSizeEstimate::for_area(10, 48, 20, 1000.0, 0.03);
    group.bench_function("check_limits_large", |b| {
        b.iter(|| {
            let _ = black_box(&large_estimate).check_limits(black_box(&limits));
        })
    });

    // Check limits that will fail
    let exceeding_estimate = ResponseSizeEstimate::for_position(100, 1000, 100);
    group.bench_function("check_limits_exceeding", |b| {
        b.iter(|| {
            let _ = black_box(&exceeding_estimate).check_limits(black_box(&limits));
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_content_negotiation,
    bench_response_size_estimation,
    bench_limit_checking,
);
criterion_main!(benches);
