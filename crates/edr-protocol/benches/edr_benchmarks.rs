//! Benchmarks for the EDR protocol crate.
//!
//! Run with: cargo bench --package edr-protocol
//! Or: cargo bench --package edr-protocol --bench edr_benchmarks

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::collections::HashMap;

use edr_protocol::{
    collections::{Collection, CollectionList, Instance, InstanceList},
    coverage_json::{CovJsonParameter, CoverageCollection, CoverageJson, Domain, NdArray},
    parameters::{Parameter, Unit},
    queries::{
        AreaQuery, BboxQuery, CorridorQuery, DateTimeQuery, DistanceUnit, PositionQuery,
        RadiusQuery, TrajectoryQuery, TrajectoryWaypoint, VerticalUnit,
    },
    responses::{ConformanceClasses, LandingPage},
    types::{Extent, Link, TemporalExtent, VerticalExtent},
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
        b.iter(|| DateTimeQuery::parse(black_box("2024-12-29T00:00:00Z/2024-12-29T23:59:59Z")))
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
// AREA (POLYGON) PARSING BENCHMARKS
// =============================================================================

fn bench_area_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("area_parsing");

    // Simple rectangle polygon (4 corners + close)
    group.bench_function("parse_rectangle", |b| {
        b.iter(|| {
            AreaQuery::parse_polygon(black_box(
                "POLYGON((-100 35, -95 35, -95 40, -100 40, -100 35))",
            ))
        })
    });

    // Larger polygon (8 vertices)
    group.bench_function("parse_octagon", |b| {
        b.iter(|| {
            AreaQuery::parse_polygon(black_box(
                "POLYGON((-98 35, -96 34, -94 35, -93 37, -94 39, -96 40, -98 39, -99 37, -98 35))",
            ))
        })
    });

    // Complex polygon (20 vertices - realistic state boundary approximation)
    let complex_polygon = "POLYGON((\
        -104.05 41.00, -102.05 41.00, -102.05 40.00, -101.00 40.00, \
        -100.00 40.00, -99.00 40.00, -98.00 40.00, -97.00 40.00, \
        -96.00 40.00, -95.31 40.00, -95.31 39.00, -95.00 38.00, \
        -94.62 37.00, -94.62 36.00, -100.00 36.00, -100.00 37.00, \
        -102.00 37.00, -102.00 38.00, -103.00 38.00, -104.05 38.00, \
        -104.05 41.00))";
    group.bench_function("parse_complex_20_vertices", |b| {
        b.iter(|| AreaQuery::parse_polygon(black_box(complex_polygon)))
    });

    // Lowercase WKT
    group.bench_function("parse_lowercase", |b| {
        b.iter(|| {
            AreaQuery::parse_polygon(black_box(
                "polygon((-100 35, -95 35, -95 40, -100 40, -100 35))",
            ))
        })
    });

    // Multi-polygon parsing
    group.bench_function("parse_multipolygon", |b| {
        b.iter(|| {
            AreaQuery::parse_polygon_multi(black_box(
                "MULTIPOLYGON(((-100 35, -95 35, -95 40, -100 40, -100 35)),((-90 30, -85 30, -85 35, -90 35, -90 30)))",
            ))
        })
    });

    group.finish();
}

// =============================================================================
// RADIUS QUERY PARSING BENCHMARKS
// =============================================================================

fn bench_radius_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("radius_parsing");

    // Parse radius value
    group.bench_function("parse_within_integer", |b| {
        b.iter(|| RadiusQuery::parse_within(black_box("100")))
    });

    group.bench_function("parse_within_decimal", |b| {
        b.iter(|| RadiusQuery::parse_within(black_box("50.5")))
    });

    // Parse distance units
    group.bench_function("parse_unit_km", |b| {
        b.iter(|| DistanceUnit::parse(black_box("km")))
    });

    group.bench_function("parse_unit_kilometers", |b| {
        b.iter(|| DistanceUnit::parse(black_box("kilometers")))
    });

    group.bench_function("parse_unit_miles", |b| {
        b.iter(|| DistanceUnit::parse(black_box("mi")))
    });

    group.bench_function("parse_unit_meters", |b| {
        b.iter(|| DistanceUnit::parse(black_box("m")))
    });

    group.bench_function("parse_unit_nautical_miles", |b| {
        b.iter(|| DistanceUnit::parse(black_box("nm")))
    });

    // Unit conversion benchmarks
    group.bench_function("convert_km_to_meters", |b| {
        let unit = DistanceUnit::Kilometers;
        b.iter(|| unit.to_meters(black_box(100.0)))
    });

    group.bench_function("convert_miles_to_meters", |b| {
        let unit = DistanceUnit::Miles;
        b.iter(|| unit.to_meters(black_box(62.137)))
    });

    // Create full RadiusQuery
    group.bench_function("create_radius_query", |b| {
        b.iter(|| {
            RadiusQuery::new(
                black_box(-97.5),
                black_box(35.2),
                black_box(100.0),
                DistanceUnit::Kilometers,
            )
        })
    });

    group.finish();
}

