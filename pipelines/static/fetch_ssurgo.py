#!/usr/bin/env python3
"""Derive per-map-unit Ks/theta_s/theta_ref/theta_wilt from a gSSURGO file
geodatabase (soil texture -> hydraulic properties, the Eq. 1 ln(Ks) input
and the Eq. 4/5 theta_wilt/theta_ref stress-function inputs).

**Download step: manual, not automated this session.** gSSURGO's
distribution changed since this project's design doc was written -- NRCS
now serves state/CONUS file geodatabases through an interactive Box folder
(`https://nrcs.app.box.com/v/soils`), not a stable, predictable download
URL. Confirmed this session: the older `websoilsurvey.sc.egov.usda.gov`
and `nrcs.usda.gov/Internet/FSE_MEDIA` URL patterns referenced in older
documentation both 404/405 now, and Box's folder browser is JS-rendered
(not scrapable for a direct link). Unlike Tarrawarra (blocked by a WAF on
data that IS at a stable URL) this is a genuine distribution-mechanism
change -- download the Colorado gSSURGO geodatabase manually via that Box
link and pass its path to `--gdb-path` below.

The processing logic below (map-unit -> dominant component -> horizon ->
representative hydraulic properties) is real and follows gSSURGO's
documented table structure (mapunit -> component via mukey, component ->
chorizon via cokey), not a novel scheme -- this is the standard SSURGO
tabular join every soil-hydrology application does. Requires `geopandas`/
`fiona` to read the .gdb (not in requirements.txt -- this pipeline step
runs standalone, not inside the trail-physics service container).

Usage:
    python3 fetch_ssurgo.py --gdb-path /path/to/gSSURGO_CO.gdb --output ssurgo_co.parquet
"""

from __future__ import annotations

import argparse


def load_map_unit_hydraulic_properties(gdb_path: str):
    """Join mapunit -> component (dominant by comppct_r) -> chorizon (top
    horizon, 0-30cm) to get one representative Ks/theta_s/theta_ref/
    theta_wilt per map unit polygon.

    gSSURGO column names (standard, per the NRCS gSSURGO table schema):
    - component.mukey, component.cokey, component.comppct_r (component %)
    - chorizon.cokey, chorizon.hzdept_r/hzdepb_r (horizon top/bottom depth, cm)
    - chorizon.ksat_r (saturated hydraulic conductivity, um/s -- convert to mm/hr: * 3.6)
    - chorizon.wsatiated_r (theta_s, saturated water content, vol %)
    - chorizon.wthirdbar_r (theta at 1/3 bar ~ field capacity, vol % -> theta_ref)
    - chorizon.wfifteenbar_r (theta at 15 bar ~ wilting point, vol % -> theta_wilt)
    """
    import geopandas as gpd  # noqa: F401 -- deferred import, not a service dependency

    try:
        import fiona

        layers = fiona.listlayers(gdb_path)
    except Exception as e:  # noqa: BLE001
        raise RuntimeError(
            f"Could not open {gdb_path} as a file geodatabase -- is this a real "
            f"gSSURGO .gdb downloaded from the Box folder? Original error: {e}"
        ) from e

    mapunit = gpd.read_file(gdb_path, layer="mapunit")
    component = gpd.read_file(gdb_path, layer="component")
    chorizon = gpd.read_file(gdb_path, layer="chorizon")
    mupolygon = gpd.read_file(gdb_path, layer="MUPOLYGON")

    # Dominant component per map unit (highest comppct_r).
    dominant = component.sort_values("comppct_r", ascending=False).drop_duplicates("mukey")

    # Shallow horizon (0-30cm overlap) per dominant component.
    shallow = chorizon[(chorizon["hzdept_r"] < 30)].sort_values("hzdept_r")
    shallow_top = shallow.drop_duplicates("cokey")

    joined = dominant.merge(shallow_top, on="cokey", suffixes=("", "_horizon"))
    joined["ksat_mm_hr"] = joined["ksat_r"] * 3.6  # um/s -> mm/hr
    joined = joined.rename(
        columns={
            "wsatiated_r": "theta_s_pct",
            "wthirdbar_r": "theta_ref_pct",
            "wfifteenbar_r": "theta_wilt_pct",
        }
    )

    result = mupolygon.merge(
        joined[["mukey", "ksat_mm_hr", "theta_s_pct", "theta_ref_pct", "theta_wilt_pct"]],
        on="mukey",
        how="left",
    )
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gdb-path", required=True, help="Path to a manually-downloaded gSSURGO .gdb")
    parser.add_argument("--output", default="ssurgo_hydraulic_properties.parquet")
    args = parser.parse_args()

    gdf = load_map_unit_hydraulic_properties(args.gdb_path)
    gdf.to_parquet(args.output)
    print(f"Wrote {len(gdf)} map unit polygons with hydraulic properties to {args.output}")


if __name__ == "__main__":
    main()
