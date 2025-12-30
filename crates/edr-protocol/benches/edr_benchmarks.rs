//! Benchmarks for the EDR protocol crate.
//!
//! Run with: cargo bench --package edr-protocol
//! Or: cargo bench --package edr-protocol --bench edr_benchmarks

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::collections::HashMap;

use edr_protocol::{
    coverage_json::{CovJsonParameter, CoverageJson, Domain, NdArray},
    parameters::{Parameter, Unit},
    queries::{BboxQuery, DateTimeQuery, PositionQuery},
    responses::{ConformanceClasses, LandingPage},
    types::{Extent, Link, TemporalExtent, VerticalExtent},
    collections::{Collection, CollectionList, Instance, InstanceList},
};

// =============================================================================
// COORDINATE PARSING BENCHMARKS
// =============================================================================

fn bench_coordinate_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("coordinate_parsing");

    // WKT POINT format
    group.bench_function("parse_wkt_point", |b| {
        b.iter(|| PositionQuery::parse_coords(black_box("POINT(-97.5 35.2)")))
    });

    // Simple lon,lat format
    group.bench_function("parse_simple_coords", |b| {
        b.iter(|| PositionQuery::parse_coords(black_box("-97.5,35.2")))
    });

    // WKT with spaces
    group.bench_function("parse_wkt_with_space", |b| {
        b.iter(|| PositionQuery::parse_coords(black_box("POINT (-97.5 35.2)")))
    });

    // Lowercase WKT
    group.bench_function("parse_wkt_lowercase", |b| {
        b.iter(|| PositionQuery::parse_coords(black_box("point(-97.5 35.2)")))
    });

    // Multiple coordinates (typical batch)
    let coords = [
        "POINT(-97.5 35.2)",
        "POINT(-100.0 40.0)",
        "POINT(-85.5 32.1)",
        "-122.4,37.8",
        "-77.0,38.9",
    ];
    group.throughput(Throughput::Elements(coords.len() as u64));
    group.bench_function("parse_batch_5", |b| {
        b.iter(|| {
            for coord in &coords {
                let _ = PositionQuery::parse_coords(black_box(coord));
            }
        })
    });

    group.finish();
}

// =============================================================================
// Z-LEVEL PARSING BENCHMARKS
// =============================================================================

fn bench_z_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("z_parsing");

    // Single level
    group.bench_function("parse_z_single", |b| {
        b.iter(|| PositionQuery::parse_z(black_box("850")))
    });

    // Multiple levels
    group.bench_function("parse_z_multiple_3", |b| {
        b.iter(|| PositionQuery::parse_z(black_box("850,700,500")))
    });

    group.bench_function("parse_z_multiple_7", |b| {
        b.iter(|| PositionQuery::parse_z(black_box("1000,925,850,700,500,300,250")))
    });

    // Range format
    group.bench_function("parse_z_range", |b| {
        b.iter(|| PositionQuery::parse_z(black_box("1000/250")))
    });

    group.finish();
}

// =============================================================================
// DATETIME PARSING BENCHMARKS
// =============================================================================

fn bench_datetime_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("datetime_parsing");

    // Instant
    group.bench_function("parse_instant", |b| {
        b.iter(|| DateTimeQuery::parse(black_box("2024-12-29T12:00:00Z")))
    });

    // Interval
    group.bench_function("parse_interval", |b| {
        b.iter(|| {
            DateTimeQuery::parse(black_box(
                "2024-12-29T00:00:00Z/2024-12-29T23:59:59Z",
            ))
        })
    });

    // Open-ended intervals
    group.bench_function("parse_open_start", |b| {
        b.iter(|| DateTimeQuery::parse(black_box("../2024-12-29T23:59:59Z")))
    });

    group.bench_function("parse_open_end", |b| {
        b.iter(|| DateTimeQuery::parse(black_box("2024-12-29T00:00:00Z/..")))
    });

    group.finish();
}

// =============================================================================
// BBOX PARSING BENCHMARKS
// =============================================================================

