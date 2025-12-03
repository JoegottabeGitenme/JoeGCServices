# GFS/HRRR Parameter Expansion Plan

## Overview

This document outlines the plan for expanding parameter ingestion from GFS and HRRR beyond the current basic set, and exposing them through WMS/WMTS.

---

## Current State

### Currently Ingested Parameters

#### GFS
| Parameter | Levels | Style | Description |
|-----------|--------|-------|-------------|
| TMP | 2m, pressure levels | temperature | Temperature |
| UGRD | 10m, pressure levels | wind | U-component wind |
| VGRD | 10m, pressure levels | wind | V-component wind |
| PRMSL | MSL | atmospheric | Sea level pressure |
| RH | 2m, pressure levels | default | Relative humidity |
| HGT | pressure levels | default | Geopotential height |
| GUST | surface | wind | Wind gust |

#### HRRR
| Parameter | Levels | Style | Description |
|-----------|--------|-------|-------------|
| TMP | 2m | temperature | Temperature |
| UGRD | 10m | wind | U-component wind |
| VGRD | 10m | wind | V-component wind |
| REFC | 1000m | reflectivity | Composite reflectivity |

### Currently Exposed WMS Layers

From the WMS capabilities, we expose:
- `gfs_TMP` - Temperature at 2m
- `gfs_WIND_BARBS` - Wind barbs (composite of UGRD/VGRD)
- `hrrr_TMP` - Temperature at 2m
- `hrrr_WIND_BARBS` - Wind barbs
- `hrrr_REFC` - Radar reflectivity
- Plus GOES and MRMS layers

---

## Proposed Parameter Expansion

### Phase 1: Surface & Near-Surface Parameters

These are the most commonly used for general weather display.

#### GFS New Parameters

| Parameter | Level | Style | Priority | Use Case |
|-----------|-------|-------|----------|----------|
| DPT | 2m | temperature | High | Dew point temperature |
| APCP | surface | precipitation | High | Total precipitation |
| CAPE | surface | atmospheric | High | Convective energy |
| CIN | surface | atmospheric | Medium | Convective inhibition |
| PWAT | atmosphere | precipitation | Medium | Precipitable water |
| VIS | surface | default | Medium | Visibility |
| TCDC | atmosphere | cloud | Medium | Total cloud cover |
| LCDC | low cloud | cloud | Low | Low cloud cover |
| MCDC | mid cloud | cloud | Low | Middle cloud cover |
| HCDC | high cloud | cloud | Low | High cloud cover |

#### HRRR New Parameters

| Parameter | Level | Style | Priority | Use Case |
|-----------|-------|-------|----------|----------|
| DPT | 2m | temperature | High | Dew point temperature |
| APCP | surface | precipitation | High | Total precipitation |
| CAPE | surface | atmospheric | High | Convective energy |
| VIS | surface | default | Medium | Visibility |
| LTNG | atmosphere | lightning | High | Lightning potential |
| MAXUVV | 100-1000mb | atmospheric | Medium | Max updraft velocity |
| MXUPHL | 2-5km | atmospheric | High | Max updraft helicity |
| RETOP | atmosphere | reflectivity | Medium | Echo top height |

### Phase 2: Upper-Air Parameters

For aviation and severe weather applications.

#### GFS Upper-Air

| Parameter | Levels (mb) | Style | Use Case |
|-----------|-------------|-------|----------|
| ABSV | 500 | vorticity | Absolute vorticity |
| CLWMR | 850, 700, 500 | cloud | Cloud water mixing ratio |
| ICMR | 850, 700, 500 | cloud | Ice mixing ratio |
| O3MR | 100, 50, 10 | atmospheric | Ozone mixing ratio |
| VVEL | 850, 700, 500 | atmospheric | Vertical velocity |

#### HRRR Upper-Air

