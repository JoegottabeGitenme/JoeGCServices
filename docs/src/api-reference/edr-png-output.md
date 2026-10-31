# EDR PNG Output

The EDR API supports PNG output for area queries, designed for GPU shader consumption in WebGL-based weather visualization applications (similar to Windy.com).

## Overview

PNG output encodes weather data as image textures that can be uploaded directly to the GPU for real-time rendering. This approach offers:

- **Client-side rendering flexibility**: Change colormaps, thresholds, and blending without re-fetching data
- **GPU-ready format**: Data goes directly to WebGL textures
- **Reduced HTTP requests**: One large PNG vs many WMS tiles
- **Full data precision**: 16-bit encoding provides 65,536 distinct values

## Basic Usage

Request PNG output by setting `f=png`:

```bash
curl "http://localhost:8083/edr/collections/hrrr-surface/area?\
coords=POLYGON((-100 35,-98 35,-98 37,-100 37,-100 35))&\
parameter-name=TMP&\
f=png" -o temperature.png
```

## Query Parameters

| Parameter | Required | Description | Example |
|-----------|----------|-------------|---------|
| `coords` | Yes | WKT POLYGON defining the area | `POLYGON((-100 35,...))` |
| `parameter-name` | Yes | Single parameter to retrieve | `TMP` |
| `f` | Yes | Must be `png` | `png` |
| `width` | No | Output width in pixels (max 4096) | `512` |
| `height` | No | Output height in pixels (max 4096) | `512` |
| `depth` | No | Bit depth: `8` or `16` (default) | `8` |
| `crop` | No | `true` cuts a projected grid (HRRR) to the polygon's box; see [Cropping a projected grid](#cropping-a-projected-grid-croptrue) | `true` |
| `datetime` | No | Time instant | `2024-12-29T12:00:00Z` |
| `z` | No | Vertical level | `850` |

> **Note**: PNG output requires exactly one parameter. Use `parameter-name` to select a single parameter.

## Encoding Formats

### 16-bit Mode (Default)

Uses RGBA channels to encode 16-bit precision:

| Channel | Purpose |
|---------|---------|
| R | High byte of 16-bit normalized value |
| G | Low byte of 16-bit normalized value |
| B | Reserved (0) |
| A | Validity mask (255=valid, 0=no data) |

**Normalization formula:**
```
normalized = (value - min) / (max - min)
uint16 = normalized * 65535
R = uint16 >> 8    (high byte)
G = uint16 & 0xFF  (low byte)
```

### 8-bit Mode

Uses Grayscale+Alpha for smaller files (~50% size reduction):

| Channel | Purpose |
|---------|---------|
| Gray | Normalized value (0-255) |
| Alpha | Validity mask (255=valid, 0=no data) |

Request 8-bit mode with `depth=8`:

```bash
curl "...&f=png&depth=8" -o temperature_8bit.png
```

## Response Headers

PNG responses include metadata headers for client-side decoding:

| Header | Description | Example |
|--------|-------------|---------|
| `X-EDR-Parameter` | Parameter name | `TMP` |
| `X-EDR-Units` | Unit symbol | `K` |
| `X-EDR-Min` | Minimum value (for denormalization) | `250.5` |
| `X-EDR-Max` | Maximum value (for denormalization) | `310.2` |
| `X-EDR-Encoding` | Encoding type | `uint16` or `uint8` |
| `X-EDR-BBox` | Bounding box (west,south,east,north) | `-100,35,-98,37` |
| `X-EDR-Width` | Image width in pixels | `256` |
| `X-EDR-Height` | Image height in pixels | `341` |
| `Cache-Control` | Cache policy | `max-age=3600` |

## GLSL Shader Decoding

### 16-bit Decoding

```glsl
uniform sampler2D uDataTexture;
uniform float uMinValue;
uniform float uMaxValue;

void main() {
    vec4 texel = texture2D(uDataTexture, vTexCoord);
    
    // Check validity
    if (texel.a < 0.5) {
        discard;  // No data at this pixel
    }
    
    // Decode 16-bit value from R and G channels
    float encoded = texel.r * 255.0 * 256.0 + texel.g * 255.0;
    float normalized = encoded / 65535.0;
    
    // Convert to physical value
    float value = normalized * (uMaxValue - uMinValue) + uMinValue;
    
    // Apply colormap...
}
```

### 8-bit Decoding

