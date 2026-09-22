"""Tests for fetch_nlcd.py's WCS request construction."""

import sys
from pathlib import Path
from urllib.parse import parse_qs, urlparse

sys.path.insert(0, str(Path(__file__).parent.parent))

from fetch_nlcd import LAND_COVER_COVERAGE_ID, build_getcoverage_url


def test_getcoverage_url_has_required_wcs_params():
    url = build_getcoverage_url(LAND_COVER_COVERAGE_ID, -105.3, 39.6, -105.1, 39.8)
    parsed = urlparse(url)
    params = parse_qs(parsed.query)
    assert params["service"] == ["WCS"]
    assert params["version"] == ["2.0.1"]
    assert params["request"] == ["GetCoverage"]
    assert params["coverageId"] == [LAND_COVER_COVERAGE_ID]


def test_getcoverage_url_subsets_both_axes():
    url = build_getcoverage_url(LAND_COVER_COVERAGE_ID, -105.3, 39.6, -105.1, 39.8)
    parsed = urlparse(url)
    params = parse_qs(parsed.query)
    subsets = params["subset"]
    assert any("Long(-105.3,-105.1)" in s for s in subsets)
    assert any("Lat(39.6,39.8)" in s for s in subsets)
