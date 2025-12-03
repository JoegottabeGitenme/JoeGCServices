# Wind Barb Renderer Implementation Plan

## Overview

Wind barbs are a standard meteorological symbol used to visualize wind speed and direction on weather maps. Each barb consists of:
- A **staff** pointing in the direction FROM which the wind blows
- **Flags/pennants** (50 knots), **long barbs** (10 knots), and **short barbs** (5 knots) indicating speed
- A **circle** at calm wind (< 3 knots)

This document outlines the implementation plan for adding wind barb rendering to the Weather WMS system.

---

## Data Requirements

### Inputs (Already Available)
- `u_data`: U-component of wind (eastward) - available as `gfs_UGRD`
- `v_data`: V-component of wind (northward) - available as `gfs_VGRD`

### Derived Values
- **Wind speed**: `sqrt(u² + v²)` in m/s → convert to knots (× 1.944)
- **Wind direction**: `atan2(-u, -v)` → direction wind is coming FROM (meteorological convention)

---

## Wind Barb Symbol Reference

```
Speed (knots)   Symbol Description
-----------     ------------------
Calm (0-2)      ○           Circle only (no staff)
3-7             ―╲          Staff + 1 short barb (5 kt)
8-12            ―╱          Staff + 1 long barb (10 kt)
13-17           ―╱╲         Staff + 1 long + 1 short (15 kt)
18-22           ―╱╱         Staff + 2 long barbs (20 kt)
23-27           ―╱╱╲        Staff + 2 long + 1 short (25 kt)
28-32           ―╱╱╱        Staff + 3 long barbs (30 kt)
...
48-52           ▶―          Pennant (filled triangle = 50 kt)
53-57           ▶―╲         Pennant + 1 short (55 kt)
58-62           ▶―╱         Pennant + 1 long (60 kt)
98-102          ▶▶―         2 pennants (100 kt)
```

### Visual Diagram

```
Wind FROM the North (pointing down):
                
    ╲           Short barb (5 kt)
     ╲          angled ~70° from staff
      │
      │         Staff (main line)
      │
      ●         Base point (plot location)


Wind FROM the Northwest at 65 knots:
    
    ▶           Pennant (50 kt)
     ╲╲         
      ╲         Long barb (10 kt)
       ╲        
        ╲       Short barb (5 kt)
         │
          ╲
           ●    Base point
```

---

## Implementation Plan

### Phase 1: Core Barb Drawing Functions

**File: `crates/renderer/src/barbs.rs`**

#### Data Structures

```rust
/// Represents a single wind barb to be drawn
pub struct WindBarb {
    /// X position on canvas (pixels)
    pub x: f32,
    /// Y position on canvas (pixels)
    pub y: f32,
    /// Wind speed in knots
    pub speed_knots: f32,
    /// Direction wind is FROM (radians, 0=North, clockwise)
    pub direction_rad: f32,
}

/// Configuration for wind barb rendering
pub struct BarbConfig {
    /// Length of main staff in pixels
    pub staff_length: f32,
    /// Length of speed barbs in pixels
    pub barb_length: f32,
    /// Spacing between barbs along staff in pixels
    pub barb_spacing: f32,
    /// Angle of barbs from staff in degrees (typically 70°)
    pub barb_angle_deg: f32,
    /// Barb line color
    pub color: Color,
    /// Line thickness in pixels
    pub line_width: u32,
    /// Radius of calm wind circle
    pub calm_radius: f32,
}

impl Default for BarbConfig {
    fn default() -> Self {
        Self {
            staff_length: 25.0,
            barb_length: 12.0,
            barb_spacing: 4.0,
            barb_angle_deg: 70.0,
            color: Color::new(0, 0, 0, 255), // Black
            line_width: 2,
            calm_radius: 4.0,
        }
    }
}
```

#### Core Functions

