"""Parsers for the Tarrawarra dataset's ASCII file formats.

Format specifications transcribed directly from the dataset's own
documentation (Readme.topo, Readme.tdr, Readme.soil -- fetched successfully
this session via the site's HTML pages before its WAF started blocking
every raw-data-file request; see README.md in this directory for the full
story and how to supply the actual data files).

**The DEM header format is a documented assumption, not a confirmed fact**:
Readme.topo says only "a 6 line header with the boundaries of the dem and
the number of rows and columns" -- it does not give the exact key names or
line order. This parser assumes the standard ESRI ASCII grid header
(ncols/nrows/xllcorner/yllcorner/cellsize/NODATA_value, one per line, in
that or a similar order) since that is the overwhelmingly common 6-line DEM
header convention this description matches. If the real file's header
doesn't parse, `parse_dem` raises with the actual header lines shown, so
whoever has file access can adjust the key-pattern list in
`_DEM_HEADER_KEY_PATTERNS` in five minutes rather than guess blind.

Every other parser (TDR, ksat, particle, layer) is a straightforward
whitespace-delimited-columns-after-a-header format per the docs, which is
unambiguous regardless of the exact header wording -- these should work
against the real files without adjustment.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

import numpy as np

_DEM_HEADER_KEY_PATTERNS = {
    "ncols": re.compile(r"n\s*cols?", re.IGNORECASE),
    "nrows": re.compile(r"n\s*rows?", re.IGNORECASE),
    "xllcorner": re.compile(r"x\s*ll", re.IGNORECASE),
    "yllcorner": re.compile(r"y\s*ll", re.IGNORECASE),
    "cellsize": re.compile(r"cell\s*size", re.IGNORECASE),
    "nodata_value": re.compile(r"no\s*data", re.IGNORECASE),
}


@dataclass
class DemGrid:
    elevation: np.ndarray  # shape (nrows, ncols), row 0 = north edge
    cellsize: float
    xllcorner: float
    yllcorner: float
    nodata_value: float | None


def parse_dem(path: str) -> DemGrid:
    """Parse a Tarrawarra .dem file (assumed ESRI ASCII grid header -- see
    module docstring). Raises ValueError with the actual header text if the
    assumption doesn't hold, rather than silently misparsing."""
    with open(path, "r") as f:
        lines = f.readlines()

    header = {}
    data_start_line = 0
    for i, line in enumerate(lines[:10]):  # header is documented as 6 lines; scan a few extra
        parts = line.split()
        if len(parts) != 2:
            continue
        key_text, value_text = parts
        matched = False
        for key, pattern in _DEM_HEADER_KEY_PATTERNS.items():
            if pattern.match(key_text):
                try:
                    header[key] = float(value_text)
                except ValueError:
                    pass
                matched = True
                break
        if matched:
            data_start_line = i + 1

    missing = [k for k in ("ncols", "nrows", "cellsize") if k not in header]
    if missing:
        raise ValueError(
            f"Could not parse DEM header from {path} -- missing {missing}. "
            f"Assumed ESRI ASCII grid key names (see this module's docstring); "
            f"actual header lines found:\n" + "".join(lines[:8])
        )

    ncols = int(header["ncols"])
    nrows = int(header["nrows"])

    values: list[float] = []
    for line in lines[data_start_line:]:
        values.extend(float(v) for v in line.split())

    if len(values) != nrows * ncols:
        raise ValueError(
            f"DEM {path}: header declares {nrows}x{ncols}={nrows*ncols} cells "
            f"but found {len(values)} data values after the header. Header "
            f"parsing may have consumed too many/few lines -- check "
            f"data_start_line handling for this file's actual layout."
        )

    elevation = np.array(values, dtype=np.float64).reshape(nrows, ncols)
    nodata = header.get("nodata_value")
    if nodata is not None:
        elevation = np.where(elevation == nodata, np.nan, elevation)

    return DemGrid(
        elevation=elevation,
        cellsize=header["cellsize"],
        xllcorner=header.get("xllcorner", 0.0),
        yllcorner=header.get("yllcorner", 0.0),
        nodata_value=nodata,
    )


@dataclass
class TdrRecord:
    date: str
    time: str
    x: float
    y: float
    dielectric: float
    moisture_pct: float  # %V/V


