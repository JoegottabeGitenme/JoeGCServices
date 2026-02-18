# NASA NLDAS-2 Integration Plan for weather-wms

## Goal

Ingest NLDAS-2 (Noah LSM outputs + Forcing data) from NASA GES DISC and expose through our EDR and WMS services.

## Scope

| Attribute | Value |
|---|---|
| Product | NLDAS-2 Noah 0.125° hourly + NLDAS-2 Forcing 0.125° hourly |
| Coverage | North America (25-53°N, 125-67°W) |
| Temporal cadence | Hourly |
| Data latency | ~4 days behind real time |
| Retention | 30-day rolling window (~720 hourly files) |
| Storage estimate | ~15 GB total (Zarr + download cache) |
| Download method | HTTPS from GES DISC with Earthdata Login |
| File format | NetCDF-4 (CF-convention, regular lat/lon grid) |

## Architecture Fit

NLDAS-2 is **gridded observation data** on a regular lat/lon grid, fitting our existing gridded pipeline:

```
lis_runner.rs (new) --> downloads .nc files from GES DISC
    --> POST /ingest to ingester service
        --> ingester detects .nc, routes to NetCDF handler
            --> NEW: generic CF-convention NetCDF reader (multi-variable)
            --> writes Zarr arrays (one per variable per timestep)
            --> registers CatalogEntry per variable
    --> EDR API reads from Zarr (data_type: observation)
    --> WMS API reads from Zarr (dimensions.type: observation)
```

This is the same path as GOES/MRMS, with the key difference being:
- No reprojection needed (already regular lat/lon vs GOES geostationary)
- Multiple variables per file (vs GOES single CMI variable)

---

## Implementation Phases

### Phase 1: Generic CF-Convention NetCDF Reader

**Crate:** `crates/netcdf-parser/`

**What:** Add a new public function `load_cf_netcdf` alongside the existing GOES-specific `load_goes_netcdf_from_bytes`. This reads any CF-compliant NetCDF-4 file on a regular lat/lon grid and returns all data variables as grid arrays.

**Details:**
- New file: `crates/netcdf-parser/src/cf_reader.rs`
- Reads `lat` and `lon` coordinate variables to determine bbox, resolution, and grid dimensions
- Detects grid orientation from lat array (ascending = RowOrigin::South, descending = RowOrigin::North)
- Enumerates all data variables (skip coordinate vars: `lat`, `lon`, `time`, `x`, `y`)
- For each data variable:
  - Reads raw values
  - Applies CF-standard `scale_factor`, `add_offset`, `_FillValue` --> f32 grid
  - Extracts `units` and `long_name` attributes
- Returns `Vec<CfVariable>` where each contains: `name`, `long_name`, `units`, `grid_data: Vec<f32>`, `width`, `height`, `bbox`, `row_origin`
- Re-export from `crates/netcdf-parser/src/lib.rs`

**No new crate dependencies** -- the `netcdf` crate is already linked.

**Tests:**
- Unit tests with a small synthetic NetCDF file (or hard-coded test data)
- Test orientation detection (ascending vs descending lat)
- Test scale/offset/fill handling
- Test variable enumeration and coordinate variable filtering

### Phase 2: Multi-Variable NetCDF Ingestion Path

**Crate:** `crates/ingestion/`

**What:** Add a new ingestion path for CF-compliant regular-grid NetCDF files that extracts multiple variables from a single file.

**Details:**
- New file: `crates/ingestion/src/cf_netcdf.rs`
- Called when `FileType::NetCdf` is detected AND the file is not GOES (checked by absence of `goes_imager_projection` variable, or by model name)
- Uses the new `load_cf_netcdf` from Phase 1
- For each variable returned:
  - Maps CF variable name to our parameter name (e.g., `SOILM_0-10cm_inst` --> `SOILM_0_10`)
  - Determines level string from variable name/attributes (e.g., `"0-10 cm depth"`)
  - Writes Zarr array via existing `ZarrWriter::write_multiscale()` -- same as GOES path
  - Uploads to object storage
  - Registers `CatalogEntry` with `forecast_hour: 0` (observational data)
