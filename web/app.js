// WMS Dashboard Application

// Smart API URL detection:
// - localhost:8000 (docker-compose) -> use localhost:8080
// - Otherwise (K8s ingress) -> use relative URLs (same origin)
const API_BASE = window.location.port === '8000' ? 'http://localhost:8080' : '';
const DOWNLOADER_URL = window.location.port === '8000' ? 'http://localhost:8081' : '/downloader';
const EDR_URL = window.location.port === '8000' ? 'http://localhost:8083/edr' : '/edr';
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
const edrStatusEl = document.getElementById('edr-status');
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

// Info Bar Cache Elements - L1
const infoL1HitsEl = document.getElementById('info-l1-hits');
const infoL1RateEl = document.getElementById('info-l1-rate');
const infoL1TilesEl = document.getElementById('info-l1-tiles');
const infoL1SizeEl = document.getElementById('info-l1-size');
// Info Bar Cache Elements - L2
const infoL2HitsEl = document.getElementById('info-l2-hits');
const infoL2RateEl = document.getElementById('info-l2-rate');
const infoL2TilesEl = document.getElementById('info-l2-tiles');
const infoL2SizeEl = document.getElementById('info-l2-size');

// Info Bar Chunk Cache Elements (Zarr)
const infoChunkEntriesEl = document.getElementById('info-chunk-entries');
const infoChunkMemoryEl = document.getElementById('info-chunk-memory');
const infoChunkHitsEl = document.getElementById('info-chunk-hits');
const infoChunkRateEl = document.getElementById('info-chunk-rate');

// Info Bar System Stats Elements
const infoSysCpusEl = document.getElementById('info-sys-cpus');
const infoSysLoad1El = document.getElementById('info-sys-load1');
const infoSysLoad5El = document.getElementById('info-sys-load5');
const infoSysRamUsedEl = document.getElementById('info-sys-ram-used');
const infoSysRamTotalEl = document.getElementById('info-sys-ram-total');
const infoSysRamPctEl = document.getElementById('info-sys-ram-pct');
const infoSysStorageEl = document.getElementById('info-sys-storage');
const infoSysFilesEl = document.getElementById('info-sys-files');
const infoSysUptimeEl = document.getElementById('info-sys-uptime');

// Info Bar Request Stats Elements
const infoWmsTotalEl = document.getElementById('info-wms-total');
const infoWms1mEl = document.getElementById('info-wms-1m');
const infoWms5mEl = document.getElementById('info-wms-5m');
const infoWmtsTotalEl = document.getElementById('info-wmts-total');
const infoWmts1mEl = document.getElementById('info-wmts-1m');
const infoWmts5mEl = document.getElementById('info-wmts-5m');
const infoRenderTotalEl = document.getElementById('info-render-total');
const infoRender1mEl = document.getElementById('info-render-1m');
const infoRender5mEl = document.getElementById('info-render-5m');
const infoRenderAvgEl = document.getElementById('info-render-avg');
const infoRenderMinMaxEl = document.getElementById('info-render-minmax');

// Info Bar Data Stats Elements
const infoDataFilesEl = document.getElementById('info-data-files');
const infoDataSizeEl = document.getElementById('info-data-size');
const infoDataDatasetsEl = document.getElementById('info-data-datasets');
const infoDataParamsEl = document.getElementById('info-data-params');
const infoDataModelsEl = document.getElementById('info-data-models');
const infoDataRawEl = document.getElementById('info-data-raw');
const infoDataShreddedEl = document.getElementById('info-data-shredded');
const infoRedisStatusEl = document.getElementById('info-redis-status');
const infoRedisKeysEl = document.getElementById('info-redis-keys');
const infoRedisMemoryEl = document.getElementById('info-redis-memory');

// State for layer selection
let availableLayers = [];
let layerStyles = {}; // Map of layer name -> array of styles
let layerBounds = {}; // Map of layer name -> {west, south, east, north}
let layerTitles = {}; // Map of layer name -> human-readable title from capabilities
let selectedProtocol = 'wmts';
let selectedStyle = 'default';
let selectedTileMatrixSet = 'WebMercatorQuad'; // 'WebMercatorQuad' or 'WorldCRS84Quad'
let selectedFormat = 'image/png'; // 'image/png', 'image/webp', or 'image/jpeg'


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
    
    // TileMatrixSet selector (WMTS projection)
    setupTileMatrixSetSelector();
    
    // Image format selector (PNG/WebP)
    setupFormatSelector();
}

// Setup TileMatrixSet selector event listeners
function setupTileMatrixSetSelector() {
    const tmsOptions = document.querySelectorAll('.tms-option');
    tmsOptions.forEach(btn => {
        btn.addEventListener('click', () => {
            const tms = btn.dataset.tms;
            if (tms === selectedTileMatrixSet) return;
            
            // Update selection state in UI
            tmsOptions.forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            
            console.log('Switching TileMatrixSet to:', tms);
            
            // Reinitialize the map with the new CRS
            // This will also reload the weather layer if one is active
            initMapWithCRS(tms);
        });
    });
}

// Setup image format selector event listeners
function setupFormatSelector() {
    const formatOptions = document.querySelectorAll('.format-option');
    formatOptions.forEach(btn => {
        btn.addEventListener('click', () => {
            const format = btn.dataset.format;
            if (format === selectedFormat) return;
            
            // Update selection state in UI
            formatOptions.forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            
            console.log('Switching image format to:', format);
            selectedFormat = format;
            
            // Reload the current layer with new format
            if (selectedLayer) {
                updateLayerTime();
            }
        });
    });
}

// Show the format selector
function showFormatSelector() {
    const container = document.getElementById('format-selector-container');
    if (container) {
        container.style.display = 'flex';
        // Sync button state with current selection
        syncFormatSelectorState();
    }
}

// Hide the format selector
function hideFormatSelector() {
    const container = document.getElementById('format-selector-container');
    if (container) {
        container.style.display = 'none';
    }
}

