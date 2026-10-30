// WMS Dashboard Application
// 
// MEMORY LEAK FIXES (Jan 2026):
// - Network pollers (service status, validation, data panel) pause while the tab is hidden
//   and never overlap: a poll is skipped while the previous run of the same poller is still pending
// - Thorough Leaflet tile cleanup on layer removal (clearing tile.el.src, event listeners)
// - Added keepBuffer:2 to tile layers to reduce off-screen tile caching

// Smart API URL detection:
// - localhost:8000 (docker-compose) -> use localhost:8080
// - Otherwise (production/K8s) -> use relative URLs (same origin)
const IS_LOCAL_DEV = window.location.port === '8000';
const API_BASE = IS_LOCAL_DEV ? 'http://localhost:8080' : '';
const EDR_URL = IS_LOCAL_DEV ? 'http://localhost:8083/edr' : '/edr';
const DOWNLOADER_URL = IS_LOCAL_DEV ? 'http://localhost:8081' : '/downloader';

// External service URLs - different for local dev vs production
const EXTERNAL_URLS = IS_LOCAL_DEV ? {
    minio: 'http://localhost:9001',
    grafana: 'http://localhost:3000',
    prometheus: 'http://localhost:9090',
    k8sDashboard: 'http://localhost:8001/api/v1/namespaces/kubernetes-dashboard/services/http:kubernetes-dashboard:/proxy/',
} : {
    minio: '/minio/',
    grafana: '/grafana/',
    prometheus: null,  // Not exposed in production
    k8sDashboard: null,  // Not exposed in production
};
let map;
let wmsLayer = null;
let selectedLayer = null;
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

// Interval references for cleanup (prevents memory leaks on page refresh/SPA navigation)
let clockInterval = null;
let observationTimeRefreshInterval = null; // Periodic refresh for observation layer times (MRMS, GOES)

// ============================================================================
// POLLING - network pollers pause while the tab is hidden and never overlap
// ============================================================================

const pollers = new Map();
const pollsInFlight = new Map();

// Run a task unless a run of the same name is still pending; callers share the pending run.
function runOnce(name, task) {
    if (pollsInFlight.has(name)) return pollsInFlight.get(name);
    const run = Promise.resolve()
        .then(task)
        .catch(error => console.error(`Poll "${name}" failed:`, error))
        .finally(() => pollsInFlight.delete(name));
    pollsInFlight.set(name, run);
    return run;
}

// Start polling `task` every `ms`. The first run happens now when `immediate` is true; pass
// false when the caller already holds fresh data. Nothing runs while the tab is hidden;
// resumePollers() restarts the timers.
function startPoller(name, task, ms, immediate = true) {
    stopPoller(name);
    const poller = { tick: () => { poller.lastRun = Date.now(); return runOnce(name, task); }, ms, ref: null, lastRun: immediate ? 0 : Date.now() };
    pollers.set(name, poller);
    if (document.hidden) return;
    if (immediate) poller.tick();
    poller.ref = setInterval(poller.tick, ms);
}

function stopPoller(name) {
    const poller = pollers.get(name);
    if (!poller) return;
    clearInterval(poller.ref);
    pollers.delete(name);
}

function pausePollers() {
    pollers.forEach(poller => {
        clearInterval(poller.ref);
        poller.ref = null;
    });
}

// Restart paused timers; a poller whose last run is older than its period runs right away.
function resumePollers() {
    pollers.forEach(poller => {
        if (poller.ref) return;
        if (Date.now() - poller.lastRun >= poller.ms) poller.tick();
        poller.ref = setInterval(poller.tick, poller.ms);
    });
}

function stopAllPollers() {
    pausePollers();
    pollers.clear();
}

// ============================================================================
// MEMORY DEBUGGING UTILITIES
// ============================================================================