- Storage path: `grids/{model}/{date}/{HH}/{param}.zarr`
- Observation time extracted from NetCDF `time` variable or filename

**Routing changes in `crates/ingestion/src/ingester.rs`:**
- When `FileType::NetCdf` and model starts with `nldas` (or is in a configured list): route to `cf_netcdf::ingest_cf_netcdf()`
- Otherwise: route to existing GOES path (`netcdf::ingest_netcdf()`)
- Alternative: detect automatically by checking for `goes_imager_projection` variable in the file

**Parameter name mapping** (configured in the model YAML, not hard-coded):
- The YAML `parameters` section maps CF variable names to our internal names
- The ingestion function reads this mapping to decide which variables to extract and what to call them

**Tests:**
- Integration test with a real (small) NLDAS-2 file downloaded from GES DISC
- Test that all expected parameters are extracted
- Test Zarr output structure and catalog registration

### Phase 3: LIS Download Runner

**Service:** `services/downloader/`

**What:** New `lis_runner.rs` that downloads NLDAS-2 NetCDF files from NASA GES DISC with Earthdata authentication.

**Details:**
- New file: `services/downloader/src/lis_runner.rs`
- Add `mod lis_runner` to `services/downloader/src/main.rs`

**Authentication:**
- NASA Earthdata Login credentials from env vars: `EARTHDATA_USERNAME`, `EARTHDATA_PASSWORD`
- GES DISC uses an OAuth2 redirect flow:
  1. Request to GES DISC returns 302 redirect to `urs.earthdata.nasa.gov/oauth/authorize/`
  2. Client follows redirect, authenticating with HTTP Basic auth (username/password)
  3. URS redirects back to GES DISC with an auth code cookie
  4. GES DISC returns the data
- `reqwest::Client` configured with:
  - `cookie_store(true)` -- stores the session cookie across redirects
  - `redirect(Policy::limited(10))` -- follows the OAuth redirect chain
- On each redirect to `urs.earthdata.nasa.gov`, inject HTTP Basic auth header
- Session cookies are reused across multiple downloads within the same poll cycle
- Note: EDL bearer tokens do NOT work with GES DISC (it uses OAuth2, not federated tokens)

**URL construction:**
- Noah base: `https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_NOAH0125_H.2.0/{YYYY}/{DDD}/`
- Forcing base: `https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_FORA0125_H.2.0/{YYYY}/{DDD}/`
- File pattern: `NLDAS_NOAH0125_H.A{YYYYMMDD}.{HH}00.020.nc` (Noah)
- File pattern: `NLDAS_FORA0125_H.002.grb.SUB.nc4` (Forcing -- verify exact pattern)
- `{DDD}` = day of year (001-366)

**Poll loop:**
```
every poll_interval_secs (3600):
  1. time_window = (now - 30 days) to (now - 4 days)
  2. query ingester/catalog for already-ingested timestamps for this model
  3. for each missing hour in time_window:
     a. construct URL
     b. download to /data/downloads/nldas/
     c. POST /ingest with model="nldas-noah" (or "nldas-forcing")
     d. on success, delete downloaded file
  4. clean up any files older than retention window
```

**Error handling:**
- 404 for individual hourly files --> log warning, skip, retry next cycle
- Auth failure --> log error, back off, retry with fresh credentials
- Network timeout --> retry with exponential backoff (same pattern as dart_runner)
- Partial cycle completion is fine -- next poll fills gaps

**Initial backfill:**
- First run will need to download ~720 files (30 days x 24 hours)
- Rate-limit downloads to avoid hammering GES DISC (e.g., max 5 concurrent, 1s delay between)
- NASA Earthdata terms of service: be a good citizen, don't exceed reasonable rates

