#!/usr/bin/env python3
"""Regenerates glm_expected.json: independent reference values for the GLM
fixtures, used by the Rust tests in src/glm.rs.

Independent on purpose: this decodes the RAW packed integers itself, in float64,
following the product's own attributes (scale_factor / add_offset / _Unsigned /
_FillValue) -- it does not share any code with the Rust parser, and it does not
use netCDF4's auto-scaling (which computes in float32).

    pip install netCDF4 numpy
    python3 make_glm_expected.py
"""
import datetime as dt
import glob
import json
import os

import netCDF4
import numpy as np

J2000 = dt.datetime(2000, 1, 1, 12, 0, 0, tzinfo=dt.timezone.utc)  # NOON, not midnight
CONUS = dict(lat=(24.0, 50.0), lon=(-125.0, -66.0))
HERE = os.path.dirname(os.path.abspath(__file__))


def attr(var, name, default=None):
    return float(np.float64(getattr(var, name))) if name in var.ncattrs() else default


out = {}
for path in sorted(glob.glob(os.path.join(HERE, "OR_GLM-L2-LCFA_*.nc"))):
    ds = netCDF4.Dataset(path)
    ds.set_auto_maskandscale(False)  # raw integers only
    pt = float(ds["product_time"][:])
    raw = lambda n: np.asarray(ds[n][:])
    fid = raw("flash_id").view(np.uint16).astype(np.int64)  # _Unsigned = true
    t_raw = raw("flash_time_offset_of_first_event").view(np.uint16).astype(np.float64)
    e_raw = raw("flash_energy").view(np.uint16).astype(np.int64)
    q = raw("flash_quality_flag").view(np.uint16).astype(np.int64)
    lat = raw("flash_lat").astype(np.float64)
    lon = raw("flash_lon").astype(np.float64)

    v = ds["flash_time_offset_of_first_event"]
    t_off = t_raw * attr(v, "scale_factor", 1.0) + attr(v, "add_offset", 0.0)
    ev = ds["flash_energy"]
    fill = np.uint16(np.int16(ev._FillValue)).astype(np.int64)
    energy = np.where(e_raw == fill, np.nan, e_raw * attr(ev, "scale_factor", 1.0) + attr(ev, "add_offset", 0.0))
    unix = (J2000 + dt.timedelta(seconds=pt)).timestamp() + t_off

    conus = (lat >= CONUS["lat"][0]) & (lat <= CONUS["lat"][1]) & (lon >= CONUS["lon"][0]) & (lon <= CONUS["lon"][1])
    first = [
        dict(flash_id=int(fid[i]), unix_time=float(unix[i]), lat=float(lat[i]), lon=float(lon[i]),
             energy_j=None if np.isnan(energy[i]) else float(energy[i]), quality=int(q[i]))
        for i in range(min(8, len(fid)))
    ]
    out[os.path.basename(path)] = dict(
        platform=str(ds.platform_ID),
        product_time_unix=(J2000 + dt.timedelta(seconds=pt)).timestamp(),
        n_flashes=int(len(fid)),
        flash_id_min=int(fid.min()), flash_id_max=int(fid.max()), flash_id_sum=int(fid.sum()),
        n_flash_id_above_32767=int((fid > 32767).sum()),
        unix_time_min=float(unix.min()), unix_time_max=float(unix.max()),
        lat_sum=float(lat.sum()), lon_sum=float(lon.sum()),
        energy_sum_j=float(np.nansum(energy)), n_energy_fill=int(np.isnan(energy).sum()),
        quality_counts={str(k): int(c) for k, c in zip(*np.unique(q, return_counts=True))},
        conus_count=int(conus.sum()), conus_flash_id_sum=int(fid[conus].sum()),
        first_flashes=first,
    )
json.dump(out, open(os.path.join(HERE, "glm_expected.json"), "w"), indent=2, sort_keys=True)
print("wrote glm_expected.json for", list(out))
