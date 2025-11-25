// WMS Dashboard Application

const API_BASE = 'http://localhost:8080';
let map;
let wmsLayer = null;
let selectedLayer = null;

// DOM Elements
const wmsStatusEl = document.getElementById('wms-status');
const wmtsStatusEl = document.getElementById('wmts-status');
const layersListEl = document.getElementById('layers-list');
const layerDetailsEl = document.getElementById('layer-details');

// Initialize the application
document.addEventListener('DOMContentLoaded', () => {
    initMap();
    checkServiceStatus();
    loadCapabilities();
});

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
    // Remove existing WMS layer
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
    }

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
        attribution: `Layer: ${layer.title}`,
        opacity: 0.7
    });

    wmsLayer.addTo(map);
}

// Utility function to escape HTML
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// Auto-refresh status every 30 seconds
setInterval(() => {
    checkServiceStatus();
}, 30000);