**Config integration:**
- Loaded from YAML like other runners via model config `source.type: nasa_gesdisc`
- `source.base_url` contains the GES DISC dataset URL
- `schedule.delay_hours: 96` (4-day latency)
- `retention.hours: 720` (30 days)

**Credential monitoring:**
- Runner should log a warning on startup if `EARTHDATA_USERNAME` or `EARTHDATA_PASSWORD` is not set
- If a request returns 401/403 after the redirect chain, log an error indicating credentials may be invalid
- NASA Earthdata Login accounts do not expire, but passwords may need periodic rotation

### Verified Data Structure (from sample file)

Downloaded and inspected `NLDAS_NOAH0125_H.A20260205.0000.020.nc`:
- **Grid**: 464 (lon) x 224 (lat), 0.125 degree
- **Lat orientation**: ascending (25.0625 to 52.9375) -- **RowOrigin::South**
- **Bbox**: lon [-124.9375, -67.0625], lat [25.0625, 52.9375]
- **Time encoding**: "hours since 1979-01-01 00:00:00" (value 412848 = 2026-02-05T00:00Z)
- **Fill value**: -9999.0 (NOT NaN -- must convert during read)
- **scale_factor/add_offset**: All 1.0/0.0 (data already in final units)
- **40 data variables** per file (all `float(time, lat, lon)`)
- **File size**: ~6.5 MB per hourly file
- **Auth flow**: GES DISC -> 302 -> URS OAuth (HTTP Basic) -> 302 back -> data (5 redirects total)
- **Prerequisite**: Must authorize "NASA GESDISC DATA ARCHIVE" app at `https://urs.earthdata.nasa.gov/approve_app?client_id=e2WVk8Pw6weeLUKZYOxvTQ`

### Phase 4: Configuration Files

#### `config/models/nldas-noah.yaml`

Variable names verified against actual file (cf_name = NetCDF variable name):