```rust
/// Convert U/V wind components to speed (knots) and direction (radians)
/// 
/// # Arguments
/// - `u`: Eastward wind component (m/s)
/// - `v`: Northward wind component (m/s)
/// 
/// # Returns
/// (speed_knots, direction_rad) where direction is FROM (meteorological)
pub fn uv_to_speed_direction(u: f32, v: f32) -> (f32, f32);

/// Draw a single wind barb onto a pixel buffer
/// 
/// # Arguments
/// - `pixels`: RGBA pixel buffer (4 bytes per pixel)
/// - `width`: Image width
/// - `height`: Image height
/// - `barb`: Wind barb data (position, speed, direction)
/// - `config`: Rendering configuration
pub fn draw_barb(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    barb: &WindBarb,
    config: &BarbConfig,
);

/// Draw a line using Bresenham's algorithm with anti-aliasing
fn draw_line(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x1: f32, y1: f32,
    x2: f32, y2: f32,
    color: Color,
    thickness: u32,
);

/// Draw the main staff line
fn draw_staff(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: f32, y: f32,
    direction: f32,
    length: f32,
    config: &BarbConfig,
) -> (f32, f32); // Returns tip position

/// Draw a pennant (50 knots - filled triangle)
fn draw_pennant(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    staff_x: f32, staff_y: f32,
    direction: f32,
    config: &BarbConfig,
) -> f32; // Returns new position along staff

/// Draw a long barb (10 knots - full line)
fn draw_long_barb(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    staff_x: f32, staff_y: f32,
    direction: f32,
    config: &BarbConfig,
);

/// Draw a short barb (5 knots - half line)
fn draw_short_barb(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    staff_x: f32, staff_y: f32,
    direction: f32,
    config: &BarbConfig,
);

/// Draw a calm wind circle (< 3 knots)
fn draw_calm_circle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x: f32, y: f32,
    config: &BarbConfig,
);

/// Fill a triangle (for pennants)
fn fill_triangle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x1: f32, y1: f32,
    x2: f32, y2: f32,
    x3: f32, y3: f32,
    color: Color,
);
```

### Phase 2: Grid Rendering with Decimation

Wind barbs should not be drawn at every grid point - they need to be spaced appropriately for readability.

```rust
/// Render wind barbs from U/V component grids
/// 
/// # Arguments
/// - `u_data`: U-component grid (eastward wind, m/s)
/// - `v_data`: V-component grid (northward wind, m/s)
/// - `grid_width`: Source grid width
/// - `grid_height`: Source grid height
/// - `output_width`: Output image width
/// - `output_height`: Output image height
/// - `barb_spacing`: Approximate pixel spacing between barbs
/// - `config`: Barb rendering configuration
/// 
/// # Returns
/// RGBA pixel data
pub fn render_wind_barbs(
    u_data: &[f32],
    v_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    output_width: usize,
    output_height: usize,
    barb_spacing: usize,
    config: &BarbConfig,
) -> Vec<u8>;

/// Calculate barb positions by decimating the grid
/// 
/// Samples U/V data at regular intervals to create readable barb spacing
fn calculate_barb_positions(
    u_data: &[f32],
    v_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    output_width: usize,
    output_height: usize,
    spacing: usize,
) -> Vec<WindBarb>;

/// Resample U/V data to output resolution with geographic bbox
pub fn resample_wind_data(
    u_data: &[f32],
    v_data: &[f32],
    grid_width: usize,
    grid_height: usize,
    output_width: usize,
    output_height: usize,
    bbox: Option<[f32; 4]>,
) -> (Vec<f32>, Vec<f32>);
```

### Phase 3: Rendering Algorithm

```
ALGORITHM: render_wind_barbs(u_data, v_data, ...)

1. Create transparent output buffer (RGBA)

2. Calculate decimation factor:
   step_x = output_width / barb_spacing
   step_y = output_height / barb_spacing

3. FOR each barb position (x, y) at decimated intervals:
   
   a. Sample U and V at this position (with interpolation)
   
   b. Calculate:
      speed_ms = sqrt(u² + v²)
      speed_knots = speed_ms * 1.944
      direction = atan2(-u, -v)  // FROM direction
   
   c. IF speed_knots < 3:
        draw_calm_circle(x, y)
        CONTINUE
   
   d. Draw staff pointing in wind direction
      tip_x, tip_y = calculate tip position from (x, y)
   
   e. Calculate barb counts:
      pennants   = floor(speed_knots / 50)
      remaining  = speed_knots % 50
      long_barbs = floor(remaining / 10)
      short_barb = floor((remaining % 10) / 5) > 0
   
   f. Draw symbols from TIP toward BASE:
      current_pos = tip
      
      FOR i in 0..pennants:
        draw_pennant(current_pos)
        current_pos += spacing along staff toward base
      
      FOR i in 0..long_barbs:
        draw_long_barb(current_pos)
        current_pos += spacing
      
      IF short_barb:
        // Short barb goes at end, slightly offset if it's the only barb
        draw_short_barb(current_pos)

4. RETURN pixel buffer
```

### Phase 4: WMS Integration

#### Updates to `services/wms-api/src/rendering.rs`

