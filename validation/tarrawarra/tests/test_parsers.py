"""Parser tests against synthetic files built to match the documented
Tarrawarra formats exactly (see parsers.py's module docstring for the
format sources). Proves the parsing logic is correct independent of
whether the real files are available this session.
"""

import numpy as np
import pytest

from parsers import (
    parse_dem,
    parse_ksat_file,
    parse_layer_file,
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
    """Per Readme.soil: x, y, depth to well base (cm), depth to water (cm), ksat (mm/hr)."""
    content = "header\n10.0 20.0 50.0 25.0 12.5\n30.0 40.0 55.0 30.0 8.2\n"
    path = tmp_path / "ksat.dat"
    path.write_text(content)
    records = parse_ksat_file(str(path))
    assert len(records) == 2
    assert records[0].ksat_mm_hr == pytest.approx(12.5)
    assert records[1].depth_water_cm == pytest.approx(30.0)


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