```yaml
model:
  id: nldas-noah
  name: "NLDAS-2 Noah Land Surface Model"
  description: "North American Land Data Assimilation System - Noah 0.125 hourly"
  enabled: true

dimensions:
  type: observation
  time: true
  elevation: true

source:
  type: nasa_gesdisc
  base_url: "https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_NOAH0125_H.2.0"
  file_pattern: "NLDAS_NOAH0125_H.A{date}.{hour}00.020.nc"

grid:
  projection: geographic
  resolution: "0.125deg"
  bbox:
    min_lon: -124.9375
    min_lat: 25.0625
    max_lon: -67.0625
    max_lat: 52.9375

schedule:
  type: observation
  poll_interval_secs: 3600
  delay_hours: 96

retention:
  hours: 720

parameters:
  # Soil Moisture (4 layers + aggregates)
  - name: SoilM_0_10cm
    cf_name: "SoilM_0_10cm"
    description: "Soil moisture content (0-10cm)"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 10
        display: "0-10 cm depth"
    valid_range: [0, 50]

  - name: SoilM_10_40cm
    cf_name: "SoilM_10_40cm"
    description: "Soil moisture content (10-40cm)"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 40
        display: "10-40 cm depth"
    valid_range: [0, 150]

  - name: SoilM_40_100cm
    cf_name: "SoilM_40_100cm"
    description: "Soil moisture content (40-100cm)"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 100
        display: "40-100 cm depth"
    valid_range: [0, 250]

  - name: SoilM_100_200cm
    cf_name: "SoilM_100_200cm"
    description: "Soil moisture content (100-200cm)"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 200
        display: "100-200 cm depth"
    valid_range: [0, 500]

  - name: SoilM_0_100cm
    cf_name: "SoilM_0_100cm"
    description: "Soil moisture content (0-100cm)"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 100
        display: "0-100 cm total"
    valid_range: [0, 500]

  - name: SoilM_0_200cm
    cf_name: "SoilM_0_200cm"
    description: "Soil moisture content (0-200cm)"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 200
        display: "0-200 cm total"
    valid_range: [0, 900]

  - name: RootMoist
    cf_name: "RootMoist"
    description: "Root zone soil moisture"
    units: "kg m-2"
    levels:
      - type: depth_below_surface
        value: 100
        display: "root zone"
    valid_range: [0, 900]

  # Soil Temperature (4 layers)
  - name: SoilT_0_10cm
    cf_name: "SoilT_0_10cm"
    description: "Soil temperature (0-10cm)"
    units: "K"
    display_units: "C"
    conversion: K_to_C
    levels:
      - type: depth_below_surface
        value: 10
        display: "0-10 cm depth"
    valid_range: [200, 350]

  - name: SoilT_10_40cm
    cf_name: "SoilT_10_40cm"
    description: "Soil temperature (10-40cm)"
    units: "K"
    display_units: "C"
    conversion: K_to_C
    levels:
      - type: depth_below_surface
        value: 40
        display: "10-40 cm depth"
    valid_range: [200, 350]

  - name: SoilT_40_100cm
    cf_name: "SoilT_40_100cm"
    description: "Soil temperature (40-100cm)"
    units: "K"
    display_units: "C"
    conversion: K_to_C
    levels:
      - type: depth_below_surface
        value: 100
        display: "40-100 cm depth"
    valid_range: [200, 350]

  - name: SoilT_100_200cm
    cf_name: "SoilT_100_200cm"
    description: "Soil temperature (100-200cm)"
    units: "K"
    display_units: "C"
    conversion: K_to_C
    levels:
      - type: depth_below_surface
        value: 200
        display: "100-200 cm depth"
    valid_range: [200, 350]

  # Snow
  - name: SWE
    cf_name: "SWE"
    description: "Snow Water Equivalent"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 2000]

  - name: SnowDepth
    cf_name: "SnowDepth"
    description: "Snow depth"
    units: "m"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 10]

  - name: SnowFrac
    cf_name: "SnowFrac"
    description: "Snow cover fraction"
    units: "fraction"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 1]

  # Surface
  - name: AvgSurfT
    cf_name: "AvgSurfT"
    description: "Average surface skin temperature"
    units: "K"
    display_units: "C"
    conversion: K_to_C
    levels:
      - type: surface
        display: "surface"
    valid_range: [200, 350]

  - name: Albedo
    cf_name: "Albedo"
    description: "Surface albedo"
    units: "%"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 100]

  - name: CanopInt
    cf_name: "CanopInt"
    description: "Plant canopy surface water"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 5]

  # Radiation fluxes
  - name: SWdown
    cf_name: "SWdown"
    description: "Shortwave radiation flux downwards (surface)"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 1500]

  - name: LWdown
    cf_name: "LWdown"
    description: "Longwave radiation flux downwards (surface)"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 700]

  - name: SWnet
    cf_name: "SWnet"
    description: "Net shortwave radiation flux (surface)"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 1500]

  - name: LWnet
    cf_name: "LWnet"
    description: "Net longwave radiation flux (surface)"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-200, 50]

  # Heat fluxes
  - name: Qle
    cf_name: "Qle"
    description: "Latent heat flux"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-100, 500]

  - name: Qh
    cf_name: "Qh"
    description: "Sensible heat flux"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-200, 500]

  - name: Qg
    cf_name: "Qg"
    description: "Ground heat flux"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-200, 200]

  - name: Qf
    cf_name: "Qf"
    description: "Snow phase-change heat flux"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-100, 100]

  # Water fluxes
  - name: Evap
    cf_name: "Evap"
    description: "Total evapotranspiration"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-1, 5]

  - name: Qs
    cf_name: "Qs"
    description: "Surface runoff (non-infiltrating)"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 100]

  - name: Qsb
    cf_name: "Qsb"
    description: "Subsurface runoff (baseflow)"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 100]

  - name: Qsm
    cf_name: "Qsm"
    description: "Snowmelt"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 50]

  - name: Snowf
    cf_name: "Snowf"
    description: "Frozen precipitation (snowfall)"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 50]

  - name: Rainf
    cf_name: "Rainf"
    description: "Liquid precipitation (rainfall)"
    units: "kg m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 100]

  # ET components
  - name: PotEvap
    cf_name: "PotEvap"
    description: "Potential evapotranspiration"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [-100, 1000]

  - name: ECanop
    cf_name: "ECanop"
    description: "Canopy water evaporation"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 200]

  - name: TVeg
    cf_name: "TVeg"
    description: "Transpiration"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 200]

  - name: ESoil
    cf_name: "ESoil"
    description: "Direct evaporation from bare soil"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 200]

  - name: SubSnow
    cf_name: "SubSnow"
    description: "Sublimation (evaporation from snow)"
    units: "W m-2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 200]

  # Vegetation / land surface
  - name: LAI
    cf_name: "LAI"
    description: "Leaf Area Index"
    units: "unitless"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 10]

  - name: GVEG
    cf_name: "GVEG"
    description: "Green vegetation fraction"
    units: "fraction"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 1]

  - name: Streamflow
    cf_name: "Streamflow"
    description: "Streamflow"
    units: "m^3 sec-1"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 50000]
```

