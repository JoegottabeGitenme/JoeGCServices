"""Parsers for the Tarrawarra dataset's ASCII file formats.

Format specifications transcribed directly from the dataset's own
documentation (Readme.topo, Readme.tdr, Readme.soil, Readme.nmm -- fetched
successfully via the site's HTML pages; see README.md in this directory for
the full story of an intermittent WAF blocking most, but not all, requests).

**The DEM header format was confirmed against a real downloaded file in
Session 3** (`tarrautm.dem`, briefly fetched through a lucky WAF window --
see README.md), correcting an earlier documented *assumption*. The real
header is NOT the standard ESRI ASCII grid format originally guessed; it is:

    Copyright (c) 1995-1998 ...   (a free-text copyright line, ignored)
    <blank line>
    north: 5831250.00
    south: 5830930.00
    east: 362615.00
    west: 361990.00
    rows: 64
    cols: 125
    <rows*cols whitespace-separated elevation values, row 0 = north edge>

`cellsize` is not given directly -- it's derived as
`(east-west)/cols` (cross-checked equal to `(north-south)/rows`, both
giving 5.0m, matching the dataset's own "5m Digital Elevation Model"
description). `0.00` is used as the fill value for cells outside the
surveyed catchment (visible as a border of zeros around the real data in
the actual file) -- there's no explicit NODATA_value line in this format,
so `0.00` is treated as nodata by convention specific to this dataset.

The full elevation grid body (8000 values) was NOT hand-transcribed into
this repo from the fetch tool's output -- reproducing thousands of numeric
values by hand through a chat-mediated tool is not a reliable way to build
ground-truth validation data, and a silently-corrupted DEM would be worse
than an honestly-missing one. Only the header format (7 lines, low
transcription risk, independently sanity-checked via the cellsize
cross-check above) was captured. See README.md for how to get the real grid
body via manual browser download.

The old ESRI-grid-header parsing path is kept as a fallback (some other
DEM tool might still produce that format) but the Tarrawarra-native format
above is now the primary, confirmed-correct path.

Every other parser (TDR, ksat, particle, layer) is a straightforward
whitespace-delimited-columns-after-a-header format per the docs. `ksat.dat`
was also fetched for real this session (see README.md) and confirms the
format: `x y bottom_depth_cm top_depth_cm ksat_mm_hr`, 5 whitespace fields
-- matching this parser's existing 5-field assumption positionally (field
names in the dataclass below say "well_base"/"water" per the original
Readme.soil wording; the real column headers say "bottom depth"/"top
depth" instead, but it's the same 5 numeric columns in the same order).
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import numpy as np

# The confirmed-real Tarrawarra format: "key: value" lines using these
# exact names (north/south/east/west/rows/cols).
_TARRAWARRA_HEADER_KEY_PATTERNS = {
    "north": re.compile(r"^north$", re.IGNORECASE),
    "south": re.compile(r"^south$", re.IGNORECASE),
    "east": re.compile(r"^east$", re.IGNORECASE),
    "west": re.compile(r"^west$", re.IGNORECASE),
    "rows": re.compile(r"^rows?$", re.IGNORECASE),
    "cols": re.compile(r"^cols?$", re.IGNORECASE),
}

# Fallback: the originally-assumed (and NOT confirmed against a real file)
# standard ESRI ASCII grid header, kept in case some other tool in this
# lineage produces that format instead.
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


def _try_parse_tarrawarra_header(lines: list[str]) -> tuple[dict, int] | None:
    """Try the confirmed-real "key: value" format (north/south/east/west/
    rows/cols). Returns (header, data_start_line) or None if this isn't
    that format."""
    header: dict = {}
    data_start_line = 0
    for i, line in enumerate(lines[:12]):
        if ":" not in line:
            continue
        key_text, _, value_text = line.partition(":")
        key_text = key_text.strip()
        value_text = value_text.strip()
        for key, pattern in _TARRAWARRA_HEADER_KEY_PATTERNS.items():
            if pattern.match(key_text):
                try:
                    header[key] = float(value_text)
                except ValueError:
                    return None
                data_start_line = i + 1
                break
    if {"north", "south", "east", "west", "rows", "cols"} <= header.keys():
        return header, data_start_line
    return None


def _try_parse_esri_header(lines: list[str]) -> tuple[dict, int] | None:
    """Try the originally-assumed-but-unconfirmed ESRI ASCII grid header,
    kept as a fallback -- see module docstring."""
    header: dict = {}
    data_start_line = 0
    for i, line in enumerate(lines[:10]):
        parts = line.split()
        if len(parts) != 2:
            continue
        key_text, value_text = parts
        for key, pattern in _DEM_HEADER_KEY_PATTERNS.items():
            if pattern.match(key_text):
                try:
                    header[key] = float(value_text)
                except ValueError:
                    pass
                data_start_line = i + 1
                break
    if {"ncols", "nrows", "cellsize"} <= header.keys():
        return header, data_start_line
    return None


def parse_dem(path: str) -> DemGrid:
    """Parse a Tarrawarra .dem file. Tries the confirmed-real Tarrawarra
    "key: value" header format first (north/south/east/west/rows/cols --
    see module docstring), then falls back to the originally-assumed (never
    confirmed) ESRI ASCII grid header. Raises ValueError with the actual
    header text if neither matches, rather than silently misparsing."""
    with open(path, "r") as f:
        lines = f.readlines()

    tarrawarra_result = _try_parse_tarrawarra_header(lines)
    if tarrawarra_result is not None:
        header, data_start_line = tarrawarra_result
        nrows = int(header["rows"])
        ncols = int(header["cols"])
        # cellsize isn't given directly in this format -- derive it, and
        # cross-check the two independent derivations agree (see module
        # docstring: both give 5.0m for the real Tarrawarra DEM).
        cellsize_ew = (header["east"] - header["west"]) / ncols
        cellsize_ns = (header["north"] - header["south"]) / nrows
        if abs(cellsize_ew - cellsize_ns) > 0.01 * max(cellsize_ew, cellsize_ns):
            raise ValueError(
                f"DEM {path}: derived cellsize disagrees between east-west "
                f"({cellsize_ew}) and north-south ({cellsize_ns}) -- header "
                f"values may not describe a square-celled grid as assumed."
            )
        xllcorner = header["west"]
        yllcorner = header["south"]
        cellsize = cellsize_ew
        nodata = 0.0  # this dataset's convention -- see module docstring
    else:
        esri_result = _try_parse_esri_header(lines)
        if esri_result is None:
            raise ValueError(
                f"Could not parse DEM header from {path} as either the "
                f"confirmed Tarrawarra format (north/south/east/west/rows/cols) "
                f"or the fallback ESRI ASCII grid format. Actual header lines "
                f"found:\n" + "".join(lines[:8])
            )
        header, data_start_line = esri_result
        nrows = int(header["nrows"])
        ncols = int(header["ncols"])
        xllcorner = header.get("xllcorner", 0.0)
        yllcorner = header.get("yllcorner", 0.0)
        cellsize = header["cellsize"]
        nodata = header.get("nodata_value")

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
    if nodata is not None:
        elevation = np.where(elevation == nodata, np.nan, elevation)

    return DemGrid(
        elevation=elevation,
        cellsize=cellsize,
        xllcorner=xllcorner,
        yllcorner=yllcorner,
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
    fine sand / coarse silt / fine silt / clay percentages.

    **Confirmed against the real file (Session 4)**: the depth range is
    given as ONE token, either hyphenated ("0-13") or open-ended (">24",
    for the bottommost sample at a site) -- never as two separate numeric
    tokens; and clay IS given explicitly as a 9th numeric column, not left
    to be computed as a residual (the original assumption, kept below as a
    fallback for whichever synthetic/other-source file doesn't include it).
    Disambiguation: if the token right after (x, y) fails to parse as a
    float, it's a single depth-range token (hyphenated or open-ended);
    otherwise it's the documented-but-unconfirmed two-separate-numeric-
    tokens form, and the next token is consumed as its second half.
    Whatever numeric tokens remain after the depth range is exactly 5
    (clay as residual) or 6 (clay explicit)."""
    records = []
    with open(path, "r") as f:
        for line in f:
            parts = line.split()
            if len(parts) < 8:
                continue
            try:
                x_f, y_f = float(parts[0]), float(parts[1])
            except ValueError:
                continue  # header/comment line

            depth_token_is_numeric = True
            try:
                float(parts[2])
            except ValueError:
                depth_token_is_numeric = False

            if depth_token_is_numeric:
                if len(parts) < 9:
                    continue
                depth_range = f"{parts[2]}-{parts[3]}"
                rest = parts[4:]
            else:
                depth_range = parts[2]
                rest = parts[3:]

            if len(rest) == 6:
                stone, cs, fs, csi, fsi, clay = rest
            elif len(rest) == 5:
                stone, cs, fs, csi, fsi = rest
                clay = None
            else:
                continue

            try:
                stone_f, cs_f, fs_f, csi_f, fsi_f = (
                    float(stone),
                    float(cs),
                    float(fs),
                    float(csi),
                    float(fsi),
                )
                clay_f = float(clay) if clay is not None else None
            except ValueError:
                continue

            # Clay is the residual (fractions sum to 100% of the <2mm
            # fraction) only when not given explicitly as a numeric column.
            if clay_f is None:
                clay_f = max(0.0, 100.0 - cs_f - fs_f - csi_f - fsi_f)

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
    # depth_b1_cm/depth_b2_cm are kept as RAW TEXT, not float, deliberately:
    # the real file (confirmed, Session 4) uses open-ended depths like
    # ">73" (the core didn't reach the bottom of that horizon) for roughly
    # a third of rows -- converting that to a float would either crash or
    # require fabricating a number the data doesn't actually give. None
    # means the field was blank in the source row (B2 not recorded, or
    # occasionally B1 texture not recorded either).
    depth_b1_cm: str | None
    texture_b1: str | None
    depth_b2_cm: str | None
    texture_b2: str | None


