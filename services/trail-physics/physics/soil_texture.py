"""USDA soil texture classification and Noah-LSM soil hydraulic parameter
lookup, built from two independently-verified public-domain sources:

1. **USDA texture triangle boundaries**: transcribed from the `USDA.dat`
   table bundled in the `soiltexture` PyPI package (GPLv3-licensed
   *code*; the boundary *data* itself is the underlying USDA scheme,
   attributed there to "Soil Survey Division Staff. 1993. Soil survey
   manual. USDA Handb. 18. U.S. Gov. Print Office, Washington, DC." --
   public-domain US government data). Deliberately re-implemented here
   as a small, dependency-free (no matplotlib) point-in-polygon
   classifier rather than depending on that GPL package directly, to
   avoid pulling a copyleft dependency and a heavy plotting library into
   this project for what is fundamentally a lookup table.

2. **Noah LSM soil hydraulic parameters (SOILPARM.TBL, STAS table)**:
   fetched live from `wrf-model/WRF`'s own `run/SOILPARM.TBL` (the
   authoritative source -- this is the actual file the Noah land surface
   model reads at runtime). The STAS table (not STAS-RUC) is used
   specifically because it's the classic Noah/Cosby-et-al. pedotransfer
   table, matching the GeoWATCH paper's own cited lineage (Ek et al.
   2003, "Implementation of Noah land surface model advances...").

Session 6 motivation: the GeoWATCH paper's Tarrawarra validation
(Section 4.2.1) states it used "site average soil moisture, 5-m DEM, and
soil texture data provided by the study authors" as inputs -- NOT the
measured saturated-hydraulic-conductivity field (`ksat.dat`) that Session
4/5's Rung 1 harness used instead. This module exists to test that
alternative: derive Ks (and theta_wilt/theta_ref/theta_s, useful later for
Eq. 4/5) from soil texture class via the same Noah lookup table GeoWATCH's
own flux-physics lineage is built on, instead of from measured
conductivity.
"""

from __future__ import annotations

from dataclasses import dataclass

# =============================================================================
# USDA texture triangle (sand %, clay %) polygon vertices.
# Source: USDA.dat from the soiltexture PyPI package (v1.0.4), itself
# citing "Soil Survey Division Staff. 1993. Soil survey manual. USDA
# Handb. 18." Vertex counts cross-checked against that source's own
# declared per-class vertex counts before transcription (6,4,5,4,6,5,5,7,
# 6,4,5,8 for classes 1-12 respectively) -- all matched exactly.
# =============================================================================

USDA_TEXTURE_TRIANGLE: dict[str, list[tuple[float, float]]] = {
    "CLAY": [(0, 100), (0, 60), (20, 40), (45, 40), (45, 55), (0, 100)],
    "SILTY CLAY": [(0, 60), (0, 40), (20, 40), (0, 60)],
    "SILTY CLAY LOAM": [(0, 40), (0, 27), (20, 27), (20, 40), (0, 40)],
    "SANDY CLAY": [(45, 55), (45, 35), (65, 35), (45, 55)],
    "SANDY CLAY LOAM": [(45, 35), (45, 27), (52, 20), (80, 20), (65, 35), (45, 35)],
    "CLAY LOAM": [(20, 40), (20, 27), (45, 27), (45, 40), (20, 40)],
    "SILT": [(0, 12), (0, 0), (20, 0), (8, 12), (0, 12)],
    "SILT LOAM": [(8, 12), (20, 0), (50, 0), (23, 27), (0, 27), (0, 12), (8, 12)],
    "LOAM": [(23, 27), (43, 7), (52, 7), (52, 20), (45, 27), (23, 27)],
    "SAND": [(85, 0), (100, 0), (90, 10), (85, 0)],
    "LOAMY SAND": [(70, 0), (85, 0), (90, 10), (85, 15), (70, 0)],
    "SANDY LOAM": [
        (43, 7),
        (50, 0),
        (70, 0),
        (85, 15),
        (80, 20),
        (52, 20),
        (52, 7),
        (43, 7),
    ],
}


def _point_in_polygon(x: float, y: float, vertices: list[tuple[float, float]]) -> bool:
    """Standard ray-casting point-in-polygon test. No external dependency
    (deliberately not using matplotlib.path, which the reference
    `soiltexture` package uses, to keep this module dependency-free)."""
    n = len(vertices)
    inside = False
    x1, y1 = vertices[0]
    for i in range(1, n + 1):
        x2, y2 = vertices[i % n]
        if y1 == y2:
            # Handle a horizontal edge lying exactly on the test point's
            # y-level as a boundary hit, not a crossing (avoids the classic
            # ray-casting degenerate case for triangle-boundary points,
            # which matter here since real samples often land close to or
            # on class boundaries).
            if y == y1 and min(x1, x2) <= x <= max(x1, x2):
                return True
        elif (y1 > y) != (y2 > y):
            x_intersect = x1 + (y - y1) * (x2 - x1) / (y2 - y1)
            if x_intersect > x:
                inside = not inside
            elif x_intersect == x:
                return True  # on the edge
        x1, y1 = x2, y2
    return inside