#### `config/models/nldas-forcing.yaml`

```yaml
model:
  id: nldas-forcing
  name: "NLDAS-2 Forcing"
  description: "North American Land Data Assimilation System - Forcing 0.125 hourly"
  enabled: true

dimensions:
  type: observation
  time: true
  elevation: true

source:
  type: nasa_gesdisc
  base_url: "https://hydro1.gesdisc.eosdis.nasa.gov/data/NLDAS/NLDAS_FORA0125_H.2.0"
  file_pattern: "NLDAS_FORA0125_H.A{date}.{hour}00.002.grb.SUB.nc4"

grid:
  projection: geographic
  resolution: "0.125deg"
  bbox:
    min_lon: -125.0
    min_lat: 25.0
    max_lon: -67.0
    max_lat: 53.0

schedule:
  type: observation
  poll_interval_secs: 3600
  delay_hours: 96

retention:
  hours: 720

parameters:
  - name: APCP
    description: "Precipitation Hourly Total"
    cf_name: "Rainf_f_tavg"
    units: "kg/m^2/s"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 0.1]

  - name: TMP2M
    description: "2-m Air Temperature"
    cf_name: "Tair_f_inst"
    units: "K"
    display_units: "C"
    conversion: K_to_C
    levels:
      - type: height_above_ground
        level_code: 103
        value: 2
        display: "2 m above ground"
    valid_range: [180, 350]

  - name: SPFH2M
    description: "2-m Specific Humidity"
    cf_name: "Qair_f_inst"
    units: "kg/kg"
    levels:
      - type: height_above_ground
        level_code: 103
        value: 2
        display: "2 m above ground"
    valid_range: [0, 0.1]

  - name: PRES
    description: "Surface Pressure"
    cf_name: "Psurf_f_inst"
    units: "Pa"
    display_units: "hPa"
    conversion: Pa_to_hPa
    levels:
      - type: surface
        display: "surface"
    valid_range: [50000, 110000]

  - name: WIND
    description: "10-m Wind Speed"
    cf_name: "Wind_f_inst"
    units: "m/s"
    levels:
      - type: height_above_ground
        level_code: 103
        value: 10
        display: "10 m above ground"
    valid_range: [0, 100]

  - name: DSWRF
    description: "Downward Shortwave Radiation Flux"
    cf_name: "SWdown_f_tavg"
    units: "W/m^2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 1500]

  - name: DLWRF
    description: "Downward Longwave Radiation Flux"
    cf_name: "LWdown_f_tavg"
    units: "W/m^2"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 700]

  - name: CAPE
    description: "Convective Available Potential Energy"
    cf_name: "CAPE"
    units: "J/kg"
    levels:
      - type: surface
        display: "surface"
    valid_range: [0, 10000]
```

