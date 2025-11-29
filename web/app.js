// WMS Dashboard Application

const API_BASE = 'http://localhost:8080';
const REDIS_URL = 'http://localhost:8080/api/ingestion'; // Placeholder - would need API endpoint

// External service URLs - can be customized for different environments
const EXTERNAL_URLS = {
    minio: 'http://localhost:9001',
    // Standard K8s dashboard URL when using kubectl proxy
    k8sDashboard: 'http://localhost:8001/api/v1/namespaces/kubernetes-dashboard/services/https:kubernetes-dashboard:/proxy/',
    // Alternative: Direct NodePort or LoadBalancer URL
    // k8sDashboard: 'https://localhost:30443',
};
let map;
let wmsLayer = null;
let selectedLayer = null;
let ingestionStatusInterval = null;
let currentForecastHour = 0; // Default to analysis/earliest (hour 0)
let currentRun = 'latest'; // Default to latest run
let currentElevation = ''; // Default to surface/empty
let availableRuns = [];
let availableForecastHours = [0, 3, 6, 12, 24];
let availableElevations = []; // Available levels for current layer
let availableObservationTimes = []; // For TIME-only layers (GOES, MRMS)
let currentObservationTime = null; // Selected observation time
let layerTimeMode = 'forecast'; // 'forecast' (RUN+FORECAST) or 'observation' (TIME only)
let playbackInterval = null;
let isPlaying = false;

// DOM Elements
const wmsStatusEl = document.getElementById('wms-status');
const wmtsStatusEl = document.getElementById('wmts-status');
const ingesterServiceStatusEl = document.getElementById('ingester-service-status');
const datasetCountEl = document.getElementById('dataset-count');
const modelsListEl = document.getElementById('models-list');
const storageSizeEl = document.getElementById('storage-size');
const ingestLogEl = document.getElementById('ingest-log');

// Layer Selection Elements
const protocolRadios = document.querySelectorAll('input[name="protocol"]');
const layerSelectEl = document.getElementById('layer-select');
const styleSelectEl = document.getElementById('style-select');
const styleGroupEl = document.getElementById('style-group');
const loadLayerBtnEl = document.getElementById('load-layer-btn');

// Performance Tracking
const performanceStats = {
    tilesLoaded: 0,
    tileTimes: [],
    currentLayer: null,
    layerStartTime: null
};

const lastTileTimeEl = document.getElementById('last-tile-time');
const avgTileTimeEl = document.getElementById('avg-tile-time');
const tilesLoadedCountEl = document.getElementById('tiles-loaded-count');
const slowestTileTimeEl = document.getElementById('slowest-tile-time');
const currentLayerNameEl = document.getElementById('current-layer-name');

// Backend Metrics Elements
const metricWmsRequestsEl = document.getElementById('metric-wms-requests');
const metricWmtsRequestsEl = document.getElementById('metric-wmts-requests');
const metricRendersEl = document.getElementById('metric-renders');
const metricRenderAvgEl = document.getElementById('metric-render-avg');
const metricRenderLastEl = document.getElementById('metric-render-last');
const metricCacheStatusEl = document.getElementById('metric-cache-status');
const metricCacheKeysEl = document.getElementById('metric-cache-keys');
const metricCacheMemoryEl = document.getElementById('metric-cache-memory');
const metricMemoryEl = document.getElementById('metric-memory');
const metricThreadsEl = document.getElementById('metric-threads');
const metricUptimeEl = document.getElementById('metric-uptime');

// State for layer selection
let availableLayers = [];
let layerStyles = {}; // Map of layer name -> array of styles
let layerBounds = {}; // Map of layer name -> {west, south, east, north}
let selectedProtocol = 'wmts';
let selectedStyle = 'default';
let queryEnabled = true;

// Initialize the application
document.addEventListener('DOMContentLoaded', () => {
    initMap();
    checkServiceStatus();
    loadAvailableLayers();
    initIngestionStatus();
    setupEventListeners();
    setupMapClickHandler();
});

// Setup event listeners for protocol and layer selection
function setupEventListeners() {
    // Protocol radio buttons
    protocolRadios.forEach(radio => {
        radio.addEventListener('change', onProtocolChange);
    });
    
    layerSelectEl.addEventListener('change', onLayerChange);
    loadLayerBtnEl.addEventListener('click', onLoadLayer);
    
    // Query toggle
    const queryToggle = document.getElementById('enable-query');
    if (queryToggle) {
        queryToggle.addEventListener('change', (e) => {
            queryEnabled = e.target.checked;
            updateQueryHint();
        });
    }
    
    // Time control listeners
    setupTimeControls();
}

// Setup time control event listeners
function setupTimeControls() {
    const timeSlider = document.getElementById('time-slider');
    const runSelect = document.getElementById('run-select');
    const elevationSelect = document.getElementById('elevation-select');
    const obsTimeSelect = document.getElementById('observation-time-select');
    
    if (timeSlider) {
        timeSlider.addEventListener('input', (e) => {
            // Slider value is an index into availableForecastHours array
            const index = parseInt(e.target.value);
            if (index >= 0 && index < availableForecastHours.length) {
                currentForecastHour = availableForecastHours[index];
                console.log('Forecast hour changed to:', currentForecastHour, '(index:', index, ')');
                updateTimeDisplay();
                updateLayerTime();
            }
        });
    }
    
    if (runSelect) {
        runSelect.addEventListener('change', (e) => {
            currentRun = e.target.value;
            console.log('Model run changed to:', currentRun);
            updateLayerTime();
        });
    }
    
    if (elevationSelect) {
        elevationSelect.addEventListener('change', (e) => {
            currentElevation = e.target.value;
            console.log('Elevation changed to:', currentElevation);
            updateLayerTime();
        });
    }
    
    // Observation time selector (for GOES, MRMS)
    if (obsTimeSelect) {
        obsTimeSelect.addEventListener('change', onObservationTimeChange);
    }
}

// Handle protocol selection change
function onProtocolChange() {
    // Get selected radio button value
    const selectedRadio = document.querySelector('input[name="protocol"]:checked');
    selectedProtocol = selectedRadio ? selectedRadio.value : 'wmts';
    console.log('Protocol changed to:', selectedProtocol);
    // Reload layers for the new protocol
    loadAvailableLayers();
}

// Handle layer selection change
function onLayerChange() {
    const layerName = layerSelectEl.value;
    console.log('Layer changed to:', layerName);
    
    if (!layerName) {
        styleGroupEl.style.display = 'none';
        return;
    }
    
    // Load styles for this layer
    const styles = layerStyles[layerName] || [];
    
    if (styles.length > 1) {
        // Show style dropdown if layer has multiple styles
        styleSelectEl.innerHTML = '';
        styles.forEach(style => {
            const option = document.createElement('option');
            option.value = style.name;
            option.textContent = style.title;
            styleSelectEl.appendChild(option);
        });
        styleGroupEl.style.display = 'block';
    } else {
        // Hide style dropdown if only one style
        styleGroupEl.style.display = 'none';
    }
}

// Initialize Leaflet map
function initMap() {
    map = L.map('map').setView([20, 0], 3);

    // Add base layer
    L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
        attribution: '© OpenStreetMap contributors',
        maxZoom: 19
    }).addTo(map);

    // Add attribution for weather data
    map.attributionControl.addAttribution('Weather Data: WMS Service');
}

