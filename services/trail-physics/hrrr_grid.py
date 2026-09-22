"""HRRR's Lambert Conformal Conic grid: geographic <-> grid-index transform.

Direct transcription of `crates/projection/src/lambert.rs`'s
`LambertConformal::hrrr()` / `geo_to_grid` / `grid_to_geo` -- NOT a fresh
implementation using pyproj's generic LCC support. This matters: the Rust
code's projection origin convention (rho0 computed relative to the grid's
own first point, not a "nice" cartographic lat_0) is idiosyncratic, and a
generic pyproj LCC definition would need careful false-easting/origin
reconciliation to align pixel-for-pixel with the Zarr grids the ingester
already wrote -- reconciliation that can't be verified without live grids to
test against. Transcribing the already-tested, already-in-production Rust
math directly sidesteps that risk entirely.

Cross-validated against the actual Rust implementation (not just internal
round-trip consistency): a throwaway `cargo run --example` against the real
`LambertConformal::hrrr()` produced exact reference (i, j) values for three
points, asserted verbatim in test_hrrr_grid.py. This is stronger proof of
port correctness than round-tripping alone -- it confirms bit-for-bit
agreement with the code that actually wrote the Zarr grids, not just
internal self-consistency of a possibly-differently-wrong reimplementation.

This is exactly the gap flagged during the trail-conditions ingredients
session: "the Lambert projection *parameters* ... are not stored anywhere
in the catalog or zarr attrs -- they're hardcoded in
`crates/projection/src/lambert.rs`... A Python consumer must replicate
these constants." This module is that replication, done carefully.
"""

from __future__ import annotations

import math
from dataclasses import dataclass


@dataclass
class HrrrGrid:
    lon0: float  # central meridian (LoV), radians
    lat0: float  # reference latitude (= lat1, the first grid point), radians
    latin1: float
    latin2: float
    lat1: float  # first grid point latitude, radians
    lon1: float  # first grid point longitude, radians
    dx: float
    dy: float
    nx: int
    ny: int
    earth_radius: float
    n: float  # cone constant
    f: float
    rho0: float

    @classmethod
    def hrrr(cls) -> "HrrrGrid":
        """HRRR's published grid definition (matches
        `LambertConformal::hrrr()` exactly): first point 21.138123N,
        122.719528W; LoV 97.5W; standard parallels 38.5N (tangent cone);
        3km spacing; 1799x1059."""
        return cls.from_grib2(
            lat1_deg=21.138123,
            lon1_deg=-122.719528,
            lov_deg=-97.5,
            latin1_deg=38.5,
            latin2_deg=38.5,
            dx=3000.0,
            dy=3000.0,
            nx=1799,
            ny=1059,
        )

    @classmethod
    def from_grib2(
        cls,
        lat1_deg: float,
        lon1_deg: float,
        lov_deg: float,
        latin1_deg: float,
        latin2_deg: float,
        dx: float,
        dy: float,
        nx: int,
        ny: int,
    ) -> "HrrrGrid":
        lat1 = math.radians(lat1_deg)
        lon1 = math.radians(lon1_deg)
        lon0 = math.radians(lov_deg)
        latin1 = math.radians(latin1_deg)
        latin2 = math.radians(latin2_deg)

        earth_radius = 6371229.0  # matches lambert.rs -- NCEP/GRIB2 spherical earth

        if abs(latin1 - latin2) < 1e-10:
            n = math.sin(latin1)
        else:
            ln_ratio = math.log(math.cos(latin1) / math.cos(latin2))
            tan_ratio = math.log(
                math.tan(math.pi / 4.0 + latin2 / 2.0) / math.tan(math.pi / 4.0 + latin1 / 2.0)
            )
            n = ln_ratio / tan_ratio

        f = (math.cos(latin1) * math.tan(math.pi / 4.0 + latin1 / 2.0) ** n) / n
        rho0 = earth_radius * f / math.tan(math.pi / 4.0 + lat1 / 2.0) ** n
        lat0 = lat1

        return cls(
            lon0=lon0,
            lat0=lat0,
            latin1=latin1,
            latin2=latin2,
            lat1=lat1,
            lon1=lon1,
            dx=dx,
            dy=dy,
            nx=nx,
            ny=ny,
            earth_radius=earth_radius,
            n=n,
            f=f,
            rho0=rho0,
        )

    def _normalize_dlon(self, dlon: float) -> float:
        while dlon > math.pi:
            dlon -= 2.0 * math.pi
        while dlon < -math.pi:
            dlon += 2.0 * math.pi
        return dlon

    def geo_to_grid(self, lat_deg: float, lon_deg: float) -> tuple[float, float]:
        """(lat, lon) in degrees -> (i, j) fractional grid indices.
        j=0 is the grid's first (southernmost, for HRRR) row."""
        lat = math.radians(lat_deg)
        lon = math.radians(lon_deg)

        dlon = self._normalize_dlon(lon - self.lon0)
        rho = self.earth_radius * self.f / math.tan(math.pi / 4.0 + lat / 2.0) ** self.n
        theta = self.n * dlon
        x = rho * math.sin(theta)
        y = self.rho0 - rho * math.cos(theta)

        dlon0 = self._normalize_dlon(self.lon1 - self.lon0)
        theta0 = self.n * dlon0
        x0 = self.rho0 * math.sin(theta0)
        y0 = self.rho0 - self.rho0 * math.cos(theta0)

        i = (x - x0) / self.dx
        j = (y - y0) / self.dy
        return i, j

    def grid_to_geo(self, i: float, j: float) -> tuple[float, float]:
        """(i, j) fractional grid indices -> (lat, lon) in degrees."""
        dlon0 = self._normalize_dlon(self.lon1 - self.lon0)
        theta0 = self.n * dlon0
        x0 = self.rho0 * math.sin(theta0)
        y0 = self.rho0 - self.rho0 * math.cos(theta0)

        x = x0 + i * self.dx
        y = y0 + j * self.dy

        rho = math.sqrt(x * x + (self.rho0 - y) ** 2)
        if self.n < 0.0:
            rho = -rho
        # NOTE: plain atan (not atan2), matching lambert.rs exactly -- see
        # this module's docstring on why exact transcription (quirks
        # included) beats an independently "corrected" reimplementation.
        theta = math.atan(x / (self.rho0 - y))

        lat = 2.0 * math.atan((self.earth_radius * self.f / rho) ** (1.0 / self.n)) - math.pi / 2.0
        lon = self.lon0 + theta / self.n
        return math.degrees(lat), math.degrees(lon)
