
## WMS/WMTS Capabilities Improvements

### Convert Forecast Hour Dimension to ISO8601 Duration Format

**Current State:**
- Forecast hours are expressed as simple integers (e.g., `0,1,2,3,6,12,24`)
- Dimension units are "hours"
- Example: `<Dimension name="FORECAST" units="hours" default="0">0,3,6,12,24</Dimension>`

**Target State:**
- Should use ISO8601 duration format for better standards compliance
- Example: `<Dimension name="FORECAST" units="ISO8601" default="PT0H">PT0H,PT3H,PT6H,PT12H,PT24H</Dimension>`
- Or use relative time: `<Dimension name="time" units="ISO8601">2025-11-26T00:00:00Z/PT1H/2025-11-26T12:00:00Z</Dimension>`

**Benefits:**
- Better WMS/WMTS standards compliance
- More flexible for non-hourly forecasts (e.g., 30-minute updates)
- Clearer for international clients
- Consistent with RUN dimension which already uses ISO8601

**Files to Modify:**
- `services/wms-api/src/handlers.rs` - WMS GetCapabilities XML generation
- May need to update WMTS capabilities as well

**References:**
- OGC WMS 1.3.0 Standard (ISO 19128)
- ISO 8601 Duration format: https://en.wikipedia.org/wiki/ISO_8601#Durations