// Check WMS and WMTS service status
async function checkServiceStatus() {
    try {
        // Check WMS
        const wmsResponse = await fetch(
            `${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`,
            { mode: 'cors', cache: 'no-cache' }
        );
        setStatusIndicator('wms', wmsResponse.ok ? 'online' : 'offline');
    } catch (error) {
        console.error('WMS status check failed:', error);
        setStatusIndicator('wms', 'offline');
    }

    try {
        // Check WMTS
        const wmtsResponse = await fetch(
            `${API_BASE}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities`,
            { mode: 'cors', cache: 'no-cache' }
        );
        setStatusIndicator('wmts', wmtsResponse.ok ? 'online' : 'offline');
    } catch (error) {
        console.error('WMTS status check failed:', error);
        setStatusIndicator('wmts', 'offline');
    }
}

// Update status indicator
function setStatusIndicator(service, status) {
    const element = service === 'wms' ? wmsStatusEl : wmtsStatusEl;
    const statusDot = element.querySelector('.status-dot');
    const statusText = element.querySelector('.status-text');

    statusDot.className = `status-dot ${status}`;
    statusText.textContent = status.charAt(0).toUpperCase() + status.slice(1);
}

// Load available layers from WMS/WMTS capabilities
async function loadAvailableLayers() {
    try {
        const service = selectedProtocol === 'wmts' ? 'WMTS' : 'WMS';
        const endpoint = selectedProtocol === 'wmts' ? 'wmts' : 'wms';
        
        const response = await fetch(
            `${API_BASE}/${endpoint}?SERVICE=${service}&REQUEST=GetCapabilities`,
            { mode: 'cors' }
        );
        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');

        // Extract layer names, styles, and bounds
        const layers = [];
        layerStyles = {}; // Reset styles map
        layerBounds = {}; // Reset bounds map
        
        if (selectedProtocol === 'wmts') {
            // WMTS uses <ows:Identifier> for layer names
            const layerElements = xml.querySelectorAll('Contents > Layer');
            layerElements.forEach(layerEl => {
                const identifierEl = layerEl.querySelector('Identifier');
                if (identifierEl && identifierEl.textContent) {
                    const layerName = identifierEl.textContent;
                    layers.push(layerName);
                    
                    // Extract styles for this layer
                    const styles = [];
                    const styleElements = layerEl.querySelectorAll('Style > Identifier');
                    styleElements.forEach(styleEl => {
                        if (styleEl.textContent) {
                            styles.push({
                                name: styleEl.textContent,
                                title: styleEl.textContent
                            });
                        }
                    });
                    layerStyles[layerName] = styles;
                    
                    // Extract bounding box for WMTS (use WGS84BoundingBox)
                    const bboxEl = layerEl.querySelector('WGS84BoundingBox');
                    if (bboxEl) {
                        const lowerCorner = bboxEl.querySelector('LowerCorner');
                        const upperCorner = bboxEl.querySelector('UpperCorner');
                        if (lowerCorner && upperCorner) {
                            const [west, south] = lowerCorner.textContent.split(' ').map(Number);
                            const [east, north] = upperCorner.textContent.split(' ').map(Number);
                            layerBounds[layerName] = { west, south, east, north };
                        }
                    }
                }
            });
        } else {
            // WMS uses <Name> for queryable layers
            const layerElements = xml.querySelectorAll('Layer[queryable="1"]');
            layerElements.forEach(layerEl => {
                const nameEl = layerEl.querySelector('Name');
                if (nameEl && nameEl.textContent) {
                    const layerName = nameEl.textContent;
                    layers.push(layerName);
                    
                    // Extract styles for this layer
                    const styles = [];
                    const styleElements = layerEl.querySelectorAll('Style');
                    styleElements.forEach(styleEl => {
                        const styleName = styleEl.querySelector('Name');
                        const styleTitle = styleEl.querySelector('Title');
                        if (styleName && styleName.textContent) {
                            styles.push({
                                name: styleName.textContent,
                                title: styleTitle ? styleTitle.textContent : styleName.textContent
                            });
                        }
                    });
                    layerStyles[layerName] = styles;
                    
                    // Extract bounding box for WMS
                    const bboxEl = layerEl.querySelector('EX_GeographicBoundingBox');
                    if (bboxEl) {
                        const west = parseFloat(bboxEl.querySelector('westBoundLongitude')?.textContent || '-180');
                        const east = parseFloat(bboxEl.querySelector('eastBoundLongitude')?.textContent || '180');
                        const south = parseFloat(bboxEl.querySelector('southBoundLatitude')?.textContent || '-90');
                        const north = parseFloat(bboxEl.querySelector('northBoundLatitude')?.textContent || '90');
                        layerBounds[layerName] = { west, south, east, north };
                    }
                }
            });
        }

        availableLayers = layers.sort();
        
        // Populate layer select
        layerSelectEl.innerHTML = '<option value="">Select a layer...</option>';
        availableLayers.forEach(layerName => {
            const option = document.createElement('option');
            option.value = layerName;
            option.textContent = formatLayerName(layerName);
            layerSelectEl.appendChild(option);
        });
        
        console.log(`Loaded ${availableLayers.length} layers for ${service}`);
    } catch (error) {
        console.error('Failed to load available layers:', error);
        layerSelectEl.innerHTML = '<option value="">Error loading layers</option>';
    }
}

// Handle load layer button click
function onLoadLayer() {
    const layerName = layerSelectEl.value;
    
    if (!layerName) {
        alert('Please select a layer');
        return;
    }
    
    loadLayerOnMap(layerName);
}

// Format layer name for display
function formatLayerName(layerName) {
    const names = {
        'TMP': 'Temperature',
        'PRMSL': 'Mean Sea Level Pressure',
        'WIND': 'Wind Speed',
        'UGRD': 'U-Wind Component',
        'VGRD': 'V-Wind Component',
        'RH': 'Relative Humidity',
        'GUST': 'Wind Gust',
        'P1_22': 'Cloud Mixing Ratio',
        'P1_23': 'Ice Mixing Ratio',
        'P1_24': 'Rain Mixing Ratio',
        'WIND_BARBS': 'Wind Barbs'
    };
    
    // Extract parameter from layer name (e.g., "gfs_PRMSL" -> "PRMSL")
    const parts = layerName.split('_');
    if (parts.length >= 2) {
        const param = parts[1];
        for (const [key, name] of Object.entries(names)) {
            if (param.includes(key)) {
                return `${parts[0].toUpperCase()} - ${name}`;
            }
        }
        return `${parts[0].toUpperCase()} - ${param}`;
    }
    
    return layerName;
}