def classify_usda_texture(sand_pct: float, clay_pct: float) -> str:
    """Classify a soil sample into one of the 12 core USDA texture classes
    from sand% and clay% (silt% is implied as 100 - sand - clay, per the
    USDA texture triangle's own convention -- not needed as a separate
    input).

    Falls back to the nearest polygon (by centroid distance) if the point
    doesn't fall cleanly inside any polygon -- real lab measurements
    occasionally sum to slightly more/less than 100% (rounding in
    particle-size analysis), which can place a point a fraction of a
    percent outside every polygon's boundary. Raises only if the input is
    wildly invalid (negative, or sand+clay > 100 by a large margin).
    """
    if sand_pct < 0 or clay_pct < 0:
        raise ValueError(f"sand_pct and clay_pct must be non-negative, got {sand_pct}, {clay_pct}")
    if sand_pct + clay_pct > 105.0:
        raise ValueError(
            f"sand_pct + clay_pct = {sand_pct + clay_pct} exceeds 100% by more than a small "
            f"rounding tolerance -- check inputs."
        )

    for name, vertices in USDA_TEXTURE_TRIANGLE.items():
        if _point_in_polygon(sand_pct, clay_pct, vertices):
            return name

    # Fallback: nearest polygon centroid (handles small rounding overshoot
    # just outside every polygon's boundary).
    best_name, best_dist = None, float("inf")
    for name, vertices in USDA_TEXTURE_TRIANGLE.items():
        cx = sum(v[0] for v in vertices) / len(vertices)
        cy = sum(v[1] for v in vertices) / len(vertices)
        dist = (cx - sand_pct) ** 2 + (cy - clay_pct) ** 2
        if dist < best_dist:
            best_name, best_dist = name, dist
    assert best_name is not None
    return best_name


# =============================================================================
# Noah LSM soil hydraulic parameters (SOILPARM.TBL, STAS table).
# Fetched live from https://raw.githubusercontent.com/wrf-model/WRF/master/
# run/SOILPARM.TBL (Session 6). Units as given in that file: SATDK in m/s,
# volumetric fractions (MAXSMC/REFSMC/WLTSMC) dimensionless (m3/m3).
# =============================================================================


@dataclass(frozen=True)
class NoahSoilParams:
    satdk_m_per_s: float  # saturated hydraulic conductivity
    maxsmc: float  # theta_s -- saturated soil moisture (porosity)
    refsmc: float  # theta_ref -- field capacity
    wltsmc: float  # theta_wilt -- wilting point


NOAH_SOILPARM_STAS: dict[str, NoahSoilParams] = {
    "SAND": NoahSoilParams(4.66e-5, 0.339, 0.192, 0.010),
    "LOAMY SAND": NoahSoilParams(1.41e-5, 0.421, 0.283, 0.028),
    "SANDY LOAM": NoahSoilParams(5.23e-6, 0.434, 0.312, 0.047),
    "SILT LOAM": NoahSoilParams(2.81e-6, 0.476, 0.360, 0.084),
    "SILT": NoahSoilParams(2.18e-6, 0.484, 0.347, 0.061),
    "LOAM": NoahSoilParams(3.38e-6, 0.439, 0.329, 0.066),
    "SANDY CLAY LOAM": NoahSoilParams(4.45e-6, 0.404, 0.315, 0.069),
    "SILTY CLAY LOAM": NoahSoilParams(2.03e-6, 0.464, 0.387, 0.120),
    "CLAY LOAM": NoahSoilParams(2.45e-6, 0.465, 0.382, 0.103),
    "SANDY CLAY": NoahSoilParams(7.22e-6, 0.406, 0.338, 0.100),
    "SILTY CLAY": NoahSoilParams(1.34e-6, 0.468, 0.404, 0.126),
    "CLAY": NoahSoilParams(9.74e-7, 0.468, 0.412, 0.138),
}


def soil_hydraulic_properties(sand_pct: float, clay_pct: float) -> NoahSoilParams:
    """Classify by texture (USDA triangle) then look up Noah's STAS
    hydraulic parameters for that class -- the single function most
    callers want."""
    texture = classify_usda_texture(sand_pct, clay_pct)
    return NOAH_SOILPARM_STAS[texture]
