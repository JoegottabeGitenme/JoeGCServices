# EDR Service Enhancements for Windy-Style Visualization

Recommendations for enhancing your JoeGCServices EDR API to optimally support a WebGL-based weather visualization frontend.

---

## Current State Assessment

Based on the documentation, your EDR service already has strong foundations:

| Aspect | Current State | Visualization Readiness |
|--------|---------------|------------------------|
| Performance | 500+ req/s (position) | ✅ Excellent |
| Query types | All 7 EDR types | ✅ Complete |
| Output formats | CoverageJSON, GeoJSON | ⚠️ Needs PNG tiles |
| Caching | Chunk cache, location cache | ⚠️ Needs tile-aware caching |
| Scaling | Horizontal | ✅ Ready |
| Framework | Rust/Axum async | ✅ Ideal for streaming |

---

## Priority 1: Data Tile Output Format

### New Output Format: `image/png` with Data Encoding

Add support for returning area queries as PNG-encoded data tiles that WebGL can consume directly as textures.

**New endpoint variant:**
```
GET /edr/collections/{id}/area?
    bbox=-180,-90,180,90&
    parameter-name=UGRD,VGRD&
    width=256&
    height=256&
    f=image/png
```

**Response:** Raw PNG bytes with wind components encoded in channels.

### Implementation Approach

```rust
// In content_negotiation.rs, add new format
pub enum OutputFormat {
    CoverageJson,
    GeoJson,
    DataPng,      // NEW: PNG-encoded data texture
    DataPngInfo,  // NEW: JSON metadata for the PNG
}

// In handlers/area.rs, add PNG rendering path
async fn handle_area_query(
    // ... existing params
    format: OutputFormat,
) -> Response {
    match format {
        OutputFormat::DataPng => {
            let grid_data = fetch_grid_data(&query).await?;
            let png_bytes = encode_data_png(&grid_data, &encoding_config)?;
            
            Response::builder()
                .header("Content-Type", "image/png")
                .header("X-Data-Encoding", "wind-uv-rg")
                .header("X-Value-Range", "-50,50")
                .header("Cache-Control", "public, max-age=21600")
                .body(png_bytes)
        }
        // ... existing formats
    }
}
```

### Encoding Configuration

Add to `config/edr/*.yaml`:

```yaml
# config/edr/hrrr.yaml
collections:
  - id: hrrr-surface
    # ... existing config
    
    png_encoding:
      enabled: true
      presets:
        wind:
          parameters: [UGRD, VGRD]
          channels: [r, g]           # U→Red, V→Green
          value_range: [-50, 50]     # m/s
          empty_value: 128           # Mid-gray = 0
        
        temperature:
          parameters: [TMP]
          channels: [r]
          value_range: [200, 330]    # Kelvin
          
        combined:
          parameters: [UGRD, VGRD, TMP]
          channels: [r, g, b]        # U, V, Temp in one texture
          value_ranges:
            UGRD: [-50, 50]
            VGRD: [-50, 50]
            TMP: [200, 330]
```

### Encoding Implementation

```rust
// crates/edr-protocol/src/png_encoder.rs

use image::{ImageBuffer, Rgba, RgbaImage};

pub struct DataPngEncoder {
    width: u32,
    height: u32,
    channels: ChannelMapping,
}

pub struct ChannelMapping {
    pub r: Option<ParameterEncoding>,
    pub g: Option<ParameterEncoding>,
    pub b: Option<ParameterEncoding>,
    pub a: Option<ParameterEncoding>,  // Usually validity mask
}

pub struct ParameterEncoding {
    pub parameter: String,
    pub min_value: f32,
    pub max_value: f32,
}

impl DataPngEncoder {
    pub fn encode(
        &self,
        data: &HashMap<String, Vec<f32>>,  // parameter -> values
        validity: Option<&[bool]>,
    ) -> Result<Vec<u8>, EncodingError> {
        let mut img: RgbaImage = ImageBuffer::new(self.width, self.height);
        
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y * self.width + x) as usize;
                
                let r = self.encode_channel(&self.channels.r, data, idx);
                let g = self.encode_channel(&self.channels.g, data, idx);
                let b = self.encode_channel(&self.channels.b, data, idx);
                let a = validity.map_or(255, |v| if v[idx] { 255 } else { 0 });
                
                img.put_pixel(x, y, Rgba([r, g, b, a]));
            }
        }
        
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)?;
        Ok(bytes)
    }
    
    fn encode_channel(
        &self,
        channel: &Option<ParameterEncoding>,
        data: &HashMap<String, Vec<f32>>,
        idx: usize,
    ) -> u8 {
        match channel {
            Some(enc) => {
                let value = data.get(&enc.parameter)
                    .and_then(|v| v.get(idx))
                    .copied()
                    .unwrap_or(f32::NAN);
                
                if value.is_nan() {
                    128  // Neutral value for missing data
                } else {
                    let normalized = (value - enc.min_value) / (enc.max_value - enc.min_value);
                    (normalized.clamp(0.0, 1.0) * 255.0) as u8
                }
            }
            None => 0,
        }
    }
}
```