// Log memory usage (for debugging memory leaks)
// Enable by setting localStorage.setItem('debugMemory', 'true') in console
function logMemoryUsage(context = '') {
    if (localStorage.getItem('debugMemory') !== 'true') return;
    
    if (performance.memory) {
        const used = (performance.memory.usedJSHeapSize / 1024 / 1024).toFixed(2);
        const total = (performance.memory.totalJSHeapSize / 1024 / 1024).toFixed(2);
        const limit = (performance.memory.jsHeapSizeLimit / 1024 / 1024).toFixed(2);
        console.log(`[Memory${context ? ' - ' + context : ''}] Used: ${used}MB / Total: ${total}MB / Limit: ${limit}MB`);
    }
    
    // Also log data structure sizes
    const preloadedSize = Object.keys(preloadedLayers).length;
    console.log(`[DataStructures${context ? ' - ' + context : ''}] PreloadedLayers: ${preloadedSize}`);
}

// Periodic memory logging (every 30 seconds if debug mode enabled)
function startMemoryLogging() {
    if (localStorage.getItem('debugMemory') !== 'true') return;
    
    setInterval(() => {
        logMemoryUsage('periodic');
    }, 30000);
    
    console.log('Memory debugging enabled. Logging every 30 seconds.');
    console.log('To disable: localStorage.removeItem("debugMemory") and refresh');
}

// Initialize memory logging on load
document.addEventListener('DOMContentLoaded', () => {
    startMemoryLogging();
});

// Handle page visibility changes - pause polling when tab is hidden
document.addEventListener('visibilitychange', () => {
    if (document.hidden) {
        console.log('Tab hidden - pausing all polling intervals to save memory');
        pausePollers();
        stopObservationTimeRefresh();
        // Also stop any playback
        if (isPlaying) {
            stopPlayback();
        }
    } else {
        console.log('Tab visible - resuming polling intervals');
        resumePollers();
        startObservationTimeRefresh(); // Resume if on an observation layer
    }
});

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
    startPoller('serviceStatus', checkServiceStatus, 30000);
    loadAvailableLayers();
    setupEventListeners();
    setupMapClickHandler();
    watchMapSize();
});

// Keep Leaflet's cached size in step with the #map element. Leaflet only re-measures on window
// resize, but the page layout can shift after init (the header reflows as the badges fill in), so
// without this the map keeps sizing itself to the container it had at init.
function watchMapSize() {
    const mapEl = document.getElementById('map');
    if (!mapEl || typeof ResizeObserver === 'undefined') return;
    new ResizeObserver(() => {
        if (map) map.invalidateSize();
    }).observe(mapEl);
}

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

// Determine whether to use preload-all (forecast with few steps) or
// on-demand frame switching (observation with many steps like MRMS/GOES).
const PRELOAD_ALL_THRESHOLD = 15; // Max steps to preload all at once

function shouldPreloadAll() {
    const timeSteps = layerTimeMode === 'observation' 
        ? availableObservationTimes 
        : availableForecastHours;
    return timeSteps.length <= PRELOAD_ALL_THRESHOLD;
}

// Preload all time step layers for smooth animation.
// Only used for forecast layers with few steps (<=PRELOAD_ALL_THRESHOLD).
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