```rust
/// Render wind barbs combining U and V component data
/// 
/// This function loads both UGRD and VGRD data, combines them,
/// and renders wind barbs at appropriate spacing.
pub async fn render_wind_barbs_layer(
    storage: &ObjectStorage,
    catalog: &Catalog,
    model: &str,
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    barb_spacing: Option<usize>,
) -> Result<Vec<u8>, String> {
    // 1. Load U component (UGRD)
    let u_entry = catalog.get_latest(model, "UGRD").await?;
    let u_grib = storage.get(&u_entry.storage_path).await?;
    
    // 2. Load V component (VGRD)  
    let v_entry = catalog.get_latest(model, "VGRD").await?;
    let v_grib = storage.get(&v_entry.storage_path).await?;
    
    // 3. Parse and extract grid data
    let u_data = parse_grib_data(&u_grib, "UGRD")?;
    let v_data = parse_grib_data(&v_grib, "VGRD")?;
    
    // 4. Resample to output bbox if needed
    let (u_resampled, v_resampled) = resample_wind_data(...);
    
    // 5. Render wind barbs
    let config = BarbConfig::default();
    let spacing = barb_spacing.unwrap_or(50);
    
    let pixels = renderer::barbs::render_wind_barbs(
        &u_resampled,
        &v_resampled,
        width as usize,
        height as usize,
        width as usize,
        height as usize,
        spacing,
        &config,
    );
    
    // 6. Encode as PNG
    renderer::png::create_png(&pixels, width as usize, height as usize)
}
```

#### Updates to `services/wms-api/src/handlers.rs`

Add a new composite layer that combines UGRD and VGRD:

```rust
// In build_wms_capabilities():
// Add new layer for wind barbs
<Layer queryable="1">
    <Name>gfs_WIND_BARBS</Name>
    <Title>GFS - Wind Barbs</Title>
    <Abstract>Wind speed and direction shown as meteorological barbs</Abstract>
    <Style>
        <Name>default</Name>
        <Title>Black Barbs</Title>
    </Style>
    <Style>
        <Name>colored</Name>
        <Title>Speed-Colored Barbs</Title>
    </Style>
</Layer>

// In wms_getmap_handler():
// Detect wind barbs layer and call special renderer
if layer_name.contains("WIND_BARBS") {
    return render_wind_barbs_layer(storage, catalog, model, width, height, bbox, None).await;
}
```

### Phase 5: Style Configuration

**New file: `config/styles/wind_barbs.json`**

```json
{
  "name": "wind_barbs",
  "title": "Wind Barbs",
  "description": "Standard meteorological wind barb symbols",
  "type": "vector",
  "parameters": {
    "staff_length": 25,
    "barb_length": 12,
    "barb_spacing": 4,
    "barb_angle": 70,
    "line_width": 2,
    "calm_radius": 4,
    "grid_spacing": 50
  },
  "colors": {
    "default": [0, 0, 0, 255],
    "calm": [128, 128, 128, 255]
  },
  "speed_colors": {
    "enabled": false,
    "thresholds": [
      {"speed": 0, "color": [128, 128, 128, 255]},
      {"speed": 10, "color": [0, 128, 0, 255]},
      {"speed": 25, "color": [255, 255, 0, 255]},
      {"speed": 50, "color": [255, 128, 0, 255]},
      {"speed": 75, "color": [255, 0, 0, 255]},
      {"speed": 100, "color": [128, 0, 128, 255]}
    ]
  }
}
```

---

## Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `crates/renderer/src/barbs.rs` | **Create** | Core wind barb rendering implementation |
| `crates/renderer/src/lib.rs` | Modify | Export barbs module (already listed) |
| `config/styles/wind_barbs.json` | **Create** | Style configuration for barbs |
| `services/wms-api/src/rendering.rs` | Modify | Add `render_wind_barbs_layer()` function |
| `services/wms-api/src/handlers.rs` | Modify | Add WIND_BARBS layer to capabilities |
| `scripts/test_rendering.sh` | Modify | Add wind barb rendering tests |

---

## Testing Strategy