// Load a specific layer on the map
function loadLayerOnMap(layerName) {
    // Remove existing layer and cleanup event listeners
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
        map.off('moveend', onMapMoveEnd); // Remove moveend listener from previous WMS layer
    }

    // Store current layer and style
    selectedLayer = layerName;
    selectedStyle = styleSelectEl.value || 'default';

    // Reset performance tracking
    performanceStats.currentLayer = layerName;
    performanceStats.tileTimes = [];
    performanceStats.tilesLoaded = 0;
    performanceStats.layerStartTime = null;

    // Reset time mode to forecast (default) before loading
    // This prevents stale observation times from being used for forecast layers
    // The correct mode will be set by fetchAvailableTimes() after loading
    layerTimeMode = 'forecast';
    currentObservationTime = null;
    currentForecastHour = 0;

    if (selectedProtocol === 'wmts') {
        // Create WMTS layer using Leaflet TileLayer with direct URL pattern
        // WMTS tile URL format: /wmts/rest/{layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png?time=N&elevation=...
        const wmtsUrl = buildWmtsTileUrl(layerName, selectedStyle);
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(layerName)} (WMTS - ${selectedStyle})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7
        });
        
        // Hook into tile load events for performance tracking
        wmsLayer.on('loading', function() {
            performanceStats.layerStartTime = Date.now();
        });
        
        wmsLayer.on('load', function() {
            if (performanceStats.layerStartTime) {
                const loadTime = Date.now() - performanceStats.layerStartTime;
                trackTileLoadTime(loadTime);
            }
        });
        
        wmsLayer.on('tileerror', function(e) {
            console.error('Tile load error:', e);
        });
        
        wmsLayer.addTo(map);
        console.log('Loaded WMTS layer:', wmtsUrl);
    } else {
        // Create WMS layer using a single GetMap request for the entire viewport
        // This uses L.imageOverlay which we update on map move
        wmsLayer = createWmsImageOverlay(layerName, selectedStyle);
        wmsLayer.addTo(map);
        
        // Update the overlay when map moves
        map.on('moveend', onMapMoveEnd);
        
        console.log('Loaded WMS layer (single GetMap):', layerName, 'with style:', selectedStyle, 'time:', currentForecastHour);
    }

    // Update current layer display
    currentLayerNameEl.textContent = `${formatLayerName(layerName)} (${selectedProtocol.toUpperCase()})`;
    updatePerformanceDisplay();
    updateQueryHint();
    
    // Fetch available times and update time controls (this detects observation vs forecast mode)
    fetchAvailableTimes();
}

// Create a WMS image overlay that covers the current viewport
function createWmsImageOverlay(layerName, style) {
    const bounds = map.getBounds();
    const size = map.getSize();
    const url = buildWmsGetMapUrl(layerName, style, bounds, size);
    
    performanceStats.layerStartTime = Date.now();
    
    const overlay = L.imageOverlay(url, bounds, {
        opacity: 0.7,
        interactive: false,
        attribution: `${formatLayerName(layerName)} (WMS - ${style})`
    });
    
    // Track when image loads
    overlay.on('load', function() {
        if (performanceStats.layerStartTime) {
            const loadTime = Date.now() - performanceStats.layerStartTime;
            trackTileLoadTime(loadTime);
            console.log(`WMS GetMap loaded in ${loadTime}ms`);
        }
    });
    
    overlay.on('error', function(e) {
        console.error('WMS GetMap error:', e);
    });
    
    return overlay;
}

// Build WMTS tile URL with optional time/elevation dimensions
function buildWmtsTileUrl(layerName, style) {
    // Use WMTS KVP format which properly supports TIME and ELEVATION dimensions
    const params = [
        'SERVICE=WMTS',
        'REQUEST=GetTile',
        `LAYER=${layerName}`,
        `STYLE=${style}`,
        'TILEMATRIXSET=WebMercatorQuad',
        'TILEMATRIX={z}',
        'TILEROW={y}',
        'TILECOL={x}',
        'FORMAT=image/png'
    ];
    
    // Add TIME parameter based on layer mode
    if (layerTimeMode === 'observation') {
        // Observation layers use ISO8601 timestamp
        if (currentObservationTime) {
            params.push(`TIME=${encodeURIComponent(currentObservationTime)}`);
        }
    } else {
        // Forecast layers use forecast hour
        if (currentForecastHour !== undefined && currentForecastHour !== null) {
            params.push(`TIME=${currentForecastHour}`);
        }
    }
    
    if (currentElevation) {
        params.push(`ELEVATION=${encodeURIComponent(currentElevation)}`);
    }
    
    const url = `${API_BASE}/wmts?${params.join('&')}`;
    console.log('WMTS tile URL template:', url);
    return url;
}

// Build WMS GetMap URL for current viewport
function buildWmsGetMapUrl(layerName, style, bounds, size) {
    // Clamp longitude values to -180 to 180 range to handle Leaflet's world wrapping
    let west = bounds.getWest();
    let east = bounds.getEast();
    let south = bounds.getSouth();
    let north = bounds.getNorth();
    
    // Normalize longitudes to -180 to 180
    while (west < -180) west += 360;
    while (west > 180) west -= 360;
    while (east < -180) east += 360;
    while (east > 180) east -= 360;
    
    // Clamp latitudes to valid Web Mercator range
    south = Math.max(-85.051129, Math.min(85.051129, south));
    north = Math.max(-85.051129, Math.min(85.051129, north));
    
    // Convert lat/lon to Web Mercator coordinates for proper projection
    // Leaflet displays in Web Mercator, so we need to request in EPSG:3857
    const mercSouth = latToMercatorY(south);
    const mercNorth = latToMercatorY(north);
    const mercWest = lonToMercatorX(west);
    const mercEast = lonToMercatorX(east);
    
    const bbox = `${mercWest},${mercSouth},${mercEast},${mercNorth}`;
    console.log('WMS BBOX (EPSG:3857):', { west, south, east, north, mercWest, mercSouth, mercEast, mercNorth });
    
    // Determine TIME parameter based on layer mode
    let timeValue;
    if (layerTimeMode === 'observation') {
        // Observation layers use ISO8601 timestamp
        timeValue = currentObservationTime || '';
    } else {
        // Forecast layers use forecast hour
        timeValue = currentForecastHour.toString();
    }
    
    const params = new URLSearchParams({
        SERVICE: 'WMS',
        VERSION: '1.3.0',
        REQUEST: 'GetMap',
        LAYERS: layerName,
        STYLES: style || '',
        CRS: 'EPSG:3857',  // Use Web Mercator to match Leaflet's display projection
        BBOX: bbox,
        WIDTH: size.x,
        HEIGHT: size.y,
        FORMAT: 'image/png',
        TRANSPARENT: 'true',
        TIME: timeValue
    });
    
    // Add elevation if set
    if (currentElevation) {
        params.set('ELEVATION', currentElevation);
    }
    
    const url = `${API_BASE}/wms?${params.toString()}`;
    console.log('WMS GetMap URL:', url);
    return url;
}

// Convert latitude to Web Mercator Y coordinate
function latToMercatorY(lat) {
    const R = 6378137; // Earth radius in meters (WGS84)
    const latRad = lat * Math.PI / 180;
    return R * Math.log(Math.tan(Math.PI / 4 + latRad / 2));
}

// Convert longitude to Web Mercator X coordinate
function lonToMercatorX(lon) {
    const R = 6378137; // Earth radius in meters (WGS84)
    return R * lon * Math.PI / 180;
}

// Handle map move end - update the WMS overlay
function onMapMoveEnd() {
    if (selectedProtocol !== 'wms' || !selectedLayer || !wmsLayer) {
        return;
    }
    
    // Remove old overlay and create new one
    map.removeLayer(wmsLayer);
    wmsLayer = createWmsImageOverlay(selectedLayer, selectedStyle);
    wmsLayer.addTo(map);
}

