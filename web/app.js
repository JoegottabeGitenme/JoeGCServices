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
const layersListEl = document.getElementById('layers-list');
const layerDetailsEl = document.getElementById('layer-details');
const ingesterServiceStatusEl = document.getElementById('ingester-service-status');
const lastIngestTimeEl = document.getElementById('last-ingest-time');
const datasetCountEl = document.getElementById('dataset-count');
const modelsListEl = document.getElementById('models-list');
const storageSizeEl = document.getElementById('storage-size');
const ingestLogEl = document.getElementById('ingest-log');

// Layer Selection Elements
const protocolSelectEl = document.getElementById('protocol-select');
const layerSelectEl = document.getElementById('layer-select');
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
let selectedProtocol = 'wmts';

// Initialize the application
document.addEventListener('DOMContentLoaded', () => {
    initMap();
    checkServiceStatus();
    loadCapabilities();
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
    selectedLayer = layerSelectEl.value;
    console.log('Layer changed to:', selectedLayer);
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

        // Extract layer names
        const layers = [];
        const layerElements = xml.querySelectorAll('Layer > Name');
        layerElements.forEach(el => {
            const name = el.textContent;
            if (name) {
                layers.push(name);
            }
        });

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

// Load WMS/WMTS capabilities
async function loadCapabilities() {
    try {
        const response = await fetch(
            `${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`,
            { mode: 'cors' }
        );
        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');

        // Parse capabilities
        const layers = parseCapabilities(xml);
        displayLayers(layers);
    } catch (error) {
        console.error('Failed to load capabilities:', error);
        layersListEl.innerHTML = '<p class="empty-state">Failed to load layers</p>';
    }
}

// Parse WMS capabilities XML
function parseCapabilities(xml) {
    const layers = [];
    const layerElements = xml.querySelectorAll('Layer');

    layerElements.forEach((layerEl, index) => {
        const name = getElementText(layerEl, 'Name');
        const title = getElementText(layerEl, 'Title');
        const abstract = getElementText(layerEl, 'Abstract');

        // Parse dimensions (like TIME)
        const dimensions = [];
        const dimensionElements = layerEl.querySelectorAll('Dimension');
        dimensionElements.forEach(dimEl => {
            const name = dimEl.getAttribute('name');
            const value = dimEl.textContent;
            if (name && value) {
                dimensions.push({
                    name: name.toUpperCase(),
                    value: value,
                    default: dimEl.getAttribute('default') || ''
                });
            }
        });

        // Parse extent
        const extent = [];
        const extentElements = layerEl.querySelectorAll('Extent');
        extentElements.forEach(extEl => {
            const name = extEl.getAttribute('name');
            const value = extEl.textContent;
            if (name && value) {
                extent.push({
                    name: name.toUpperCase(),
                    value: value
                });
            }
        });

        // Only add layers that have a name
        if (name) {
            layers.push({
                name: name,
                title: title || name,
                abstract: abstract || 'No description available',
                dimensions: dimensions,
                extent: extent,
                queryable: layerEl.getAttribute('queryable') === '1'
            });
        }
    });

    return layers;
}

// Get text content from XML element
function getElementText(parent, tagName) {
    const element = parent.querySelector(tagName);
    return element ? element.textContent : '';
}

// Load a specific layer on the map
function loadLayerOnMap(layerName) {
    // Remove existing WMS layer
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
    }

    // Reset performance tracking
    performanceStats.currentLayer = layerName;
    performanceStats.tileTimes = [];
    performanceStats.tilesLoaded = 0;
    performanceStats.layerStartTime = null;

    if (selectedProtocol === 'wmts') {
        // Create WMTS layer using Leaflet TileLayer with direct URL pattern
        // WMTS tile URL format: /wmts/rest/{layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png
        const wmtsUrl = `${API_BASE}/wmts/rest/${layerName}/default/WebMercatorQuad/{z}/{x}/{y}.png`;
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(layerName)} (WMTS)`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7
        });
        
        console.log('Loaded WMTS layer:', wmtsUrl);
    } else {
        // Create WMS layer
        wmsLayer = L.tileLayer.wms(`${API_BASE}/wms`, {
            layers: layerName,
            styles: 'default',
            format: 'image/png',
            transparent: true,
            attribution: `${formatLayerName(layerName)} (WMS)`,
            version: '1.3.0',
            opacity: 0.7
        });
        
        console.log('Loaded WMS layer:', layerName);
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

// Display layers in sidebar
function displayLayers(layers) {
    if (layers.length === 0) {
        layersListEl.innerHTML = '<p class="empty-state">No layers available</p>';
        return;
    }

    layersListEl.innerHTML = layers.map((layer, index) => `
        <div class="layer-item" data-index="${index}" onclick="selectLayer(${index}, this)">
            <span class="layer-name">${escapeHtml(layer.title)}</span>
            <span class="layer-title">${escapeHtml(layer.name)}</span>
        </div>
    `).join('');

    // Auto-select first layer
    if (layers.length > 0) {
        selectLayer(0, layersListEl.querySelector('.layer-item'));
    }
}

// Select a layer and show details
function selectLayer(index, element) {
    // Update active state
    document.querySelectorAll('.layer-item').forEach(el => {
        el.classList.remove('active');
    });
    element.classList.add('active');

    // Load capabilities again to get full data
    fetch(`${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`, { mode: 'cors' })
        .then(r => r.text())
        .then(text => {
            const parser = new DOMParser();
            const xml = parser.parseFromString(text, 'text/xml');
            const layers = parseCapabilities(xml);
            
            if (layers[index]) {
                selectedLayer = layers[index];
                displayLayerDetails(layers[index]);
                addWmsLayerToMap(layers[index]);
            }
        })
        .catch(error => console.error('Failed to load layer details:', error));
}

// Display layer details in sidebar
function displayLayerDetails(layer) {
    let html = `
        <div class="detail-row">
            <div class="detail-label">Name</div>
            <div class="detail-value">${escapeHtml(layer.name)}</div>
        </div>
        <div class="detail-row">
            <div class="detail-label">Title</div>
            <div class="detail-value">${escapeHtml(layer.title)}</div>
        </div>
        <div class="detail-row">
            <div class="detail-label">Description</div>
            <div class="detail-value">${escapeHtml(layer.abstract)}</div>
        </div>
    `;

    if (layer.dimensions.length > 0) {
        html += `
            <div class="detail-row">
                <div class="detail-label">Dimensions</div>
                <div class="dimensions-list">
                    ${layer.dimensions.map(dim => `
                        <div class="dimension-item">
                            <strong>${escapeHtml(dim.name)}:</strong> ${escapeHtml(dim.value.substring(0, 100))}${dim.value.length > 100 ? '...' : ''}
                            ${dim.default ? `<br><small>Default: ${escapeHtml(dim.default)}</small>` : ''}
                        </div>
                    `).join('')}
                </div>
            </div>
        `;
    }

    if (layer.extent.length > 0) {
        html += `
            <div class="detail-row">
                <div class="detail-label">Extent</div>
                <div class="dimensions-list">
                    ${layer.extent.map(ext => `
                        <div class="dimension-item">
                            <strong>${escapeHtml(ext.name)}:</strong> ${escapeHtml(ext.value.substring(0, 100))}${ext.value.length > 100 ? '...' : ''}
                        </div>
                    `).join('')}
                </div>
            </div>
        `;
    }

    html += `
        <div class="detail-row">
            <div class="detail-label">Queryable</div>
            <div class="detail-value">${layer.queryable ? 'Yes' : 'No'}</div>
        </div>
    `;

    layerDetailsEl.innerHTML = html;
}

// Add WMS layer to map
function addWmsLayerToMap(layer) {
    // Reset performance tracking for new layer
    performanceStats.currentLayer = layer.name;
    performanceStats.tileTimes = [];
    performanceStats.tilesLoaded = 0;
    updatePerformanceDisplay();

    // Remove existing WMS layer
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
    }

    if (selectedProtocol === 'wmts') {
        // Create WMTS layer using Leaflet TileLayer with direct URL pattern
        const wmtsUrl = `${API_BASE}/wmts/rest/${layer.name}/default/WebMercatorQuad/{z}/{x}/{y}.png`;
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `Layer: ${layer.title} (WMTS)`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7
        });
        
        console.log('Loaded WMTS layer:', wmtsUrl);
    } else {
        // Create WMS layer
        const wmsUrl = `${API_BASE}/wms`;
        const params = {
            'SERVICE': 'WMS',
            'VERSION': '1.3.0',
            'REQUEST': 'GetMap',
            'LAYERS': layer.name,
            'STYLES': '',
            'FORMAT': 'image/png',
            'BBOX': '{bbox}',
            'WIDTH': '{width}',
            'HEIGHT': '{height}',
            'CRS': 'EPSG:4326',
            'TRANSPARENT': 'true'
        };

        // Add TIME dimension if available
        if (layer.dimensions.some(d => d.name === 'TIME') && layer.dimensions[0].default) {
            params['TIME'] = layer.dimensions[0].default;
        }

        wmsLayer = L.tileLayer.wms(wmsUrl, {
            layers: layer.name,
            format: 'image/png',
            transparent: true,
            attribution: `Layer: ${layer.title} (WMS)`,
            opacity: 0.7
        });
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
    
    wmsLayer.on('tileerror', function() {
        if (performanceStats.layerStartTime) {
            const loadTime = Date.now() - performanceStats.layerStartTime;
            trackTileLoadTime(loadTime);
        }
    });

    wmsLayer.addTo(map);
}

// Utility function to escape HTML
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
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