fn bench_bbox_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("bbox_parsing");

    // CONUS bbox
    group.bench_function("parse_conus", |b| {
        b.iter(|| BboxQuery::parse(black_box("-125,24,-66,50")))
    });

    // Global bbox
    group.bench_function("parse_global", |b| {
        b.iter(|| BboxQuery::parse(black_box("-180,-90,180,90")))
    });

    // Small region
    group.bench_function("parse_small_region", |b| {
        b.iter(|| BboxQuery::parse(black_box("-98,34,-96,36")))
    });

    // Area calculation
    let conus_bbox = BboxQuery::parse("-125,24,-66,50").unwrap();
    group.bench_function("calculate_area", |b| {
        b.iter(|| black_box(&conus_bbox).area_sq_degrees())
    });

    group.finish();
}

// =============================================================================
// PARAMETER NAME PARSING BENCHMARKS
// =============================================================================

fn bench_parameter_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("parameter_parsing");

    // Single parameter
    group.bench_function("parse_single", |b| {
        b.iter(|| PositionQuery::parse_parameter_names(black_box("TMP")))
    });

    // Multiple parameters
    group.bench_function("parse_multiple_3", |b| {
        b.iter(|| PositionQuery::parse_parameter_names(black_box("TMP,UGRD,VGRD")))
    });

    group.bench_function("parse_multiple_7", |b| {
        b.iter(|| {
            PositionQuery::parse_parameter_names(black_box(
                "TMP,UGRD,VGRD,RH,HGT,PRMSL,APCP",
            ))
        })
    });

    // With spaces (common in URLs)
    group.bench_function("parse_with_spaces", |b| {
        b.iter(|| PositionQuery::parse_parameter_names(black_box(" TMP , UGRD , VGRD ")))
    });

    group.finish();
}

// =============================================================================
// COVERAGE JSON CREATION BENCHMARKS
// =============================================================================

fn bench_coverage_json_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("coverage_json");

    // Point coverage creation
    group.bench_function("create_point", |b| {
        b.iter(|| {
            CoverageJson::point(
                black_box(-97.5),
                black_box(35.2),
                black_box(Some("2024-12-29T12:00:00Z".to_string())),
                black_box(Some(2.0)),
            )
        })
    });

    // Point without optional fields
    group.bench_function("create_point_minimal", |b| {
        b.iter(|| CoverageJson::point(black_box(-97.5), black_box(35.2), None, None))
    });

    // Add single parameter
    group.bench_function("add_parameter", |b| {
        let base = CoverageJson::point(-97.5, 35.2, None, None);
        b.iter(|| {
            let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
            black_box(&base).clone().with_parameter("TMP", param, 288.5)
        })
    });

    // Add multiple parameters (typical weather query)
    group.bench_function("add_5_parameters", |b| {
        let base = CoverageJson::point(-97.5, 35.2, Some("2024-12-29T12:00:00Z".to_string()), Some(2.0));
        let params = [
            ("TMP", Unit::kelvin(), 288.5),
            ("UGRD", Unit::meters_per_second(), 5.2),
            ("VGRD", Unit::meters_per_second(), -3.1),
            ("RH", Unit::percent(), 65.0),
            ("PRMSL", Unit::pascals(), 101325.0),
        ];
        b.iter(|| {
            let mut cov = black_box(&base).clone();
            for (name, unit, value) in &params {
                let param = CovJsonParameter::new(*name).with_unit(unit.clone());
                cov = cov.with_parameter(name, param, *value);
            }
            cov
        })
    });

    group.finish();
}

// =============================================================================
// COVERAGE JSON SERIALIZATION BENCHMARKS
// =============================================================================

