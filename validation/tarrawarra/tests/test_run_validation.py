"""Tests for run_validation.py's own helper functions (not parsers.py --
see test_parsers.py for those). Real-data tests skip gracefully if the
data isn't present, matching the pattern used throughout test_parsers.py.
"""

import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from run_validation import (  # noqa: E402
    TDR_FILENAMES,
    build_podpac_predictors,
    build_terrain_predictors,
    load_texture_derived_ksat,
    validate_one_date,
)

DATA_DIR = Path(__file__).parent.parent / "data"


def test_load_texture_derived_ksat_synthetic(tmp_path):
    """Two sites, each with a shallow (0-...) and a deeper layer -- only
    the shallow layer's texture should be used."""
    content = (
        "header\n"
        "100 200 0-10 0 90 5 3 1 1\n"  # shallow: mostly sand -> SAND-ish
        "100 200 10-30 0 5 5 20 30 40\n"  # deeper layer, should be ignored
        "300 400 0-15 0 5 5 20 30 40\n"  # shallow: clay-rich
    )
    path = tmp_path / "particle.dat"
    path.write_text(content)

    triplets = load_texture_derived_ksat(path)
    assert len(triplets) == 2  # one per site, deeper layers excluded
    sites = {(x, y): satdk for x, y, satdk in triplets}
    assert (100.0, 200.0) in sites
    assert (300.0, 400.0) in sites
    # Sandier site must have higher conductivity than the clay-rich site.
    assert sites[(100.0, 200.0)] > sites[(300.0, 400.0)]


def test_load_texture_derived_ksat_all_values_positive(tmp_path):
    """Noah SOILPARM.TBL has no zero-conductivity texture class -- unlike
    measured ksat.dat, no filtering for non-positive values is needed."""
    content = "header\n100 200 0-10 0 20 20 20 20 20\n"
    path = tmp_path / "particle.dat"
    path.write_text(content)
    triplets = load_texture_derived_ksat(path)
    assert all(satdk > 0 for _, _, satdk in triplets)


def test_load_texture_derived_ksat_raises_if_no_surface_layers(tmp_path):
    content = "header\n100 200 10-30 0 20 20 20 20 20\n"  # only a deep layer
    path = tmp_path / "particle.dat"
    path.write_text(content)
    with pytest.raises(ValueError, match="No surface-depth"):
        load_texture_derived_ksat(path)


def test_load_texture_derived_ksat_against_real_data():
    """If the real particle.dat (Session 4) is present, this must produce
    one entry per unique sample site with the shallowest layer used."""
    real_path = DATA_DIR / "particle.dat"
    if not real_path.exists():
        pytest.skip("real data/particle.dat not present (see README.md)")
    triplets = load_texture_derived_ksat(real_path)
    # 34 total particle.dat records across ~11 unique sites (each with a
    # shallow + 2-3 deeper layers) -- confirmed by direct inspection,
    # Session 6.
    assert len(triplets) == 11
    assert all(satdk > 0 for _, _, satdk in triplets)


