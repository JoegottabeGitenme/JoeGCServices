"""Parser tests against synthetic files built to match the documented
Tarrawarra formats exactly (see parsers.py's module docstring for the
format sources). Proves the parsing logic is correct independent of
whether the real files are available this session.
"""

from pathlib import Path

import numpy as np
import pytest

from parsers import (
    parse_dem,
    parse_ksat_file,
    parse_layer_file,
    parse_nmm_file,
    parse_particle_file,
    parse_tdr_file,
)


@pytest.fixture
def synthetic_dem_file(tmp_path):
    """A tiny 3x4 ESRI-ASCII-style grid, matching Readme.topo's description
    ('6 line header with the boundaries... starting in the North western
    corner... proceeding row by row')."""
    content = (
        "ncols 4\n"
        "nrows 3\n"
        "xllcorner 100.0\n"
        "yllcorner 200.0\n"
        "cellsize 5.0\n"
        "NODATA_value -9999\n"
        "10.0 10.5 11.0 11.5\n"
        "9.0 9.5 10.0 10.5\n"
        "8.0 8.5 9.0 9.5\n"
    )
    path = tmp_path / "test.dem"
    path.write_text(content)
    return str(path)


def test_parse_dem_shape_and_origin(synthetic_dem_file):
    grid = parse_dem(synthetic_dem_file)
    assert grid.elevation.shape == (3, 4)
    assert grid.cellsize == pytest.approx(5.0)
    assert grid.xllcorner == pytest.approx(100.0)
    assert grid.yllcorner == pytest.approx(200.0)


def test_parse_dem_row_major_north_first(synthetic_dem_file):
    """Row 0 must be the northwest-starting row (first data row in the
    file, per Readme.topo)."""
    grid = parse_dem(synthetic_dem_file)
    np.testing.assert_allclose(grid.elevation[0], [10.0, 10.5, 11.0, 11.5])
    np.testing.assert_allclose(grid.elevation[-1], [8.0, 8.5, 9.0, 9.5])


def test_parse_dem_nodata_becomes_nan(tmp_path):
    content = (
        "ncols 2\nnrows 2\nxllcorner 0\nyllcorner 0\ncellsize 5\nNODATA_value -9999\n"
        "1.0 -9999\n2.0 3.0\n"
    )
    path = tmp_path / "nodata.dem"
    path.write_text(content)
    grid = parse_dem(str(path))
    assert np.isnan(grid.elevation[0, 1])
    assert grid.elevation[0, 0] == pytest.approx(1.0)


def test_parse_dem_raises_with_header_shown_on_bad_format(tmp_path):
    path = tmp_path / "bad.dem"
    path.write_text("this is not\na valid header\nat all\n1.0 2.0\n")
    with pytest.raises(ValueError, match="Could not parse DEM header"):
        parse_dem(str(path))


@pytest.fixture
def real_format_dem_file(tmp_path):
    """A tiny grid in the CONFIRMED-REAL Tarrawarra header format (verified
    against an actual downloaded tarrautm.dem in Session 3 -- see parsers.py's
    module docstring). 2 rows x 3 cols, 5.0m cells: north-south=10 over 2
    rows, east-west=15 over 3 cols -> both give cellsize=5.0."""
    content = (
        "Copyright (c) 1995-1998 Centre for Environmental Applied Hydrology, "
        "The University of Melbourne.\n"
        "\n"
        "north: 110.00\n"
        "south: 100.00\n"
        "east: 115.00\n"
        "west: 100.00\n"
        "rows: 2\n"
        "cols: 3\n"
        "10.0 10.5 11.0\n"
        "0.00 9.5 10.0\n"
    )
    path = tmp_path / "real_format.dem"
    path.write_text(content)
    return str(path)


def test_parse_dem_real_tarrawarra_format_shape_and_origin(real_format_dem_file):
    grid = parse_dem(real_format_dem_file)
    assert grid.elevation.shape == (2, 3)
    assert grid.cellsize == pytest.approx(5.0)
    assert grid.xllcorner == pytest.approx(100.0)  # west
    assert grid.yllcorner == pytest.approx(100.0)  # south