fn bench_coverage_json_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("coverage_json_serialization");

    // Simple point coverage
    let simple_cov = CoverageJson::point(-97.5, 35.2, None, None);
    let simple_json = serde_json::to_string(&simple_cov).unwrap();
    group.throughput(Throughput::Bytes(simple_json.len() as u64));
    group.bench_function("serialize_simple", |b| {
        b.iter(|| serde_json::to_string(black_box(&simple_cov)))
    });

    // Full point coverage with parameters
    let mut full_cov = CoverageJson::point(
        -97.5,
        35.2,
        Some("2024-12-29T12:00:00Z".to_string()),
        Some(2.0),
    );
    full_cov = full_cov.with_parameter(
        "TMP",
        CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
        288.5,
    );
    full_cov = full_cov.with_parameter(
        "UGRD",
        CovJsonParameter::new("U-Wind").with_unit(Unit::meters_per_second()),
        5.2,
    );
    full_cov = full_cov.with_parameter(
        "VGRD",
        CovJsonParameter::new("V-Wind").with_unit(Unit::meters_per_second()),
        -3.1,
    );

    let full_json = serde_json::to_string(&full_cov).unwrap();
    group.throughput(Throughput::Bytes(full_json.len() as u64));
    group.bench_function("serialize_full", |b| {
        b.iter(|| serde_json::to_string(black_box(&full_cov)))
    });

    // Pretty print (common for debugging/dev)
    group.bench_function("serialize_pretty", |b| {
        b.iter(|| serde_json::to_string_pretty(black_box(&full_cov)))
    });

    group.finish();
}

// =============================================================================
// NDARRAY BENCHMARKS
// =============================================================================

fn bench_ndarray(c: &mut Criterion) {
    let mut group = c.benchmark_group("ndarray");

    // Scalar creation
    group.bench_function("create_scalar", |b| {
        b.iter(|| NdArray::scalar(black_box(288.5)))
    });

    // Small array (point query with levels)
    group.bench_function("create_small_array", |b| {
        b.iter(|| {
            NdArray::new(
                black_box(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]),
                vec![7],
                vec!["z".to_string()],
            )
        })
    });

    // Medium array (small grid)
    let medium_values: Vec<f32> = (0..256).map(|i| i as f32).collect();
    group.bench_function("create_medium_array", |b| {
        b.iter(|| {
            NdArray::new(
                black_box(medium_values.clone()),
                vec![16, 16],
                vec!["y".to_string(), "x".to_string()],
            )
        })
    });

    // Array with missing values
    let sparse_values: Vec<Option<f32>> = (0..256)
        .map(|i| if i % 3 == 0 { None } else { Some(i as f32) })
        .collect();
    group.bench_function("create_sparse_array", |b| {
        b.iter(|| {
            NdArray::with_missing(
                black_box(sparse_values.clone()),
                vec![16, 16],
                vec!["y".to_string(), "x".to_string()],
            )
        })
    });

    group.finish();
}

// =============================================================================
// DOMAIN CREATION BENCHMARKS
// =============================================================================

fn bench_domain(c: &mut Criterion) {
    let mut group = c.benchmark_group("domain");

    // Point domain
    group.bench_function("create_point", |b| {
        b.iter(|| {
            Domain::point(
                black_box(-97.5),
                black_box(35.2),
                black_box(Some("2024-12-29T12:00:00Z".to_string())),
                black_box(Some(850.0)),
            )
        })
    });

    // Grid domain (small)
    group.bench_function("create_grid_small", |b| {
        let x_values: Vec<f64> = (0..16).map(|i| -100.0 + i as f64 * 0.25).collect();
        let y_values: Vec<f64> = (0..16).map(|i| 30.0 + i as f64 * 0.25).collect();
        b.iter(|| {
            Domain::grid(
                black_box(x_values.clone()),
                black_box(y_values.clone()),
                None,
                black_box(Some(vec![850.0, 700.0, 500.0])),
            )
        })
    });

    // Grid domain (larger)
    group.bench_function("create_grid_large", |b| {
        let x_values: Vec<f64> = (0..64).map(|i| -130.0 + i as f64 * 1.0).collect();
        let y_values: Vec<f64> = (0..64).map(|i| 20.0 + i as f64 * 0.5).collect();
        let t_values: Vec<String> = (0..24)
            .map(|i| format!("2024-12-29T{:02}:00:00Z", i))
            .collect();
        b.iter(|| {
            Domain::grid(
                black_box(x_values.clone()),
                black_box(y_values.clone()),
                black_box(Some(t_values.clone())),
                black_box(Some(vec![1000.0, 925.0, 850.0, 700.0, 500.0, 300.0, 250.0])),
            )
        })
    });

    group.finish();
}

