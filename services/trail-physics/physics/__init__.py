"""Trail-conditions physics core (GeoWATCH-lineage soil moisture downscaling).

Confidence levels, stated explicitly per module because this is exactly the
kind of code where fabricated precision is actively harmful. **Session 3
update: the primary source (Eylander et al. 2023) was obtained (user-
supplied PDF) and every equation below has now been transcribed directly
from it, not reconstructed from adjacent literature.**

- ``redistribution.py`` (Eq. 1): **Fully specified**, transcribed directly
  and confirmed to match docs/trail-conditions-design.md's Session 1/2
  summary exactly (k=13). This is what Rung 1 (Tarrawarra) actually
  validates -- the static-pattern topographic/soil-transmissivity
  redistribution of a known catchment-mean moisture to individual points.
  High confidence.

- ``flux.py`` (Eq. 3/4/5): **Transcribed directly from the paper (Session
  3)**. Eq. 4 (vegetation transpiration) is an exact match to Session 2's
  independent Ek et al. (2003) reconstruction -- no change needed there.
  Eq. 5 (direct soil evaporation) turned out to need two real fixes: the
  paper's `[Rd + (1-Rd)*iota]` radiative prefactor was missing entirely
  from Session 2's version, and Session 2 had *replaced* the paper's
  printed `(theta-theta_ref)/(theta_s-theta_ref)` ratio with the Ek-2003
  form, predicting (without having the paper) that the printed form must
  be a transcription error. Having read the paper: **it really does print
  that ratio, verbatim, unclipped.** Both forms are now implemented
  (`form="ek2003"` default, `form="geowatch"` paper-literal) so the
  Tarrawarra harness can empirically determine which one reproduces the
  published 0.0321/0.030 targets, rather than guessing.

- ``radiation.py`` (Eq. 6, solar view factor): the direct-beam/sky-view
  terrain-correction machinery (`terrain_corrected_shortwave`) was already
  standard, non-paywalled methodology (Session 2). `solar_view_factor()`
  (the actual Eq. 6 -- a daily integral of sun-surface alignment over the
  sun's azimuth sweep) is new in Session 3, transcribed from the paper.
  Its normalization (dividing by the flat-ground reference so iota=1 for
  flat ground) is a documented Session 3 *inference*, not stated
  explicitly in the paper -- see that function's docstring.

- ``relaxation.py`` (Eq. 2, Eq. 7): **Completely rewritten in Session 3.**
  Session 2's version (an exponential anomaly-decay model, invented without
  the paper) is gone. The paper's actual Eq. 7 is a piecewise time-to-dry
  estimate using indicator functions on theta* vs. theta_s, and Eq. 2 is a
  single additive flux-difference correction evaluated with carefully
  distinct fine-resolution vs. weather-scale-averaged soil properties
  (`SoilProperties`, kept as two separate required arguments specifically
  so that distinction can't be silently dropped). One genuine open
  question remains even with the paper in hand: the paper never states the
  depth-normalization needed to make Eq. 2's units work out (see
  relaxation.py's module docstring) -- an empirical calibration question
  for Tarrawarra to answer, same as k=13.

Nothing in this package is validated end-to-end yet. See
validation/tarrawarra/ for the Rung 1 gate (blocked on manually-acquired
data -- see that directory's README) and
docs/trail-conditions-design.md Section 8 for the full validation ladder.
"""