### Companion Metadata Endpoint

For each PNG request, clients need to know encoding parameters:

```
GET /edr/collections/{id}/area?...&f=image/png+info
```

Returns:
```json
{
  "format": "image/png",
  "width": 256,
  "height": 256,
  "encoding": {
    "r": { "parameter": "UGRD", "min": -50, "max": 50, "unit": "m/s" },
    "g": { "parameter": "VGRD", "min": -50, "max": 50, "unit": "m/s" },
    "b": null,
    "a": "validity_mask"
  },
  "bbox": [-180, -90, 180, 90],
  "crs": "CRS:84",
  "time": "2024-12-29T12:00:00Z"
}
```

---

## Priority 2: Tile Pyramid Support

### OGC API - Tiles Integration

Add a tiles endpoint that generates data tiles in a standard tile pyramid:

```
GET /edr/collections/{id}/tiles/{tileMatrixSetId}/{z}/{x}/{y}.png?
    parameter-name=UGRD,VGRD&
    datetime=2024-12-29T12:00:00Z
```

### Tile Matrix Sets

Support common web mapping tile schemes:

```yaml
# config/edr/tilematrixsets.yaml
tile_matrix_sets:
  WebMercatorQuad:
    crs: "EPSG:3857"
    tile_size: 256
    max_zoom: 10  # Weather data doesn't need street-level detail
    
  WorldCRS84Quad:
    crs: "CRS:84"
    tile_size: 256
    max_zoom: 8
```

### Implementation

```rust
// handlers/tiles.rs

use tile_grid::{TileMatrixSet, Tile};

pub async fn handle_tile_request(
    Path((collection_id, tms_id, z, x, y)): Path<(String, String, u8, u32, u32)>,
    Query(params): Query<TileQueryParams>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let tms = state.tile_matrix_sets.get(&tms_id)
        .ok_or(ApiError::NotFound("Tile matrix set not found"))?;
    
    let tile = Tile { z, x, y };
    let bbox = tms.tile_bounds(&tile);
    
    // Determine optimal resolution for this zoom level
    let (width, height) = (256, 256);
    
    // Reuse existing area query logic
    let grid_data = state.grid_processor
        .query_area(&collection_id, &bbox, width, height, &params.parameters, params.datetime)
        .await?;
    
    // Encode as PNG
    let png = encode_data_png(&grid_data, &params.encoding)?;
    
    // Cache headers based on data type
    let cache_control = get_cache_control_for_collection(&collection_id);
    
    Ok(Response::builder()
        .header("Content-Type", "image/png")
        .header("Cache-Control", cache_control)
        .header("ETag", generate_etag(&collection_id, &tile, params.datetime))
        .body(png.into()))
}
```

### Tile Caching Layer

Add a dedicated tile cache (separate from chunk cache):

```rust
// tile_cache.rs

use moka::future::Cache;

pub struct TileCache {
    cache: Cache<TileCacheKey, Arc<Vec<u8>>>,
}

#[derive(Hash, Eq, PartialEq, Clone)]
pub struct TileCacheKey {
    collection: String,
    z: u8,
    x: u32,
    y: u32,
    parameters: Vec<String>,
    datetime: DateTime<Utc>,
}

impl TileCache {
    pub fn new(max_size_mb: usize) -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity((max_size_mb * 1024 * 1024 / 50_000) as u64)  // ~50KB per tile
                .time_to_live(Duration::from_secs(3600))
                .build(),
        }
    }
    
    pub async fn get_or_generate<F, Fut>(
        &self,
        key: TileCacheKey,
        generate: F,
    ) -> Result<Arc<Vec<u8>>, Error>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, Error>>,
    {
        self.cache.try_get_with(key, async {
            generate().await.map(Arc::new)
        }).await
    }
}
```

