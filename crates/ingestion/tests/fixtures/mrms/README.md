Real NOAA MRMS granules (public bucket `noaa-mrms-pds`, valid 2026-10-09 15:00 UTC), used by
`tests/mrms_qpe_ingest.rs`. Renamed with the `mrms-qpe_` prefix the downloader gives them.

- `...QPE_01H_Pass2...`: GRIB2 discipline 209, category 6, **number 37**
- `...QPE_01H_Pass1...`: GRIB2 discipline 209, category 6, **number 30** (same quantity, different code)

Both are 7000x3500, 0.01 degree, ~0.6 MB gzipped.

- `...QPE_24H_Pass2..._20261009-160000`: GRIB2 discipline 209, category 6, number 41, valid 16:00 UTC
  (an hour for which there is no QPE_01H fixture; used by edr-api's `mrms_qpe_series` test to make
  "another QPE parameter has this hour, QPE_01H does not").
