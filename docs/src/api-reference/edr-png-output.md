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
| 400 Dimensions too large | width or height > 4096 | Reduce dimensions |
| 413 Area too large | Exceeds `max_area_sq_degrees_png` | Reduce polygon size |

## See Also

- [EDR Endpoints](./edr.md) - Full EDR API reference
- [WMS Endpoints](./wms.md) - Traditional tile-based rendering
- [decode_edr_png.py](../reference/scripts.md) - Python decoder script
