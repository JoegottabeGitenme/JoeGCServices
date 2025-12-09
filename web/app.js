// WMS Dashboard Application

// Smart API URL detection:
// - localhost:8000 (docker-compose) -> use localhost:8080
// - Otherwise (K8s ingress) -> use relative URLs (same origin)
const API_BASE = window.location.port === '8000' ? 'http://localhost:8080' : '';
const REDIS_URL = `${API_BASE}/api/ingestion`;

// External service URLs - can be customized for different environments
const EXTERNAL_URLS = {
    minio: 'http://localhost:9001',
    grafana: 'http://localhost:3000',
    prometheus: 'http://localhost:9090',
    k8sDashboard: 'http://localhost:8001/api/v1/namespaces/kubernetes-dashboard/services/http:kubernetes-dashboard:/proxy/',
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
let playbackSpeed = 1; // 0.5 = slow, 1 = medium, 2 = fast, 3 = very fast
const PLAYBACK_SPEEDS = [0.5, 1, 2, 3];
const SPEED_LABELS = ['0.5x', '1x', '2x', '3x'];
let layerControl = null; // Leaflet layer control
let preloadedLayers = {}; // Cache of preloaded tile layers for animation

// DOM Elements
const wmsStatusEl = document.getElementById('wms-status');
const wmtsStatusEl = document.getElementById('wmts-status');
const ingesterServiceStatusEl = document.getElementById('ingester-service-status');
const datasetCountEl = document.getElementById('dataset-count');
const modelsListEl = document.getElementById('models-list');
const storageSizeEl = document.getElementById('storage-size');



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
let layerTitles = {}; // Map of layer name -> human-readable title from capabilities
let selectedProtocol = 'wmts';
let selectedStyle = 'default';


// Initialize the application
document.addEventListener('DOMContentLoaded', () => {
    initMap();
    checkServiceStatus();
    loadAvailableLayers();
    initIngestionStatus();
    setupEventListeners();
    setupMapClickHandler();
});

// Setup event listeners
function setupEventListeners() {
    // Time control listeners
    setupTimeControls();
}

// Setup time control event listeners
function setupTimeControls() {
    const mapTimeSlider = document.getElementById('map-time-slider');
    const elevationSlider = document.getElementById('elevation-slider');
    const playBtn = document.getElementById('time-slider-play-btn');
    
    if (mapTimeSlider) {
        mapTimeSlider.addEventListener('input', (e) => {
            const index = parseInt(e.target.value);
            onTimeSliderChange(index);
            updateSliderProgress();
        });
    }
    
    if (elevationSlider) {
        elevationSlider.addEventListener('input', (e) => {
            const index = parseInt(e.target.value);
            onElevationSliderChange(index);
        });
    }
    
    if (playBtn) {
        playBtn.addEventListener('click', togglePlayback);
    }
    
    const speedBtn = document.getElementById('time-slider-speed-btn');
    if (speedBtn) {
        speedBtn.addEventListener('click', cyclePlaybackSpeed);
    }
}

// Cycle through playback speeds
function cyclePlaybackSpeed() {
    const currentIndex = PLAYBACK_SPEEDS.indexOf(playbackSpeed);
    const nextIndex = (currentIndex + 1) % PLAYBACK_SPEEDS.length;
    playbackSpeed = PLAYBACK_SPEEDS[nextIndex];
    
    // Update button label
    const speedBtn = document.getElementById('time-slider-speed-btn');
    if (speedBtn) {
        speedBtn.querySelector('.speed-label').textContent = SPEED_LABELS[nextIndex];
    }
    
    // If currently playing, restart with new speed
    if (isPlaying) {
        clearInterval(playbackInterval);
        startPlaybackInterval();
    }
}

// Get playback interval in ms based on speed
function getPlaybackInterval() {
    // Base interval is 1000ms, divide by speed
    return Math.round(1000 / playbackSpeed);
}

// Toggle playback of time steps
function togglePlayback() {
    if (isPlaying) {
        stopPlayback();
    } else {
        startPlayback();
    }
}

// Preload all time step layers for smooth animation
function preloadAllLayers() {
    if (selectedProtocol !== 'wmts' || !selectedLayer) return;
    
    // Clear old preloaded layers
    clearPreloadedLayers();
    
    const timeSteps = layerTimeMode === 'observation' 
        ? availableObservationTimes 
        : availableForecastHours;
    
    console.log(`Preloading ${timeSteps.length} layers for animation...`);
    
    timeSteps.forEach((timeStep, index) => {
        // Temporarily set the time to build the URL
        const originalTime = layerTimeMode === 'observation' ? currentObservationTime : currentForecastHour;
        
        if (layerTimeMode === 'observation') {
            // For observation mode, we need to reverse the index like in onTimeSliderChange
            const reversedIndex = availableObservationTimes.length - 1 - index;
            currentObservationTime = availableObservationTimes[reversedIndex];
        } else {
            currentForecastHour = timeStep;
        }
        
        const wmtsUrl = buildWmtsTileUrl(selectedLayer, selectedStyle);
        
        const layer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(selectedLayer)} (WMTS - ${selectedStyle})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0,  // Start invisible
            pane: 'weatherPane'
        });
        
        // Add to map (invisible) to trigger tile loading
        layer.addTo(map);
        preloadedLayers[index] = layer;
        
        // Restore original time
        if (layerTimeMode === 'observation') {
            currentObservationTime = originalTime;
        } else {
            currentForecastHour = originalTime;
        }
    });
}