def test_parse_dem_real_tarrawarra_format_row_order(real_format_dem_file):
    """Row 0 (first data row after the header) is the north edge, matching
    the ESRI-format convention this dataset also follows."""
    grid = parse_dem(real_format_dem_file)
    np.testing.assert_allclose(grid.elevation[0], [10.0, 10.5, 11.0])


def test_parse_dem_real_tarrawarra_format_zero_is_nodata(real_format_dem_file):
    """0.00 is this dataset's fill value for cells outside the surveyed
    catchment -- confirmed by the border-of-zeros pattern in the real file."""
    grid = parse_dem(real_format_dem_file)
    assert np.isnan(grid.elevation[1, 0])


def test_parse_dem_real_format_takes_priority_over_esri_fallback(tmp_path):
    """If a file happens to parse under both header conventions, the
    confirmed-real Tarrawarra format must win -- it's the one actually
    verified against a real file, the ESRI path is an unconfirmed guess."""
    content = "north: 20.00\nsouth: 10.00\neast: 20.00\nwest: 10.00\nrows: 2\ncols: 2\n1.0 2.0\n3.0 4.0\n"
    path = tmp_path / "ambiguous.dem"
    path.write_text(content)
    grid = parse_dem(str(path))
    assert grid.elevation.shape == (2, 2)
    assert grid.cellsize == pytest.approx(5.0)


def test_parse_dem_real_format_disagreeing_cellsize_raises(tmp_path):
    """A non-square-celled header (east-west and north-south derived
    cellsizes disagree) should raise rather than silently pick one."""
    content = "north: 100.00\nsouth: 0.00\neast: 50.00\nwest: 0.00\nrows: 10\ncols: 10\n" + (
        " ".join(["1.0"] * 10) + "\n"
    ) * 10
    path = tmp_path / "nonsquare.dem"
    path.write_text(content)
    with pytest.raises(ValueError, match="cellsize disagrees"):
        parse_dem(str(path))


def test_parse_tdr_file(tmp_path):
    """Per Readme.tdr: 'date, time, x, y, dielectric constant, moisture'."""
    content = (
        "TDR data header line, grid info, whatever\n"
        "27/09/95 09:15 10.0 20.0 15.3 32.1\n"
        "27/09/95 09:16 10.0 40.0 12.1 28.5\n"
    )
    path = tmp_path / "sm270995.tdr"
    path.write_text(content)
    records = parse_tdr_file(str(path))
    assert len(records) == 2
    assert records[0].x == pytest.approx(10.0)
    assert records[0].y == pytest.approx(20.0)
    assert records[0].moisture_pct == pytest.approx(32.1)
    assert records[1].moisture_pct == pytest.approx(28.5)


def test_parse_tdr_skips_header_lines(tmp_path):
    """A header line with stray tokens that don't parse as 6 numeric-tailed
    fields must be silently skipped, not raise."""
    content = "header with six words in it too\n10/11/96 08:00 5.0 5.0 20.0 40.0\n"
    path = tmp_path / "test.tdr"
    path.write_text(content)
    records = parse_tdr_file(str(path))
    assert len(records) == 1


def test_parse_ksat_file(tmp_path):
    """5 whitespace fields: x, y, [bottom/well-base depth cm], [top/water
    depth cm], ksat (mm/hr) -- field 5 (ksat) confirmed against the real
    ksat.dat fetched in Session 3 (see module docstring); fields 3/4's
    exact semantic ordering doesn't affect anything Eq. 1 uses (only
    ksat_mm_hr, the 5th field, feeds redistribution.py)."""
    content = "header\n10.0 20.0 50.0 25.0 12.5\n30.0 40.0 55.0 30.0 8.2\n"
    path = tmp_path / "ksat.dat"
    path.write_text(content)
    records = parse_ksat_file(str(path))
    assert len(records) == 2
    assert records[0].ksat_mm_hr == pytest.approx(12.5)
    assert records[1].depth_water_cm == pytest.approx(30.0)


