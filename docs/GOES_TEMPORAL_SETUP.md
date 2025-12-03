# GOES-19 Temporal Testing Setup

## Overview

We've created infrastructure for downloading and testing temporal GOES-19 satellite data. The script downloads Level 1b Radiance data (ABI-L1b-RadC) from GOES-19 (GOES-West) which provides CONUS imagery every ~5 minutes.

## Download Script Created

**File**: `./scripts/download_goes_temporal.sh`

**Features**:
- Downloads GOES-19 ABI-L1b-RadC (CONUS Radiances)
- Three bands: Band 02 (Visible), Band 08 (Water Vapor), Band 13 (Clean IR)
- Temporal coverage: Last N hours (configurable)
- Update frequency: ~5 minutes per scan
- Direct HTTP download (no AWS CLI required)

**Usage**:
```bash
# Download last 3 hours (default)
./scripts/download_goes_temporal.sh

# Download last 6 hours, max 30 files per band
GOES_HOURS=6 MAX_FILES=30 ./scripts/download_goes_temporal.sh

# Custom output directory
OUTPUT_DIR=./data/goes-test GOES_HOURS=4 ./scripts/download_goes_temporal.sh
```

**Expected output**:
- Band 02: ~8-12 files/hour (day only)
- Band 08: ~12 files/hour (24/7)
- Band 13: ~12 files/hour (24/7)
- File sizes: ~2-30 MB per file (varies by band and resolution)

## GOES-19 Data Properties

```
Satellite: GOES-19 (GOES-West, launched 2024)
Product: ABI-L1b-RadC (Level 1b Radiances, CONUS)
Scan Mode: Mode 6 (CONUS scan)
Coverage: Continental United States
Update Frequency: ~5 minutes
Format: NetCDF-4
Projection: Geostationary (Fixed Earth Grid)

Band Details:
  Band 02: Red Visible (0.64 µm)
    - Resolution: 0.5 km
    - Use: Clouds, fog detection
    - Availability: Daytime only
    - File size: ~28 MB
  
  Band 08: Water Vapor (6.19 µm)
    - Resolution: 2 km
    - Use: Upper-level moisture
    - Availability: 24/7
    - File size: ~2.8 MB
  
  Band 13: Clean IR (10.35 µm)
    - Resolution: 2 km
    - Use: Cloud-top temperatures
    - Availability: 24/7
    - File size: ~2.8 MB
```

## File Naming Convention

GOES files follow this pattern:
```
OR_ABI-L1b-RadC-M6C08_G19_s20253321456174_e20253321458547_c20253321459041.nc
│  │            │  │   │  │             │  │             │  │
│  │            │  │   │  │             │  │             │  └─ Creation time
│  │            │  │   │  │             │  │             └──── End time
│  │            │  │   │  │             │  └────────────────── Scan start time (sYYYYDDDHHMMSSS)
│  │            │  │   │  │             └───────────────────── Satellite (G19 = GOES-19)
│  │            │  │   │  └─────────────────────────────────── Channel/Band
│  │            │  │   └────────────────────────────────────── Scan mode (M6 = Mode 6 CONUS)
│  │            │  └────────────────────────────────────────── Product
│  └───────────────────────────────────────────────────────── Agency/Instrument
└──────────────────────────────────────────────────────────── Operational/Real-time
```

**Scan start time format**: `sYYYYDDDHHMMSSS`
- YYYY = Year
- DDD = Day of year (001-366)
- HH = Hour (00-23)
- MM = Minute (00-59)
- SS = Second (00-59)
- S = Tenth of second

**Example**: `s20253321456174`
- Year: 2025
- Day: 332 (November 28)
- Time: 14:56:17.4 UTC

## Creating Temporal Test Scenarios

Once you have downloaded GOES data, create temporal load test scenarios similar to MRMS:

### Step 1: Extract Timestamps

```bash
# Extract timestamps from GOES Band 02 filenames
ls ./data/goes-temporal/band02/*.nc | \
  xargs -n1 basename | \
  grep -oP "s\d{14}" | \
  sed 's/s\(....\)\(...\)\(..\)\(..\)\(..\).*/\1-\2-\3T\4:\5:\6Z/' | \
  sed 's/-\(...\)-/-\1T/' | \
  while read ts; do
    # Convert day-of-year to month-day
    python3 -c "from datetime import datetime; dt = datetime.strptime('$ts', '%Y-%jT%H:%M:%SZ'); print(dt.strftime('%Y-%m-%dT%H:%M:%SZ'))"
  done
```