| Parameter | Levels (mb) | Style | Use Case |
|-----------|-------------|-------|----------|
| TMP | 850, 700, 500, 300 | temperature | Upper-air temperature |
| UGRD | 850, 700, 500, 300 | wind | Upper-air U-wind |
| VGRD | 850, 700, 500, 300 | wind | Upper-air V-wind |
| RH | 850, 700, 500 | humidity | Upper-air humidity |
| HGT | 850, 700, 500, 300 | default | Geopotential height |

### Phase 3: Derived/Composite Products

Create virtual layers combining multiple parameters.

| Composite | Requires | Style | Description |
|-----------|----------|-------|-------------|
| WIND_SPEED | UGRD, VGRD | wind | Calculated wind speed |
| WIND_DIR | UGRD, VGRD | wind | Calculated wind direction |
| HEAT_INDEX | TMP, RH | temperature | Heat index |
| WIND_CHILL | TMP, UGRD, VGRD | temperature | Wind chill |
| THETA_E | TMP, DPT | temperature | Equivalent potential temp |
| SIG_TORNADO | CAPE, MXUPHL | severe | Significant tornado param |

---

## Implementation Plan

### Step 1: Update Model YAML Configs

Add new parameters to `config/models/gfs.yaml`:

```yaml
parameters:
  # ... existing parameters ...
  
  # Dew Point Temperature
  - name: DPT
    description: "Dew Point Temperature"
    levels:
      - type: height_above_ground
        value: 2
        display: "2 m above ground"
    style: temperature
    units: K
    display_units: °C
    conversion: K_to_C
    
  # Total Precipitation
  - name: APCP
    description: "Total Precipitation"
    levels:
      - type: surface
        display: "surface"
    style: precipitation
    units: kg/m^2
    display_units: mm
    accumulation: true
    
  # CAPE
  - name: CAPE
    description: "Convective Available Potential Energy"
    levels:
      - type: surface
        display: "surface"
    style: atmospheric
    units: J/kg
    
  # Precipitable Water
  - name: PWAT
    description: "Precipitable Water"
    levels:
      - type: entire_atmosphere
        display: "entire atmosphere"
    style: precipitation
    units: kg/m^2
    display_units: mm
    
  # Total Cloud Cover
  - name: TCDC
    description: "Total Cloud Cover"
    levels:
      - type: entire_atmosphere
        display: "entire atmosphere"
    style: cloud
    units: "%"
```

### Step 2: Create New Styles

Add new style configurations:

**`config/styles/precipitation.json`** (already exists, enhance):
```json
{
  "name": "precipitation",
  "type": "gradient",
  "description": "Precipitation amount coloring",
  "variants": {
    "default": {
      "stops": [
        { "value": 0, "color": "#ffffff00" },
        { "value": 0.1, "color": "#a6f28f" },
        { "value": 2.5, "color": "#39b54a" },
        { "value": 6.25, "color": "#2b74b0" },
        { "value": 12.5, "color": "#553285" },
        { "value": 25, "color": "#eb1c24" },
        { "value": 50, "color": "#f79621" },
        { "value": 100, "color": "#fff200" }
      ]
    }
  }
}
```

**`config/styles/cloud.json`** (new):
```json
{
  "name": "cloud",
  "type": "gradient",
  "description": "Cloud cover percentage",
  "variants": {
    "default": {
      "stops": [
        { "value": 0, "color": "#ffffff00" },
        { "value": 10, "color": "#f0f0f0" },
        { "value": 30, "color": "#c0c0c0" },
        { "value": 50, "color": "#909090" },
        { "value": 70, "color": "#606060" },
        { "value": 90, "color": "#404040" },
        { "value": 100, "color": "#303030" }
      ]
    }
  }
}
```

**`config/styles/cape.json`** (new):
```json
{
  "name": "cape",
  "type": "gradient", 
  "description": "CAPE values for convective potential",
  "variants": {
    "default": {
      "stops": [
        { "value": 0, "color": "#ffffff00" },
        { "value": 100, "color": "#ffffcc" },
        { "value": 500, "color": "#ffcc00" },
        { "value": 1000, "color": "#ff9900" },
        { "value": 2000, "color": "#ff0000" },
        { "value": 3000, "color": "#cc0066" },
        { "value": 5000, "color": "#990099" }
      ]
    }
  }
}
```