// Clear all preloaded layers with thorough cleanup to prevent memory leaks
function clearPreloadedLayers() {
    Object.values(preloadedLayers).forEach(layer => {
        if (layer) {
            // Remove all event listeners
            layer.off();
            // Clear internal tile cache if available
            if (layer._tiles) {
                Object.keys(layer._tiles).forEach(key => {
                    const tile = layer._tiles[key];
                    if (tile && tile.el) {
                        tile.el.src = '';  // Clear image src to release memory
                        tile.el.onload = null;
                        tile.el.onerror = null;
                    }
                });
                layer._tiles = {};
            }
            layer._tilesToLoad = 0;
            // Remove from map
            if (map && map.hasLayer(layer)) {
                map.removeLayer(layer);
            }
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
    
    if (selectedProtocol === 'wmts') {
        if (shouldPreloadAll()) {
            // Forecast layers with few steps: preload all frames for instant switching
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
        } else {
            // Observation layers with many steps (MRMS 65+, GOES 36+):
            // Use on-demand frame switching instead of preloading all at once.
            // Each frame swaps the tile layer URL — server-side cache handles performance.
            console.log(`On-demand playback for ${availableObservationTimes.length} observation timestamps`);
        }
    }
    
    // Start the animation loop
    startPlaybackInterval();
}

// Start the playback interval (separated so we can restart with new speed)
function startPlaybackInterval() {
    const slider = document.getElementById('map-time-slider');
    if (!slider) return;
    
    const usePreloaded = selectedProtocol === 'wmts' && shouldPreloadAll();
    
    playbackInterval = setInterval(() => {
        let currentIndex = parseInt(slider.value);
        const maxIndex = parseInt(slider.max);
        
        // Hide current frame (only for preloaded mode)
        if (usePreloaded && preloadedLayers[currentIndex]) {
            preloadedLayers[currentIndex].setOpacity(0);
        }
        
        // Move to next step, loop back to start if at end
        currentIndex++;
        if (currentIndex > maxIndex) {
            currentIndex = 0;
        }
        
        // Update slider position
        slider.value = currentIndex;
        
        if (usePreloaded) {
            // Preloaded mode: toggle layer opacity (fast, no network)
            if (preloadedLayers[currentIndex]) {
                preloadedLayers[currentIndex].setOpacity(0.7);
            }
            onTimeSliderChange(currentIndex);
        } else {
            // On-demand mode: swap the tile layer URL directly.
            // onTimeSliderChange sets currentObservationTime/currentForecastHour
            // and updates the time label, but skips updateLayerTime() when isPlaying.
            // We call updateLayerTime() ourselves to swap the actual tile layer.
            onTimeSliderChange(currentIndex);
            updateLayerTime();
        }
        
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
            // During playback, the playback interval handles layer updates
            // (either via preloaded layer opacity or on-demand updateLayerTime)
            if (!isPlaying) {
                updateLayerTime();
            }
        }
    } else {
        // Forecast mode - index into availableForecastHours (already in ascending order)
        if (index >= 0 && index < availableForecastHours.length) {
            currentForecastHour = availableForecastHours[index];
            updateTimeSliderDisplay();
            // During playback, the playback interval handles layer updates
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
        
        // Preserve slider position if we have a current observation time, otherwise default to latest
        if (currentObservationTime) {
            const posIndex = timesReversed.indexOf(currentObservationTime);
            slider.value = posIndex >= 0 ? posIndex : timesReversed.length - 1;
        } else {
            slider.value = timesReversed.length - 1; // Start at latest
        }
        
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
        // Web Mercator mode: use OpenFreeMap/OSM base layers
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

// OpenFreeMap serves free vector basemaps (no API key). Leaflet draws them through MapLibre GL via
// the maplibre-gl-leaflet plugin loaded in index.html. Their terms require this attribution, and it
// is passed to the plugin explicitly because Leaflet's attribution control cannot read MapLibre's own
// (the plugin would also fail to remove it again when the layer is taken off the map).
const OPENFREEMAP_ATTRIBUTION =
    '<a href="https://openfreemap.org" target="_blank">OpenFreeMap</a> ' +
    '&copy; <a href="https://www.openmaptiles.org/" target="_blank">OpenMapTiles</a> ' +
    'Data from <a href="https://www.openstreetmap.org/copyright" target="_blank">OpenStreetMap</a>';

function openFreeMapLayer(styleName) {
    // No pane option: the GL canvas lives in Leaflet's tilePane (z-index 200), below weatherPane (450).
    return L.maplibreGL({
        style: `https://tiles.openfreemap.org/styles/${styleName}`,
        attributionControl: { customAttribution: OPENFREEMAP_ATTRIBUTION }
    });
}

// Add Mercator base layers (OpenFreeMap Light/Dark, OSM)
function addMercatorBaseLayers() {
    // Reset background color
    document.getElementById('map').style.backgroundColor = '';
    
    const osmLayer = L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
        attribution: '&copy; OpenStreetMap contributors',
        maxZoom: 19
    });

    // Without the MapLibre scripts (CDN blocked or down) fall back to the raster OSM layer alone
    if (typeof L.maplibreGL !== 'function') {
        console.warn('maplibre-gl-leaflet is not loaded; using the OpenStreetMap base layer only');
        osmLayer.addTo(map);
        window.baseLayers = { 'OpenStreetMap': osmLayer };
        return;
    }

    const lightLayer = openFreeMapLayer('positron');
    const darkLayer = openFreeMapLayer('dark');

    // Add default base layer
    lightLayer.addTo(map);

    // Store base layers for layer control (the first one is the default)
    window.baseLayers = {
        'Light': lightLayer,
        'Dark': darkLayer,
        'OpenStreetMap': osmLayer
    };
}

// Health checks behind the header status badges: a service is online when its probe succeeds.
const SERVICE_CHECKS = {
    wms: { url: `${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities` },
    wmts: { url: `${API_BASE}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities` },
    edr: { url: EDR_URL },
    downloader: { url: `${DOWNLOADER_URL}/health` },
    // wms-api proxies the ingester's /status and answers 200 with an "error" field when it is unreachable
    ingester: {
        url: `${API_BASE}/api/admin/ingestion/active`,
        isOnline: async response => !(await response.json()).error
    }
};

// Check WMS, WMTS, EDR, downloader and ingester status
async function checkServiceStatus() {
    await Promise.all(Object.entries(SERVICE_CHECKS).map(async ([service, check]) => {
        try {
            const response = await fetch(check.url, { mode: 'cors', cache: 'no-cache' });
            const online = response.ok && (!check.isOnline || await check.isOnline(response));
            setStatusIndicator(service, online ? 'online' : 'offline');
        } catch (error) {
            console.error(`${service} status check failed:`, error);
            setStatusIndicator(service, 'offline');
        }
    }));
}

// Update status indicator
function setStatusIndicator(service, status) {
    const element = document.getElementById(`${service}-status`);
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

// Clear the current weather layer with thorough memory cleanup
function clearWeatherLayer() {
    stopPlayback();
    clearPreloadedLayers();
    if (wmsLayer) {
        // Remove all event listeners
        wmsLayer.off();
        // Clear internal tile cache if it's a TileLayer
        if (wmsLayer._tiles) {
            Object.keys(wmsLayer._tiles).forEach(key => {
                const tile = wmsLayer._tiles[key];
                if (tile && tile.el) {
                    tile.el.src = '';  // Clear image src to release memory
                    tile.el.onload = null;
                    tile.el.onerror = null;
                }
            });
            wmsLayer._tiles = {};
        }
        wmsLayer._tilesToLoad = 0;
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
    // Stop any active playback and observation time refresh from previous layer
    stopPlayback();
    stopObservationTimeRefresh();
    
    // Remove existing layer and cleanup event listeners
    if (wmsLayer) {
        map.removeLayer(wmsLayer);
        map.off('moveend', onMapMoveEnd); // Remove moveend listener from previous WMS layer
        wmsLayer = null;
    }

    // Store current layer and reset style to default for new layer
    selectedLayer = layerName;
    selectedStyle = 'default';

    // Reset time mode to forecast (default) before loading
    layerTimeMode = 'forecast';
    currentObservationTime = null;
    currentForecastHour = null; // Will be set by fetchAvailableTimes
    currentElevation = ''; // Reset elevation - will be populated from capabilities

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
            pane: 'weatherPane', // Custom pane above base layers
            // Memory optimization options
            updateWhenIdle: true,  // Don't update during pan/zoom for smoother UX
            keepBuffer: 2          // Reduce tile buffer to minimize memory usage
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
    
    const overlay = L.imageOverlay(url, bounds, {
        opacity: 0.7,
        pane: 'weatherPane', // Custom pane above base layers
        interactive: false,
        attribution: `${formatLayerName(layerName)} (WMS - ${style})`
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

// Format bytes to human readable
function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
}

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

// ============================================================================
// Validation Status Functions
// ============================================================================

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
    
    // Check now, then every 5 minutes
    startPoller('validation', checkValidationStatus, 5 * 60 * 1000);
});

// Clean up on page unload - centralized interval cleanup to prevent memory leaks
window.addEventListener('beforeunload', () => {
    console.log('Page unloading - cleaning up all resources');
    
    stopPlayback();

    stopAllPollers();
    if (clockInterval) clearInterval(clockInterval);
    
    // Clear preloaded layers
    clearPreloadedLayers();
    
    // Clear weather layer
    if (wmsLayer) {
        clearWeatherLayer();
    }
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
            // Thorough cleanup to prevent memory leaks
            wmsLayer.off();
            if (wmsLayer._tiles) {
                Object.keys(wmsLayer._tiles).forEach(key => {
                    const tile = wmsLayer._tiles[key];
                    if (tile && tile.el) {
                        tile.el.src = '';
                        tile.el.onload = null;
                        tile.el.onerror = null;
                    }
                });
                wmsLayer._tiles = {};
            }
            wmsLayer._tilesToLoad = 0;
            map.removeLayer(wmsLayer);
        }
        
        wmsLayer = L.tileLayer(wmtsUrl, {
            attribution: `${formatLayerName(selectedLayer)} (WMTS - ${selectedStyle} - ${selectedTileMatrixSet})`,
            maxZoom: 18,
            tileSize: 256,
            opacity: 0.7,
            pane: 'weatherPane',
            // Memory optimization: don't keep tiles that are off-screen
            updateWhenIdle: true,
            keepBuffer: 2  // Reduce buffer from default 2 to minimize cached tiles
        });
        wmsLayer.addTo(map);
    } else {
        // For WMS, we need to recreate the overlay
        if (wmsLayer) {
            wmsLayer.off();
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
        const response = await fetch(`${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`, { cache: 'no-cache' });
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
        
        // Start periodic time refresh for observation layers (MRMS, GOES)
        // whose data rotates continuously and can go stale
        startObservationTimeRefresh();
        
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
        stopObservationTimeRefresh();
    }
}

// Silently refresh observation times for the active layer without disrupting the UI.
// Called periodically (~60s) for observation layers (MRMS, GOES) whose data rotates.
async function refreshObservationTimes() {
    if (!selectedLayer || layerTimeMode !== 'observation') return;

    try {
        const response = await fetch(`${API_BASE}/wms?SERVICE=WMS&REQUEST=GetCapabilities`, { cache: 'no-cache' });
        const xml = await response.text();
        const parser = new DOMParser();
        const doc = parser.parseFromString(xml, 'text/xml');

        const layers = doc.getElementsByTagName('Layer');
        for (let layer of layers) {
            const nameEl = layer.getElementsByTagName('Name')[0];
            if (nameEl && nameEl.textContent === selectedLayer) {
                const dimensions = layer.getElementsByTagName('Dimension');
                for (let dim of dimensions) {
                    if (dim.getAttribute('name') === 'TIME') {
                        const timeText = dim.textContent.trim();
                        const newTimes = timeText.split(',').filter(t => t && t !== 'latest');
                        newTimes.sort((a, b) => new Date(b) - new Date(a));

                        if (newTimes.length === 0) return;

                        // Detect if the set of times actually changed
                        const oldSet = new Set(availableObservationTimes);
                        const newSet = new Set(newTimes);
                        const changed = newTimes.length !== availableObservationTimes.length ||
                            newTimes.some(t => !oldSet.has(t));

                        if (!changed) return; // No change, nothing to do

                        console.log(`Observation times refreshed for ${selectedLayer}: ${availableObservationTimes.length} -> ${newTimes.length} times`);

                        // Remember current selection so we can preserve the user's slider position
                        const previousTime = currentObservationTime;
                        availableObservationTimes = newTimes;

                        // Try to keep the user on the same (or nearest) time
                        if (previousTime && newSet.has(previousTime)) {
                            currentObservationTime = previousTime;
                        } else if (previousTime) {
                            // Find nearest available time to what the user had selected
                            const prevMs = new Date(previousTime).getTime();
                            let bestTime = newTimes[0];
                            let bestDelta = Math.abs(new Date(bestTime).getTime() - prevMs);
                            for (const t of newTimes) {
                                const delta = Math.abs(new Date(t).getTime() - prevMs);
                                if (delta < bestDelta) {
                                    bestDelta = delta;
                                    bestTime = t;
                                }
                            }
                            currentObservationTime = bestTime;
                            console.log(`Snapped observation time from ${previousTime} to nearest ${bestTime} (delta ${Math.round(bestDelta/1000)}s)`);
                        } else {
                            currentObservationTime = newTimes[0]; // Latest
                        }

                        // Re-populate the slider with updated times and refresh the tile layer
                        populateTimeSlider();
                        if (!isPlaying) {
                            updateLayerTime();
                        }
                        return;
                    }
                }
                break;
            }
        }
    } catch (error) {
        console.warn('Failed to refresh observation times:', error);
    }
}

// Start periodic observation time refresh (called when an observation layer is selected)
function startObservationTimeRefresh() {
    stopObservationTimeRefresh();
    if (layerTimeMode === 'observation') {
        // Refresh every 60 seconds — fast enough for MRMS (2-min updates) without being noisy
        observationTimeRefreshInterval = setInterval(refreshObservationTimes, 60000);
        console.log('Started observation time refresh interval (60s)');
    }
}

// Stop periodic observation time refresh
function stopObservationTimeRefresh() {
    if (observationTimeRefreshInterval) {
        clearInterval(observationTimeRefreshInterval);
        observationTimeRefreshInterval = null;
    }
}

// Note: stopPlayback() is defined earlier in this file (around line 289)
// with full implementation including button state updates

// Initialize the data panel (PostgreSQL + MinIO tree) once the DOM is ready
document.addEventListener('DOMContentLoaded', () => {
    initDatabasePanel();
});

// ============================================================================
// Data Panel Widget (PostgreSQL + MinIO Tree View)
// ============================================================================

let dbTreeData = null;  // Cached PostgreSQL data
let minioTreeData = null;  // Cached MinIO data
let minioFetchedAt = 0;  // When the MinIO tree was last loaded (0 = never)
let expandedNodes = new Set();  // Track expanded nodes across refreshes

// The MinIO tree and the sync check both walk the whole bucket server-side and can take over
// a minute, so the tree loads only once its section is opened and the sync check runs rarely.
const DATABASE_REFRESH_MS = 15 * 1000;
const MINIO_REFRESH_MS = 60 * 1000;
const SYNC_CHECK_MS = 5 * 60 * 1000;

// Initialize the data panel
function initDataPanel() {
    startPoller('databaseTree', fetchDatabaseTree, DATABASE_REFRESH_MS);
    startPoller('syncStatus', checkSyncStatus, SYNC_CHECK_MS);
}

// Refresh everything the panel shows (after a sync run)
function refreshDataPanel() {
    runOnce('databaseTree', fetchDatabaseTree);
    runOnce('syncStatus', checkSyncStatus);
    if (minioFetchedAt) runOnce('minioTree', fetchMinioTree);
}

// Alias for backward compatibility
function initDatabasePanel() {
    initDataPanel();
}

// Toggle section collapse
function toggleDataSection(sectionId) {
    const section = document.getElementById(sectionId);
    if (!section) return;
    section.classList.toggle('collapsed');

    if (sectionId === 'minio-section') {
        if (section.classList.contains('collapsed')) {
            stopPoller('minioTree');
        } else {
            startPoller('minioTree', fetchMinioTree, MINIO_REFRESH_MS, Date.now() - minioFetchedAt >= MINIO_REFRESH_MS);
        }
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
    if (countEl) countEl.textContent = (data.total_datasets || 0).toLocaleString();
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
        minioFetchedAt = Date.now();
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
            refreshDataPanel();
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
        setTimeout(refreshDataPanel, 1500);
        
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
    clockInterval = setInterval(updateClocks, 1000); // Update every second
});