// Clear all preloaded layers
function clearPreloadedLayers() {
    Object.values(preloadedLayers).forEach(layer => {
        if (layer) {
            map.removeLayer(layer);
        }
    });
    preloadedLayers = {};
}

// Start playback
function startPlayback() {
    const slider = document.getElementById('map-time-slider');
    const playBtn = document.getElementById('time-slider-play-btn');
    
    if (!slider) return;
    
    isPlaying = true;
    
    // Lock map zoom and pan during animation
    lockMap();
    
    // Update button appearance
    if (playBtn) {
        playBtn.classList.add('playing');
        playBtn.querySelector('.play-icon').textContent = '❚❚';
    }
    
    // Preload all layers for smooth animation
    if (selectedProtocol === 'wmts') {
        preloadAllLayers();
        
        // Hide the main layer during animation
        if (wmsLayer) {
            wmsLayer.setOpacity(0);
        }
        
        // Show the current frame
        const currentIndex = parseInt(slider.value);
        if (preloadedLayers[currentIndex]) {
            preloadedLayers[currentIndex].setOpacity(0.7);
        }
    }
    
    // Start the animation loop
    startPlaybackInterval();
}

// Start the playback interval (separated so we can restart with new speed)
function startPlaybackInterval() {
    const slider = document.getElementById('map-time-slider');
    if (!slider) return;
    
    playbackInterval = setInterval(() => {
        let currentIndex = parseInt(slider.value);
        const maxIndex = parseInt(slider.max);
        
        // Hide current frame
        if (selectedProtocol === 'wmts' && preloadedLayers[currentIndex]) {
            preloadedLayers[currentIndex].setOpacity(0);
        }
        
        // Move to next step, loop back to start if at end
        currentIndex++;
        if (currentIndex > maxIndex) {
            currentIndex = 0;
        }
        
        // Show next frame
        if (selectedProtocol === 'wmts' && preloadedLayers[currentIndex]) {
            preloadedLayers[currentIndex].setOpacity(0.7);
        }
        
        slider.value = currentIndex;
        onTimeSliderChange(currentIndex);
        updateSliderProgress();
    }, getPlaybackInterval());
}

// Stop playback
function stopPlayback() {
    // Clear interval first to stop any pending callbacks
    if (playbackInterval) {
        clearInterval(playbackInterval);
        playbackInterval = null;
    }
    
    // Only proceed with cleanup if we were actually playing
    const wasPlaying = isPlaying;
    isPlaying = false;
    
    // Unlock map zoom and pan
    unlockMap();
    
    // Clear preloaded layers and restore main layer
    if (selectedProtocol === 'wmts') {
        clearPreloadedLayers();
        if (wmsLayer) {
            wmsLayer.setOpacity(0.7);
        }
    }
    
    // Update button appearance
    const playBtn = document.getElementById('time-slider-play-btn');
    if (playBtn) {
        playBtn.classList.remove('playing');
        const playIcon = playBtn.querySelector('.play-icon');
        if (playIcon) {
            playIcon.textContent = '▶';
        }
    }
    
    console.log('Playback stopped, wasPlaying:', wasPlaying);
}

// Handle elevation slider change
function onElevationSliderChange(index) {
    if (index >= 0 && index < availableElevations.length) {
        currentElevation = availableElevations[index];
        console.log('Elevation changed to:', currentElevation);
        updateElevationSliderDisplay();
        updateLayerTime();
    }
}

