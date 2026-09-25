"""Tests for validation/shalehills/stage2.py (Session 10). Real-data tests
skip gracefully if the data isn't present.
"""

import datetime
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from parsers import MeteoRecord, parse_dem_geotiff  # noqa: E402
from stage2 import (  # noqa: E402
    coarse_averaged_soil_params,
    compute_daily_ep_mm,
    compute_point_slope_aspect,
    load_all_meteo,
    soil_params_for_points,
)
from physics.soil_texture import NoahSoilParams  # noqa: E402

DATA_DIR = Path(__file__).parent.parent / "data"


def _skip_if_absent(*paths: Path):
    for p in paths:
        if not p.exists():
            pytest.skip(f"real data {p} not present (see README.md)")


class TestComputeDailyEpMm:
    def test_missing_day_returns_none(self):
        assert compute_daily_ep_mm({}, datetime.date(2010, 1, 1), elevation_m=285.0) is None

    def test_too_few_valid_scans_returns_none(self):
        """Fewer than 6 valid scans in a day (out of an expected ~144) is
        not a trustworthy daily aggregate."""
        records = [
            MeteoRecord(
                timestamp=datetime.datetime(2010, 6, 15, 12, 0),
                pressure_kpa=97.5,
                temperature_c=20.0,
                relative_humidity_pct=60.0,
                net_radiation_w_m2=400.0,
            )
        ]
        by_date = {datetime.date(2010, 6, 15): records}
        assert compute_daily_ep_mm(by_date, datetime.date(2010, 6, 15), elevation_m=285.0) is None

    def test_real_data_returns_plausible_summer_value(self):
        _skip_if_absent(DATA_DIR / "met")
        met_by_date = load_all_meteo(DATA_DIR / "met")
        ep = compute_daily_ep_mm(met_by_date, datetime.date(2010, 6, 15), elevation_m=285.0)
        assert ep is not None
        # A forested PA site in mid-June: a plausible reference-ET range,
        # not a tight bound (real weather varies day to day).
        assert 0.5 < ep < 8.0


class TestSoilParamsForPoints:
    def test_real_data(self):
        _skip_if_absent(
            DATA_DIR / "ssurgo" / "mapunit.txt", DATA_DIR / "ssurgo" / "comp.txt",
            DATA_DIR / "ssurgo" / "chorizon.txt", DATA_DIR / "ssurgo" / "soilmu_subset.shp",
        )
        points = np.array([[254435.0, 4505558.0], [254500.0, 4505600.0]])
        params = soil_params_for_points(
            points,
            DATA_DIR / "ssurgo" / "mapunit.txt",
            DATA_DIR / "ssurgo" / "comp.txt",
            DATA_DIR / "ssurgo" / "chorizon.txt",
            DATA_DIR / "ssurgo" / "soilmu_subset.shp",
        )
        assert len(params) == 2
        for p in params:
            assert p.maxsmc > p.wltsmc > 0  # physically required: porosity > wilting point


class TestCoarseAveragedSoilParams:
    def test_simple_mean(self):
        params = [
            NoahSoilParams(satdk_m_per_s=1e-6, maxsmc=0.4, refsmc=0.3, wltsmc=0.1),
            NoahSoilParams(satdk_m_per_s=3e-6, maxsmc=0.6, refsmc=0.5, wltsmc=0.2),
        ]
        coarse = coarse_averaged_soil_params(params)
        assert coarse.maxsmc == pytest.approx(0.5)
        assert coarse.wltsmc == pytest.approx(0.15)


class TestComputePointSlopeAspect:
    def test_flat_dem_gives_zero_slope(self):
        elevation = np.full((10, 10), 100.0)
        points = np.array([[5.0, 5.0]])
        slope, _aspect = compute_point_slope_aspect(elevation, cellsize=1.0, xllcorner=0.0, yllcorner=0.0, points_xy=points)
        assert slope[0] == pytest.approx(0.0, abs=1e-6)

    def test_real_data(self):
        _skip_if_absent(DATA_DIR / "shalehills_dem_3m_nad83utm18n.tif")
        dem = parse_dem_geotiff(str(DATA_DIR / "shalehills_dem_3m_nad83utm18n.tif"))
        points = np.array([[254435.0, 4505558.0]])
        slope, aspect = compute_point_slope_aspect(
            dem.elevation, dem.cellsize, dem.xllcorner, dem.yllcorner, points
        )
        assert 0.0 <= slope[0] < 5.0  # a real, but not absurd, slope (tan of a steep hillslope)
        assert 0.0 <= aspect[0] <= 360.0
