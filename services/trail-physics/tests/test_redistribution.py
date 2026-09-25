"""Unit tests for Eq. 1 (topographic/transmissivity redistribution)."""

import numpy as np
import pytest

from physics.redistribution import DEFAULT_K, redistribute, redistribute_podpac


def test_uniform_terrain_returns_coarse_value_everywhere():
    """If every point has identical TWI and ln(Ks) (no spatial variability),
    Eq. 1 must reduce to exactly the coarse-scale value everywhere -- both
    correction terms vanish (each point equals its own mean)."""
    twi = np.full((5, 5), 8.0)
    log_ks = np.full((5, 5), 2.0)
    result = redistribute(theta_coarse=25.0, twi=twi, log_ks=log_ks)
    np.testing.assert_allclose(result, 25.0)


def test_wetter_topography_gives_wetter_prediction():
    """A point with above-mean TWI (more convergent, e.g. valley bottom)
    must be predicted WETTER than the catchment mean -- this is the
    physical sign-check documented in the module docstring."""
    twi = np.array([5.0, 8.0, 11.0])  # below-mean, mean, above-mean
    log_ks = np.array([2.0, 2.0, 2.0])  # uniform soil, isolate the TWI effect
    result = redistribute(theta_coarse=25.0, twi=twi, log_ks=log_ks)
    assert result[0] < result[1] < result[2]
    assert result[1] == pytest.approx(25.0)  # the mean-TWI point matches the coarse value


def test_higher_ks_gives_drier_prediction():
    """A point with above-mean ln(Ks) (more free-draining soil) must be
    predicted DRIER than the catchment mean."""
    twi = np.array([8.0, 8.0, 8.0])  # uniform terrain, isolate the Ks effect
    log_ks = np.array([1.0, 2.0, 3.0])  # below-mean, mean, above-mean
    result = redistribute(theta_coarse=25.0, twi=twi, log_ks=log_ks)
    assert result[0] > result[1] > result[2]
    assert result[1] == pytest.approx(25.0)


def test_default_k_matches_design_doc():
    """Locks in the doc-specified constant so an accidental edit doesn't
    silently change validation results without a visible test failure."""
    assert DEFAULT_K == 13.0


def test_k_scales_correction_magnitude():
    """A larger k must produce a SMALLER correction (k is a divisor) --
    doubling k should exactly halve the deviation from the coarse value."""
    twi = np.array([5.0, 11.0])
    log_ks = np.array([2.0, 2.0])
    result_k13 = redistribute(theta_coarse=25.0, twi=twi, log_ks=log_ks, k=13.0)
    result_k26 = redistribute(theta_coarse=25.0, twi=twi, log_ks=log_ks, k=26.0)
    deviation_k13 = result_k13 - 25.0
    deviation_k26 = result_k26 - 25.0
    np.testing.assert_allclose(deviation_k26, deviation_k13 / 2.0)


def test_explicit_means_override_array_means():
    """Tarrawarra validation calibrates twi_mean/log_ks_mean from the FULL
    13-date-independent terrain grid (stable across dates), not from
    whatever subset of points happen to have TDR readings on a given date --
    explicit override must take precedence over the array's own mean."""
    twi = np.array([8.0, 8.0])  # array mean = 8.0
    log_ks = np.array([2.0, 2.0])  # array mean = 2.0
    result = redistribute(
        theta_coarse=25.0, twi=twi, log_ks=log_ks, twi_mean=10.0, log_ks_mean=2.0
    )
    # twi_i (8.0) is now below the OVERRIDDEN mean (10.0) -> must be drier than 25.0
    assert np.all(result < 25.0)


def test_mass_conservation_property():
    """Averaged back over a uniform-weight grid, the redistributed field's
    mean must equal the coarse input -- Eq. 1 redistributes, it does not
    invent or destroy moisture (same discipline the design doc calls out
    for the snow-partition stage, S2)."""
    rng = np.random.default_rng(42)
    twi = rng.normal(8.0, 2.0, size=1000)
    log_ks = rng.normal(2.0, 0.5, size=1000)
    result = redistribute(theta_coarse=30.0, twi=twi, log_ks=log_ks)
    assert result.mean() == pytest.approx(30.0, abs=1e-9)