// Update elevation slider display
function updateElevationSliderDisplay() {
    const label = document.getElementById('elevation-label');
    const bubble = document.getElementById('elevation-slider-current');
    const slider = document.getElementById('elevation-slider');
    
    if (label && currentElevation) {
        // Format the elevation label (e.g., "850 hPa" or "10 m")
        label.textContent = currentElevation;
    } else if (label) {
        label.textContent = 'Sfc';
    }
    
    // Position the bubble to match the slider thumb
    if (bubble && slider) {
        const percent = slider.max > 0 ? (slider.value / slider.max) * 100 : 0;
        // Invert because slider goes bottom (high value) to top (low value)
        bubble.style.top = (100 - percent) + '%';
    }
}

// Show the elevation slider
function showElevationSlider() {
    const container = document.getElementById('elevation-slider-container');
    if (container) {
        container.style.display = 'flex';
    }
}

// Hide the elevation slider
function hideElevationSlider() {
    const container = document.getElementById('elevation-slider-container');
    if (container) {
        container.style.display = 'none';
    }
}

// Show the style selector
function showStyleSelector() {
    const container = document.getElementById('style-selector-container');
    if (container) {
        container.style.display = 'flex';
    }
}

// Hide the style selector
function hideStyleSelector() {
    const container = document.getElementById('style-selector-container');
    if (container) {
        container.style.display = 'none';
    }
}

// Populate the style selector
function populateStyleSelector() {
    const container = document.getElementById('style-selector-options');
    if (!container || !selectedLayer) return;
    
    const styles = layerStyles[selectedLayer] || [];
    
    // Only show if we have multiple styles
    if (styles.length <= 1) {
        hideStyleSelector();
        return;
    }
    
    container.innerHTML = '';
    
    styles.forEach((style, index) => {
        const option = document.createElement('label');
        option.className = 'style-option' + (style.name === selectedStyle || (index === 0 && !selectedStyle) ? ' active' : '');
        option.dataset.style = style.name;
        
        const radio = document.createElement('input');
        radio.type = 'radio';
        radio.name = 'map-style';
        radio.value = style.name;
        radio.checked = style.name === selectedStyle || (index === 0 && !selectedStyle);
        
        const radioVisual = document.createElement('span');
        radioVisual.className = 'style-option-radio';
        
        const label = document.createElement('span');
        label.className = 'style-option-label';
        label.textContent = style.title || style.name;
        label.title = style.title || style.name;
        
        option.appendChild(radio);
        option.appendChild(radioVisual);
        option.appendChild(label);
        
        option.addEventListener('click', () => {
            // Update active state
            container.querySelectorAll('.style-option').forEach(opt => opt.classList.remove('active'));
            option.classList.add('active');
            radio.checked = true;
            
            // Update style and reload layer (use updateLayerTime to avoid re-fetching capabilities)
            selectedStyle = style.name;
            console.log('Style changed to:', selectedStyle);
            updateLayerTime();
        });
        
        container.appendChild(option);
    });
    
    showStyleSelector();
}

// Populate the elevation slider
function populateElevationSlider() {
    const slider = document.getElementById('elevation-slider');
    const labelsContainer = document.getElementById('elevation-slider-labels');
    
    if (!slider || !labelsContainer) return;
    
    // Only show if we have multiple elevations
    if (availableElevations.length <= 1) {
        hideElevationSlider();
        return;
    }
    
    labelsContainer.innerHTML = '';
    
    slider.min = 0;
    slider.max = availableElevations.length - 1;
    
    // Find index of current elevation, default to 0 (surface)
    let currentIndex = availableElevations.indexOf(currentElevation);
    if (currentIndex === -1) currentIndex = 0;
    slider.value = currentIndex;
    
    // Create labels for top and bottom
    const topLabel = document.createElement('span');
    topLabel.textContent = availableElevations[0];
    labelsContainer.appendChild(topLabel);
    
    const bottomLabel = document.createElement('span');
    bottomLabel.textContent = availableElevations[availableElevations.length - 1];
    labelsContainer.appendChild(bottomLabel);
    
    updateElevationSliderDisplay();
    showElevationSlider();
}