def test_parse_ksat_file_against_real_downloaded_data():
    """If the real ksat.dat (fetched live in Session 3 -- see README.md) is
    present, parse it end-to-end and sanity-check against the actual table
    values observed at fetch time, rather than only against synthetic data."""
    real_path = Path(__file__).parent.parent / "data" / "ksat.dat"
    if not real_path.exists():
        pytest.skip("real data/ksat.dat not present (see README.md)")
    records = parse_ksat_file(str(real_path))
    assert len(records) == 42
    first = records[0]
    assert first.x == pytest.approx(1241)
    assert first.y == pytest.approx(876)
    assert first.ksat_mm_hr == pytest.approx(74.3)
    # Every real ksat value must be non-negative (it's a conductivity).
    assert all(r.ksat_mm_hr >= 0 for r in records)


def test_parse_particle_file_hyphenated_depth_range(tmp_path):
    content = "header\n10.0 20.0 0-10 5.0 20.0 15.0 10.0 20.0\n"
    path = tmp_path / "particle.dat"
    path.write_text(content)
    records = parse_particle_file(str(path))
    assert len(records) == 1
    r = records[0]
    assert r.depth_range_cm == "0-10"
    assert r.stone_pct == pytest.approx(5.0)
    # clay is the residual: 100 - 20 - 15 - 10 - 20 = 35
    assert r.clay_pct == pytest.approx(35.0)


def test_parse_nmm_file_multiple_profiles(tmp_path):
    """Per Readme.nmm: header of unspecified length, then blank-line-
    separated date/time + depth/moisture blocks."""
    content = (
        "Site: 1\n"
        "Coordinates: 1200 900\n"
        "Depth to bedrock: 150cm\n"
        "Profile: clay loam\n"
        "\n"
        "27/09/1995 0915\n"
        "15 32.1\n"
        "30 35.4\n"
        "45 38.0\n"
        "\n"
        "14/02/1996 1030\n"
        "15 18.2\n"
        "30 22.5\n"
    )
    path = tmp_path / "tube_1.dat"
    path.write_text(content)
    profiles = parse_nmm_file(str(path))
    assert len(profiles) == 2
    assert profiles[0].site == 1
    assert profiles[0].date == "27/09/1995"
    assert profiles[0].time == "0915"
    np.testing.assert_allclose(profiles[0].depths_cm, [15, 30, 45])
    np.testing.assert_allclose(profiles[0].moisture_pct, [32.1, 35.4, 38.0])
    assert profiles[1].date == "14/02/1996"
    assert len(profiles[1].depths_cm) == 2


def test_parse_nmm_file_site_number_inferred_from_filename(tmp_path):
    content = "header\n\n01/01/1996 1200\n15 20.0\n"
    path = tmp_path / "tube_17.dat"
    path.write_text(content)
    profiles = parse_nmm_file(str(path))
    assert profiles[0].site == 17


def test_parse_nmm_file_site_number_explicit_override(tmp_path):
    content = "header\n\n01/01/1996 1200\n15 20.0\n"
    path = tmp_path / "not_named_by_convention.dat"
    path.write_text(content)
    profiles = parse_nmm_file(str(path), site=5)
    assert profiles[0].site == 5


def test_parse_nmm_file_no_profiles_raises(tmp_path):
    path = tmp_path / "tube_1.dat"
    path.write_text("just a header\nwith no date lines at all\n")
    with pytest.raises(ValueError, match="No NMM profiles parsed"):
        parse_nmm_file(str(path))


def test_parse_layer_file_with_and_without_b2(tmp_path):
    """Per Readme.soil: B2 horizon columns are optional per-row ('[if present]')."""
    content = "header\n10.0 20.0 20.0 45.0 silt\n30.0 40.0 15.0 40.0 clay 70.0 silt-clay\n"
    path = tmp_path / "layer.dat"
    path.write_text(content)
    records = parse_layer_file(str(path))
    assert len(records) == 2
    assert records[0].depth_b2_cm is None
    assert records[0].texture_b2 is None
    assert records[1].depth_b2_cm == pytest.approx(70.0)
    assert records[1].texture_b2 == "silt-clay"
