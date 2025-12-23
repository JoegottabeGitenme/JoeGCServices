# GOES (Geostationary Satellites)

NOAA's geostationary weather satellites providing continuous imagery of the Western Hemisphere.

## Overview

- **Satellites**: GOES-16 (East), GOES-18 (West)
- **Orbit**: Geostationary (35,786 km altitude)
- **Coverage**: 
  - GOES-16: Eastern CONUS, Atlantic, South America
  - GOES-18: Western CONUS, Eastern Pacific
- **Resolution**: 0.5-2 km (channel dependent)
- **Update Frequency**: 5-15 minutes (full disk)
- **Format**: NetCDF-4

## Satellites

### GOES-16 (East)

- **Position**: 75.2°W
- **Coverage**: Atlantic, Eastern US
- **Scan Modes**:
  - Full Disk: Every 15 minutes
  - CONUS: Every 5 minutes
  - Mesoscale: Every 1 minute (2 regions)

### GOES-18 (West)

- **Position**: 137.2°W
- **Coverage**: Pacific, Western US
- **Scan Modes**: Same as GOES-16

## Channels

### Visible/Near-IR

| Channel | Wavelength | Name | Resolution | Use |
|---------|------------|------|------------|-----|
| C01 | 0.47 µm | Blue | 1 km | Aerosols, daytime |
| C02 | 0.64 µm | Red | 0.5 km | Clouds, fog |
| C03 | 0.86 µm | Near-IR | 1 km | Vegetation, snow |

### Infrared

| Channel | Wavelength | Name | Resolution | Use |
|---------|------------|------|------------|-----|
| C07 | 3.9 µm | Shortwave IR | 2 km | Fog, low clouds |
| C08 | 6.2 µm | Water vapor (upper) | 2 km | Upper-level moisture |
| C09 | 6.9 µm | Water vapor (mid) | 2 km | Mid-level moisture |
| C10 | 7.3 µm | Water vapor (lower) | 2 km | Lower-level moisture |
| **C13** | **10.3 µm** | **Clean longwave IR** | **2 km** | **Cloud-top temp** |
| C14 | 11.2 µm | Longwave IR | 2 km | Cloud imagery |
| C15 | 12.3 µm | Dirty longwave IR | 2 km | Volcanic ash |
| C16 | 13.3 µm | CO2 longwave IR | 2 km | Air temperature |

**Most Used**: **Channel 13** (10.3 µm) - Standard infrared imagery

## Layer Names

Examples:
- `goes16_CMI_C13` - GOES-16 Channel 13 (IR)
- `goes16_CMI_C02` - GOES-16 Channel 2 (Visible)
- `goes18_CMI_C13` - GOES-18 Channel 13 (IR)
- `goes18_CMI_C08` - GOES-18 Channel 8 (Water Vapor)

## Data Source

**AWS S3 (NOAA Open Data)**:
```
s3://noaa-goes{SATELLITE}/ABI-L2-CMIPC/{YYYY}/{DDD}/{HH}/OR_ABI-L2-CMIPC-M6C{CHANNEL}_G{SATELLITE}_s{START}_e{END}_c{CREATED}.nc
```

**Example**:
```
s3://noaa-goes18/ABI-L2-CMIPC/2024/338/18/OR_ABI-L2-CMIPC-M6C13_G18_s20243371800207_e20243371809515_c20243371810002.nc
```

### Efficient File Discovery

The downloader uses S3's `start_after` parameter to efficiently discover files for specific channels. Since files are sorted lexicographically (C01 < C02 < ... < C16), we can skip directly to the desired channel:

```
start_after: ABI-L2-CMIPC/2024/338/18/OR_ABI-L2-CMIPC-M6C13_G18_
```

This avoids listing all files in an hour directory (which may contain hundreds of files across all 16 channels) and instead jumps directly to the target channel, discovering all ~12 timesteps per hour efficiently.

## File Sizes

- Per file: 40-400 MB (channel/resolution dependent)
  - C13 (IR, 2km): ~40 MB
  - C02 (Visible, 0.5km): ~400 MB
- Per hour (16 channels, 4-12 scans): ~3-5 GB
- Per day: ~72-120 GB

## Download Script

```bash
./scripts/download_goes.sh

# Downloads recent GOES-18 imagery
# Channels: C02 (visible), C13 (IR)
# Total: ~500 MB
```

## Typical Uses

- **Visible imagery** (C02): Daytime clouds, fog, storm structure
- **Infrared** (C13): 24/7 cloud-top temperature, storm intensity
- **Water vapor** (C08-C10): Upper-air moisture, jet stream
- **Fog detection**: C02 + C07 difference
- **Fire detection**: C07 shortwave IR hotspots
- **Hurricane tracking**: Eye definition, intensity estimation
- **Aviation**: Cloud-top heights, convection

## Projection

GOES uses **geostationary projection** - a specialized projection for satellite view geometry. See [Projection Crate](../crates/projection.md#geostationary) for details.