Or use this simpler approach that generates ISO 8601 directly from the scan time:

```bash
# Parse GOES timestamps to ISO 8601
for file in ./data/goes-temporal/band02/*.nc; do
  filename=$(basename "$file")
  # Extract sYYYYDDDHHMMSSS
  scan_time=$(echo "$filename" | grep -oP "s\d{14}")
  year=${scan_time:1:4}
  doy=${scan_time:5:3}
  hour=${scan_time:8:2}
  minute=${scan_time:10:2}
  second=${scan_time:12:2}
  
  # Convert to ISO 8601 (requires date command or Python)
  python3 -c "from datetime import datetime, timedelta; base = datetime($year, 1, 1); dt = base + timedelta(days=$doy-1, hours=$hour, minutes=$minute, seconds=$second); print(dt.strftime('%Y-%m-%dT%H:%M:%SZ'))"
done
```

### Step 2: Create Test Scenario YAML

Create `validation/load-test/scenarios/goes_temporal_stress.yaml`:

```yaml
name: goes_temporal_stress
description: |
  Temporal load test for GOES-19 satellite imagery.
  Tests cache behavior across multiple time steps with varying zoom levels.

base_url: http://localhost:8080
duration_secs: 300
concurrency: 30
warmup_secs: 10
seed: 77777

layers:
  - name: goes19_VIS  # Visible imagery
    style: goes_visible
    weight: 1.0
  - name: goes19_WV   # Water Vapor
    style: goes_ir
    weight: 0.8
  - name: goes19_IR   # Clean IR
    style: goes_ir
    weight: 0.9

tile_selection:
  type: random
  zoom_range: [3, 10]  # GOES has lower resolution than MRMS
  bbox:
    # CONUS coverage
    min_lon: -130.0
    min_lat: 20.0
    max_lon: -60.0
    max_lat: 55.0

time_selection:
  type: sequential
  times:
    # Insert extracted timestamps here
    - "2025-11-28T14:56:17Z"
    - "2025-11-28T15:01:17Z"
    - "2025-11-28T15:06:17Z"
    # ... (add all downloaded timestamps)
```

### Step 3: Single-Tile Temporal Test

Create `validation/load-test/scenarios/goes_single_tile_temporal.yaml`:

```yaml
name: goes_single_tile_temporal
description: |
  Single-tile temporal test for GOES-19.
  Isolates temporal caching from spatial variation.

base_url: http://localhost:8080
duration_secs: 60
concurrency: 5
warmup_secs: 5
seed: 88888

layers:
  - name: goes19_IR
    style: goes_ir
    weight: 1.0

tile_selection:
  type: fixed
  tiles:
    # Fixed tile covering central US
    - [5, 7, 10]

time_selection:
  type: sequential
  times:
    # Add extracted timestamps
    - "2025-11-28T14:56:17Z"
    - "2025-11-28T15:01:17Z"
    # ... etc
```

## Comparison: GOES vs MRMS Temporal Characteristics

| Aspect | MRMS | GOES-19 |
|--------|------|---------|
| **Update Frequency** | ~2 minutes | ~5 minutes |
| **Coverage** | CONUS ground radar | CONUS satellite |
| **Resolution** | 0.01° (~1 km) | 0.5-2 km (varies by band) |
| **Files per Hour** | ~30 | ~12 |
| **File Size** | 380-430 KB | 2.8-30 MB |
| **Cache Pressure** | Lower (small files) | Higher (larger files) |
| **Data Type** | GRIB2 (grid) | NetCDF-4 (grid) |
| **Projection** | Lat-Lon | Geostationary |
| **Temporal Depth** | Hours (limited archive) | Days-weeks on S3 |

## Expected Cache Behavior

### GRIB/NetCDF Cache (In-Memory)
- **MRMS**: 59 files × 400 KB = ~24 MB
- **GOES**: 36 files × 3 MB = ~108 MB (12 files/hour × 3 hours)
- **GOES (Band 02)**: 36 files × 28 MB = ~1 GB (high pressure!)