// =============================================================================
// TRAJECTORY QUERY PARSING BENCHMARKS
// =============================================================================

fn bench_trajectory_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("trajectory_parsing");

    // Simple linestring (3 points)
    group.bench_function("parse_linestring_3_points", |b| {
        b.iter(|| TrajectoryQuery::parse_coords(black_box("LINESTRING(-100 35, -97 36, -94 35)")))
    });

    // Medium linestring (10 points)
    let medium_linestring = "LINESTRING(\
        -100 35, -99 35.5, -98 36, -97 36.2, -96 36, \
        -95 35.5, -94 35, -93 34.5, -92 34, -91 33.5)";
    group.bench_function("parse_linestring_10_points", |b| {
        b.iter(|| TrajectoryQuery::parse_coords(black_box(medium_linestring)))
    });

    // Long linestring (25 points - cross-country trajectory)
    let long_linestring = "LINESTRING(\
        -122.4 37.8, -121 38, -119 38.5, -117 39, -115 39.5, \
        -113 40, -111 40.5, -109 41, -107 41.2, -105 41, \
        -103 40.5, -101 40, -99 39.5, -97 39, -95 38.5, \
        -93 38, -91 37.5, -89 37, -87 36.5, -85 36, \
        -83 35.5, -81 35, -79 34.5, -77 34, -75 33.5)";
    group.bench_function("parse_linestring_25_points", |b| {
        b.iter(|| TrajectoryQuery::parse_coords(black_box(long_linestring)))
    });

    // Linestring with Z values (3D trajectory)
    group.bench_function("parse_linestringz", |b| {
        b.iter(|| {
            TrajectoryQuery::parse_coords(black_box(
                "LINESTRINGZ(-100 35 1000, -97 36 850, -94 35 700)",
            ))
        })
    });

    // Linestring with M values (with timestamps/measures)
    group.bench_function("parse_linestringm", |b| {
        b.iter(|| {
            TrajectoryQuery::parse_coords(black_box(
                "LINESTRINGM(-100 35 0, -97 36 3600, -94 35 7200)",
            ))
        })
    });

    // Linestring with Z and M values
    group.bench_function("parse_linestringzm", |b| {
        b.iter(|| {
            TrajectoryQuery::parse_coords(black_box(
                "LINESTRINGZM(-100 35 1000 0, -97 36 850 3600, -94 35 700 7200)",
            ))
        })
    });

    // Lowercase WKT
    group.bench_function("parse_lowercase", |b| {
        b.iter(|| TrajectoryQuery::parse_coords(black_box("linestring(-100 35, -97 36, -94 35)")))
    });

    // Multilinestring (for complex trajectories)
    group.bench_function("parse_multilinestring", |b| {
        b.iter(|| {
            TrajectoryQuery::parse_coords(black_box(
                "MULTILINESTRING((-100 35, -97 36, -94 35),(-90 32, -87 33, -84 32))",
            ))
        })
    });

    group.finish();
}

// =============================================================================
// CORRIDOR QUERY PARSING BENCHMARKS
// =============================================================================