@dataclass
class NmmProfile:
    site: int
    date: str  # e.g. "20-Sep-95" -- see date_line_re note below on the doc/reality mismatch
    time: str  # e.g. "11:10"
    depths_cm: np.ndarray
    moisture_pct: np.ndarray  # %V/V


def parse_nmm_file(path: str, site: int | None = None) -> list[NmmProfile]:
    """Parse a Tarrawarra nmm_data/tube_N.dat file. Per Readme.nmm: a header
    (site ID/coordinates/collection period/measurement method, unspecified
    exact length -- skipped by scanning for the first date/time line rather
    than assuming a fixed header length), then repeated blocks separated by
    blank lines:

        date   time
        depth(cm)   moisture(%V/V)
        ...
        depth(cm)   moisture(%V/V)

    **Confirmed against real downloaded tube_N.dat files (all 20, Session
    4)**: Readme.nmm documents the date/time format as "dd/mm/yyyy" and
    "hhmm" -- the real files instead use "DD-Mon-YY" and "HH:MM" (e.g.
    "20-Sep-95" / "11:10"), matching the TDR files' own date convention
    rather than their own readme. `date_line_re` accepts both forms since
    the documented one might appear in some other distribution of this
    data. Fields are tab-separated with leading whitespace/tabs on every
    line (including blank-looking separator lines, which are a lone tab +
    CRLF, not truly empty) -- handled fine by `str.split()`'s any-whitespace
    behavior and `str.strip()` correctly treating a lone tab as blank.

    `site` defaults to parsing it from the filename (`tube_N.dat` -> N) if
    not given explicitly -- callers that already know the site number
    (e.g. iterating `tube_1.dat` .. `tube_20.dat`) can skip that guess.
    """
    if site is None:
        m = re.search(r"tube_(\d+)", Path(path).name)
        if m is None:
            raise ValueError(
                f"Could not infer site number from filename {path!r} -- "
                f"pass site= explicitly."
            )
        site = int(m.group(1))

    with open(path, "r") as f:
        raw_lines = [line.rstrip("\r\n") for line in f]

    # A "date line" is exactly 2 tokens: a date and a time. Accept both the
    # documented dd/mm/yyyy form and the real DD-Mon-YY form actually used
    # in the files (see docstring). Depth/moisture lines are exactly 2
    # numeric tokens. Skip everything before the first date line (the
    # free-text header, whose exact length isn't documented).
    date_line_re = re.compile(r"^(\d{1,2}/\d{1,2}/\d{2,4}|\d{1,2}-[A-Za-z]{3}-\d{2,4})$")

    profiles: list[NmmProfile] = []
    i = 0
    while i < len(raw_lines):
        parts = raw_lines[i].split()
        if len(parts) == 2 and date_line_re.match(parts[0]):
            date, time = parts
            i += 1
            depths: list[float] = []
            moistures: list[float] = []
            while i < len(raw_lines) and raw_lines[i].strip():
                dparts = raw_lines[i].split()
                if len(dparts) != 2:
                    break
                try:
                    depth, moisture = float(dparts[0]), float(dparts[1])
                except ValueError:
                    break
                depths.append(depth)
                moistures.append(moisture)
                i += 1
            if depths:
                profiles.append(
                    NmmProfile(
                        site=site,
                        date=date,
                        time=time,
                        depths_cm=np.array(depths),
                        moisture_pct=np.array(moistures),
                    )
                )
        else:
            i += 1

    if not profiles:
        raise ValueError(
            f"No NMM profiles parsed from {path} -- check the file's actual "
            f"layout against Readme.nmm's documented date-line/depth-line format."
        )
    return profiles


