# Selective GRIB Download via Index Files

## Overview

This document describes the implementation of selective GRIB downloading using `.idx` index files to extract only the parameters we need via HTTP Range requests. This optimization reduces download volume by ~80-90% for supported data sources.

## Status

| Phase | Status | Notes |
|-------|--------|-------|
| Research | Complete | See findings below |
| Design | Complete | This document |
| Implementation | Complete | 2026-01-16 |
| Testing | Partial | Unit tests complete, integration pending |
| Deployment | Pending | |

## Background

### Problem

Currently, the downloader fetches entire GRIB2 files from NOAA data sources. These files contain hundreds of meteorological parameters, but we only ingest ~20-30 parameters per model. This wastes bandwidth and storage:

- GFS files: ~500 MB each, we use ~10%
- HRRR files: ~400 MB each, we use ~10%

### Solution

NOAA provides `.idx` index files alongside GRIB files that list the byte offset of each parameter. We can use these to make HTTP Range requests and download only the GRIB messages we need.

## Research Findings

### Data Sources with .idx Files

| Data Source | .idx Available | Full File Size | Potential Savings |
|-------------|---------------|----------------|-------------------|
| **GFS (0.25deg)** | Yes | ~500 MB | ~90% |
| **HRRR (3km)** | Yes | ~135-400 MB | ~90% |
| **NBM (all regions)** | Yes | ~100-200 MB | ~80% |
| **MRMS** | No | ~1-5 MB (gzipped) | N/A - single product |
| **NDFD** | No | ~5-20 MB | N/A - per-parameter files |
| **GOES** | No | NetCDF format | N/A - different format |

### Index File Format

The `.idx` files use a simple colon-delimited text format:

```
line_number:byte_offset:d=YYYYMMDDHH:PARAMETER:level:type:
```

Example entries:
```
1:0:d=2026011500:PRMSL:mean sea level:anl:
580:407331265:d=2026011500:TMP:2 m above ground:anl:
585:411277095:d=2026011500:UGRD:10 m above ground:anl:
586:412239265:d=2026011500:VGRD:10 m above ground:anl:
```

Fields:
- `line_number`: Sequential message number (1-based)
- `byte_offset`: Start byte of GRIB message in file
- `d=YYYYMMDDHH`: Reference date/time
- `PARAMETER`: Variable short name (TMP, UGRD, etc.)
- `level`: Level description string
- `type`: Analysis (anl) or forecast (fcst)

### Byte Range Calculation

To extract a specific parameter:
- **Start byte**: `byte_offset` from the entry
- **End byte**: `byte_offset` from next entry - 1 (or file size - 1 for last entry)

### Level String Mapping

The `.idx` level strings differ from our YAML config level codes:

| YAML Config | .idx Level String |
|-------------|-------------------|
| `level_code: 103, value: 2` | "2 m above ground" |
| `level_code: 103, value: 10` | "10 m above ground" |
| `level_code: 100, value: 85000` | "850 mb" |
| `level_code: 100, value: 50000` | "500 mb" |
| `level_code: 101` | "mean sea level" |
| `level_code: 1` | "surface" |
| `level_code: 200` | "entire atmosphere" |
| `level_code: 214` | "low cloud layer" |
| `level_code: 224` | "middle cloud layer" |
| `level_code: 234` | "high cloud layer" |

## Design

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       ModelRunner                                │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ 1. Check if use_index_file enabled                      │    │
│  │ 2. Build parameter filter from model config             │    │
│  │ 3. Call download_selective() or download()              │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DownloadManager                              │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ download_selective()                                     │    │
│  │  1. Fetch .idx file                                     │    │
│  │  2. Parse index → GribIndex                             │    │
│  │  3. Match parameters → byte ranges                      │    │
│  │  4. Merge adjacent ranges                               │    │
│  │  5. Download byte ranges sequentially                   │    │
│  │  6. Concatenate to output file                          │    │
│  │  7. On any error → fallback to full download            │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                       GribIndex                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ - parse(content: &str) → Result<GribIndex>              │    │
│  │ - find_entries(param, level) → Vec<IndexEntry>          │    │
│  │ - calculate_byte_ranges() → Vec<(start, end)>           │    │
│  │ - merge_adjacent_ranges() → Vec<(start, end)>           │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

### Configuration Changes

New fields in `SourceConfig`:

```yaml
source:
  type: aws_s3
  bucket: noaa-gfs-bdp-pds
  # ... existing fields ...
  
  # Enable selective download (default: false)
  use_index_file: true
  
  # Index file suffix (default: ".idx")
  index_suffix: ".idx"
```

### Fallback Behavior

The selective download gracefully falls back to full download when:

1. Index file fetch fails (404, timeout, etc.)
2. Index file parsing fails (format changed, corrupt)
3. No matching parameters found in index
4. Any byte range download fails

All fallback events are logged as warnings for monitoring.

### Partial Match Handling

When some but not all requested parameters are found:
- Download the available parameters
- Log a warning listing the missing parameters
- Continue with ingestion of available data

## Implementation

### New Files

