# GRIB2 Data Rendering - Fix Implementation Plan

## Current Status

### ✅ FIXED
1. **Grid Definition Parsing (Section 3)** - Grid dimensions now correctly read as (721, 1440)
   - Issue: Byte offsets were wrong for reading Ni/Nj values
   - Fix: Use u16 reads at offsets [25:27] and [29:31] instead of u32 at [4:8] and [8:12]
   - Location: `crates/grib2-parser/src/sections/mod.rs:218-220`

### ❌ STILL BROKEN

1. **Complex Packing Not Implemented**
   - Most GFS data uses Template 5.15 (complex packing with spatial differencing)
   - Current code only supports Template 5.0 (simple packing)
   - Messages 1, 5-12, etc. fail with "Unsupported template" errors
   
2. **Messages 3 and 4 Return All Zeros**
   - These successfully unpack as simple packing
   - But all values are 0.0, indicating reference value or scale factor is zero
   - Likely due to Section 5 parsing still reading from wrong byte offsets
   - Template 5.0 structure: Reference value at bytes [13:17], but data might be in different positions for this file

3. **Section 4 (Product Definition) Not Parsing Correctly**
   - All messages show "surface" level
   - Forecast hours not being read correctly

## Root Cause Analysis

The GFS test file contains multiple message types:
- Most use packing Template 5.15 (PNG or JPEG2000 compression)
- Some use Template 5.0 (simple packing, uncompressed)
- Only simple packing messages (3 & 4) can be unpacked with current code

### GRIB2 Section 5 Structure is Template-Specific
```
Template 5.0 (Simple Packing):
  [0-3]: Length
  [4]: Section number
  [5-6]: Template number (0)
  [7-10]: Number of data points
  [11]: Data type (packing method)
  [12]: Original data type
  [13-16]: Reference value (IEEE float)
  [17-18]: Binary scale factor
  [19-20]: Decimal scale factor
  [21]: Number of bits per value
  [22+]: Template-specific data

Template 5.2, 5.3, 5.15 (Complex Packing):
  [0-3]: Length
  [4]: Section number
  [5-6]: Template number (2, 3, 15, etc.)
  [7-10]: Number of data points
  [11]: Data type
  [12]: Original data type
  [13-16]: Reference value
  [17-18]: Binary scale factor
  [19-20]: Decimal scale factor
  [21]: Number of bits per value
  [22-25]: Group split method (Template 5.2 only)
  [26-29]: Missing value management (Template 5.2 only)
  [30]: Order of spatial differencing
  [31]: Number of octets for each group width
  [32]: Number of octets for each group length
  [33]: Number of octets for maximum group value
  [34+]: Packed data
```

## Implementation Plan

### Phase 1: Quick Win (Get Messages 3 & 4 Working)
**Effort: 30 minutes**

1. Verify Section 5 parsing for Template 5.0
   - Check if reference value and scale factors are being read correctly
   - Debug why all values unpack to 0.0
   - Location: `crates/grib2-parser/src/lib.rs:142-168`

2. Test with inspect_grib2 to see if real data values appear

### Phase 2: Support Simple Packing Fully
**Effort: 1 hour**

1. Implement proper error handling for messages that don't use supported templates
2. Add logging to show which templates are encountered
3. Count how many messages in test file use simple packing vs complex packing

### Phase 3: Implement Complex Packing (Template 5.2)
**Effort: 3-4 hours**

Complex packing requires:
1. Parse Template 5.2-specific structure (group splitting, spatial differencing parameters)
2. Implement second-order spatial differencing reconstruction
3. Handle group widths and lengths
4. Handle min/max group values

This is the most complex part - requires understanding the WMO GRIB2 specification deeply.

### Phase 4: Consider External Library
**Effort: 2-3 hours for integration**

The proper GRIB2 support would require implementing hundreds of templates. Consider:
- **gribberish** crate (pure Rust, GRIB2 support)
- **eccodes-rs** crate (FFI to ECMWF eccodes library, production-grade)

Alternative: Use system eccodes tool to convert GRIB2 to NetCDF/HDF5 during ingestion.

## Recommended Immediate Fix

Since the rendering pipeline is otherwise working, the fastest path to get data rendering is:

1. **For messages with simple packing (Templates 5.0):**
   - Fix Section 5 byte offsets if needed
   - Debug why values are all zeros
   - Should start seeing real temperature/wind data

2. **For messages with complex packing:**
   - Create a skip mechanism that logs but doesn't crash
   - Allows the system to render the simple-packing messages
   - Makes partial data available rather than complete failure

## Testing

After implementing fixes:

```bash
# Test parsing
cargo run --package grib2-parser --example inspect_grib2 -- testdata/gfs_sample.grib2

# Look for:
# ✓ Correct grid dimensions (should be 721x1440)
# ✓ Some messages with actual data values (not all zeros)
# ✓ Proper error messages for unsupported templates
```

## Files to Modify

1. `crates/grib2-parser/src/sections/mod.rs`
   - parse_data_representation() - verify/fix Section 5 byte offsets
   - parse_product_definition() - fix Section 4 byte offsets

2. `crates/grib2-parser/src/lib.rs`
   - unpack_data() - add better error handling

3. `crates/grib2-parser/src/unpacking/mod.rs`
   - unpack_simple() - debug why values are zero

## Success Criteria

✅ At least messages 3 & 4 should show real temperature data instead of zeros
✅ System should handle unsupported packing methods gracefully without crashing
✅ GridDimensions correctly parsed (already done: 721x1440)
✅ WMS requests return properly rendered temperature maps with real data