// Handle time slider change
function onTimeSliderChange(index) {
    if (layerTimeMode === 'observation') {
        // Observation mode - availableObservationTimes is sorted newest first,
        // but slider goes oldest (left) to newest (right), so reverse the index
        const reversedIndex = availableObservationTimes.length - 1 - index;
        if (reversedIndex >= 0 && reversedIndex < availableObservationTimes.length) {
            currentObservationTime = availableObservationTimes[reversedIndex];
            updateTimeSliderDisplay();
            // Only update layer if not playing (during playback we use preloaded layers)
            if (!isPlaying) {
                updateLayerTime();
            }
        }
    } else {
        // Forecast mode - index into availableForecastHours (already in ascending order)
        if (index >= 0 && index < availableForecastHours.length) {
            currentForecastHour = availableForecastHours[index];
            updateTimeSliderDisplay();
            // Only update layer if not playing (during playback we use preloaded layers)
            if (!isPlaying) {
                updateLayerTime();
            }
        }
    }
}

// Update the slider progress bar and bubble position
function updateSliderProgress() {
    const slider = document.getElementById('map-time-slider');
    const progress = document.getElementById('time-slider-progress');
    const bubble = document.getElementById('time-slider-current');
    
    if (slider && progress) {
        const percent = (slider.value / slider.max) * 100;
        progress.style.width = percent + '%';
    }
    
    // Position the bubble to follow the slider thumb
    if (slider && bubble) {
        const percent = (slider.value / slider.max) * 100;
        bubble.style.left = percent + '%';
    }
}

// Update the time slider display (current time label)
function updateTimeSliderDisplay() {
    const label = document.getElementById('current-time-label');
    if (!label) return;
    
    if (layerTimeMode === 'observation') {
        if (currentObservationTime) {
            const date = new Date(currentObservationTime);
            // Format: "Dec 9, 2:30 PM"
            label.textContent = date.toLocaleDateString('en-US', { 
                month: 'short', 
                day: 'numeric',
                hour: 'numeric',
                minute: '2-digit'
            });
        } else {
            label.textContent = '--';
        }
    } else {
        // Forecast mode - show valid time
        if (availableRuns.length > 0 && currentForecastHour !== undefined) {
            const runDate = new Date(availableRuns[0]); // Latest run
            const validDate = new Date(runDate.getTime() + currentForecastHour * 60 * 60 * 1000);
            // Format: "Dec 9, 2 PM" with forecast indicator
            const timeStr = validDate.toLocaleDateString('en-US', { 
                month: 'short', 
                day: 'numeric',
                hour: 'numeric'
            });
            label.textContent = `${timeStr} (F+${currentForecastHour}h)`;
        } else {
            label.textContent = `F+${currentForecastHour}h`;
        }
    }
}

// Show the time slider and populate it with times
function showTimeSlider() {
    const container = document.getElementById('time-slider-container');
    if (container) {
        container.style.display = 'flex';
    }
}

// Hide the time slider
function hideTimeSlider() {
    const container = document.getElementById('time-slider-container');
    if (container) {
        container.style.display = 'none';
    }
}

// Populate the time slider with available times
function populateTimeSlider() {
    const slider = document.getElementById('map-time-slider');
    const labelsContainer = document.getElementById('time-slider-labels');
    const layerNameEl = document.getElementById('time-slider-layer-name');
    
    if (!slider || !labelsContainer) return;
    
    // Set the layer name
    if (layerNameEl && selectedLayer) {
        layerNameEl.textContent = formatLayerName(selectedLayer);
    }
    
    labelsContainer.innerHTML = '';
    
    if (layerTimeMode === 'observation') {
        // Observation mode - use availableObservationTimes
        const times = availableObservationTimes;
        if (times.length === 0) {
            hideTimeSlider();
            return;
        }
        
        // Reverse the array so oldest is first (left) and newest is last (right)
        const timesReversed = [...times].reverse();
        
        slider.min = 0;
        slider.max = timesReversed.length - 1;
        slider.value = timesReversed.length - 1; // Start at latest (last in reversed array)
        
        // Create labels (show ~5 evenly spaced)
        const numLabels = Math.min(5, timesReversed.length);
        for (let i = 0; i < numLabels; i++) {
            const idx = Math.floor(i * (timesReversed.length - 1) / (numLabels - 1));
            const date = new Date(timesReversed[idx]);
            const span = document.createElement('span');
            span.textContent = formatShortTimestamp(date);
            labelsContainer.appendChild(span);
        }
        
    } else {
        // Forecast mode - use availableForecastHours
        const hours = availableForecastHours;
        if (hours.length === 0) {
            hideTimeSlider();
            return;
        }
        
        slider.min = 0;
        slider.max = hours.length - 1;
        
        // Find index of current forecast hour, default to 0
        let currentIndex = hours.indexOf(currentForecastHour);
        if (currentIndex === -1) currentIndex = 0;
        slider.value = currentIndex;
        
        // Create labels (show ~5 evenly spaced)
        const numLabels = Math.min(5, hours.length);
        for (let i = 0; i < numLabels; i++) {
            const idx = Math.floor(i * (hours.length - 1) / (numLabels - 1));
            const hour = hours[idx];
            const span = document.createElement('span');
            
            // Format with valid time if we have run info
            if (availableRuns.length > 0) {
                const runDate = new Date(availableRuns[0]);
                const validDate = new Date(runDate.getTime() + hour * 60 * 60 * 1000);
                span.textContent = formatShortTimestamp(validDate);
            } else {
                span.textContent = `+${hour}h`;
            }
            labelsContainer.appendChild(span);
        }
    }
    
    updateSliderProgress();
    updateTimeSliderDisplay();
    showTimeSlider();
}

