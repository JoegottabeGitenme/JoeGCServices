"""Tests for run_validation.py's own helper functions (not parsers.py --
see test_parsers.py for those). Real-data tests skip gracefully if the
data isn't present, matching the pattern used throughout test_parsers.py.
"""

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))
sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "services" / "trail-physics"))

from run_validation import load_texture_derived_ksat  # noqa: E402

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
