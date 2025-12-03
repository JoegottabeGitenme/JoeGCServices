## GOES Satellite Imagery Enhancements

### Future Improvements for GOES Data

The GOES satellite pipeline is fully operational with automatic downloads, ingestion, and WMS/WMTS rendering. The following enhancements would improve functionality and performance:

#### 1. Add Additional GOES Bands
**Status:** Potential Enhancement  
**Priority:** Medium

Currently downloading and serving bands 1, 2, 8, 13 (Blue, Red, Water Vapor, IR). Additional bands available:

**Visible/Near-IR Bands:**
- Band 3 (0.86µm) - "Veggie" band for vegetation monitoring
- Band 4 (1.37µm) - Cirrus cloud detection
- Band 5 (1.6µm) - Snow/ice discrimination
- Band 6 (2.2µm) - Cloud particle size

**IR Bands:**
- Band 7 (3.9µm) - Shortwave window
- Band 9 (6.9µm) - Mid-level water vapor
- Band 10 (7.3µm) - Lower-level water vapor
- Band 11 (8.4µm) - Cloud-top phase
- Band 12 (9.6µm) - Ozone
- Band 14 (11.2µm) - "Longwave" IR (standard IR)
- Band 15 (12.3µm) - "Dirty" longwave IR
- Band 16 (13.3µm) - CO2 absorption

**Implementation:**
- Update `config/models/goes16.yaml` and `goes18.yaml` to include additional bands in `bands:` array
- Add corresponding parameter definitions with appropriate styles
- Create additional style JSON files if needed for specialized rendering

**Benefits:**
- More comprehensive satellite imagery coverage
- Better atmospheric analysis capabilities
- Enhanced cloud and precipitation monitoring

#### 2. Optimize NetCDF Data Extraction
**Status:** ✅ COMPLETED (December 2025)  
**Priority:** ~~Medium~~ DONE

~~Current~~ **Previous** implementation used `ncdump` command-line tool for NetCDF parsing, which worked but had performance limitations.

**Performance Improvement Achieved:**
- **Before:** ~300ms per file using ncdump subprocess + ASCII parsing
- **After:** ~25ms per file using native netcdf Rust library
- **Speedup:** ~12x faster! 🚀

**Implementation Completed:**
1. ✅ Added system dependencies to Dockerfile (libhdf5-dev, libnetcdf-dev)
2. ✅ Updated `crates/netcdf-parser/Cargo.toml` to use `netcdf = "0.11"` with HDF5 1.14 support
3. ✅ Created `load_goes_netcdf_from_bytes()` in `crates/netcdf-parser/src/lib.rs`
4. ✅ Updated `services/wms-api/src/rendering.rs` to use native NetCDF parsing
5. ✅ Removed ncdump subprocess calls and ASCII parsing code

**Benefits Realized:**
- ✅ 12x faster NetCDF parsing (~25ms vs ~300ms)
- ✅ Better error handling with Rust Result types
- ✅ Type-safe attribute and dimension reading
- ✅ Native handling of fill values and scale/offset transformations
- ✅ No subprocess overhead or temp file cleanup race conditions

**Files Modified:**
- `crates/netcdf-parser/Cargo.toml`
- `crates/netcdf-parser/src/lib.rs`
- `services/wms-api/src/rendering.rs`
- `services/wms-api/Dockerfile`

**Notes:**
- Still requires temp file for netcdf crate API (library limitation)
- Main speedup comes from eliminating subprocess overhead and direct binary data reading
- Consider removing netcdf-bin from runtime dependencies (no longer needed)

#### 3. Add RGB Composite Products
**Status:** Potential Feature Enhancement  
**Priority:** Low-Medium

Create derived products that combine multiple GOES bands into useful visualizations:

**True Color RGB:**
- Combines bands 1, 2, 3 (Blue, Red, Veggie) to simulate natural color
- Most intuitive for general users
- Excellent for cloud/surface feature identification

**Day Cloud Phase RGB:**
- Combines bands 13, 2, 5 (IR, Red, Snow/Ice)
- Distinguishes between ice/water clouds and snow/ice on surface
- Useful for aviation and winter weather

**Air Mass RGB:**
- Combines bands 8, 10, 13 (Upper WV, Lower WV, Clean IR)
- Shows different air mass characteristics
- Useful for severe weather forecasting

**Nighttime Microphysics RGB:**
- Combines bands 15, 13, 7 (Dirty IR, Clean IR, Shortwave Window)
- Works in darkness
- Distinguishes fog/low clouds from higher clouds

**Implementation:**
- Create new layer types in model config (e.g., `goes18_TRUE_COLOR`)
- Add composite rendering functions in `services/wms-api/src/rendering.rs`
- Load multiple bands simultaneously and combine according to RGB recipe
- Add normalization and gamma correction as needed
- Create new style configurations for composites