class TestRedistributePodpac:
    """Tests for the REAL Creare/GeoWATCH production equation (Session 8
    discovery -- see redistribution.py's module docstring), as published
    in the paper's own linked notebook:
    theta* = theta_coarse + (theta_s - theta_wilt)/k * (twi - twi_mean).
    """

    def test_uniform_terrain_returns_coarse_value_everywhere(self):
        """No TWI variability -> the amplitude term vanishes regardless of
        soil parameters."""
        twi = np.full(5, 8.0)
        result = redistribute_podpac(
            theta_coarse=0.25, twi=twi, theta_s=0.47, theta_wilt=0.09
        )
        np.testing.assert_allclose(result, 0.25)

    def test_wetter_topography_gives_wetter_prediction(self):
        """Same sign convention as the paper-printed form: above-mean TWI
        must predict wetter, not drier."""
        twi = np.array([5.0, 8.0, 11.0])
        result = redistribute_podpac(
            theta_coarse=0.25, twi=twi, theta_s=0.47, theta_wilt=0.09
        )
        assert result[0] < result[1] < result[2]
        assert result[1] == pytest.approx(0.25)

    def test_no_ln_ks_term(self):
        """The real equation has no soil-conductivity term at all -- unlike
        `redistribute`, only theta_s/theta_wilt (the water-holding RANGE)
        enter, and only as an amplitude scale, not an independent additive
        correction. Locked in via the actual function signature, not just
        by not passing one in a call site."""
        import inspect

        params = inspect.signature(redistribute_podpac).parameters
        assert "log_ks" not in params
        assert "log_ks_mean" not in params

    def test_amplitude_scales_with_water_holding_range(self):
        """Doubling (theta_s - theta_wilt) must exactly double the
        deviation from the coarse value -- it is a direct multiplicative
        scale on the TWI anomaly, not an offset."""
        twi = np.array([5.0, 11.0])
        result_narrow = redistribute_podpac(
            theta_coarse=0.25, twi=twi, theta_s=0.30, theta_wilt=0.20
        )  # range = 0.10
        result_wide = redistribute_podpac(
            theta_coarse=0.25, twi=twi, theta_s=0.40, theta_wilt=0.20
        )  # range = 0.20
        dev_narrow = result_narrow - 0.25
        dev_wide = result_wide - 0.25
        np.testing.assert_allclose(dev_wide, dev_narrow * 2.0)

    def test_default_k_matches_paper_printed_form(self):
        """The Session 8 discovery corrects the amplitude's STRUCTURE, not
        its calibrated value -- k=13 must remain the shared default for
        both equation forms."""
        assert redistribute_podpac.__defaults__[-1] == DEFAULT_K == 13.0

    def test_per_point_soil_params_supported(self):
        """theta_s/theta_wilt may be per-point arrays (the notebook's
        porosity/wilt nodes are evaluated at each output coordinate, not
        necessarily as a single scalar)."""
        twi = np.array([5.0, 11.0])
        theta_s = np.array([0.40, 0.50])
        theta_wilt = np.array([0.10, 0.10])
        result = redistribute_podpac(
            theta_coarse=0.25, twi=twi, theta_s=theta_s, theta_wilt=theta_wilt
        )
        assert result.shape == (2,)
        # Manually compute expected values to lock in the exact formula.
        twi_mean = twi.mean()
        expected = 0.25 + (theta_s - theta_wilt) / 13.0 * (twi - twi_mean)
        np.testing.assert_allclose(result, expected)

    def test_explicit_twi_mean_overrides_array_mean(self):
        twi = np.array([8.0, 8.0])  # array mean = 8.0
        result = redistribute_podpac(
            theta_coarse=0.25, twi=twi, theta_s=0.47, theta_wilt=0.09, twi_mean=10.0
        )
        assert np.all(result < 0.25)

    def test_mass_conservation_property(self):
        rng = np.random.default_rng(42)
        twi = rng.normal(8.0, 2.0, size=1000)
        result = redistribute_podpac(
            theta_coarse=0.30, twi=twi, theta_s=0.47, theta_wilt=0.09
        )
        assert result.mean() == pytest.approx(0.30, abs=1e-9)
