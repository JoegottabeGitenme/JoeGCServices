# Configuration Consolidation - Next Steps

## Current State Summary

The configuration consolidation effort (documented in `INGESTION_CONSOLIDATION_PLAN.md`) is **largely complete**:

- **Model configs**: `config/models/*.yaml` - 5 models configured (GFS, HRRR, GOES-16, GOES-18, MRMS)
- **Parameter tables**: `config/parameters/*.yaml` - 3 tables (WMO, NCEP, MRMS) with 250+ parameters
- **Global settings**: `config/ingestion.yaml` - Database, cache, storage, download settings
- **Admin dashboard**: `web/admin.html` - View status, logs, configs, and shred previews
- **Config loader**: `services/ingester/src/config_loader.rs` - Parses YAML with env var substitution

---

## Remaining Work

### 1. Admin Dashboard - Show Detailed Parameter Extraction

**Current Problem**: The admin dashboard shows *how many* parameters will be extracted, but not *what* they will be in detail.

**Goal**: Show a comprehensive preview of:
- Each parameter name and description
- All levels for each parameter (expanded, not summarized)
- The corresponding WMS/WMTS layer names that will be created
- Available styles for each parameter
- Whether the parameter has rendering support

#### Implementation Plan

##### A. Enhance `ShredPreviewResponse` (backend)

**File**: `services/wms-api/src/admin.rs`