// Format a date as short timestamp (e.g., "Dec 9, 2 PM")
function formatShortTimestamp(date) {
    return date.toLocaleDateString('en-US', { 
        month: 'short', 
        day: 'numeric',
        hour: 'numeric'
    });
}

// Lock map interactions (zoom, pan, etc.) during animation
function lockMap() {
    if (!map) return;
    
    map.dragging.disable();
    map.touchZoom.disable();
    map.doubleClickZoom.disable();
    map.scrollWheelZoom.disable();
    map.boxZoom.disable();
    map.keyboard.disable();
    if (map.tap) map.tap.disable();
    
    // Add visual indicator that map is locked
    document.getElementById('map').classList.add('map-locked');
    
    console.log('Map interactions locked for animation');
}

// Unlock map interactions after animation stops
function unlockMap() {
    if (!map) return;
    
    map.dragging.enable();
    map.touchZoom.enable();
    map.doubleClickZoom.enable();
    map.scrollWheelZoom.enable();
    map.boxZoom.enable();
    map.keyboard.enable();
    if (map.tap) map.tap.enable();
    
    // Remove visual indicator
    document.getElementById('map').classList.remove('map-locked');
    
    console.log('Map interactions unlocked');
}

// Initialize Leaflet map
function initMap() {
    map = L.map('map').setView([39, -98], 4); // Center on US

    // Create a custom pane for weather overlays that sits above the tile pane
    map.createPane('weatherPane');
    map.getPane('weatherPane').style.zIndex = 450; // Above tilePane (200) but below popups (700)

    // Define base layers
    const osmLayer = L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
        attribution: '&copy; OpenStreetMap contributors',
        maxZoom: 19
    });
    
    const cartoLight = L.tileLayer('https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}{r}.png', {
        attribution: '&copy; OpenStreetMap contributors &copy; CARTO',
        maxZoom: 19
    });
    
    const cartoDark = L.tileLayer('https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png', {
        attribution: '&copy; OpenStreetMap contributors &copy; CARTO',
        maxZoom: 19
    });

    // Add default base layer
    cartoLight.addTo(map);

    // Store base layers for layer control
    window.baseLayers = {
        'CartoDB Light': cartoLight,
        'CartoDB Dark': cartoDark,
        'OpenStreetMap': osmLayer
    };

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

