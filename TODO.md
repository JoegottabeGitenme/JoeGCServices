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
**Status:** Potential Performance Enhancement  
**Priority:** Medium

Current implementation uses `ncdump` command-line tool for NetCDF parsing, which works but has performance limitations:

**Current Performance:**
- NetCDF parsing: ~4-5 seconds per file using ncdump
- File sizes: 4-54 MB depending on band resolution
- Grid dimensions: 5000x3000 (GOES-16), 10000x6000 (GOES-18)
- Total render time: ~11-12 seconds for 512x512 tile

**Proposed Optimization:**
- Switch from `ncdump` to direct HDF5/NetCDF library access
- Use `netcdf` Rust crate with HDF5 backend
- Read CMI variable data directly into memory without temp files

**Implementation Steps:**
1. Add system dependencies to Dockerfile:
   ```dockerfile
   RUN apt-get update && apt-get install -y libhdf5-dev libnetcdf-dev
   ```

2. Update `crates/netcdf-parser/Cargo.toml`:
   ```toml
   netcdf = "0.9"
   hdf5 = "0.8"
   ```

3. Rewrite `load_netcdf_grid_data()` in `services/wms-api/src/rendering.rs` to use direct NetCDF API instead of ncdump subprocess

**Expected Benefits:**
- 5-10x faster NetCDF parsing (estimated 0.5-1 second instead of 4-5 seconds)
- No temp file creation/cleanup overhead
- Reduced memory usage
- Better error handling and metadata extraction
- Native handling of fill values and compression

**Trade-offs:**
- Adds system library dependencies (libhdf5, libnetcdf)
- Slightly more complex build process
- More code to maintain vs. simple ncdump wrapper

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