// Sync format selector button state with current selectedFormat
function syncFormatSelectorState() {
    const formatOptions = document.querySelectorAll('.format-option');
    formatOptions.forEach(btn => {
        if (btn.dataset.format === selectedFormat) {
            btn.classList.add('active');
        } else {
            btn.classList.remove('active');
        }
    });
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

// Show the TileMatrixSet selector (only for WMTS)
function showTmsSelector() {
    const container = document.getElementById('tms-selector-container');
    if (container && selectedProtocol === 'wmts') {
        container.style.display = 'flex';
        // Sync button state with current selection
        syncTmsSelectorState();
    }
}

// Hide the TileMatrixSet selector
function hideTmsSelector() {
    const container = document.getElementById('tms-selector-container');
    if (container) {
        container.style.display = 'none';
    }
}

// Sync TMS selector button state with current selectedTileMatrixSet
function syncTmsSelectorState() {
    const tmsOptions = document.querySelectorAll('.tms-option');
    tmsOptions.forEach(btn => {
        if (btn.dataset.tms === selectedTileMatrixSet) {
            btn.classList.add('active');
        } else {
            btn.classList.remove('active');
        }
    });
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
    showTmsSelector();
    showFormatSelector();
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

// Initialize Leaflet map with default CRS (Web Mercator)
function initMap() {
    initMapWithCRS('WebMercatorQuad');
}

// Initialize or reinitialize the Leaflet map with a specific CRS
// crsType: 'WebMercatorQuad' (EPSG:3857) or 'WorldCRS84Quad' (EPSG:4326/CRS:84)
function initMapWithCRS(crsType) {
    // Save current position if map exists
    let savedCenter = [39, -98]; // Default: US center
    let savedZoom = 4;
    let hadWeatherLayer = false;
    let savedLayerName = selectedLayer;
    let savedStyle = selectedStyle;
    
    if (map) {
        savedCenter = [map.getCenter().lat, map.getCenter().lng];
        savedZoom = map.getZoom();
        hadWeatherLayer = wmsLayer !== null;
        
        // Clean up existing map
        if (wmsLayer) {
            map.removeLayer(wmsLayer);
            map.off('moveend', onMapMoveEnd);
            wmsLayer = null;
        }
        map.remove();
        map = null;
    }
    
    // Choose CRS based on TileMatrixSet
    const isWGS84 = crsType === 'WorldCRS84Quad';
    const crs = isWGS84 ? L.CRS.EPSG4326 : L.CRS.EPSG3857;
    
    // Adjust zoom for CRS differences
    // EPSG:4326 zoom levels are different - typically need to subtract 1
    let adjustedZoom = savedZoom;
    if (isWGS84 && selectedTileMatrixSet !== 'WorldCRS84Quad') {
        // Switching TO WGS84 from Mercator - adjust zoom
        adjustedZoom = Math.max(0, savedZoom - 1);
    } else if (!isWGS84 && selectedTileMatrixSet === 'WorldCRS84Quad') {
        // Switching FROM WGS84 to Mercator - adjust zoom
        adjustedZoom = Math.min(18, savedZoom + 1);
    }
    
    // Create map with selected CRS
    const mapOptions = {
        crs: crs,
        center: savedCenter,
        zoom: adjustedZoom,
        maxZoom: 18,
        minZoom: 0
    };
    
    // For WGS84, set world bounds to full extent
    if (isWGS84) {
        mapOptions.maxBounds = [[-90, -180], [90, 180]];
        mapOptions.maxBoundsViscosity = 1.0;
    }
    
    map = L.map('map', mapOptions);
    
    // Create a custom pane for weather overlays
    map.createPane('weatherPane');
    map.getPane('weatherPane').style.zIndex = 450;
    
    // Add appropriate base layer based on CRS
    if (isWGS84) {
        // WGS84 mode: use graticule on light gray background
        addGraticuleLayer();
        map.attributionControl.addAttribution('Graticule: 5° intervals | Weather Data: WMTS Service');
    } else {
        // Web Mercator mode: use CartoDB/OSM tiles
        addMercatorBaseLayers();
        map.attributionControl.addAttribution('Weather Data: WMTS Service');
    }
    
    // Update the TileMatrixSet variable
    selectedTileMatrixSet = crsType;
    
    // Restore weather layer if one was active
    if (hadWeatherLayer && savedLayerName) {
        selectedLayer = savedLayerName;
        selectedStyle = savedStyle;
        createWeatherLayer();
    }
    
    console.log(`Map initialized with CRS: ${crsType}, zoom: ${adjustedZoom}`);
}

// Add graticule layer for WGS84 mode (5° intervals with labels)
function addGraticuleLayer() {
    // Set background color
    document.getElementById('map').style.backgroundColor = '#f0f0f0';
    
    const graticuleGroup = L.layerGroup();
    
    // Style for grid lines
    const gridStyle = {
        color: '#999',
        weight: 1,
        opacity: 0.6
    };
    
    const majorGridStyle = {
        color: '#666',
        weight: 1.5,
        opacity: 0.8
    };
    
    // Draw longitude lines (meridians) every 5 degrees
    for (let lon = -180; lon <= 180; lon += 5) {
        const isMajor = lon % 30 === 0;
        const style = isMajor ? majorGridStyle : gridStyle;
        
        L.polyline([[-90, lon], [90, lon]], style).addTo(graticuleGroup);
        
        // Add label at equator for major lines
        if (isMajor && lon !== 180) {
            const label = lon === 0 ? '0°' : (lon > 0 ? `${lon}°E` : `${Math.abs(lon)}°W`);
            L.marker([0, lon], {
                icon: L.divIcon({
                    className: 'graticule-label',
                    html: `<span class="graticule-label-text">${label}</span>`,
                    iconSize: [50, 20],
                    iconAnchor: [25, 10]
                })
            }).addTo(graticuleGroup);
        }
    }
    
    // Draw latitude lines (parallels) every 5 degrees
    for (let lat = -90; lat <= 90; lat += 5) {
        const isMajor = lat % 30 === 0;
        const style = isMajor ? majorGridStyle : gridStyle;
        
        L.polyline([[lat, -180], [lat, 180]], style).addTo(graticuleGroup);
        
        // Add label at prime meridian for major lines
        if (isMajor) {
            const label = lat === 0 ? '0°' : (lat > 0 ? `${lat}°N` : `${Math.abs(lat)}°S`);
            L.marker([lat, 5], {
                icon: L.divIcon({
                    className: 'graticule-label',
                    html: `<span class="graticule-label-text">${label}</span>`,
                    iconSize: [50, 20],
                    iconAnchor: [0, 10]
                })
            }).addTo(graticuleGroup);
        }
    }
    
    graticuleGroup.addTo(map);
    
    // Store for layer control
    window.baseLayers = {
        'Graticule (5°)': graticuleGroup
    };
}

// Add Mercator base layers (CartoDB, OSM)
function addMercatorBaseLayers() {
    // Reset background color
    document.getElementById('map').style.backgroundColor = '';
    
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
}

// Check WMS, WMTS, and EDR service status
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

    try {
        // Check EDR
        const edrResponse = await fetch(
            `${EDR_URL}`,
            { mode: 'cors', cache: 'no-cache' }
        );
        setStatusIndicator('edr', edrResponse.ok ? 'online' : 'offline');
    } catch (error) {
        console.error('EDR status check failed:', error);
        setStatusIndicator('edr', 'offline');
    }
}

// Update status indicator
function setStatusIndicator(service, status) {
    let element;
    if (service === 'wms') {
        element = wmsStatusEl;
    } else if (service === 'wmts') {
        element = wmtsStatusEl;
    } else if (service === 'edr') {
        element = edrStatusEl;
    }
    
    if (!element) return;
    
    const statusDot = element.querySelector('.status-dot');
    const statusText = element.querySelector('.status-text');

    if (statusDot) statusDot.className = `status-dot ${status}`;
    if (statusText) statusText.textContent = status.charAt(0).toUpperCase() + status.slice(1);
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
    hideTmsSelector();
    hideFormatSelector();
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
        wmsLayer = null;
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
    layerTimeMode = 'forecast';
    currentObservationTime = null;
    currentForecastHour = null; // Will be set by fetchAvailableTimes
    currentElevation = ''; // Reset elevation - will be populated from capabilities

    // Update current layer display
    if (currentLayerNameEl) {
        currentLayerNameEl.textContent = `${formatLayerName(layerName)} (${selectedProtocol.toUpperCase()})`;
    }
    updatePerformanceDisplay();
    
    // Fetch available times FIRST, then create the layer with correct time values
    fetchAvailableTimes();
}

// Create and add the weather layer to the map (called after fetchAvailableTimes)
function createWeatherLayer() {
    if (!selectedLayer) return;
    
    // Remove existing layer if any
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
        map.off('moveend', onMapMoveEnd);
    }

    if (selectedProtocol === 'wmts') {
        // Create WMTS layer using Leaflet TileLayer with direct URL pattern
        const wmtsUrl = buildWmtsTileUrl(selectedLayer, selectedStyle);
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(selectedLayer)} (WMTS - ${selectedStyle} - ${selectedTileMatrixSet})`,
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
        wmsLayer = createWmsImageOverlay(selectedLayer, selectedStyle);
        wmsLayer.addTo(map);
        
        // Update the overlay when map moves
        map.on('moveend', onMapMoveEnd);
        
        console.log('Loaded WMS layer (single GetMap):', selectedLayer, 'with style:', selectedStyle, 'time:', currentForecastHour);
    }
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
    // Use WMTS KVP format which properly supports dimensions
    const params = [
        'SERVICE=WMTS',
        'REQUEST=GetTile',
        `LAYER=${layerName}`,
        `STYLE=${style}`,
        `TILEMATRIXSET=${selectedTileMatrixSet}`,
        'TILEMATRIX={z}',
        'TILEROW={y}',
        'TILECOL={x}',
        `FORMAT=${selectedFormat}`
    ];
    
    // Add dimension parameters based on layer mode
    console.log('Building WMTS URL - layerTimeMode:', layerTimeMode, 'currentRun:', currentRun, 'currentForecastHour:', currentForecastHour, 'currentObservationTime:', currentObservationTime);
    
    if (layerTimeMode === 'observation') {
        // Observation layers (GOES, MRMS) use TIME dimension with ISO8601 timestamp
        if (currentObservationTime) {
            params.push(`TIME=${encodeURIComponent(currentObservationTime)}`);
        } else {
            console.warn('Observation mode but no currentObservationTime set');
        }
    } else {
        // Forecast layers (GFS, HRRR) use RUN + FORECAST dimensions
        // RUN: model run time (ISO8601 or 'latest')
        if (currentRun) {
            params.push(`RUN=${encodeURIComponent(currentRun)}`);
        }
        // FORECAST: forecast hour offset
        const forecastHour = (currentForecastHour !== undefined && currentForecastHour !== null) 
            ? currentForecastHour 
            : 0;  // Default to 0 if not set
        params.push(`FORECAST=${forecastHour}`);
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
    
    // Build base WMS parameters
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
        FORMAT: selectedFormat,
        TRANSPARENT: 'true'
    });
    
    // Add dimension parameters based on layer mode
    if (layerTimeMode === 'observation') {
        // Observation layers (GOES, MRMS) use TIME dimension with ISO8601 timestamp
        if (currentObservationTime) {
            params.set('TIME', currentObservationTime);
        }
        console.log('WMS TIME parameter:', currentObservationTime, 'layerTimeMode:', layerTimeMode);
    } else {
        // Forecast layers (GFS, HRRR) use RUN + FORECAST dimensions
        if (currentRun) {
            params.set('RUN', currentRun);
        }
        const forecastHour = (currentForecastHour !== undefined && currentForecastHour !== null) 
            ? currentForecastHour 
            : 0;
        params.set('FORECAST', forecastHour.toString());
        console.log('WMS RUN:', currentRun, 'FORECAST:', forecastHour, 'layerTimeMode:', layerTimeMode);
    }
    
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
    fetchDataStats();
    fetchWmsLayerCounts();
    // Refresh ingestion status every 10 seconds
    ingestionStatusInterval = setInterval(() => {
        checkIngestionStatus();
    }, 10000);
    // Refresh backend metrics every 2 seconds
    setInterval(fetchBackendMetrics, 2000);
    // Refresh container stats every 5 seconds
    setInterval(fetchContainerStats, 5000);
    // Refresh data stats every 30 seconds
    setInterval(fetchDataStats, 30000);
    // Refresh WMS layer counts every 60 seconds
    setInterval(fetchWmsLayerCounts, 60000);
}

// Fetch WMS layer counts from GetCapabilities
async function fetchWmsLayerCounts() {
    try {
        const response = await fetch(`${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`);
        if (!response.ok) return;
        
        const text = await response.text();
        const parser = new DOMParser();
        const doc = parser.parseFromString(text, 'text/xml');
        const layers = doc.querySelectorAll('Layer > Name');
        
        // Map model names to display IDs
        const modelMapping = {
            'gfs': 'gfs',
            'hrrr': 'hrrr',
            'goes16': 'goes',
            'goes18': 'goes',
            'mrms': 'mrms'
        };
        
        // Count parameter layers per model (e.g., gfs_TMP, hrrr_GUST, goes18_CMI_C01)
        const layerCounts = { gfs: 0, hrrr: 0, goes: 0, mrms: 0 };
        layers.forEach(layer => {
            const name = layer.textContent;
            // Match pattern: model_PARAM (parameter layers only, not group layers)
            const match = name.match(/^(gfs|hrrr|goes16|goes18|mrms)_/i);
            if (match) {
                const modelName = match[1].toLowerCase();
                const displayName = modelMapping[modelName] || modelName;
                if (layerCounts[displayName] !== undefined) {
                    layerCounts[displayName]++;
                }
            }
        });
        
        // Update UI
        for (const [model, count] of Object.entries(layerCounts)) {
            const el = document.getElementById(`info-layer-${model}`);
            if (el) {
                const countEl = el.querySelector('.info-layer-count');
                if (countEl) {
                    countEl.textContent = count > 0 ? count : '-';
                    countEl.className = 'info-layer-count' + (count === 0 ? ' none' : '');
                }
            }
        }
    } catch (e) {
        console.debug('Could not fetch WMS layer counts:', e.message);
    }
}

// Fetch data/ingestion stats from admin API
async function fetchDataStats() {
    try {
        const response = await fetch(`${API_BASE}/api/admin/ingestion/status`);
        if (!response.ok) return;
        
        const data = await response.json();
        const catalog = data.catalog_summary || {};
        const models = data.models || [];
        
        // Update Info Bar data stats
        if (infoDataDatasetsEl) {
            infoDataDatasetsEl.textContent = (catalog.total_datasets ?? 0).toLocaleString();
        }
        if (infoDataParamsEl) {
            infoDataParamsEl.textContent = (catalog.total_parameters ?? 0).toLocaleString();
        }
        if (infoDataModelsEl) {
            // Count active models
            const activeModels = models.filter(m => m.status === 'active').length;
            infoDataModelsEl.textContent = activeModels.toLocaleString();
        }
        if (infoDataRawEl) {
            infoDataRawEl.textContent = (catalog.raw_object_count ?? 0).toLocaleString();
        }
        if (infoDataShreddedEl) {
            infoDataShreddedEl.textContent = (catalog.shredded_object_count ?? 0).toLocaleString();
        }
        
        // Update per-model stats in the Data info bar
        updatePerModelStats(models, catalog);
    } catch (error) {
        console.error('Error fetching data stats:', error);
    }
}

// Update per-model statistics in the Data info bar
// Fetches from: downloader schedule, ingestion status, and WMS capabilities
async function updatePerModelStats(models, catalog) {
    // Map model names to element IDs (handle variations like goes16/goes18 -> goes)
    const modelMapping = {
        'gfs': 'gfs',
        'hrrr': 'hrrr',
        'goes16': 'goes',
        'goes18': 'goes',
        'mrms': 'mrms'
    };
    
    // Aggregate stats per display model (e.g., combine goes16 + goes18)
    const aggregated = {};
    
    // First, try to get enabled status from downloader schedule
    try {
        const scheduleResponse = await fetch(`${DOWNLOADER_URL}/schedule`);
        if (scheduleResponse.ok) {
            const schedule = await scheduleResponse.json();
            (schedule.models || []).forEach(m => {
                const name = (m.id || '').toLowerCase();
                const displayName = modelMapping[name] || name;
                if (!displayName) return;
                
                if (!aggregated[displayName]) {
                    aggregated[displayName] = {
                        status: m.enabled ? 'enabled' : 'disabled',
                        files: 0,
                        params: 0,
                        hasData: false
                    };
                } else if (m.enabled && aggregated[displayName].status === 'disabled') {
                    aggregated[displayName].status = 'enabled';
                }
            });
        }
    } catch (e) {
        // Downloader not available, continue with ingestion data only
    }
    
    // Process models array from ingestion status
    models.forEach(model => {
        // Use model.id (e.g., "gfs") not model.name (e.g., "GFS Model")
        const name = (model.id || model.name || '').toLowerCase();
        const displayName = modelMapping[name] || name;
        
        if (!displayName) return;
        
        if (!aggregated[displayName]) {
            aggregated[displayName] = {
                status: 'inactive',
                files: 0,
                params: 0,
                hasData: false
            };
        }
        
        // If any sub-model is active (has data), mark as active
        if (model.status === 'active') {
            aggregated[displayName].status = 'active';
        } else if (model.status === 'error' && aggregated[displayName].status !== 'active') {
            aggregated[displayName].status = 'error';
        }
        
        // Sum up files and params (API uses total_files and parameters)
        aggregated[displayName].files += model.total_files || model.file_count || 0;
        aggregated[displayName].params += (model.parameters ? model.parameters.length : 0) || model.parameter_count || 0;
        
        if ((model.total_files || model.file_count) > 0) {
            aggregated[displayName].hasData = true;
            aggregated[displayName].status = 'active';
        }
    });
    
    // Also check catalog per-model stats if available (it's an array, not object)
    const catalogModels = catalog.models || [];
    catalogModels.forEach(stats => {
        const name = (stats.model || '').toLowerCase();
        const displayName = modelMapping[name] || name;
        
        if (!displayName) return;
        
        if (!aggregated[displayName]) {
            aggregated[displayName] = {
                status: 'inactive',
                files: 0,
                params: 0,
                hasData: false
            };
        }
        
        // Use catalog counts if higher (more accurate)
        const datasetCount = stats.dataset_count || stats.datasets || 0;
        const paramCount = stats.parameter_count || stats.parameters || 0;
        
        if (datasetCount > aggregated[displayName].files) {
            aggregated[displayName].files = datasetCount;
        }
        if (paramCount > aggregated[displayName].params) {
            aggregated[displayName].params = paramCount;
        }
        if (datasetCount > 0) {
            aggregated[displayName].hasData = true;
            aggregated[displayName].status = 'active';
        }
    });
    
    // Update the UI for each model
    ['gfs', 'hrrr', 'goes', 'mrms'].forEach(modelName => {
        const modelEl = document.getElementById(`info-model-${modelName}`);
        if (!modelEl) return;
        
        const stats = aggregated[modelName] || { status: 'inactive', files: 0, params: 0 };
        
        const statusEl = modelEl.querySelector('.info-model-status');
        const filesEl = modelEl.querySelector('.info-model-files');
        const paramsEl = modelEl.querySelector('.info-model-params');
        
        if (statusEl) {
            // Show a status indicator
            statusEl.className = 'info-model-status ' + stats.status;
            if (stats.status === 'active') {
                statusEl.textContent = '\u2713'; // checkmark - has data
            } else if (stats.status === 'enabled') {
                statusEl.textContent = '\u25cb'; // circle - enabled but no data yet
            } else if (stats.status === 'error') {
                statusEl.textContent = '\u2717'; // X mark - error
            } else if (stats.status === 'disabled') {
                statusEl.textContent = '\u2212'; // minus - disabled
            } else {
                statusEl.textContent = '-';
            }
        }
        
        if (filesEl) {
            filesEl.textContent = stats.files > 0 ? stats.files.toLocaleString() + 'f' : '-';
        }
        
        if (paramsEl) {
            paramsEl.textContent = stats.params > 0 ? stats.params.toLocaleString() + 'p' : '-';
        }
    });
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
        if (datasetCountEl) datasetCountEl.textContent = datasetCount;
        if (modelsListEl) modelsListEl.textContent = Array.from(models).join(', ') || 'None';
        
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
    if (!ingesterServiceStatusEl) return;
    
    const statusDot = ingesterServiceStatusEl.querySelector('.status-dot');
    const statusText = ingesterServiceStatusEl.querySelector('.status-text');
    
    if (statusDot) statusDot.className = `status-dot ${status}`;
    if (statusText) statusText.textContent = status.charAt(0).toUpperCase() + status.slice(1);
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
            if (storageSizeEl) {
                storageSizeEl.textContent = `${formatBytes(stats.total_size)} (${stats.object_count} files)`;
            }
            
            // Update Info Bar storage stats
            if (infoSysStorageEl) {
                infoSysStorageEl.textContent = formatBytes(stats.total_size);
            }
            if (infoSysFilesEl) {
                infoSysFilesEl.textContent = stats.object_count.toLocaleString();
            }
            
            // Update Info Bar data stats
            if (infoDataFilesEl) {
                infoDataFilesEl.textContent = stats.object_count.toLocaleString();
            }
            if (infoDataSizeEl) {
                infoDataSizeEl.textContent = formatBytes(stats.total_size);
            }
        }
    } catch (error) {
        console.error('Error fetching storage stats:', error);
        if (storageSizeEl) storageSizeEl.textContent = 'N/A';
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
        
        // Update request counts (sidebar)
        if (metricWmsRequestsEl) metricWmsRequestsEl.textContent = metrics.wms_requests ?? 0;
        if (metricWmtsRequestsEl) metricWmtsRequestsEl.textContent = metrics.wmts_requests ?? 0;
        
        // Update render stats (sidebar)
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
        
        // Update Info Bar request stats
        if (infoWmsTotalEl) {
            infoWmsTotalEl.textContent = (metrics.wms_requests ?? 0).toLocaleString();
        }
        if (infoWms1mEl) {
            infoWms1mEl.textContent = (metrics.wms_count_1m ?? 0).toLocaleString();
        }
        if (infoWms5mEl) {
            infoWms5mEl.textContent = (metrics.wms_count_5m ?? 0).toLocaleString();
        }
        if (infoWmtsTotalEl) {
            infoWmtsTotalEl.textContent = (metrics.wmts_requests ?? 0).toLocaleString();
        }
        if (infoWmts1mEl) {
            infoWmts1mEl.textContent = (metrics.wmts_count_1m ?? 0).toLocaleString();
        }
        if (infoWmts5mEl) {
            infoWmts5mEl.textContent = (metrics.wmts_count_5m ?? 0).toLocaleString();
        }
        if (infoRenderTotalEl) {
            infoRenderTotalEl.textContent = (metrics.renders_total ?? 0).toLocaleString();
        }
        if (infoRender1mEl) {
            infoRender1mEl.textContent = (metrics.render_count_1m ?? 0).toLocaleString();
        }
        if (infoRender5mEl) {
            infoRender5mEl.textContent = (metrics.render_count_5m ?? 0).toLocaleString();
        }
        if (infoRenderAvgEl) {
            infoRenderAvgEl.textContent = (metrics.render_avg_ms ?? 0).toFixed(0) + 'ms';
        }
        if (infoRenderMinMaxEl) {
            const min = (metrics.render_min_ms ?? 0).toFixed(0);
            const max = (metrics.render_max_ms ?? 0).toFixed(0);
            infoRenderMinMaxEl.textContent = `${min}/${max}ms`;
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
        
        // Update Info Bar Redis stats
        if (infoRedisStatusEl) {
            const connected = l2Cache.connected ?? false;
            infoRedisStatusEl.textContent = connected ? 'OK' : 'Down';
            infoRedisStatusEl.style.color = connected ? '#10b981' : '#ef4444';
        }
        if (infoRedisKeysEl) {
            infoRedisKeysEl.textContent = (l2Cache.key_count ?? 0).toLocaleString();
        }
        if (infoRedisMemoryEl) {
            infoRedisMemoryEl.textContent = l2Cache.memory_used ? formatBytes(l2Cache.memory_used) : '-';
        }
        
        // Update Info Bar L1 cache stats
        const l1Cache = data.l1_cache || {};
        if (infoL1HitsEl) {
            infoL1HitsEl.textContent = (l1Cache.hits ?? 0).toLocaleString();
        }
        if (infoL1RateEl) {
            infoL1RateEl.textContent = (l1Cache.hit_rate ?? 0).toFixed(1) + '%';
        }
        if (infoL1TilesEl) {
            // Estimate tile count from hits + misses that weren't evicted
            const l1Tiles = (l1Cache.hits ?? 0) + (l1Cache.misses ?? 0) - (l1Cache.evictions ?? 0) - (l1Cache.expired ?? 0);
            infoL1TilesEl.textContent = Math.max(0, l1Tiles).toLocaleString();
        }
        if (infoL1SizeEl) {
            infoL1SizeEl.textContent = formatBytes(l1Cache.size_bytes ?? 0);
        }
        
        // Update Info Bar L2 cache stats
        if (infoL2HitsEl) {
            infoL2HitsEl.textContent = (metrics.cache_hits ?? 0).toLocaleString();
        }
        if (infoL2RateEl) {
            infoL2RateEl.textContent = (metrics.cache_hit_rate ?? 0).toFixed(1) + '%';
        }
        if (infoL2TilesEl) {
            infoL2TilesEl.textContent = (l2Cache.key_count ?? 0).toLocaleString();
        }
        if (infoL2SizeEl) {
            infoL2SizeEl.textContent = l2Cache.memory_used ? formatBytes(l2Cache.memory_used) : '-';
        }
        
        // Update Info Bar chunk cache stats (Zarr)
        const chunkCache = data.chunk_cache || {};
        if (infoChunkEntriesEl) {
            infoChunkEntriesEl.textContent = (chunkCache.entries ?? 0).toLocaleString();
        }
        if (infoChunkMemoryEl) {
            const memoryMb = chunkCache.memory_mb ?? 0;
            infoChunkMemoryEl.textContent = memoryMb < 1 ? '<1 MB' : memoryMb.toFixed(1) + ' MB';
        }
        if (infoChunkHitsEl) {
            infoChunkHitsEl.textContent = (chunkCache.hits ?? 0).toLocaleString();
        }
        if (infoChunkRateEl) {
            const total = (chunkCache.hits ?? 0) + (chunkCache.misses ?? 0);
            const rate = total > 0 ? ((chunkCache.hits ?? 0) / total) * 100 : 0;
            infoChunkRateEl.textContent = rate.toFixed(1) + '%';
        }
        
        // Update per-source stats from data_source_stats
        const dataSourceStats = metrics.data_source_stats || {};
        for (const [source, stats] of Object.entries(dataSourceStats)) {
            // Update per-source stats
            const sourceEl = document.getElementById(`info-src-${source}`);
            if (sourceEl) {
                const rateEl = sourceEl.querySelector('.info-source-rate');
                const timeEl = sourceEl.querySelector('.info-source-time');
                
                if (rateEl) {
                    const rate = stats.cache_hit_rate || 0;
                    rateEl.textContent = rate.toFixed(0) + '%';
                    rateEl.className = 'info-source-rate' + (rate === 0 ? ' none' : rate < 20 ? ' low' : '');
                }
                if (timeEl) {
                    const avgMs = stats.avg_parse_ms || 0;
                    timeEl.textContent = avgMs < 1 ? '<1ms' : avgMs.toFixed(0) + 'ms';
                }
            }
        }
        
        // Update system stats (sidebar)
        if (metricMemoryEl) {
            metricMemoryEl.textContent = system.memory_used_bytes ? formatBytes(system.memory_used_bytes) : '--';
        }
        if (metricThreadsEl) {
            metricThreadsEl.textContent = system.num_threads ?? '--';
        }
        if (metricUptimeEl) {
            metricUptimeEl.textContent = metrics.uptime_secs ? formatUptime(metrics.uptime_secs) : '--';
        }
        
        // Update Info Bar system stats - uptime from metrics
        if (infoSysUptimeEl) {
            infoSysUptimeEl.textContent = metrics.uptime_secs ? formatUptime(metrics.uptime_secs) : '-';
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
    
    // Update Info Bar system stats
    if (infoSysCpusEl) {
        infoSysCpusEl.textContent = cpu.count;
    }
    if (infoSysLoad1El) {
        infoSysLoad1El.textContent = cpu.load_1m.toFixed(2);
    }
    if (infoSysLoad5El) {
        infoSysLoad5El.textContent = cpu.load_5m.toFixed(2);
    }
    if (infoSysRamUsedEl) {
        infoSysRamUsedEl.textContent = formatBytes(process.rss_bytes);
    }
    if (infoSysRamTotalEl) {
        infoSysRamTotalEl.textContent = formatBytes(memory.host_total_bytes);
    }
    if (infoSysRamPctEl) {
        infoSysRamPctEl.textContent = memory.percent_used.toFixed(1) + '%';
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
    if (performanceStats.tileTimes.length > 0 && lastTileTimeEl) {
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
    if (performanceStats.tileTimes.length > 0 && avgTileTimeEl) {
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
    if (performanceStats.tileTimes.length > 0 && slowestTileTimeEl) {
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
    if (tilesLoadedCountEl) {
        tilesLoadedCountEl.textContent = performanceStats.tilesLoaded;
    }
    
    // Current layer
    if (performanceStats.currentLayer && currentLayerNameEl) {
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

// Run validation manually (triggered by compliance badge click)
async function runValidation() {
    const wmsBtn = document.getElementById('wms-compliance-btn');
    const wmtsBtn = document.getElementById('wmts-compliance-btn');
    
    // Disable buttons and show loading state
    if (wmsBtn) {
        wmsBtn.disabled = true;
        wmsBtn.querySelector('.compliance-text').textContent = 'Checking...';
    }
    if (wmtsBtn) {
        wmtsBtn.disabled = true;
        wmtsBtn.querySelector('.compliance-text').textContent = 'Checking...';
    }
    
    try {
        const response = await fetch(`${API_BASE}/api/validation/run`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }
        
        const data = await response.json();
        updateValidationUI(data);
    } catch (error) {
        console.error('Error running validation:', error);
        // Show error state
        if (wmsBtn) {
            wmsBtn.querySelector('.compliance-text').textContent = 'Error';
        }
        if (wmtsBtn) {
            wmtsBtn.querySelector('.compliance-text').textContent = 'Error';
        }
    } finally {
        // Re-enable buttons
        if (wmsBtn) wmsBtn.disabled = false;
        if (wmtsBtn) wmtsBtn.disabled = false;
    }
}

// Update validation UI with results
function updateValidationUI(data) {
    // Update WMS compliance badge in header
    updateComplianceBadge('wms', data.wms);
    
    // Update WMTS compliance badge in header
    updateComplianceBadge('wmts', data.wmts);
}

// Update compliance badge in header
function updateComplianceBadge(service, serviceData) {
    const btn = document.getElementById(`${service}-compliance-btn`);
    if (!btn) return;
    
    const dot = btn.querySelector('.status-dot');
    const text = btn.querySelector('.compliance-text');
    
    // Remove all status classes
    btn.classList.remove('compliant', 'non-compliant', 'partial');
    
    if (serviceData.status === 'compliant') {
        btn.classList.add('compliant');
        text.textContent = 'Compliant';
    } else if (serviceData.status === 'non-compliant') {
        btn.classList.add('non-compliant');
        text.textContent = 'Non-Compliant';
    } else if (serviceData.status === 'partial') {
        btn.classList.add('partial');
        text.textContent = 'Partial';
    } else {
        text.textContent = 'Unknown';
    }
    
    // Update tooltip with check details
    const checks = serviceData.checks || {};
    const checkNames = Object.keys(checks);
    const passed = checkNames.filter(c => checks[c] && checks[c].status === 'pass').length;
    const total = checkNames.length;
    btn.title = `OGC ${service.toUpperCase()} Compliance: ${passed}/${total} checks passed\nClick to revalidate`;
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

// Add event listener for compliance badge buttons
document.addEventListener('DOMContentLoaded', () => {
    const wmsComplianceBtn = document.getElementById('wms-compliance-btn');
    const wmtsComplianceBtn = document.getElementById('wmts-compliance-btn');
    
    // Navigate to compliance viewer pages on click
    if (wmsComplianceBtn) {
        wmsComplianceBtn.addEventListener('click', () => {
            window.location.href = 'wms-compliance.html';
        });
    }
    if (wmtsComplianceBtn) {
        wmtsComplianceBtn.addEventListener('click', () => {
            window.location.href = 'wmts-compliance.html';
        });
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
        
        // Always remove and recreate layer to ensure Leaflet fetches fresh tiles
        // setUrl() + redraw() doesn't reliably clear Leaflet's internal tile cache
        if (wmsLayer) {
            map.removeLayer(wmsLayer);
        }
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(selectedLayer)} (WMTS - ${selectedStyle} - ${selectedTileMatrixSet})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7,
            pane: 'weatherPane'
        });
        wmsLayer.addTo(map);
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
        console.log('Layer updated with RUN:', currentRun, 'FORECAST:', currentForecastHour, 'ELEVATION:', currentElevation || 'default');
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
                
                for (let dim of dimensions) {
                    const dimName = dim.getAttribute('name');
                    const dimUnits = dim.getAttribute('units') || '';
                    
                    if (dimName === 'TIME') {
                        // TIME dimension is for observation layers (GOES, MRMS)
                        // Contains ISO8601 timestamps
                        const timeText = dim.textContent.trim();
                        const timeValues = timeText.split(',').filter(t => t && t !== 'latest');
                        
                        layerTimeMode = 'observation';
                        availableObservationTimes = timeValues;
                        // Sort by date descending (latest first)
                        availableObservationTimes.sort((a, b) => new Date(b) - new Date(a));
                        // Set default to latest observation time
                        currentObservationTime = availableObservationTimes.length > 0 
                            ? availableObservationTimes[0] 
                            : null;
                        console.log('Layer has TIME dimension (observation mode):', availableObservationTimes.length, 'times');
                    }
                    
                    if (dimName === 'RUN') {
                        // RUN dimension is for forecast models (GFS, HRRR)
                        // Contains ISO8601 model run times
                        const runsText = dim.textContent.trim();
                        availableRuns = runsText.split(',').filter(r => r && r.trim());
                        // Sort by date descending (latest first)
                        availableRuns.sort((a, b) => {
                            if (a === 'latest') return -1;
                            if (b === 'latest') return 1;
                            return new Date(b) - new Date(a);
                        });
                        // Default to latest run
                        currentRun = availableRuns.length > 0 ? availableRuns[0] : 'latest';
                        layerTimeMode = 'forecast';
                        console.log('Layer has RUN dimension:', availableRuns.length, 'runs, current:', currentRun);
                    }
                    
                    if (dimName === 'FORECAST') {
                        // FORECAST dimension is for forecast models (GFS, HRRR)
                        // Contains forecast hours (integers)
                        const forecastText = dim.textContent.trim();
                        availableForecastHours = forecastText.split(',').map(h => {
                            h = h.trim();
                            // Handle ISO 8601 duration format (PT0H, PT3H) just in case
                            const isoMatch = h.match(/^PT(\d+)H$/i);
                            if (isoMatch) return parseInt(isoMatch[1]);
                            const parsed = parseInt(h);
                            return isNaN(parsed) ? 0 : parsed;
                        }).filter(h => !isNaN(h));
                        // Sort ascending (earliest first)
                        availableForecastHours.sort((a, b) => a - b);
                        // Default to first (earliest) forecast hour
                        currentForecastHour = availableForecastHours.length > 0 ? availableForecastHours[0] : 0;
                        layerTimeMode = 'forecast';
                        console.log('Layer has FORECAST dimension:', availableForecastHours.length, 'hours:', availableForecastHours);
                    }
                    
                    if (dimName === 'ELEVATION') {
                        const elevationText = dim.textContent.trim();
                        availableElevations = elevationText.split(',');
                    }
                }
                
                // Ensure defaults are set for forecast mode
                if (layerTimeMode === 'forecast' && (currentForecastHour === null || currentForecastHour === undefined)) {
                    currentForecastHour = availableForecastHours.length > 0 ? availableForecastHours[0] : 0;
                    console.log('Set default forecast hour:', currentForecastHour);
                }
                
                console.log('Layer time mode:', layerTimeMode, 
                    layerTimeMode === 'observation' ? 
                        `currentObservationTime: ${currentObservationTime}` : 
                        `currentForecastHour: ${currentForecastHour}`);
                break;
            }
        }
        
        // Populate and show the time slider
        populateTimeSlider();
        
        // Populate and show the elevation slider if available
        populateElevationSlider();
        
        // Populate and show the style selector if available
        populateStyleSelector();
        
        // Show TileMatrixSet selector for WMTS layers
        if (selectedProtocol === 'wmts') {
            showTmsSelector();
        } else {
            hideTmsSelector();
        }
        
        // Now create the layer with the correct time parameters
        createWeatherLayer();
        
    } catch (error) {
        console.error('Failed to fetch available times:', error);
        hideTimeSlider();
        hideElevationSlider();
        hideTmsSelector();
        // Default to forecast mode with hour 0 if time fetch fails
        layerTimeMode = 'forecast';
        currentForecastHour = 0;
        console.log('Using default forecast mode with hour 0 due to error');
        // Still try to create layer even if time fetch fails
        createWeatherLayer();
    }
}



// Note: stopPlayback() is defined earlier in this file (around line 289)
// with full implementation including button state updates

// ============================================================================
// Minimap with Tile Request Heatmap
// ============================================================================

let minimap = null;
let minimapViewportRect = null;
let tileRequestHeatmap = {};  // Map of bbox key -> tile data with timestamp (client-side only for fade mode)
let totalTileRequests = 0;
let heatmapLayer = null;
let fadeEnabled = false;
let fadeTimeoutSecs = 10;
let fadeInterval = null;
let lastServerCounts = {};  // Track last known count per tile to detect new requests

// Initialize the minimap
function initMinimap() {
    const minimapEl = document.getElementById('minimap');
    if (!minimapEl) return;
    
    // Create minimap with no controls - locked to global view
    minimap = L.map('minimap', {
        zoomControl: false,
        attributionControl: false,
        dragging: false,
        touchZoom: false,
        scrollWheelZoom: false,
        doubleClickZoom: false,
        boxZoom: false,
        keyboard: false,
        tap: false,
        minZoom: 1,
        maxZoom: 1,
        maxBounds: [[-90, -180], [90, 180]],
        maxBoundsViscosity: 1.0
    }).setView([20, 0], 1);  // Global view centered
    
    // Add dark base layer
    L.tileLayer('https://{s}.basemaps.cartocdn.com/dark_nolabels/{z}/{x}/{y}{r}.png', {
        maxZoom: 18,
        opacity: 0.6
    }).addTo(minimap);
    
    // Create heatmap canvas layer
    heatmapLayer = L.layerGroup().addTo(minimap);
    
    // Create viewport rectangle showing main map bounds
    minimapViewportRect = L.rectangle([[0, 0], [0, 0]], {
        color: '#f59e0b',
        weight: 2,
        fillColor: '#f59e0b',
        fillOpacity: 0.15,
        interactive: false
    }).addTo(minimap);
    
    // Setup clear button
    const clearBtn = document.getElementById('minimap-clear-btn');
    if (clearBtn) {
        clearBtn.addEventListener('click', clearHeatmap);
    }
    
    // Setup fade controls
    const fadeCheckbox = document.getElementById('minimap-fade-enabled');
    const fadeSlider = document.getElementById('minimap-fade-slider');
    const fadeValueEl = document.getElementById('minimap-fade-value');
    
    if (fadeCheckbox) {
        fadeCheckbox.addEventListener('change', (e) => {
            fadeEnabled = e.target.checked;
            if (fadeSlider) fadeSlider.disabled = !fadeEnabled;
            if (fadeEnabled) {
                startFadeInterval();
            } else {
                stopFadeInterval();
            }
        });
        // Initialize slider state
        if (fadeSlider) fadeSlider.disabled = !fadeCheckbox.checked;
    }
    
    if (fadeSlider && fadeValueEl) {
        fadeSlider.addEventListener('input', (e) => {
            fadeTimeoutSecs = parseInt(e.target.value, 10);
            fadeValueEl.textContent = `${fadeTimeoutSecs}s`;
        });
    }
    
    // Initial sync of viewport rectangle
    syncMinimapViewport();
    
    console.log('Minimap initialized (global view)');
}

// Start the fade interval timer
function startFadeInterval() {
    if (fadeInterval) return;
    // Update display frequently for smooth fade animation
    fadeInterval = setInterval(() => {
        if (fadeEnabled) {
            pruneAndUpdateFadingTiles();
        }
    }, 200); // Update 5x per second for smooth fading
}

// Stop the fade interval timer
function stopFadeInterval() {
    if (fadeInterval) {
        clearInterval(fadeInterval);
        fadeInterval = null;
    }
}

// Update fading tiles and remove fully faded ones
function pruneAndUpdateFadingTiles() {
    const now = Date.now();
    const cutoff = now - (fadeTimeoutSecs * 1000);
    
    // Remove tiles that have completely faded
    Object.keys(tileRequestHeatmap).forEach(key => {
        const tile = tileRequestHeatmap[key];
        if (tile.lastSeen && tile.lastSeen < cutoff) {
            delete tileRequestHeatmap[key];
        }
    });
    
    // Redraw with current fade levels
    updateHeatmapDisplay();
    updateMinimapStats();
}

// Sync minimap viewport rectangle with main map bounds (minimap stays fixed at global view)
function syncMinimapViewport() {
    if (!minimap || !map || !minimapViewportRect) return;
    
    // Update only the viewport rectangle to show where the main map is looking
    const bounds = map.getBounds();
    minimapViewportRect.setBounds(bounds);
    
    // Minimap stays locked at global view - no zoom/pan changes
}

// Update the heatmap visualization on the minimap
function updateHeatmapDisplay() {
    if (!minimap || !heatmapLayer) return;
    
    // Clear existing markers
    heatmapLayer.clearLayers();
    
    const now = Date.now();
    
    // Add tile box markers using actual tile bounds from server
    Object.values(tileRequestHeatmap).forEach(tile => {
        // Color based on cache status:
        // - Green: mostly L1 hits (fastest)
        // - Blue: mostly L2 hits (fast)
        // - Red/Magenta: mostly misses (slow - had to render)
        const total = tile.count || 1;
        const l1Ratio = (tile.l1_hits || 0) / total;
        const l2Ratio = (tile.l2_hits || 0) / total;
        const missRatio = (tile.misses || 0) / total;
        
        let fillColor, strokeColor;
        if (l1Ratio >= 0.5) {
            // Mostly L1 hits - green
            fillColor = '#22c55e';
            strokeColor = '#4ade80';
        } else if (l2Ratio >= 0.5) {
            // Mostly L2 hits - blue
            fillColor = '#3b82f6';
            strokeColor = '#60a5fa';
        } else if (missRatio >= 0.7) {
            // Mostly misses - red/magenta
            fillColor = '#ef4444';
            strokeColor = '#f87171';
        } else {
            // Mixed - orange/yellow
            fillColor = '#f59e0b';
            strokeColor = '#fbbf24';
        }
        
        // Use actual tile bounds from server
        const bounds = [
            [tile.min_lat, tile.min_lon],
            [tile.max_lat, tile.max_lon]
        ];
        
        // Calculate fade multiplier based on age (1.0 = fresh, 0.0 = about to disappear)
        let fadeMult = 1.0;
        if (fadeEnabled && tile.lastSeen) {
            const age = now - tile.lastSeen;
            const maxAge = fadeTimeoutSecs * 1000;
            fadeMult = Math.max(0, 1.0 - (age / maxAge));
        }
        
        // Base opacity adjusted by fade
        const baseFillOpacity = 0.15 + (Math.min(tile.count, 20) / 20) * 0.15;  // 0.15 - 0.30
        const baseStrokeOpacity = 0.4;
        
        const fillOpacity = baseFillOpacity * fadeMult;
        const strokeOpacity = baseStrokeOpacity * fadeMult;
        
        // Skip completely faded tiles
        if (fadeMult <= 0.05) return;
        
        const rect = L.rectangle(bounds, {
            fillColor: fillColor,
            fillOpacity: fillOpacity,
            color: strokeColor,
            weight: 1,
            opacity: strokeOpacity
        });
        
        // Tooltip with cache breakdown
        const l1Pct = (l1Ratio * 100).toFixed(0);
        const l2Pct = (l2Ratio * 100).toFixed(0);
        const missPct = (missRatio * 100).toFixed(0);
        rect.bindTooltip(
            `${tile.count} requests<br>L1: ${l1Pct}% | L2: ${l2Pct}% | Miss: ${missPct}%`,
            {
                permanent: false,
                direction: 'top',
                className: 'heatmap-tooltip'
            }
        );
        
        heatmapLayer.addLayer(rect);
    });
}

// Update minimap statistics display
function updateMinimapStats() {
    const requestCountEl = document.getElementById('minimap-request-count');
    const hotspotCountEl = document.getElementById('minimap-hotspot-count');
    
    if (requestCountEl) {
        requestCountEl.textContent = totalTileRequests.toLocaleString();
    }
    
    if (hotspotCountEl) {
        // Count hotspots (locations with >5 requests)
        const hotspots = Object.values(tileRequestHeatmap).filter(p => p.count > 5).length;
        hotspotCountEl.textContent = hotspots.toLocaleString();
    }
}

// Clear the heatmap data (also clears server-side)
async function clearHeatmap() {
    tileRequestHeatmap = {};
    lastServerCounts = {};
    totalTileRequests = 0;
    
    if (heatmapLayer) {
        heatmapLayer.clearLayers();
    }
    
    updateMinimapStats();
    
    // Also clear server-side heatmap
    try {
        await fetch(`${API_BASE}/api/tile-heatmap/clear`, { method: 'POST' });
        console.log('Heatmap cleared (client + server)');
    } catch (error) {
        console.error('Failed to clear server heatmap:', error);
    }
}

// Fetch heatmap data from the server API
async function fetchServerHeatmap() {
    try {
        const response = await fetch(`${API_BASE}/api/tile-heatmap`);
        if (!response.ok) return;
        
        const data = await response.json();
        const now = Date.now();
        
        // Track total requests
        totalTileRequests = data.total_requests || 0;
        
        if (data.cells && Array.isArray(data.cells)) {
            if (fadeEnabled) {
                // FADE MODE: Only show tiles that have NEW requests since last fetch
                // Each new request creates a fresh tile that fades out independently
                data.cells.forEach(cell => {
                    const key = `${cell.min_lon},${cell.min_lat},${cell.max_lon},${cell.max_lat}`;
                    const lastCount = lastServerCounts[key] || 0;
                    const newRequests = cell.count - lastCount;
                    
                    if (newRequests > 0) {
                        // Create a unique key for this "burst" of requests so they fade independently
                        const burstKey = `${key}_${now}`;
                        tileRequestHeatmap[burstKey] = {
                            min_lon: cell.min_lon,
                            min_lat: cell.min_lat,
                            max_lon: cell.max_lon,
                            max_lat: cell.max_lat,
                            count: newRequests,
                            l1_hits: Math.max(0, (cell.l1_hits || 0) - (lastServerCounts[`${key}_l1`] || 0)),
                            l2_hits: Math.max(0, (cell.l2_hits || 0) - (lastServerCounts[`${key}_l2`] || 0)),
                            misses: Math.max(0, (cell.misses || 0) - (lastServerCounts[`${key}_miss`] || 0)),
                            lastSeen: now
                        };
                    }
                    
                    // Update last known counts
                    lastServerCounts[key] = cell.count;
                    lastServerCounts[`${key}_l1`] = cell.l1_hits || 0;
                    lastServerCounts[`${key}_l2`] = cell.l2_hits || 0;
                    lastServerCounts[`${key}_miss`] = cell.misses || 0;
                });
            } else {
                // NON-FADE MODE: Show cumulative server data as before
                tileRequestHeatmap = {};
                data.cells.forEach(cell => {
                    const key = `${cell.min_lon},${cell.min_lat},${cell.max_lon},${cell.max_lat}`;
                    tileRequestHeatmap[key] = {
                        min_lon: cell.min_lon,
                        min_lat: cell.min_lat,
                        max_lon: cell.max_lon,
                        max_lat: cell.max_lat,
                        count: cell.count,
                        l1_hits: cell.l1_hits || 0,
                        l2_hits: cell.l2_hits || 0,
                        misses: cell.misses || 0,
                        lastSeen: now
                    };
                    
                    // Also update last known counts for when fade is re-enabled
                    lastServerCounts[key] = cell.count;
                    lastServerCounts[`${key}_l1`] = cell.l1_hits || 0;
                    lastServerCounts[`${key}_l2`] = cell.l2_hits || 0;
                    lastServerCounts[`${key}_miss`] = cell.misses || 0;
                });
            }
        }
        
        // Update display (only if fade is disabled - fade mode updates via interval)
        if (!fadeEnabled) {
            updateHeatmapDisplay();
        }
        updateMinimapStats();
    } catch (error) {
        // Silently fail - server might not support this endpoint yet
        console.debug('Heatmap API not available:', error.message);
    }
}

// Hook into tile loading to record requests (for local display before server update)
function setupTileRequestTracking() {
    // Periodically sync minimap viewport
    map.on('moveend', syncMinimapViewport);
    map.on('zoomend', syncMinimapViewport);
    
    // Poll server for heatmap updates every 2 seconds
    setInterval(fetchServerHeatmap, 2000);
    
    // Initial fetch
    fetchServerHeatmap();
}

// Initialize minimap after main map is ready
document.addEventListener('DOMContentLoaded', () => {
    // Wait a bit for main map to initialize
    setTimeout(() => {
        initMinimap();
        setupTileRequestTracking();
    }, 500);
    
    // Initialize downloads widget
    initDownloadsWidget();
    
    // Initialize database panel
    initDatabasePanel();
    
    // Initialize ingestion widget
    initIngestionWidget();
});

// ============================================================================
// Downloads Widget
// ============================================================================

let downloadsWidgetInterval = null;

// Initialize the downloads widget
function initDownloadsWidget() {
    // Initial fetch
    fetchDownloadsStatus();
    
    // Refresh every 5 seconds
    downloadsWidgetInterval = setInterval(fetchDownloadsStatus, 5000);
}

// Fetch downloads status from the downloader service
async function fetchDownloadsStatus() {
    try {
        // Fetch status and downloads in parallel
        const [statusResponse, downloadsResponse, scheduleResponse] = await Promise.all([
            fetch(`${DOWNLOADER_URL}/status`).catch((e) => { console.error('Status fetch failed:', e); return null; }),
            fetch(`${DOWNLOADER_URL}/downloads?limit=5`).catch((e) => { console.error('Downloads fetch failed:', e); return null; }),
            fetch(`${DOWNLOADER_URL}/schedule`).catch((e) => { console.error('Schedule fetch failed:', e); return null; })
        ]);
        
        // Update service status
        if (statusResponse && statusResponse.ok) {
            const status = await statusResponse.json();
            updateDownloaderServiceStatus(true, status);
            updateDownloadsStats(status.stats);
        } else {
            updateDownloaderServiceStatus(false);
        }
        
        // Update active downloads list
        if (downloadsResponse && downloadsResponse.ok) {
            const downloads = await downloadsResponse.json();
            updateDownloadsList(downloads);
        }
        
        // Update next availability - pass full schedule for better display
        if (scheduleResponse && scheduleResponse.ok) {
            const schedule = await scheduleResponse.json();
            updateNextAvailability(schedule);
        }
        
    } catch (error) {
        console.error('Error fetching downloads status:', error);
        updateDownloaderServiceStatus(false);
    }
}

// Update downloader service status indicator
function updateDownloaderServiceStatus(online, status = null) {
    const dot = document.getElementById('downloader-status-dot');
    const text = document.getElementById('downloader-status-text');
    
    if (dot) {
        dot.className = 'downloads-status-dot ' + (online ? 'online' : 'offline');
    }
    
    if (text) {
        if (online && status) {
            text.textContent = status.status || 'Online';
        } else {
            text.textContent = 'Offline';
        }
    }
}

// Update downloads statistics
function updateDownloadsStats(stats) {
    if (!stats) return;
    
    const pendingEl = document.getElementById('dl-pending');
    const activeEl = document.getElementById('dl-active');
    const completedEl = document.getElementById('dl-completed');
    const failedEl = document.getElementById('dl-failed');
    const bytesEl = document.getElementById('dl-total-bytes');
    
    if (pendingEl) pendingEl.textContent = stats.pending ?? 0;
    if (activeEl) activeEl.textContent = stats.in_progress ?? 0;
    if (completedEl) completedEl.textContent = stats.completed ?? 0;
    if (failedEl) failedEl.textContent = stats.failed ?? 0;
    if (bytesEl) bytesEl.textContent = formatBytes(stats.total_bytes_downloaded ?? 0);
}

// Update active downloads list
function updateDownloadsList(downloads) {
    const listEl = document.getElementById('downloads-list');
    if (!listEl) return;
    
    // Combine active and pending, prioritize active
    const active = downloads.active || [];
    const pending = downloads.pending || [];
    const items = [...active, ...pending.slice(0, 3 - active.length)];
    
    if (items.length === 0) {
        listEl.innerHTML = '<div class="downloads-empty">No active downloads</div>';
        return;
    }
    
    listEl.innerHTML = items.map(d => {
        const progressPercent = d.progress_percent ?? 0;
        const filename = d.filename || extractFilename(d.url);
        const model = d.model || extractModel(filename);
        
        return `
            <div class="download-item">
                <div class="download-item-header">
                    <span class="download-item-name">${truncateFilename(filename)}</span>
                    ${model ? `<span class="download-item-model">${model}</span>` : ''}
                </div>
                ${d.status === 'in_progress' ? `
                <div class="download-item-progress">
                    <div class="download-progress-bar">
                        <div class="download-progress-fill" style="width: ${progressPercent}%"></div>
                    </div>
                    <div class="download-progress-text">
                        <span>${formatBytes(d.downloaded_bytes ?? 0)}</span>
                        <span>${progressPercent.toFixed(0)}%</span>
                    </div>
                </div>
                ` : `
                <div class="download-progress-text">
                    <span style="color: #60a5fa;">Pending</span>
                </div>
                `}
            </div>
        `;
    }).join('');
}

// Update next data availability section
function updateNextAvailability(schedule) {
    const listEl = document.getElementById('downloads-next-list');
    if (!listEl) return;
    
    const models = schedule.models || [];
    const nextChecks = schedule.next_checks || [];
    
    if (models.length === 0) {
        listEl.innerHTML = '<div class="downloads-empty">No models configured</div>';
        return;
    }
    
    // Create display items - distinguish observation vs forecast models
    const displayItems = models
        .filter(m => m.enabled)
        .map(m => {
            const isObservation = !m.cycles || m.cycles.length === 0;
            
            if (isObservation) {
                // Observation model (GOES, MRMS) - calculate next check time
                const pollSecs = m.poll_interval_secs || 300;
                const nextCheckInfo = calculateNextCheckTime(pollSecs);
                return {
                    model: m.id.toUpperCase(),
                    info: nextCheckInfo,
                    isObservation: true
                };
            } else {
                // Forecast model (GFS, HRRR) - find next check info
                const check = nextChecks.find(c => c.model === m.id);
                if (check) {
                    return {
                        model: m.id.toUpperCase(),
                        info: `${check.next_cycle} - ${check.expected_available}`,
                        isObservation: false
                    };
                } else {
                    return {
                        model: m.id.toUpperCase(),
                        info: `Cycles: ${m.cycles.map(c => c + 'Z').join(', ')}`,
                        isObservation: false
                    };
                }
            }
        });
    
    // Show all items (compact widget)
    listEl.innerHTML = displayItems.map(item => {
        const timeClass = item.isObservation ? 'downloads-next-continuous' : '';
        return `
            <div class="downloads-next-item">
                <span class="downloads-next-model">${item.model}</span>
                <span class="downloads-next-time ${timeClass}">${item.info}</span>
            </div>
        `;
    }).join('');
}

// Calculate next check time based on poll interval
function calculateNextCheckTime(pollIntervalSecs) {
    const now = new Date();
    const currentMinute = now.getMinutes();
    const currentSecond = now.getSeconds();
    
    // Calculate seconds into current interval
    const pollMins = pollIntervalSecs / 60;
    const secsIntoInterval = (currentMinute % pollMins) * 60 + currentSecond;
    const secsUntilNext = pollIntervalSecs - secsIntoInterval;
    
    // Calculate next check time
    const nextCheck = new Date(now.getTime() + secsUntilNext * 1000);
    
    // Format time as HH:MM (local time)
    const timeStr = nextCheck.toLocaleTimeString('en-US', { 
        hour: 'numeric', 
        minute: '2-digit',
        hour12: true 
    }).toLowerCase();
    
    // Format minutes away
    const minsAway = Math.ceil(secsUntilNext / 60);
    const minsAwayStr = minsAway <= 1 ? '<1 min' : `${minsAway} min`;
    
    return `${timeStr} (${minsAwayStr})`;
}

// Helper: Extract filename from URL
function extractFilename(url) {
    if (!url) return 'Unknown';
    try {
        const parts = url.split('/');
        return parts[parts.length - 1] || 'Unknown';
    } catch {
        return 'Unknown';
    }
}

// Helper: Extract model from filename
function extractModel(filename) {
    if (!filename) return null;
    const lower = filename.toLowerCase();
    if (lower.includes('gfs')) return 'GFS';
    if (lower.includes('hrrr')) return 'HRRR';
    if (lower.includes('goes')) return 'GOES';
    if (lower.includes('mrms')) return 'MRMS';
    return null;
}

// Helper: Truncate filename for display
function truncateFilename(filename, maxLen = 40) {
    if (!filename || filename.length <= maxLen) return filename;
    // Keep extension visible
    const ext = filename.split('.').pop();
    const name = filename.substring(0, filename.length - ext.length - 1);
    const truncLen = maxLen - ext.length - 4; // 4 for "..." and "."
    return name.substring(0, truncLen) + '...' + ext;
}

// Cleanup on page unload
window.addEventListener('beforeunload', () => {
    if (downloadsWidgetInterval) {
        clearInterval(downloadsWidgetInterval);
    }
    if (dataPanelInterval) {
        clearInterval(dataPanelInterval);
    }
    if (ingestionWidgetInterval) {
        clearInterval(ingestionWidgetInterval);
    }
});

// ============================================================================
// Ingestion Pipeline Widget
// ============================================================================

let ingestionWidgetInterval = null;

// Initialize the ingestion widget
function initIngestionWidget() {
    // Initial fetch
    fetchIngestionStatus();
    
    // Refresh every 2 seconds for real-time updates
    ingestionWidgetInterval = setInterval(fetchIngestionStatus, 2000);
}

// Toggle ingestion widget expanded/collapsed
function toggleIngestionWidget() {
    const widget = document.getElementById('ingestion-widget');
    if (widget) {
        widget.classList.toggle('collapsed');
    }
}

// Fetch ingestion status from the WMS API
async function fetchIngestionStatus() {
    try {
        const response = await fetch(`${API_BASE}/api/admin/ingestion/active`);
        
        if (response.ok) {
            const data = await response.json();
            updateIngestionWidget(data);
        } else {
            updateIngestionWidgetOffline();
        }
    } catch (error) {
        console.error('Error fetching ingestion status:', error);
        updateIngestionWidgetOffline();
    }
}

// Update the ingestion widget with data
function updateIngestionWidget(data) {
    const statusDot = document.getElementById('ingestion-status-dot');
    const statusText = document.getElementById('ingestion-status-text');
    const activeCount = document.getElementById('ingestion-active-count');
    const activeList = document.getElementById('ingestion-active-list');
    const recentList = document.getElementById('ingestion-recent-list');
    const recentStats = document.getElementById('ingestion-recent-stats');
    
    const isActive = data.active && data.active.length > 0;
    
    // Update status indicator
    if (statusDot) {
        statusDot.className = 'ingestion-status-dot ' + (isActive ? 'active' : 'idle');
    }
    if (statusText) {
        statusText.textContent = isActive ? 'Processing' : 'Idle';
    }
    
    // Update active count
    if (activeCount) {
        activeCount.textContent = data.active ? data.active.length : 0;
    }
    
    // Update active ingestions list
    if (activeList) {
        if (data.active && data.active.length > 0) {
            activeList.innerHTML = data.active.map(item => {
                const filename = extractFilename(item.file_path);
                const elapsed = formatElapsedTime(item.started_at);
                const model = extractModelFromPath(item.file_path) || 'unknown';
                return `
                    <div class="ingestion-item">
                        <div class="ingestion-item-header">
                            <span class="ingestion-item-model">${model}</span>
                            <span class="ingestion-item-status">${formatIngestionStatus(item.status)}</span>
                        </div>
                        <div class="ingestion-item-file" title="${item.file_path}">${filename}</div>
                        <div class="ingestion-item-progress">
                            <span>${elapsed}</span>
                        </div>
                    </div>
                `;
            }).join('');
        } else {
            activeList.innerHTML = '<div class="ingestion-empty">No active ingestions</div>';
        }
    }
    
    // Update recent stats
    if (recentStats && data.recent) {
        const successful = data.recent.filter(r => r.success).length;
        const failed = data.recent.filter(r => !r.success).length;
        if (data.recent.length > 0) {
            const avgTime = data.recent.reduce((sum, r) => sum + r.duration_ms, 0) / data.recent.length;
            recentStats.textContent = `${successful}/${data.recent.length} ok, avg ${formatDuration(avgTime)}`;
        } else {
            recentStats.textContent = '--';
        }
    }
    
    // Update recent ingestions list
    if (recentList) {
        if (data.recent && data.recent.length > 0) {
            recentList.innerHTML = data.recent.slice(0, 5).map(item => {
                const timeClass = item.success ? '' : ' failed';
                // Extract model name from file_path (e.g., "hrrr/..." -> "hrrr")
                const model = extractModelFromPath(item.file_path) || 'unknown';
                // Use datasets_registered (API field) or parameters count
                const count = item.datasets_registered ?? item.parameters?.length ?? 0;
                return `
                    <div class="ingestion-recent-item">
                        <span class="ingestion-recent-model">${model}</span>
                        <span class="ingestion-recent-params">${count} datasets</span>
                        <span class="ingestion-recent-time${timeClass}">${formatDuration(item.duration_ms)}</span>
                    </div>
                `;
            }).join('');
        } else {
            recentList.innerHTML = '<div class="ingestion-empty">No recent ingestions</div>';
        }
    }
}

// Update widget when API is offline
function updateIngestionWidgetOffline() {
    const statusDot = document.getElementById('ingestion-status-dot');
    const statusText = document.getElementById('ingestion-status-text');
    
    if (statusDot) {
        statusDot.className = 'ingestion-status-dot';
    }
    if (statusText) {
        statusText.textContent = 'Offline';
    }
}

// Format ingestion status for display
function formatIngestionStatus(status) {
    const statusMap = {
        'parsing': 'Parsing',
        'parsing_netcdf': 'Parsing NC',
        'shredding': 'Shredding',
        'storing': 'Storing',
        'registering': 'Registering'
    };
    return statusMap[status] || status;
}

// Format elapsed time since start
function formatElapsedTime(startedAt) {
    try {
        const start = new Date(startedAt);
        const now = new Date();
        const ms = now - start;
        return formatDuration(ms);
    } catch {
        return '--';
    }
}

// Format duration in ms to human readable
function formatDuration(ms) {
    if (ms < 1000) return `${Math.round(ms)}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    const mins = Math.floor(ms / 60000);
    const secs = Math.round((ms % 60000) / 1000);
    return `${mins}m ${secs}s`;
}

// Extract filename from path
function extractFilename(path) {
    if (!path) return 'unknown';
    return path.split('/').pop() || path;
}

// Extract model name from ingestion file path
// e.g., "grids/hrrr/2024/..." -> "hrrr"
// e.g., "/data/hrrr.t00z.grib2" -> "hrrr"
// e.g., "/data/downloads/goes18_OR_ABI-..." -> "goes18"
// e.g., "/data/downloads/mrms_MRMS_..." -> "mrms"
// e.g., "/data/downloads/hrrr_20251229_17z_f006.grib2" -> "hrrr"
function extractModelFromPath(path) {
    if (!path) return null;
    
    // Try to extract from grids/model/ pattern
    const gridsMatch = path.match(/grids\/([^/]+)\//);
    if (gridsMatch) return gridsMatch[1].toUpperCase();
    
    // Extract filename from path
    const filename = path.split('/').pop() || '';
    
    // Try to match download filename pattern: {model}_{...}
    // e.g., "goes18_OR_ABI...", "mrms_MRMS_...", "hrrr_20251229_..."
    const downloadMatch = filename.match(/^(goes16|goes18|mrms|hrrr|gfs)_/i);
    if (downloadMatch) return downloadMatch[1].toUpperCase();
    
    // Try to extract model name from grib2 filename pattern (e.g., hrrr.t00z.grib2)
    const gribMatch = filename.match(/^([a-zA-Z]+)\./);
    if (gribMatch) return gribMatch[1].toUpperCase();
    
    // Try to extract from common path patterns
    const pathParts = path.split('/');
    for (const part of pathParts) {
        const lower = part.toLowerCase();
        if (['hrrr', 'gfs', 'mrms', 'goes16', 'goes18'].includes(lower)) {
            return part.toUpperCase();
        }
    }
    
    return null;
}

// ============================================================================
// Data Panel Widget (PostgreSQL + MinIO Tree View)
// ============================================================================

let dataPanelInterval = null;
let dbTreeData = null;  // Cached PostgreSQL data
let minioTreeData = null;  // Cached MinIO data
let expandedNodes = new Set();  // Track expanded nodes across refreshes

// Initialize the data panel
function initDataPanel() {
    // Initial fetch for both panels
    fetchDatabaseTree();
    fetchMinioTree();
    checkSyncStatus();
    
    // Refresh every 15 seconds
    dataPanelInterval = setInterval(() => {
        fetchDatabaseTree();
        fetchMinioTree();
        checkSyncStatus();
    }, 15000);
}

// Alias for backward compatibility
function initDatabasePanel() {
    initDataPanel();
}

// Toggle section collapse
function toggleDataSection(sectionId) {
    const section = document.getElementById(sectionId);
    if (section) {
        section.classList.toggle('collapsed');
    }
}

// ============================================================================
// PostgreSQL Tree
// ============================================================================

async function fetchDatabaseTree() {
    try {
        const response = await fetch(`${API_BASE}/api/admin/database/details`);
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        
        const data = await response.json();
        dbTreeData = data;
        renderDatabaseTree(data);
        updateDbSummary(data);
    } catch (error) {
        console.error('Error fetching database details:', error);
        const treeEl = document.getElementById('database-tree');
        if (treeEl) {
            treeEl.innerHTML = '<div class="tree-loading">Error loading data</div>';
        }
    }
}

function updateDbSummary(data) {
    const countEl = document.getElementById('db-total-datasets');
    const sizeEl = document.getElementById('db-total-size');
    
    if (countEl) countEl.textContent = (data.total_datasets || 0).toLocaleString();
    if (sizeEl) sizeEl.textContent = formatBytes(data.total_size_bytes || 0);
}

function renderDatabaseTree(data) {
    const treeEl = document.getElementById('database-tree');
    if (!treeEl) return;
    
    if (!data.models || data.models.length === 0) {
        treeEl.innerHTML = '<div class="tree-empty">No data ingested yet</div>';
        return;
    }
    
    treeEl.innerHTML = data.models.map(model => renderDbModelNode(model)).join('');
}

function renderDbModelNode(model) {
    const modelId = `db-model-${model.model}`;
    const isExpanded = expandedNodes.has(modelId);
    const datasetCount = model.dataset_count || 0;
    const sizeStr = formatBytes(model.total_size_bytes || 0);
    
    return `
        <div class="tree-node ${isExpanded ? 'expanded' : ''}" data-model="${model.model}" id="${modelId}">
            <div class="tree-node-row" data-level="0" onclick="toggleDbModelNode('${modelId}', '${model.model}')">
                <span class="tree-node-expander ${isExpanded ? 'expanded' : ''}">▶</span>
                <span class="tree-node-icon">📦</span>
                <span class="tree-node-name">${model.model.toUpperCase()}</span>
                <div class="tree-node-meta">
                    <span class="tree-node-count">${datasetCount}</span>
                    <span class="tree-node-size">${sizeStr}</span>
                </div>
            </div>
            <div class="tree-node-children">
                ${(model.parameters || []).map(param => renderDbParamNode(model.model, param)).join('')}
            </div>
        </div>
    `;
}

function renderDbParamNode(modelName, param) {
    const paramId = `db-param-${modelName}-${param.parameter}`;
    const isExpanded = expandedNodes.has(paramId);
    const sizeStr = formatBytes(param.total_size_bytes || 0);
    
    return `
        <div class="tree-node ${isExpanded ? 'expanded' : ''}" id="${paramId}">
            <div class="tree-node-row" data-level="1" onclick="toggleDbParamNode('${paramId}', '${modelName}', '${param.parameter}')">
                <span class="tree-node-expander ${isExpanded ? 'expanded' : ''}">▶</span>
                <span class="tree-node-icon">📋</span>
                <span class="tree-node-name">${param.parameter}</span>
                <div class="tree-node-meta">
                    <span class="tree-node-count">${param.count}</span>
                    <span class="tree-node-size">${sizeStr}</span>
                </div>
            </div>
            <div class="tree-node-children" id="${paramId}-children">
                ${isExpanded ? '' : '<div class="tree-loading">Loading...</div>'}
            </div>
        </div>
    `;
}

function toggleDbModelNode(nodeId, modelName) {
    const node = document.getElementById(nodeId);
    if (!node) return;
    
    const isExpanded = node.classList.contains('expanded');
    if (isExpanded) {
        node.classList.remove('expanded');
        expandedNodes.delete(nodeId);
        node.querySelector('.tree-node-expander').classList.remove('expanded');
    } else {
        node.classList.add('expanded');
        expandedNodes.add(nodeId);
        node.querySelector('.tree-node-expander').classList.add('expanded');
    }
}

async function toggleDbParamNode(nodeId, modelName, paramName) {
    const node = document.getElementById(nodeId);
    if (!node) return;
    
    const isExpanded = node.classList.contains('expanded');
    const childrenEl = document.getElementById(`${nodeId}-children`);
    
    if (isExpanded) {
        node.classList.remove('expanded');
        expandedNodes.delete(nodeId);
        node.querySelector('.tree-node-expander').classList.remove('expanded');
    } else {
        node.classList.add('expanded');
        expandedNodes.add(nodeId);
        node.querySelector('.tree-node-expander').classList.add('expanded');
        
        // Fetch datasets if not already loaded
        if (childrenEl && childrenEl.querySelector('.tree-loading')) {
            try {
                const response = await fetch(`${API_BASE}/api/admin/database/datasets/${modelName}/${paramName}`);
                if (!response.ok) throw new Error(`HTTP ${response.status}`);
                
                const datasets = await response.json();
                childrenEl.innerHTML = datasets.map(ds => renderDatasetNode(ds)).join('');
            } catch (error) {
                console.error('Error fetching datasets:', error);
                childrenEl.innerHTML = '<div class="tree-loading">Error loading datasets</div>';
            }
        }
    }
}

function renderDatasetNode(dataset) {
    const validTime = dataset.valid_time ? formatDbTime(dataset.valid_time) : '--';
    const sizeStr = formatBytes(dataset.file_size || 0);
    const level = dataset.level || 'surface';
    const fHour = dataset.forecast_hour !== undefined ? `f${String(dataset.forecast_hour).padStart(3, '0')}` : '';
    
    // Build display name: valid time + forecast hour + level
    const displayParts = [validTime];
    if (fHour) displayParts.push(fHour);
    if (level !== 'surface') displayParts.push(level);
    const displayName = displayParts.join(' | ');
    
    return `
        <div class="tree-node">
            <div class="tree-node-row" data-level="2">
                <span class="tree-node-expander empty"></span>
                <span class="tree-node-icon">📄</span>
                <span class="tree-node-name" title="${dataset.storage_path || ''}">${displayName}</span>
                <div class="tree-node-meta">
                    <span class="tree-node-size">${sizeStr}</span>
                </div>
            </div>
        </div>
    `;
}

// ============================================================================
// MinIO Tree
// ============================================================================

async function fetchMinioTree() {
    try {
        const response = await fetch(`${API_BASE}/api/admin/storage/tree`);
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        
        const data = await response.json();
        minioTreeData = data;
        renderMinioTree(data);
        updateMinioSummary(data);
    } catch (error) {
        console.error('Error fetching MinIO tree:', error);
        const treeEl = document.getElementById('minio-tree');
        if (treeEl) {
            treeEl.innerHTML = '<div class="tree-loading">Error loading storage</div>';
        }
    }
}

function updateMinioSummary(data) {
    const countEl = document.getElementById('minio-total-objects');
    const sizeEl = document.getElementById('minio-total-size');
    
    if (countEl) countEl.textContent = (data.total_objects || 0).toLocaleString();
    if (sizeEl) sizeEl.textContent = formatBytes(data.total_size || 0);
}

function renderMinioTree(data) {
    const treeEl = document.getElementById('minio-tree');
    if (!treeEl) return;
    
    // API returns 'nodes' not 'tree'
    const nodes = data.nodes || data.tree || [];
    if (nodes.length === 0) {
        treeEl.innerHTML = '<div class="tree-empty">No files in storage</div>';
        return;
    }
    
    treeEl.innerHTML = nodes.map(node => renderMinioNode(node, 0)).join('');
}

function renderMinioNode(node, level) {
    const nodeId = `minio-${node.path.replace(/[^a-zA-Z0-9]/g, '-')}`;
    const isExpanded = expandedNodes.has(nodeId);
    const isFolder = node.node_type === 'directory';
    
    if (isFolder) {
        const childCount = countMinioChildren(node);
        const totalSize = sumMinioSize(node);
        
        return `
            <div class="tree-node ${isExpanded ? 'expanded' : ''}" id="${nodeId}">
                <div class="tree-node-row" data-level="${level}" onclick="toggleMinioNode('${nodeId}')">
                    <span class="tree-node-expander ${isExpanded ? 'expanded' : ''}">▶</span>
                    <span class="tree-node-icon folder"></span>
                    <span class="tree-node-name">${node.name}</span>
                    <div class="tree-node-meta">
                        <span class="tree-node-count">${childCount}</span>
                        <span class="tree-node-size">${formatBytes(totalSize)}</span>
                    </div>
                </div>
                <div class="tree-node-children">
                    ${(node.children || []).map(child => renderMinioNode(child, level + 1)).join('')}
                </div>
            </div>
        `;
    } else {
        // File node
        const ext = node.name.split('.').pop().toLowerCase();
        let iconClass = 'file-other';
        if (ext === 'grib2' || ext === 'grb2') iconClass = 'file-grib';
        else if (ext === 'nc') iconClass = 'file-nc';
        
        return `
            <div class="tree-node" id="${nodeId}">
                <div class="tree-node-row" data-level="${level}">
                    <span class="tree-node-expander empty"></span>
                    <span class="tree-node-icon ${iconClass}"></span>
                    <span class="tree-node-name" title="${node.path}">${node.name}</span>
                    <div class="tree-node-meta">
                        <span class="tree-node-size">${formatBytes(node.size || 0)}</span>
                    </div>
                </div>
            </div>
        `;
    }
}

function toggleMinioNode(nodeId) {
    const node = document.getElementById(nodeId);
    if (!node) return;
    
    const isExpanded = node.classList.contains('expanded');
    if (isExpanded) {
        node.classList.remove('expanded');
        expandedNodes.delete(nodeId);
        node.querySelector('.tree-node-expander').classList.remove('expanded');
    } else {
        node.classList.add('expanded');
        expandedNodes.add(nodeId);
        node.querySelector('.tree-node-expander').classList.add('expanded');
    }
}

function countMinioChildren(node) {
    if (!node.children) return 0;
    return node.children.reduce((acc, child) => {
        if (child.node_type === 'directory') {
            return acc + countMinioChildren(child);
        }
        return acc + 1;
    }, 0);
}

function sumMinioSize(node) {
    if (node.node_type === 'file') return node.size || 0;
    if (!node.children) return 0;
    return node.children.reduce((acc, child) => acc + sumMinioSize(child), 0);
}

// ============================================================================
// Sync Status
// ============================================================================

async function checkSyncStatus() {
    const statusEl = document.getElementById('data-sync-status');
    const textEl = document.getElementById('sync-status-text');
    const actionsEl = document.getElementById('sync-actions');
    if (!statusEl || !textEl) return;
    
    try {
        const response = await fetch(`${API_BASE}/api/admin/sync/status`);
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        
        const data = await response.json();
        
        statusEl.className = 'data-sync-status';
        const orphanDb = data.orphan_db_records || 0;
        const orphanMinio = data.orphan_minio_objects || 0;
        
        if (orphanDb === 0 && orphanMinio === 0) {
            statusEl.classList.add('synced');
            textEl.textContent = 'DB and storage in sync';
            if (actionsEl) actionsEl.style.display = 'none';
        } else {
            statusEl.classList.add('warning');
            const issues = [];
            if (orphanDb > 0) issues.push(`${orphanDb} orphan DB`);
            if (orphanMinio > 0) issues.push(`${orphanMinio} orphan files`);
            textEl.textContent = issues.join(', ');
            // Show sync buttons when there are orphans
            if (actionsEl) actionsEl.style.display = 'flex';
        }
    } catch (error) {
        console.error('Error checking sync status:', error);
        statusEl.className = 'data-sync-status error';
        textEl.textContent = 'Could not check sync';
        if (actionsEl) actionsEl.style.display = 'none';
    }
}

// ============================================================================
// Sync Preview and Run
// ============================================================================

let currentSyncPreview = null;

async function showSyncPreview() {
    const overlay = document.getElementById('sync-modal-overlay');
    const body = document.getElementById('sync-modal-body');
    const confirmBtn = document.getElementById('sync-modal-confirm');
    
    if (!overlay || !body) return;
    
    // Show modal with loading state
    overlay.classList.add('visible');
    body.innerHTML = '<div class="tree-loading">Loading preview...</div>';
    if (confirmBtn) confirmBtn.disabled = true;
    
    try {
        const response = await fetch(`${API_BASE}/api/admin/sync/preview`);
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        
        const data = await response.json();
        currentSyncPreview = data;
        
        renderSyncPreview(data);
        if (confirmBtn) {
            confirmBtn.disabled = (data.orphan_db_paths.length === 0 && data.orphan_minio_paths.length === 0);
        }
    } catch (error) {
        console.error('Error fetching sync preview:', error);
        body.innerHTML = `<div class="sync-result error">Failed to load preview: ${error.message}</div>`;
    }
}

function renderSyncPreview(data) {
    const body = document.getElementById('sync-modal-body');
    if (!body) return;
    
    const orphanDbCount = data.orphan_db_paths?.length || 0;
    const orphanMinioCount = data.orphan_minio_paths?.length || 0;
    
    let html = '';
    
    // Summary
    html += `<div style="margin-bottom: 16px; font-size: 11px; color: #9ca3af;">
        Checked ${data.db_records_checked || 0} DB records and ${data.minio_objects_checked || 0} MinIO objects.
    </div>`;
    
    // Orphan DB records section
    html += `<div class="sync-modal-section">
        <div class="sync-modal-section-title">
            Orphan DB Records (missing from MinIO)
            <span class="sync-modal-section-count">${orphanDbCount}</span>
        </div>
        <div class="sync-modal-list">`;
    
    if (orphanDbCount === 0) {
        html += '<div class="sync-modal-empty">No orphan database records</div>';
    } else {
        // Show up to 50 paths
        const dbPathsToShow = data.orphan_db_paths.slice(0, 50);
        html += dbPathsToShow.map(path => `<div class="sync-modal-list-item">${escapeHtml(path)}</div>`).join('');
        if (orphanDbCount > 50) {
            html += `<div class="sync-modal-list-item" style="color: #6b7280; font-style: italic;">... and ${orphanDbCount - 50} more</div>`;
        }
    }
    html += '</div></div>';
    
    // Orphan MinIO objects section
    html += `<div class="sync-modal-section">
        <div class="sync-modal-section-title">
            Orphan MinIO Files (missing from DB)
            <span class="sync-modal-section-count">${orphanMinioCount}</span>
        </div>
        <div class="sync-modal-list">`;
    
    if (orphanMinioCount === 0) {
        html += '<div class="sync-modal-empty">No orphan MinIO files</div>';
    } else {
        const minioPathsToShow = data.orphan_minio_paths.slice(0, 50);
        html += minioPathsToShow.map(path => `<div class="sync-modal-list-item">${escapeHtml(path)}</div>`).join('');
        if (orphanMinioCount > 50) {
            html += `<div class="sync-modal-list-item" style="color: #6b7280; font-style: italic;">... and ${orphanMinioCount - 50} more</div>`;
        }
    }
    html += '</div></div>';
    
    // Warning message
    if (orphanDbCount > 0 || orphanMinioCount > 0) {
        html += `<div style="margin-top: 16px; padding: 10px; background: rgba(245, 158, 11, 0.1); border: 1px solid rgba(245, 158, 11, 0.3); border-radius: 4px; font-size: 11px; color: #f59e0b;">
            <strong>Warning:</strong> Clicking "Sync Now" will delete ${orphanDbCount} orphan DB records and ${orphanMinioCount} orphan MinIO files. This action cannot be undone.
        </div>`;
    }
    
    body.innerHTML = html;
}

function closeSyncModal(event) {
    // If called from overlay click, only close if clicking directly on overlay
    if (event && event.target !== event.currentTarget) return;
    
    const overlay = document.getElementById('sync-modal-overlay');
    if (overlay) {
        overlay.classList.remove('visible');
    }
    currentSyncPreview = null;
}

async function confirmSync() {
    const body = document.getElementById('sync-modal-body');
    const confirmBtn = document.getElementById('sync-modal-confirm');
    
    if (confirmBtn) confirmBtn.disabled = true;
    
    try {
        const response = await fetch(`${API_BASE}/api/admin/sync/run`, {
            method: 'POST'
        });
        
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        
        const result = await response.json();
        
        // Show success message
        if (body) {
            body.innerHTML = `<div class="sync-result success">
                <strong>Sync completed successfully!</strong><br>
                Deleted ${result.orphan_db_deleted || 0} orphan DB records and ${result.orphan_minio_deleted || 0} orphan MinIO files.
            </div>`;
        }
        
        // Refresh data panels after a short delay
        setTimeout(() => {
            closeSyncModal();
            checkSyncStatus();
            fetchDatabaseTree();
            fetchMinioTree();
        }, 2000);
        
    } catch (error) {
        console.error('Error running sync:', error);
        if (body) {
            body.innerHTML += `<div class="sync-result error">Sync failed: ${error.message}</div>`;
        }
        if (confirmBtn) confirmBtn.disabled = false;
    }
}

// Quick sync without preview modal
async function runSyncNow() {
    if (!confirm('Are you sure you want to sync now? This will delete orphan records.')) {
        return;
    }
    
    const textEl = document.getElementById('sync-status-text');
    if (textEl) textEl.textContent = 'Syncing...';
    
    try {
        const response = await fetch(`${API_BASE}/api/admin/sync/run`, {
            method: 'POST'
        });
        
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        
        const result = await response.json();
        
        if (textEl) {
            textEl.textContent = `Synced! Deleted ${result.orphan_db_deleted || 0} DB, ${result.orphan_minio_deleted || 0} files`;
        }
        
        // Refresh everything
        setTimeout(() => {
            checkSyncStatus();
            fetchDatabaseTree();
            fetchMinioTree();
        }, 1500);
        
    } catch (error) {
        console.error('Error running sync:', error);
        if (textEl) textEl.textContent = `Sync failed: ${error.message}`;
    }
}

// Helper to escape HTML
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// Format database time for display
function formatDbTime(isoString) {
    try {
        const date = new Date(isoString);
        const now = new Date();
        const diffMs = now - date;
        const diffMins = Math.floor(diffMs / 60000);
        const diffHours = Math.floor(diffMs / 3600000);
        
        // If within last hour, show relative time
        if (diffMins < 60) {
            return `${diffMins}m ago`;
        }
        // If within last 24 hours, show hours ago
        if (diffHours < 24) {
            return `${diffHours}h ago`;
        }
        // Otherwise show date/time
        return date.toLocaleString('en-US', {
            month: 'short',
            day: 'numeric',
            hour: 'numeric',
            minute: '2-digit',
            hour12: true
        });
    } catch {
        return isoString;
    }
}

// ============================================================================
// Dual Clock (Local + UTC)
// ============================================================================

function updateClocks() {
    const now = new Date();
    
    // Local time
    const localTime = now.toLocaleTimeString('en-US', {
        hour12: false,
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit'
    });
    
    // UTC time
    const utcTime = now.toLocaleTimeString('en-US', {
        hour12: false,
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        timeZone: 'UTC'
    });
    
    const localTimeEl = document.getElementById('local-time');
    const utcTimeEl = document.getElementById('utc-time');
    
    if (localTimeEl) localTimeEl.textContent = localTime;
    if (utcTimeEl) utcTimeEl.textContent = utcTime;
}

// Initialize clocks
document.addEventListener('DOMContentLoaded', () => {
    updateClocks();
    setInterval(updateClocks, 1000); // Update every second
});

// ============================================================================
// Model Configuration Widget
// ============================================================================

let configWidgetInterval = null;
let configData = null;

// Initialize configuration widget
function initConfigWidget() {
    fetchConfigData();
    // Refresh every 5 minutes (config doesn't change often)
    configWidgetInterval = setInterval(fetchConfigData, 300000);
}

// Toggle config widget expanded/collapsed
function toggleConfigWidget() {
    const widget = document.getElementById('config-widget');
    if (widget) {
        widget.classList.toggle('collapsed');
        // When not collapsed, add expanded class to fill remaining space
        if (widget.classList.contains('collapsed')) {
            widget.classList.remove('expanded');
        } else {
            widget.classList.add('expanded');
        }
    }
}

// Fetch configuration data
async function fetchConfigData() {
    try {
        const response = await fetch(`${API_BASE}/api/admin/config/full`);
        if (!response.ok) {
            console.error('Failed to fetch config:', response.status);
            return;
        }
        
        configData = await response.json();
        renderConfigWidget(configData);
    } catch (error) {
        console.error('Error fetching config:', error);
    }
}

// Render the configuration widget
function renderConfigWidget(data) {
    const modelCountEl = document.getElementById('config-model-count');
    const styleCountEl = document.getElementById('config-style-count');
    const modelsListEl = document.getElementById('config-models-list');
    const stylesListEl = document.getElementById('config-styles-list');
    
    // Update counts in header
    if (modelCountEl) {
        const enabledCount = data.models.filter(m => m.enabled).length;
        modelCountEl.textContent = `${enabledCount} models`;
        modelCountEl.title = `${enabledCount} enabled models`;
    }
    
    if (styleCountEl) {
        const totalStyles = data.styles.reduce((sum, s) => sum + s.style_count, 0);
        styleCountEl.textContent = `${totalStyles} styles`;
        styleCountEl.title = `${totalStyles} rendering styles across ${data.styles.length} files`;
    }
    
    // Render models
    if (modelsListEl) {
        modelsListEl.innerHTML = data.models.map(model => renderModelCard(model)).join('');
    }
    
    // Render styles
    if (stylesListEl) {
        stylesListEl.innerHTML = data.styles.map(style => renderStyleCard(style)).join('');
    }
}

// Render a model card
function renderModelCard(model) {
    const scheduleInfo = model.schedule_type === 'observation' 
        ? 'Observation' 
        : `${model.cycles.length} cycles/day`;
    
    const forecastInfo = model.forecast_hours 
        ? `F${model.forecast_hours.start}-${model.forecast_hours.end}h` 
        : '';
    
    const badges = [];
    if (model.enabled) {
        badges.push('<span class="config-badge enabled">ON</span>');
    } else {
        badges.push('<span class="config-badge disabled">OFF</span>');
    }
    if (model.precaching_enabled) {
        badges.push('<span class="config-badge precache">PRE</span>');
    }
    
    // Create parameter tags (show first 5, then "+X more")
    const paramTags = model.parameters.slice(0, 5).map(p => 
        `<span class="config-param-tag" title="${p.description} (${p.units})">${p.name}</span>`
    ).join('');
    const moreParams = model.parameters.length > 5 
        ? `<span class="config-param-tag">+${model.parameters.length - 5}</span>` 
        : '';
    
    return `
        <div class="config-model-card" onclick="toggleModelCard(this)">
            <div class="config-model-header">
                <div class="config-model-info">
                    <div class="config-model-name">
                        ${model.name.split(' - ')[0]}
                        <span class="config-model-id">${model.id}</span>
                    </div>
                    <div class="config-model-desc">${model.description || model.name}</div>
                </div>
                <div class="config-model-badges">
                    ${badges.join('')}
                </div>
            </div>
            <div class="config-model-details">
                <div class="config-detail-row">
                    <span class="config-detail-label">Source</span>
                    <span class="config-detail-value">${model.source_type} / ${model.source_bucket || '-'}</span>
                </div>
                <div class="config-detail-row">
                    <span class="config-detail-label">Grid</span>
                    <span class="config-detail-value">${model.projection} @ ${model.resolution || '-'}</span>
                </div>
                <div class="config-detail-row">
                    <span class="config-detail-label">Schedule</span>
                    <span class="config-detail-value">${scheduleInfo} ${forecastInfo}</span>
                </div>
                <div class="config-detail-row">
                    <span class="config-detail-label">Retention</span>
                    <span class="config-detail-value">${model.retention_hours}h</span>
                </div>
                <div class="config-detail-row">
                    <span class="config-detail-label">Parameters</span>
                    <span class="config-detail-value">${model.parameter_count}</span>
                </div>
                <div class="config-params-list">
                    ${paramTags}${moreParams}
                </div>
            </div>
        </div>
    `;
}

// Render a style card
function renderStyleCard(style) {
    const variantsHtml = style.styles.map(s => {
        const range = s.range_min !== null && s.range_max !== null 
            ? `${s.range_min} - ${s.range_max} ${s.units}` 
            : s.units || '-';
        return `
            <div class="config-style-variant">
                <span class="config-style-variant-name">${s.id}</span>
                <span class="config-style-variant-meta">
                    <span>${s.style_type}</span>
                    <span>${s.stop_count} stops</span>
                </span>
            </div>
        `;
    }).join('');
    
    return `
        <div class="config-style-card" onclick="toggleStyleCard(this)">
            <div class="config-style-header">
                <span class="config-style-name">${style.name}</span>
                <span class="config-style-count">${style.style_count}</span>
            </div>
            <div class="config-style-details">
                <div class="config-style-desc">${style.description}</div>
                <div class="config-style-variants">
                    ${variantsHtml}
                </div>
            </div>
        </div>
    `;
}

// Toggle model card expanded/collapsed
function toggleModelCard(card) {
    card.classList.toggle('expanded');
}

// Toggle style card expanded/collapsed
function toggleStyleCard(card) {
    card.classList.toggle('expanded');
}

// ============================================================================
// Performance & Cache Widget
// ============================================================================

let perfWidgetInterval = null;

// Initialize performance widget
function initPerfWidget() {
    fetchPerfData();
    // Refresh every 10 seconds
    perfWidgetInterval = setInterval(fetchPerfData, 10000);
}

// Toggle performance widget expanded/collapsed
function togglePerfWidget() {
    const widget = document.getElementById('perf-widget');
    if (widget) {
        widget.classList.toggle('collapsed');
        if (widget.classList.contains('collapsed')) {
            widget.classList.remove('expanded');
        } else {
            widget.classList.add('expanded');
        }
    }
}

// Fetch performance and cache data
async function fetchPerfData() {
    try {
        // Fetch metrics, model config, and optimization config in parallel
        const [metricsRes, modelsRes, optConfigRes] = await Promise.all([
            fetch(`${API_BASE}/api/metrics`),
            fetch(`${API_BASE}/api/admin/config/full`),
            fetch(`${API_BASE}/api/config`)
        ]);
        
        if (!metricsRes.ok || !modelsRes.ok || !optConfigRes.ok) {
            console.error('Failed to fetch perf data');
            return;
        }
        
        const metrics = await metricsRes.json();
        const modelsConfig = await modelsRes.json();
        const optConfig = await optConfigRes.json();
        
        renderPerfWidget(metrics, modelsConfig, optConfig);
    } catch (error) {
        console.error('Error fetching perf data:', error);
    }
}

// Render the performance widget
function renderPerfWidget(metrics, modelsConfig, optConfig) {
    // Get memory pressure config
    const memPressure = optConfig?.optimizations?.memory_pressure || {};
    const memLimitMB = memPressure.limit_mb || 4000;
    const memThreshold = memPressure.threshold || 0.80;
    const memTarget = memPressure.target || 0.60;
    
    // Update header memory usage (using chunk cache now)
    const memUsageEl = document.getElementById('perf-memory-usage');
    if (memUsageEl && metrics.chunk_cache) {
        const memMB = metrics.chunk_cache.memory_mb || 0;
        memUsageEl.textContent = formatMemory(memMB * 1024 * 1024);
        
        // Color based on usage relative to threshold
        const thresholdMB = memLimitMB * memThreshold;
        const targetMB = memLimitMB * memTarget;
        if (memMB > thresholdMB) {
            memUsageEl.style.background = '#ef4444'; // Red - above threshold
        } else if (memMB > targetMB) {
            memUsageEl.style.background = '#f59e0b'; // Yellow - between target and threshold
        } else {
            memUsageEl.style.background = '#10b981'; // Green - below target
        }
    }
    
    // Memory Pressure Stats (using chunk cache now)
    const chunkMemMB = metrics.chunk_cache?.memory_mb || 0;
    const usageRatio = chunkMemMB / memLimitMB;
    
    const pressureEl = document.getElementById('perf-mem-pressure');
    if (pressureEl) {
        if (usageRatio > memThreshold) {
            pressureEl.textContent = 'HIGH';
            pressureEl.className = 'perf-value perf-status critical';
        } else if (usageRatio > memTarget) {
            pressureEl.textContent = 'MODERATE';
            pressureEl.className = 'perf-value perf-status warning';
        } else {
            pressureEl.textContent = 'LOW';
            pressureEl.className = 'perf-value perf-status';
        }
    }
    updateElement('perf-mem-threshold', `${Math.round(memThreshold * 100)}%`);
    updateElement('perf-mem-target', `${Math.round(memTarget * 100)}%`);
    
    // Feature Flags
    const opts = optConfig?.optimizations || {};
    updateFeatureFlag('perf-flag-l1', opts.l1_cache?.enabled !== false);
    updateFeatureFlag('perf-flag-l2', opts.l2_cache?.enabled !== false);
    updateFeatureFlag('perf-flag-chunk', opts.chunk_cache?.enabled !== false);
    updateFeatureFlag('perf-flag-prefetch', opts.prefetch?.enabled !== false);
    updateFeatureFlag('perf-flag-warming', modelsConfig.models?.some(m => m.precaching_enabled));
    
    // Model Precaching List
    renderPrecacheList(modelsConfig.models || []);
}

// Update feature flag styling
function updateFeatureFlag(id, enabled) {
    const el = document.getElementById(id);
    if (el) {
        if (enabled) {
            el.classList.add('enabled');
        } else {
            el.classList.remove('enabled');
        }
    }
}

// Helper to update element text
function updateElement(id, text) {
    const el = document.getElementById(id);
    if (el) el.textContent = text;
}

// Render model precaching list
function renderPrecacheList(models) {
    const listEl = document.getElementById('perf-precache-list');
    if (!listEl) return;
    
    const html = models.map(model => {
        const enabled = model.precaching_enabled || false;
        const keepRecent = model.precache_keep_recent || 0;
        const warmOnIngest = model.precache_warm_on_ingest || false;
        const params = model.precache_parameters || [];
        const paramsText = params.length > 0 ? params.join(', ') : 'all';
        
        return `
            <div class="perf-precache-item">
                <div class="perf-precache-model">
                    <span class="perf-precache-name">${model.id.toUpperCase()}</span>
                    <span class="perf-precache-status ${enabled ? 'enabled' : 'disabled'}">
                        ${enabled ? 'ON' : 'OFF'}
                    </span>
                </div>
                <div class="perf-precache-config">
                    <span class="perf-precache-detail">Keep: <span>${keepRecent}</span></span>
                    <span class="perf-precache-detail">Warm: <span>${warmOnIngest ? 'Yes' : 'No'}</span></span>
                    <span class="perf-precache-params" title="${paramsText}">${paramsText}</span>
                </div>
            </div>
        `;
    }).join('');
    
    listEl.innerHTML = html || '<div class="perf-loading">No models configured</div>';
}

// Format memory size
function formatMemory(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

// Initialize on DOM ready (add to existing init)
document.addEventListener('DOMContentLoaded', () => {
    initConfigWidget();
    initPerfWidget();
    initGridProcessorWidget();
});

// Cleanup on page unload
window.addEventListener('beforeunload', () => {
    if (configWidgetInterval) {
        clearInterval(configWidgetInterval);
    }
    if (perfWidgetInterval) {
        clearInterval(perfWidgetInterval);
    }
    if (gridProcessorWidgetInterval) {
        clearInterval(gridProcessorWidgetInterval);
    }
});

// ============================================================================
// Grid Processor Widget (Zarr/Chunk Cache)
// ============================================================================

let gridProcessorWidgetInterval = null;

// Initialize grid processor widget
function initGridProcessorWidget() {
    fetchGridProcessorData();
    // Refresh every 5 seconds
    gridProcessorWidgetInterval = setInterval(fetchGridProcessorData, 5000);
}

// Toggle grid processor widget expanded/collapsed
function toggleGridProcessorWidget() {
    const widget = document.getElementById('grid-processor-widget');
    if (widget) {
        widget.classList.toggle('collapsed');
        if (widget.classList.contains('collapsed')) {
            widget.classList.remove('expanded');
        } else {
            widget.classList.add('expanded');
        }
    }
}

// Fetch grid processor data from API
async function fetchGridProcessorData() {
    try {
        const response = await fetch(`${API_BASE}/api/grid-processor/stats`);
        if (!response.ok) {
            console.error('Failed to fetch grid processor stats:', response.status);
            return;
        }
        const data = await response.json();
        renderGridProcessorWidget(data);
    } catch (error) {
        console.error('Error fetching grid processor data:', error);
    }
}

// Render the grid processor widget
function renderGridProcessorWidget(data) {
    const chunk = data.chunk_cache || {};
    const zarr = data.zarr || {};
    
    // Update summary in header
    const summaryEl = document.getElementById('grid-proc-summary');
    if (summaryEl) {
        const memMB = chunk.memory_mb || 0;
        const hitRate = chunk.hit_rate_percent || 0;
        summaryEl.textContent = `${memMB.toFixed(1)} MB / ${hitRate.toFixed(0)}%`;
        
        // Color based on hit rate
        if (hitRate >= 80) {
            summaryEl.style.background = '#10b981'; // Green - great hit rate
        } else if (hitRate >= 50) {
            summaryEl.style.background = '#f59e0b'; // Yellow - moderate hit rate
        } else if (chunk.total_requests > 0) {
            summaryEl.style.background = '#ef4444'; // Red - poor hit rate
        } else {
            summaryEl.style.background = '#6b7280'; // Gray - no data yet
        }
    }
    
    // Chunk Cache Stats
    updateElement('grid-proc-memory', formatMemory(chunk.memory_bytes || 0));
    updateElement('grid-proc-entries', `${chunk.entries || 0}`);
    updateElement('grid-proc-max-size', `${chunk.configured_size_mb || 0} MB`);
    updateElement('grid-proc-hitrate', `${(chunk.hit_rate_percent || 0).toFixed(1)}%`);
    
    // Request Stats
    updateElement('grid-proc-hits', formatNumber(chunk.hits || 0));
    updateElement('grid-proc-misses', formatNumber(chunk.misses || 0));
    updateElement('grid-proc-evictions', formatNumber(chunk.evictions || 0));
    updateElement('grid-proc-total', formatNumber(chunk.total_requests || 0));
    
    // Color hits based on value
    const hitsEl = document.getElementById('grid-proc-hits');
    if (hitsEl) {
        if ((chunk.hits || 0) > 0) {
            hitsEl.classList.add('perf-value-good');
        } else {
            hitsEl.classList.remove('perf-value-good');
        }
    }
    
    // Feature Flags
    updateFeatureFlag('grid-proc-flag-enabled', chunk.enabled !== false);
    updateFeatureFlag('grid-proc-flag-zarr', zarr.supported !== false);
    updateFeatureFlag('grid-proc-flag-storage', zarr.storage_backend === 'minio');
}

// Format large numbers with commas
function formatNumber(num) {
    if (num >= 1000000) {
        return (num / 1000000).toFixed(1) + 'M';
    } else if (num >= 1000) {
        return (num / 1000).toFixed(1) + 'K';
    }
    return num.toString();
}