### Unit Tests (`crates/renderer/src/barbs.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uv_to_speed_direction() {
        // Pure eastward wind (u=10, v=0) -> from West (270°)
        let (speed, dir) = uv_to_speed_direction(10.0, 0.0);
        assert!((speed - 19.44).abs() < 0.1); // ~10 m/s = 19.44 knots
        assert!((dir - 4.712).abs() < 0.1);   // 270° in radians
        
        // Pure northward wind (u=0, v=10) -> from South (180°)
        let (speed, dir) = uv_to_speed_direction(0.0, 10.0);
        assert!((dir - 3.14159).abs() < 0.1); // 180° in radians
    }

    #[test]
    fn test_barb_counts() {
        // 65 knots = 1 pennant (50) + 1 long (10) + 1 short (5)
        assert_eq!(calculate_barb_counts(65.0), (1, 1, true));
        
        // 25 knots = 0 pennants + 2 long (20) + 1 short (5)
        assert_eq!(calculate_barb_counts(25.0), (0, 2, true));
        
        // 100 knots = 2 pennants
        assert_eq!(calculate_barb_counts(100.0), (2, 0, false));
    }

    #[test]
    fn test_calm_wind() {
        let (speed, _) = uv_to_speed_direction(0.5, 0.5);
        assert!(speed < 3.0); // Should be calm
    }
    
    #[test]
    fn test_draw_barb_north_wind() {
        let mut pixels = vec![0u8; 100 * 100 * 4];
        let barb = WindBarb {
            x: 50.0,
            y: 50.0,
            speed_knots: 25.0,
            direction_rad: 0.0, // From North
        };
        draw_barb(&mut pixels, 100, 100, &barb, &BarbConfig::default());
        // Verify pixels were modified
        assert!(pixels.iter().any(|&p| p != 0));
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_wind_barbs_wms_layer() {
    // Test that WIND_BARBS layer renders without error
    let response = wms_getmap_handler(
        "gfs_WIND_BARBS",
        "default",
        512, 256,
        Some([-180.0, -90.0, 180.0, 90.0]),
    ).await;
    
    assert!(response.is_ok());
    let png = response.unwrap();
    assert!(png.len() > 1000); // Should have content
}
```

### Visual Test Script Addition

```bash
# Add to scripts/test_rendering.sh

echo "=== WIND BARBS ==="
echo ""
echo "12. Global wind barbs"
test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (Global)" "-180,-90,180,90" 1024 512

echo "13. North Atlantic wind barbs"
test_getmap "gfs_WIND_BARBS" "GFS Wind Barbs (N. Atlantic)" "-80,20,-20,60" 512 384
```

---

## Edge Cases to Handle

1. **Calm wind** (< 3 knots): Draw circle only, no staff
2. **Very high winds** (> 100 knots): Multiple pennants
3. **Exactly 50 knots**: Single pennant, no additional barbs
4. **Wind at cardinal directions**: 0°, 90°, 180°, 270°
5. **Barbs at image edges**: Clip or skip barbs that would extend outside
6. **Missing data**: Skip barb if U or V is NaN/missing
7. **Antimeridian crossing**: Handle 180°/-180° longitude wrap

---

## Performance Considerations

1. **Decimation**: Don't draw barbs at every grid point
   - Typical spacing: 40-60 pixels between barbs
   - Adjust based on zoom level

2. **Early culling**: Skip barbs outside visible bbox

3. **Batch drawing**: Group similar operations

4. **Caching**: Cache barb symbol bitmaps for common speeds

---

## Estimated Effort

| Phase | Task | Time Estimate |
|-------|------|---------------|
| 1 | Core data structures | 30 min |
| 1 | Line drawing (Bresenham) | 1 hour |
| 1 | Staff/barb/pennant drawing | 2 hours |
| 1 | Calm circle drawing | 30 min |
| 2 | Grid decimation | 1 hour |
| 2 | UV resampling with bbox | 1 hour |
| 3 | WMS integration | 1.5 hours |
| 4 | Style configuration | 30 min |
| 5 | Unit tests | 1 hour |
| 5 | Integration tests | 1 hour |
| 5 | Visual testing & refinement | 1 hour |

**Total: ~10-12 hours**

---

## Future Enhancements

1. **Wind arrows**: Alternative to barbs (simpler, modern look)
2. **Streamlines**: Continuous flow lines following wind direction
3. **Animated particles**: Moving dots showing wind flow
4. **Color-coded barbs**: Color by speed (optional style)
5. **Variable density**: More barbs at higher zoom levels
6. **Label overlay**: Show speed values near barbs
7. **Beaufort scale option**: Alternative to knots

---

## References

- [WMO Manual on Codes](https://library.wmo.int/doc_num.php?explnum_id=10235) - Official wind barb specification
- [NOAA Wind Barb Guide](https://www.weather.gov/hfo/windbarbinfo)
- [MetPy Wind Barb Implementation](https://unidata.github.io/MetPy/latest/api/generated/metpy.plots.StationPlot.html)

---

## Appendix: Coordinate System Notes

### Meteorological Wind Direction Convention

- Wind direction is reported as the direction FROM which wind blows
- 0° = North, 90° = East, 180° = South, 270° = West
- Staff points INTO the wind (opposite of flow direction)

### U/V Component Signs

- **U (eastward)**: Positive = wind blowing toward east
- **V (northward)**: Positive = wind blowing toward north

### Conversion Formula

```rust
// From U/V to meteorological direction (FROM)
let speed = (u*u + v*v).sqrt();
let direction_rad = (-u).atan2(-v);  // Note: negated for FROM convention

// Convert radians to degrees if needed
let direction_deg = direction_rad.to_degrees();
let direction_deg = if direction_deg < 0.0 { 
    direction_deg + 360.0 
} else { 
    direction_deg 
};
```

---

**Document Version**: 1.0  
**Created**: 2025-11-25  
**Status**: Planning