def parse_layer_file(path: str) -> list[LayerRecord]:
    """Per Readme.soil: 'coordinate coordinate, depth to bottom of A
    horizon, depth to bottom of B1 horizon, texture category of B1, depth
    to bottom of B2 horizon [if present], texture category of B2 [if
    present]' -- 7 columns, always present as 7 tab-separated fields in the
    real file (confirmed, Session 4), with missing B1/B2 texture or B2
    depth represented as an EMPTY field (not a dropped column) -- this is
    why this parser splits on the literal tab character rather than
    generic whitespace: several texture values are themselves multi-word
    ("silt clay", "silt&mudsone"), which a plain `.split()` on whitespace
    would incorrectly break into extra fields and misalign every column
    after it. Splitting on tabs keeps a multi-word texture as one field."""
    records = []
    with open(path, "r") as f:
        for line in f:
            line = line.rstrip("\r\n")
            parts = line.split("\t")
            if len(parts) != 7:
                continue
            x, y, da, db1, tb1, db2, tb2 = parts
            try:
                x_f, y_f, da_f = float(x), float(y), float(da)
            except ValueError:
                continue  # header/comment line
            records.append(
                LayerRecord(
                    x_f,
                    y_f,
                    da_f,
                    db1 or None,
                    tb1 or None,
                    db2 or None,
                    tb2 or None,
                )
            )
    if not records:
        raise ValueError(f"No layer records parsed from {path}")
    return records