---

## Priority 3: Temporal Batch Endpoints

For smooth animation, clients need multiple time steps efficiently.

### Batch Time Query

```
GET /edr/collections/{id}/area?
    bbox=...&
    parameter-name=UGRD,VGRD&
    datetime=2024-12-29T00:00:00Z/2024-12-30T00:00:00Z&
    datetime-step=PT1H&
    f=application/x-temporal-bundle
```

### Response: Temporal Bundle

A zip-like container with multiple PNGs and an index:

```
temporal-bundle.tar
├── index.json
├── 2024-12-29T00:00:00Z.png
├── 2024-12-29T01:00:00Z.png
├── 2024-12-29T02:00:00Z.png
└── ...
```

`index.json`:
```json
{
  "times": [
    "2024-12-29T00:00:00Z",
    "2024-12-29T01:00:00Z",
    "2024-12-29T02:00:00Z"
  ],
  "encoding": { ... },
  "bbox": [-180, -90, 180, 90]
}
```

### Alternative: Streaming Response

For progressive loading, use chunked transfer encoding:

```rust
pub async fn handle_temporal_stream(
    query: TemporalQuery,
    state: AppState,
) -> impl IntoResponse {
    let stream = async_stream::stream! {
        // First, send metadata
        let metadata = build_metadata(&query);
        yield Ok::<_, Error>(format!("META:{}\n", serde_json::to_string(&metadata)?));
        
        // Then stream each time step
        for time in query.time_range.iter_steps(query.step) {
            let png = generate_tile_for_time(&state, &query, time).await?;
            let b64 = base64::encode(&png);
            yield Ok(format!("DATA:{}:{}\n", time.to_rfc3339(), b64));
        }
    };
    
    Response::builder()
        .header("Content-Type", "application/x-edr-stream")
        .header("Transfer-Encoding", "chunked")
        .body(Body::wrap_stream(stream))
}
```

---

## Priority 4: Enhanced Cache Headers

### Per-Collection Cache Policies

```yaml
# config/edr/cache-policies.yaml
cache_policies:
  # GFS updates every 6 hours
  gfs:
    data_tiles:
      max_age: 21600           # 6 hours
      stale_while_revalidate: 3600
      stale_if_error: 86400
    metadata:
      max_age: 300
      
  # HRRR updates hourly  
  hrrr:
    data_tiles:
      max_age: 3600            # 1 hour
      stale_while_revalidate: 900
      stale_if_error: 7200
    metadata:
      max_age: 60
      
  # MRMS updates every 2 minutes
  mrms:
    data_tiles:
      max_age: 120             # 2 minutes
      stale_while_revalidate: 60
    metadata:
      max_age: 30
```

### Implementation

```rust
fn get_cache_headers(collection: &str, content_type: ContentType) -> HeaderMap {
    let policy = CONFIG.cache_policies.get(collection)
        .unwrap_or(&DEFAULT_POLICY);
    
    let settings = match content_type {
        ContentType::DataTile => &policy.data_tiles,
        ContentType::Metadata => &policy.metadata,
    };
    
    let mut headers = HeaderMap::new();
    
    let cache_control = format!(
        "public, max-age={}, stale-while-revalidate={}",
        settings.max_age,
        settings.stale_while_revalidate
    );
    
    headers.insert(CACHE_CONTROL, cache_control.parse().unwrap());
    
    // Add Surrogate-Control for CDN-specific behavior
    headers.insert(
        HeaderName::from_static("surrogate-control"),
        format!("max-age={}", settings.max_age * 2).parse().unwrap()
    );
    
    headers
}
```

### ETag Strategy

```rust
fn generate_etag(collection: &str, query: &Query, model_run: &DateTime<Utc>) -> String {
    use sha2::{Sha256, Digest};
    
    let mut hasher = Sha256::new();
    hasher.update(collection.as_bytes());
    hasher.update(query.bbox.to_string().as_bytes());
    hasher.update(query.parameters.join(",").as_bytes());
    hasher.update(model_run.timestamp().to_le_bytes());
    
    let hash = hasher.finalize();
    format!("\"{}\"", hex::encode(&hash[..8]))
}
```

---

## Priority 5: WebSocket for Live Updates

For real-time data like radar, add WebSocket support:

### Endpoint

```
WS /edr/collections/{id}/subscribe?
    bbox=...&
    parameter-name=...
```