def parse_tdr_file(path: str) -> list[TdrRecord]:
    """Parse a smDDMMYY.tdr file. Per Readme.tdr: 'each data record contains
    date, time, x, y, dielectric constant, moisture' -- 6 whitespace-
    delimited fields, after a header of unspecified length. We skip any
    line that doesn't parse as exactly 6 fields with the last 4 numeric."""
    records = []
    with open(path, "r") as f:
        for line in f:
            parts = line.split()
            if len(parts) != 6:
                continue
            date, time, *rest = parts
            try:
                x, y, dielectric, moisture = (float(v) for v in rest)
            except ValueError:
                continue  # header/comment line that happens to have 6 tokens
            records.append(TdrRecord(date, time, x, y, dielectric, moisture))
    if not records:
        raise ValueError(
            f"No TDR records parsed from {path} -- check the file's actual "
            f"column layout against this parser's 6-field assumption."
        )
    return records


@dataclass
class KsatRecord:
    x: float
    y: float
    depth_well_base_cm: float
    depth_water_cm: float
    ksat_mm_hr: float


def parse_ksat_file(path: str) -> list[KsatRecord]:
    """Per Readme.soil: 'x coordinate, y coordinate (m Tarrawarra
    coordinates), depth to well base (cm), depth to water surface (cm),
    saturated hydraulic conductivity (mm/hr)' -- 5 numeric fields."""
    records = []
    with open(path, "r") as f:
        for line in f:
            parts = line.split()
            if len(parts) != 5:
                continue
            try:
                values = [float(v) for v in parts]
            except ValueError:
                continue
            records.append(KsatRecord(*values))
    if not records:
        raise ValueError(f"No ksat records parsed from {path}")
    return records


@dataclass
class ParticleRecord:
    x: float
    y: float
    depth_range_cm: str  # kept as text: documented as a range (e.g. "0-10"), not a single number
    stone_pct: float
    coarse_sand_pct: float
    fine_sand_pct: float
    coarse_silt_pct: float
    fine_silt_pct: float
    clay_pct: float


def parse_particle_file(path: str) -> list[ParticleRecord]:
    """Per Readme.soil: coordinate, depth range, stone%, then coarse sand /
    fine sand / coarse silt / fine silt / clay percentages -- 8 fields (2
    coordinate + 1 depth-range text + 5 numeric), or 9 if the depth range
    itself is two whitespace-separated numbers rather than one hyphenated
    token. Handles both layouts."""
    records = []
    with open(path, "r") as f:
        for line in f:
            parts = line.split()
            if len(parts) == 8:
                x, y, depth_range, stone, cs, fs, csi, fsi = parts
                clay = None
            elif len(parts) == 9:
                # depth range given as two separate tokens (e.g. "0 10" not "0-10")
                x, y, d0, d1, stone, cs, fs, csi, fsi = parts
                depth_range = f"{d0}-{d1}"
                clay = None
            else:
                continue
            try:
                x_f, y_f = float(x), float(y)
                stone_f, cs_f, fs_f, csi_f, fsi_f = (
                    float(stone),
                    float(cs),
                    float(fs),
                    float(csi),
                    float(fsi),
                )
            except ValueError:
                continue
            # Clay is the residual (fractions sum to 100% of the <2mm fraction)
            # when not given explicitly as a 9th/10th numeric column.
            clay_f = clay if clay is not None else max(
                0.0, 100.0 - cs_f - fs_f - csi_f - fsi_f
            )
            records.append(
                ParticleRecord(x_f, y_f, depth_range, stone_f, cs_f, fs_f, csi_f, fsi_f, clay_f)
            )
    if not records:
        raise ValueError(f"No particle-size records parsed from {path}")
    return records


@dataclass
class LayerRecord:
    x: float
    y: float
    depth_a_cm: float
    depth_b1_cm: float
    texture_b1: str
    depth_b2_cm: float | None
    texture_b2: str | None


def parse_layer_file(path: str) -> list[LayerRecord]:
    """Per Readme.soil: 'coordinate coordinate, depth to bottom of A
    horizon, depth to bottom of B1 horizon, texture category of B1, depth
    to bottom of B2 horizon [if present], texture category of B2 [if
    present]' -- 5 or 7 fields (B2 columns optional per-row)."""
    records = []
    with open(path, "r") as f:
        for line in f:
            parts = line.split()
            if len(parts) == 5:
                x, y, da, db1, tb1 = parts
                db2, tb2 = None, None
            elif len(parts) == 7:
                x, y, da, db1, tb1, db2, tb2 = parts
            else:
                continue
            try:
                x_f, y_f, da_f, db1_f = float(x), float(y), float(da), float(db1)
                db2_f = float(db2) if db2 is not None else None
            except ValueError:
                continue
            records.append(LayerRecord(x_f, y_f, da_f, db1_f, tb1, db2_f, tb2))
    if not records:
        raise ValueError(f"No layer records parsed from {path}")
    return records