```glsl
uniform sampler2D uDataTexture;
uniform float uMinValue;
uniform float uMaxValue;

void main() {
    vec4 texel = texture2D(uDataTexture, vTexCoord);
    
    // Check validity
    if (texel.a < 0.5) {
        discard;
    }
    
    // Grayscale is already normalized 0-1
    float normalized = texel.r;
    
    // Convert to physical value
    float value = normalized * (uMaxValue - uMinValue) + uMinValue;
    
    // Apply colormap...
}
```

## Area Limits

PNG queries support larger area limits than JSON queries, since they're designed for regional/continental coverage:

| Model | JSON Area Limit | PNG Area Limit |
|-------|-----------------|----------------|
| HRRR | 100 sq degrees | 2,500 sq degrees |
| GFS | 200 sq degrees | 10,000 sq degrees |
| MRMS | 25 sq degrees | (default) |

Configure limits in the model's EDR config:

```yaml
# config/edr/hrrr.yaml
limits:
  max_area_sq_degrees: 100          # JSON queries
  max_area_sq_degrees_png: 2500     # PNG queries - full CONUS
```

## Resizing

Request specific output dimensions with `width` and `height`:

```bash
# Resample to 512x512 using nearest-neighbor interpolation
curl "...&f=png&width=512&height=512" -o resized.png
```

- Maximum dimension: 4096 pixels
- Both width and height must be specified together
- Uses nearest-neighbor resampling (preserves discrete values)

### Overview levels

Every grid is stored with a pyramid of coarser overview levels (each half the size of the one
before). A projected grid cut with `crop=true` (below) is read from the coarsest level that still
has a cell per pixel across the polygon, which is a fraction of the data: a polygon 600 cells
wide into an image 120 pixels wide comes from a level with a quarter of the cells across. HRRR
composite reflectivity (`REFC`) is downsampled by **maximum**, so a small storm core survives;
every other continuous parameter by mean, HRRR's precipitation rate (`PRATE`) included, so a
convective core of it reads lower from a level than it is in the grid. (MRMS reflectivity,
precipitation rate and hail size are downsampled by maximum too, but MRMS is a lat/lon grid and is
never read from a level, see below.) A small `width` and `height` on a large polygon is therefore
cheaper than the same polygon at native size.

Parameters whose values are category codes or on/off flags are **never** read from an overview
level, whatever the size asked for: MRMS `PRECIP_FLAG` and HRRR's `CRAIN`, `CSNOW`, `CFRZR` and
`CICEP`. An overview of a code is another cell's code at best and a fraction that is no code at
worst (HRRR flags ingested before they were marked categorical hold fractions in their overview
levels), so those are always read at native resolution and sampled nearest-cell (see the
[position](./edr.md) query).

Nothing else is read from an overview level. A lat/lon grid (MRMS, GFS) is always read at native
resolution, whatever `width` and `height`, so every parameter of a dataset answers a polygon on the
same grid with the same `X-EDR-BBox`, and a client can lay one over another. A projected grid read
without `crop` is returned whole whatever the polygon, so the polygon says nothing about how much
of it the image needs.

## Bandwidth Comparison

### EDR PNG vs WMS Tiles

| Aspect | WMS Tiles | EDR PNG |
|--------|-----------|---------|
| **File size per pixel** | Smaller (indexed color) | Larger (16-bit data) |
| **HTTP requests** | Many (6-100+ for CONUS) | One |
| **Data precision** | 8-bit (256 colors) | 16-bit (65,536 values) |
| **Client-side flexibility** | None - server renders | Full - client renders |
| **Re-fetch for colormap** | Yes | No |
| **GPU-ready** | Requires decode | Direct texture upload |

### Size Examples

For a 10x10 degree area at HRRR resolution (~251x341 pixels):

| Format | Size | Notes |
|--------|------|-------|
| EDR PNG (16-bit) | ~146 KB | RGBA, 16-bit encoded |
| EDR PNG (8-bit) | ~80 KB | Grayscale+Alpha |
| WMS PNG (256x256) | ~30 KB | 8-bit indexed color |
| WMS PNG (512x512) | ~106 KB | 8-bit indexed color |

### When to Use Each

**Use EDR PNG when:**
- Building WebGL/GPU-based visualizations
- Need client-side colormap changes
- Want to minimize HTTP requests
- Need precise data values (16-bit mode)

**Use WMS tiles when:**
- Bandwidth is constrained (mobile)
- Need standard web map integration
- Server-side rendering is acceptable
- Using traditional mapping libraries (Leaflet, OpenLayers)

## Full CONUS Example

Request full CONUS coverage with 8-bit encoding for optimal size:

```bash
# CONUS bounds: approximately -125 to -66 longitude, 24 to 50 latitude
curl "http://localhost:8083/edr/collections/hrrr-surface/area?\
coords=POLYGON((-125 24,-66 24,-66 50,-125 50,-125 24))&\
parameter-name=TMP&\
f=png&\
depth=8&\
width=1024&\
height=512" -o conus_temp.png
```

## PNG Metadata

Encoding parameters are also embedded in PNG tEXt chunks for self-describing files:

| Chunk Key | Description |
|-----------|-------------|
| `EDR:parameter` | Parameter name |
| `EDR:units` | Unit symbol |
| `EDR:min` | Minimum value |
| `EDR:max` | Maximum value |
| `EDR:bbox` | Bounding box |
| `EDR:encoding` | `uint8` or `uint16` |
| `EDR:width` | Image width |
| `EDR:height` | Image height |

Read metadata with standard PNG tools:

```bash
# Using pnginfo (from libpng)
pnginfo temperature.png

# Using Python
python scripts/decode_edr_png.py temperature.png
```

## Cache Policy

PNG responses include cache headers based on model update frequency:

| Model | Cache max-age | Reason |
|-------|---------------|--------|
| HRRR | 3600s (1 hour) | Updates hourly |
| GFS | 21600s (6 hours) | Updates every 6 hours |
| MRMS | 120s (2 minutes) | Updates every 2 minutes |

These values apply to a PNG for an explicit time (a `datetime` that has ended,
or an instance). A PNG of the latest data (no time given, for example
`mrms-single-level/area` with no `datetime`, or an interval still open at the
end) is capped at 60 seconds whatever the model is configured with, because a
newer scan may replace it at any time. The area endpoint ignores `run` (it
always renders the newest run), so `run` does not make a PNG explicit.

A `datetime` is answered with the dataset **nearest** to it, so naming a time does not by itself
make the PNG that time's. The explicit lifetime is given only when the dataset the server found is
valid at the instant asked for (within one second). An instant the server has no dataset for (a
scan that has expired or not arrived yet, a time in the future, a time between two model steps) is
answered with a neighbouring one and gets the 60 second cap: it changes when the instant's own data
arrives.