### Implementation

```rust
// handlers/websocket.rs

use axum::extract::ws::{WebSocket, WebSocketUpgrade};

pub async fn handle_subscription(
    ws: WebSocketUpgrade,
    Path(collection_id): Path<String>,
    Query(params): Query<SubscriptionParams>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, collection_id, params, state))
}

async fn handle_socket(
    mut socket: WebSocket,
    collection_id: String,
    params: SubscriptionParams,
    state: AppState,
) {
    // Subscribe to data updates for this collection
    let mut rx = state.update_notifier.subscribe(&collection_id);
    
    // Send initial data
    let initial = generate_tile(&state, &collection_id, &params).await;
    if let Ok(data) = initial {
        let msg = SubscriptionMessage::Data {
            time: Utc::now(),
            png_base64: base64::encode(&data),
        };
        let _ = socket.send(Message::Text(serde_json::to_string(&msg).unwrap())).await;
    }
    
    // Stream updates
    loop {
        tokio::select! {
            // New data available
            Ok(update) = rx.recv() => {
                let data = generate_tile(&state, &collection_id, &params).await;
                if let Ok(data) = data {
                    let msg = SubscriptionMessage::Data {
                        time: update.time,
                        png_base64: base64::encode(&data),
                    };
                    if socket.send(Message::Text(serde_json::to_string(&msg).unwrap())).await.is_err() {
                        break;
                    }
                }
            }
            
            // Client message (ping/pong, unsubscribe)
            Some(msg) = socket.recv() => {
                match msg {
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(data)) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum SubscriptionMessage {
    Data {
        time: DateTime<Utc>,
        png_base64: String,
    },
    Metadata {
        encoding: EncodingInfo,
    },
    Error {
        message: String,
    },
}
```

### Update Notifier

```rust
// update_notifier.rs

use tokio::sync::broadcast;

pub struct UpdateNotifier {
    channels: DashMap<String, broadcast::Sender<DataUpdate>>,
}

impl UpdateNotifier {
    pub fn subscribe(&self, collection: &str) -> broadcast::Receiver<DataUpdate> {
        self.channels
            .entry(collection.to_string())
            .or_insert_with(|| broadcast::channel(16).0)
            .subscribe()
    }
    
    pub fn notify(&self, collection: &str, update: DataUpdate) {
        if let Some(tx) = self.channels.get(collection) {
            let _ = tx.send(update);
        }
    }
}
```

---

## Priority 6: CORS and Security Headers

Ensure the frontend can access the API from browsers:

```rust
// main.rs

use tower_http::cors::{CorsLayer, Any};

let cors = CorsLayer::new()
    .allow_origin(Any)  // Or specific origins for production
    .allow_methods([Method::GET, Method::OPTIONS])
    .allow_headers([
        ACCEPT,
        CONTENT_TYPE,
        HeaderName::from_static("x-requested-with"),
    ])
    .expose_headers([
        CACHE_CONTROL,
        ETAG,
        HeaderName::from_static("x-data-encoding"),
        HeaderName::from_static("x-value-range"),
    ])
    .max_age(Duration::from_secs(86400));

let app = Router::new()
    .merge(edr_routes())
    .layer(cors)
    // ... other layers
```

---

## New Configuration Section

Add to your EDR configuration:

```yaml
# config/edr/visualization.yaml
visualization:
  # PNG data tile settings
  png_tiles:
    enabled: true
    default_size: 256
    max_size: 1024
    compression_level: 6  # PNG compression (0-9)
    
  # Tile pyramid settings  
  tile_pyramids:
    enabled: true
    matrix_sets:
      - WebMercatorQuad
      - WorldCRS84Quad
    max_zoom: 10
    tile_cache_mb: 512
    
  # Temporal batch settings
  temporal_batch:
    enabled: true
    max_time_steps: 48
    formats:
      - tar
      - stream
      
  # WebSocket subscriptions
  subscriptions:
    enabled: true
    max_connections_per_collection: 1000
    heartbeat_interval_secs: 30
    
  # Common encoding presets
  encoding_presets:
    wind_surface:
      name: "Surface Wind"
      parameters: ["UGRD_10m", "VGRD_10m"]
      channels: { r: "UGRD_10m", g: "VGRD_10m" }
      ranges: { UGRD_10m: [-50, 50], VGRD_10m: [-50, 50] }
      
    wind_upper:
      name: "Upper Level Wind"
      parameters: ["UGRD", "VGRD"]
      channels: { r: "UGRD", g: "VGRD" }
      ranges: { UGRD: [-100, 100], VGRD: [-100, 100] }
      
    temperature:
      name: "Temperature"
      parameters: ["TMP"]
      channels: { r: "TMP" }
      ranges: { TMP: [200, 330] }
      
    precip_rate:
      name: "Precipitation Rate"
      parameters: ["PRATE"]
      channels: { r: "PRATE" }
      ranges: { PRATE: [0, 0.01] }  # kg/m²/s
```