// Load available layers from WMS/WMTS capabilities and create layer control
async function loadAvailableLayers() {
    try {
        // Always fetch from WMTS for layer list (protocol selection is per-layer now)
        const response = await fetch(
            `${API_BASE}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities`,
            { mode: 'cors' }
        );
        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');

        // Extract layer names, styles, bounds, and titles
        const layers = [];
        layerStyles = {}; // Reset styles map
        layerBounds = {}; // Reset bounds map
        layerTitles = {}; // Reset titles map
        
        // WMTS uses <ows:Identifier> for layer names and <ows:Title> for display
        const layerElements = xml.querySelectorAll('Contents > Layer');
        layerElements.forEach(layerEl => {
            const identifierEl = layerEl.querySelector('Identifier');
            if (identifierEl && identifierEl.textContent) {
                const layerName = identifierEl.textContent;
                layers.push(layerName);
                
                // Extract title for this layer (use ows:Title)
                const titleEl = layerEl.querySelector('Title');
                if (titleEl && titleEl.textContent) {
                    layerTitles[layerName] = titleEl.textContent;
                }
                
                // Extract styles for this layer
                const styles = [];
                const styleElements = layerEl.querySelectorAll('Style');
                styleElements.forEach(styleEl => {
                    const styleId = styleEl.querySelector('Identifier');
                    const styleTitle = styleEl.querySelector('Title');
                    if (styleId && styleId.textContent) {
                        styles.push({
                            name: styleId.textContent,
                            title: styleTitle ? styleTitle.textContent : styleId.textContent
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

        availableLayers = layers;
        
        // Sort layers by their display title for better UX
        availableLayers.sort((a, b) => {
            const titleA = layerTitles[a] || a;
            const titleB = layerTitles[b] || b;
            return titleA.localeCompare(titleB);
        });
        
        // Create the Leaflet layer control
        createLayerControl();
        
        console.log(`Loaded ${availableLayers.length} layers`);
    } catch (error) {
        console.error('Failed to load available layers:', error);
    }
}

// Create custom layer control with radio buttons for weather layers
function createLayerControl() {
    // Remove existing control if any
    if (layerControl) {
        map.removeControl(layerControl);
    }
    
    // Group layers by model (gfs, hrrr, mrms, goes16, goes18)
    const overlayGroups = {};
    
    availableLayers.forEach(layerName => {
        const parts = layerName.split('_');
        const model = parts[0].toUpperCase();
        
        if (!overlayGroups[model]) {
            overlayGroups[model] = [];
        }
        
        overlayGroups[model].push({
            name: layerName,
            displayName: formatLayerName(layerName)
        });
    });
    
    // Create custom control
    const CustomLayerControl = L.Control.extend({
        options: {
            position: 'topright',
            collapsed: true
        },
        
        onAdd: function(map) {
            const container = L.DomUtil.create('div', 'leaflet-control-layers leaflet-control');
            
            // Prevent map clicks when interacting with control
            L.DomEvent.disableClickPropagation(container);
            L.DomEvent.disableScrollPropagation(container);
            
            // Toggle button
            const toggle = L.DomUtil.create('a', 'leaflet-control-layers-toggle', container);
            toggle.href = '#';
            toggle.title = 'Layers';
            
            // Content container
            const content = L.DomUtil.create('div', 'leaflet-control-layers-list', container);
            
            // Base layers section
            const baseSection = L.DomUtil.create('div', 'leaflet-control-layers-base', content);
            Object.keys(window.baseLayers).forEach((name, idx) => {
                const label = L.DomUtil.create('label', '', baseSection);
                const input = L.DomUtil.create('input', 'leaflet-control-layers-selector', label);
                input.type = 'radio';
                input.name = 'leaflet-base-layers';
                input.checked = idx === 0; // First one is default
                input.dataset.layerName = name;
                
                const span = L.DomUtil.create('span', '', label);
                span.textContent = ' ' + name;
                
                L.DomEvent.on(input, 'change', function() {
                    // Remove all base layers, add selected one
                    Object.values(window.baseLayers).forEach(layer => map.removeLayer(layer));
                    window.baseLayers[name].addTo(map);
                });
            });
            
            // Separator
            L.DomUtil.create('div', 'leaflet-control-layers-separator', content);
            
            // Weather layers section with radio buttons
            const overlaySection = L.DomUtil.create('div', 'leaflet-control-layers-overlays', content);
            
            // "None" option
            const noneLabel = L.DomUtil.create('label', '', overlaySection);
            const noneInput = L.DomUtil.create('input', 'leaflet-control-layers-selector', noneLabel);
            noneInput.type = 'radio';
            noneInput.name = 'leaflet-weather-layers';
            noneInput.checked = true;
            noneInput.value = '';
            
            const noneSpan = L.DomUtil.create('span', '', noneLabel);
            noneSpan.textContent = ' None';
            
            const clearLayer = function(e) {
                if (e) {
                    L.DomEvent.stopPropagation(e);
                }
                noneInput.checked = true;
                clearWeatherLayer();
            };
            
            L.DomEvent.on(noneInput, 'change', clearLayer);
            L.DomEvent.on(noneLabel, 'click', function(e) {
                if (e.target !== noneInput) {
                    clearLayer(e);
                }
            });
            
            // Group layers by model
            Object.keys(overlayGroups).sort().forEach(model => {
                // Model header
                const header = L.DomUtil.create('div', 'layer-group-header', overlaySection);
                header.textContent = model;
                
                // Layers in this model
                overlayGroups[model].forEach(layer => {
                    const label = L.DomUtil.create('label', '', overlaySection);
                    const input = L.DomUtil.create('input', 'leaflet-control-layers-selector', label);
                    input.type = 'radio';
                    input.name = 'leaflet-weather-layers';
                    input.value = layer.name;
                    
                    const span = L.DomUtil.create('span', '', label);
                    span.textContent = ' ' + layer.displayName.replace(model + ' - ', '');
                    
                    // Use both change and click events for better reliability
                    const selectLayer = function(e) {
                        if (e) {
                            L.DomEvent.stopPropagation(e);
                        }
                        input.checked = true;
                        loadLayerOnMap(layer.name);
                    };
                    
                    L.DomEvent.on(input, 'change', selectLayer);
                    L.DomEvent.on(label, 'click', function(e) {
                        // Only handle if clicking the label text, not the input itself
                        if (e.target !== input) {
                            selectLayer(e);
                        }
                    });
                });
            });
            
            // Expand/collapse behavior
            let expanded = false;
            
            L.DomEvent.on(toggle, 'click', function(e) {
                L.DomEvent.preventDefault(e);
                expanded = !expanded;
                container.classList.toggle('leaflet-control-layers-expanded', expanded);
            });
            
            // Expand on hover (desktop)
            L.DomEvent.on(container, 'mouseenter', function() {
                container.classList.add('leaflet-control-layers-expanded');
                expanded = true;
            });
            
            L.DomEvent.on(container, 'mouseleave', function() {
                container.classList.remove('leaflet-control-layers-expanded');
                expanded = false;
            });
            
            return container;
        }
    });
    
    layerControl = new CustomLayerControl();
    map.addControl(layerControl);
}

// Clear the current weather layer
function clearWeatherLayer() {
    stopPlayback();
    clearPreloadedLayers();
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
        map.off('moveend', onMapMoveEnd);
        wmsLayer = null;
    }
    selectedLayer = null;
    hideTimeSlider();
    hideElevationSlider();
    hideStyleSelector();
}

// Format layer name for display
// Uses title from WMS/WMTS capabilities if available, falls back to parsing layer name
function formatLayerName(layerName) {
    // First, check if we have a title from capabilities
    if (layerTitles[layerName]) {
        return layerTitles[layerName];
    }
    
    // Fallback: parse layer name to create display name
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
        'WIND_BARBS': 'Wind Barbs',
        'CMI_C01': 'Visible Blue - Band 1',
        'CMI_C02': 'Visible Red - Band 2',
        'CMI_C08': 'Upper-Level Water Vapor - Band 8',
        'CMI_C13': 'Clean Longwave IR - Band 13'
    };
    
    // Extract parameter from layer name (e.g., "gfs_PRMSL" -> "PRMSL")
    const parts = layerName.split('_');
    if (parts.length >= 2) {
        // Check for multi-part parameter names like CMI_C01
        const param = parts.slice(1).join('_');
        if (names[param]) {
            return `${parts[0].toUpperCase()} - ${names[param]}`;
        }
        
        // Check single part
        const singleParam = parts[1];
        for (const [key, name] of Object.entries(names)) {
            if (singleParam.includes(key)) {
                return `${parts[0].toUpperCase()} - ${name}`;
            }
        }
        return `${parts[0].toUpperCase()} - ${param}`;
    }
    
    return layerName;
}

// Load a specific layer on the map
function loadLayerOnMap(layerName) {
    // Stop any active playback
    stopPlayback();
    
    // Remove existing layer and cleanup event listeners
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
        map.off('moveend', onMapMoveEnd); // Remove moveend listener from previous WMS layer
    }

    // Store current layer and reset style to default for new layer
    selectedLayer = layerName;
    selectedStyle = 'default';

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
    currentElevation = ''; // Reset elevation - will be populated from capabilities

    if (selectedProtocol === 'wmts') {
        // Create WMTS layer using Leaflet TileLayer with direct URL pattern
        // WMTS tile URL format: /wmts/rest/{layer}/{style}/{TileMatrixSet}/{z}/{x}/{y}.png?time=N&elevation=...
        const wmtsUrl = buildWmtsTileUrl(layerName, selectedStyle);
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(layerName)} (WMTS - ${selectedStyle})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7,
            pane: 'weatherPane' // Custom pane above base layers
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
        pane: 'weatherPane', // Custom pane above base layers
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

// Add entry to ingestion log (optional - only if log element exists)
function addLogEntry(message, type = 'info') {
    const ingestLogEl = document.getElementById('ingest-log');
    if (!ingestLogEl) {
        // Log element doesn't exist in the current UI, just log to console
        console.log(`[${type.toUpperCase()}] ${message}`);
        return;
    }
    
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
        
        // Update Redis cache stats (L2 cache)
        const l2Cache = data.l2_cache || {};
        if (metricCacheStatusEl) {
            const connected = l2Cache.connected ?? false;
            metricCacheStatusEl.textContent = connected ? 'Connected' : 'Disconnected';
            metricCacheStatusEl.className = 'metric-value ' + (connected ? 'good' : 'bad');
        }
        if (metricCacheKeysEl) {
            metricCacheKeysEl.textContent = l2Cache.key_count ?? '--';
        }
        if (metricCacheMemoryEl) {
            metricCacheMemoryEl.textContent = l2Cache.memory_used ? formatBytes(l2Cache.memory_used) : '--';
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
    if (!selectedLayer) {
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

// Update layer with new time/elevation (without refetching capabilities)
function updateLayerTime() {
    if (!selectedLayer) {
        return;
    }
    
    if (selectedProtocol === 'wmts') {
        const wmtsUrl = buildWmtsTileUrl(selectedLayer, selectedStyle);
        
        if (wmsLayer) {
            // Use setUrl to update tiles without recreating the layer (smoother animation)
            wmsLayer.setUrl(wmtsUrl);
        } else {
            // Create new layer if none exists
            wmsLayer = L.tileLayer(wmtsUrl, {
                attribution: `${formatLayerName(selectedLayer)} (WMTS - ${selectedStyle})`,
                maxZoom: 18,
                tileSize: 256,
                opacity: 0.7,
                pane: 'weatherPane'
            });
            wmsLayer.addTo(map);
        }
    } else {
        // For WMS, we need to recreate the overlay
        if (wmsLayer) {
            map.removeLayer(wmsLayer);
            map.off('moveend', onMapMoveEnd);
        }
        wmsLayer = createWmsImageOverlay(selectedLayer, selectedStyle);
        wmsLayer.addTo(map);
        map.on('moveend', onMapMoveEnd);
    }
    
    // Log appropriate time info based on mode
    if (layerTimeMode === 'observation') {
        console.log('Layer updated with TIME (observation):', currentObservationTime, 'ELEVATION:', currentElevation || 'default');
    } else {
        console.log('Layer updated with TIME (forecast):', currentForecastHour, 'ELEVATION:', currentElevation || 'default');
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
        availableForecastHours = [];
        availableRuns = [];
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
                        // Sort by date descending (latest first)
                        availableObservationTimes.sort((a, b) => new Date(b) - new Date(a));
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
                        // Sort descending (latest first)
                        availableRuns.sort((a, b) => new Date(b) - new Date(a));
                        console.log('Layer has RUN dimension:', availableRuns.length, 'runs, latest:', availableRuns[0]);
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
                        // Sort ascending
                        availableForecastHours.sort((a, b) => a - b);
                        // Default to first (earliest) forecast hour
                        currentForecastHour = availableForecastHours.length > 0 ? availableForecastHours[0] : 0;
                        console.log('Layer has FORECAST dimension:', availableForecastHours.length, 'hours');
                    }
                    if (dimName === 'ELEVATION') {
                        const elevationText = dim.textContent.trim();
                        availableElevations = elevationText.split(',');
                    }
                }
                
                // Determine layer time mode
                if (hasTimeDimension && !hasRunDimension) {
                    layerTimeMode = 'observation';
                } else {
                    layerTimeMode = 'forecast';
                }
                
                console.log('Layer time mode:', layerTimeMode);
                break;
            }
        }
        
        // Populate and show the time slider
        populateTimeSlider();
        
        // Populate and show the elevation slider if available
        populateElevationSlider();
        
        // Populate and show the style selector if available
        populateStyleSelector();
        
        // Reload the layer with the correct time parameters now that we know the time mode
        updateLayerTime();
        
    } catch (error) {
        console.error('Failed to fetch available times:', error);
        hideTimeSlider();
        hideElevationSlider();
    }
}



// Note: stopPlayback() is defined earlier in this file (around line 289)
// with full implementation including button state updates
