# Radar and precipitation parameters (EDR)

Live on folkweather.com. All are EDR `position` queries
(`/edr/collections/<collection>/position?coords=POINT(lon lat)&parameter-name=<NAME>`).

| Parameter | Collection | Meaning |
|---|---|---|
| `REFC` | `hrrr-atmosphere` | HRRR composite reflectivity, dBZ. **-10 = no echo** (HRRR's floor). Forecast hours, not observed. |
| `TCDC` | `hrrr-atmosphere` | HRRR total cloud cover, %. |
| `MSLMA` | `hrrr-mean-sea-level` | HRRR sea-level pressure, Pa. Replaces `PRMSL`, which HRRR does not publish. WMS layer is `hrrr_MSLMA`. |
| `PRECIP_FLAG` | `mrms-single-level` (+ `-latest`) | MRMS precipitation type, 2-minute cadence. Code table below. |
| `MESH` | `mrms-single-level` (+ `-latest`) | MRMS maximum estimated hail size, mm. **-1 = covered, no hail; null = outside radar coverage.** |

## PRECIP_FLAG codes

| Code | Meaning |
|---|---|
| 0 | no precipitation |
| 1 | warm stratiform rain |
| 3 | snow |
| 6 | convective rain |
| 7 | hail |
| 10 | cold stratiform rain (rain/snow transition) |
| 91 | tropical / stratiform rain mix |
| 96 | tropical / convective rain mix |
| null | outside radar coverage (MRMS -3) |

`PRECIP_FLAG` is read from the nearest grid cell, never interpolated (averaging codes would invent
values). In overview pyramids it is downsampled by nearest; `REFC` and `MESH` by max.

## Notes

- REFC / TCDC / MSLMA history accumulates from the deploy (2026-10-10 ~19:45Z); HRRR retention is 24 h (and at least the 4 latest runs).
- Not done: sub-hourly (15-minute) HRRR precipitation/reflectivity, and a single HRRR precipitation-type grid.
  The four HRRR categorical masks (`CRAIN`, `CSNOW`, `CFRZR`, `CICEP`) are already in EDR.
- Existing bug, not fixed here: `GridMetadata::resolution()` divides the first-to-last cell-centre span by
  the cell count instead of count-1, so regular-grid point reads drift from 0 up to about one cell at the
  east edge (about 1 km for MRMS). Separate fix.
