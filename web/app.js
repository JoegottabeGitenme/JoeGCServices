// WMS Dashboard Application

const API_BASE = 'http://localhost:8080';
const REDIS_URL = 'http://localhost:8080/api/ingestion'; // Placeholder - would need API endpoint
let map;
let wmsLayer = null;
let selectedLayer = null;
let ingestionStatusInterval = null;

// DOM Elements
const wmsStatusEl = document.getElementById('wms-status');
const wmtsStatusEl = document.getElementById('wmts-status');
const ingesterServiceStatusEl = document.getElementById('ingester-service-status');
const lastIngestTimeEl = document.getElementById('last-ingest-time');
const datasetCountEl = document.getElementById('dataset-count');
const modelsListEl = document.getElementById('models-list');
const storageSizeEl = document.getElementById('storage-size');
const ingestLogEl = document.getElementById('ingest-log');

// Layer Selection Elements
const protocolSelectEl = document.getElementById('protocol-select');
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

// State for layer selection
let availableLayers = [];
let layerStyles = {}; // Map of layer name -> array of styles
let selectedProtocol = 'wmts';

// Initialize the application
document.addEventListener('DOMContentLoaded', () => {
    initMap();
    checkServiceStatus();
    loadAvailableLayers();
    initIngestionStatus();
    setupEventListeners();
});

// Setup event listeners for protocol and layer selection
function setupEventListeners() {
    protocolSelectEl.addEventListener('change', onProtocolChange);
    layerSelectEl.addEventListener('change', onLayerChange);
    loadLayerBtnEl.addEventListener('click', onLoadLayer);
}

// Handle protocol selection change
function onProtocolChange() {
    selectedProtocol = protocolSelectEl.value;
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

        // Extract layer names and styles
        const layers = [];
        layerStyles = {}; // Reset styles map
        
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
        'PRMSL': 'Pressure (MSL)',
        'WIND': 'Wind Speed',
        'UGRD': 'U-Wind Component',
        'VGRD': 'V-Wind Component',
        'RH': 'Relative Humidity',
        'GUST': 'Wind Gust'
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
    // Remove existing WMS layer
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
    }

    // Get selected style (or default)
    const selectedStyle = styleSelectEl.value || 'default';

    // Reset performance tracking
    performanceStats.currentLayer = layerName;
    performanceStats.tileTimes = [];
    performanceStats.tilesLoaded = 0;
    performanceStats.layerStartTime = null;

    if (selectedProtocol === 'wmts') {
        // Create WMTS layer using Leaflet TileLayer with direct URL pattern
        // WMTS tile URL format: /wmts/rest/{layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png
        const wmtsUrl = `${API_BASE}/wmts/rest/${layerName}/${selectedStyle}/WebMercatorQuad/{z}/{x}/{y}.png`;
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(layerName)} (WMTS - ${selectedStyle})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7
        });
        
        console.log('Loaded WMTS layer:', wmtsUrl);
    } else {
        // Create WMS layer
        wmsLayer = L.tileLayer.wms(`${API_BASE}/wms`, {
            layers: layerName,
            styles: selectedStyle,
            format: 'image/png',
            transparent: true,
            attribution: `${formatLayerName(layerName)} (WMS - ${selectedStyle})`,
            version: '1.3.0',
            opacity: 0.7
        });
        
        console.log('Loaded WMS layer:', layerName, 'with style:', selectedStyle);
    }

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
        if (performanceStats.layerStartTime) {
            const loadTime = Date.now() - performanceStats.layerStartTime;
            trackTileLoadTime(loadTime);
        }
    });

    wmsLayer.addTo(map);

    // Update current layer display
    currentLayerNameEl.textContent = `${formatLayerName(layerName)} (${selectedProtocol.toUpperCase()})`;
    updatePerformanceDisplay();
}



// Initialize ingestion status monitoring
function initIngestionStatus() {
    checkIngestionStatus();
    // Refresh ingestion status every 10 seconds
    ingestionStatusInterval = setInterval(() => {
        checkIngestionStatus();
    }, 10000);
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
        
        // Extract unique models from layer names
        const models = new Set();
        layers.forEach(layer => {
            const name = getElementText(layer, 'Name');
            if (name) {
                const model = name.split(':')[0];
                if (model) models.add(model);
            }
        });

        // Update UI
        setIngestionStatus('online');
        datasetCountEl.textContent = datasetCount;
        modelsListEl.textContent = Array.from(models).join(', ') || 'None';
        
        // Update timestamp
        const now = new Date();
        lastIngestTimeEl.textContent = now.toLocaleTimeString();
        
        // Simulate storage size (would come from actual API in production)
        if (datasetCount > 0) {
            const estimatedSize = (datasetCount * 100).toFixed(0);
            storageSizeEl.textContent = formatBytes(estimatedSize);
        }
        
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
