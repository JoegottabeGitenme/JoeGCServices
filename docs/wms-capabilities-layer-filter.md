# WMS GetCapabilities for specific layers

Ask for an up-to-date capabilities document for just the layers you care about,
instead of downloading and parsing the whole catalog.

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