function updateQueryHint() {
    const hint = document.getElementById('query-hint');
    if (hint) {
        hint.style.display = (selectedLayer && queryEnabled) ? 'block' : 'none';
    }
}



// Initialize ingestion status monitoring
function initIngestionStatus() {
    checkIngestionStatus();
    fetchBackendMetrics();
    fetchContainerStats();
    // Refresh ingestion status every 10 seconds
    ingestionStatusInterval = setInterval(() => {
        checkIngestionStatus();
    }, 10000);
    // Refresh backend metrics every 2 seconds
    setInterval(fetchBackendMetrics, 2000);
    // Refresh container stats every 5 seconds
    setInterval(fetchContainerStats, 5000);
}

// Check ingestion status from catalog database
async function checkIngestionStatus() {
    try {
        // For now, we'll simulate ingestion status
        // In a production system, this would fetch from the ingester API
        await updateIngestionMetrics();
    } catch (error) {
        console.error('Failed to check ingestion status:', error);
        setIngestionStatus('unknown');
    }
}

// Update ingestion metrics from WMS capabilities
async function updateIngestionMetrics() {
    try {
        const response = await fetch(
            `${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`,
            { mode: 'cors', cache: 'no-cache' }
        );
        
        if (!response.ok) {
            setIngestionStatus('offline');
            return;
        }

        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');
        
        // Count available layers
        const layers = xml.querySelectorAll('Layer[queryable="1"]');
        const datasetCount = layers.length;
        
        // Extract unique models from layer names (format: "gfs_TMP" -> "gfs")
        const models = new Set();
        layers.forEach(layer => {
            const name = getElementText(layer, 'Name');
            if (name) {
                // Split on underscore and take the first part as model name
                const model = name.split('_')[0];
                if (model) models.add(model);
            }
        });

        // Update UI
        setIngestionStatus('online');
        datasetCountEl.textContent = datasetCount;
        modelsListEl.textContent = Array.from(models).join(', ') || 'None';
        
        // Fetch actual storage stats from API
        fetchStorageStats();
        
        // Add log entry
        addLogEntry(`Found ${datasetCount} datasets, models: ${Array.from(models).join(', ')}`, 'success');
        
    } catch (error) {
        console.error('Error updating ingestion metrics:', error);
        addLogEntry(`Error: ${error.message}`, 'error');
        setIngestionStatus('offline');
    }
}

// Set ingestion service status
function setIngestionStatus(status) {
    const statusDot = ingesterServiceStatusEl.querySelector('.status-dot');
    const statusText = ingesterServiceStatusEl.querySelector('.status-text');
    
    statusDot.className = `status-dot ${status}`;
    statusText.textContent = status.charAt(0).toUpperCase() + status.slice(1);
}

// Add entry to ingestion log
function addLogEntry(message, type = 'info') {
    const timestamp = new Date().toLocaleTimeString();
    const entry = document.createElement('div');
    entry.className = `log-entry ${type}`;
    entry.textContent = `[${timestamp}] ${message}`;
    
    // Keep log to last 10 entries
    while (ingestLogEl.children.length >= 10) {
        ingestLogEl.removeChild(ingestLogEl.firstChild);
    }
    
    ingestLogEl.appendChild(entry);
    ingestLogEl.scrollTop = ingestLogEl.scrollHeight;
}

// Format bytes to human readable
function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
}

// Fetch storage stats from MinIO via API
async function fetchStorageStats() {
    try {
        const response = await fetch(`${API_BASE}/api/storage/stats`);
        if (response.ok) {
            const stats = await response.json();
            storageSizeEl.textContent = `${formatBytes(stats.total_size)} (${stats.object_count} files)`;
        }
    } catch (error) {
        console.error('Error fetching storage stats:', error);
        storageSizeEl.textContent = 'N/A';
    }
}

// Fetch backend metrics from API
async function fetchBackendMetrics() {
    try {
        const response = await fetch(`${API_BASE}/api/metrics`);
        if (!response.ok) return;
        
        const data = await response.json();
        const metrics = data.metrics || {};
        const system = data.system || {};
        const cache = data.cache || {};
        
        // Update request counts
        if (metricWmsRequestsEl) metricWmsRequestsEl.textContent = metrics.wms_requests ?? 0;
        if (metricWmtsRequestsEl) metricWmtsRequestsEl.textContent = metrics.wmts_requests ?? 0;
        
        // Update render stats
        if (metricRendersEl) {
            const errors = (metrics.render_errors ?? 0) > 0 ? ` (${metrics.render_errors} err)` : '';
            metricRendersEl.textContent = (metrics.renders_total ?? 0) + errors;
        }
        if (metricRenderAvgEl) {
            const avgMs = (metrics.render_avg_ms ?? 0).toFixed(0);
            metricRenderAvgEl.textContent = `${avgMs}ms`;
            metricRenderAvgEl.className = 'metric-value ' + getSpeedClass(metrics.render_avg_ms ?? 0);
        }
        if (metricRenderLastEl) {
            const lastMs = (metrics.render_last_ms ?? 0).toFixed(0);
            metricRenderLastEl.textContent = `${lastMs}ms`;
            metricRenderLastEl.className = 'metric-value ' + getSpeedClass(metrics.render_last_ms ?? 0);
        }
        
        // Update Redis cache stats
        if (metricCacheStatusEl) {
            const connected = cache.connected ?? false;
            metricCacheStatusEl.textContent = connected ? 'Connected' : 'Disconnected';
            metricCacheStatusEl.className = 'metric-value ' + (connected ? 'good' : 'bad');
        }
        if (metricCacheKeysEl) {
            metricCacheKeysEl.textContent = cache.key_count ?? '--';
        }
        if (metricCacheMemoryEl) {
            metricCacheMemoryEl.textContent = cache.memory_used ? formatBytes(cache.memory_used) : '--';
        }
        
        // Update system stats
        if (metricMemoryEl) {
            metricMemoryEl.textContent = system.memory_used_bytes ? formatBytes(system.memory_used_bytes) : '--';
        }
        if (metricThreadsEl) {
            metricThreadsEl.textContent = system.num_threads ?? '--';
        }
        if (metricUptimeEl) {
            metricUptimeEl.textContent = metrics.uptime_secs ? formatUptime(metrics.uptime_secs) : '--';
        }
    } catch (error) {
        console.error('Error fetching backend metrics:', error);
    }
}

// Get CSS class based on render speed
function getSpeedClass(ms) {
    if (ms < 200) return 'good';
    if (ms < 1000) return 'warning';
    return 'bad';
}

// Format uptime in human-readable format
function formatUptime(seconds) {
    if (seconds < 60) return `${seconds}s`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
    const hours = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    return `${hours}h ${mins}m`;
}

// Fetch container/pod resource stats
async function fetchContainerStats() {
    try {
        const response = await fetch(`${API_BASE}/api/container/stats`);
        if (!response.ok) return;
        
        const data = await response.json();
        updateContainerStatsUI(data);
    } catch (error) {
        console.error('Error fetching container stats:', error);
    }
}

