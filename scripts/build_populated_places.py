#!/usr/bin/env python3
"""
Build crates/storage/data/us_populated_places.csv for the EDR `populated`
collection.

Joins two public-domain US Census sources on the 7-digit place GEOID
(2-digit state FIPS + 5-digit place FIPS):

  1. Gazetteer 2023 place files  -> place name + interior-point lat/lon
     https://www2.census.gov/geo/docs/maps-data/data/gazetteer/2023_Gazetteer/
  2. Population Estimates (PEP) sub-est2023.csv -> 2023 population estimate
     https://www2.census.gov/programs-surveys/popest/datasets/2020-2023/cities/totals/

Output columns (LON,LAT order = GeoJSON convention, matches parse_stations_csv):
  id,name,lon,lat,elevation_m,state,population

  id = "PP" + 7-digit GEOID  (e.g. PP0107000 = Birmingham, AL)

Filter: population >= MIN_POP (default 1000). CDPs (Gazetteer places with no
PEP incorporated-place row) are included when they have a population row too
(PEP SUMLEV 157 = CDP). Only US states/DC (no territories) are kept, matching
the CONUS/forecast focus.

Run:  python3 scripts/build_populated_places.py
(Requires network access to census.gov; output is committed to the repo.)
"""

import csv
import io
import sys
import urllib.request
from pathlib import Path

MIN_POP = 1000

GAZ_BASE = "https://www2.census.gov/geo/docs/maps-data/data/gazetteer/2023_Gazetteer"
PEP_URL = (
    "https://www2.census.gov/programs-surveys/popest/datasets/"
    "2020-2023/cities/totals/sub-est2023.csv"
)

# FIPS -> USPS for the 50 states + DC. (Territories excluded on purpose.)
STATE_FIPS = {
    "01": "AL", "02": "AK", "04": "AZ", "05": "AR", "06": "CA", "08": "CO",
    "09": "CT", "10": "DE", "11": "DC", "12": "FL", "13": "GA", "15": "HI",
    "16": "ID", "17": "IL", "18": "IN", "19": "IA", "20": "KS", "21": "KY",
    "22": "LA", "23": "ME", "24": "MD", "25": "MA", "26": "MI", "27": "MN",
    "28": "MS", "29": "MO", "30": "MT", "31": "NE", "32": "NV", "33": "NH",
    "34": "NJ", "35": "NM", "36": "NY", "37": "NC", "38": "ND", "39": "OH",
    "40": "OK", "41": "OR", "42": "PA", "44": "RI", "45": "SC", "46": "SD",
    "47": "TN", "48": "TX", "49": "UT", "50": "VT", "51": "VA", "53": "WA",
    "54": "WV", "55": "WI", "56": "WY",
}


def fetch(url: str) -> str:
    print(f"  fetching {url}", file=sys.stderr)
    req = urllib.request.Request(url, headers={"User-Agent": "weather-wms-build"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read().decode("latin-1")


def load_population() -> dict:
    """GEOID(7) -> population, from PEP. Keeps incorporated places (162) and
    CDPs (157); drops higher-level summaries."""
    text = fetch(PEP_URL)
    pop = {}
    reader = csv.DictReader(io.StringIO(text))
    for row in reader:
        sumlev = row["SUMLEV"]
        if sumlev not in ("157", "162"):  # 162 incorporated place, 157 CDP
            continue
        place = row["PLACE"]
        if place == "00000":
            continue
        geoid = row["STATE"] + place  # 2 + 5 = 7
        try:
            p = int(row["POPESTIMATE2023"])
        except (ValueError, KeyError):
            continue
        # Keep the larger if a GEOID appears twice (shouldn't, but be safe)
        if geoid not in pop or p > pop[geoid]:
            pop[geoid] = p
    print(f"  loaded {len(pop)} population rows", file=sys.stderr)
    return pop


def clean_name(name: str) -> str:
    """Strip Census LSAD suffixes for a display-friendly name."""
    for suffix in (
        " city", " town", " village", " borough", " CDP",
        " municipality", " (balance)", " metro government",
        " consolidated government", " unified government",
        " metropolitan government", " urban county",
    ):
        if name.endswith(suffix):
            name = name[: -len(suffix)]
    return name.strip()


def load_gazetteer() -> dict:
    """GEOID(7) -> (name, lat, lon) from per-state Gazetteer place files."""
    places = {}
    for fips, usps in STATE_FIPS.items():
        url = f"{GAZ_BASE}/2023_gaz_place_{fips}.txt"
        try:
            text = fetch(url)
        except Exception as e:
            print(f"  WARN: {usps} gazetteer failed: {e}", file=sys.stderr)
            continue
        # Tab-separated, but trailing spaces pad the last column
        reader = csv.reader(io.StringIO(text), delimiter="\t")
        header = next(reader, None)
        if not header:
            continue
        for row in reader:
            if len(row) < 12:
                continue
            geoid = row[1].strip()
            name = row[3].strip()
            try:
                lat = float(row[10].strip())
                lon = float(row[11].strip())
            except ValueError:
                continue
            places[geoid] = (name, lat, lon, usps)
    print(f"  loaded {len(places)} gazetteer places", file=sys.stderr)
    return places


def main():
    print("Building populated places dataset...", file=sys.stderr)
    pop = load_population()
    gaz = load_gazetteer()

    rows = []
    for geoid, (name, lat, lon, usps) in gaz.items():
        p = pop.get(geoid)
        if p is None or p < MIN_POP:
            continue
        rows.append(
            {
                "id": f"PP{geoid}",
                "name": clean_name(name),
                "lon": f"{lon:.5f}",
                "lat": f"{lat:.5f}",
                "elevation_m": "",  # not in Gazetteer
                "state": usps,
                "population": str(p),
            }
        )

    # Sort by population desc so the file is human-scannable (biggest first)
    rows.sort(key=lambda r: int(r["population"]), reverse=True)

    out_path = (
        Path(__file__).resolve().parent.parent
        / "crates" / "storage" / "data" / "us_populated_places.csv"
    )
    with open(out_path, "w", newline="\n") as f:
        f.write(
            "# US populated places (Census Gazetteer 2023 coords + PEP 2023 population)\n"
            f"# population >= {MIN_POP}; id = PP<7-digit GEOID>; LON,LAT order\n"
            "# Generated by scripts/build_populated_places.py\n"
        )
        writer = csv.DictWriter(
            f,
            fieldnames=["id", "name", "lon", "lat", "elevation_m", "state", "population"],
        )
        writer.writeheader()
        for r in rows:
            writer.writerow(r)

    print(f"Wrote {len(rows)} places to {out_path}", file=sys.stderr)


if __name__ == "__main__":
    main()