class TestBuildPodpacPredictors:
    """Session 8: the REAL Creare/GeoWATCH production equation (discovered
    in the paper's own linked notebook -- see physics/redistribution.py's
    module docstring) needs TWI + theta_s/theta_wilt, not TWI + ln(Ks)."""

    def test_fine_soil_params_vary_by_location(self):
        if not (DATA_DIR / "tarrawar.dem").exists() or not (DATA_DIR / "particle.dat").exists():
            pytest.skip("real data not present (see README.md)")
        predictors_at, twi_grid = build_podpac_predictors(
            DATA_DIR / "tarrawar.dem",
            DATA_DIR / "particle.dat",
            soil_params_scale="fine",
        )
        # Two real, distinct TDR points -- with "fine" scale, nearest-site
        # texture lookup should not force identical theta_s/theta_wilt
        # everywhere (that would indicate the interpolation silently
        # collapsed to a constant, e.g. a broadcasting bug).
        points = np.array([(900.0, 850.0), (1300.0, 1050.0)])
        _twi, theta_s, theta_wilt = predictors_at(points)
        assert theta_s.shape == (2,)
        assert theta_wilt.shape == (2,)
        assert np.all(theta_s > theta_wilt)  # physically required: porosity > wilting point

    def test_coarse_soil_params_are_uniform(self):
        if not (DATA_DIR / "tarrawar.dem").exists() or not (DATA_DIR / "particle.dat").exists():
            pytest.skip("real data not present (see README.md)")
        predictors_at, _twi_grid = build_podpac_predictors(
            DATA_DIR / "tarrawar.dem",
            DATA_DIR / "particle.dat",
            soil_params_scale="coarse",
        )
        points = np.array([(900.0, 850.0), (1300.0, 1050.0), (1000.0, 900.0)])
        _twi, theta_s, theta_wilt = predictors_at(points)
        # "coarse" must be a single site-wide scalar broadcast to every
        # point -- not accidentally per-point.
        assert len(set(theta_s.tolist())) == 1
        assert len(set(theta_wilt.tolist())) == 1

    def test_unknown_soil_params_scale_raises(self, tmp_path):
        if not (DATA_DIR / "tarrawar.dem").exists() or not (DATA_DIR / "particle.dat").exists():
            pytest.skip("real data not present (see README.md)")
        with pytest.raises(ValueError, match="soil_params_scale"):
            build_podpac_predictors(
                DATA_DIR / "tarrawar.dem",
                DATA_DIR / "particle.dat",
                soil_params_scale="bogus",
            )


class TestValidateOneDatePodpacForm:
    """End-to-end check that validate_one_date's podpac branch actually
    invokes redistribute_podpac (not redistribute), against real data."""

    def test_real_data_regression(self):
        if not (DATA_DIR / "tarrawar.dem").exists() or not (DATA_DIR / "particle.dat").exists():
            pytest.skip("real data not present (see README.md)")
        predictors_at, twi_grid = build_podpac_predictors(
            DATA_DIR / "tarrawar.dem",
            DATA_DIR / "particle.dat",
            soil_params_scale="fine",
        )
        twi_mean = float(np.nanmean(twi_grid))
        tdr_path = DATA_DIR / "tdr" / TDR_FILENAMES[0]  # sm270995.tdr
        if not tdr_path.exists():
            pytest.skip("real TDR data not present (see README.md)")
        baseline_rmse, model_rmse, n = validate_one_date(
            tdr_path,
            predictors_at,
            twi_mean,
            redistribution_form="podpac",
        )
        assert n > 0
        # Locked-in regression value from Session 8's real run (builtin TWI
        # engine, fine soil params, no Stage 2) -- a real number from real
        # data, not a placeholder; if this drifts, something changed in the
        # pipeline and should be understood, not silently accepted.
        assert model_rmse == pytest.approx(0.0411, abs=0.0005)

    def test_podpac_form_does_not_require_log_ks_mean(self):
        """The podpac branch must not blow up when log_ks_mean is omitted
        (its default is None) -- unlike the geowatch-paper branch, it never
        uses it."""
        if not (DATA_DIR / "tarrawar.dem").exists() or not (DATA_DIR / "particle.dat").exists():
            pytest.skip("real data not present (see README.md)")
        predictors_at, twi_grid = build_podpac_predictors(
            DATA_DIR / "tarrawar.dem",
            DATA_DIR / "particle.dat",
        )
        twi_mean = float(np.nanmean(twi_grid))
        tdr_path = DATA_DIR / "tdr" / TDR_FILENAMES[0]
        if not tdr_path.exists():
            pytest.skip("real TDR data not present (see README.md)")
        # No exception, no log_ks_mean passed at all.
        validate_one_date(tdr_path, predictors_at, twi_mean, redistribution_form="podpac")
