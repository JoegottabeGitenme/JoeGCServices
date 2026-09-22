"""Trail-conditions physics core (GeoWATCH-lineage soil moisture downscaling).

Confidence levels, stated explicitly per module because this is exactly the
kind of code where fabricated precision is actively harmful:

- ``redistribution.py`` (Eq. 1): **Fully specified** in
  docs/trail-conditions-design.md with an explicit formula and constant
  (k=13). This is what Rung 1 (Tarrawarra) actually validates -- the
  static-pattern topographic/soil-transmissivity redistribution of a known
  catchment-mean moisture to individual points. High confidence.

- ``flux.py`` (Eq. 4/5, soil evaporation efficiency): The direct-soil-evap
  beta function is implemented per the well-established Ek et al. (2003) /
  Noah LSM formulation, which is independently documented (Chen et al. 1996)
  and is *not* paywalled -- this also happens to be the correct form our
  design doc says the source paper's Eq. 5 should have matched (the doc
  flags the paper's own text as having a transcription error here).

- ``radiation.py`` (Eq. 6, solar/sky-view correction): standard terrain
  radiation-correction form (comparable to Dozier & Frew 1990 lineage),
  not paywall-dependent.

- ``relaxation.py`` (Eq. 2, Eq. 7): **Reconstructed, not verified against
  the primary source.** Eylander et al. (2023) is behind a ScienceDirect
  paywall; every fetch attempt this session (WebFetch, dl.acm.org mirror)
  was blocked (400/403). This module implements a physically-motivated
  relaxation-time flux correction consistent with the Equilibrium Moisture
  Theory lineage the design doc itself names (Coleman & Niemann 2013 EMT;
  Ranney et al. 2015 EMT-VS) and the doc's given constants/bounds
  (Cts=0.1, Rd=0.15, dt clipped to [0, 30] days). Treat this module's
  *exact* algebraic form as unverified until checked against the primary
  paper -- the qualitative behavior (local flux anomalies relax the
  redistributed anomaly back toward the coarse-scale value over a bounded
  timescale) is well-grounded; the precise equation is not.

Nothing in this package is validated end-to-end yet. See
validation/tarrawarra/ for the Rung 1 gate (blocked on manually-acquired
data -- see that directory's README) and
docs/trail-conditions-design.md Section 8 for the full validation ladder.
"""