fn bench_corridor_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("corridor_parsing");

    // Corridor coordinates (uses same parsing as trajectory)
    group.bench_function("parse_corridor_coords", |b| {
        b.iter(|| CorridorQuery::parse_coords(black_box("LINESTRING(-100 35, -97 36, -94 35)")))
    });

    // Vertical unit parsing
    group.bench_function("parse_vertical_unit_meters", |b| {
        b.iter(|| VerticalUnit::parse(black_box("m")))
    });

    group.bench_function("parse_vertical_unit_kilometers", |b| {
        b.iter(|| VerticalUnit::parse(black_box("km")))
    });

    group.bench_function("parse_vertical_unit_feet", |b| {
        b.iter(|| VerticalUnit::parse(black_box("ft")))
    });

    group.bench_function("parse_vertical_unit_hectopascals", |b| {
        b.iter(|| VerticalUnit::parse(black_box("hPa")))
    });

    group.bench_function("parse_vertical_unit_millibars", |b| {
        b.iter(|| VerticalUnit::parse(black_box("mb")))
    });

    // Create full CorridorQuery
    group.bench_function("create_corridor_query", |b| {
        let waypoints = vec![
            TrajectoryWaypoint {
                lon: -100.0,
                lat: 35.0,
                z: None,
                m: None,
            },
            TrajectoryWaypoint {
                lon: -97.0,
                lat: 36.0,
                z: None,
                m: None,
            },
            TrajectoryWaypoint {
                lon: -94.0,
                lat: 35.0,
                z: None,
                m: None,
            },
        ];
        b.iter(|| {
            CorridorQuery::new(
                black_box(waypoints.clone()),
                black_box(10.0), // corridor width
                DistanceUnit::Kilometers,
                black_box(1000.0), // corridor height
                VerticalUnit::Meters,
            )
        })
    });

    // Width/height calculations
    let corridor = CorridorQuery::new(
        vec![TrajectoryWaypoint {
            lon: -100.0,
            lat: 35.0,
            z: None,
            m: None,
        }],
        50.0,
        DistanceUnit::Kilometers,
        5000.0,
        VerticalUnit::Meters,
    );

    group.bench_function("calculate_width_meters", |b| {
        b.iter(|| black_box(&corridor).width_meters())
    });

    group.bench_function("calculate_half_width_meters", |b| {
        b.iter(|| black_box(&corridor).half_width_meters())
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
            PositionQuery::parse_parameter_names(black_box("TMP,UGRD,VGRD,RH,HGT,PRMSL,APCP"))
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
        let base = CoverageJson::point(
            -97.5,
            35.2,
            Some("2024-12-29T12:00:00Z".to_string()),
            Some(2.0),
        );
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

    // Point series creation (time series at a point)
    group.bench_function("create_point_series_24", |b| {
        let t_values: Vec<String> = (0..24)
            .map(|i| format!("2024-12-29T{:02}:00:00Z", i))
            .collect();
        b.iter(|| {
            CoverageJson::point_series(
                black_box(-97.5),
                black_box(35.2),
                black_box(t_values.clone()),
                black_box(Some(2.0)),
            )
        })
    });

    group.bench_function("create_point_series_168", |b| {
        // Week of hourly data
        let t_values: Vec<String> = (0..168)
            .map(|i| {
                let day = i / 24;
                let hour = i % 24;
                format!("2024-12-{:02}T{:02}:00:00Z", 22 + day, hour)
            })
            .collect();
        b.iter(|| {
            CoverageJson::point_series(
                black_box(-97.5),
                black_box(35.2),
                black_box(t_values.clone()),
                black_box(Some(2.0)),
            )
        })
    });

    // Vertical profile creation
    group.bench_function("create_vertical_profile_7", |b| {
        let z_values = vec![1000.0, 925.0, 850.0, 700.0, 500.0, 300.0, 250.0];
        b.iter(|| {
            CoverageJson::vertical_profile(
                black_box(-97.5),
                black_box(35.2),
                black_box(Some("2024-12-29T12:00:00Z".to_string())),
                black_box(z_values.clone()),
            )
        })
    });

    group.bench_function("create_vertical_profile_37", |b| {
        // Full atmospheric profile (37 levels typical for global models)
        let z_values: Vec<f64> = vec![
            1000.0, 975.0, 950.0, 925.0, 900.0, 875.0, 850.0, 825.0, 800.0, 775.0, 750.0, 700.0,
            650.0, 600.0, 550.0, 500.0, 450.0, 400.0, 350.0, 300.0, 250.0, 225.0, 200.0, 175.0,
            150.0, 125.0, 100.0, 70.0, 50.0, 30.0, 20.0, 10.0, 7.0, 5.0, 3.0, 2.0, 1.0,
        ];
        b.iter(|| {
            CoverageJson::vertical_profile(
                black_box(-97.5),
                black_box(35.2),
                black_box(Some("2024-12-29T12:00:00Z".to_string())),
                black_box(z_values.clone()),
            )
        })
    });

    // Add time series data to coverage
    group.bench_function("add_time_series_24_values", |b| {
        let t_values: Vec<String> = (0..24)
            .map(|i| format!("2024-12-29T{:02}:00:00Z", i))
            .collect();
        let base = CoverageJson::point_series(-97.5, 35.2, t_values, Some(2.0));
        let values: Vec<Option<f32>> = (0..24).map(|i| Some(280.0 + i as f32 * 0.5)).collect();
        b.iter(|| {
            let param = CovJsonParameter::new("Temperature").with_unit(Unit::kelvin());
            black_box(&base)
                .clone()
                .with_time_series("TMP", param, black_box(values.clone()))
        })
    });

    // CoverageCollection creation
    group.bench_function("create_coverage_collection_5", |b| {
        let coverages: Vec<CoverageJson> = (0..5)
            .map(|i| {
                CoverageJson::point(
                    -100.0 + i as f64 * 2.0,
                    35.0 + i as f64 * 0.5,
                    Some("2024-12-29T12:00:00Z".to_string()),
                    Some(2.0),
                )
            })
            .collect();
        b.iter(|| {
            let mut coll = CoverageCollection::new();
            for cov in black_box(&coverages) {
                coll = coll.with_coverage(cov.clone());
            }
            coll
        })
    });

    group.bench_function("create_coverage_collection_20", |b| {
        let coverages: Vec<CoverageJson> = (0..20)
            .map(|i| {
                CoverageJson::point(
                    -100.0 + (i % 5) as f64 * 2.0,
                    30.0 + (i / 5) as f64 * 2.0,
                    Some("2024-12-29T12:00:00Z".to_string()),
                    Some(2.0),
                )
            })
            .collect();
        b.iter(|| {
            let mut coll = CoverageCollection::new();
            for cov in black_box(&coverages) {
                coll = coll.with_coverage(cov.clone());
            }
            coll
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

    // Point series serialization
    let t_values: Vec<String> = (0..24)
        .map(|i| format!("2024-12-29T{:02}:00:00Z", i))
        .collect();
    let mut point_series = CoverageJson::point_series(-97.5, 35.2, t_values, Some(2.0));
    let values: Vec<Option<f32>> = (0..24).map(|i| Some(280.0 + i as f32 * 0.5)).collect();
    point_series = point_series.with_time_series(
        "TMP",
        CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
        values,
    );
    let series_json = serde_json::to_string(&point_series).unwrap();
    group.throughput(Throughput::Bytes(series_json.len() as u64));
    group.bench_function("serialize_point_series_24", |b| {
        b.iter(|| serde_json::to_string(black_box(&point_series)))
    });

    // Vertical profile serialization
    let z_values = vec![1000.0, 925.0, 850.0, 700.0, 500.0, 300.0, 250.0];
    let mut profile = CoverageJson::vertical_profile(
        -97.5,
        35.2,
        Some("2024-12-29T12:00:00Z".to_string()),
        z_values,
    );
    let values: Vec<Option<f32>> = vec![
        Some(288.0),
        Some(285.0),
        Some(280.0),
        Some(270.0),
        Some(255.0),
        Some(230.0),
        Some(220.0),
    ];
    profile = profile.with_vertical_profile_data(
        "TMP",
        CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
        values,
    );
    let profile_json = serde_json::to_string(&profile).unwrap();
    group.throughput(Throughput::Bytes(profile_json.len() as u64));
    group.bench_function("serialize_vertical_profile", |b| {
        b.iter(|| serde_json::to_string(black_box(&profile)))
    });

    // CoverageCollection serialization
    let coverages: Vec<CoverageJson> = (0..10)
        .map(|i| {
            let mut cov = CoverageJson::point(
                -100.0 + i as f64 * 2.0,
                35.0 + i as f64 * 0.5,
                Some("2024-12-29T12:00:00Z".to_string()),
                Some(2.0),
            );
            cov = cov.with_parameter(
                "TMP",
                CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
                280.0 + i as f32,
            );
            cov
        })
        .collect();
    let mut collection = CoverageCollection::new();
    for cov in coverages {
        collection = collection.with_coverage(cov);
    }
    let collection_json = serde_json::to_string(&collection).unwrap();
    group.throughput(Throughput::Bytes(collection_json.len() as u64));
    group.bench_function("serialize_coverage_collection_10", |b| {
        b.iter(|| serde_json::to_string(black_box(&collection)))
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

    // Large array (realistic grid data - 64x64 = 4096 elements)
    let large_values: Vec<f32> = (0..4096).map(|i| (i as f32) * 0.01).collect();
    group.throughput(Throughput::Elements(4096));
    group.bench_function("create_large_array_4k", |b| {
        b.iter(|| {
            NdArray::new(
                black_box(large_values.clone()),
                vec![64, 64],
                vec!["y".to_string(), "x".to_string()],
            )
        })
    });

    // Very large array (256x256 = 65536 elements - typical subgrid)
    let very_large_values: Vec<f32> = (0..65536).map(|i| (i as f32) * 0.001).collect();
    group.throughput(Throughput::Elements(65536));
    group.bench_function("create_very_large_array_64k", |b| {
        b.iter(|| {
            NdArray::new(
                black_box(very_large_values.clone()),
                vec![256, 256],
                vec!["y".to_string(), "x".to_string()],
            )
        })
    });

    // 3D array (typical cube query: 32x32x7 = 7168 elements)
    let cube_values: Vec<f32> = (0..7168).map(|i| (i as f32) * 0.01).collect();
    group.throughput(Throughput::Elements(7168));
    group.bench_function("create_3d_array_cube", |b| {
        b.iter(|| {
            NdArray::new(
                black_box(cube_values.clone()),
                vec![7, 32, 32],
                vec!["z".to_string(), "y".to_string(), "x".to_string()],
            )
        })
    });

    // Large sparse array with missing values
    let large_sparse: Vec<Option<f32>> = (0..10000)
        .map(|i| {
            if i % 7 == 0 {
                None
            } else {
                Some(i as f32 * 0.01)
            }
        })
        .collect();
    group.throughput(Throughput::Elements(10000));
    group.bench_function("create_large_sparse_10k", |b| {
        b.iter(|| {
            NdArray::with_missing(
                black_box(large_sparse.clone()),
                vec![100, 100],
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

    // Very large grid (256x256 - typical CONUS subgrid)
    group.bench_function("create_grid_256x256", |b| {
        let x_values: Vec<f64> = (0..256).map(|i| -130.0 + i as f64 * 0.25).collect();
        let y_values: Vec<f64> = (0..256).map(|i| 20.0 + i as f64 * 0.125).collect();
        b.iter(|| {
            Domain::grid(
                black_box(x_values.clone()),
                black_box(y_values.clone()),
                None,
                None,
            )
        })
    });

    // Large grid with full 4D (x, y, t, z)
    group.bench_function("create_grid_4d", |b| {
        let x_values: Vec<f64> = (0..128).map(|i| -130.0 + i as f64 * 0.5).collect();
        let y_values: Vec<f64> = (0..128).map(|i| 20.0 + i as f64 * 0.25).collect();
        let t_values: Vec<String> = (0..48)
            .map(|i| {
                let day = i / 24;
                let hour = i % 24;
                format!("2024-12-{:02}T{:02}:00:00Z", 29 + day, hour)
            })
            .collect();
        let z_values: Vec<f64> = vec![
            1000.0, 975.0, 950.0, 925.0, 900.0, 850.0, 800.0, 750.0, 700.0, 650.0, 600.0, 550.0,
            500.0, 450.0, 400.0, 350.0, 300.0, 250.0, 200.0, 150.0,
        ];
        b.iter(|| {
            Domain::grid(
                black_box(x_values.clone()),
                black_box(y_values.clone()),
                black_box(Some(t_values.clone())),
                black_box(Some(z_values.clone())),
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
                Parameter::new("UGRD", "U-Wind").with_unit(Unit::meters_per_second()),
            );
            params.insert(
                "VGRD".to_string(),
                Parameter::new("VGRD", "V-Wind").with_unit(Unit::meters_per_second()),
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
                Collection::new(format!("collection-{}", i)).with_title(format!("Collection {}", i))
            })
            .collect();
        b.iter(|| CollectionList::new(black_box(collections.clone()), "http://localhost:8083/edr"))
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
            Instance::new(black_box("2024-12-29T12:00:00Z")).with_title("HRRR Run 2024-12-29 12Z")
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
    let list = InstanceList::new(
        instances.clone(),
        "http://localhost:8083/edr",
        "hrrr-surface",
    );
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
    group.bench_function("create_unit_kelvin", |b| b.iter(|| Unit::kelvin()));

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
    let cov = CoverageJson::point(
        -97.5,
        35.2,
        Some("2024-12-29T12:00:00Z".to_string()),
        Some(2.0),
    )
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

    // Landing page deserialization
    let landing = LandingPage::new(
        "Weather EDR API",
        "Environmental data retrieval for weather models",
        "http://localhost:8083/edr",
    );
    let landing_json = serde_json::to_string(&landing).unwrap();
    group.throughput(Throughput::Bytes(landing_json.len() as u64));
    group.bench_function("deserialize_landing_page", |b| {
        b.iter(|| serde_json::from_str::<LandingPage>(black_box(&landing_json)))
    });

    // Instance list deserialization
    let instances: Vec<Instance> = (0..24)
        .map(|i| {
            Instance::new(format!("2024-12-29T{:02}:00:00Z", 23 - i))
                .with_title(format!("Run {:02}Z", 23 - i))
        })
        .collect();
    let instance_list = InstanceList::new(instances, "http://localhost:8083/edr", "hrrr-surface");
    let instance_list_json = serde_json::to_string(&instance_list).unwrap();
    group.throughput(Throughput::Bytes(instance_list_json.len() as u64));
    group.bench_function("deserialize_instance_list_24", |b| {
        b.iter(|| serde_json::from_str::<InstanceList>(black_box(&instance_list_json)))
    });

    // Large CoverageJSON deserialization (time series)
    let t_values: Vec<String> = (0..24)
        .map(|i| format!("2024-12-29T{:02}:00:00Z", i))
        .collect();
    let mut point_series = CoverageJson::point_series(-97.5, 35.2, t_values, Some(2.0));
    let values: Vec<Option<f32>> = (0..24).map(|i| Some(280.0 + i as f32 * 0.5)).collect();
    point_series = point_series.with_time_series(
        "TMP",
        CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
        values,
    );
    let series_json = serde_json::to_string(&point_series).unwrap();
    group.throughput(Throughput::Bytes(series_json.len() as u64));
    group.bench_function("deserialize_point_series", |b| {
        b.iter(|| serde_json::from_str::<CoverageJson>(black_box(&series_json)))
    });

    // CoverageCollection deserialization
    let coverages: Vec<CoverageJson> = (0..10)
        .map(|i| {
            CoverageJson::point(
                -100.0 + i as f64 * 2.0,
                35.0 + i as f64 * 0.5,
                Some("2024-12-29T12:00:00Z".to_string()),
                Some(2.0),
            )
            .with_parameter(
                "TMP",
                CovJsonParameter::new("Temperature").with_unit(Unit::kelvin()),
                280.0 + i as f32,
            )
        })
        .collect();
    let mut cov_collection = CoverageCollection::new();
    for cov in coverages {
        cov_collection = cov_collection.with_coverage(cov);
    }
    let cov_collection_json = serde_json::to_string(&cov_collection).unwrap();
    group.throughput(Throughput::Bytes(cov_collection_json.len() as u64));
    group.bench_function("deserialize_coverage_collection", |b| {
        b.iter(|| serde_json::from_str::<CoverageCollection>(black_box(&cov_collection_json)))
    });

    // Large NdArray deserialization
    let large_values: Vec<f32> = (0..4096).map(|i| i as f32 * 0.01).collect();
    let large_ndarray = NdArray::new(
        large_values,
        vec![64, 64],
        vec!["y".to_string(), "x".to_string()],
    );
    let large_ndarray_json = serde_json::to_string(&large_ndarray).unwrap();
    group.throughput(Throughput::Bytes(large_ndarray_json.len() as u64));
    group.bench_function("deserialize_ndarray_4k", |b| {
        b.iter(|| serde_json::from_str::<NdArray>(black_box(&large_ndarray_json)))
    });

    // Conformance deserialization
    let conformance = ConformanceClasses::current();
    let conformance_json = serde_json::to_string(&conformance).unwrap();
    group.bench_function("deserialize_conformance", |b| {
        b.iter(|| serde_json::from_str::<ConformanceClasses>(black_box(&conformance_json)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_coordinate_parsing,
    bench_z_parsing,
    bench_datetime_parsing,
    bench_bbox_parsing,
    bench_area_parsing,
    bench_radius_parsing,
    bench_trajectory_parsing,
    bench_corridor_parsing,
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