Add new fields to provide richer information:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ShredPreviewResponse {
    pub model_id: String,
    pub model_name: String,
    pub source_type: String,
    pub parameters: Vec<ParameterPreview>,  // Renamed from parameters_to_extract
    pub composites: Vec<CompositePreview>,  // NEW: Show composite layers
    pub total_extractions: usize,
    pub total_layers: usize,                // NEW: Total WMS layers that will be created
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterPreview {
    pub name: String,
    pub description: String,
    pub units: String,
    pub display_units: Option<String>,      // NEW: Display units after conversion
    pub style: String,
    pub style_exists: bool,                 // NEW: Check if style file exists
    pub available_styles: Vec<String>,      // NEW: All styles that could apply
    pub levels: Vec<LevelPreview>,
    pub layer_names: Vec<String>,           // NEW: Actual WMS layer names
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelPreview {
    pub level_type: String,
    pub value: Option<String>,
    pub display: String,
    pub storage_path_template: String,
    pub wms_layer_name: String,             // NEW: e.g., "gfs_TMP_2m"
    pub wmts_layer_name: String,            // NEW: e.g., "gfs:TMP:2m"
}

#[derive(Debug, Clone, Serialize)]
pub struct CompositePreview {
    pub name: String,
    pub description: String,
    pub requires: Vec<String>,              // Required base parameters
    pub renderer: String,
    pub style: String,
    pub wms_layer_name: String,
}
```

##### B. Add Style Lookup (backend)

Create a helper to check which styles are available:

```rust
/// Check if a style file exists and list all available styles
fn get_available_styles(param_name: &str, default_style: &str) -> (bool, Vec<String>) {
    let styles_dir = Path::new("config/styles");
    let mut available = Vec::new();
    let mut default_exists = false;
    
    if let Ok(entries) = fs::read_dir(styles_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.path().file_stem().and_then(|s| s.to_str()) {
                available.push(name.to_string());
                if name == default_style {
                    default_exists = true;
                }
            }
        }
    }
    
    (default_exists, available)
}
```

##### C. Update Admin UI (frontend)

**File**: `web/admin.html`

Add a new detailed view for the shred preview:

```html
<!-- Enhanced Shred Preview Tab -->
<div class="tab-content" id="preview-tab">
    <div id="shred-preview-container">
        <!-- Summary Stats -->
        <div class="preview-summary">
            <div class="stat-box">
                <div class="stat-value" id="preview-params">--</div>
                <div class="stat-label">Parameters</div>
            </div>
            <div class="stat-box">
                <div class="stat-value" id="preview-levels">--</div>
                <div class="stat-label">Levels</div>
            </div>
            <div class="stat-box">
                <div class="stat-value" id="preview-layers">--</div>
                <div class="stat-label">WMS Layers</div>
            </div>
            <div class="stat-box">
                <div class="stat-value" id="preview-composites">--</div>
                <div class="stat-label">Composites</div>
            </div>
        </div>
        
        <!-- Parameter Details Table -->
        <div class="preview-table-container">
            <table class="preview-table">
                <thead>
                    <tr>
                        <th>Parameter</th>
                        <th>Level</th>
                        <th>WMS Layer</th>
                        <th>Style</th>
                        <th>Units</th>
                    </tr>
                </thead>
                <tbody id="preview-table-body">
                    <!-- Populated by JavaScript -->
                </tbody>
            </table>
        </div>
        
        <!-- Composites Section -->
        <div class="composites-section">
            <h3>Composite Layers</h3>
            <div id="composites-list"></div>
        </div>
    </div>
</div>
```

**File**: `web/admin.js`

Add function to render detailed preview:

```javascript
function renderShredPreview(preview) {
    // Update summary stats
    document.getElementById('preview-params').textContent = preview.parameters.length;
    document.getElementById('preview-levels').textContent = preview.total_extractions;
    document.getElementById('preview-layers').textContent = preview.total_layers;
    document.getElementById('preview-composites').textContent = preview.composites?.length || 0;
    
    // Build parameter table
    const tbody = document.getElementById('preview-table-body');
    tbody.innerHTML = '';
    
    for (const param of preview.parameters) {
        for (const level of param.levels) {
            const row = document.createElement('tr');
            row.innerHTML = `
                <td>
                    <strong>${param.name}</strong>
                    <div class="param-desc">${param.description}</div>
                </td>
                <td>${level.display}</td>
                <td><code>${level.wms_layer_name}</code></td>
                <td>
                    <span class="style-badge ${param.style_exists ? 'exists' : 'missing'}">
                        ${param.style}
                    </span>
                </td>
                <td>${param.display_units || param.units}</td>
            `;
            tbody.appendChild(row);
        }
    }
    
    // Build composites list
    const compositesList = document.getElementById('composites-list');
    compositesList.innerHTML = '';
    
    for (const comp of (preview.composites || [])) {
        const item = document.createElement('div');
        item.className = 'composite-item';
        item.innerHTML = `
            <div class="composite-name">${comp.name}</div>
            <div class="composite-desc">${comp.description}</div>
            <div class="composite-requires">Requires: ${comp.requires.join(', ')}</div>
            <div class="composite-layer">Layer: <code>${comp.wms_layer_name}</code></div>
        `;
        compositesList.appendChild(item);
    }
}
```

##### D. Add Layer Name Generation

The ingester needs to generate consistent layer names. Add to config_loader.rs:

```rust
impl ModelConfig {
    /// Generate WMS layer name for a parameter at a specific level
    pub fn wms_layer_name(&self, param: &str, level: &str) -> String {
        // Format: {model}_{param}_{level_sanitized}
        // Examples: gfs_TMP_2m, hrrr_UGRD_10m, gfs_TMP_850mb
        let level_sanitized = level
            .replace(' ', "")
            .replace("above", "")
            .replace("ground", "")
            .replace("_", "");
        format!("{}_{}", self.model.id, format!("{}_{}", param, level_sanitized))
    }
    
    /// Generate WMTS layer name for a parameter at a specific level
    pub fn wmts_layer_name(&self, param: &str, level: &str) -> String {
        // Format: {model}:{param}:{level}
        format!("{}:{}:{}", self.model.id, param, level)
    }
}
```

---

### 2. Sync Ingester with Config Files

**Current Problem**: The ingester still has some hardcoded fallbacks and doesn't fully use the YAML configs at runtime.

#### Implementation Tasks

##### A. Remove Remaining Hardcoded Values

**File**: `services/ingester/src/main.rs`

- [ ] Remove hardcoded `pressure_levels` HashSet (lines 261-265)
- [ ] Remove hardcoded `target_params` Vec (lines 269-291)
- [ ] Load these from `config/models/{model}.yaml` at startup
- [ ] Add validation that all referenced parameters exist in parameter tables

##### B. Add Config Validation on Startup

```rust
async fn validate_config_on_startup(config: &ModelConfig) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    
    // Check that referenced styles exist
    for param in &config.parameters {
        let style_path = format!("config/styles/{}.json", param.style);
        if !Path::new(&style_path).exists() {
            errors.push(format!(
                "Parameter {} references non-existent style: {}",
                param.name, param.style
            ));
        }
    }
    
    // Check that composite requirements are met
    for composite in &config.composites {
        for required in &composite.requires {
            if !config.parameters.iter().any(|p| &p.name == required) {
                errors.push(format!(
                    "Composite {} requires parameter {} which is not configured",
                    composite.name, required
                ));
            }
        }
    }
    
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
```

##### C. Add Config Hot-Reload (Optional)

For a future enhancement, add file watcher to reload configs without restart:

```rust
use notify::{Watcher, RecursiveMode, watcher};

fn watch_config_changes(config_dir: &Path, tx: Sender<ConfigReloadEvent>) {
    let (file_tx, file_rx) = std::sync::mpsc::channel();
    let mut watcher = watcher(file_tx, Duration::from_secs(2)).unwrap();
    
    watcher.watch(config_dir, RecursiveMode::Recursive).unwrap();
    
    std::thread::spawn(move || {
        for event in file_rx {
            if let Ok(event) = event {
                // Reload config
                tx.send(ConfigReloadEvent::Reload).unwrap();
            }
        }
    });
}
```

---

### 3. Style System Improvements

**Current Problem**: Styles are JSON files but not connected to the config system.

#### Implementation Tasks

##### A. Add Style References in Model Configs

Already done in YAML, but ensure consistency:

```yaml
parameters:
  - name: TMP
    style: temperature          # References config/styles/temperature.json
    style_variants:             # NEW: Optional style variants
      - temperature_isolines
      - temperature_colors
```

##### B. Create Style Index API

New endpoint to list available styles:

```
GET /api/admin/styles
```

Response:
```json
{
    "styles": [
        {
            "name": "temperature",
            "description": "Temperature gradient coloring",
            "type": "gradient",
            "applies_to": ["TMP", "T2M"],
            "variants": ["default", "isolines", "fine"]
        },
        {
            "name": "wind_barbs",
            "description": "Wind barb symbols",
            "type": "symbols",
            "applies_to": ["WIND_BARBS"],
            "variants": ["default"]
        }
    ]
}
```

##### C. Style Validation

Add style validation to the admin API:

```rust
fn validate_style_json(content: &str) -> Vec<String> {
    let mut errors = Vec::new();
    
    let style: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("JSON syntax error: {}", e));
            return errors;
        }
    };
    
    // Check required fields
    if style.get("type").is_none() {
        errors.push("Missing required field: 'type'".to_string());
    }
    
    if style.get("colors").is_none() && style.get("symbols").is_none() {
        errors.push("Must have either 'colors' or 'symbols'".to_string());
    }
    
    errors
}
```

---

## Implementation Priority

| Task | Priority | Effort | Impact |
|------|----------|--------|--------|
| Enhance ShredPreviewResponse | High | Medium | Better visibility into ingestion |
| Update Admin UI for detailed preview | High | Medium | Essential for operators |
| Add Layer Name Generation | High | Low | Required for WMS layer mapping |
| Remove hardcoded ingester values | Medium | Medium | Code cleanup |
| Style index API | Medium | Low | Nice to have |
| Config hot-reload | Low | High | Convenience feature |
| Style validation | Low | Low | Quality improvement |

---

## Testing Plan

1. **Unit Tests**
   - Test layer name generation with various inputs
   - Test style existence checking
   - Test config validation

2. **Integration Tests**
   - Load each model config and verify shred preview
   - Verify WMS layer names match expected format
   - Test admin API endpoints return correct data

3. **Manual Testing**
   - Open admin dashboard
   - Select each model and verify preview shows all parameters
   - Check that layer names are correct
   - Verify style badges show correct status

---

## Success Criteria

- [ ] Admin dashboard shows complete list of parameters to be extracted
- [ ] Each parameter shows all its levels (not just a count)
- [ ] WMS layer names are displayed for each parameter/level combination
- [ ] Style existence is indicated (green = exists, red = missing)
- [ ] Composite layers are shown with their required parameters
- [ ] No hardcoded parameter lists remain in ingester code
