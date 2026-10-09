# GLM test fixtures

Real, unmodified NOAA GOES-R GLM L2 LCFA granules (public domain, from the
`noaa-goes18` / `noaa-goes19` AWS Open Data buckets) used by `src/glm.rs`:

| File | Satellite | Flashes | In CONUS box | Notes |
|---|---|---|---|---|
| `OR_GLM-L2-LCFA_G19_s20262802033000_...nc` | GOES-19 (East) | 899 | 21 | every `flash_id` > 32,767 (exercises the `_Unsigned` trap); 44 flashes with quality flag 3 |
| `OR_GLM-L2-LCFA_G18_s20262802033000_...nc` | GOES-18 (West) | 57 | 9 | quiet granule |
| `glm_empty_no_flashes.nc` | synthetic | 0 | 0 | header copied from the G18 granule, zero-length flash dimension, `product_time` = 2026-10-07T20:35:00Z. Built with `ncgen`; its `time_coverage_*` global attributes still say 20:33 (irrelevant: the reader uses `product_time`). |

`glm_expected.json` holds reference values produced by `make_glm_expected.py`,
an **independent** decoder (it shares no code with the Rust reader and does its
own float64 arithmetic from the raw packed integers). Regenerate with
`pip install netCDF4 numpy && python3 make_glm_expected.py`.