| File | Purpose |
|------|---------|
| `services/downloader/src/grib_index.rs` | Index parser and byte range calculator (~350 lines) |

### Modified Files

| File | Changes |
|------|---------|
| `services/downloader/src/main.rs` | Added `mod grib_index` |
| `services/downloader/src/config.rs` | Added `use_index_file`, `index_suffix` fields, `LevelConfig` struct, `build_param_filters()` method |
| `services/downloader/src/download.rs` | Added `download_selective()`, `fetch_index_file()`, `download_byte_ranges()` methods, `SelectiveDownloadResult` enum |
| `services/downloader/src/model_runner.rs` | Integrated selective download in `download_files()` with fallback logic |
| `config/models/gfs.yaml` | Added `use_index_file: true` |
| `config/models/hrrr.yaml` | Added `use_index_file: true` |
| `config/models/nbm-conus.yaml` | Added `use_index_file: true` |
| `config/models/nbm-alaska.yaml` | Added `use_index_file: true` |
| `config/models/nbm-hawaii.yaml` | Added `use_index_file: true` |
| `config/models/nbm-puertorico.yaml` | Added `use_index_file: true` |
| `config/models/nbm-guam.yaml` | Added `use_index_file: true` |

### Key Types

```rust
/// Single entry from .idx file
pub struct IndexEntry {
    pub message_number: u32,
    pub byte_offset: u64,
    pub date: String,
    pub parameter: String,
    pub level: String,
    pub forecast_type: String,
}

/// Parsed index file
pub struct GribIndex {
    entries: Vec<IndexEntry>,
    file_size: Option<u64>,
}

/// Parameter filter for matching
pub struct ParamFilter {
    pub parameter: String,
    pub level: String,
}
```

### HTTP Range Request Strategy

Using sequential single-range requests because:
1. AWS S3 does not support multi-range requests
2. Simpler error handling per range
3. Progress tracking per parameter

Request format:
```
GET /path/to/file.grib2
Range: bytes=407331265-409395881
```

## Estimated Savings

### Per-File Savings

| Model | Full Size | Selective (~30 params) | Savings |
|-------|-----------|------------------------|---------|
| GFS | ~500 MB | ~50 MB | ~90% |
| HRRR | ~400 MB | ~40 MB | ~90% |
| NBM | ~150 MB | ~30 MB | ~80% |

### Daily Bandwidth Savings

| Model | Calculation | Daily Savings |
|-------|-------------|---------------|
| GFS | 4 cycles x 25 files x 450 MB | ~45 GB |
| HRRR | 24 cycles x 48 files x 360 MB | ~415 GB |
| NBM (all regions) | 24 cycles x 48 files x ~100 MB | ~115 GB |
| **Total** | | **~575 GB/day** |

## Testing Plan

### Unit Tests (Complete - 14 tests)

- [x] Parse valid .idx content (`test_parse_index`)
- [x] Handle malformed lines gracefully (`test_parse_entry_invalid`)
- [x] Byte range calculation with single entry (`test_get_byte_ranges`)
- [x] Byte range calculation with multiple entries (`test_find_matching_entries`)
- [x] Range merging with adjacent ranges (`test_merge_adjacent_ranges`)
- [x] Range merging with gaps (`test_merge_with_gap_threshold`)
- [x] Parameter matching exact (`test_param_filter_matches`)
- [x] Level string conversion (`test_level_to_idx_string`)
- [x] Byte range size calculation (`test_byte_range_size`)
- [x] HTTP Range header formatting (`test_http_range_format`)
- [x] Last message uses file size (`test_last_message_uses_file_size`)
- [x] Parameter list extraction (`test_parameters`)
- [x] Level list extraction (`test_levels_for_parameter`)
- [x] Entry parsing (`test_parse_entry`)

### Integration Tests (Pending)

- [ ] Mock HTTP server returning .idx + partial GRIB
- [ ] Verify correct byte ranges requested
- [ ] Fallback on index fetch failure
- [ ] Fallback on parse failure
- [ ] Partial match warning logged

### Manual Validation (Pending)

- [ ] Compare ingested data: selective vs full download
- [ ] Verify all expected parameters present
- [ ] Check no data corruption

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| .idx format changes | Download fails | Fallback to full download |
| .idx file unavailable | Cannot use selective | Fallback to full download |
| Incomplete parameter coverage | Missing data | Log warning, ingest available |
| Concatenated GRIB invalid | Ingestion fails | Each message is self-contained; tested |

## Future Enhancements

1. **Parallel range downloads**: Download multiple ranges concurrently
2. **Caching**: Cache .idx files for repeated access
3. **Metrics**: Track selective vs full download ratio
4. **Smart merging**: Merge ranges if gap is small (save HTTP overhead)

## References

- [NOAA GFS on AWS](https://registry.opendata.aws/noaa-gfs-bdp-pds/)
- [NOAA HRRR on AWS](https://registry.opendata.aws/noaa-hrrr-pds/)
- [HTTP Range Requests (RFC 7233)](https://tools.ietf.org/html/rfc7233)