// Update container stats UI
function updateContainerStatsUI(data) {
    const { container, memory, process, cpu } = data;
    
    // Memory stats
    const memUsedEl = document.getElementById('container-mem-used');
    const memLimitEl = document.getElementById('container-mem-limit');
    const memPercentEl = document.getElementById('container-mem-percent');
    const memBarEl = document.getElementById('container-mem-bar');
    
    if (memUsedEl) memUsedEl.textContent = formatBytes(memory.used_bytes);
    if (memLimitEl) {
        if (memory.cgroup_limit_bytes > 0) {
            memLimitEl.textContent = formatBytes(memory.cgroup_limit_bytes);
        } else {
            memLimitEl.textContent = formatBytes(memory.host_total_bytes) + ' (host)';
        }
    }
    if (memPercentEl) {
        memPercentEl.textContent = memory.percent_used.toFixed(1) + '%';
        // Color code based on usage
        memPercentEl.className = 'metric-value ' + getMemoryUsageClass(memory.percent_used);
    }
    if (memBarEl) {
        memBarEl.style.width = Math.min(memory.percent_used, 100) + '%';
        memBarEl.className = 'memory-bar ' + getMemoryUsageClass(memory.percent_used);
    }
    
    // Process stats
    const procRssEl = document.getElementById('container-proc-rss');
    const procVmsEl = document.getElementById('container-proc-vms');
    
    if (procRssEl) procRssEl.textContent = formatBytes(process.rss_bytes);
    if (procVmsEl) procVmsEl.textContent = formatBytes(process.vms_bytes);
    
    // CPU stats
    const cpuCountEl = document.getElementById('container-cpu-count');
    const load1mEl = document.getElementById('container-load-1m');
    const load5mEl = document.getElementById('container-load-5m');
    
    if (cpuCountEl) cpuCountEl.textContent = cpu.count;
    if (load1mEl) {
        load1mEl.textContent = cpu.load_1m.toFixed(2);
        load1mEl.className = 'metric-value ' + getLoadClass(cpu.load_1m, cpu.count);
    }
    if (load5mEl) {
        load5mEl.textContent = cpu.load_5m.toFixed(2);
        load5mEl.className = 'metric-value ' + getLoadClass(cpu.load_5m, cpu.count);
    }
    
    // Container info
    const containerTypeEl = document.getElementById('container-type');
    const hostnameEl = document.getElementById('container-hostname');
    
    if (containerTypeEl) {
        containerTypeEl.textContent = container.in_container ? 'Yes' : 'No (bare metal)';
    }
    if (hostnameEl) {
        hostnameEl.textContent = container.hostname;
        hostnameEl.title = container.hostname;
    }
}

// Get CSS class based on memory usage percentage
function getMemoryUsageClass(percent) {
    if (percent < 60) return 'good';
    if (percent < 80) return 'warning';
    return 'bad';
}

// Get CSS class based on CPU load relative to core count
function getLoadClass(load, cores) {
    const ratio = load / cores;
    if (ratio < 0.7) return 'good';
    if (ratio < 1.0) return 'warning';
    return 'bad';
}

// ============================================================================
// Performance Tracking
// ============================================================================

function trackTileLoadTime(time) {
    performanceStats.tileTimes.push(time);
    performanceStats.tilesLoaded++;
    
    // Keep only last 100 tiles in memory
    if (performanceStats.tileTimes.length > 100) {
        performanceStats.tileTimes.shift();
    }
    
    updatePerformanceDisplay();
}

function updatePerformanceDisplay() {
    // Last tile time
    if (performanceStats.tileTimes.length > 0) {
        const lastTime = performanceStats.tileTimes[performanceStats.tileTimes.length - 1];
        lastTileTimeEl.textContent = `${lastTime.toFixed(0)}ms`;
        lastTileTimeEl.parentElement.classList.remove('fast', 'medium', 'slow');
        if (lastTime < 500) {
            lastTileTimeEl.parentElement.classList.add('fast');
        } else if (lastTime < 1500) {
            lastTileTimeEl.parentElement.classList.add('medium');
        } else {
            lastTileTimeEl.parentElement.classList.add('slow');
        }
    }
    
    // Average time
    if (performanceStats.tileTimes.length > 0) {
        const avgTime = performanceStats.tileTimes.reduce((a, b) => a + b, 0) / performanceStats.tileTimes.length;
        avgTileTimeEl.textContent = `${avgTime.toFixed(0)}ms`;
        avgTileTimeEl.parentElement.classList.remove('fast', 'medium', 'slow');
        if (avgTime < 500) {
            avgTileTimeEl.parentElement.classList.add('fast');
        } else if (avgTime < 1500) {
            avgTileTimeEl.parentElement.classList.add('medium');
        } else {
            avgTileTimeEl.parentElement.classList.add('slow');
        }
    }
    
    // Slowest time
    if (performanceStats.tileTimes.length > 0) {
        const slowestTime = Math.max(...performanceStats.tileTimes);
        slowestTileTimeEl.textContent = `${slowestTime.toFixed(0)}ms`;
        slowestTileTimeEl.parentElement.classList.remove('fast', 'medium', 'slow');
        if (slowestTime < 500) {
            slowestTileTimeEl.parentElement.classList.add('fast');
        } else if (slowestTime < 1500) {
            slowestTileTimeEl.parentElement.classList.add('medium');
        } else {
            slowestTileTimeEl.parentElement.classList.add('slow');
        }
    }
    
    // Tiles loaded
    tilesLoadedCountEl.textContent = performanceStats.tilesLoaded;
    
    // Current layer
    if (performanceStats.currentLayer) {
        currentLayerNameEl.textContent = performanceStats.currentLayer;
    }
}

// Auto-refresh status every 30 seconds
setInterval(() => {
    checkServiceStatus();
}, 30000);

// ============================================================================
// GetFeatureInfo - Click-to-Query
// ============================================================================

function setupMapClickHandler() {
    map.on('click', onMapClick);
}

async function onMapClick(e) {
    if (!queryEnabled || !selectedLayer) {
        return;
    }
    
    // Show loading indicator
    const loadingPopup = L.popup()
        .setLatLng(e.latlng)
        .setContent('<div class="feature-info-loading">Querying data...</div>')
        .openOn(map);
    
    try {
        const featureInfo = await queryFeatureInfo(e.latlng);
        
        if (featureInfo && featureInfo.features && featureInfo.features.length > 0) {
            showFeatureInfoPopup(e.latlng, featureInfo);
        } else {
            L.popup()
                .setLatLng(e.latlng)
                .setContent('<div class="feature-info">No data available at this location</div>')
                .openOn(map);
        }
    } catch (error) {
        console.error('GetFeatureInfo failed:', error);
        L.popup()
            .setLatLng(e.latlng)
            .setContent(`<div class="feature-info-error">Query failed: ${error.message}</div>`)
            .openOn(map);
    }
}

async function queryFeatureInfo(latlng) {
    const bounds = map.getBounds();
    const size = map.getSize();
    const point = map.latLngToContainerPoint(latlng);
    
    // Build GetFeatureInfo URL
    const url = buildGetFeatureInfoUrl(
        selectedLayer,
        bounds,
        size.x,
        size.y,
        Math.round(point.x),
        Math.round(point.y)
    );
    
    console.log('GetFeatureInfo URL:', url);
    
    const response = await fetch(url, { mode: 'cors' });
    
    if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    
    return await response.json();
}

