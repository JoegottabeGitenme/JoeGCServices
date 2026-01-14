# 48-Hour Forecast Coverage - Configuration Changes

**Date**: January 2026  
**Purpose**: Enable full "Today + Tomorrow" (48-hour) forecast coverage for FolkWeather frontend application

## Summary of Changes

Modified retention and forecast horizon settings for three weather models to ensure consistent 48-hour forecast coverage:

| Model | Previous Coverage | New Coverage | Storage Impact |
|-------|------------------|--------------|----------------|
| **GFS** | 24 hours | 48 hours | +25 GB |
| **HRRR** | Inconsistent (18-48h) | Consistent 48h | +24 GB |
| **NBM-CONUS** | 48 hours (2 runs) | 48 hours (3 runs) | +14 GB |
| **Total** | - | - | **+63 GB** |

---

## Detailed Changes

### 1. GFS Configuration
**File**: `config/models/gfs.yaml`

**Changes**:
```yaml
schedule:
  forecast_hours:
    end: 24 → 48  # Download 48 hours of forecast instead of 24

retention:
  hours: 12 → 24              # Retain data for 24 hours
  keep_latest_runs: 2 → 3     # Keep 3 complete runs minimum
```

**Rationale**:
- GFS previously only downloaded 24 hours of its 384-hour forecast capability
- Limited "Today + Tomorrow" coverage to partial second day
- Extending to 48 hours provides full two-day coverage
- 24-hour retention ensures at least one complete 48h forecast survives cleanup
- 3 runs provides safety margin for download gaps

**Storage Impact**:
- Previous: 2 runs × 25 hours × 208 MB/hour = ~10.4 GB
- New: 3 runs × 49 hours × 208 MB/hour = ~30.6 GB
- **Additional: ~20 GB**

---

### 2. HRRR Configuration
**File**: `config/models/hrrr.yaml`

**Changes**:
```yaml
retention:
  hours: 3 → 12               # Retain data for 12 hours
  keep_latest_runs: 3 → 4     # Keep 4 complete runs minimum
```

**Rationale**:
- HRRR runs hourly but forecast horizon varies:
  - **Major runs** (00z, 06z, 12z, 18z): 0-48 hours
  - **Intermediate runs** (all other hours): 0-18 hours only
- Previous 3-hour retention often missed major runs, causing gaps in 48h coverage
- 12-hour retention ensures at least 2 major runs with full 48h forecasts
- 4 runs provides buffer for intermediate hourly runs

**Storage Impact**:
- Previous: 3 runs × 49 hours × 247 MB/hour = ~36.3 GB
- New: 4 runs × 49 hours × 247 MB/hour = ~48.4 GB
- **Additional: ~12 GB**

**Note**: Actual storage varies as intermediate runs only store 18h vs 48h.

---

### 3. NBM-CONUS Configuration
**File**: `config/models/nbm-conus.yaml`

**Changes**:
```yaml
retention:
  hours: 6 → 12               # Retain data for 12 hours
  keep_latest_runs: 2 → 3     # Keep 3 complete runs minimum
```

**Rationale**:
- NBM already downloads 1-48 hours of forecast (no change needed)
- NBM runs hourly with consistent 48h+ forecasts (more reliable than HRRR)
- Increasing retention from 2 to 3 runs provides better redundancy
- Acts as fallback when HRRR intermediate runs only have 18h

**Storage Impact**:
- Previous: 2 runs × 48 hours × 301 MB/hour = ~28.8 GB
- New: 3 runs × 48 hours × 301 MB/hour = ~43.3 GB
- **Additional: ~14.5 GB**

---

## Total Storage Impact

| Component | Storage |
|-----------|---------|
| Available before changes | 780 GB |
| GFS increase | -25 GB |
| HRRR increase | -24 GB |
| NBM-CONUS increase | -14 GB |
| **Remaining available** | **~717 GB (92% free)** |

---

## Expected Benefits

### For Users (Frontend Application)
1. **Full 48-hour coverage**: Consistent "Today + Tomorrow" forecast data
2. **No grey blocks**: GFS gaps eliminated (was limited to 24h)
3. **Better reliability**: Multiple model runs provide redundancy
4. **Historical context**: 2-3 runs of past data available for visualization

### Coverage by Model
| Model | Coverage Window | Primary Use |
|-------|----------------|-------------|
| **HRRR** | 48h from major runs | High-resolution hourly forecasts |
| **GFS** | 48h continuous | Medium-resolution backup/extended |
| **NBM** | 48h continuous | Statistical blend, most reliable |
| **NDFD** | 7 days (unchanged) | Long-range guidance |

---

## Timeline & Next Steps

### Immediate (After Configuration Change)
1. **Restart services** to load new configuration
2. **Downloader** begins fetching 48h of GFS data (instead of 24h)
3. **Existing data** remains until cleanup runs