#### `config/edr/nldas-noah.yaml`

```yaml
model: nldas-noah
data_type: observation

collections:
  - id: nldas-soil-moisture
    title: "NLDAS-2 Noah - Soil Moisture"
    description: "Soil moisture at 4 depth layers plus root zone (0.125 hourly)"
    level_filter:
      level_type: depth_below_surface
    parameters:
      - name: SOILM_0_10
        levels: [10]
        valid_range: { min: 0, max: 500 }
      - name: SOILM_10_40
        levels: [40]
        valid_range: { min: 0, max: 500 }
      - name: SOILM_40_100
        levels: [100]
        valid_range: { min: 0, max: 1000 }
      - name: SOILM_100_200
        levels: [200]
        valid_range: { min: 0, max: 1000 }
      - name: SOILM_ROOT
        levels: [100]
        valid_range: { min: 0, max: 1000 }
    run_mode: latest

  - id: nldas-soil-temperature
    title: "NLDAS-2 Noah - Soil Temperature"
    description: "Soil temperature at 4 depth layers (0.125 hourly)"
    level_filter:
      level_type: depth_below_surface
    parameters:
      - name: SOILT_0_10
        levels: [10]
        valid_range: { min: 200, max: 350 }
      - name: SOILT_10_40
        levels: [40]
        valid_range: { min: 200, max: 350 }
      - name: SOILT_40_100
        levels: [100]
        valid_range: { min: 200, max: 350 }
      - name: SOILT_100_200
        levels: [200]
        valid_range: { min: 200, max: 350 }
    run_mode: latest

  - id: nldas-snow
    title: "NLDAS-2 Noah - Snow"
    description: "Snow water equivalent and snow depth (0.125 hourly)"
    level_filter:
      level_type: surface
      level_code: 1
    parameters:
      - name: SWE
        levels: [surface]
        valid_range: { min: 0, max: 5000 }
      - name: SNOD
        levels: [surface]
        valid_range: { min: 0, max: 50 }
    run_mode: latest

  - id: nldas-fluxes
    title: "NLDAS-2 Noah - Surface Fluxes"
    description: "ET, heat fluxes, runoff, and radiation (0.125 hourly)"
    level_filter:
      level_type: surface
      level_code: 1
    parameters:
      - name: EVP
        levels: [surface]
        valid_range: { min: -0.001, max: 0.001 }
      - name: LHF
        levels: [surface]
        valid_range: { min: -500, max: 1000 }
      - name: SHF
        levels: [surface]
        valid_range: { min: -500, max: 1000 }
      - name: GHF
        levels: [surface]
        valid_range: { min: -500, max: 500 }
      - name: SSRUN
        levels: [surface]
        valid_range: { min: 0, max: 500 }
      - name: BGRUN
        levels: [surface]
        valid_range: { min: 0, max: 500 }
      - name: SWNET
        levels: [surface]
        valid_range: { min: 0, max: 1500 }
      - name: LWNET
        levels: [surface]
        valid_range: { min: -500, max: 200 }
    run_mode: latest

settings:
  output_formats:
    - application/vnd.cov+json
    - application/geo+json
    - image/png
  default_crs: "CRS:84"

limits:
  max_parameters_per_request: 10
  max_time_steps: 720
  max_vertical_levels: 5
  max_response_size_mb: 100
  max_area_sq_degrees: 500
  max_area_sq_degrees_png: 3000
  max_radius_km: 750
```

#### `config/edr/nldas-forcing.yaml`

