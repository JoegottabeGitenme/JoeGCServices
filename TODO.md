
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