### Step 3: Update Ingester Target Parameters

The ingester needs to know which new parameters to extract. Add to the config loader:

```rust
// In config_loader.rs, update GRIB2 parameter matching

fn should_ingest_parameter(config: &ModelConfig, discipline: u8, category: u8, number: u8, level_type: u8, level_value: i32) -> bool {
    let param_name = grib2_param_name(discipline, category, number);
    
    for param in &config.parameters {
        if param.name == param_name {
            // Check if level matches any configured level
            for level_config in &param.levels {
                if level_matches(level_type, level_value, level_config) {
                    return true;
                }
            }
        }
    }
    
    false
}
```

### Step 4: Add Rendering Support

For each new parameter, ensure the renderer can handle it:

```rust
// In rendering.rs

fn get_style_for_layer(model: &str, param: &str, style_name: Option<&str>) -> WmsResult<Style> {
    // If explicit style requested, use it
    if let Some(name) = style_name {
        return load_style(name);
    }
    
    // Otherwise, get default style from config
    let config = load_model_config(model)?;
    
    for p in &config.parameters {
        if p.name == param {
            return load_style(&p.style);
        }
    }
    
    // Fallback to default gradient
    Ok(Style::default_gradient())
}
```

### Step 5: Update WMS Capabilities

The WMS GetCapabilities response needs to list all available layers:

```rust
// In capabilities.rs

async fn build_layer_list(catalog: &Catalog, configs: &[ModelConfig]) -> Vec<WmsLayer> {
    let mut layers = Vec::new();
    
    for config in configs {
        // Add individual parameter layers
        for param in &config.parameters {
            for level in &param.levels {
                let layer_name = format!("{}_{}", config.model.id, param.name);
                let title = format!("{} - {} ({})", 
                    config.model.name, 
                    param.description,
                    level.display
                );
                
                layers.push(WmsLayer {
                    name: layer_name,
                    title,
                    abstract_text: param.description.clone(),
                    styles: get_applicable_styles(param),
                    bbox: config.grid.bbox.clone(),
                    queryable: true,
                });
            }
        }
        
        // Add composite layers
        for composite in &config.composites {
            layers.push(WmsLayer {
                name: format!("{}_{}", config.model.id, composite.name),
                title: format!("{} - {}", config.model.name, composite.description),
                abstract_text: composite.description.clone(),
                styles: vec![composite.style.clone()],
                bbox: config.grid.bbox.clone(),
                queryable: true,
            });
        }
    }
    
    layers
}
```

### Step 6: Add Unit Conversions

Implement the conversion functions referenced in configs:

```rust
// In wms-common/src/conversions.rs

pub fn apply_conversion(value: f32, conversion: &str) -> f32 {
    match conversion {
        "K_to_C" => value - 273.15,
        "K_to_F" => (value - 273.15) * 9.0 / 5.0 + 32.0,
        "Pa_to_hPa" => value / 100.0,
        "Pa_to_mb" => value / 100.0,
        "m/s_to_kt" => value * 1.94384,
        "m/s_to_mph" => value * 2.23694,
        "kg/m2_to_mm" => value,  // For water, 1 kg/m² = 1 mm
        "percent_to_fraction" => value / 100.0,
        _ => value,
    }
}
```

---

## Layer Naming Convention

To maintain consistency and enable auto-discovery:

### Format
```
{model}_{parameter}[_{level}]
```

### Examples
- `gfs_TMP` - GFS temperature (default level: 2m)
- `gfs_TMP_850mb` - GFS temperature at 850mb
- `gfs_WIND_BARBS` - GFS wind barbs composite
- `hrrr_REFC` - HRRR composite reflectivity
- `hrrr_CAPE` - HRRR surface CAPE

### Query Parameters
For layers with multiple levels:
```
/wms?...&LAYERS=gfs_TMP&LEVEL=850mb
```