```yaml
model: nldas-forcing
data_type: observation

collections:
  - id: nldas-forcing
    title: "NLDAS-2 Forcing"
    description: "Meteorological forcing: precip, temperature, humidity, wind, radiation (0.125 hourly)"
    level_filter:
      level_type: surface
      level_code: 1
    parameters:
      - name: APCP
        levels: [surface]
        valid_range: { min: 0, max: 0.1 }
      - name: TMP2M
        levels: [2]
        valid_range: { min: 180, max: 350 }
      - name: SPFH2M
        levels: [2]
        valid_range: { min: 0, max: 0.1 }
      - name: PRES
        levels: [surface]
        valid_range: { min: 50000, max: 110000 }
      - name: WIND
        levels: [10]
        valid_range: { min: 0, max: 100 }
      - name: DSWRF
        levels: [surface]
        valid_range: { min: 0, max: 1500 }
      - name: DLWRF
        levels: [surface]
        valid_range: { min: 0, max: 700 }
      - name: CAPE
        levels: [surface]
        valid_range: { min: 0, max: 10000 }
    run_mode: latest

settings:
  output_formats:
    - application/vnd.cov+json
    - application/geo+json
    - image/png
  default_crs: "CRS:84"

limits:
  max_parameters_per_request: 10
  max_time_steps: 720
  max_vertical_levels: 5
  max_response_size_mb: 100
  max_area_sq_degrees: 500
  max_area_sq_degrees_png: 3000
  max_radius_km: 750
```

### Phase 5: Scheduler Integration

**File:** `services/downloader/src/scheduler.rs`

**What:** Wire up the LIS runner alongside existing NDBC/DART runners.

**Details:**
- In `load_observation_configs()` or a new `load_lis_configs()`, detect model configs with `source.type: nasa_gesdisc`
- In `run_forever()`, spawn an `LisRunner` for each LIS model config
- The runner manages its own poll loop (same pattern as `ObservationRunner`, `DartRunner`)

### Phase 6: Validator Update

**File:** `config/models/validate.py`

**What:** Add `nasa_gesdisc` to `VALID_SOURCE_TYPES` set (line 40-48).

Also confirm `depth_below_surface` is in `VALID_LEVEL_TYPES` (it is -- line 73).

### Phase 7: End-to-End Testing

1. **Manual test:** Download a single NLDAS-2 Noah file from GES DISC, place it in the downloads directory, manually trigger ingestion, verify Zarr output and catalog entries
2. **Runner test:** Start the LIS runner, verify it authenticates, downloads, ingests, and queries work via EDR
3. **Integration test:** Add test in `services/edr-api/tests/` for NLDAS-2 collections

### Phase 8: WMS Layer Config (Optional, can defer)

- Add color scale definitions for soil moisture (brown-to-blue), snow (white-to-blue), fluxes (diverging)
- Add WMS model config entries matching the EDR collections
- This is purely styling/config -- no code changes needed

### Phase 9: Production Deployment

1. Set `EARTHDATA_USERNAME` and `EARTHDATA_PASSWORD` env vars in `deploy/production/docker-compose.prod.yml`
2. Deploy updated containers
3. Monitor initial backfill (30 days x 24 hours = 720 files)
4. Verify EDR queries return data
5. Monitor disk usage (~15 GB steady state)

---

## Files Modified/Created Summary