// =============================================================================
// COLLECTION/RESPONSE BENCHMARKS
// =============================================================================

fn bench_collections(c: &mut Criterion) {
    let mut group = c.benchmark_group("collections");

    // Create single collection
    group.bench_function("create_collection", |b| {
        b.iter(|| {
            Collection::new(black_box("hrrr-surface"))
                .with_title("HRRR Surface")
                .with_description("High-Resolution Rapid Refresh surface level parameters")
        })
    });

    // Collection with all fields
    group.bench_function("create_full_collection", |b| {
        b.iter(|| {
            let mut params = HashMap::new();
            params.insert(
                "TMP".to_string(),
                Parameter::new("TMP", "Temperature")
                    .with_unit(Unit::kelvin())
                    .with_description("Air temperature"),
            );
            params.insert(
                "UGRD".to_string(),
                Parameter::new("UGRD", "U-Wind")
                    .with_unit(Unit::meters_per_second()),
            );
            params.insert(
                "VGRD".to_string(),
                Parameter::new("VGRD", "V-Wind")
                    .with_unit(Unit::meters_per_second()),
            );

            Collection::new(black_box("hrrr-surface"))
                .with_title("HRRR Surface")
                .with_description("High-Resolution Rapid Refresh surface level parameters")
                .with_extent(Extent::with_spatial([-134.0, 21.0, -60.0, 53.0], None))
                .with_crs(vec!["CRS:84".to_string(), "EPSG:4326".to_string()])
                .with_output_formats(vec!["application/vnd.cov+json".to_string()])
                .with_parameters(params)
        })
    });

    // Create collection list
    group.bench_function("create_collection_list", |b| {
        let collections: Vec<Collection> = (0..5)
            .map(|i| {
                Collection::new(format!("collection-{}", i))
                    .with_title(format!("Collection {}", i))
            })
            .collect();
        b.iter(|| {
            CollectionList::new(
                black_box(collections.clone()),
                "http://localhost:8083/edr",
            )
        })
    });

    // Serialize collection list
    let collections: Vec<Collection> = vec![
        Collection::new("hrrr-surface").with_title("HRRR Surface"),
        Collection::new("hrrr-isobaric").with_title("HRRR Isobaric"),
        Collection::new("gfs-surface").with_title("GFS Surface"),
    ];
    let list = CollectionList::new(collections, "http://localhost:8083/edr");
    let json = serde_json::to_string(&list).unwrap();
    group.throughput(Throughput::Bytes(json.len() as u64));
    group.bench_function("serialize_collection_list", |b| {
        b.iter(|| serde_json::to_string(black_box(&list)))
    });

    group.finish();
}

// =============================================================================
// INSTANCE BENCHMARKS
// =============================================================================

fn bench_instances(c: &mut Criterion) {
    let mut group = c.benchmark_group("instances");

    // Create instance
    group.bench_function("create_instance", |b| {
        b.iter(|| {
            Instance::new(black_box("2024-12-29T12:00:00Z"))
                .with_title("HRRR Run 2024-12-29 12Z")
        })
    });

    // Create instance list (typical model runs)
    let instances: Vec<Instance> = (0..24)
        .map(|i| {
            let hour = 23 - i;
            Instance::new(format!("2024-12-29T{:02}:00:00Z", hour))
                .with_title(format!("HRRR Run 2024-12-29 {:02}Z", hour))
        })
        .collect();

    group.bench_function("create_instance_list", |b| {
        b.iter(|| {
            InstanceList::new(
                black_box(instances.clone()),
                "http://localhost:8083/edr",
                "hrrr-surface",
            )
        })
    });

    // Serialize instance list
    let list = InstanceList::new(instances.clone(), "http://localhost:8083/edr", "hrrr-surface");
    let json = serde_json::to_string(&list).unwrap();
    group.throughput(Throughput::Bytes(json.len() as u64));
    group.bench_function("serialize_instance_list", |b| {
        b.iter(|| serde_json::to_string(black_box(&list)))
    });

    group.finish();
}

// =============================================================================
// LANDING PAGE AND CONFORMANCE BENCHMARKS
// =============================================================================