Or use explicit layer name:
```
/wms?...&LAYERS=gfs_TMP_850mb
```

---

## Testing Strategy

### 1. Ingestion Tests
```bash
# Test new parameter extraction
./scripts/download_gfs.sh
cargo run --package ingester -- --file /tmp/gfs.t00z.pgrb2.0p25.f000 --model gfs

# Verify in catalog
psql -c "SELECT DISTINCT parameter, level FROM datasets WHERE model='gfs' ORDER BY parameter"
```

### 2. Rendering Tests
```bash
# Test each new layer renders without error
for layer in gfs_DPT gfs_APCP gfs_CAPE gfs_TCDC; do
  curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer}&STYLES=&CRS=EPSG:4326&BBOX=-130,20,-60,55&WIDTH=512&HEIGHT=256&FORMAT=image/png" -o "/tmp/${layer}.png"
  echo "Generated ${layer}.png"
done
```

### 3. Style Tests
```bash
# Verify styles render correctly
for style in precipitation cloud cape; do
  curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=gfs_APCP&STYLES=${style}&CRS=EPSG:4326&BBOX=-130,20,-60,55&WIDTH=512&HEIGHT=256&FORMAT=image/png" -o "/tmp/style_${style}.png"
done
```

### 4. Capabilities Tests
```bash
# Verify all layers appear in capabilities
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities" | grep -o '<Layer queryable="1">' | wc -l
```

---

## Priority Matrix

| Parameter | Model | Priority | Effort | Impact |
|-----------|-------|----------|--------|--------|
| DPT | GFS, HRRR | High | Low | High - Common request |
| APCP | GFS, HRRR | High | Medium | High - Precipitation maps |
| CAPE | GFS, HRRR | High | Low | High - Severe weather |
| TCDC | GFS | Medium | Low | Medium - Cloud cover |
| VIS | GFS, HRRR | Medium | Low | Medium - Aviation |
| MXUPHL | HRRR | High | Medium | High - Tornado prediction |
| LTNG | HRRR | High | Medium | High - Lightning forecast |
| Upper TMP | HRRR | Medium | Low | Medium - Aviation |
| Upper WIND | GFS, HRRR | Medium | Low | Medium - Aviation |

---

## Rollout Plan

### Week 1: Phase 1 Surface Parameters
- Add DPT, APCP, CAPE, TCDC to GFS config
- Add DPT, APCP, CAPE to HRRR config
- Create precipitation, cloud, cape styles
- Test ingestion and rendering

### Week 2: HRRR Severe Weather Parameters
- Add MXUPHL, LTNG, RETOP to HRRR
- Create severe weather styles
- Test with recent severe weather data

### Week 3: Upper-Air Parameters
- Add upper-level TMP, RH, HGT
- Add WIND_BARBS at multiple levels
- Create aviation-focused styles

### Week 4: Documentation & Dashboard
- Update admin dashboard to show all parameters
- Document new layers in API docs
- Add parameter descriptions to GetCapabilities

---

## Storage Considerations

### Estimated Additional Storage Per Model Cycle

| Model | Current | Phase 1 | Phase 2 | Phase 3 | Total |
|-------|---------|---------|---------|---------|-------|
| GFS | ~500MB | +200MB | +300MB | +100MB | ~1.1GB |
| HRRR | ~200MB | +100MB | +150MB | +50MB | ~500MB |

### Retention Impact

With 7-day GFS retention and hourly HRRR:
- GFS: 4 cycles/day × 7 days × 1.1GB = ~31GB
- HRRR: 24 cycles/day × 3 days × 500MB = ~36GB

Total: ~67GB (up from current ~25GB)

---

## Success Criteria

- [ ] All Phase 1 parameters ingested and rendering correctly
- [ ] All new layers appear in WMS GetCapabilities
- [ ] Style selection works for all parameters
- [ ] Unit conversions applied correctly (e.g., K to °C)
- [ ] Admin dashboard shows full parameter list
- [ ] No significant increase in render latency
- [ ] Load tests pass with expanded parameter set