function buildGetFeatureInfoUrl(layer, bounds, width, height, x, y) {
    const sw = bounds.getSouthWest();
    const ne = bounds.getNorthEast();
    
    // Determine CRS based on protocol
    const crs = selectedProtocol === 'wmts' ? 'EPSG:3857' : 'EPSG:4326';
    
    // Convert bounds to bbox string based on CRS
    let bbox;
    if (crs === 'EPSG:3857') {
        // Convert lat/lng to Web Mercator
        const swMerc = latLngToWebMercator(sw.lat, sw.lng);
        const neMerc = latLngToWebMercator(ne.lat, ne.lng);
        bbox = `${swMerc.x},${swMerc.y},${neMerc.x},${neMerc.y}`;
    } else {
        bbox = `${sw.lng},${sw.lat},${ne.lng},${ne.lat}`;
    }
    
    // Determine TIME parameter based on layer mode
    let timeValue;
    if (layerTimeMode === 'observation') {
        timeValue = currentObservationTime || '';
    } else {
        timeValue = currentForecastHour.toString();
    }
    
    const params = new URLSearchParams({
        SERVICE: 'WMS',
        REQUEST: 'GetFeatureInfo',
        VERSION: '1.3.0',
        LAYERS: layer,
        QUERY_LAYERS: layer,
        STYLES: selectedStyle || 'default',
        CRS: crs,
        BBOX: bbox,
        WIDTH: width.toString(),
        HEIGHT: height.toString(),
        I: x.toString(),
        J: y.toString(),
        INFO_FORMAT: 'application/json',
        TIME: timeValue
    });
    
    // Add elevation if set
    if (currentElevation) {
        params.set('ELEVATION', currentElevation);
    }
    
    return `${API_BASE}/wms?${params.toString()}`;
}

function showFeatureInfoPopup(latlng, featureInfo) {
    let content = '<div class="feature-info-popup">';
    
    featureInfo.features.forEach(feature => {
        content += `<div class="feature-item">`;
        content += `<h4>${formatLayerName(feature.layer_name)}</h4>`;
        content += `<table>`;
        content += `<tr><td>Parameter:</td><td class="value">${feature.parameter}</td></tr>`;
        content += `<tr><td>Value:</td><td class="value">${feature.value.toFixed(2)} ${feature.unit}</td></tr>`;
        content += `<tr><td>Location:</td><td class="value">${feature.location.latitude.toFixed(3)}°, ${feature.location.longitude.toFixed(3)}°</td></tr>`;
        
        // Show level/elevation if available
        if (feature.level) {
            content += `<tr><td>Level:</td><td class="value">${feature.level}</td></tr>`;
        }
        
        if (feature.forecast_hour !== undefined) {
            content += `<tr><td>Forecast:</td><td class="value">+${feature.forecast_hour} hours</td></tr>`;
        }
        
        content += `</table>`;
        content += `</div>`;
    });
    
    content += '</div>';
    
    L.popup({ maxWidth: 300 })
        .setLatLng(latlng)
        .setContent(content)
        .openOn(map);
}

// Convert lat/lng to Web Mercator coordinates (EPSG:3857)
function latLngToWebMercator(lat, lng) {
    const x = lng * 20037508.34 / 180.0;
    const y = Math.log(Math.tan((90 + lat) * Math.PI / 360.0)) / (Math.PI / 180.0);
    const yMerc = y * 20037508.34 / 180.0;
    return { x: x, y: yMerc };
}

// Helper function to get text content from XML element
function getElementText(element, tagName) {
    const el = element.querySelector(tagName);
    return el ? el.textContent : null;
}

// ============================================================================
// Validation Status Functions
// ============================================================================

let validationInterval = null;

// Fetch and update validation status
async function checkValidationStatus() {
    try {
        const response = await fetch(`${API_BASE}/api/validation/status`);
        if (!response.ok) {
            console.error('Failed to fetch validation status:', response.statusText);
            return;
        }
        
        const data = await response.json();
        updateValidationUI(data);
    } catch (error) {
        console.error('Error fetching validation status:', error);
    }
}

// Run validation manually
async function runValidation() {
    const button = document.getElementById('run-validation-btn');
    const originalText = button.innerHTML;
    
    // Disable button and show loading state
    button.disabled = true;
    button.innerHTML = '⏳ Running...';
    
    try {
        const response = await fetch(`${API_BASE}/api/validation/run`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }
        
        const data = await response.json();
        updateValidationUI(data);
    } catch (error) {
        console.error('Error running validation:', error);
        alert(`Failed to run validation: ${error.message}`);
    } finally {
        // Re-enable button
        button.disabled = false;
        button.innerHTML = originalText;
    }
}

// Update validation UI with results
function updateValidationUI(data) {
    // Update WMS status
    updateServiceStatus('wms', data.wms);
    updateChecks('wms', data.wms.checks);
    
    // Update WMTS status
    updateServiceStatus('wmts', data.wmts);
    updateChecks('wmts', data.wmts.checks);
    
    // Update timestamp
    const timestamp = new Date(data.timestamp);
    const now = new Date();
    const diffMs = now - timestamp;
    const diffMins = Math.floor(diffMs / 60000);
    
    let timeAgo;
    if (diffMins < 1) {
        timeAgo = 'Just now';
    } else if (diffMins === 1) {
        timeAgo = '1 minute ago';
    } else if (diffMins < 60) {
        timeAgo = `${diffMins} minutes ago`;
    } else {
        const diffHours = Math.floor(diffMins / 60);
        timeAgo = diffHours === 1 ? '1 hour ago' : `${diffHours} hours ago`;
    }
    
    const lastChecked = document.getElementById('validation-last-checked');
    if (lastChecked) {
        lastChecked.textContent = `Last checked: ${timeAgo}`;
    }
}

// Update service status badge
function updateServiceStatus(service, serviceData) {
    const badge = document.getElementById(`${service}-validation-badge`);
    if (!badge) return;
    
    const dot = badge.querySelector('.status-dot');
    const text = badge.querySelector('.status-text');
    
    // Remove all status classes
    badge.classList.remove('status-compliant', 'status-non-compliant', 'status-partial', 'status-checking');
    
    if (serviceData.status === 'compliant') {
        badge.classList.add('status-compliant');
        dot.style.backgroundColor = '#22c55e';
        text.textContent = 'Compliant';
    } else if (serviceData.status === 'non-compliant') {
        badge.classList.add('status-non-compliant');
        dot.style.backgroundColor = '#ef4444';
        text.textContent = 'Non-Compliant';
    } else {
        badge.classList.add('status-partial');
        dot.style.backgroundColor = '#f59e0b';
        text.textContent = 'Partial';
    }
}

