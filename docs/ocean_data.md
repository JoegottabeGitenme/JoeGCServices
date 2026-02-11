# NDBC Data Access Reference

> Based on the [NDBC Web Data Guide (June 2023)](https://www.ndbc.noaa.gov/docs/ndbc_web_data_guide.pdf)
> and live directory inspection. All data is free and public domain.

---

## Station Discovery & Metadata

| Resource | URL | Format | Update Cadence | Approx Size |
|----------|-----|--------|----------------|-------------|
| Active Station List | `https://www.ndbc.noaa.gov/activestations.xml` | XML | ~5 min (as needed) | ~300 KB |
| Station Metadata (historical back to 2000) | `https://www.ndbc.noaa.gov/metadata/stationmetadata.xml` | XML | Daily @ 05:00/06:00 UTC | ~2–5 MB |
| Station Metadata XSD Schema | `https://www.ndbc.noaa.gov/metadata/stationmetadata.xsd` | XSD | Static | < 10 KB |
| KML – Observations by Program | `https://www.ndbc.noaa.gov/kml/marineobs_by_pgm.kml` | KML | Periodic | ~500 KB |
| KML – Observations by Owner | `https://www.ndbc.noaa.gov/kml/marineobs_by_owner.kml` | KML | Periodic | ~500 KB |

**Notes:**
- As of the last check, `activestations.xml` reported **1,366 stations**.
- The active stations XML includes: station ID, lat/lon, name, owner, program, type, and boolean flags for `met`, `currents`, `waterquality`, and `dart` data availability.
- Changes are driven by service visits, new station deployments, or sensor failures — expect minimal updates per 24-hour period.

---

## Realtime Data (Last 45 Days)

**Base URL:** `https://www.ndbc.noaa.gov/data/realtime2/`

**File naming:** `{STATION_ID}.{extension}` — CMAN/land stations use UPPERCASE (e.g., `FPSN7.txt`).

| Extension | Data Type | Description | Typical Size (45 days) |
|-----------|-----------|-------------|----------------------|
| `.txt` | Standard Meteorological | Wind, pressure, air/water temp, visibility, waves | 85–100 KB |
| `.cwind` | Continuous Winds | 10-minute averaged wind speed/direction | 50–80 KB |
| `.spec` | Spectral Wave Summary | Significant height, peak period, direction | 40–60 KB |
| `.data_spec` | Raw Spectral Wave | Full spectral energy density data | 200–500 KB |
| `.swdir` | Spectral Wave (alpha1) | Mean wave direction per frequency bin | 100–300 KB |
| `.swdir2` | Spectral Wave (alpha2) | Principal wave direction per frequency bin | 100–300 KB |
| `.swr1` | Spectral Wave (r1) | First normalized polar coordinate | 100–300 KB |
| `.swr2` | Spectral Wave (r2) | Second normalized polar coordinate | 100–300 KB |
| `.ocean` | Oceanographic | Water temp profiles, salinity, currents, O₂ | 70–85 KB |
| `.srad` | Solar Radiation | Shortwave, longwave, UV radiation | 35–40 KB |
| `.dart` | DART/Tsunami | Water column height (tsunameter) | 60–130 KB |
| `.supl` | Supplemental | Additional measurements (varies by station) | 30–60 KB |
| `.rain` | Rainfall | Hourly rain accumulation | 20–40 KB |
| `.adcp` | ADCP | Current profiles at depth (speed/direction) | 50–200 KB |
| `.drift` | Drifting Buoy | Met data from drifting buoys / international partners | 85–95 KB |
| `.tide` | Tide/Water Level | Non-DART water level | 30–60 KB |

**Update cadence:** Most stations report **hourly**; data available by ~25 minutes past the hour. Continuous wind stations may report at **10-minute intervals**. DART stations in standby report every **15 minutes**, switching to **1-minute or 15-second** intervals during tsunami events.

**Example URLs:**
```
https://www.ndbc.noaa.gov/data/realtime2/41002.txt        # Standard met, station 41002
https://www.ndbc.noaa.gov/data/realtime2/41002.spec       # Spectral wave summary
https://www.ndbc.noaa.gov/data/realtime2/41002.ocean      # Oceanographic data
https://www.ndbc.noaa.gov/data/realtime2/46087.cwind      # Continuous winds
https://www.ndbc.noaa.gov/data/realtime2/21413.dart       # DART tsunami data
https://www.ndbc.noaa.gov/data/realtime2/FPSN7.txt        # CMAN station (uppercase)
```

---

## Latest Observations (All Stations, Single File)

| Resource | URL | Format | Update Cadence | Approx Size |
|----------|-----|--------|----------------|-------------|
| Latest observations (all stations) | `https://www.ndbc.noaa.gov/data/latest_obs/latest_obs.txt` | Fixed-width text | ~5 minutes | < 100 KB |

**Best use case:** If you want a single poll to get the most recent observation from every active station. Only includes observations less than 2 hours old. Includes lat/lon per station.

**Columns:** `STN, LAT, LON, YYYY, MM, DD, hh, mm, WDIR, WSPD, GST, WVHT, DPD, APD, MWD, PRES, PTDY, ATMP, WTMP, DEWP, VIS, TIDE`

Missing values are reported as `MM`.

---

## 5-Day Data (All Active Stations)

**Base URL:** `https://www.ndbc.noaa.gov/data/5day2/`

Same file extensions as realtime2. Contains only the last 5 days of data per station. Useful if you don't need the full 45-day window and want smaller files.

---

## Hourly Snapshots (All Stations per Hour)

**Base URL:** `https://www.ndbc.noaa.gov/data/hourly2/`

Contains subdirectories for hours 0–23 (UTC). Each file contains observations from **all stations** for that hour. Data types: ADCP, continuous winds, oceanographic, spectral wave summary, solar radiation, supplemental, and standard met.

**Note:** Directories for hours not yet reached contain the previous day's data.

---

## Monthly Data (Current Calendar Year)

**Base URL pattern:** `https://www.ndbc.noaa.gov/data/{datatype}/`

| Data Type Directory | Contents |
|---------------------|----------|
| `stdmet/` | Standard meteorological, by station, by month |
| `ocean/` | Oceanographic data, by station, by month |
| `cwind/` | Continuous winds, by station, by month |
| `adcp/` | ADCP profiles (high-level QC), by station, by month |
| `adcp2/` | ADCP profiles (full RDI QC, mostly Gulf oil rigs) |
| `dart/` | DART water column height, by station, by month |
| `drift/` | Drifting buoy data, by station, by month |
| `swden/` | Spectral wave density, by station, by month |
| `swdir/` | Spectral wave direction (alpha1), by station, by month |
| `srad/` | Solar radiation, by station, by month |
| `rain/` | Hourly rain, by station, by month |
| `rain10/` | 10-minute rain, by station, by month |
| `rain24/` | Daily rain, by station, by month |
| `supl/` | Supplemental measurements, by station, by month |

---

## Previous Month Data

**Base URL pattern:** `https://www.ndbc.noaa.gov/data/l_{datatype}/`

Updated around mid-month for the previous month. Available types:

`l_stdmet`, `l_cwind`, `l_ocean`, `l_dart`, `l_drift`, `l_srad`, `l_supl`, `l_swden`, `l_swdir`, `l_swdir2`, `l_swr1`, `l_swr2`, `l_adcp`, `l_adcp2`, `l_wlevel`

---

## Historical Data (Previous Calendar Years)

**Base URL:** `https://www.ndbc.noaa.gov/data/historical/`

Subdirectories mirror the data types above. Files are organized by station and calendar year.

**Per-station history page:**
```
https://www.ndbc.noaa.gov/station_history.php?station={STATION_ID}
```

Sections include:
- Quality-controlled data for the current year (by month)
- Historical data (by calendar year)
- Conditional threshold search
- Climatic summary tables and plots

---

## DART Tsunami Data (Custom Query)

**Query URL:**
```
https://www.ndbc.noaa.gov/dart_data.php?station={ID}&startmonth={M}&startday={D}&startyear={YYYY}&endmonth={M}&endday={D}&endyear={YYYY}
```

**Example:**
```
https://www.ndbc.noaa.gov/dart_data.php?station=43412&startmonth=6&startday=01&startyear=2023&endmonth=6&endday=6&endyear=2023
```

Returns plain text with columns: `YYYY MM DD hh mm ss T HEIGHT(meters)`

Measurement types: `1` = 15-min, `2` = 1-min, `3` = 15-sec

---

## THREDDS / OPeNDAP / NetCDF Access (DODS Server)

**Base URL:** `https://dods.ndbc.noaa.gov/`

**THREDDS Catalog:** `https://dods.ndbc.noaa.gov/thredds/catalog.html`

Data served as **NetCDF** with **CF Metadata Conventions** via OPeNDAP. Organized by type, then by station within each type. Both historical and realtime data available as separate files per year plus a current realtime file.

| Dataset | THREDDS Path | Description |
|---------|-------------|-------------|
| Standard Met | `thredds/catalog/data/stdmet/` | Wind, pressure, temp, visibility |
| Continuous Winds | `thredds/catalog/data/cwind/` | High-frequency wind obs |
| Oceanographic | `thredds/catalog/data/ocean/` | Water temp, salinity, waves |
| Spectral Wave Density | `thredds/catalog/data/swden/` | Wave energy by frequency |
| ADCP | `thredds/catalog/data/adcp/` | Current profiles at depth |
| DART | `thredds/catalog/data/dart/` | Tsunami water column height |
| Peak Winds | `thredds/catalog/data/pwind/` | Peak wind events |
| Water Level | `thredds/catalog/data/wlevel/` | Tide (non-DART) |
| Marsh-McBirney Currents | `thredds/catalog/data/mbcurr/` | Point current measurements |

**Additional DODS datasets:**

| Dataset | URL | Description |
|---------|-----|-------------|
| HF Radar (aggregated) | `https://dods.ndbc.noaa.gov/thredds/hfradar.html` | 4-day aggregated surface currents by region |
| HF Radar (hourly) | `https://dods.ndbc.noaa.gov/thredds/catalog/hfradar/catalog.html` | Individual hourly gridded files |
| TAO Buoy Data | `https://dods.ndbc.noaa.gov/thredds/catalog/oceansites/` | Equatorial Pacific climate array |
| TAO CTD Data | DODS > tao-ctd | Cruise-collected CTD profiles |
| OceanSITES | `https://dods.ndbc.noaa.gov/oceansites/` | Global GDAC data |

---

## HF Radar Surface Currents

| Resource | URL | Format | Notes |
|----------|-----|--------|-------|
| HF Radar Main Page | `https://hfradar.ndbc.noaa.gov/` | Interactive map | Tabular download at bottom of page |
| HF Radar THREDDS (aggregated) | `https://dods.ndbc.noaa.gov/thredds/hfradar.html` | NetCDF via OPeNDAP | 4-day aggregated, by region/resolution |
| HF Radar THREDDS (hourly) | `https://dods.ndbc.noaa.gov/thredds/catalog/hfradar/catalog.html` | NetCDF via OPeNDAP | Individual hourly files |

---

## TAO (Tropical Atmosphere Ocean) Array

**Data Download Page:** `https://tao.ndbc.noaa.gov/tao/data_download/search_map.shtml`

User-selectable: stations, data types, date ranges, temporal resolution, file format. Limited to TAO equatorial Pacific stations only.

---

## Supplementary Resources

| Resource | URL | Description |
|----------|-----|-------------|
| Observation Search (radial/box) | `https://www.ndbc.noaa.gov/os.shtml` | Query by lat/lon, screen only |
| Ship Observations | `https://www.ndbc.noaa.gov/ship_obs.php` | Last 12 hours of ship obs |
| Historical Conditional Search | `https://www.ndbc.noaa.gov/histsearch.php` | Threshold-based historical queries |
| BuoyCAM Images | `https://www.ndbc.noaa.gov/buoycam.php?station={ID}` | Latest camera image (daylight only) |
| Observation Widget (HTML embed) | `https://www.ndbc.noaa.gov/widgets/` | Embed latest obs on your site |
| Measurement Descriptions & Units | `https://www.ndbc.noaa.gov/measdes.shtml` | Parameter definitions |
| Data Directory (browse all) | `https://www.ndbc.noaa.gov/data/` | Root of all data subdirectories |
| Derived Data (45 days) | `https://www.ndbc.noaa.gov/data/derived2/` | Wind chill, heat index, icing, 10/20m winds |
| Climatic Summaries | `https://www.ndbc.noaa.gov/data/climatic/` | Per-station (not updated since 2013) |
| Station Elevation/Sensor Metadata | `https://www.ndbc.noaa.gov/data/stations/` | Daily-updated station metadata files |

---

## Recommended Polling Strategy

| Use Case | Best Source | Poll Frequency |
|----------|-----------|----------------|
| All stations, latest obs only | `latest_obs.txt` | Every 5–10 min |
| Single station, full parameters | `realtime2/{station}.{ext}` | Hourly (by :25 past) |
| Station inventory changes | `activestations.xml` | Every 15–60 min |
| Bulk historical ingest | THREDDS/OPeNDAP (NetCDF) | On-demand |
| Tsunami monitoring | `realtime2/{station}.dart` | Every 5 min |
| Surface currents (HF Radar) | THREDDS HF Radar | Hourly |

> **NDBC asks** that you limit retrieval frequency to match actual data update cadence to conserve bandwidth. Avoid screen scraping — use the documented endpoints above.

---

## File Format Summary

| Access Method | Format | Best For |
|---------------|--------|----------|
| `realtime2/`, `5day2/`, `latest_obs/` | Fixed-width ASCII text | Simple HTTP polling, wget, scripts |
| `station_history.php` | HTML (interactive) | Manual browsing |
| `activestations.xml`, `stationmetadata.xml` | XML | Station discovery, metadata sync |
| THREDDS / DODS | NetCDF (CF conventions) | OPeNDAP clients, programmatic access, GIS |
| KML feeds | KML | Google Earth, GIS overlay |
| TAO download page | User-selectable (CSV, etc.) | TAO-specific research |
| DART query | Plain text | Custom date-range tsunami data |