---

## API Summary: New Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/edr/collections/{id}/area?f=image/png` | GET | Data tile (PNG encoded) |
| `/edr/collections/{id}/area?f=image/png+info` | GET | PNG encoding metadata |
| `/edr/collections/{id}/tiles/{tms}/{z}/{x}/{y}.png` | GET | Tiled data pyramid |
| `/edr/collections/{id}/tiles` | GET | Available tile matrix sets |
| `/edr/collections/{id}/temporal-bundle` | GET | Batch time steps |
| `/edr/collections/{id}/subscribe` | WS | Real-time updates |
| `/edr/encoding-presets` | GET | Available encoding presets |

---

## Implementation Roadmap

### Phase 1: Core PNG Output (1-2 weeks)
- [ ] Add `image/png` format to content negotiation
- [ ] Implement `DataPngEncoder` 
- [ ] Add encoding configuration to collection YAML
- [ ] Add `X-Data-Encoding` headers
- [ ] Update cache headers per collection

### Phase 2: Tile Pyramid (1-2 weeks)
- [ ] Add tile matrix set configuration
- [ ] Implement tile endpoint handler
- [ ] Add tile cache layer
- [ ] Integrate with existing grid processor

### Phase 3: Temporal Features (1 week)
- [ ] Implement temporal bundle format
- [ ] Add streaming response option
- [ ] Test with frontend animation

### Phase 4: Real-Time (1 week)
- [ ] Add WebSocket handler
- [ ] Implement update notifier
- [ ] Integrate with ingester notifications

### Phase 5: Production Hardening (1 week)
- [ ] Load testing with realistic traffic patterns
- [ ] CDN integration testing
- [ ] Monitoring and alerting for new endpoints
- [ ] Documentation updates

---

## Frontend Integration Example

With these enhancements, your frontend code becomes:

```javascript
class WeatherDataSource {
  constructor(edrBaseUrl) {
    this.baseUrl = edrBaseUrl;
    this.encodingCache = new Map();
  }
  
  async loadWindTile(bbox, datetime) {
    // Fetch PNG data tile directly
    const url = `${this.baseUrl}/collections/hrrr-surface/area?` +
      `bbox=${bbox.join(',')}&` +
      `parameter-name=UGRD_10m,VGRD_10m&` +
      `datetime=${datetime}&` +
      `width=256&height=256&` +
      `f=image/png`;
    
    const response = await fetch(url);
    
    // Get encoding info from headers
    const encoding = {
      type: response.headers.get('X-Data-Encoding'),
      range: response.headers.get('X-Value-Range')?.split(',').map(Number)
    };
    
    // Load directly as WebGL texture
    const blob = await response.blob();
    const bitmap = await createImageBitmap(blob);
    
    return { bitmap, encoding };
  }
  
  async loadTile(z, x, y, datetime) {
    const url = `${this.baseUrl}/collections/hrrr-surface/tiles/WebMercatorQuad/${z}/${x}/${y}.png?` +
      `parameter-name=UGRD_10m,VGRD_10m&` +
      `datetime=${datetime}`;
    
    // Browser will cache based on response headers
    const response = await fetch(url);
    const blob = await response.blob();
    return createImageBitmap(blob);
  }
  
  subscribeToUpdates(collection, bbox, onUpdate) {
    const ws = new WebSocket(
      `${this.baseUrl.replace('http', 'ws')}/collections/${collection}/subscribe?` +
      `bbox=${bbox.join(',')}&parameter-name=UGRD,VGRD`
    );
    
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.type === 'Data') {
        const binary = atob(msg.png_base64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
          bytes[i] = binary.charCodeAt(i);
        }
        onUpdate(new Blob([bytes], { type: 'image/png' }), msg.time);
      }
    };
    
    return () => ws.close();
  }
}
```

The key benefit: your frontend receives data in a format that goes directly to the GPU without any client-side transformation.