// Update individual checks
function updateChecks(service, checks) {
    const container = document.getElementById(`${service}-checks`);
    if (!container) return;
    
    const checkItems = container.querySelectorAll('.check-item');
    const checkNames = ['capabilities', 'getmap', 'getfeatureinfo', 'exceptions', 'crs_support'];
    const wmtsCheckNames = ['capabilities', 'gettile_rest', 'gettile_kvp', 'tilematrixset'];
    
    const names = service === 'wms' ? checkNames : wmtsCheckNames;
    
    names.forEach((name, index) => {
        const check = checks[name];
        if (!check || index >= checkItems.length) return;
        
        const item = checkItems[index];
        const icon = item.querySelector('.check-icon');
        
        if (check.status === 'pass') {
            icon.textContent = '✓';
            icon.style.color = '#22c55e';
            item.title = check.message;
        } else if (check.status === 'fail') {
            icon.textContent = '✗';
            icon.style.color = '#ef4444';
            item.title = check.message;
        } else {
            icon.textContent = '⊘';
            icon.style.color = '#94a3b8';
            item.title = check.message;
        }
    });
}

// Start auto-refresh for validation status
function startValidationAutoRefresh() {
    // Check immediately
    checkValidationStatus();
    
    // Then check every 5 minutes
    validationInterval = setInterval(checkValidationStatus, 5 * 60 * 1000);
}

// Stop auto-refresh
function stopValidationAutoRefresh() {
    if (validationInterval) {
        clearInterval(validationInterval);
        validationInterval = null;
    }
}

// Add event listener for manual validation button
document.addEventListener('DOMContentLoaded', () => {
    const runButton = document.getElementById('run-validation-btn');
    if (runButton) {
        runButton.addEventListener('click', runValidation);
    }
    
    // Start auto-refresh
    startValidationAutoRefresh();
});

// Clean up on page unload
window.addEventListener('beforeunload', () => {
    stopValidationAutoRefresh();
    stopPlayback();
});

// ============================================================================
// Time Control Functions
// ============================================================================

// Update time display
function updateTimeDisplay() {
    if (layerTimeMode === 'observation') {
        // Observation mode display
        const obsTimeDisplay = document.getElementById('current-obs-time-display');
        const obsCountDisplay = document.getElementById('obs-time-count');
        
        if (obsTimeDisplay) {
            if (currentObservationTime) {
                // Format ISO8601 timestamp for display
                const date = new Date(currentObservationTime);
                const formatted = date.toISOString().replace('T', ' ').substring(0, 19) + 'Z';
                obsTimeDisplay.textContent = formatted;
            } else {
                obsTimeDisplay.textContent = '--';
            }
        }
        
        if (obsCountDisplay) {
            obsCountDisplay.textContent = `${availableObservationTimes.length} times`;
        }
    } else {
        // Forecast mode display
        const timeDisplay = document.getElementById('current-time-display');
        const validTimeDisplay = document.getElementById('valid-time-display');
        
        if (timeDisplay) {
            // Format as F+000, F+003, etc. (standard meteorological notation)
            const formatted = `F+${String(currentForecastHour).padStart(3, '0')}`;
            timeDisplay.textContent = formatted;
            timeDisplay.style.opacity = '1';
        }
        
        // Calculate and display valid time (run + forecast hour)
        if (validTimeDisplay) {
            const runTime = currentRun === 'latest' ? 
                (availableRuns.length > 0 ? availableRuns[0] : null) : 
                currentRun;
            
            if (runTime) {
                const runDate = new Date(runTime);
                const validDate = new Date(runDate.getTime() + currentForecastHour * 60 * 60 * 1000);
                
                // Format as: "2025-11-27 20:50Z"
                const formatted = validDate.toISOString().replace('T', ' ').substring(0, 16) + 'Z';
                validTimeDisplay.textContent = formatted;
            } else {
                validTimeDisplay.textContent = '--';
            }
        }
    }
}

// Update layer with new time/elevation (without refetching capabilities)
function updateLayerTime() {
    if (!selectedLayer || !wmsLayer) {
        return;
    }
    
    // Remove existing layer
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
        map.off('moveend', onMapMoveEnd);
    }
    
    // Recreate the layer with current time/elevation settings
    if (selectedProtocol === 'wmts') {
        const wmtsUrl = buildWmtsTileUrl(selectedLayer, selectedStyle);
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(selectedLayer)} (WMTS - ${selectedStyle})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7
        });
        wmsLayer.addTo(map);
    } else {
        wmsLayer = createWmsImageOverlay(selectedLayer, selectedStyle);
        wmsLayer.addTo(map);
        map.on('moveend', onMapMoveEnd);
    }
    
    // Update displays
    updateTimeDisplay();
    
    // Log appropriate time info based on mode
    if (layerTimeMode === 'observation') {
        console.log('Layer updated with TIME (observation):', currentObservationTime, 'ELEVATION:', currentElevation || 'default');
    } else {
        console.log('Layer updated with TIME (forecast):', currentForecastHour, 'ELEVATION:', currentElevation || 'default');
    }
}



// Show/hide time controls based on protocol and layer time mode
function updateTimeControlsVisibility() {
    const timeControlSection = document.getElementById('time-control-section');
    const forecastControls = document.getElementById('forecast-controls');
    const observationControls = document.getElementById('observation-controls');
    const sectionTitle = document.getElementById('time-section-title');
    
    if (timeControlSection) {
        // Show time controls when a layer is selected
        if (selectedLayer) {
            timeControlSection.style.display = 'block';
            
            // Toggle between forecast and observation controls based on layer type
            if (layerTimeMode === 'observation') {
                // Observation mode (GOES, MRMS)
                if (forecastControls) forecastControls.style.display = 'none';
                if (observationControls) observationControls.style.display = 'block';
                if (sectionTitle) sectionTitle.textContent = 'Observation Time';
            } else {
                // Forecast mode (GFS, HRRR)
                if (forecastControls) forecastControls.style.display = 'block';
                if (observationControls) observationControls.style.display = 'none';
                if (sectionTitle) sectionTitle.textContent = 'Model Run & Forecast';
            }
            
            updateTimeDisplay();
        } else {
            timeControlSection.style.display = 'none';
            stopPlayback();
        }
    }
}