**Recommendation**: Start with Band 08 or 13 (smaller files) for initial temporal tests.

### Rendered Tile Cache (Redis)
- Same as MRMS: depends on zoom level and tile variety
- GOES may have slightly fewer unique tiles due to lower resolution

## Test Execution Plan

### Phase 1: Download Data
```bash
# Download 3 hours of Band 08 (Water Vapor) - smaller files
GOES_HOURS=3 MAX_FILES=36 OUTPUT_DIR=./data/goes ./scripts/download_goes_temporal.sh
```

### Phase 2: Ingest Data
```bash
# Ingest GOES NetCDF files
for nc_file in ./data/goes/band08/*.nc; do
  cargo run --package ingester -- --test-file "$nc_file"
done
```

### Phase 3: Extract Timestamps and Create Scenarios
```bash
# Extract timestamps (save to file)
./scripts/extract_goes_timestamps.sh > /tmp/goes_times.txt

# Manually create YAML scenarios with these timestamps
```

### Phase 4: Run Tests
```bash
# Single-tile temporal test
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_single_tile_temporal.yaml

# Full temporal stress test
cargo run --package load-test -- run \
  --scenario validation/load-test/scenarios/goes_temporal_stress.yaml
```

## Known Limitations

1. **Download Speed**: GOES files are larger (especially Band 02 at ~28 MB), so downloads take longer
2. **Cache Pressure**: Band 02 files may exceed reasonable cache sizes for temporal testing
3. **Ingestion**: NetCDF parser must support GOES geostationary projection
4. **Layer Configuration**: GOES layers must be configured in WMS catalog with correct projection

## Recommendations

### For Initial Testing:
1. **Use Band 08 or 13** (Water Vapor or Clean IR) - smaller files (~2.8 MB)
2. **Limit temporal range**: 2-3 hours = 24-36 files
3. **Zoom range**: 3-8 (GOES resolution doesn't benefit from higher zooms)
4. **Start with single-tile tests** to verify temporal caching works

### For Production:
1. **Separate cache tiers**: Consider separate GRIB and NetCDF caches
2. **Size limits**: Band 02 files may need different cache strategy
3. **Time-based eviction**: Prioritize recent times over old
4. **Per-band configuration**: Different cache sizes for different bands

## Helper Script: Extract Timestamps

Create `./scripts/extract_goes_timestamps.sh`:

```bash
#!/bin/bash
# Extract GOES timestamps and convert to ISO 8601

BAND_DIR="${1:-./data/goes/band08}"

for file in "$BAND_DIR"/*.nc; do
  filename=$(basename "$file")
  scan_time=$(echo "$filename" | grep -oP "s\d{14}")
  
  if [ -n "$scan_time" ]; then
    year=${scan_time:1:4}
    doy=${scan_time:5:3}
    hour=${scan_time:8:2}
    minute=${scan_time:10:2}
    second=${scan_time:12:2}
    
    python3 -c "from datetime import datetime, timedelta; base = datetime($year, 1, 1); dt = base + timedelta(days=$doy-1, hours=$hour, minutes=$minute, seconds=$second); print(dt.strftime('%Y-%m-%dT%H:%M:%SZ'))"
  fi
done | sort
```

## Status

- ✅ Download script created and tested
- ✅ GOES-19 S3 bucket confirmed accessible
- ✅ File naming convention documented
- ✅ Timestamp extraction method provided
- ⏳ Full temporal download (in progress - slow due to file sizes)
- 🔲 Create timestamp extraction script
- 🔲 Create GOES temporal test scenarios
- 🔲 Ingest GOES data
- 🔲 Run temporal tests

## Next Steps

1. **Complete download**: Let the download script finish or re-run with smaller MAX_FILES
2. **Create helper script**: `extract_goes_timestamps.sh` for easy timestamp extraction
3. **Create test scenarios**: YAML files with extracted timestamps
4. **Test ingestion**: Verify NetCDF parser handles GOES projection
5. **Run tests**: Start with single-tile, then progress to full stress tests

---

**Note**: Due to larger file sizes, GOES temporal testing will have different cache characteristics than MRMS. Consider this when analyzing results and setting cache size limits.