fn bench_responses(c: &mut Criterion) {
    let mut group = c.benchmark_group("responses");

    // Landing page creation
    group.bench_function("create_landing_page", |b| {
        b.iter(|| {
            LandingPage::new(
                black_box("Weather EDR API"),
                black_box("Environmental data retrieval for weather models"),
                black_box("http://localhost:8083/edr"),
            )
        })
    });

    // Conformance creation
    group.bench_function("create_conformance", |b| {
        b.iter(|| ConformanceClasses::current())
    });

    // Serialize landing page
    let landing = LandingPage::new(
        "Weather EDR API",
        "Environmental data retrieval for weather models",
        "http://localhost:8083/edr",
    );
    let json = serde_json::to_string(&landing).unwrap();
    group.throughput(Throughput::Bytes(json.len() as u64));
    group.bench_function("serialize_landing", |b| {
        b.iter(|| serde_json::to_string(black_box(&landing)))
    });

    // Serialize conformance
    let conformance = ConformanceClasses::current();
    group.bench_function("serialize_conformance", |b| {
        b.iter(|| serde_json::to_string(black_box(&conformance)))
    });

    group.finish();
}

// =============================================================================
// LINK AND TYPE BENCHMARKS
// =============================================================================

fn bench_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("types");

    // Link creation
    group.bench_function("create_link_simple", |b| {
        b.iter(|| Link::new(black_box("http://example.com"), black_box("self")))
    });

    group.bench_function("create_link_full", |b| {
        b.iter(|| {
            Link::new(black_box("http://example.com/data"), black_box("data"))
                .with_type("application/json")
                .with_title("Data endpoint")
                .templated()
        })
    });

    // Extent creation
    group.bench_function("create_extent", |b| {
        b.iter(|| {
            Extent::with_spatial(black_box([-180.0, -90.0, 180.0, 90.0]), None)
                .with_temporal(TemporalExtent::new(
                    Some("2024-01-01T00:00:00Z".to_string()),
                    Some("2024-12-31T23:59:59Z".to_string()),
                ))
                .with_vertical(VerticalExtent::new(1000.0, 250.0))
        })
    });

    // Unit presets
    group.bench_function("create_unit_kelvin", |b| {
        b.iter(|| Unit::kelvin())
    });

    group.bench_function("create_unit_from_symbol", |b| {
        b.iter(|| Unit::from_symbol(black_box("m/s")))
    });

    group.finish();
}

// =============================================================================
// DESERIALIZATION BENCHMARKS
// =============================================================================

fn bench_deserialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("deserialization");

    // CoverageJSON deserialization
    let cov = CoverageJson::point(-97.5, 35.2, Some("2024-12-29T12:00:00Z".to_string()), Some(2.0))
        .with_parameter(
            "TMP",
            CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
            288.5,
        );
    let json = serde_json::to_string(&cov).unwrap();
    group.throughput(Throughput::Bytes(json.len() as u64));
    group.bench_function("deserialize_coverage_json", |b| {
        b.iter(|| serde_json::from_str::<CoverageJson>(black_box(&json)))
    });

    // Link deserialization
    let link_json = r#"{"href":"http://example.com","rel":"self","type":"application/json"}"#;
    group.bench_function("deserialize_link", |b| {
        b.iter(|| serde_json::from_str::<Link>(black_box(link_json)))
    });

    // Collection deserialization
    let collection = Collection::new("test")
        .with_title("Test Collection")
        .with_crs(vec!["CRS:84".to_string()]);
    let collection_json = serde_json::to_string(&collection).unwrap();
    group.bench_function("deserialize_collection", |b| {
        b.iter(|| serde_json::from_str::<Collection>(black_box(&collection_json)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_coordinate_parsing,
    bench_z_parsing,
    bench_datetime_parsing,
    bench_bbox_parsing,
    bench_parameter_parsing,
    bench_coverage_json_creation,
    bench_coverage_json_serialization,
    bench_ndarray,
    bench_domain,
    bench_collections,
    bench_instances,
    bench_responses,
    bench_types,
    bench_deserialization,
);
criterion_main!(benches);