// Fetch available runs, forecast hours, and elevations from capabilities
async function fetchAvailableTimes() {
    try {
        const response = await fetch(`${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`);
        const xml = await response.text();
        const parser = new DOMParser();
        const doc = parser.parseFromString(xml, 'text/xml');
        
        // Reset state
        availableElevations = [];
        availableObservationTimes = [];
        layerTimeMode = 'forecast'; // Default to forecast mode
        
        // Find the current layer
        const layers = doc.getElementsByTagName('Layer');
        for (let layer of layers) {
            const nameEl = layer.getElementsByTagName('Name')[0];
            if (nameEl && nameEl.textContent === selectedLayer) {
                // Get dimensions
                const dimensions = layer.getElementsByTagName('Dimension');
                let hasTimeDimension = false;
                let hasRunDimension = false;
                
                for (let dim of dimensions) {
                    const dimName = dim.getAttribute('name');
                    
                    if (dimName === 'TIME') {
                        // Observational data - single TIME dimension with ISO8601 timestamps
                        hasTimeDimension = true;
                        const timeText = dim.textContent.trim();
                        availableObservationTimes = timeText.split(',').filter(t => t && t !== 'latest');
                        // Set default to latest observation time
                        currentObservationTime = availableObservationTimes.length > 0 
                            ? availableObservationTimes[0] 
                            : null;
                        console.log('Layer has TIME dimension (observation mode):', availableObservationTimes.length, 'times');
                    }
                    if (dimName === 'RUN') {
                        hasRunDimension = true;
                        const runsText = dim.textContent.trim();
                        availableRuns = runsText.split(',');
                        updateRunSelector();
                    }
                    if (dimName === 'FORECAST') {
                        const forecastText = dim.textContent.trim();
                        // Parse forecast hours - handle both plain integers and ISO 8601 duration format (PT0H, PT3H, etc.)
                        availableForecastHours = forecastText.split(',').map(h => {
                            h = h.trim();
                            // Check for ISO 8601 duration format (e.g., PT0H, PT3H, PT12H)
                            const isoMatch = h.match(/^PT(\d+)H$/i);
                            if (isoMatch) {
                                return parseInt(isoMatch[1]);
                            }
                            // Try plain integer
                            const parsed = parseInt(h);
                            return isNaN(parsed) ? 0 : parsed;
                        }).filter(h => !isNaN(h));
                        updateForecastSlider();
                    }
                    if (dimName === 'ELEVATION') {
                        const elevationText = dim.textContent.trim();
                        availableElevations = elevationText.split(',');
                        updateElevationSelector();
                    }
                }
                
                // Determine layer time mode
                if (hasTimeDimension && !hasRunDimension) {
                    layerTimeMode = 'observation';
                    updateObservationTimeSelector();
                } else {
                    layerTimeMode = 'forecast';
                }
                
                console.log('Layer time mode:', layerTimeMode);
                break;
            }
        }
        
        // Update UI visibility based on time mode
        updateTimeControlsVisibility();
        updateElevationVisibility();
        
        // Reload the layer with the correct time parameters now that we know the time mode
        updateLayerTime();
        
    } catch (error) {
        console.error('Failed to fetch available times:', error);
    }
}

// Update run selector dropdown
function updateRunSelector() {
    const runSelect = document.getElementById('run-select');
    if (!runSelect) return;
    
    runSelect.innerHTML = '<option value="latest">Latest Available Run</option>';
    
    availableRuns.forEach((run, index) => {
        const option = document.createElement('option');
        option.value = run;
        const date = new Date(run);
        
        // Format as: "2025-11-26 20:50Z" (meteorological standard)
        const formatted = date.toISOString().replace('T', ' ').substring(0, 16) + 'Z';
        
        // Add label for latest run
        const label = index === 0 ? ' (Latest)' : '';
        option.textContent = formatted + label;
        runSelect.appendChild(option);
    });
}

// Update forecast hour slider
// Uses index-based slider for non-uniform forecast hour intervals
function updateForecastSlider() {
    const timeSlider = document.getElementById('time-slider');
    if (!timeSlider || availableForecastHours.length === 0) return;
    
    // Sort forecast hours
    availableForecastHours.sort((a, b) => a - b);
    
    // Use index-based slider (0 to length-1) for non-uniform intervals
    timeSlider.min = 0;
    timeSlider.max = availableForecastHours.length - 1;
    timeSlider.step = 1;
    
    // Find index of current forecast hour, or default to last (latest)
    let currentIndex = availableForecastHours.indexOf(currentForecastHour);
    if (currentIndex === -1) {
        // Current hour not available, default to latest
        currentIndex = availableForecastHours.length - 1;
        currentForecastHour = availableForecastHours[currentIndex];
    }
    
    timeSlider.value = currentIndex;
    
    updateTimeDisplay();
    updateSliderLabels();
}

// Update slider labels to match available hours
function updateSliderLabels() {
    const labelsContainer = document.querySelector('.slider-labels');
    if (!labelsContainer) return;
    
    labelsContainer.innerHTML = '';
    
    // Show 5 evenly distributed labels in F+NNN format
    const numLabels = Math.min(5, availableForecastHours.length);
    const step = Math.floor(availableForecastHours.length / (numLabels - 1));
    
    for (let i = 0; i < numLabels; i++) {
        const idx = i === numLabels - 1 ? availableForecastHours.length - 1 : i * step;
        const hour = availableForecastHours[idx];
        const label = document.createElement('span');
        label.textContent = `F+${String(hour).padStart(3, '0')}`;
        labelsContainer.appendChild(label);
    }
}

// Update elevation selector dropdown
function updateElevationSelector() {
    const elevationSelect = document.getElementById('elevation-select');
    if (!elevationSelect) return;
    
    // Clear and rebuild options
    elevationSelect.innerHTML = '<option value="">Surface / Default</option>';
    
    availableElevations.forEach(level => {
        const option = document.createElement('option');
        option.value = level;
        option.textContent = level;
        elevationSelect.appendChild(option);
    });
    
    // Restore current selection if still available, otherwise reset
    if (currentElevation && availableElevations.includes(currentElevation)) {
        elevationSelect.value = currentElevation;
    } else {
        currentElevation = '';
        elevationSelect.value = '';
    }
    
    console.log('Elevation selector updated:', availableElevations.length, 'levels, current:', currentElevation || 'default');
}

// Show/hide elevation selector based on available levels
function updateElevationVisibility() {
    const elevationGroup = document.getElementById('elevation-group');
    if (!elevationGroup) return;
    
    // Show only if there are multiple elevations available
    if (availableElevations.length > 1) {
        elevationGroup.style.display = 'block';
    } else {
        elevationGroup.style.display = 'none';
        currentElevation = ''; // Reset to default
    }
}

// Update observation time selector dropdown
function updateObservationTimeSelector() {
    const obsTimeSelect = document.getElementById('observation-time-select');
    if (!obsTimeSelect) return;
    
    obsTimeSelect.innerHTML = '';
    
    if (availableObservationTimes.length === 0) {
        const option = document.createElement('option');
        option.value = '';
        option.textContent = 'No times available';
        obsTimeSelect.appendChild(option);
        return;
    }
    
    // Sort times descending (most recent first)
    const sortedTimes = [...availableObservationTimes].sort((a, b) => {
        return new Date(b) - new Date(a);
    });
    
    sortedTimes.forEach((time, index) => {
        const option = document.createElement('option');
        option.value = time;
        
        // Format for display: "2025-11-27 14:30:00Z (Latest)"
        const date = new Date(time);
        const formatted = date.toISOString().replace('T', ' ').substring(0, 19) + 'Z';
        const label = index === 0 ? ' (Latest)' : '';
        option.textContent = formatted + label;
        
        obsTimeSelect.appendChild(option);
    });
    
    // Select the first (latest) time by default
    if (sortedTimes.length > 0 && !currentObservationTime) {
        currentObservationTime = sortedTimes[0];
    }
    
    // Set the current selection
    if (currentObservationTime) {
        obsTimeSelect.value = currentObservationTime;
    }
    
    console.log('Observation time selector updated:', availableObservationTimes.length, 'times, current:', currentObservationTime);
}

// Handle observation time selection change
function onObservationTimeChange(e) {
    currentObservationTime = e.target.value;
    console.log('Observation time changed to:', currentObservationTime);
    updateTimeDisplay();
    updateLayerTime();
}

// Stop any active playback animation
function stopPlayback() {
    if (playbackInterval) {
        clearInterval(playbackInterval);
        playbackInterval = null;
    }
    isPlaying = false;
}
