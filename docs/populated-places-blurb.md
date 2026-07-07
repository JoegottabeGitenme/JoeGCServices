# Populated Places API — frontend blurb

_Short handoff summary. Full reference with example responses:
`docs/populated-places-frontend.md`._

---

**Populated Places API — city search + forecasts by US city (drop the geocoder)**

The `populated` EDR collection is a registry of ~10,200 US cities (population ≥
1,000). It powers **city-name search** (replacing Nominatim), resolves cities
to coordinates, and returns **point-forecast data by city** — no external
geocode or forecast provider needed.

Base: `https://folkweather.com/edr/collections/populated`
City IDs are `PP` + Census GEOID (e.g. `PP0820000` = Denver). Stable — safe to
persist client-side.

**1. Search by name (typeahead)** — replaces Nominatim
```
GET /collections/populated/locations?q=denv&limit=10
```
- Prefix + substring, **accent- and punctuation-insensitive** (`q=st louis` →
  St. Louis, `q=cañon` → Cañon City).
- Ranked exact > prefix > substring, then population — most relevant/populous
  first.
- `"City, ST"` / `"City ST"` to constrain by state (`q=springfield, il`), or
  use `state=IL`. Multi-word cities stay intact (`q=new york`).
- `limit` default 10, max 50. No population floor in search mode (small towns
  are findable). Combinable with `min-population` and `bbox`.
- No matches → **200** with `features: []`. Latency is typeahead-grade.
- Returns GeoJSON points: `id`, `properties.{name,state,population}`,
  `geometry.coordinates [lng, lat]`. No forecast payload — fetch that
  separately (step 3) when the user picks a result.

**2. Browse by population** (default lists)
```
GET /collections/populated/locations?min-population=50000&bbox=-98,32,-96,34
```
Same shape as search; `min-population` defaults to 25,000 when omitted.

**3. Forecast for one city** (weather card)
```
GET /collections/populated/locations/PP0820000?collections=gfs-surface,hrrr-surface&datetime=2026-07-08T00:00:00Z
```
Returns a GeoJSON feature with a `forecasts` object — one entry per model in
`collections=` (comma list, up to 5; default `gfs-surface`). `datetime` picks
the closest valid time; omit for latest. Values are native SI units.

**4. Forecasts for all cities near a point** (dropped pin / area)
```
GET /collections/populated/radius?coords=POINT(-96.8 32.78)&within=80km&min-population=100000&collections=gfs-surface
```
FeatureCollection, one feature per city, distance-ordered, each with the same
`forecasts` structure. `within` accepts `100km` / `50000m` / `30mi`.

**4b. Reverse geocode** (name a dropped pin)
```
GET /collections/populated/radius?coords=POINT(-104.99 39.74)&within=25km&min-population=0&limit=1
```
Returns the single nearest city — read `features[0].properties` for the
name/coords. `min-population=0` resolves even small nearby towns.

**Notes**
- US only (50 states + DC). Cities outside a model's grid just omit that model
  (e.g. HRRR is CONUS-only, so AK/HI won't have `hrrr-*`).
- URL-encode the space in `POINT(lon lat)` as `%20`.
- Errors: 404 unknown city id, 400 missing `coords` or >5 collections.