`/collections/{id}/instances/{run}/area` reads that run: the dataset of the run valid at the
`datetime` (the run's first forecast when none is given). For a `datetime` list or interval it is
the first of those times that the run holds: an interval that starts before the run's first step
(or is open at the start) is answered with that step, whatever other runs hold. A time the run does
not hold, or a list or interval with none of the run's times in it, is a `404`, never another
run's data, and the answer keeps the explicit lifetime because a run's step does not change.

Configure cache policy per model:

```yaml
# config/edr/hrrr.yaml
settings:
  cache_policy:
    png_max_age: 3600
```

## Error Responses

| Error | Cause | Solution |
|-------|-------|----------|
| 400 Multiple parameters | PNG requires exactly one parameter | Use `parameter-name=X` |
| 400 Invalid depth | depth must be 8 or 16 | Use `depth=8` or `depth=16` |
| 400 Invalid crop | `crop` must be `true` or `false`, and is only for `f=png` | Use `crop=true` |
| 400 No data available | The polygon is outside the collection's geographic bounds (with `crop=true`: outside the model domain) | Use a polygon over the data |
| 404 Not found | An instance that does not exist, or a `datetime` with no time in it that the instance holds | Use a time listed for the instance |
| 400 Dimensions too large | width or height > 4096 | Reduce dimensions |
| 413 Area too large | Exceeds `max_area_sq_degrees_png` | Reduce polygon size |

## Best Practices for Georeferencing

The `X-EDR-BBox` header provides the geographic extent of the returned PNG, which may differ slightly from your request due to grid snapping.

### Understanding X-EDR-BBox

The bbox represents the **outer edges of the pixel grid**, not pixel centers:

```
X-EDR-BBox: west,south,east,north
            │     │     │     │
            │     │     │     └── North edge of top row
            │     │     └──────── East edge of rightmost column
            │     └────────────── South edge of bottom row
            └──────────────────── West edge of leftmost column
```

### Georeferencing Recommendations

1. **Don't specify width/height** unless you need a specific size
   - The server returns native grid resolution by default
   - Specifying dimensions triggers resampling which can affect alignment

2. **Use X-EDR-BBox for placement**, not your request bbox
   - The returned bbox is snapped to actual grid cell boundaries
   - Your request may be expanded to include full grid cells

3. **Account for half-pixel offset** when mapping textures
   ```javascript
   // First pixel CENTER is 0.5 cells inward from bbox corner
   const pixelWidth = (bbox.east - bbox.west) / imageWidth;
   const pixelHeight = (bbox.north - bbox.south) / imageHeight;
   const firstPixelCenterLon = bbox.west + pixelWidth * 0.5;
   const firstPixelCenterLat = bbox.north - pixelHeight * 0.5;
   ```

4. **Match aspect ratio** if specifying dimensions
   ```javascript
   // Calculate dimensions that match the geographic aspect ratio
   const aspectRatio = (east - west) / (north - south);
   const height = Math.round(desiredWidth / aspectRatio);
   ```

### Example: WebGL Texture Mapping

```javascript
// Fetch PNG and extract headers
const response = await fetch(edrUrl);
const bbox = response.headers.get('X-EDR-BBox').split(',').map(Number);
const [west, south, east, north] = bbox;

// Calculate texture coordinates for a geographic point
function geoToTexCoord(lon, lat) {
  const u = (lon - west) / (east - west);
  const v = (north - lat) / (north - south);  // Y is inverted for textures
  return [u, v];
}
```

### Projected Grids (HRRR, NDFD, NBM)

For non-geographic grids, the server reprojects data to CRS:84 (WGS84). Request in geographic coordinates:

```bash
# HRRR area request - use lat/lon coordinates
curl ".../edr/collections/hrrr-surface/area?\
coords=POLYGON((-100 35,-98 35,-98 37,-100 37,-100 35))&\
parameter-name=TMP&f=png"
```

The reprojection uses bilinear interpolation for smooth results.

**Without `crop`, a projected grid answers with the whole model domain, whatever polygon is sent.**
The polygon only has to be valid (and under the area limit). `X-EDR-BBox` is the extent of the
whole domain (for HRRR about -134.1, 21.1, -60.9, 52.6) and `width`/`height` squeeze that into the
image. Clients that want the same URL for every request, and the whole domain every time (the
app's wind, pressure and heat layers), rely on this and keep working unchanged.

### Cropping a projected grid (`crop=true`)

`crop=true` asks a projected grid for **the polygon's own bounding box**:

```bash
# A 2.5 x 1.5 degree trail view of HRRR: 250 x 150 pixels instead of the whole domain
curl ".../edr/collections/hrrr-subh-atmosphere/area?\
coords=POLYGON((-106 39,-103.5 39,-103.5 40.5,-106 40.5,-106 39))&\
parameter-name=REFC&f=png&width=250&height=150&crop=true"
```

- The image covers exactly the polygon's bounding box, at `width` x `height` (when they are
  omitted, at the grid's own cell spacing: about 3 km per pixel for HRRR). For a `MULTIPOLYGON`
  that is the box around all of its polygons, and the pixels outside every polygon are no-data
  (alpha 0), whatever the shape of the first polygon.
- `X-EDR-BBox` is that bounding box (the outer pixel edges, as always), so a client places the
  image from the header and nothing else.
- Pixels outside the model domain are no-data (alpha 0), so a box over the edge of the domain is
  fine. A box entirely outside it is the usual `400` no-data answer.
- Each pixel is sampled from the grid with the same bilinear interpolation as the whole-domain
  image, so the two agree wherever they overlap, as long as the image is not much smaller than
  the polygon. When it is, the pixels come from an overview level (see
  [Overview levels](#overview-levels)): bilinear samples of a level whose cells are the maximum
  (`REFC`) or the mean (everything else) of the cells they cover, so a reflectivity core can
  read higher, and a precipitation-rate core lower, than it does in the whole-domain image.
- Only the chunks of the grid that hold the box are read (one or two of HRRR's twelve for a trail
  view, against all twelve), and a large box into a small image comes from an overview level
  (see [Overview levels](#overview-levels)). A trail view is a few kilobytes and takes
  milliseconds; the whole-domain image is hundreds of kilobytes and a reprojection of the whole
  domain.
- `crop` has no effect on a lat/lon grid (MRMS, GFS), which is always cut to the polygon (for a
  `MULTIPOLYGON`, to its first polygon, as ever). It is only for `f=png`; `crop=false` is the
  default.

A client can tell a server that does not know `crop` (an older deployment ignores it) by
`X-EDR-BBox`: if the box in the header is far larger than the polygon asked for, the answer is the
whole domain and the client should place it by the header as it always has.

## See Also

- [EDR Endpoints](./edr.md) - Full EDR API reference
- [WMS Endpoints](./wms.md) - Traditional tile-based rendering
- [decode_edr_png.py](../reference/scripts.md) - Python decoder script