### 6-12 Hours
- First complete GFS 48h run downloaded
- HRRR major runs start accumulating
- NBM third run begins retention

### 12-24 Hours
- Full 48h coverage should be available across all models
- Frontend should show complete "Today + Tomorrow" data
- Storage usage increases to new steady state (~63 GB more)

### Monitoring
Monitor these metrics:
- **Storage usage**: Should stabilize at ~220 GB (previous ~157 GB)
- **Collection temporal extents** via `/edr/collections/{id}` endpoints
- **Frontend coverage**: Grey blocks should be eliminated for hours 0-48

---

## Rollback Instructions

If issues arise, revert changes:

### 1. Edit Configuration Files

**GFS** (`config/models/gfs.yaml`):
```yaml
forecast_hours:
  end: 48 → 24
retention:
  hours: 24 → 12
  keep_latest_runs: 3 → 2
```

**HRRR** (`config/models/hrrr.yaml`):
```yaml
retention:
  hours: 12 → 3
  keep_latest_runs: 4 → 3
```

**NBM-CONUS** (`config/models/nbm-conus.yaml`):
```yaml
retention:
  hours: 12 → 6
  keep_latest_runs: 3 → 2
```

### 2. Restart Services
```bash
docker-compose restart downloader wms-api
# or
systemctl restart weather-wms-downloader weather-wms-api
```

### 3. Cleanup (Optional)
To immediately free up storage after rollback:
```bash
# Trigger cleanup to remove data beyond old retention limits
# (or wait for automatic cleanup cycle)
```

---

## Technical Details

### Download Schedule Impact

**GFS**:
- 4 cycles/day (00z, 06z, 12z, 18z)
- Previously: 4 × 25 files/run = 100 files/day
- Now: 4 × 49 files/run = **196 files/day** (+96 files)
- Download time: +4-6 hours spread across day

**HRRR** (no change):
- 24 cycles/day (hourly)
- Major runs (00z, 06z, 12z, 18z): 49 files each
- Intermediate runs: 19 files each
- Total: ~580 files/day (unchanged)

**NBM-CONUS** (no change):
- 24 cycles/day (hourly)
- 48 files per cycle
- Total: 1,152 files/day (unchanged)

### Cleanup Behavior

The WMS-API cleanup service runs hourly and:
1. Deletes data older than `retention.hours`
2. **Always preserves** `keep_latest_runs` most recent complete runs
3. This prevents gaps during download delays or failures

**Example for GFS**:
- If 00z run is still downloading at hour 24
- Cleanup won't delete the 06z/12z/18z runs (protected by `keep_latest_runs: 3`)
- Once 00z completes, cleanup can remove runs beyond 24h if >3 runs exist

---

## Verification

### Check Configuration
```bash
# Verify GFS forecast hours
grep "end:" config/models/gfs.yaml

# Verify retention settings
grep -A 2 "^retention:" config/models/gfs.yaml
grep -A 2 "^retention:" config/models/hrrr.yaml
grep -A 2 "^retention:" config/models/nbm-conus.yaml
```

### Check Coverage via API
```bash
# GFS temporal extent
curl http://localhost:8083/edr/collections/gfs-surface | jq '.extent.temporal'

# HRRR temporal extent
curl http://localhost:8083/edr/collections/hrrr-surface | jq '.extent.temporal'

# NBM temporal extent
curl http://localhost:8083/edr/collections/nbm-conus | jq '.extent.temporal'
```

### Check Storage Usage
```bash
# Total storage in MinIO/S3
df -h /path/to/data

# Per-model breakdown (if using MinIO CLI)
mc du minio/weather-data/grids/gfs/
mc du minio/weather-data/grids/hrrr/
mc du minio/weather-data/grids/nbm/
```

---

## Related Documentation

- **Ingestion Configuration**: `config/ingestion.yaml`
- **Model Configs**: `config/models/`
- **Downloader Service**: `docs/src/services/downloader.md`
- **WMS-API Cleanup**: `services/wms-api/src/cleanup.rs`
- **Storage Structure**: `docs/src/services/ingester.md`

---

## Questions & Support

If you experience issues:

1. **Check logs**:
   - Downloader: Look for "Forecast hour X exceeds available" errors
   - WMS-API: Check cleanup logs for retention policy enforcement

2. **Verify data**:
   - Query EDR endpoints to see actual temporal extent
   - Check MinIO/S3 storage for expected Zarr arrays

3. **Monitor storage**:
   - Ensure 63 GB additional space is actually being used
   - If storage grows beyond expected, check cleanup service

4. **Frontend issues**:
   - Clear browser cache if grey blocks persist
   - Check browser dev tools for EDR API errors
   - Verify timestamp alignment between collections