| File | Action | Phase |
|---|---|---|
| `crates/netcdf-parser/src/cf_reader.rs` | **New** -- Generic CF-convention NetCDF reader | 1 |
| `crates/netcdf-parser/src/lib.rs` | Modify -- Add `mod cf_reader` and re-exports | 1 |
| `crates/ingestion/src/cf_netcdf.rs` | **New** -- Multi-variable NetCDF to Zarr ingestion | 2 |
| `crates/ingestion/src/lib.rs` | Modify -- Add `mod cf_netcdf` | 2 |
| `crates/ingestion/src/ingester.rs` | Modify -- Route CF-convention NetCDF to new handler | 2 |
| `crates/ingestion/src/metadata.rs` | Modify -- Add NLDAS filename parsing and model detection | 2 |
| `services/downloader/src/lis_runner.rs` | **New** -- LIS/NLDAS download runner with Earthdata auth | 3 |
| `services/downloader/src/main.rs` | Modify -- Add `mod lis_runner` | 3 |
| `services/downloader/src/scheduler.rs` | Modify -- Spawn LIS runners | 5 |
| `config/models/nldas-noah.yaml` | **New** -- Noah model config | 4 |
| `config/models/nldas-forcing.yaml` | **New** -- Forcing model config | 4 |
| `config/edr/nldas-noah.yaml` | **New** -- Noah EDR collections | 4 |
| `config/edr/nldas-forcing.yaml` | **New** -- Forcing EDR collections | 4 |
| `config/models/validate.py` | Modify -- Add `nasa_gesdisc` to valid source types | 6 |
| `deploy/production/docker-compose.prod.yml` | Modify -- Add `EARTHDATA_USERNAME`/`EARTHDATA_PASSWORD` env vars | 9 |

---

## EDR API: Level String Handling for Depth Layers

Our current `build_level_string()` in `services/edr-api/src/config.rs` does not handle `depth_below_surface` level types. A new case needs to be added:

```rust
"depth_below_surface" => {
    // Depth layers stored as "X-Y cm depth" or "0-X cm depth"
    level_value.map(|v| format!("{} cm depth", v as i32))
}
```

This is needed for the soil moisture and soil temperature collections to resolve correctly.

---

## Risk Areas

| Risk | Mitigation |
|---|---|
| Earthdata OAuth redirect flow in reqwest | Tested and working with `--netrc-file` + cookie jar. In Rust: reqwest with `cookie_store(true)` + custom redirect policy that injects Basic auth on URS host. |
| NLDAS-2 variable names may differ from documentation | **RESOLVED**: Downloaded sample file and verified all 40 variable names. Plan updated with correct cf_names. |
| GES DISC app authorization | **RESOLVED**: Must pre-authorize "NASA GESDISC DATA ARCHIVE" app at URS. Already done. Document this as a setup step. |
| Grid orientation (south-to-north vs north-to-south) | Detect from lat coordinate array. Zarr writer already supports `RowOrigin::South`. |
| GES DISC rate limiting | Limit concurrent downloads (max 5). Add delays between requests during initial backfill. |
| 30-day backfill = 720 files at ~15 MB each = ~10.5 GB download | Spread over time. First run may take several hours. Not a blocker. |
| `depth_below_surface` level type needs new `build_level_string()` mapping | Add new case in `edr-api/src/config.rs`. Small change. |

---

## Open Questions

1. ~~**CF variable names**~~: **RESOLVED** -- Downloaded sample file. All 40 variable names verified and updated in plan.
2. **Forcing file pattern**: Still need to verify the exact URL/filename for NLDAS-2 Forcing NetCDF-4 files. May need to download a sample from the Forcing dataset.
3. **Level string format for depth layers**: Need to decide on canonical format (e.g., `"0-10 cm depth"` vs `"10 cm depth"`) and ensure consistency between ingestion and EDR query resolution.
4. **Variable selection**: The Noah file has 40 variables. Some are diagnostic/internal (ACond, CCond, RCS, RCT, RCQ, RCSOL, RSmin, RSMacr, SMLiq_* variants, SMAvail_*). We should ingest the high-value ones and skip the diagnostic ones to avoid unnecessary storage.

---


## Future Expansion

Once the NLDAS-2 pipeline is working, adding other LIS products is primarily config work:

| Product | Additional Code Needed | Config Only |
|---|---|---|
| GLDAS-2.1 (global 0.25) | None -- same CF-convention reader | New model + EDR YAML files |
| FLDAS (Africa 0.10) | None | New model + EDR YAML files |
| NCA-LDAS (CONUS 0.125) | None | New model + EDR YAML files |

The `lis_runner.rs` handles all products -- just different base URLs and file patterns configured in YAML.
