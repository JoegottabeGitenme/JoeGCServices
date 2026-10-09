# WMS and WMTS GetCapabilities for specific layers

Ask for an up-to-date capabilities document for just the layers you care about,
instead of downloading and parsing the whole catalog. Works on both `/wms` and
`/wmts`, with the same parameters.

```
GET https://folkweather.com/wms?REQUEST=GetCapabilities&SERVICE=WMS&VERSION=1.3.0&layer=hrrr_DPT
```

| | Full document | `layer=hrrr_DPT` |
|---|---|---|
| Size | ~690 KB (219 layers) | ~3 KB |
| Server time (measured on the NUC) | ~1.0 s cold, ~1 ms cached | ~2 ms |
| Database queries | 228 (one per model/parameter) | 1 per layer |
| Cached by the server | yes (120 s) | no, always current |

A filtered document is built from the live catalog on every request, so the
`RUN` / `FORECAST` / `ELEVATION` / `TIME` dimensions and defaults are never
older than the data. (The full document can be up to 120 s stale.)

## Parameters

- `layer=` or `layers=`: either spelling, case-insensitive, same meaning.
  `layers` is the same parameter GetMap uses, so you can reuse the value you
  already have.
- Several layers: comma-separated, `layers=hrrr_DPT,hrrr_TMP,gfs_TMP`.
- If both are given, the two lists are combined. Duplicates are ignored.
- Layer names are case-insensitive and are the names the full document
  advertises (`{model}_{PARAMETER}`, e.g. `hrrr_DPT`, `nbm-conus_WIND_BARBS`).
- An empty value (`layer=`) means no filter: you get the full document.
- Omit both for the full document, exactly as before.

## What you get back

The normal `WMS_Capabilities` document: same `Service` and `Capability`
sections, but only the requested `<Layer>` elements. Each layer element is
byte-for-byte what the full document has for that layer (bounding boxes,
styles, dimensions). Composite layers such as `hrrr_WIND_BARBS` work; the
component layers they are derived from (`UGRD`, `VGRD`) are not included unless
you ask for them.

The layers are inside the usual wrapper layers (`WMS Server Root Layer` >
`Weather Data` > model), so parse them the same way you parse the full
document.

## Errors

If any requested name is not available, the whole request fails with a standard
WMS exception (HTTP 400) and no partial document:

```xml
<ServiceExceptionReport version="1.3.0" ...>
  <ServiceException code="LayerNotDefined">Layer 'nope_TMP' is not defined.</ServiceException>
</ServiceExceptionReport>
```

- `Layer 'x' is not defined.`: no such layer in the configuration. Answered
  without touching the database.
- `Layer 'x' has no data available.`: the layer exists but has no data right
  now, so the full document would not list it either. Treat it as "not
  currently available" and retry later.
- A database problem is reported as `NoApplicableCode` (HTTP 500), never as a
  missing layer.

## Notes

- `VERSION` is echoed into the document as before.
- CITE conformance test layers are never included in a filtered document.
- `gfs_TMP` appears twice in the full document and in a `layer=gfs_TMP`
  response, because `config/layers/gfs.yaml` also defines `gfs_SST` with
  parameter `TMP`. This is existing behavior.

## WMTS

```
GET https://folkweather.com/wmts?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0&layer=hrrr_DPT
```

Same parameters and rules as above (`layer` / `layers`, comma-separated,
case-insensitive, combined if both are given, empty = no filter). A comma may
be sent URL-encoded (`layers=hrrr_DPT%2Chrrr_TMP`).

| | Full document | `layer=hrrr_DPT` |
|---|---|---|
| Size | ~1.65 MB (218 layers) | ~19 KB |
| Server time (measured on the NUC) | ~0.5 s cold, ~2 ms cached | ~2 ms |
| Cached by the server | yes (120 s) | no, always current |

About 15.6 KB of the single-layer document is fixed: the service metadata and
the two `TileMatrixSet` definitions (WebMercatorQuad and WorldCRS84Quad), which
every WMTS document carries. The layer itself is ~3.8 KB (it includes the
`ResourceURL` tile templates and the dimension values).

You get the normal `Capabilities` document with only the requested `<Layer>`
elements; each is byte-for-byte what the full document has. `hrrr_WIND_BARBS`
works and does not include its UGRD/VGRD components.

### WMTS errors

WMTS has no `LayerNotDefined` code. Failures use the standard OWS
`ExceptionReport` with `InvalidParameterValue` and `locator="layer"` (HTTP 400),
the same response GetTile gives for an unknown layer:

```xml
<ows:ExceptionReport version="1.1.0" ...>
  <ows:Exception exceptionCode="InvalidParameterValue" locator="layer">
    <ows:ExceptionText>Layer 'nope_TMP' is not defined.</ows:ExceptionText>
  </ows:Exception>
</ows:ExceptionReport>
```

The messages are the same as for WMS (`is not defined` / `has no data
available`), and a database problem is `NoApplicableCode` (HTTP 500).

`LAYER` keeps its normal meaning on GetTile; the filter only applies to
`REQUEST=GetCapabilities`.