**Benefits:**
- More intuitive and useful imagery for end users
- Standard products used by meteorologists worldwide
- Better cloud type discrimination
- Day/night coverage with different products

**Considerations:**
- Requires downloading/storing 3x as many bands per composite
- More complex rendering logic
- Need to handle cases where not all bands are available
- Increased processing time per tile

---

## Load Testing Improvements

### 1. Handle Fewer Available Timesteps Gracefully
**Status:** ⚠️ PARTIALLY ADDRESSED (December 2025)  
**Priority:** Medium

**Issue:**
Load test scenarios specify a desired number of timesteps (e.g., `count: 5`), but tests fail or behave unexpectedly when fewer timesteps are currently ingested in the system.

**Current Implementation:**
- ✅ Added warning messages in `validation/load-test/src/generator.rs` when fewer times are available than requested
- ✅ Tests now proceed with available times instead of failing
- ⚠️ **TODO:** Warning currently goes to stderr, which may not be visible in all test runners
- ⚠️ **TODO:** No clear indication in test results summary that fewer times were used
- ⚠️ **TODO:** Cache hit rates and performance metrics may be misleading with fewer times

**Remaining Work:**
1. **Improve visibility of time mismatch:**
   - Add timestep count to test result summary output
   - Include "Expected X times, found Y times" in the final report
   - Consider making it a test warning in the results table

2. **Document minimum data requirements:**
   - Add metadata to scenario files indicating minimum required timesteps
   - Show warning if running scenario with insufficient data
   - Provide guidance on ingesting more data

3. **Adjust test interpretation:**
   - Cache hit rate expectations change dramatically with fewer timesteps
   - Document expected behavior: 2 timesteps = ~50% cache hit, 5 timesteps = ~20% hit (for random access)
   - Consider adding a "confidence level" to results when data is insufficient

**Example Scenario Issue:**
```yaml
# Scenario expects 5 timesteps for realistic cache miss testing
time_selection:
  type: query_random
  layer: goes18_CMI_C13
  count: 5  # ← Only 2 actually available!
```

With 2 timesteps instead of 5, random access patterns won't exercise cache properly, and performance characteristics will be very different from production scenarios.

**Benefits:**
- More predictable test behavior
- Clear warnings when test conditions aren't ideal
- Better understanding of test validity
- Prevents misleading performance conclusions

---

### 2. Fix Remaining Hardcoded Timestamps in Load Test Scenarios  
**Status:** ✅ COMPLETED (December 2025)  
**Priority:** ~~Low~~ DONE

**Completed:**
- ✅ Three GOES scenarios updated to use dynamic time queries from WMS GetCapabilities
- ✅ WMS client XML parsing implemented and working (`validation/load-test/src/wms_client.rs`)
- ✅ `query_sequential` and `query_random` time selection modes implemented

**Implementation:**
```yaml
time_selection:
  type: query_sequential  # or query_random
  layer: layer_name
  count: 5
  order: newest_first  # or oldest_first
```

**Remaining Work:**
- Update MRMS scenarios to use dynamic queries (if MRMS supports temporal queries)
- Verify all temporal scenarios use dynamic time selection

**Benefits Realized:**
- ✅ Scenarios automatically use current available data
- ✅ No need to manually update timestamps as data ages
- ✅ Tests remain valid as new data is ingested
- ✅ Easier to run tests on any environment without modification

---

## WMS/WMTS Capabilities Improvements

### ✅ Convert Forecast Hour Dimension to ISO8601 Duration Format (COMPLETED - Nov 28, 2024)

**Implementation:**
- Converted FORECAST dimension from simple integers to ISO8601 duration format
- Updated both WMS and WMTS GetCapabilities XML generation
- Changed units from "hours" to "ISO8601"
- Forecast values now use PT#H format (e.g., PT0H, PT3H, PT6H, PT12H, PT24H)

**Examples:**
- **GFS**: `<Dimension name="FORECAST" units="ISO8601" default="PT0H">PT0H,PT3H,PT6H,PT12H,PT24H</Dimension>`
- **HRRR**: `<Dimension name="FORECAST" units="ISO8601" default="PT0H">PT0H,PT1H,PT2H,PT3H,PT6H,PT12H</Dimension>`
- **WMTS**: `<ows:UOM>ISO8601</ows:UOM>` with `<Value>PT0H</Value>` format

**Benefits Achieved:**
- ✅ Better WMS/WMTS standards compliance (ISO 19128)
- ✅ Consistent with RUN dimension which already uses ISO8601
- ✅ More flexible for non-hourly forecasts (HRRR uses PT1H, PT2H)
- ✅ Clearer for international clients
- ✅ Proper semantic representation of time durations

**Files Modified:**
- `services/wms-api/src/handlers.rs` - WMS and WMTS GetCapabilities XML generation (lines 1493-1503, 1736-1766, 1783-1820)

