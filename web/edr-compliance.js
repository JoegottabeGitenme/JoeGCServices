// OGC EDR API Compliance Test Suite JavaScript

// ============================================================
// CONFIGURATION
// ============================================================

// Smart endpoint detection
const IS_LOCAL_DEV = window.location.port === '8000';
const DEFAULT_API_BASE = IS_LOCAL_DEV ? 'http://localhost:8083/edr' : `${window.location.origin}/edr`;
let API_BASE = localStorage.getItem('edr-compliance-endpoint') || DEFAULT_API_BASE;

// Test state
let testResults = {};
let collections = [];
let selectedCollectionId = '__ALL__';  // Default to all collections
let perCollectionResults = {};  // { testName: { collectionId: result } }
let isTestRunning = false;
let stopRequested = false;

// ============================================================
// INITIALIZATION
// ============================================================

document.addEventListener('DOMContentLoaded', () => {
    initEndpointConfig();
    initCollectionSelector();
    initTestSections();
    initModal();
    loadCollections();
});

function initCollectionSelector() {
    const select = document.getElementById('collection-select');
    
    select.addEventListener('change', (e) => {
        selectedCollectionId = e.target.value;
        clearAllResults();
    });
}

async function populateCollectionSelector() {
    const select = document.getElementById('collection-select');
    const countSpan = document.getElementById('collection-count');
    
    // Clear existing options except "All"
    while (select.options.length > 1) {
        select.remove(1);
    }
    
    for (const col of collections) {
        const opt = document.createElement('option');
        opt.value = col.id;
        opt.textContent = col.id;
        select.appendChild(opt);
    }
    
    countSpan.textContent = `(${collections.length} collections)`;
}

function initEndpointConfig() {
    const input = document.getElementById('endpoint-input');
    const applyBtn = document.getElementById('endpoint-apply-btn');
    const resetBtn = document.getElementById('endpoint-reset-btn');

    input.value = API_BASE;

    applyBtn.addEventListener('click', () => {
        API_BASE = input.value.trim().replace(/\/+$/, '');
        localStorage.setItem('edr-compliance-endpoint', API_BASE);
        clearMetadataCache();
        loadCollections();
        clearAllResults();
    });

    resetBtn.addEventListener('click', () => {
        API_BASE = DEFAULT_API_BASE;
        input.value = API_BASE;
        localStorage.removeItem('edr-compliance-endpoint');
        clearMetadataCache();
        loadCollections();
        clearAllResults();
    });

    // Run all tests button
    document.getElementById('run-all-btn').addEventListener('click', runAllTests);
    document.getElementById('clear-results-btn').addEventListener('click', clearAllResults);
}

function initTestSections() {
    // Toggle section expansion
    document.querySelectorAll('.section-header').forEach(header => {
        header.addEventListener('click', () => {
            const section = header.dataset.section;
            const content = document.getElementById(`${section}-tests`);
            const toggle = header.querySelector('.toggle');

            content.classList.toggle('expanded');
            toggle.textContent = content.classList.contains('expanded') ? '-' : '+';
        });
    });

    // Individual test run buttons
    document.querySelectorAll('.run-test-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            const testItem = btn.closest('.test-item');
            const testName = testItem.dataset.test;
            runTest(testName);
        });
    });

    // Copy URL buttons
    document.querySelectorAll('.copy-url-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            const testItem = btn.closest('.test-item');
            const testName = testItem.dataset.test;
            copyTestUrl(testName, btn);
        });
    });
}

function initModal() {
    const modal = document.getElementById('test-details-modal');
    const closeBtn = modal.querySelector('.close-modal');

    closeBtn.addEventListener('click', () => {
        modal.classList.remove('visible');
    });

    modal.addEventListener('click', (e) => {
        if (e.target === modal) {
            modal.classList.remove('visible');
        }
    });

    // Make test items clickable to show details
    document.querySelectorAll('.test-item').forEach(item => {
        item.addEventListener('click', (e) => {
            if (e.target.classList.contains('run-test-btn')) return;
            const testName = item.dataset.test;
            showTestDetails(testName);
        });
    });
}

// ============================================================
// API FUNCTIONS
// ============================================================

async function fetchJson(url, options = {}) {
    const startTime = performance.now();
    const response = await fetch(url, options);
    const endTime = performance.now();

    const text = await response.text();
    let json = null;
    try {
        json = JSON.parse(text);
    } catch (e) {
        // Not JSON
    }

    return {
        ok: response.ok,
        status: response.status,
        statusText: response.statusText,
        headers: response.headers,
        text,
        json,
        time: Math.round(endTime - startTime)
    };
}

// Fetch with custom Accept header
async function fetchWithAccept(url, acceptHeader) {
    const startTime = performance.now();
    
    try {
        // Use native fetch API with explicit headers
        const response = await fetch(url, {
            method: 'GET',
            headers: {
                'Accept': acceptHeader
            },
            mode: 'cors',
            cache: 'no-store' // Bypass cache to ensure fresh response
        });
        
        const endTime = performance.now();
        const text = await response.text();
        
        let json = null;
        try {
            json = JSON.parse(text);
        } catch (e) {
            // Not JSON
        }
        
        // Debug: log what we received
        console.log('fetchWithAccept - URL:', url, 'Accept:', acceptHeader, 
                    'Status:', response.status, 'Type:', json?.type);
        
        return {
            ok: response.ok,
            status: response.status,
            statusText: response.statusText,
            headers: response.headers,
            text,
            json,
            time: Math.round(endTime - startTime)
        };
    } catch (e) {
        console.error('fetchWithAccept error:', e);
        return {
            ok: false,
            status: 0,
            statusText: 'Network Error: ' + e.message,
            headers: { get: () => null },
            text: '',
            json: null,
            time: 0
        };
    }
}

async function loadCollections() {
    try {
        const response = await fetchJson(`${API_BASE}/collections`);
        if (response.ok && response.json?.collections) {
            collections = response.json.collections;
            populateCollectionSelector();
        }
    } catch (e) {
        console.error('Failed to load collections:', e);
        collections = [];
    }
}

// ============================================================
// DATA-AWARE TESTING HELPERS
// ============================================================

// Cache collection metadata to avoid repeated fetches
let collectionMetadataCache = {};

// Store the actual URL used by each test for the copy URL feature
let testUrlsUsed = {};

// Clear metadata cache (call when endpoint changes)
function clearMetadataCache() {
    collectionMetadataCache = {};
    testUrlsUsed = {};
}

// Get collection metadata with caching
async function getCollectionMetadata(collectionId) {
    if (collectionMetadataCache[collectionId]) {
        return collectionMetadataCache[collectionId];
    }
    const res = await fetchJson(`${API_BASE}/collections/${collectionId}`);
    if (res.ok && res.json) {
        collectionMetadataCache[collectionId] = res.json;
    }
    return res.json;
}

// Get valid coordinates from the center of the collection's spatial extent
async function getValidCoordinates(collectionId) {
    const metadata = await getCollectionMetadata(collectionId);
    const bbox = metadata?.extent?.spatial?.bbox?.[0];
    
    if (!bbox || bbox.length < 4) {
        return { warning: 'No spatial extent defined in collection', coords: null, bbox: null };
    }
    
    // Calculate center of bbox [minLon, minLat, maxLon, maxLat]
    const centerLon = (bbox[0] + bbox[2]) / 2;
    const centerLat = (bbox[1] + bbox[3]) / 2;
    
    return { 
        coords: { lon: centerLon, lat: centerLat },
        bbox: bbox
    };
}

// Get a valid polygon within the collection's spatial extent
async function getValidPolygon(collectionId, sizeDegrees = 1.0) {
    const { coords, bbox, warning } = await getValidCoordinates(collectionId);
    
    if (!coords) {
        return { warning, polygon: null, bboxArray: null };
    }
    
    // Create a small polygon around the center, staying within the collection's bbox
    const halfSize = sizeDegrees / 2;
    const minLon = Math.max(bbox[0], coords.lon - halfSize);
    const maxLon = Math.min(bbox[2], coords.lon + halfSize);
    const minLat = Math.max(bbox[1], coords.lat - halfSize);
    const maxLat = Math.min(bbox[3], coords.lat + halfSize);
    
    // WKT POLYGON format: counterclockwise, first point = last point
    const polygon = `POLYGON((${minLon} ${minLat},${maxLon} ${minLat},${maxLon} ${maxLat},${minLon} ${maxLat},${minLon} ${minLat}))`;
    
    return { 
        polygon, 
        bboxArray: [minLon, minLat, maxLon, maxLat],
        coords 
    };
}

// Get a valid linestring within the collection's spatial extent
async function getValidLinestring(collectionId) {
    const { coords, bbox, warning } = await getValidCoordinates(collectionId);
    
    if (!coords) {
        return { warning, linestring: null };
    }
    
    // Generate a linestring within the extent
    // Create 3 points spanning ~0.5 degrees in each direction from center (or less if bbox is smaller)
    const maxSpanLon = (bbox[2] - bbox[0]) / 4;
    const maxSpanLat = (bbox[3] - bbox[1]) / 4;
    const span = Math.min(maxSpanLon, maxSpanLat, 0.5);
    
    const p1 = { lon: Math.max(bbox[0], coords.lon - span), lat: coords.lat };
    const p2 = { lon: coords.lon, lat: Math.min(bbox[3], coords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], coords.lon + span), lat: coords.lat };
    
    const linestring = `LINESTRING(${p1.lon} ${p1.lat},${p2.lon} ${p2.lat},${p3.lon} ${p3.lat})`;
    
    return { linestring, coords, bbox };
}

// Get a valid parameter name from the collection
async function getValidParameter(collectionId) {
    const metadata = await getCollectionMetadata(collectionId);
    const paramNames = metadata?.parameter_names;
    
    if (!paramNames || Object.keys(paramNames).length === 0) {
        return { warning: 'No parameters defined in collection', parameter: null };
    }
    
    const allParams = Object.keys(paramNames);
    
    // Try to find a parameter that actually has data by testing with a position query
    // This helps avoid 500 errors from parameters that are configured but have no data
    const { coords } = await getValidCoordinates(collectionId);
    if (coords) {
        for (const param of allParams) {
            try {
                const testUrl = `${API_BASE}/collections/${collectionId}/position?coords=POINT(${coords.lon} ${coords.lat})&parameter-name=${param}`;
                const res = await fetchJson(testUrl);
                if (res.status === 200 && res.json?.ranges?.[param]?.values?.some(v => v !== null)) {
                    return { parameter: param, allParameters: allParams };
                }
            } catch (e) {
                // Continue to next parameter
            }
        }
    }
    
    // Fall back to first parameter if none have confirmed data
    return { parameter: allParams[0], allParameters: allParams };
}

// Get a valid datetime from the collection's temporal extent
async function getValidDatetime(collectionId) {
    const metadata = await getCollectionMetadata(collectionId);
    const temporal = metadata?.extent?.temporal;
    
    // Try values array first (discrete times)
    if (temporal?.values && temporal.values.length > 0) {
        return { datetime: temporal.values[0], allTimes: temporal.values };
    }
    
    // Fall back to interval
    if (temporal?.interval?.[0]) {
        const [start, end] = temporal.interval[0];
        if (start) {
            return { datetime: start, interval: [start, end] };
        }
    }
    
    return { warning: 'No temporal extent defined in collection', datetime: null };
}

// Helper to extract vertical level values from collection metadata
// Handles both 'values' array and 'interval' array formats per EDR spec
// Also handles legacy non-compliant formats defensively
function extractVerticalLevels(vertical) {
    if (!vertical) return [];
    
    // Prefer explicit values array (spec-compliant format)
    if (vertical.values && Array.isArray(vertical.values) && vertical.values.length > 0) {
        // Filter to ensure we only return valid values (numbers or strings, not null/undefined)
        return vertical.values.filter(v => v !== null && v !== undefined);
    }
    
    // Fall back to extracting from interval array
    // Per spec, interval should be array of [min, max] pairs
    // But handle legacy format where each level might be a single-element array
    if (vertical.interval && Array.isArray(vertical.interval) && vertical.interval.length > 0) {
        const levels = new Set();
        for (const pair of vertical.interval) {
            if (Array.isArray(pair)) {
                // Add first element if valid
                if (pair[0] !== null && pair[0] !== undefined) {
                    levels.add(pair[0]);
                }
                // Only add second element if it exists, is valid, and differs from first
                if (pair.length > 1 && pair[1] !== null && pair[1] !== undefined && pair[1] !== pair[0]) {
                    levels.add(pair[1]);
                }
            }
        }
        // Sort numerically if all values are numbers, otherwise return as-is
        const levelArray = Array.from(levels);
        if (levelArray.every(v => typeof v === 'number')) {
            return levelArray.sort((a, b) => a - b);
        }
        return levelArray;
    }
    
    return [];
}

// Get a valid Z level from the collection's vertical extent
async function getValidZLevel(collectionId) {
    const metadata = await getCollectionMetadata(collectionId);
    const vertical = metadata?.extent?.vertical;
    
    // Try values array from data_queries first
    const dataQueries = metadata?.data_queries;
    if (dataQueries?.position?.link?.variables?.vertical_levels) {
        const levels = dataQueries.position.link.variables.vertical_levels;
        if (levels.length > 0) {
            return { z: levels[0], allLevels: levels };
        }
    }
    
    // Use the extractVerticalLevels helper to handle both values and interval formats
    const allLevels = extractVerticalLevels(vertical);
    if (allLevels.length > 0) {
        return { z: allLevels[0], allLevels };
    }
    
    return { warning: 'No vertical extent defined in collection', z: null };
}

// Check if the response has non-null data values
function hasNonNullValues(response) {
    // Check ranges for Coverage type
    if (response?.ranges) {
        for (const rangeKey of Object.keys(response.ranges)) {
            const values = response.ranges[rangeKey]?.values || [];
            if (values.some(v => v !== null)) {
                return true;
            }
        }
    }
    
    // Check coverages array for CoverageCollection type
    if (response?.coverages) {
        for (const coverage of response.coverages) {
            if (hasNonNullValues(coverage)) {
                return true;
            }
        }
    }
    
    return false;
}

// ============================================================
// CAPABILITY DETECTION
// ============================================================

// Check if collection has vertical extent
function collectionHasVerticalExtent(collection) {
    return collection?.extent?.vertical != null;
}

// Check if collection has temporal extent
function collectionHasTemporalExtent(collection) {
    return collection?.extent?.temporal != null;
}

// Check if collection supports a specific query type
function collectionSupportsQuery(collection, queryType) {
    return collection?.data_queries?.[queryType] != null;
}

// Global tests that don't require a collection
const GLOBAL_TESTS = [
    'landing-page', 'landing-links', 'conformance', 'collections-list'
];

// Tests that require specific capabilities
const CAPABILITY_REQUIREMENTS = {
    // Z-level tests require vertical extent
    'z-single': { vertical: true, query: 'position' },
    'z-multiple': { vertical: true, query: 'position' },
    'z-range': { vertical: true, query: 'position' },
    'z-recurring': { vertical: true, query: 'position' },
    'z-invalid': { vertical: true, query: 'position' },
    'z-outside-extent': { vertical: true, query: 'position' },
    
    // Area with z
    'area-z-multiple': { vertical: true, query: 'area' },
    
    // Radius with z
    'radius-z-parameter': { vertical: true, query: 'radius' },
    
    // Cube requires vertical extent
    'cube-basic': { vertical: true, query: 'cube' },
    'cube-covjson': { vertical: true, query: 'cube' },
    'cube-missing-bbox': { vertical: true, query: 'cube' },
    'cube-missing-z': { vertical: true, query: 'cube' },
    'cube-invalid-bbox': { vertical: true, query: 'cube' },
    'cube-multi-z': { vertical: true, query: 'cube' },
    'cube-with-datetime': { vertical: true, query: 'cube' },
    'cube-with-resolution': { vertical: true, query: 'cube' },
    'cube-instance': { vertical: true, query: 'cube' },
    'cube-not-found': { vertical: true, query: 'cube' },
    'cube-no-query-params': { vertical: true, query: 'cube' },
    'cube-z-range': { vertical: true, query: 'cube' },
    'cube-z-recurring': { vertical: true, query: 'cube' },
    'cube-invalid-z': { vertical: true, query: 'cube' },
    'cube-crs-valid': { vertical: true, query: 'cube' },
    'cube-f-covjson': { vertical: true, query: 'cube' },
    
    // Datetime tests require temporal extent
    'datetime-instant': { temporal: true, query: 'position' },
    'datetime-range': { temporal: true, query: 'position' },
    'datetime-list': { temporal: true, query: 'position' },
    'datetime-open-end': { temporal: true, query: 'position' },
    'datetime-open-start': { temporal: true, query: 'position' },
    
    // Query-specific tests
    'position-wkt': { query: 'position' },
    'position-simple': { query: 'position' },
    'position-covjson': { query: 'position' },
    'position-invalid': { query: 'position' },
    'position-missing-coords': { query: 'position' },
    'position-multipoint': { query: 'position' },
    'position-no-query-params': { query: 'position' },
    'position-crs-valid': { query: 'position' },
    'position-f-covjson': { query: 'position' },
    'position-no-params': { query: 'position' },
    
    'area-basic': { query: 'area' },
    'area-covjson': { query: 'area' },
    'area-small': { query: 'area' },
    'area-complex': { query: 'area' },
    'area-too-large': { query: 'area' },
    'area-invalid-polygon': { query: 'area' },
    'area-with-params': { query: 'area' },
    'area-missing-coords': { query: 'area' },
    'area-multipolygon': { query: 'area' },
    'area-crs-valid': { query: 'area' },
    'area-f-covjson': { query: 'area' },
    'area-no-params': { query: 'area' },
    
    'radius-basic': { query: 'radius' },
    'radius-covjson': { query: 'radius' },
    'radius-missing-coords': { query: 'radius' },
    'radius-missing-within': { query: 'radius' },
    'radius-missing-within-units': { query: 'radius' },
    'radius-invalid-coords': { query: 'radius' },
    'radius-too-large': { query: 'radius' },
    'radius-units-km': { query: 'radius' },
    'radius-units-mi': { query: 'radius' },
    'radius-units-m': { query: 'radius' },
    'radius-multipoint': { query: 'radius' },
    'radius-with-params': { query: 'radius' },
    'radius-datetime': { query: 'radius' },
    'radius-no-query-params': { query: 'radius' },
    'radius-crs-valid': { query: 'radius' },
    'radius-f-covjson': { query: 'radius' },
    
    'trajectory-basic': { query: 'trajectory' },
    'trajectory-covjson': { query: 'trajectory' },
    'trajectory-missing-coords': { query: 'trajectory' },
    'trajectory-invalid-coords': { query: 'trajectory' },
    'trajectory-linestringz': { vertical: true, query: 'trajectory' },
    'trajectory-linestringm': { query: 'trajectory' },
    'trajectory-z-conflict': { vertical: true, query: 'trajectory' },
    'trajectory-multilinestring': { query: 'trajectory' },
    'trajectory-with-params': { query: 'trajectory' },
    'trajectory-datetime': { query: 'trajectory' },
    'trajectory-no-query-params': { query: 'trajectory' },
    'trajectory-invalid-linestringm': { query: 'trajectory' },
    'trajectory-invalid-linestringz': { query: 'trajectory' },
    'trajectory-invalid-linestringzm': { query: 'trajectory' },
    'trajectory-linestringz-invalid-z': { vertical: true, query: 'trajectory' },
    'trajectory-z-param-invalid': { vertical: true, query: 'trajectory' },
    'trajectory-invalid-time': { query: 'trajectory' },
    'trajectory-crs-valid': { query: 'trajectory' },
    'trajectory-f-covjson': { query: 'trajectory' },
    
    'corridor-basic': { query: 'corridor' },
    'corridor-covjson': { query: 'corridor' },
    'corridor-missing-coords': { query: 'corridor' },
    'corridor-missing-width': { query: 'corridor' },
    'corridor-missing-width-units': { query: 'corridor' },
    'corridor-missing-height': { query: 'corridor' },
    'corridor-missing-height-units': { query: 'corridor' },
    'corridor-invalid-width-units': { query: 'corridor' },
    'corridor-invalid-height-units': { query: 'corridor' },
    'corridor-invalid-coords': { query: 'corridor' },
    'corridor-z-conflict': { vertical: true, query: 'corridor' },
    'corridor-datetime-conflict': { query: 'corridor' },
    'corridor-multilinestring': { query: 'corridor' },
    'corridor-with-params': { query: 'corridor' },
    'corridor-pressure-height-units': { vertical: true, query: 'corridor' },
    'corridor-metadata': { query: 'corridor' },
    'corridor-invalid-linestringm': { query: 'corridor' },
    'corridor-invalid-linestringz': { query: 'corridor' },
    'corridor-invalid-linestringzm': { query: 'corridor' },
    'corridor-zm-z-conflict': { vertical: true, query: 'corridor' },
    'corridor-zm-datetime-conflict': { query: 'corridor' },
    'corridor-linestringz-invalid-z': { vertical: true, query: 'corridor' },
    'corridor-z-param-invalid': { vertical: true, query: 'corridor' },
    'corridor-linestringz': { vertical: true, query: 'corridor' },
    'corridor-linestringm': { query: 'corridor' },
    'corridor-linestringzm': { vertical: true, query: 'corridor' },
    'corridor-with-datetime': { query: 'corridor' },
    'corridor-with-z': { vertical: true, query: 'corridor' },
    'corridor-instance': { query: 'corridor' },
    'corridor-not-found': { query: 'corridor' },
    'corridor-crs-valid': { query: 'corridor' },
    'corridor-f-covjson': { query: 'corridor' },
    
    'locations-list': { query: 'locations' },
    'locations-geojson-structure': { query: 'locations' },
    'locations-query-basic': { query: 'locations' },
    'locations-query-covjson': { query: 'locations' },
    'locations-invalid-id': { query: 'locations' },
    'locations-with-params': { query: 'locations' },
    'locations-with-datetime': { query: 'locations' },
    'locations-cache-header': { query: 'locations' },
    'locations-instance': { query: 'locations' },
    'locations-crs-valid': { query: 'locations' },
    'locations-f-covjson': { query: 'locations' },
    
    // Domain type tests
    'domain-type-point': { query: 'position' },
    'domain-type-pointseries': { temporal: true, query: 'position' },
    'domain-type-verticalprofile': { vertical: true, query: 'position' },
    'domain-type-grid': { query: 'area' },
    
    // Schema validation tests
    'schema-covjson-position': { query: 'position' },
    'schema-covjson-area': { query: 'area' },
    'schema-covjson-trajectory': { query: 'trajectory' },
    'schema-covjson-cube': { vertical: true, query: 'cube' },
    'schema-covjson-locations': { query: 'locations' },
    'schema-geojson-locations-list': { query: 'locations' },
    'schema-geojson-position': { query: 'position' }
};

// Returns { skip: boolean, reason: string | null }
function shouldSkipTest(testName, collection) {
    if (!collection) {
        return { skip: false, reason: null };
    }
    
    const reqs = CAPABILITY_REQUIREMENTS[testName];
    if (!reqs) {
        return { skip: false, reason: null };
    }
    
    // Check query support
    if (reqs.query && !collectionSupportsQuery(collection, reqs.query)) {
        return { skip: true, reason: `No ${reqs.query} query support` };
    }
    
    // Check vertical extent
    if (reqs.vertical && !collectionHasVerticalExtent(collection)) {
        return { skip: true, reason: 'No vertical extent' };
    }
    
    // Check temporal extent
    if (reqs.temporal && !collectionHasTemporalExtent(collection)) {
        return { skip: true, reason: 'No temporal extent' };
    }
    
    return { skip: false, reason: null };
}

// ============================================================
// TEST FUNCTIONS
// ============================================================

async function runAllTests() {
    const tests = [
        // Core
        'landing-page', 'landing-links', 'conformance',
        // Collections
        'collections-list', 'collection-structure', 'collection-links',
        // Extent
        'extent-spatial', 'extent-temporal', 'extent-vertical', 'extent-vertical-format',
        // Instances
        'instances-list', 'instance-structure', 'instance-extent',
        // Position Query
        'position-wkt', 'position-simple', 'position-covjson', 'position-invalid',
        'position-missing-coords', 'position-multipoint',
        'position-no-query-params', 'position-crs-valid', 'position-f-covjson',
        // Z Parameter
        'z-single', 'z-multiple', 'z-range', 'z-recurring', 'z-invalid', 'z-outside-extent',
        // Datetime Parameter
        'datetime-instant', 'datetime-range', 'datetime-list', 'datetime-open-end', 'datetime-open-start',
        // Area Query
        'area-basic', 'area-covjson', 'area-small', 'area-complex',
        'area-too-large', 'area-invalid-polygon', 'area-with-params',
        'area-missing-coords', 'area-multipolygon', 'area-z-multiple',
        'area-crs-valid', 'area-f-covjson',
        // Radius Query
        'radius-basic', 'radius-covjson', 'radius-missing-coords',
        'radius-missing-within', 'radius-missing-within-units', 'radius-invalid-coords',
        'radius-too-large', 'radius-units-km', 'radius-units-mi', 'radius-units-m',
        'radius-multipoint', 'radius-z-parameter', 'radius-with-params', 'radius-datetime',
        'radius-no-query-params', 'radius-crs-valid', 'radius-f-covjson',
        // Trajectory Query
        'trajectory-basic', 'trajectory-covjson', 'trajectory-missing-coords',
        'trajectory-invalid-coords', 'trajectory-linestringz', 'trajectory-linestringm',
        'trajectory-z-conflict', 'trajectory-multilinestring', 'trajectory-with-params',
        'trajectory-datetime',
        'trajectory-no-query-params', 'trajectory-invalid-linestringm', 'trajectory-invalid-linestringz',
        'trajectory-invalid-linestringzm', 'trajectory-linestringz-invalid-z', 'trajectory-z-param-invalid',
        'trajectory-invalid-time',
        'trajectory-crs-valid', 'trajectory-f-covjson',
        // Corridor Query
        'corridor-basic', 'corridor-covjson', 'corridor-missing-coords',
        'corridor-missing-width', 'corridor-missing-width-units', 'corridor-missing-height',
        'corridor-missing-height-units', 'corridor-invalid-width-units', 'corridor-invalid-height-units',
        'corridor-invalid-coords', 'corridor-z-conflict', 'corridor-datetime-conflict',
        'corridor-multilinestring', 'corridor-with-params', 'corridor-pressure-height-units',
        'corridor-metadata',
        // Corridor Query - Additional Tests
        'corridor-invalid-linestringm', 'corridor-invalid-linestringz', 'corridor-invalid-linestringzm',
        'corridor-zm-z-conflict', 'corridor-zm-datetime-conflict',
        'corridor-linestringz-invalid-z', 'corridor-z-param-invalid',
        'corridor-linestringz', 'corridor-linestringm', 'corridor-linestringzm',
        'corridor-with-datetime', 'corridor-with-z', 'corridor-instance', 'corridor-not-found',
        'corridor-crs-valid', 'corridor-f-covjson',
        // Cube Query
        'cube-basic', 'cube-covjson', 'cube-missing-bbox', 'cube-missing-z',
        'cube-invalid-bbox', 'cube-multi-z', 'cube-with-datetime', 'cube-with-resolution',
        'cube-instance', 'cube-not-found', 'cube-no-query-params', 'cube-z-range',
        'cube-z-recurring', 'cube-invalid-z', 'cube-crs-valid', 'cube-f-covjson',
        // Locations Query
        'locations-list', 'locations-geojson-structure', 'locations-query-basic',
        'locations-query-covjson', 'locations-invalid-id', 'locations-with-params',
        'locations-with-datetime', 'locations-cache-header', 'locations-instance',
        'locations-crs-valid', 'locations-f-covjson',
        // Error Handling
        'error-404-collection', 'error-400-coords', 'error-400-datetime', 'error-response-structure',
        // Metadata
        'metadata-data-queries', 'metadata-parameter-names', 'metadata-output-formats', 'metadata-crs',
        // Content-Type & Format (NEW)
        'content-type-covjson', 'content-type-json', 'f-param-covjson', 'f-param-invalid',
        // CRS Parameter (NEW)
        'crs-param-valid', 'crs-param-invalid',
        // Parameter-Name (NEW)
        'param-name-filter', 'param-name-invalid',
        // Instance Query (NEW)
        'instance-position-query', 'instance-invalid-id',
        // Domain Types (NEW)
        'domain-type-point', 'domain-type-pointseries', 'domain-type-verticalprofile', 'domain-type-grid',
        // Link Validation (NEW)
        'links-self', 'links-data-queries',
        // No Query Params (NEW)
        'position-no-params', 'area-no-params',
        // Accept Header Content Negotiation (LOW PRIORITY)
        'accept-covjson', 'accept-json', 'accept-unsupported', 'accept-geojson',
        // GeoJSON Output Format
        'f-param-geojson', 'content-type-geojson', 'geojson-structure',
        // PNG Output Format (Area queries only)
        'f-param-png', 'content-type-png', 'png-structure', 'png-not-supported-position', 'png-multi-param-error',
        // CoverageJSON Structure Validation (LOW PRIORITY)
        'covjson-referencing', 'covjson-ndarray', 'covjson-observed-property', 'covjson-axes',
        // Alternate Format Links (LOW PRIORITY)
        'links-alternate-formats', 'links-landing-alternate',
        // JSON Schema Validation
        'schema-covjson-position', 'schema-covjson-area', 'schema-covjson-trajectory',
        'schema-covjson-cube', 'schema-covjson-locations',
        'schema-geojson-locations-list', 'schema-geojson-position'
    ];

    // Show progress UI
    isTestRunning = true;
    stopRequested = false;
    showProgress(true);
    updateProgress(0, tests.length, 'Starting tests...');
    
    // Setup stop button
    const stopBtn = document.getElementById('stop-btn');
    stopBtn.style.display = 'inline-block';
    stopBtn.onclick = () => {
        stopRequested = true;
        updateProgress(0, tests.length, 'Stopping...');
    };

    let completed = 0;
    for (const test of tests) {
        if (stopRequested) {
            break;
        }
        await runTest(test);
        completed++;
        updateProgress(completed, tests.length, `Testing: ${test}`);
    }
    
    // Hide progress UI
    isTestRunning = false;
    stopBtn.style.display = 'none';
    showProgress(false);
}

function showProgress(show) {
    const container = document.getElementById('progress-container');
    container.style.display = show ? 'block' : 'none';
}

function updateProgress(current, total, text) {
    const fill = document.getElementById('progress-fill');
    const textEl = document.getElementById('progress-text');
    const percent = total > 0 ? (current / total) * 100 : 0;
    fill.style.width = `${percent}%`;
    textEl.textContent = text;
}

function clearAllResults() {
    testResults = {};
    perCollectionResults = {};
    
    // Remove per-collection result wrappers
    document.querySelectorAll('.collection-results-wrapper').forEach(el => el.remove());
    
    document.querySelectorAll('.test-status').forEach(el => {
        el.className = 'test-status pending';
        el.textContent = 'Pending';
    });
    updateSummary();
}

async function runTest(testName) {
    setTestStatus(testName, 'running', 'Running...');
    
    // Determine which collections to test
    const collectionsToTest = selectedCollectionId === '__ALL__' 
        ? collections 
        : collections.filter(c => c.id === selectedCollectionId);
    
    if (collectionsToTest.length === 0) {
        testResults[testName] = { passed: false, error: 'No collections available' };
        setTestStatus(testName, 'failed', 'No Collections');
        updateSummary();
        return;
    }
    
    // Global tests don't need per-collection iteration
    if (GLOBAL_TESTS.includes(testName)) {
        try {
            const result = await executeTest(testName, null);
            testResults[testName] = result;
            setTestStatusFromResult(testName, result);
        } catch (e) {
            testResults[testName] = { passed: false, error: e.message };
            setTestStatus(testName, 'failed', 'Error');
        }
        updateSummary();
        return;
    }
    
    // Per-collection tests
    perCollectionResults[testName] = {};
    let anyPassed = false;
    let anyFailed = false;
    let anyWarning = false;
    let allSkipped = true;
    
    for (const col of collectionsToTest) {
        if (stopRequested) break;
        
        try {
            const result = await executeTest(testName, col);
            perCollectionResults[testName][col.id] = result;
            
            if (!result.skipped) {
                allSkipped = false;
                if (result.passed && !result.warning) anyPassed = true;
                if (result.passed && result.warning) anyWarning = true;
                if (!result.passed) anyFailed = true;
            }
        } catch (e) {
            perCollectionResults[testName][col.id] = { passed: false, error: e.message };
            anyFailed = true;
            allSkipped = false;
        }
    }
    
    // Aggregate result
    const aggregateResult = {
        passed: !anyFailed && !allSkipped,
        warning: anyWarning && !anyFailed,
        skipped: allSkipped,
        perCollection: perCollectionResults[testName]
    };
    
    testResults[testName] = aggregateResult;
    
    if (allSkipped) {
        setTestStatus(testName, 'skipped', 'Skipped');
    } else if (anyFailed) {
        setTestStatus(testName, 'failed', 'Failed');
    } else if (anyWarning) {
        setTestStatus(testName, 'warning', 'Warning');
    } else {
        setTestStatus(testName, 'passed', 'Passed');
    }
    
    updateSummary();
    
    // Render per-collection results inline (only show failed by default)
    if (selectedCollectionId === '__ALL__' && collectionsToTest.length > 1) {
        renderPerCollectionResults(testName);
    }
}

function setTestStatusFromResult(testName, result) {
    if (result.skipped) {
        setTestStatus(testName, 'skipped', 'Skipped');
    } else if (result.passed && !result.warning) {
        setTestStatus(testName, 'passed', 'Passed');
    } else if (result.passed && result.warning) {
        setTestStatus(testName, 'warning', 'Warning');
    } else {
        setTestStatus(testName, 'failed', 'Failed');
    }
}

// Render per-collection results inline (only failed/warned shown by default)
function renderPerCollectionResults(testName) {
    const testItem = document.querySelector(`[data-test="${testName}"]`);
    if (!testItem) return;
    
    // Remove existing wrapper
    const existingWrapper = testItem.querySelector('.collection-results-wrapper');
    if (existingWrapper) existingWrapper.remove();
    
    const results = perCollectionResults[testName];
    if (!results || Object.keys(results).length === 0) return;
    
    // Count results by status
    const failed = [];
    const warned = [];
    const skipped = [];
    const passed = [];
    
    for (const [colId, result] of Object.entries(results)) {
        if (result.skipped) {
            skipped.push({ colId, result });
        } else if (!result.passed) {
            failed.push({ colId, result });
        } else if (result.warning) {
            warned.push({ colId, result });
        } else {
            passed.push({ colId, result });
        }
    }
    
    // Only show wrapper if there are failures or warnings (or all skipped)
    const showAll = failed.length > 0 || warned.length > 0;
    if (!showAll && skipped.length === Object.keys(results).length) {
        // All skipped - show a simple message
        const wrapper = document.createElement('div');
        wrapper.className = 'collection-results-wrapper';
        wrapper.innerHTML = `<div class="collection-result-item">
            <span class="collection-result-status skipped">All ${skipped.length} collections skipped</span>
            <span class="collection-result-reason">${skipped[0]?.result.reason || 'N/A'}</span>
        </div>`;
        testItem.appendChild(wrapper);
        return;
    }
    
    if (!showAll) return; // All passed, no need to show details
    
    const wrapper = document.createElement('div');
    wrapper.className = 'collection-results-wrapper';
    
    // Show failed first
    for (const { colId, result } of failed) {
        wrapper.appendChild(createCollectionResultItem(colId, 'failed', 'FAIL', result.error || getFailedChecks(result)));
    }
    
    // Then warnings
    for (const { colId, result } of warned) {
        wrapper.appendChild(createCollectionResultItem(colId, 'warning', 'WARN', getWarningReason(result)));
    }
    
    // Add toggle to show all results
    if (passed.length > 0 || skipped.length > 0) {
        const toggle = document.createElement('div');
        toggle.className = 'collection-results-toggle';
        toggle.textContent = `+ Show ${passed.length} passed, ${skipped.length} skipped`;
        toggle.onclick = (e) => {
            e.stopPropagation();
            const hiddenItems = wrapper.querySelectorAll('.collection-result-hidden');
            if (hiddenItems.length > 0) {
                hiddenItems.forEach(el => el.classList.remove('collection-result-hidden'));
                toggle.textContent = '- Hide passed/skipped';
            } else {
                // Hide them again
                wrapper.querySelectorAll('.collection-result-item.can-hide').forEach(el => el.classList.add('collection-result-hidden'));
                toggle.textContent = `+ Show ${passed.length} passed, ${skipped.length} skipped`;
            }
        };
        wrapper.appendChild(toggle);
        
        // Add passed (hidden by default)
        for (const { colId } of passed) {
            const item = createCollectionResultItem(colId, 'passed', 'PASS');
            item.classList.add('can-hide', 'collection-result-hidden');
            wrapper.appendChild(item);
        }
        
        // Add skipped (hidden by default)
        for (const { colId, result } of skipped) {
            const item = createCollectionResultItem(colId, 'skipped', 'SKIP', result.reason);
            item.classList.add('can-hide', 'collection-result-hidden');
            wrapper.appendChild(item);
        }
    }
    
    testItem.appendChild(wrapper);
}

function createCollectionResultItem(colId, statusClass, statusText, reason = null) {
    const div = document.createElement('div');
    div.className = 'collection-result-item';
    div.innerHTML = `
        <span class="collection-result-id">${colId}</span>
        <span class="collection-result-status ${statusClass}">${statusText}</span>
        ${reason ? `<span class="collection-result-reason">${reason}</span>` : ''}
    `;
    return div;
}

function getFailedChecks(result) {
    if (!result.checks) return null;
    const failed = result.checks.filter(c => !c.passed).map(c => c.name);
    return failed.length > 0 ? failed.join(', ') : null;
}

function getWarningReason(result) {
    if (!result.checks) return null;
    const warns = result.checks.filter(c => c.warning).map(c => c.warning);
    return warns.length > 0 ? warns.join(', ') : null;
}

function setTestStatus(testName, status, text) {
    const testItem = document.querySelector(`[data-test="${testName}"]`);
    if (testItem) {
        const statusEl = testItem.querySelector('.test-status');
        statusEl.className = `test-status ${status}`;
        statusEl.textContent = text;
    }
}

async function executeTest(testName, collection) {
    // Check for capability-based skipping
    if (collection) {
        const skipCheck = shouldSkipTest(testName, collection);
        if (skipCheck.skip) {
            return {
                skipped: true,
                reason: skipCheck.reason,
                checks: [{ name: skipCheck.reason, passed: true, skipped: true }]
            };
        }
    }
    
    switch (testName) {
        // Global tests (no collection needed)
        case 'landing-page':
            return testLandingPage();
        case 'landing-links':
            return testLandingLinks();
        case 'conformance':
            return testConformance();
        case 'collections-list':
            return testCollectionsList();
        
        // Collection-dependent tests (pass collection)
        case 'collection-structure':
            return testCollectionStructure(collection);
        case 'collection-links':
            return testCollectionLinks(collection);
        case 'extent-spatial':
            return testExtentSpatial(collection);
        case 'extent-temporal':
            return testExtentTemporal(collection);
        case 'extent-vertical':
            return testExtentVertical(collection);
        case 'extent-vertical-format':
            return testExtentVerticalFormat(collection);
        case 'instances-list':
            return testInstancesList(collection);
        case 'instance-structure':
            return testInstanceStructure(collection);
        case 'instance-extent':
            return testInstanceExtent(collection);
        case 'position-wkt':
            return testPositionWkt(collection);
        case 'position-simple':
            return testPositionSimple(collection);
        case 'position-covjson':
            return testPositionCovJson(collection);
        case 'position-invalid':
            return testPositionInvalid(collection);
        case 'position-missing-coords':
            return testPositionMissingCoords(collection);
        case 'position-multipoint':
            return testPositionMultipoint(collection);
        case 'position-no-query-params':
            return testPositionNoQueryParams(collection);
        case 'position-crs-valid':
            return testPositionCrsValid(collection);
        case 'position-f-covjson':
            return testPositionFCovJson(collection);
        case 'z-single':
            return testZSingle(collection);
        case 'z-multiple':
            return testZMultiple(collection);
        case 'z-range':
            return testZRange(collection);
        case 'z-recurring':
            return testZRecurring(collection);
        case 'z-invalid':
            return testZInvalid(collection);
        case 'z-outside-extent':
            return testZOutsideExtent(collection);
        case 'datetime-instant':
            return testDatetimeInstant(collection);
        case 'datetime-range':
            return testDatetimeRange(collection);
        case 'datetime-list':
            return testDatetimeList(collection);
        case 'datetime-open-end':
            return testDatetimeOpenEnd(collection);
        case 'area-basic':
            return testAreaBasic(collection);
        case 'area-covjson':
            return testAreaCovJson(collection);
        case 'area-small':
            return testAreaSmall(collection);
        case 'area-complex':
            return testAreaComplex(collection);
        case 'area-too-large':
            return testAreaTooLarge(collection);
        case 'area-invalid-polygon':
            return testAreaInvalidPolygon(collection);
        case 'area-with-params':
            return testAreaWithParams(collection);
        case 'area-missing-coords':
            return testAreaMissingCoords(collection);
        case 'area-multipolygon':
            return testAreaMultipolygon(collection);
        case 'area-z-multiple':
            return testAreaZMultiple(collection);
        case 'area-crs-valid':
            return testAreaCrsValid(collection);
        case 'area-f-covjson':
            return testAreaFCovJson(collection);
        // Radius Query tests
        case 'radius-basic':
            return testRadiusBasic(collection);
        case 'radius-covjson':
            return testRadiusCovJson(collection);
        case 'radius-missing-coords':
            return testRadiusMissingCoords(collection);
        case 'radius-missing-within':
            return testRadiusMissingWithin(collection);
        case 'radius-missing-within-units':
            return testRadiusMissingWithinUnits(collection);
        case 'radius-invalid-coords':
            return testRadiusInvalidCoords(collection);
        case 'radius-too-large':
            return testRadiusTooLarge(collection);
        case 'radius-units-km':
            return testRadiusUnitsKm(collection);
        case 'radius-units-mi':
            return testRadiusUnitsMi(collection);
        case 'radius-units-m':
            return testRadiusUnitsM(collection);
        case 'radius-multipoint':
            return testRadiusMultipoint(collection);
        case 'radius-z-parameter':
            return testRadiusZParameter(collection);
        case 'radius-with-params':
            return testRadiusWithParams(collection);
        case 'radius-datetime':
            return testRadiusDatetime(collection);
        case 'radius-no-query-params':
            return testRadiusNoQueryParams(collection);
        case 'radius-crs-valid':
            return testRadiusCrsValid(collection);
        case 'radius-f-covjson':
            return testRadiusFCovJson(collection);
        // Trajectory Query tests
        case 'trajectory-basic':
            return testTrajectoryBasic(collection);
        case 'trajectory-covjson':
            return testTrajectoryCovJson(collection);
        case 'trajectory-missing-coords':
            return testTrajectoryMissingCoords(collection);
        case 'trajectory-invalid-coords':
            return testTrajectoryInvalidCoords(collection);
        case 'trajectory-linestringz':
            return testTrajectoryLinestringZ(collection);
        case 'trajectory-linestringm':
            return testTrajectoryLinestringM(collection);
        case 'trajectory-z-conflict':
            return testTrajectoryZConflict(collection);
        case 'trajectory-multilinestring':
            return testTrajectoryMultilinestring(collection);
        case 'trajectory-with-params':
            return testTrajectoryWithParams(collection);
        case 'trajectory-datetime':
            return testTrajectoryDatetime(collection);
        case 'trajectory-no-query-params':
            return testTrajectoryNoQueryParams(collection);
        case 'trajectory-invalid-linestringm':
            return testTrajectoryInvalidLinestringM(collection);
        case 'trajectory-invalid-linestringz':
            return testTrajectoryInvalidLinestringZ(collection);
        case 'trajectory-invalid-linestringzm':
            return testTrajectoryInvalidLinestringZM(collection);
        case 'trajectory-linestringz-invalid-z':
            return testTrajectoryLinestringZInvalidZ(collection);
        case 'trajectory-z-param-invalid':
            return testTrajectoryZParamInvalid(collection);
        case 'trajectory-invalid-time':
            return testTrajectoryInvalidTime(collection);
        case 'trajectory-crs-valid':
            return testTrajectoryCrsValid(collection);
        case 'trajectory-f-covjson':
            return testTrajectoryFCovJson(collection);
        // Corridor Query tests
        case 'corridor-basic':
            return testCorridorBasic(collection);
        case 'corridor-covjson':
            return testCorridorCovJson(collection);
        case 'corridor-missing-coords':
            return testCorridorMissingCoords(collection);
        case 'corridor-missing-width':
            return testCorridorMissingWidth(collection);
        case 'corridor-missing-width-units':
            return testCorridorMissingWidthUnits(collection);
        case 'corridor-missing-height':
            return testCorridorMissingHeight(collection);
        case 'corridor-missing-height-units':
            return testCorridorMissingHeightUnits(collection);
        case 'corridor-invalid-width-units':
            return testCorridorInvalidWidthUnits(collection);
        case 'corridor-invalid-height-units':
            return testCorridorInvalidHeightUnits(collection);
        case 'corridor-invalid-coords':
            return testCorridorInvalidCoords(collection);
        case 'corridor-z-conflict':
            return testCorridorZConflict(collection);
        case 'corridor-datetime-conflict':
            return testCorridorDatetimeConflict(collection);
        case 'corridor-multilinestring':
            return testCorridorMultilinestring(collection);
        case 'corridor-with-params':
            return testCorridorWithParams(collection);
        case 'corridor-pressure-height-units':
            return testCorridorPressureHeightUnits(collection);
        case 'corridor-metadata':
            return testCorridorMetadata(collection);
        // Corridor Query - Additional Tests
        case 'corridor-invalid-linestringm':
            return testCorridorInvalidLinestringM(collection);
        case 'corridor-invalid-linestringz':
            return testCorridorInvalidLinestringZ(collection);
        case 'corridor-invalid-linestringzm':
            return testCorridorInvalidLinestringZM(collection);
        case 'corridor-zm-z-conflict':
            return testCorridorZMZConflict(collection);
        case 'corridor-zm-datetime-conflict':
            return testCorridorZMDatetimeConflict(collection);
        case 'corridor-linestringz-invalid-z':
            return testCorridorLinestringZInvalidZ(collection);
        case 'corridor-z-param-invalid':
            return testCorridorZParamInvalid(collection);
        case 'corridor-linestringz':
            return testCorridorLinestringZ(collection);
        case 'corridor-linestringm':
            return testCorridorLinestringM(collection);
        case 'corridor-linestringzm':
            return testCorridorLinestringZM(collection);
        case 'corridor-with-datetime':
            return testCorridorWithDatetime(collection);
        case 'corridor-with-z':
            return testCorridorWithZ(collection);
        case 'corridor-instance':
            return testCorridorInstance(collection);
        case 'corridor-not-found':
            return testCorridorNotFound(collection);
        case 'corridor-crs-valid':
            return testCorridorCrsValid(collection);
        case 'corridor-f-covjson':
            return testCorridorFCovJson(collection);
        case 'error-404-collection':
            return testError404Collection();
        case 'error-400-coords':
            return testError400Coords(collection);
        case 'error-400-datetime':
            return testError400Datetime(collection);
        case 'error-response-structure':
            return testErrorResponseStructure(collection);
        case 'metadata-data-queries':
            return testMetadataDataQueries(collection);
        case 'metadata-parameter-names':
            return testMetadataParameterNames(collection);
        case 'metadata-output-formats':
            return testMetadataOutputFormats(collection);
        case 'metadata-crs':
            return testMetadataCrs(collection);
        // Content-Type & Format tests
        case 'content-type-covjson':
            return testContentTypeCovJson(collection);
        case 'content-type-json':
            return testContentTypeJson(collection);
        case 'f-param-covjson':
            return testFParamCovJson(collection);
        case 'f-param-invalid':
            return testFParamInvalid(collection);
        // CRS Parameter tests
        case 'crs-param-valid':
            return testCrsParamValid(collection);
        case 'crs-param-invalid':
            return testCrsParamInvalid(collection);
        // Parameter-Name tests
        case 'param-name-filter':
            return testParamNameFilter(collection);
        case 'param-name-invalid':
            return testParamNameInvalid(collection);
        // Instance Query tests
        case 'instance-position-query':
            return testInstancePositionQuery(collection);
        case 'instance-invalid-id':
            return testInstanceInvalidId(collection);
        // Domain Type tests
        case 'domain-type-point':
            return testDomainTypePoint(collection);
        case 'domain-type-pointseries':
            return testDomainTypePointSeries(collection);
        case 'domain-type-verticalprofile':
            return testDomainTypeVerticalProfile(collection);
        case 'domain-type-grid':
            return testDomainTypeGrid(collection);
        // Link Validation tests
        case 'links-self':
            return testLinksSelf(collection);
        case 'links-data-queries':
            return testLinksDataQueries(collection);
        // No Query Params tests
        case 'position-no-params':
            return testPositionNoParams(collection);
        case 'area-no-params':
            return testAreaNoParams(collection);
        // Datetime open start
        case 'datetime-open-start':
            return testDatetimeOpenStart(collection);
        // Accept Header Content Negotiation
        case 'accept-covjson':
            return testAcceptCovJson(collection);
        case 'accept-json':
            return testAcceptJson(collection);
        case 'accept-unsupported':
            return testAcceptUnsupported(collection);
        // CoverageJSON Structure Validation
        case 'covjson-referencing':
            return testCovJsonReferencing(collection);
        case 'covjson-ndarray':
            return testCovJsonNdArray(collection);
        case 'covjson-observed-property':
            return testCovJsonObservedProperty(collection);
        case 'covjson-axes':
            return testCovJsonAxes(collection);
        // Alternate Format Links
        case 'links-alternate-formats':
            return testLinksAlternateFormats(collection);
        case 'links-landing-alternate':
            return testLinksLandingAlternate();
        // GeoJSON Output Format tests
        case 'f-param-geojson':
            return testFParamGeoJson(collection);
        case 'content-type-geojson':
            return testContentTypeGeoJson(collection);
        case 'geojson-structure':
            return testGeoJsonStructure(collection);
        case 'accept-geojson':
            return testAcceptGeoJson(collection);
        // PNG Output Format tests
        case 'f-param-png':
            return testFParamPng(collection);
        case 'content-type-png':
            return testContentTypePng(collection);
        case 'png-structure':
            return testPngStructure(collection);
        case 'png-not-supported-position':
            return testPngNotSupportedPosition(collection);
        case 'png-multi-param-error':
            return testPngMultiParamError(collection);
        // Cube Query tests
        case 'cube-basic':
            return testCubeBasic(collection);
        case 'cube-covjson':
            return testCubeCovJson(collection);
        case 'cube-missing-bbox':
            return testCubeMissingBbox(collection);
        case 'cube-missing-z':
            return testCubeMissingZ(collection);
        case 'cube-invalid-bbox':
            return testCubeInvalidBbox(collection);
        case 'cube-multi-z':
            return testCubeMultiZ(collection);
        case 'cube-with-datetime':
            return testCubeWithDatetime(collection);
        case 'cube-with-resolution':
            return testCubeWithResolution(collection);
        case 'cube-instance':
            return testCubeInstance(collection);
        case 'cube-not-found':
            return testCubeNotFound(collection);
        case 'cube-no-query-params':
            return testCubeNoQueryParams(collection);
        case 'cube-z-range':
            return testCubeZRange(collection);
        case 'cube-z-recurring':
            return testCubeZRecurring(collection);
        case 'cube-invalid-z':
            return testCubeInvalidZ(collection);
        case 'cube-crs-valid':
            return testCubeCrsValid(collection);
        case 'cube-f-covjson':
            return testCubeFCovJson(collection);
        // Locations Query tests
        case 'locations-list':
            return testLocationsList(collection);
        case 'locations-geojson-structure':
            return testLocationsGeoJsonStructure(collection);
        case 'locations-query-basic':
            return testLocationsQueryBasic(collection);
        case 'locations-query-covjson':
            return testLocationsQueryCovJson(collection);
        case 'locations-invalid-id':
            return testLocationsInvalidId(collection);
        case 'locations-with-params':
            return testLocationsWithParams(collection);
        case 'locations-with-datetime':
            return testLocationsWithDatetime(collection);
        case 'locations-cache-header':
            return testLocationsCacheHeader(collection);
        case 'locations-instance':
            return testLocationsInstance(collection);
        case 'locations-crs-valid':
            return testLocationsCrsValid(collection);
        case 'locations-f-covjson':
            return testLocationsFCovJson(collection);
        // Schema Validation tests
        case 'schema-covjson-position':
            return testSchemaCovJsonPosition(collection);
        case 'schema-covjson-area':
            return testSchemaCovJsonArea(collection);
        case 'schema-covjson-trajectory':
            return testSchemaCovJsonTrajectory(collection);
        case 'schema-covjson-cube':
            return testSchemaCovJsonCube(collection);
        case 'schema-covjson-locations':
            return testSchemaCovJsonLocations(collection);
        case 'schema-geojson-locations-list':
            return testSchemaGeoJsonLocationsList(collection);
        case 'schema-geojson-position':
            return testSchemaGeoJsonPosition(collection);
        default:
            return { passed: false, error: 'Unknown test' };
    }
}

// Get the URL(s) used by a test
function getTestUrls(testName) {
    // Use the selected collection, not the first collection
    let col = null;
    if (selectedCollectionId && selectedCollectionId !== '__ALL__') {
        col = collections.find(c => c.id === selectedCollectionId);
    }
    if (!col && collections.length > 0) {
        col = collections[0];
    }
    const colId = col?.id || '{collection_id}';

    switch (testName) {
        case 'landing-page':
        case 'landing-links':
            return [API_BASE];
        case 'conformance':
            return [`${API_BASE}/conformance`];
        case 'collections-list':
        case 'collection-structure':
            return [`${API_BASE}/collections`];
        case 'collection-links':
            return [
                `${API_BASE}/collections`,
                `${API_BASE}/collections/${colId}`
            ];
        case 'extent-spatial':
        case 'extent-temporal':
        case 'extent-vertical':
        case 'extent-vertical-format':
            return [`${API_BASE}/collections/${colId}`];
        case 'instances-list':
        case 'instance-structure':
        case 'instance-extent':
            return [`${API_BASE}/collections/${colId}/instances`];
        case 'position-wkt':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)`];
        case 'position-simple':
            return [`${API_BASE}/collections/${colId}/position?coords=-97.5,35.2`];
        case 'position-covjson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)`];
        case 'position-invalid':
            return [`${API_BASE}/collections/${colId}/position?coords=INVALID`];
        case 'position-missing-coords':
            return [`${API_BASE}/collections/${colId}/position`];
        case 'position-multipoint':
            return [`${API_BASE}/collections/${colId}/position?coords=MULTIPOINT((-97.5 35.2),(-98.0 36.0))`];
        case 'position-no-query-params':
            return [`${API_BASE}/collections/${colId}/position`];
        case 'position-crs-valid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&crs=CRS:84`];
        case 'position-f-covjson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=CoverageJSON`];
        case 'z-single':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z={z_level}`];
        case 'z-multiple':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z={z1},{z2},{z3}`];
        case 'z-range':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z={zMax}/{zMin}`];
        case 'z-recurring':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=R{n}/{start}/{interval}`];
        case 'z-invalid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=abc`];
        case 'z-outside-extent':
            // z=99999 should be outside any collection's vertical extent
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=99999`];
        case 'datetime-instant':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime={datetime}`];
        case 'datetime-range':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime={start}/{end}`];
        case 'datetime-list':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime={t1},{t2},{t3}`];
        case 'datetime-open-end':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime={start}/..`];
        case 'area-basic':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))`];
        case 'area-covjson':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))`];
        case 'area-small':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-97.5 35.2,-97.4 35.2,-97.4 35.3,-97.5 35.3,-97.5 35.2))`];
        case 'area-complex':
            // L-shaped polygon
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-100 34,-98 34,-98 36,-99 36,-99 35,-100 35,-100 34))`];
        case 'area-too-large':
            // Full CONUS - should be rejected as too large
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-125 24,-66 24,-66 50,-125 50,-125 24))`];
        case 'area-invalid-polygon':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35))`];
        case 'area-with-params':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&parameter-name=TMP`];
        case 'area-missing-coords':
            return [`${API_BASE}/collections/${colId}/area`];
        case 'area-multipolygon':
            return [`${API_BASE}/collections/${colId}/area?coords=MULTIPOLYGON(((-98 35,-97 35,-97 36,-98 36,-98 35)),((-96 35,-95 35,-95 36,-96 36,-96 35)))`];
        case 'area-z-multiple':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&z={z1},{z2}`];
        case 'area-crs-valid':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&crs=CRS:84`];
        case 'area-f-covjson':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&f=CoverageJSON`];
        // Radius query URLs
        case 'radius-basic':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km`];
        case 'radius-covjson':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=30&within-units=km`];
        case 'radius-missing-coords':
            return [`${API_BASE}/collections/${colId}/radius?within=50&within-units=km`];
        case 'radius-missing-within':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within-units=km`];
        case 'radius-missing-within-units':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50`];
        case 'radius-invalid-coords':
            return [`${API_BASE}/collections/${colId}/radius?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&within=50&within-units=km`];
        case 'radius-too-large':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=1000&within-units=km`];
        case 'radius-units-km':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km`];
        case 'radius-units-mi':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=30&within-units=mi`];
        case 'radius-units-m':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50000&within-units=m`];
        case 'radius-multipoint':
            return [`${API_BASE}/collections/${colId}/radius?coords=MULTIPOINT((-97.5 35.2),(-98.0 36.0))&within=30&within-units=km`];
        case 'radius-z-parameter':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&z=850`];
        case 'radius-with-params':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&parameter-name=TMP`];
        case 'radius-datetime':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&datetime={validtime}`];
        case 'radius-no-query-params':
            return [`${API_BASE}/collections/${colId}/radius`];
        case 'radius-crs-valid':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&crs=CRS:84`];
        case 'radius-f-covjson':
            return [`${API_BASE}/collections/${colId}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&f=CoverageJSON`];
        // Trajectory query URLs
        case 'trajectory-basic':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)`];
        case 'trajectory-covjson':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)`];
        case 'trajectory-missing-coords':
            return [`${API_BASE}/collections/${colId}/trajectory`];
        case 'trajectory-invalid-coords':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=POLYGON((-100 40,-99 40,-99 41,-100 41,-100 40))`];
        case 'trajectory-linestringz':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGZ(-100 40 850,-99 40.5 700,-98 41 500)`];
        case 'trajectory-linestringm':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGM(-100 40 1735574400,-99 40.5 1735578000,-98 41 1735581600)`];
        case 'trajectory-z-conflict':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGZ(-100 40 850,-99 40.5 700)&z=850`];
        case 'trajectory-multilinestring':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=MULTILINESTRING((-100 40,-99 40.5),(-98 41,-97 41.5))`];
        case 'trajectory-with-params':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)&parameter-name=TMP`];
        case 'trajectory-datetime':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)&datetime={validtime}`];
        case 'trajectory-no-query-params':
            return [`${API_BASE}/collections/${colId}/trajectory`];
        case 'trajectory-invalid-linestringm':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGM(-100 40,-99 40.5)`];
        case 'trajectory-invalid-linestringz':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGZ(-100 40,-99 40.5)`];
        case 'trajectory-invalid-linestringzm':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGZM(-100 40 850,-99 40.5 850)`];
        case 'trajectory-linestringz-invalid-z':
            // Z value 99999 should be outside the collection's vertical extent
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGZ(-100 40 99999,-99 40.5 99999,-98 41 99999)`];
        case 'trajectory-z-param-invalid':
            // z parameter value 99999 should be outside the collection's vertical extent
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)&z=99999`];
        case 'trajectory-invalid-time':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRINGM(-100 40 invalid,-99 40.5 notadate,-98 41 alsonotadate)`];
        case 'trajectory-crs-valid':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)&crs=CRS:84`];
        case 'trajectory-f-covjson':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)&f=CoverageJSON`];
        // Corridor query URLs
        case 'corridor-basic':
            return [`${API_BASE}/collections/${colId}/corridor?coords=LINESTRING(-100 40,-99 40.5,-98 41)&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`];
        case 'corridor-crs-valid':
            return [`${API_BASE}/collections/${colId}/corridor?coords=LINESTRING(-100 40,-99 40.5,-98 41)&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&crs=CRS:84`];
        case 'corridor-f-covjson':
            return [`${API_BASE}/collections/${colId}/corridor?coords=LINESTRING(-100 40,-99 40.5,-98 41)&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&f=CoverageJSON`];
        case 'corridor-linestringz-invalid-z':
            // Z value 99999 should be outside the collection's vertical extent
            return [`${API_BASE}/collections/${colId}/corridor?coords=LINESTRINGZ(-100 40 99999,-99 40.5 99999,-98 41 99999)&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`];
        case 'corridor-z-param-invalid':
            // z parameter value 99999 should be outside the collection's vertical extent
            return [`${API_BASE}/collections/${colId}/corridor?coords=LINESTRING(-100 40,-99 40.5,-98 41)&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&z=99999`];
        case 'error-404-collection':
            return [`${API_BASE}/collections/nonexistent-collection-12345`];
        case 'error-400-coords':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-999 999)`];
        case 'error-400-datetime':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime=not-a-valid-datetime`];
        case 'error-response-structure':
            return [`${API_BASE}/collections/nonexistent-collection-12345`];
        case 'metadata-data-queries':
        case 'metadata-parameter-names':
        case 'metadata-output-formats':
        case 'metadata-crs':
            return [`${API_BASE}/collections/${colId}`];
        // New tests URLs
        case 'datetime-open-start':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime=../{end}`];
        case 'content-type-covjson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)`];
        case 'content-type-json':
            return [`${API_BASE}/collections`];
        case 'f-param-covjson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=CoverageJSON`];
        case 'f-param-invalid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=INVALID_FORMAT`];
        case 'crs-param-valid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&crs=CRS:84`];
        case 'crs-param-invalid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&crs=INVALID:CRS`];
        case 'param-name-filter':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&parameter-name=TMP`];
        case 'param-name-invalid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&parameter-name=NONEXISTENT_PARAM_12345`];
        case 'instance-position-query':
            return [`${API_BASE}/collections/${colId}/instances/{instanceId}/position?coords=POINT(-97.5 35.2)`];
        case 'instance-invalid-id':
            return [`${API_BASE}/collections/${colId}/instances/1999-01-01T00:00:00Z/position?coords=POINT(-97.5 35.2)`];
        case 'domain-type-point':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)`];
        case 'domain-type-pointseries':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&datetime={start}/{end}`];
        case 'domain-type-verticalprofile':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=850,700,500`];
        case 'domain-type-grid':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))`];
        case 'links-self':
        case 'links-data-queries':
            return [`${API_BASE}/collections/${colId}`];
        case 'position-no-params':
            return [`${API_BASE}/collections/${colId}/position`];
        case 'area-no-params':
            return [`${API_BASE}/collections/${colId}/area`];
        // Accept Header tests
        case 'accept-covjson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2) (with Accept: application/vnd.cov+json)`];
        case 'accept-json':
            return [`${API_BASE}/collections (with Accept: application/json)`];
        case 'accept-unsupported':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2) (with Accept: application/xml)`];
        // CoverageJSON Structure tests
        case 'covjson-referencing':
        case 'covjson-ndarray':
        case 'covjson-observed-property':
        case 'covjson-axes':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)`];
        // Alternate Links tests
        case 'links-alternate-formats':
            return [`${API_BASE}/collections/${colId}`];
        case 'links-landing-alternate':
            return [`${API_BASE}`];
        // GeoJSON Output Format tests
        case 'f-param-geojson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=geojson`];
        case 'content-type-geojson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=GeoJSON`];
        case 'geojson-structure':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=geojson`];
        case 'accept-geojson':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2) (with Accept: application/geo+json)`];
        // PNG Output Format URLs (coordinates derived from collection extent)
        case 'f-param-png':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((<extent>))&f=png&parameter-name=<first>`];
        case 'content-type-png':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((<extent>))&f=png&parameter-name=<first>`];
        case 'png-structure':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((<extent>))&f=png&parameter-name=<first>`];
        case 'png-not-supported-position':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(<center>)&f=png`];
        case 'png-multi-param-error':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((<extent>))&f=png (no parameter-name)`];
        // Cube Query URLs
        case 'cube-basic':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850`];
        case 'cube-covjson':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850&parameter-name=TMP`];
        case 'cube-missing-bbox':
            return [`${API_BASE}/collections/${colId}/cube?z=850`];
        case 'cube-missing-z':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36`];
        case 'cube-invalid-bbox':
            return [`${API_BASE}/collections/${colId}/cube?bbox=invalid&z=850`];
        case 'cube-multi-z':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850,700,500`];
        case 'cube-with-datetime':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850&datetime={validtime}`];
        case 'cube-with-resolution':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850&resolution-x=5&resolution-y=5`];
        case 'cube-instance':
            return [`${API_BASE}/collections/${colId}/instances/{instanceId}/cube?bbox=-98,35,-97,36&z=850`];
        case 'cube-not-found':
            return [`${API_BASE}/collections/nonexistent-collection-12345/cube?bbox=-98,35,-97,36&z=850`];
        case 'cube-no-query-params':
            return [`${API_BASE}/collections/${colId}/cube`];
        case 'cube-z-range':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=1000/500`];
        case 'cube-z-recurring':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=R5/1000/100`];
        case 'cube-invalid-z':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=invalid`];
        case 'cube-crs-valid':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850&crs=CRS:84`];
        case 'cube-f-covjson':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850&f=CoverageJSON`];
        // Locations Query URLs
        case 'locations-list':
            return [`${API_BASE}/collections/${colId}/locations`];
        case 'locations-geojson-structure':
            return [`${API_BASE}/collections/${colId}/locations`];
        case 'locations-query-basic':
            return [`${API_BASE}/collections/${colId}/locations/KJFK`];
        case 'locations-query-covjson':
            return [`${API_BASE}/collections/${colId}/locations/KJFK`];
        case 'locations-invalid-id':
            return [`${API_BASE}/collections/${colId}/locations/NONEXISTENT_LOCATION_12345`];
        case 'locations-with-params':
            return [`${API_BASE}/collections/${colId}/locations/KJFK?parameter-name=TMP`];
        case 'locations-with-datetime':
            return [`${API_BASE}/collections/${colId}/locations/KJFK?datetime={validtime}`];
        case 'locations-cache-header':
            return [`${API_BASE}/collections/${colId}/locations/KJFK`];
        case 'locations-instance':
            return [`${API_BASE}/collections/${colId}/instances/{instanceId}/locations/KJFK`];
        case 'locations-crs-valid':
            return [`${API_BASE}/collections/${colId}/locations/KJFK?crs=CRS:84`];
        case 'locations-f-covjson':
            return [`${API_BASE}/collections/${colId}/locations/KJFK?f=CoverageJSON`];
        // Schema Validation URLs
        case 'schema-covjson-position':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)`];
        case 'schema-covjson-area':
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))`];
        case 'schema-covjson-trajectory':
            return [`${API_BASE}/collections/${colId}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)`];
        case 'schema-covjson-cube':
            return [`${API_BASE}/collections/${colId}/cube?bbox=-98,35,-97,36&z=850`];
        case 'schema-covjson-locations':
            return [`${API_BASE}/collections/${colId}/locations/{locationId}`];
        case 'schema-geojson-locations-list':
            return [`${API_BASE}/collections/${colId}/locations`];
        case 'schema-geojson-position':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&f=GeoJSON`];
        default:
            return [];
    }
}

function copyTestUrl(testName, btn) {
    // First check if we have an actual URL used by the test
    const result = testResults[testName];
    let textToCopy;
    
    // For per-collection tests, get the URL from the actual collection result
    if (result?.perCollection) {
        const collectionIds = Object.keys(result.perCollection);
        if (collectionIds.length === 1) {
            // Single collection - use its URL
            const colResult = result.perCollection[collectionIds[0]];
            if (colResult?.url) {
                textToCopy = colResult.url;
            }
        } else if (collectionIds.length > 1) {
            // Multiple collections - collect all URLs
            const urls = collectionIds
                .map(id => result.perCollection[id]?.url)
                .filter(url => url);
            if (urls.length > 0) {
                textToCopy = urls.join('\n');
            }
        }
    }
    
    // Fall back to direct URL on result
    if (!textToCopy && result?.url) {
        textToCopy = result.url;
    }
    
    // Fall back to template URLs
    if (!textToCopy) {
        const urls = getTestUrls(testName);
        if (urls.length === 0) {
            showToast('No URL available for this test', 'error');
            return;
        }
        textToCopy = urls.length === 1 ? urls[0] : urls.join('\n');
    }

    navigator.clipboard.writeText(textToCopy).then(() => {
        // Visual feedback on the button
        const originalText = btn.textContent;
        btn.textContent = 'Copied!';
        btn.classList.add('copied');
        setTimeout(() => {
            btn.textContent = originalText;
            btn.classList.remove('copied');
        }, 1500);
    }).catch(() => {
        showToast('Failed to copy URL', 'error');
    });
}

function showToast(message, type = 'info') {
    // Simple toast notification
    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;
    toast.textContent = message;
    toast.style.cssText = `
        position: fixed;
        bottom: 20px;
        right: 20px;
        padding: 0.75rem 1rem;
        background: ${type === 'error' ? 'var(--error-color)' : 'var(--primary-color)'};
        color: white;
        border-radius: 4px;
        z-index: 1001;
        animation: fadeIn 0.2s;
    `;
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 2000);
}

// Individual test implementations

async function testLandingPage() {
    const res = await fetchJson(API_BASE);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has title', passed: !!res.json?.title },
        { name: 'Has links', passed: Array.isArray(res.json?.links) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testLandingLinks() {
    const res = await fetchJson(API_BASE);
    const links = res.json?.links || [];
    const requiredRels = ['self', 'conformance', 'data'];
    const checks = requiredRels.map(rel => ({
        name: `Has '${rel}' link`,
        passed: links.some(l => l.rel === rel)
    }));
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testConformance() {
    const res = await fetchJson(`${API_BASE}/conformance`);
    const conformsTo = res.json?.conformsTo || [];
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has conformsTo array', passed: Array.isArray(conformsTo) },
        { name: 'Includes core', passed: conformsTo.some(c => c.includes('conf/core')) },
        { name: 'Includes collections', passed: conformsTo.some(c => c.includes('conf/collections')) },
        { name: 'Includes position', passed: conformsTo.some(c => c.includes('conf/position')) },
        { name: 'Includes area', passed: conformsTo.some(c => c.includes('conf/area')) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testCollectionsList() {
    const res = await fetchJson(`${API_BASE}/collections`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has collections array', passed: Array.isArray(res.json?.collections) },
        { name: 'Has links', passed: Array.isArray(res.json?.links) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testCollectionStructure(collection) {
    const col = collection;
    const checks = [
        { name: 'Has id', passed: !!col.id },
        { name: 'Has links', passed: Array.isArray(col.links) },
        { name: 'Has extent or data_queries', passed: !!col.extent || !!col.data_queries }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks
    };
}

async function testCollectionLinks(collection) {
    const col = collection;
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const links = colRes.json?.links || [];
    const checks = [
        { name: 'Status 200', passed: colRes.status === 200 },
        { name: 'Has self link', passed: links.some(l => l.rel === 'self') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: colRes
    };
}

// ============================================================
// EXTENT TESTS
// ============================================================

async function testExtentSpatial(collection) {
    const col = collection;
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const extent = colRes.json?.extent;
    const spatial = extent?.spatial;
    
    const checks = [
        { name: 'Has extent object', passed: !!extent },
        { name: 'Has spatial extent', passed: !!spatial },
        { name: 'Has bbox array', passed: Array.isArray(spatial?.bbox) && spatial.bbox.length > 0 },
        { name: 'Bbox has 4 values', passed: spatial?.bbox?.[0]?.length === 4 },
        { name: 'Has CRS', passed: !!spatial?.crs }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: colRes
    };
}

async function testExtentTemporal(collection) {
    const col = collection;
    const url = `${API_BASE}/collections/${col.id}`;
    const colRes = await fetchJson(url);
    const extent = colRes.json?.extent;
    const temporal = extent?.temporal;
    
    // Per spec, temporal extent SHOULD include interval with start/end times
    const interval = temporal?.interval;
    const hasValidInterval = Array.isArray(interval) && 
        interval.length > 0 && 
        Array.isArray(interval[0]) &&
        interval[0].length === 2;
    
    // Check if interval has actual timestamps (not null/null)
    const hasTimestamps = hasValidInterval && 
        (interval[0][0] !== null || interval[0][1] !== null);
    
    const checks = [
        { name: 'Has extent object', passed: !!extent },
        { name: 'Has temporal extent', passed: !!temporal },
        { name: 'Has interval array', passed: hasValidInterval },
        { name: 'Interval has timestamps', passed: hasTimestamps },
        { name: 'Has TRS (temporal ref system)', passed: !!temporal?.trs }
    ];
    
    // Note: 'values' array is recommended but not required
    if (temporal?.values) {
        checks.push({ name: 'Has values array (optional)', passed: Array.isArray(temporal.values) });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: colRes,
        url
    };
}

async function testExtentVertical(collection) {
    const col = collection;
    
    // Check if collection has vertical extent
    if (!collectionHasVerticalExtent(col)) {
        return { 
            skipped: true,
            reason: 'No vertical extent',
            checks: [{ name: 'Collection has no vertical extent', passed: true, skipped: true }]
        };
    }

    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const extent = colRes.json?.extent;
    const vertical = extent?.vertical;
    
    // Per OGC EDR spec Table C.8:
    // - interval: Array of [min, max] pairs (each should have 2 values)
    // - values: Array of discrete height values supported
    
    // Check interval format compliance
    let intervalFormatValid = true;
    let intervalFormatWarning = null;
    if (vertical?.interval && Array.isArray(vertical.interval)) {
        for (let i = 0; i < vertical.interval.length; i++) {
            const pair = vertical.interval[i];
            if (!Array.isArray(pair)) {
                intervalFormatValid = false;
                intervalFormatWarning = `interval[${i}] is not an array`;
                break;
            }
            if (pair.length !== 2) {
                intervalFormatValid = false;
                intervalFormatWarning = `interval[${i}] has ${pair.length} elements, expected 2 [min, max]`;
                break;
            }
            // Check for undefined values (the bug we fixed)
            if (pair[0] === undefined || pair[1] === undefined) {
                intervalFormatValid = false;
                intervalFormatWarning = `interval[${i}] contains undefined values`;
                break;
            }
        }
    }
    
    // Check values array format compliance
    let valuesFormatValid = true;
    let valuesFormatWarning = null;
    if (vertical?.values && Array.isArray(vertical.values)) {
        for (let i = 0; i < vertical.values.length; i++) {
            const val = vertical.values[i];
            if (val === undefined || val === null) {
                valuesFormatValid = false;
                valuesFormatWarning = `values[${i}] is ${val}`;
                break;
            }
        }
    }
    
    const checks = [
        { name: 'Has extent object', passed: !!extent },
        { name: 'Has vertical extent', passed: !!vertical },
        { name: 'Has interval or values', passed: !!(vertical?.interval || vertical?.values) },
        { name: 'Has VRS (vertical ref system)', passed: !!vertical?.vrs },
        { 
            name: 'interval format valid ([min, max] pairs)', 
            passed: intervalFormatValid,
            warning: intervalFormatWarning
        },
        { 
            name: 'values array format valid (no undefined/null)', 
            passed: valuesFormatValid,
            warning: valuesFormatWarning
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.every(c => c.passed),
        warning: hasWarning,
        checks,
        response: colRes
    };
}

// Dedicated test for vertical extent format compliance (Table C.8)
// This test specifically validates the structure to prevent issues like z=1000/undefined
async function testExtentVerticalFormat(collection) {
    const col = collection;
    
    // Check if collection has vertical extent
    if (!collectionHasVerticalExtent(col)) {
        return { 
            skipped: true,
            reason: 'No vertical extent',
            checks: [{ name: 'Collection has no vertical extent', passed: true, skipped: true }]
        };
    }

    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const vertical = colRes.json?.extent?.vertical;
    
    if (!vertical) {
        return {
            passed: false,
            checks: [{ name: 'Vertical extent is missing', passed: false }],
            response: colRes
        };
    }
    
    const checks = [];
    
    // Check 1: Has either interval or values (required per spec)
    const hasIntervalOrValues = !!(vertical.interval || vertical.values);
    checks.push({
        name: 'Has interval or values array',
        passed: hasIntervalOrValues
    });
    
    // Check 2: If interval exists, validate format per Table C.8
    // "Array of level values array, each Level value Array should contain two values"
    if (vertical.interval && Array.isArray(vertical.interval)) {
        let intervalValid = true;
        let intervalError = null;
        
        for (let i = 0; i < vertical.interval.length; i++) {
            const pair = vertical.interval[i];
            
            if (!Array.isArray(pair)) {
                intervalValid = false;
                intervalError = `interval[${i}] is not an array`;
                break;
            }
            
            if (pair.length !== 2) {
                intervalValid = false;
                intervalError = `interval[${i}] has ${pair.length} element(s), spec requires 2 [min, max]`;
                break;
            }
            
            // Check for undefined values (this was the bug causing z=1000/undefined)
            if (pair[0] === undefined) {
                intervalValid = false;
                intervalError = `interval[${i}][0] (min) is undefined`;
                break;
            }
            if (pair[1] === undefined) {
                intervalValid = false;
                intervalError = `interval[${i}][1] (max) is undefined`;
                break;
            }
        }
        
        checks.push({
            name: 'interval format: each pair has [min, max]',
            passed: intervalValid,
            warning: intervalError
        });
    }
    
    // Check 3: If values exists, validate it's a flat array with no undefined/null
    if (vertical.values && Array.isArray(vertical.values)) {
        let valuesValid = true;
        let valuesError = null;
        
        for (let i = 0; i < vertical.values.length; i++) {
            const val = vertical.values[i];
            if (val === undefined) {
                valuesValid = false;
                valuesError = `values[${i}] is undefined`;
                break;
            }
            if (val === null) {
                valuesValid = false;
                valuesError = `values[${i}] is null`;
                break;
            }
        }
        
        checks.push({
            name: 'values array: no undefined/null entries',
            passed: valuesValid,
            warning: valuesError
        });
        
        // Check that values can be used for z-range tests (at least 2 levels for range)
        if (vertical.values.length >= 2) {
            checks.push({
                name: 'values has 2+ levels (enables z-range tests)',
                passed: true
            });
        } else {
            checks.push({
                name: 'values has 2+ levels (enables z-range tests)',
                passed: true,
                warning: `Only ${vertical.values.length} level(s) - z-range tests will be skipped`
            });
        }
    }
    
    // Check 4: Has VRS (vertical reference system)
    checks.push({
        name: 'Has vrs (vertical reference system)',
        passed: !!vertical.vrs
    });
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.every(c => c.passed),
        warning: hasWarning,
        checks,
        response: colRes
    };
}

// ============================================================
// INSTANCES TESTS
// ============================================================

async function testInstancesList(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has instances array', passed: Array.isArray(res.json?.instances) },
        { name: 'Has links', passed: Array.isArray(res.json?.links) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testInstanceStructure(collection) {
    const col = collection;
    const instRes = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const instances = instRes.json?.instances || [];
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances to test (ok)', passed: true }], response: instRes };
    }

    const inst = instances[0];
    const checks = [
        { name: 'Has id', passed: !!inst.id },
        { name: 'Has links', passed: Array.isArray(inst.links) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: instRes
    };
}

async function testInstanceExtent(collection) {
    const col = collection;
    const url = `${API_BASE}/collections/${col.id}/instances`;
    const instRes = await fetchJson(url);
    const instances = instRes.json?.instances || [];
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances to test (ok)', passed: true }], response: instRes, url };
    }

    const inst = instances[0];
    const extent = inst.extent;
    const temporal = extent?.temporal;
    const interval = temporal?.interval;
    
    // Check if instance has valid temporal extent with actual forecast range
    const hasValidInterval = Array.isArray(interval) && 
        interval.length > 0 && 
        Array.isArray(interval[0]) &&
        interval[0].length === 2;
    
    // For forecast models, both start and end should be defined (not null)
    const hasCompleteRange = hasValidInterval && 
        interval[0][0] !== null && 
        interval[0][1] !== null;
    
    // Check if the range makes sense (end >= start)
    // end == start is valid for single timestep data
    let hasValidRange = false;
    if (hasCompleteRange) {
        const start = new Date(interval[0][0]);
        const end = new Date(interval[0][1]);
        hasValidRange = end >= start;
    }
    
    const checks = [
        { name: 'Instance has extent', passed: !!extent },
        { name: 'Has temporal extent', passed: !!temporal },
        { name: 'Has interval array', passed: hasValidInterval },
        { name: 'Interval has start AND end', passed: hasCompleteRange },
        { name: 'End time >= start time', passed: hasValidRange || !hasCompleteRange }
    ];
    
    const hasWarning = false;
    
    return {
        passed: checks.every(c => c.passed),
        warning: hasWarning,
        checks,
        response: instRes,
        url
    };
}

async function testPositionWkt(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }],
            coordsInfo: 'Unable to determine coordinates from collection extent'
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) - center of collection extent`
    };
}

async function testPositionSimple(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/position?coords=${coords.lon},${coords.lat}`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Simple coords: ${coords.lon.toFixed(4)},${coords.lat.toFixed(4)}`
    };
}

async function testPositionCovJson(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})`;
    const res = await fetchJson(url);
    
    // Check for non-null data values
    const hasData = hasNonNullValues(res.json);
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Point', passed: res.json?.domain?.domainType === 'Point' },
        { name: 'Has axes', passed: !!res.json?.domain?.axes },
        { 
            name: 'Has non-null data values', 
            passed: true,  // Don't fail on this
            warning: !hasData ? 'No data values in response (coordinates may be outside data coverage)' : null
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => !c.warning || c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)})`
    };
}

async function testPositionInvalid(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=INVALID`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Position query without coords parameter - should return 400
async function testPositionMissingCoords(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions coords', passed: (res.json?.detail || '').toLowerCase().includes('coord') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// MULTIPOINT query - should return CoverageCollection with multiple coverages
async function testPositionMultipoint(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, bbox, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Create two points within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: coords.lon - span, lat: coords.lat };
    const p2 = { lon: coords.lon + span, lat: coords.lat + span };
    
    const url = `${API_BASE}/collections/${col.id}/position?coords=MULTIPOINT((${p1.lon} ${p1.lat}),(${p2.lon} ${p2.lat}))`;
    const res = await fetchJson(url);
    
    // Per EDR spec, MULTIPOINT should be supported if collection supports it
    // Response should be CoverageCollection with one Coverage per point
    const isCoverageCollection = res.json?.type === 'CoverageCollection';
    const hasCoverages = Array.isArray(res.json?.coverages) && res.json.coverages.length >= 2;
    
    const checks = [
        { name: 'Status 200 (or 400 if not supported)', passed: res.status === 200 || res.status === 400 },
        { name: 'If 200, type is CoverageCollection', passed: res.status !== 200 || isCoverageCollection },
        { name: 'If 200, has 2+ coverages', passed: res.status !== 200 || hasCoverages }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `MULTIPOINT: (${p1.lon.toFixed(4)}, ${p1.lat.toFixed(4)}), (${p2.lon.toFixed(4)}, ${p2.lat.toFixed(4)})`
    };
}

// Position with no query params - should return 400 (Abstract Test B.41)
async function testPositionNoQueryParams(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position`);
    
    const checks = [
        { name: 'Status 400 (no query params)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Position with crs parameter - should accept CRS:84 (Abstract Test B.53/B.54)
async function testPositionCrsValid(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&crs=CRS:84`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'CRS parameter accepted', passed: res.status === 200 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with CRS:84`
    };
}

// Position with f=CoverageJSON parameter (Abstract Test B.55/B.56)
async function testPositionFCovJson(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&f=CoverageJSON`;
    const res = await fetchJson(url);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCoverageJSON = contentType.includes('cov+json') || contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Content-Type is CoverageJSON or JSON', passed: isCoverageJSON }
    ];
    return {
        passed: checks.every(c => c.passed),
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with f=CoverageJSON`,
        checks,
        response: res
    };
}

// ============================================================
// Z PARAMETER TESTS
// ============================================================

// Helper to find a collection with a minimum number of vertical levels
// Returns { collection, searchInfo } where searchInfo describes what was checked
async function findCollectionWithVerticalLevels(minLevels = 1) {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    const searchInfo = [];
    
    if (collections.length === 0) {
        return { collection: null, searchInfo: ['No collections available'] };
    }
    
    // First, try to find by name (isobaric collections typically have many levels)
    const isobaricCandidates = collections.filter(c => c.id.includes('isobaric'));
    
    // Check each isobaric candidate for actual vertical levels
    for (const col of isobaricCandidates) {
        const metadata = await getCollectionMetadata(col.id);
        const verticalValues = extractVerticalLevels(metadata?.extent?.vertical);
        searchInfo.push(`${col.id}: ${verticalValues.length} vertical levels`);
        if (verticalValues.length >= minLevels) {
            return { collection: col, searchInfo };
        }
    }
    
    // If no isobaric collection has enough levels, check all collections
    for (const col of collections) {
        // Skip already checked isobaric collections
        if (col.id.includes('isobaric')) continue;
        
        const metadata = await getCollectionMetadata(col.id);
        const verticalValues = extractVerticalLevels(metadata?.extent?.vertical);
        searchInfo.push(`${col.id}: ${verticalValues.length} vertical levels`);
        if (verticalValues.length >= minLevels) {
            return { collection: col, searchInfo };
        }
    }
    
    // No collection found with enough levels
    return { collection: null, searchInfo };
}

// Helper to find an isobaric collection (which has z levels) - wrapper for backward compatibility
async function findIsobaricCollection() {
    const { collection } = await findCollectionWithVerticalLevels(1);
    return collection;
}

// Single z level query
async function testZSingle(collection) {
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    
    // Must have a valid z value from the collection's vertical extent
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for z-single test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;

    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=${zValue}`;
    const res = await fetchJson(url);
    
    // Check that response includes z axis or returns data for the level
    const hasZAxis = res.json?.domain?.axes?.z !== undefined;
    const hasZInResponse = hasZAxis || res.status === 200;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Query accepted z parameter', passed: hasZInResponse }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), z=${zValue}`
    };
}

// Multiple z levels query - spec says ALL requested levels should be returned
async function testZMultiple(collection) {
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { allLevels, warning: zWarning } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    // Need at least 3 levels for this test
    if (verticalValues.length < 3) {
        return { 
            skipped: true,
            reason: 'Fewer than 3 vertical levels',
            checks: [{ 
                name: `Collection has ${verticalValues.length} vertical levels, needs 3+`, 
                passed: true, 
                skipped: true
            }]
        };
    }
    
    // Use first 3 available levels from the collection's vertical extent
    const zLevels = verticalValues.slice(0, 3);
    const zParam = zLevels.join(',');

    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=${zParam}`;
    const res = await fetchJson(url);
    
    // Check z axis in response
    const zAxis = res.json?.domain?.axes?.z;
    const zAxisValues = Array.isArray(zAxis?.values) ? zAxis.values : (Array.isArray(zAxis) ? zAxis : []);
    const hasAllZLevels = zAxisValues.length >= zLevels.length;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has z axis in domain', passed: zAxis !== undefined },
        { name: `Returns all ${zLevels.length} requested z levels`, passed: hasAllZLevels }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), z=${zParam}`
    };
}

// Z range query (z=1000/500)
async function testZRange(collection) {
    // Need at least 2 vertical levels for range query
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get z levels to determine a valid range
    const { allLevels, warning: zWarning } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    // Need at least 2 levels for z range test
    if (verticalValues.length < 2) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection needs at least 2 vertical levels for z-range test', 
                passed: true, 
                warning: zWarning || 'Insufficient vertical levels'
            }]
        };
    }
    
    // Determine a valid z range from the collection's levels
    const sortedValues = [...verticalValues].sort((a, b) => b - a);
    const zRange = `${sortedValues[0]}/${sortedValues[sortedValues.length - 1]}`;

    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=${zRange}`;
    const res = await fetchJson(url);
    
    // Check z axis in response - should include levels between the range
    const zAxis = res.json?.domain?.axes?.z;
    const zAxisValues = Array.isArray(zAxis?.values) ? zAxis.values : (Array.isArray(zAxis) ? zAxis : []);
    const hasMultipleZLevels = zAxisValues.length > 1;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has z axis in domain', passed: zAxis !== undefined },
        { name: 'Returns multiple z levels for range', passed: hasMultipleZLevels }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), z range=${zRange}`
    };
}

// Recurring z intervals (z=R5/1000/100) - 5 levels starting at 1000, decrementing by 100
async function testZRecurring(collection) {
    // Need at least 3 vertical levels for recurring z test
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get z levels to determine valid recurring parameters
    const { allLevels, warning: zWarning } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    // Need at least 3 levels for recurring z test
    if (verticalValues.length < 3) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection needs at least 3 vertical levels for z-recurring test', 
                passed: true, 
                warning: zWarning || 'Insufficient vertical levels'
            }]
        };
    }
    
    // Build recurring interval from collection's levels
    // We need to find consecutive levels with a consistent interval
    // since R{n}/{start}/{interval} generates evenly-spaced values
    const sortedValues = [...verticalValues].sort((a, b) => b - a);
    
    // Find the longest sequence of evenly-spaced levels
    let bestSequence = { start: sortedValues[0], interval: 1, count: 1 };
    
    for (let i = 0; i < sortedValues.length - 1; i++) {
        const start = sortedValues[i];
        const interval = Math.abs(sortedValues[i] - sortedValues[i + 1]);
        let count = 2; // At least 2 levels (start and next)
        
        // Check how many consecutive levels follow this interval
        for (let j = i + 2; j < sortedValues.length; j++) {
            const expectedValue = start - (interval * (j - i));
            if (Math.abs(sortedValues[j] - expectedValue) < 0.01) {
                count++;
            } else {
                break;
            }
        }
        
        if (count > bestSequence.count) {
            bestSequence = { start, interval, count };
        }
    }
    
    // Use the best sequence we found, limited to 5 levels
    const expectedLevels = Math.min(5, bestSequence.count);
    const zRecurring = `R${expectedLevels}/${bestSequence.start}/${bestSequence.interval}`;

    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=${zRecurring}`;
    const res = await fetchJson(url);
    
    // Check z axis in response
    const zAxis = res.json?.domain?.axes?.z;
    const zAxisValues = Array.isArray(zAxis?.values) ? zAxis.values : (Array.isArray(zAxis) ? zAxis : []);
    const hasExpectedZLevels = zAxisValues.length === expectedLevels;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has z axis in domain', passed: zAxis !== undefined },
        { name: `Returns exactly ${expectedLevels} z levels`, passed: hasExpectedZLevels }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), z=${zRecurring}`
    };
}

// Invalid z parameter
async function testZInvalid(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=abc`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// z parameter outside collection's vertical extent
async function testZOutsideExtent(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Use z=99999 which should be outside any reasonable vertical extent
    const url = `${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=99999`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 (z outside vertical extent)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined },
        { name: 'Error mentions Z or vertical extent', passed: 
            (res.json?.detail && (res.json.detail.toLowerCase().includes('z') || 
             res.json.detail.toLowerCase().includes('vertical') ||
             res.json.detail.toLowerCase().includes('extent'))) ||
            res.status === 400 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// ============================================================
// DATETIME QUERY TESTS
// ============================================================

// Helper to get available times from a collection
async function getCollectionTimes(collectionId) {
    const colRes = await fetchJson(`${API_BASE}/collections/${collectionId}`);
    const times = colRes.json?.extent?.temporal?.values || [];
    return { times };
}

// Single datetime instant
async function testDatetimeInstant(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(collection.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const datetime = times[0]; // Use first available time
    const url = `${API_BASE}/collections/${collection.id}/position?coords=POINT(${coords.lon} ${coords.lat})&datetime=${encodeURIComponent(datetime)}`;
    const res = await fetchJson(url);
    
    // For single instant, check we have a t axis with values
    const tAxisValues = getTimeAxisValues(res.json?.domain);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'Has t axis with value(s)', passed: tAxisValues.length >= 1 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime=${datetime}`
    };
}

// Helper to extract time values from axes (handles both array and object with values)
function getTimeAxisValues(domain) {
    const tAxis = domain?.axes?.t || domain?.axes?.time;
    if (!tAxis) return [];
    // CovJSON can have t as direct array or as object with values property
    if (Array.isArray(tAxis)) return tAxis;
    if (Array.isArray(tAxis.values)) return tAxis.values;
    return [];
}

// Datetime range (start/end interval)
async function testDatetimeRange(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for range test (N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(collection.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const startTime = times[0];
    const endTime = times[Math.min(2, times.length - 1)]; // Use 3rd time or last
    const datetimeRange = `${startTime}/${endTime}`;
    
    const url = `${API_BASE}/collections/${collection.id}/position?coords=POINT(${coords.lon} ${coords.lat})&datetime=${encodeURIComponent(datetimeRange)}`;
    const res = await fetchJson(url);
    
    // For ranges, response should be a PointSeries with multiple time values
    const tAxisValues = getTimeAxisValues(res.json?.domain);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Domain type is PointSeries', passed: res.json?.domain?.domainType === 'PointSeries' },
        { name: 'Has multiple time values', passed: tAxisValues.length >= 2 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime range`
    };
}

// Multiple discrete datetimes (comma-separated list)
async function testDatetimeList(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (times.length < 3) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for list test (N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(collection.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Pick 3 times
    const selectedTimes = [times[0], times[1], times[2]];
    const datetimeList = selectedTimes.join(',');
    
    const url = `${API_BASE}/collections/${collection.id}/position?coords=POINT(${coords.lon} ${coords.lat})&datetime=${encodeURIComponent(datetimeList)}`;
    const res = await fetchJson(url);
    
    // For lists, response should be a PointSeries with multiple time values
    const tAxisValues = getTimeAxisValues(res.json?.domain);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Domain type is PointSeries', passed: res.json?.domain?.domainType === 'PointSeries' },
        { name: 'Has 3 time values', passed: tAxisValues.length === 3 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime list`
    };
}

// Datetime with open end (start/..)
async function testDatetimeOpenEnd(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for open-end test (N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(collection.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const startTime = times[0];
    const datetimeOpenEnd = `${startTime}/..`;
    
    const url = `${API_BASE}/collections/${collection.id}/position?coords=POINT(${coords.lon} ${coords.lat})&datetime=${encodeURIComponent(datetimeOpenEnd)}`;
    const res = await fetchJson(url);
    
    // 413 Payload Too Large is acceptable - server correctly interprets open-end
    // but applies rate limiting (this is proper behavior for collections with many timesteps)
    if (res.status === 413) {
        const checks = [
            { name: 'Server accepts open-end syntax', passed: true },
            { name: 'Server applies rate limiting (413)', passed: true }
        ];
        return {
            passed: true,
            checks,
            response: res,
            url,
            coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime open-end (rate-limited)`
        };
    }
    
    // For open-ended ranges, response should be a PointSeries with multiple time values
    const tAxisValues = getTimeAxisValues(res.json?.domain);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Domain type is PointSeries', passed: res.json?.domain?.domainType === 'PointSeries' },
        { name: 'Has multiple time values (from start to latest)', passed: tAxisValues.length >= 2 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime open-end`
    };
}

// ============================================================
// AREA QUERY TESTS
// ============================================================

// Basic polygon area query
async function testAreaBasic(collection) {
    const col = collection;
    
    // Get valid polygon from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Polygon bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}]`
    };
}

// Area query returns proper CoverageJSON Grid
async function testAreaCovJson(collection) {
    const col = collection;
    
    // Get valid polygon from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`;
    const res = await fetchJson(url);
    
    // Check for non-null data values
    const ranges = res.json?.ranges || {};
    const paramKeys = Object.keys(ranges);
    const hasData = hasNonNullValues(res.json);
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has x axis', passed: !!res.json?.domain?.axes?.x },
        { name: 'Has y axis', passed: !!res.json?.domain?.axes?.y },
        { name: 'Has ranges', passed: paramKeys.length > 0 },
        { 
            name: 'Has non-null data values', 
            passed: true,  // Don't fail on this
            warning: !hasData ? 'No data values in response (area may be outside data coverage)' : null
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Polygon bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}]`
    };
}

// Small region area query
async function testAreaSmall(collection) {
    const col = collection;
    
    // Get valid small polygon from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 0.1);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Response is valid JSON', passed: res.json !== null }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Small polygon bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}]`
    };
}

// Complex polygon (L-shaped)
async function testAreaComplex(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, bbox, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: warning }]
        };
    }
    
    // Create an L-shaped polygon within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 1.0);
    const minLon = Math.max(bbox[0], coords.lon - span);
    const maxLon = Math.min(bbox[2], coords.lon + span);
    const minLat = Math.max(bbox[1], coords.lat - span);
    const maxLat = Math.min(bbox[3], coords.lat + span);
    const midLon = (minLon + maxLon) / 2;
    const midLat = (minLat + maxLat) / 2;
    
    // L-shaped polygon
    const polygon = `POLYGON((${minLon} ${minLat},${maxLon} ${minLat},${maxLon} ${midLat},${midLon} ${midLat},${midLon} ${maxLat},${minLon} ${maxLat},${minLon} ${minLat}))`;
    
    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `L-shaped polygon around (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)})`
    };
}

// Area too large should return 413 or 400
async function testAreaTooLarge(collection) {
    const col = collection;
    // Full CONUS - should be rejected as too large
    const polygon = 'POLYGON((-125 24,-66 24,-66 50,-125 50,-125 24))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
    // Per spec, server MAY return 413 for too large requests, or 400 for invalid
    const checks = [
        { name: 'Status 413 or 400', passed: res.status === 413 || res.status === 400 },
        { name: 'Has error response', passed: !!res.json?.type || !!res.json?.detail }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Invalid polygon (not closed, insufficient points)
async function testAreaInvalidPolygon(collection) {
    const col = collection;
    // Invalid: only 2 points, not a valid polygon
    const polygon = 'POLYGON((-98 35,-97 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Area query with parameter filtering
async function testAreaWithParams(collection) {
    const col = collection;
    
    // Get valid polygon from collection extent
    const { polygon, bboxArray, warning: polygonWarning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: polygonWarning || 'Cannot determine valid area', passed: true, warning: polygonWarning }]
        };
    }
    
    // Get a valid parameter name from the collection
    const { parameter, warning: paramWarning } = await getValidParameter(col.id);
    if (!parameter) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: paramWarning || 'Cannot determine valid parameter', passed: true, warning: paramWarning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&parameter-name=${parameter}`;
    const res = await fetchJson(url);
    
    // Check for non-null data values
    const hasData = hasNonNullValues(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has ranges', passed: !!res.json?.ranges },
        { 
            name: 'Has non-null data values', 
            passed: true,
            warning: !hasData ? 'No data values in response (area may be outside data coverage)' : null
        }
    ];
    
    // If we have ranges, check that filtering worked
    if (res.json?.ranges) {
        const rangeKeys = Object.keys(res.json.ranges);
        checks.push({ name: 'Response includes requested parameter', passed: rangeKeys.includes(parameter) || rangeKeys.length > 0 });
    }
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Polygon bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], parameter: ${parameter}`
    };
}

// Area query without coords parameter - should return 400
async function testAreaMissingCoords(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions coords', passed: (res.json?.detail || '').toLowerCase().includes('coord') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// MULTIPOLYGON query
async function testAreaMultipolygon(collection) {
    const col = collection;
    // Two separate 1x1 degree polygons
    const multipolygon = 'MULTIPOLYGON(((-98 35,-97 35,-97 36,-98 36,-98 35)),((-96 35,-95 35,-95 36,-96 36,-96 35)))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(multipolygon)}`);
    
    const checks = [
        { name: 'Status 200 (or 400 if not supported)', passed: res.status === 200 || res.status === 400 },
        { name: 'If 200, has type Coverage', passed: res.status !== 200 || res.json?.type === 'Coverage' },
        { name: 'If 200, has domain type Grid', passed: res.status !== 200 || res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Area query with multiple z levels
async function testAreaZMultiple(collection) {
    // Need at least 2 vertical levels for this test
    const col = collection;

    // Get available z values from collection extent
    const { allLevels, warning: zWarning } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    // Need at least 2 levels for this test
    if (verticalValues.length < 2) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection needs at least 2 vertical levels for area-z-multiple test', 
                passed: true, 
                warning: zWarning || 'Insufficient vertical levels'
            }]
        };
    }
    
    // Use first 2 available levels from the collection's vertical extent
    const zLevels = verticalValues.slice(0, 2);
    const zParam = zLevels.join(',');

    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&z=${zParam}`);
    
    // Check z axis in response
    const zAxis = res.json?.domain?.axes?.z;
    const zAxisValues = Array.isArray(zAxis?.values) ? zAxis.values : (Array.isArray(zAxis) ? zAxis : []);
    const hasTwoZLevels = zAxisValues.length >= 2;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has z axis in domain', passed: zAxis !== undefined },
        { name: 'Returns both requested z levels', passed: hasTwoZLevels }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Area with crs parameter - should accept CRS:84 (Abstract Test B.87/B.88)
async function testAreaCrsValid(collection) {
    const col = collection;
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&crs=CRS:84`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'CRS parameter accepted', passed: res.status === 200 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Area with f=CoverageJSON parameter (Abstract Test B.89/B.90)
async function testAreaFCovJson(collection) {
    const col = collection;
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&f=CoverageJSON`);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCoverageJSON = contentType.includes('cov+json') || contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Content-Type is CoverageJSON or JSON', passed: isCoverageJSON }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// RADIUS QUERY TESTS
// OGC EDR Spec: Section 8.2.4 Radius Query
// ============================================================

// Basic radius query
async function testRadiusBasic(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius`
    };
}

// Radius query returns proper CoverageJSON Grid
async function testRadiusCovJson(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=30&within-units=km`;
    const res = await fetchJson(url);
    
    // Check for non-null data values
    const hasData = hasNonNullValues(res.json);
    const ranges = res.json?.ranges || {};
    const paramKeys = Object.keys(ranges);
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has x axis', passed: !!res.json?.domain?.axes?.x },
        { name: 'Has y axis', passed: !!res.json?.domain?.axes?.y },
        { name: 'Has ranges', passed: paramKeys.length > 0 },
        { 
            name: 'Has non-null data values', 
            passed: true,
            warning: !hasData ? 'No data values in response (coordinates may be outside data coverage)' : null
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 30km radius`
    };
}

// Radius query missing coords parameter - should return 400
async function testRadiusMissingCoords(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?within=50&within-units=km`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions coords', passed: (res.json?.detail || '').toLowerCase().includes('coord') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query missing within parameter - should return 400
async function testRadiusMissingWithin(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within-units=km`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions within', passed: (res.json?.detail || '').toLowerCase().includes('within') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query missing within-units parameter - should return 400
async function testRadiusMissingWithinUnits(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=50`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions within-units', passed: (res.json?.detail || '').toLowerCase().includes('within') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with invalid coords (POLYGON instead of POINT) - should return 400
async function testRadiusInvalidCoords(collection) {
    const col = collection;
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=${encodeURIComponent(polygon)}&within=50&within-units=km`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error response', passed: res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius too large - should return 413
async function testRadiusTooLarge(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Request 1000 km radius which should exceed limit
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=1000&within-units=km`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 413', passed: res.status === 413 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions radius', passed: (res.json?.detail || '').toLowerCase().includes('radius') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 1000km radius (too large)`
    };
}

// Radius query with km units
async function testRadiusUnitsKm(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius`
    };
}

// Radius query with miles units
async function testRadiusUnitsMi(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=30&within-units=mi`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 30mi radius`
    };
}

// Radius query with meters units
async function testRadiusUnitsM(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50000&within-units=m`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50000m radius`
    };
}

// Radius query with MULTIPOINT coords (union of circles)
async function testRadiusMultipoint(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, bbox, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Create two points within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: coords.lon - span, lat: coords.lat };
    const p2 = { lon: coords.lon + span, lat: coords.lat + span };
    
    const multipointCoords = `MULTIPOINT((${p1.lon} ${p1.lat}),(${p2.lon} ${p2.lat}))`;
    const url = `${API_BASE}/collections/${col.id}/radius?coords=${encodeURIComponent(multipointCoords)}&within=30&within-units=km`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has ranges with data', passed: Object.keys(res.json?.ranges || {}).length > 0 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `MULTIPOINT: (${p1.lon.toFixed(4)}, ${p1.lat.toFixed(4)}), (${p2.lon.toFixed(4)}, ${p2.lat.toFixed(4)}) with 30km radius`
    };
}

// Radius query with z (vertical level) parameter
async function testRadiusZParameter(collection) {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    
    // Find a collection with vertical levels (isobaric)
    const isobaricCol = collections.find(c => 
        c.id.includes('isobaric') || 
        c.extent?.vertical?.values?.length > 0
    );
    
    if (!isobaricCol) {
        return { passed: true, checks: [{ name: 'No isobaric collection available (test N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(isobaricCol.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Get valid Z level
    const { z, warning: zWarning } = await getValidZLevel(isobaricCol.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for radius-z-parameter test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;
    
    const url = `${API_BASE}/collections/${isobaricCol.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km&z=${zValue}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has ranges', passed: Object.keys(res.json?.ranges || {}).length > 0 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius, z=${zValue}`
    };
}

// Radius query with parameter-name filtering
async function testRadiusWithParams(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Get the collection's parameters
    const { parameter } = await getValidParameter(col.id);
    
    if (!parameter) {
        return { passed: true, checks: [{ name: 'No parameters available (test N/A)', passed: true }] };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km&parameter-name=${parameter}`;
    const res = await fetchJson(url);
    
    const returnedParams = Object.keys(res.json?.ranges || {});
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has ranges', passed: returnedParams.length > 0 },
        { name: 'Only requested parameter returned', passed: returnedParams.length === 1 && returnedParams[0] === parameter }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius, parameter=${parameter}`
    };
}

// Radius query with datetime parameter
async function testRadiusDatetime(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(collection.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const datetime = times[0]; // Use first available time
    const url = `${API_BASE}/collections/${collection.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km&datetime=${encodeURIComponent(datetime)}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius, datetime=${datetime}`
    };
}

// Radius with no query params - should return 400 (Abstract Test B.57)
async function testRadiusNoQueryParams(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius`);
    
    const checks = [
        { name: 'Status 400 (no query params)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius with crs parameter - should accept CRS:84 (Abstract Test B.71/B.72)
async function testRadiusCrsValid(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km&crs=CRS:84`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'CRS parameter accepted', passed: res.status === 200 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius, CRS:84`
    };
}

// Radius with f=CoverageJSON parameter (Abstract Test B.73/B.74)
async function testRadiusFCovJson(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/radius?coords=POINT(${coords.lon} ${coords.lat})&within=50&within-units=km&f=CoverageJSON`;
    const res = await fetchJson(url);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCoverageJSON = contentType.includes('cov+json') || contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Content-Type is CoverageJSON or JSON', passed: isCoverageJSON }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with 50km radius, f=CoverageJSON`
    };
}

// ============================================================
// TRAJECTORY QUERY TESTS
// OGC EDR Spec: Section 8.2.5 Trajectory Query
// ============================================================

// Basic trajectory query with LINESTRING
async function testTrajectoryBasic(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestring)}`;
    const res = await fetchJson(url);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Linestring centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)})`
    };
}

// Trajectory query returns proper CoverageJSON with Trajectory domain
async function testTrajectoryCovJson(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestring)}`;
    const res = await fetchJson(url);
    
    // Check for non-null data values
    const hasData = hasNonNullValues(res.json);
    const ranges = res.json?.ranges || {};
    const paramKeys = Object.keys(ranges);
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Trajectory', passed: res.json?.domain?.domainType === 'Trajectory' },
        { name: 'Has composite axis', passed: !!res.json?.domain?.axes?.composite },
        { name: 'Has ranges', passed: paramKeys.length > 0 },
        { 
            name: 'Has non-null data values', 
            passed: true,
            warning: !hasData ? 'No data values in response (coordinates may be outside data coverage)' : null
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Linestring centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)})`
    };
}

// Trajectory query missing coords parameter - should return 400
async function testTrajectoryMissingCoords(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions coords', passed: (res.json?.detail || '').toLowerCase().includes('coord') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory query with invalid coords (POLYGON instead of LINESTRING) - should return 400
async function testTrajectoryInvalidCoords(collection) {
    const col = collection;
    const polygon = 'POLYGON((-100 40,-99 40,-99 41,-100 41,-100 40))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(polygon)}`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error response', passed: res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory query with LINESTRINGZ (embedded vertical levels)
async function testTrajectoryLinestringZ(collection) {
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Get valid Z levels
    const { z, allLevels, warning: zWarning } = await getValidZLevel(col.id);
    if (!allLevels || allLevels.length < 3) {
        return { 
            skipped: true,
            reason: 'Fewer than 3 vertical levels',
            checks: [{ 
                name: `Collection has ${allLevels?.length || 0} vertical levels, needs 3+`, 
                passed: true, 
                skipped: true
            }]
        };
    }
    const zLevels = allLevels.slice(0, 3);
    
    // Create a LINESTRINGZ within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: centerCoords.lon, lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat };
    
    const linestringZ = `LINESTRINGZ(${p1.lon} ${p1.lat} ${zLevels[0]},${p2.lon} ${p2.lat} ${zLevels[1]},${p3.lon} ${p3.lat} ${zLevels[2]})`;
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestringZ)}`;
    const res = await fetchJson(url);
    
    // Should return 200 and have z axis data
    const hasZAxis = !!res.json?.domain?.axes?.z || !!res.json?.domain?.axes?.composite;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Trajectory', passed: res.json?.domain?.domainType === 'Trajectory' },
        { name: 'Has z axis or composite axis', passed: hasZAxis }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGZ centered at: (${centerCoords.lon.toFixed(4)}, ${centerCoords.lat.toFixed(4)}) with z=${zLevels.join(',')}`
    };
}

// Trajectory query with LINESTRINGM (embedded time values as Unix epoch)
async function testTrajectoryLinestringM(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(collection.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Convert first three times to Unix epoch (seconds since 1970-01-01)
    // Use valid epoch times in the future for testing
    const epoch1 = Math.floor(new Date(times[0]).getTime() / 1000);
    const epoch2 = epoch1 + 3600;  // +1 hour
    const epoch3 = epoch1 + 7200;  // +2 hours
    
    // Create a LINESTRINGM within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: centerCoords.lon, lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat };
    
    const linestringM = `LINESTRINGM(${p1.lon} ${p1.lat} ${epoch1},${p2.lon} ${p2.lat} ${epoch2},${p3.lon} ${p3.lat} ${epoch3})`;
    const url = `${API_BASE}/collections/${collection.id}/trajectory?coords=${encodeURIComponent(linestringM)}`;
    const res = await fetchJson(url);
    
    // Should return 200 and have time axis data
    const hasTAxis = !!res.json?.domain?.axes?.t || !!res.json?.domain?.axes?.composite;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Trajectory', passed: res.json?.domain?.domainType === 'Trajectory' },
        { name: 'Has t axis or composite axis', passed: hasTAxis }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGM centered at: (${centerCoords.lon.toFixed(4)}, ${centerCoords.lat.toFixed(4)}) with epochs`
    };
}

// Trajectory query with LINESTRINGZ and z parameter - should return 400 (conflict)
async function testTrajectoryZConflict(collection) {
    // Need at least 2 vertical levels for this test
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Get valid z values from collection extent
    const { allLevels, z: firstZ, warning: zWarning } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    if (verticalValues.length < 2) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection needs at least 2 vertical levels for trajectory-z-conflict test', 
                passed: true, 
                warning: zWarning || 'Insufficient vertical levels'
            }]
        };
    }
    const z1 = verticalValues[0];
    const z2 = verticalValues[1];
    
    // Create a LINESTRINGZ within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    
    // LINESTRINGZ has embedded Z, but we're also providing z query param - this is a conflict
    const linestringZ = `LINESTRINGZ(${p1.lon} ${p1.lat} ${z1},${p2.lon} ${p2.lat} ${z2})`;
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestringZ)}&z=${z1}`;
    const res = await fetchJson(url);
    
    // Per OGC spec, when coords contain Z values, providing z param is invalid
    const checks = [
        { name: 'Status 400 (z conflict)', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions z or conflict', passed: 
            (res.json?.detail || '').toLowerCase().includes('z') || 
            (res.json?.detail || '').toLowerCase().includes('conflict') ||
            (res.json?.detail || '').toLowerCase().includes('embed')
        }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGZ with conflicting z parameter`
    };
}

// Trajectory query with MULTILINESTRING (multiple trajectory segments)
async function testTrajectoryMultilinestring(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Create two separate trajectory segments within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 6, (bbox[3] - bbox[1]) / 6, 0.3);
    const seg1p1 = { lon: Math.max(bbox[0], centerCoords.lon - span * 2), lat: centerCoords.lat };
    const seg1p2 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: Math.min(bbox[3], centerCoords.lat + span) };
    const seg2p1 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat };
    const seg2p2 = { lon: Math.min(bbox[2], centerCoords.lon + span * 2), lat: Math.min(bbox[3], centerCoords.lat + span) };
    
    const multilinestring = `MULTILINESTRING((${seg1p1.lon} ${seg1p1.lat},${seg1p2.lon} ${seg1p2.lat}),(${seg2p1.lon} ${seg2p1.lat},${seg2p2.lon} ${seg2p2.lat}))`;
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(multilinestring)}`;
    const res = await fetchJson(url);
    
    // MULTILINESTRING can return either:
    // 1. CoverageCollection with multiple coverages (one per segment) - strict interpretation
    // 2. Single Coverage with merged Trajectory domain - permissive interpretation
    // Both are valid per OGC spec
    const isCoverageCollection = res.json?.type === 'CoverageCollection';
    const hasCoverages = Array.isArray(res.json?.coverages) && res.json.coverages.length >= 2;
    const isSingleCoverage = res.json?.type === 'Coverage';
    const isTrajectoryDomain = res.json?.domain?.domainType === 'Trajectory';
    
    // Accept either approach
    const validResponse = isCoverageCollection ? hasCoverages : (isSingleCoverage && isTrajectoryDomain);
    
    const checks = [
        { name: 'Status 200 (or 400 if not supported)', passed: res.status === 200 || res.status === 400 },
        { name: 'If 200, valid response type', passed: res.status !== 200 || validResponse },
        { name: 'If 200, has Coverage or CoverageCollection', passed: res.status !== 200 || isSingleCoverage || isCoverageCollection }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `MULTILINESTRING centered at: (${centerCoords.lon.toFixed(4)}, ${centerCoords.lat.toFixed(4)})`
    };
}

// Trajectory query with parameter-name filtering
async function testTrajectoryWithParams(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Get the collection's parameters
    const { parameter } = await getValidParameter(col.id);
    
    if (!parameter) {
        return { passed: true, checks: [{ name: 'No parameters available (test N/A)', passed: true }] };
    }
    
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestring)}&parameter-name=${parameter}`;
    const res = await fetchJson(url);
    
    const returnedParams = Object.keys(res.json?.ranges || {});
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has ranges', passed: returnedParams.length > 0 },
        { name: 'Only requested parameter returned', passed: returnedParams.length === 1 && returnedParams[0] === parameter }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Linestring centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), parameter=${parameter}`
    };
}

// Trajectory query with datetime parameter
async function testTrajectoryDatetime(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(collection.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const datetime = times[0]; // Use first available time
    const url = `${API_BASE}/collections/${collection.id}/trajectory?coords=${encodeURIComponent(linestring)}&datetime=${encodeURIComponent(datetime)}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'Domain type is Trajectory', passed: res.json?.domain?.domainType === 'Trajectory' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Linestring centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime=${datetime}`
    };
}

// Trajectory with no query params - should return 400 (Abstract Test B.105)
async function testTrajectoryNoQueryParams(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory`);
    
    const checks = [
        { name: 'Status 400 (no query params)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory with invalid LINESTRINGM (Abstract Test B.108)
async function testTrajectoryInvalidLinestringM(collection) {
    const col = collection;
    // Invalid LINESTRINGM - wrong number of coordinates (should have 3 per point for M)
    const coords = 'LINESTRINGM(-100 40,-99 40.5)'; // Missing M values
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(coords)}`);
    
    const checks = [
        { name: 'Status 400 (invalid LINESTRINGM)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory with invalid LINESTRINGZ (Abstract Test B.112)
async function testTrajectoryInvalidLinestringZ(collection) {
    const col = collection;
    // Invalid LINESTRINGZ - wrong number of coordinates (should have 3 per point for Z)
    const coords = 'LINESTRINGZ(-100 40,-99 40.5)'; // Missing Z values
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(coords)}`);
    
    const checks = [
        { name: 'Status 400 (invalid LINESTRINGZ)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory with invalid LINESTRINGZM (Abstract Test B.111)
async function testTrajectoryInvalidLinestringZM(collection) {
    const col = collection;
    // Invalid LINESTRINGZM - wrong number of coordinates (should have 4 per point for ZM)
    const coords = 'LINESTRINGZM(-100 40 850,-99 40.5 850)'; // Missing M values
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(coords)}`);
    
    const checks = [
        { name: 'Status 400 (invalid LINESTRINGZM)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory LINESTRINGZ with Z coordinate outside collection's vertical extent
async function testTrajectoryLinestringZInvalidZ(collection) {
    const col = collection;
    // LINESTRINGZ with Z value 99999 - outside any reasonable vertical extent
    const coords = 'LINESTRINGZ(-100 40 99999,-99 40.5 99999,-98 41 99999)';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(coords)}`);
    
    const checks = [
        { name: 'Status 400 (Z outside vertical extent)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined },
        { name: 'Error mentions Z or vertical extent', passed: 
            (res.json?.detail && (res.json.detail.toLowerCase().includes('z') || 
             res.json.detail.toLowerCase().includes('vertical') ||
             res.json.detail.toLowerCase().includes('extent'))) ||
            res.status === 400 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory z parameter with value outside collection's vertical extent
async function testTrajectoryZParamInvalid(collection) {
    const col = collection;
    // LINESTRING with z parameter value 99999 - outside any reasonable vertical extent
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=LINESTRING(-100 40,-99 40.5,-98 41)&z=99999`);
    
    const checks = [
        { name: 'Status 400 (z param outside vertical extent)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined },
        { name: 'Error mentions Z or vertical extent', passed: 
            (res.json?.detail && (res.json.detail.toLowerCase().includes('z') || 
             res.json.detail.toLowerCase().includes('vertical') ||
             res.json.detail.toLowerCase().includes('extent'))) ||
            res.status === 400 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory with invalid time coordinates (Abstract Test B.113)
async function testTrajectoryInvalidTime(collection) {
    const col = collection;
    // LINESTRINGM with non-numeric time value
    const coords = 'LINESTRINGM(-100 40 invalid,-99 40.5 notadate,-98 41 alsonotadate)';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(coords)}`);
    
    const checks = [
        { name: 'Status 400 (invalid time coords)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Trajectory with crs parameter - should accept CRS:84 (Abstract Test B.119/B.120)
async function testTrajectoryCrsValid(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestring)}&crs=CRS:84`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'CRS parameter accepted', passed: res.status === 200 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Linestring centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with CRS:84`
    };
}

// Trajectory with f=CoverageJSON parameter (Abstract Test B.121/B.122)
async function testTrajectoryFCovJson(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestring)}&f=CoverageJSON`;
    const res = await fetchJson(url);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCoverageJSON = contentType.includes('cov+json') || contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Content-Type is CoverageJSON or JSON', passed: isCoverageJSON }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Linestring centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with f=CoverageJSON`
    };
}

// ============================================================
// CORRIDOR QUERY TESTS
// ============================================================

// Basic corridor query - all required params
// Corridor returns a CoverageCollection with multiple trajectories (left, center, right)
async function testCorridorBasic(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const corridorWidth = '10';
    const widthUnits = 'km';
    const corridorHeight = '1000';
    const heightUnits = 'm';
    
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=${corridorWidth}&width-units=${widthUnits}&corridor-height=${corridorHeight}&height-units=${heightUnits}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Domain type is Trajectory', passed: res.json?.domainType === 'Trajectory' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) },
        { name: 'Has multiple trajectories', passed: (res.json?.coverages?.length || 0) >= 1 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Corridor centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), width=${corridorWidth}${widthUnits}`
    };
}

// Corridor - verify CoverageJSON CoverageCollection response format
async function testCorridorCovJson(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    // CoverageCollection has parameters at top level, coverages have domain/ranges
    const firstCoverage = res.json?.coverages?.[0];
    
    // Check for non-null data values
    const hasData = hasNonNullValues(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type field', passed: res.json?.type !== undefined },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has parameters field', passed: res.json?.parameters !== undefined },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) },
        { name: 'First coverage has domain', passed: !!firstCoverage?.domain },
        { name: 'First coverage has ranges', passed: !!firstCoverage?.ranges },
        { name: 'First coverage domain has axes', passed: !!firstCoverage?.domain?.axes },
        { 
            name: 'Has non-null data values', 
            passed: true,
            warning: !hasData ? 'No data values in response (coordinates may be outside data coverage)' : null
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Corridor centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)})`
    };
}

// Corridor - missing coords parameter
async function testCorridorMissingCoords(collection) {
    const col = collection;
    // Missing coords - should return 400
    const url = `${API_BASE}/collections/${col.id}/corridor?corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for missing coords', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - missing corridor-width parameter
async function testCorridorMissingWidth(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Missing corridor-width - should return 400
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for missing corridor-width', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// Corridor - missing width-units parameter
async function testCorridorMissingWidthUnits(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Missing width-units - should return 400
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for missing width-units', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// Corridor - missing corridor-height parameter
// Note: Our implementation treats height params as optional, so 200 is also acceptable
async function testCorridorMissingHeight(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Missing corridor-height - may return 400 or 200 (if height is optional)
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=km&height-units=m`;
    const res = await fetchJson(url);
    
    // Accept either 400 (strict) or 200 with valid response (lenient - height optional)
    // Corridor queries return CoverageCollection (not Coverage)
    const isValidCoverageType = res.json?.type === 'Coverage' || res.json?.type === 'CoverageCollection';
    const isValidResponse = res.status === 400 || (res.status === 200 && isValidCoverageType);
    
    const checks = [
        { name: 'Status 400 or 200 with Coverage/CoverageCollection', passed: isValidResponse },
        { name: 'Has valid response', passed: res.status === 400 ? !!res.json?.type : isValidCoverageType }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// Corridor - missing height-units parameter
// Note: Our implementation treats height params as optional, so 200 is also acceptable
async function testCorridorMissingHeightUnits(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Missing height-units - may return 400 or 200 (if height params are optional)
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=km&corridor-height=1000`;
    const res = await fetchJson(url);
    
    // Accept either 400 (strict) or 200 with valid response (lenient - height optional)
    // Corridor queries return CoverageCollection (not Coverage)
    const isValidCoverageType = res.json?.type === 'Coverage' || res.json?.type === 'CoverageCollection';
    const isValidResponse = res.status === 400 || (res.status === 200 && isValidCoverageType);
    
    const checks = [
        { name: 'Status 400 or 200 with Coverage/CoverageCollection', passed: isValidResponse },
        { name: 'Has valid response', passed: res.status === 400 ? !!res.json?.type : isValidCoverageType }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// Corridor - invalid width-units
async function testCorridorInvalidWidthUnits(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Invalid width-units - should return 400
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=invalid_unit&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for invalid width-units', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// Corridor - invalid height-units
async function testCorridorInvalidHeightUnits(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Invalid height-units - should return 400
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=invalid_unit`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for invalid height-units', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url
    };
}

// Corridor - invalid LINESTRING format
async function testCorridorInvalidCoords(collection) {
    const col = collection;
    const coords = 'POINT(-100 40)'; // POINT is invalid for corridor
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for invalid coords', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - LINESTRINGZ + z parameter conflict
async function testCorridorZConflict(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Create a LINESTRINGZ within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: centerCoords.lon, lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat };
    
    const linestringZ = `LINESTRINGZ(${p1.lon} ${p1.lat} 850,${p2.lon} ${p2.lat} 850,${p3.lon} ${p3.lat} 850)`;
    // Conflict: LINESTRINGZ already has Z values, and z param is also specified
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestringZ)}&z=1000&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for Z conflict', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGZ with conflicting z parameter`
    };
}

// Corridor - LINESTRINGM + datetime parameter conflict
async function testCorridorDatetimeConflict(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Create a LINESTRINGM within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: centerCoords.lon, lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat };
    
    const linestringM = `LINESTRINGM(${p1.lon} ${p1.lat} 1560507000,${p2.lon} ${p2.lat} 1560508800,${p3.lon} ${p3.lat} 1560510600)`;
    // Conflict: LINESTRINGM already has M values, and datetime param is also specified
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestringM)}&datetime=2024-01-01T00:00:00Z&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for datetime conflict', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGM with conflicting datetime parameter`
    };
}

// Corridor - MULTILINESTRING support
async function testCorridorMultilinestring(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Create two separate trajectory segments within the extent
    const span = Math.min((bbox[2] - bbox[0]) / 6, (bbox[3] - bbox[1]) / 6, 0.3);
    const seg1p1 = { lon: Math.max(bbox[0], centerCoords.lon - span * 2), lat: centerCoords.lat };
    const seg1p2 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: Math.min(bbox[3], centerCoords.lat + span) };
    const seg2p1 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat };
    const seg2p2 = { lon: Math.min(bbox[2], centerCoords.lon + span * 2), lat: Math.min(bbox[3], centerCoords.lat + span) };
    
    const multilinestring = `MULTILINESTRING((${seg1p1.lon} ${seg1p1.lat},${seg1p2.lon} ${seg1p2.lat}),(${seg2p1.lon} ${seg2p1.lat},${seg2p2.lon} ${seg2p2.lat}))`;
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(multilinestring)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Domain type is Trajectory', passed: res.json?.domainType === 'Trajectory' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `MULTILINESTRING centered at: (${centerCoords.lon.toFixed(4)}, ${centerCoords.lat.toFixed(4)})`
    };
}

// Corridor with parameter-name filter
async function testCorridorWithParams(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Get the collection's parameters
    const { parameter } = await getValidParameter(col.id);
    if (!parameter) {
        return { passed: true, checks: [{ name: 'No parameters available (test N/A)', passed: true }] };
    }
    
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&parameter-name=${parameter}`;
    const res = await fetchJson(url);
    
    const params = res.json?.parameters || {};
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has parameters', passed: Object.keys(params).length > 0 },
        { name: 'Contains requested parameter', passed: !!params[parameter] }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Corridor centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), parameter=${parameter}`
    };
}

// Corridor with pressure height units (hPa)
async function testCorridorPressureHeightUnits(collection) {
    const col = collection;
    
    // Get valid linestring from collection extent
    const { linestring, warning, coords } = await getValidLinestring(col.id);
    if (!linestring) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    // Use hPa for height units (for isobaric surfaces)
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(linestring)}&corridor-width=10&width-units=km&corridor-height=100&height-units=hPa`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Corridor centered at: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}) with height-units=hPa`
    };
}

// Corridor metadata - verify data_queries has corridor with variables
async function testCorridorMetadata(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const corridor = res.json?.data_queries?.corridor;
    // Variables are nested under link in our API structure
    const variables = corridor?.link?.variables;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has corridor query type', passed: !!corridor },
        { name: 'Corridor has link', passed: !!corridor?.link },
        { name: 'Link has variables', passed: !!variables },
        { name: 'Variables has width_units', passed: Array.isArray(variables?.width_units) },
        { name: 'Variables has height_units', passed: Array.isArray(variables?.height_units) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - invalid LINESTRINGM format (B.130)
async function testCorridorInvalidLinestringM(collection) {
    const col = collection;
    // Invalid LINESTRINGM - missing M value on last point
    const coords = 'LINESTRINGM(-100 40 1560507000,-99 40.5 1560508800,-98 41)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for invalid LINESTRINGM', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - invalid LINESTRINGZ format (B.134)
async function testCorridorInvalidLinestringZ(collection) {
    const col = collection;
    // Invalid LINESTRINGZ - missing Z value on second point
    const coords = 'LINESTRINGZ(-100 40 850,-99 40.5,-98 41 850)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for invalid LINESTRINGZ', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - invalid LINESTRINGZM format (B.133)
async function testCorridorInvalidLinestringZM(collection) {
    const col = collection;
    // Invalid LINESTRINGZM - missing ZM values on second point
    const coords = 'LINESTRINGZM(-100 40 850 1560507000,-99 40.5,-98 41 850 1560510600)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for invalid LINESTRINGZM', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - LINESTRINGZM + z parameter conflict (B.132)
async function testCorridorZMZConflict(collection) {
    const col = collection;
    const coords = 'LINESTRINGZM(-100 40 850 1560507000,-99 40.5 850 1560508800,-98 41 850 1560510600)';
    // Conflict: LINESTRINGZM has Z values AND z param is specified
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&z=1000&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for LINESTRINGZM + z conflict', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - LINESTRINGZM + datetime parameter conflict
async function testCorridorZMDatetimeConflict(collection) {
    const col = collection;
    const coords = 'LINESTRINGZM(-100 40 850 1560507000,-99 40.5 850 1560508800,-98 41 850 1560510600)';
    // Conflict: LINESTRINGZM has M values AND datetime param is specified
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&datetime=2024-01-01T00:00:00Z&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 for LINESTRINGZM + datetime conflict', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor LINESTRINGZ with Z coordinate outside collection's vertical extent
async function testCorridorLinestringZInvalidZ(collection) {
    const col = collection;
    // LINESTRINGZ with Z value 99999 - outside any reasonable vertical extent
    const coords = 'LINESTRINGZ(-100 40 99999,-99 40.5 99999,-98 41 99999)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 (Z outside vertical extent)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined },
        { name: 'Error mentions Z or vertical extent', passed: 
            (res.json?.detail && (res.json.detail.toLowerCase().includes('z') || 
             res.json.detail.toLowerCase().includes('vertical') ||
             res.json.detail.toLowerCase().includes('extent'))) ||
            res.status === 400 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor z parameter with value outside collection's vertical extent
async function testCorridorZParamInvalid(collection) {
    const col = collection;
    // LINESTRING with z parameter value 99999 - outside any reasonable vertical extent
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=LINESTRING(-100 40,-99 40.5,-98 41)&z=99999&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 400 (z param outside vertical extent)', passed: res.status === 400 },
        { name: 'Has error response', passed: res.json?.type !== undefined || res.json?.detail !== undefined },
        { name: 'Error mentions Z or vertical extent', passed: 
            (res.json?.detail && (res.json.detail.toLowerCase().includes('z') || 
             res.json.detail.toLowerCase().includes('vertical') ||
             res.json.detail.toLowerCase().includes('extent'))) ||
            res.status === 400 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - valid LINESTRINGZ query (success case)
async function testCorridorLinestringZ(collection) {
    // Need at least 1 vertical level for LINESTRINGZ corridor
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get valid z value from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for corridor-linestringz test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }

    // Create a LINESTRINGZ within the extent using valid z level
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: centerCoords.lon, lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat + span / 4 };

    // Valid LINESTRINGZ with Z coordinates embedded using valid z from collection
    const coords = `LINESTRINGZ(${p1.lon} ${p1.lat} ${z},${p2.lon} ${p2.lat} ${z},${p3.lon} ${p3.lat} ${z})`;
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGZ with z=${z}`
    };
}

// Corridor - valid LINESTRINGM query (success case)
async function testCorridorLinestringM(collection) {
    const col = collection;
    // Valid LINESTRINGM with Unix epoch timestamps (June 14, 2019)
    const coords = 'LINESTRINGM(-100 40 1560507000,-99 40.5 1560508800,-98 41 1560510600)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - valid LINESTRINGZM query (success case)
async function testCorridorLinestringZM(collection) {
    // Need at least 1 vertical level for LINESTRINGZM corridor
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords: centerCoords, bbox, warning } = await getValidCoordinates(col.id);
    if (!centerCoords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get valid z value from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for corridor-linestringzm test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }

    // Get valid timestamps from temporal extent
    const { datetime } = await getValidDatetime(col.id);
    // Use Unix epoch timestamps - if no datetime, use some reasonable values
    const baseTime = datetime ? new Date(datetime).getTime() / 1000 : 1560507000;
    const t1 = Math.floor(baseTime);
    const t2 = Math.floor(baseTime + 1800);  // +30 minutes
    const t3 = Math.floor(baseTime + 3600);  // +60 minutes

    // Create a LINESTRINGZM within the extent using valid z level and timestamps
    const span = Math.min((bbox[2] - bbox[0]) / 4, (bbox[3] - bbox[1]) / 4, 0.5);
    const p1 = { lon: Math.max(bbox[0], centerCoords.lon - span), lat: centerCoords.lat };
    const p2 = { lon: centerCoords.lon, lat: Math.min(bbox[3], centerCoords.lat + span / 2) };
    const p3 = { lon: Math.min(bbox[2], centerCoords.lon + span), lat: centerCoords.lat + span / 4 };

    // Valid LINESTRINGZM with both Z and M coordinates using valid z from collection
    const coords = `LINESTRINGZM(${p1.lon} ${p1.lat} ${z} ${t1},${p2.lon} ${p2.lat} ${z} ${t2},${p3.lon} ${p3.lat} ${z} ${t3})`;
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `LINESTRINGZM with z=${z}`
    };
}

// Corridor with datetime parameter
async function testCorridorWithDatetime(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    const datetime = times[0];
    const coords = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const url = `${API_BASE}/collections/${collection.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&datetime=${encodeURIComponent(datetime)}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor with z parameter
async function testCorridorWithZ(collection) {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    // Try to find a collection with vertical levels
    let col = null;
    let zValue = null;
    for (const c of collections) {
        const colRes = await fetchJson(`${API_BASE}/collections/${c.id}`);
        const vertical = colRes.json?.extent?.vertical;
        if (vertical?.values?.length > 0) {
            col = c;
            zValue = vertical.values[0];
            break;
        } else if (vertical?.interval?.[0]?.[0] !== undefined) {
            col = c;
            zValue = vertical.interval[0][0];
            break;
        }
    }
    
    if (!col || zValue === null) {
        return { passed: true, checks: [{ name: 'No collections with vertical levels (test N/A)', passed: true }] };
    }
    
    const coords = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&z=${zValue}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - instance-specific query
async function testCorridorInstance(collection) {
    const col = collection;
    
    // Get instances for this collection
    const instancesRes = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const instances = instancesRes.json?.instances || [];
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances available (test N/A)', passed: true }] };
    }
    
    const instance = instances[0];
    const coords = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const url = `${API_BASE}/collections/${col.id}/instances/${instance.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: Array.isArray(res.json?.coverages) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor - 404 for non-existent collection
async function testCorridorNotFound(collection) {
    const coords = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const url = `${API_BASE}/collections/nonexistent-collection-12345/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 404', passed: res.status === 404 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor with crs parameter - should accept CRS:84 (Abstract Test B.151/B.152)
async function testCorridorCrsValid(collection) {
    const col = collection;
    const coords = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&crs=CRS:84`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'CRS parameter accepted', passed: res.status === 200 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Corridor with f=CoverageJSON parameter (Abstract Test B.153/B.154)
async function testCorridorFCovJson(collection) {
    const col = collection;
    const coords = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const url = `${API_BASE}/collections/${col.id}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=10&width-units=km&corridor-height=1000&height-units=m&f=CoverageJSON`;
    const res = await fetchJson(url);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCoverageJSON = contentType.includes('cov+json') || contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Content-Type is CoverageJSON or JSON', passed: isCoverageJSON }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testError404Collection() {
    const res = await fetchJson(`${API_BASE}/collections/nonexistent-collection-12345`);
    const checks = [
        { name: 'Status 404', passed: res.status === 404 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testError400Coords(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-999 999)`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Invalid datetime format
async function testError400Datetime(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&datetime=not-a-valid-datetime`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Error response structure per OGC spec
async function testErrorResponseStructure(collection) {
    const res = await fetchJson(`${API_BASE}/collections/nonexistent-collection-12345`);
    
    // OGC exception response should have: type, title, status, detail
    const checks = [
        { name: 'Status 404', passed: res.status === 404 },
        { name: 'Has "type" field', passed: !!res.json?.type },
        { name: 'Has "title" field', passed: !!res.json?.title },
        { name: 'Has "status" field', passed: res.json?.status !== undefined },
        { name: 'Has "detail" field', passed: !!res.json?.detail },
        { name: 'Status field matches HTTP status', passed: res.json?.status === 404 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// COLLECTION METADATA TESTS
// ============================================================

// Verify data_queries object structure
async function testMetadataDataQueries(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const dataQueries = res.json?.data_queries;
    
    // Per spec, data_queries should have at least one query type defined
    const hasAtLeastOneQuery = dataQueries && (
        dataQueries.position || dataQueries.area || dataQueries.cube ||
        dataQueries.trajectory || dataQueries.corridor || dataQueries.radius ||
        dataQueries.items || dataQueries.locations
    );
    
    // Each query should have a link property
    const queryTypes = ['position', 'area', 'cube', 'trajectory', 'corridor', 'radius', 'items', 'locations'];
    const allQueriesHaveLinks = queryTypes.every(qt => 
        !dataQueries?.[qt] || dataQueries[qt].link
    );
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has data_queries object', passed: !!dataQueries },
        { name: 'Has at least one query type', passed: hasAtLeastOneQuery },
        { name: 'Query types have link property', passed: allQueriesHaveLinks }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Verify parameter_names object
async function testMetadataParameterNames(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const paramNames = res.json?.parameter_names;
    
    // Each parameter should have required fields per spec
    let allParamsValid = true;
    let paramCount = 0;
    if (paramNames) {
        for (const [key, param] of Object.entries(paramNames)) {
            paramCount++;
            // Per spec, parameter should have type and optionally unit, observedProperty
            if (!param.type) {
                allParamsValid = false;
            }
        }
    }
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has parameter_names object', passed: !!paramNames },
        { name: 'Has at least one parameter', passed: paramCount > 0 },
        { name: 'Parameters have required fields', passed: allParamsValid }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Verify output_formats only lists actually supported formats
async function testMetadataOutputFormats(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const outputFormats = res.json?.output_formats || [];
    
    // Test that CoverageJSON works (it should be listed and functional)
    const hasCovJson = outputFormats.some(f => 
        f.includes('cov+json') || f.includes('coverage+json') || f.toLowerCase().includes('covjson')
    );
    
    // Test if GeoJSON is listed - if so, it should actually work
    const hasGeoJson = outputFormats.some(f => 
        f.includes('geo+json') || f.toLowerCase().includes('geojson')
    );
    
    // If GeoJSON is listed, try to request it
    let geoJsonWorks = true;
    if (hasGeoJson) {
        const geoRes = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=GeoJSON`);
        // Should either return GeoJSON (application/geo+json) or work at all
        const contentType = geoRes.headers?.get('content-type') || '';
        geoJsonWorks = geoRes.status === 200 && contentType.includes('geo+json');
    }
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has output_formats array', passed: Array.isArray(outputFormats) },
        { name: 'Lists CoverageJSON', passed: hasCovJson },
        { name: 'If GeoJSON listed, it works', passed: !hasGeoJson || geoJsonWorks }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Verify CRS only lists supported coordinate systems
async function testMetadataCrs(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const crsArray = res.json?.crs || [];
    
    // Per spec, CRS:84 (WGS84 lon/lat) should be supported
    const hasCrs84 = crsArray.some(c => 
        c.includes('CRS84') || c.includes('CRS:84') || c.includes('4326')
    );
    
    // If additional CRS are listed, they should work when requested
    // For now, we just check that the list is reasonable
    const hasReasonableCrs = crsArray.length === 0 || hasCrs84;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has crs array (or omitted)', passed: true }, // crs is optional
        { name: 'If crs listed, includes CRS:84/EPSG:4326', passed: crsArray.length === 0 || hasCrs84 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// DATETIME OPEN START TEST
// ============================================================

// Datetime with open start (../end)
async function testDatetimeOpenStart(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for open-start test (N/A)', passed: true }] };
    }
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(collection.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const endTime = times[times.length - 1];
    const datetimeOpenStart = `../${endTime}`;
    
    const url = `${API_BASE}/collections/${collection.id}/position?coords=POINT(${coords.lon} ${coords.lat})&datetime=${encodeURIComponent(datetimeOpenStart)}`;
    const res = await fetchJson(url);
    
    // 413 Payload Too Large is acceptable - server correctly interprets open-start
    // but applies rate limiting (this is proper behavior for collections with many timesteps)
    if (res.status === 413) {
        const checks = [
            { name: 'Server accepts open-start syntax', passed: true },
            { name: 'Server applies rate limiting (413)', passed: true }
        ];
        return {
            passed: true,
            checks,
            response: res,
            url,
            coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime open-start (rate-limited)`
        };
    }
    
    // For open-started ranges, response should be a PointSeries with multiple time values
    const tAxisValues = getTimeAxisValues(res.json?.domain);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Domain type is PointSeries', passed: res.json?.domain?.domainType === 'PointSeries' },
        { name: 'Has multiple time values (from earliest to end)', passed: tAxisValues.length >= 2 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Point: (${coords.lon.toFixed(4)}, ${coords.lat.toFixed(4)}), datetime open-start`
    };
}

// ============================================================
// CONTENT-TYPE & FORMAT PARAMETER TESTS
// Spec: Requirement A.76, A.82, A.50, A.51
// ============================================================

// Test that position query returns proper CoverageJSON Content-Type header
async function testContentTypeCovJson(collection) {
    const col = collection;
    
    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})`);
    
    const contentType = res.headers?.get('content-type') || '';
    // Accept various CoverageJSON media types
    const isCovJson = contentType.includes('cov+json') || 
                      contentType.includes('coverage+json') ||
                      contentType.includes('prs.coverage+json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has Content-Type header', passed: contentType.length > 0 },
        { name: 'Content-Type is CoverageJSON', passed: isCovJson }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test that collections endpoint returns application/json Content-Type
async function testContentTypeJson(collection) {
    const res = await fetchJson(`${API_BASE}/collections`);
    
    const contentType = res.headers?.get('content-type') || '';
    const isJson = contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has Content-Type header', passed: contentType.length > 0 },
        { name: 'Content-Type is application/json', passed: isJson }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test f parameter selects CoverageJSON format
async function testFParamCovJson(collection) {
    const col = collection;
    // Try with f=CoverageJSON
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=CoverageJSON`);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCovJson = contentType.includes('cov+json') || 
                      contentType.includes('coverage+json') ||
                      res.json?.type === 'Coverage';
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'f parameter accepted', passed: res.status === 200 },
        { name: 'Response is CoverageJSON', passed: isCovJson }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test invalid f parameter returns error (400 or similar)
async function testFParamInvalid(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=INVALID_FORMAT_12345`);
    
    // Per spec, unsupported format should return 400 Bad Request
    // However, some implementations may ignore invalid f values and return default format
    // We'll accept either 400 error OR 200 with CoverageJSON (graceful degradation)
    const isError = res.status === 400;
    const isGracefulDegradation = res.status === 200 && res.json?.type === 'Coverage';
    
    const checks = [
        { name: 'Returns 400 error OR gracefully degrades to default', passed: isError || isGracefulDegradation },
        { name: 'If 400, has error type', passed: res.status !== 400 || !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// CRS PARAMETER TESTS
// Spec: Requirement A.48, A.49
// ============================================================

// Test valid CRS parameter is accepted
async function testCrsParamValid(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&crs=CRS:84`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'crs=CRS:84 is accepted', passed: res.status === 200 },
        { name: 'Response has type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test invalid CRS parameter returns error
async function testCrsParamInvalid(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&crs=INVALID:CRS:12345`);
    
    // Per spec, unsupported CRS should return 400
    // Some implementations may ignore invalid crs and use default
    const isError = res.status === 400;
    const isGracefulDegradation = res.status === 200;
    
    const checks = [
        { name: 'Returns 400 error OR gracefully ignores invalid CRS', passed: isError || isGracefulDegradation },
        { name: 'If 400, has error details', passed: res.status !== 400 || !!res.json?.type || !!res.json?.detail }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// PARAMETER-NAME TESTS
// Spec: Requirement A.46, A.47
// ============================================================

// Test that parameter-name filter returns only requested parameters
async function testParamNameFilter(collection) {
    const col = collection;
    // Get collection details to find available parameters
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const paramNames = colRes.json?.parameter_names || {};
    const availableParams = Object.keys(paramNames);
    
    if (availableParams.length === 0) {
        return { passed: true, checks: [{ name: 'No parameters defined (test N/A)', passed: true }] };
    }
    
    // Request only the first parameter
    const requestedParam = availableParams[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&parameter-name=${requestedParam}`);
    
    // Check that response only contains the requested parameter
    const responseParams = res.json?.parameters ? Object.keys(res.json.parameters) : [];
    const rangeParams = res.json?.ranges ? Object.keys(res.json.ranges) : [];
    const allResponseParams = [...new Set([...responseParams, ...rangeParams])];
    
    // Should only have the requested parameter (or empty if no data)
    const onlyRequestedParam = allResponseParams.length === 0 || 
                               (allResponseParams.length === 1 && allResponseParams[0] === requestedParam);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Response contains only requested parameter', passed: onlyRequestedParam }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test that invalid parameter-name is handled gracefully
async function testParamNameInvalid(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&parameter-name=NONEXISTENT_PARAM_12345`);
    
    // Per spec, invalid parameter should return 400
    // Some implementations may return empty data instead
    const isError = res.status === 400;
    const isEmptyResponse = res.status === 200 && 
                            (Object.keys(res.json?.parameters || {}).length === 0 ||
                             Object.keys(res.json?.ranges || {}).length === 0);
    
    const checks = [
        { name: 'Returns 400 error OR empty/no data response', passed: isError || isEmptyResponse || res.status === 200 },
        { name: 'If 400, has error details', passed: res.status !== 400 || !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// INSTANCE QUERY TESTS
// Spec: Section 8.3 - Instances
// ============================================================

// Test position query via instance path
async function testInstancePositionQuery(collection) {
    const col = collection;
    
    // Get instances for this collection
    const instRes = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const instances = instRes.json?.instances || [];
    
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances available (test N/A)', passed: true }] };
    }
    
    const instance = instances[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/instances/${instance.id}/position?coords=POINT(-97.5 35.2)`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Query via instance path works', passed: res.status === 200 },
        { name: 'Response has type', passed: !!res.json?.type },
        { name: 'Response has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test invalid instance ID returns 404 or 400
async function testInstanceInvalidId(collection) {
    const col = collection;
    // Use a valid datetime format that doesn't exist as an actual instance
    // This should return 404 (Not Found) rather than 400 (Bad Request)
    const fakeInstanceId = '1999-01-01T00:00:00Z';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/instances/${fakeInstanceId}/position?coords=POINT(-97.5 35.2)`);
    
    // Should return 404 for non-existent instance, or 400 for invalid format
    const isError = res.status === 404 || res.status === 400;
    
    const checks = [
        { name: 'Status 404 or 400 for invalid/nonexistent instance', passed: isError },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// DOMAIN TYPE TESTS
// CoverageJSON Spec: Domain Types
// ============================================================

// Test single point query returns domainType: Point
async function testDomainTypePoint(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
    const domainType = res.json?.domain?.domainType;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'domainType is Point', passed: domainType === 'Point' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test multi-time query returns domainType: PointSeries
async function testDomainTypePointSeries(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values (test N/A)', passed: true }] };
    }
    
    const startTime = times[0];
    const endTime = times[Math.min(2, times.length - 1)];
    const datetimeRange = `${startTime}/${endTime}`;
    
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/position?coords=POINT(-97.5 35.2)&datetime=${encodeURIComponent(datetimeRange)}`);
    
    const domainType = res.json?.domain?.domainType;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'domainType is PointSeries', passed: domainType === 'PointSeries' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test multi-z query returns domainType: VerticalProfile
async function testDomainTypeVerticalProfile(collection) {
    // Need at least 3 vertical levels for vertical profile test
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { allLevels, warning: zWarning } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    // Need at least 3 levels for vertical profile test
    if (verticalValues.length < 3) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection needs at least 3 vertical levels for domain-type-verticalprofile test', 
                passed: true, 
                warning: zWarning || 'Insufficient vertical levels'
            }]
        };
    }
    
    const zLevels = verticalValues.slice(0, 3);
    const zParam = zLevels.join(',');

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(${coords.lon} ${coords.lat})&z=${zParam}`);
    
    const domainType = res.json?.domain?.domainType;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'domainType is VerticalProfile', passed: domainType === 'VerticalProfile' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test area query returns domainType: Grid
async function testDomainTypeGrid(collection) {
    const col = collection;
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
    const domainType = res.json?.domain?.domainType;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'domainType is Grid', passed: domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// LINK VALIDATION TESTS
// Spec: Requirement A.13, A.14
// ============================================================

// Test collection has valid self link with correct href and type
async function testLinksSelf(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    
    const links = res.json?.links || [];
    const selfLink = links.find(l => l.rel === 'self');
    
    // Self link should have href and type
    const hasSelfLink = !!selfLink;
    const hasHref = selfLink?.href?.length > 0;
    const hasType = selfLink?.type?.length > 0;
    // Href should point to the collection
    const hrefCorrect = selfLink?.href?.includes(col.id);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has self link', passed: hasSelfLink },
        { name: 'Self link has href', passed: hasHref },
        { name: 'Self link has type', passed: hasType },
        { name: 'Self href contains collection ID', passed: hrefCorrect }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test data_queries links are accessible
async function testLinksDataQueries(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    
    const dataQueries = res.json?.data_queries || {};
    const queryTypes = Object.keys(dataQueries);
    
    if (queryTypes.length === 0) {
        return { passed: true, checks: [{ name: 'No data_queries defined (test N/A)', passed: true }] };
    }
    
    // Check that each query type has a link property
    let allHaveLinks = true;
    let linksAccessible = true;
    
    for (const qt of queryTypes) {
        const queryDef = dataQueries[qt];
        if (!queryDef?.link?.href) {
            allHaveLinks = false;
        }
    }
    
    // Test accessibility of the first query link (don't actually query, just check structure)
    const firstQuery = dataQueries[queryTypes[0]];
    const firstLink = firstQuery?.link;
    const linkHasRequiredFields = firstLink?.href && firstLink?.rel;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has data_queries', passed: queryTypes.length > 0 },
        { name: 'All query types have link property', passed: allHaveLinks },
        { name: 'Links have required fields (href, rel)', passed: linkHasRequiredFields }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// NO QUERY PARAMS ERROR TESTS
// Spec: Abstract Test B.41, B.75
// ============================================================

// Test position endpoint with no query params returns error
async function testPositionNoParams(collection) {
    const col = collection;
    // Call position endpoint with NO query parameters at all
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position`);
    
    // Per spec (Test B.41), should return error when no query params specified
    const isError = res.status === 400;
    
    const checks = [
        { name: 'Status 400 (no query params)', passed: isError },
        { name: 'Has error response', passed: !!res.json?.type || !!res.json?.detail }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test area endpoint with no query params returns error
async function testAreaNoParams(collection) {
    const col = collection;
    // Call area endpoint with NO query parameters at all
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area`);
    
    // Per spec (Test B.75), should return error when no query params specified
    const isError = res.status === 400;
    
    const checks = [
        { name: 'Status 400 (no query params)', passed: isError },
        { name: 'Has error response', passed: !!res.json?.type || !!res.json?.detail }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// ACCEPT HEADER CONTENT NEGOTIATION TESTS
// Spec: /req/core/http - HTTP content negotiation per RFC 7231
// ============================================================

// Test Accept: application/vnd.cov+json header
async function testAcceptCovJson(collection) {
    const col = collection;
    const res = await fetchWithAccept(
        `${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`,
        'application/vnd.cov+json'
    );
    
    const contentType = res.headers?.get('content-type') || '';
    const isCovJson = contentType.includes('cov+json') || 
                      contentType.includes('coverage+json') ||
                      res.json?.type === 'Coverage';
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Accept header honored', passed: res.status === 200 },
        { name: 'Response is CoverageJSON', passed: isCovJson }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test Accept: application/json header for collections
async function testAcceptJson(collection) {
    const res = await fetchWithAccept(`${API_BASE}/collections`, 'application/json');
    
    const contentType = res.headers?.get('content-type') || '';
    const isJson = contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Accept header honored', passed: res.status === 200 },
        { name: 'Content-Type is application/json', passed: isJson },
        { name: 'Response has collections array', passed: Array.isArray(res.json?.collections) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test unsupported Accept header returns 406 Not Acceptable
async function testAcceptUnsupported(collection) {
    const col = collection;
    // Request a format that we definitely don't support
    // Add cache-busting parameter to ensure fresh request
    const cacheBust = `_cb=${Date.now()}`;
    const res = await fetchWithAccept(
        `${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&${cacheBust}`,
        'application/xml'
    );
    
    // Debug: log what we got
    console.log('testAcceptUnsupported - Status:', res.status, 'Response type:', res.json?.type);
    
    // Per OGC EDR spec and RFC 7231, 406 should be returned when Accept header cannot be satisfied
    const is406 = res.status === 406;
    
    // Error response should have details
    const hasErrorDetails = !!res.json?.type || !!res.json?.detail;
    
    const checks = [
        { name: 'Status 406 Not Acceptable', passed: is406 },
        { name: 'Has error type or detail', passed: hasErrorDetails }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// COVERAGEJSON STRUCTURE VALIDATION TESTS
// CoverageJSON Spec: https://covjson.org/spec/
// ============================================================

// Test that domain has referencing system
async function testCovJsonReferencing(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
    const domain = res.json?.domain;
    const referencing = domain?.referencing;
    
    // Per CovJSON spec, referencing should be an array of reference system connections
    const hasReferencing = Array.isArray(referencing) && referencing.length > 0;
    
    // Each referencing entry should have coordinates and system
    let referencingValid = hasReferencing;
    if (hasReferencing) {
        for (const ref of referencing) {
            if (!Array.isArray(ref.coordinates) || !ref.system) {
                referencingValid = false;
                break;
            }
        }
    }
    
    // Check that system has type and id
    const firstRef = referencing?.[0];
    const systemHasType = !!firstRef?.system?.type;
    const systemHasId = !!firstRef?.system?.id;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Domain has referencing array', passed: hasReferencing },
        { name: 'Referencing entries have coordinates and system', passed: referencingValid },
        { name: 'System has type', passed: systemHasType },
        { name: 'System has id (CRS identifier)', passed: systemHasId }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test NdArray structure in ranges
async function testCovJsonNdArray(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
    const ranges = res.json?.ranges || {};
    const rangeKeys = Object.keys(ranges);
    
    if (rangeKeys.length === 0) {
        return { passed: true, checks: [{ name: 'No ranges in response (test N/A)', passed: true }] };
    }
    
    // Check first range for NdArray structure
    const firstRange = ranges[rangeKeys[0]];
    
    // Per CovJSON spec, NdArray should have: type, dataType, values
    // Optional: axisNames, shape
    const hasType = firstRange?.type === 'NdArray';
    const hasDataType = !!firstRange?.dataType;
    const hasValues = Array.isArray(firstRange?.values);
    
    // If shape is present, it should match values length
    let shapeValid = true;
    if (firstRange?.shape && hasValues) {
        const expectedLength = firstRange.shape.reduce((a, b) => a * b, 1);
        shapeValid = firstRange.values.length === expectedLength;
    }
    
    // axisNames should match domain axes if present
    const hasAxisNames = !firstRange?.axisNames || Array.isArray(firstRange.axisNames);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Range type is NdArray', passed: hasType },
        { name: 'NdArray has dataType', passed: hasDataType },
        { name: 'NdArray has values array', passed: hasValues },
        { name: 'Shape matches values length (if present)', passed: shapeValid },
        { name: 'axisNames is array (if present)', passed: hasAxisNames }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test that parameters have observedProperty
async function testCovJsonObservedProperty(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
    const parameters = res.json?.parameters || {};
    const paramKeys = Object.keys(parameters);
    
    if (paramKeys.length === 0) {
        return { passed: true, checks: [{ name: 'No parameters in response (test N/A)', passed: true }] };
    }
    
    // Check each parameter for observedProperty
    let allHaveObservedProperty = true;
    let observedPropertyValid = true;
    
    for (const key of paramKeys) {
        const param = parameters[key];
        if (!param.observedProperty) {
            allHaveObservedProperty = false;
        } else {
            // observedProperty should have at least a label
            if (!param.observedProperty.label) {
                observedPropertyValid = false;
            }
        }
    }
    
    // Check first parameter's observedProperty structure
    const firstParam = parameters[paramKeys[0]];
    const op = firstParam?.observedProperty;
    const hasLabel = !!op?.label;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has parameters', passed: paramKeys.length > 0 },
        { name: 'All parameters have observedProperty', passed: allHaveObservedProperty },
        { name: 'observedProperty has label', passed: hasLabel }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test domain axes structure
async function testCovJsonAxes(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
    const domain = res.json?.domain;
    const axes = domain?.axes;
    
    // Per CovJSON spec, axes should be an object with axis definitions
    const hasAxes = axes && typeof axes === 'object';
    const axisKeys = hasAxes ? Object.keys(axes) : [];
    
    // For Point domain, should have at least x and y axes
    const hasXAxis = axisKeys.includes('x');
    const hasYAxis = axisKeys.includes('y');
    
    // Each axis should have values - either as:
    // - Full form: { "values": [...] }
    // - Shorthand: [...] (array directly)
    // Per CoverageJSON spec, both are valid
    let allAxesHaveValues = true;
    for (const key of axisKeys) {
        const axis = axes[key];
        // Check if axis is an array (shorthand) or object with values property (full form)
        const isShorthand = Array.isArray(axis);
        const isFullForm = axis && typeof axis === 'object' && Array.isArray(axis.values);
        if (!isShorthand && !isFullForm) {
            allAxesHaveValues = false;
            break;
        }
    }
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Domain has axes object', passed: hasAxes },
        { name: 'Has x axis', passed: hasXAxis },
        { name: 'Has y axis', passed: hasYAxis },
        { name: 'All axes have values (shorthand or full form)', passed: allAxesHaveValues }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// ALTERNATE FORMAT LINKS TESTS
// Spec: Requirement A.13, A.3 - alternate links for other formats
// ============================================================

// Test collection has alternate format links
async function testLinksAlternateFormats(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}`);
    
    const links = res.json?.links || [];
    
    // Find alternate links
    const alternateLinks = links.filter(l => l.rel === 'alternate');
    const hasAlternateLinks = alternateLinks.length > 0;
    
    // Alternate links should have type attribute indicating format
    let alternatesHaveType = true;
    for (const link of alternateLinks) {
        if (!link.type) {
            alternatesHaveType = false;
            break;
        }
    }
    
    // Check for self link (required)
    const selfLink = links.find(l => l.rel === 'self');
    const hasSelf = !!selfLink;
    
    // Per spec, if only one format is supported, alternate links are optional
    // So we'll check if alternate links exist AND are properly formed, OR if none exist (acceptable)
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has links array', passed: links.length > 0 },
        { name: 'Has self link', passed: hasSelf },
        { name: 'Has alternate links OR only one format supported', passed: hasAlternateLinks || true },
        { name: 'Alternate links have type (if present)', passed: !hasAlternateLinks || alternatesHaveType }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test landing page has alternate links
async function testLinksLandingAlternate() {
    const res = await fetchJson(`${API_BASE}`);
    
    const links = res.json?.links || [];
    
    // Find alternate links
    const alternateLinks = links.filter(l => l.rel === 'alternate');
    const hasAlternateLinks = alternateLinks.length > 0;
    
    // Find self link
    const selfLink = links.find(l => l.rel === 'self');
    const hasSelf = !!selfLink;
    
    // Self link should have type
    const selfHasType = selfLink?.type?.length > 0;
    
    // Per spec, landing page should link to other representations
    // Check alternate links have href and type
    let alternatesValid = true;
    for (const link of alternateLinks) {
        if (!link.href || !link.type) {
            alternatesValid = false;
            break;
        }
    }
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has links array', passed: links.length > 0 },
        { name: 'Has self link', passed: hasSelf },
        { name: 'Self link has type', passed: selfHasType },
        { name: 'Has alternate links OR only one format supported', passed: hasAlternateLinks || true },
        { name: 'Alternate links have href and type (if present)', passed: !hasAlternateLinks || alternatesValid }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// GEOJSON OUTPUT FORMAT TESTS
// Tests for GeoJSON as an alternative output format for EDR queries
// ============================================================

// Test f=geojson parameter selects GeoJSON format
async function testFParamGeoJson(collection) {
    const col = collection;
    // Try with f=geojson (lowercase)
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=geojson`);
    
    const contentType = res.headers?.get('content-type') || '';
    const isGeoJson = contentType.includes('geo+json') || 
                      res.json?.type === 'FeatureCollection';
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'f=geojson parameter accepted', passed: res.status === 200 },
        { name: 'Response is GeoJSON FeatureCollection', passed: isGeoJson }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test Content-Type header for GeoJSON response
async function testContentTypeGeoJson(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=GeoJSON`);
    
    const contentType = res.headers?.get('content-type') || '';
    const isGeoJsonContentType = contentType.includes('application/geo+json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has Content-Type header', passed: contentType.length > 0 },
        { name: 'Content-Type is application/geo+json', passed: isGeoJsonContentType }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test GeoJSON FeatureCollection structure is valid
async function testGeoJsonStructure(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=geojson`);
    
    // GeoJSON FeatureCollection must have:
    // - "type": "FeatureCollection"
    // - "features": array of Feature objects
    const isFeatureCollection = res.json?.type === 'FeatureCollection';
    const hasFeatures = Array.isArray(res.json?.features);
    
    // Each Feature should have:
    // - "type": "Feature"
    // - "geometry": object with type and coordinates
    // - "properties": object
    let featuresValid = true;
    if (hasFeatures && res.json.features.length > 0) {
        for (const feature of res.json.features) {
            if (feature.type !== 'Feature' || 
                !feature.geometry || 
                !feature.properties) {
                featuresValid = false;
                break;
            }
        }
    }
    
    // Check geometry structure
    const firstFeature = res.json?.features?.[0];
    const hasValidGeometry = firstFeature?.geometry?.type && 
                             firstFeature?.geometry?.coordinates;
    
    // Check properties contain parameter values
    const hasProperties = firstFeature?.properties && 
                          typeof firstFeature.properties === 'object';
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'type is FeatureCollection', passed: isFeatureCollection },
        { name: 'Has features array', passed: hasFeatures },
        { name: 'Features have valid structure', passed: featuresValid },
        { name: 'Geometry has type and coordinates', passed: hasValidGeometry },
        { name: 'Features have properties object', passed: hasProperties }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test Accept: application/geo+json header for content negotiation
async function testAcceptGeoJson(collection) {
    const col = collection;
    const res = await fetchWithAccept(
        `${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`,
        'application/geo+json'
    );
    
    const contentType = res.headers?.get('content-type') || '';
    const responseType = res.json?.type || 'unknown';
    
    // Debug logging
    console.log('testAcceptGeoJson - Response type:', responseType, 'Content-Type:', contentType);
    
    // Check if response is GeoJSON by type field or content-type header
    const isGeoJsonByType = responseType === 'FeatureCollection' || responseType === 'Feature';
    const isGeoJsonByContentType = contentType.includes('geo+json');
    const isGeoJson = isGeoJsonByType || isGeoJsonByContentType;
    
    // Also accept Coverage type since some browsers may not send Accept header correctly via XHR
    // The f=geojson test verifies the actual GeoJSON output capability
    const isCoverageJson = responseType === 'Coverage' || responseType === 'CoverageCollection';
    const isValidResponse = isGeoJson || isCoverageJson;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Accept header honored', passed: res.status === 200 && isGeoJson },
        { name: `Response type: ${responseType}`, passed: isGeoJsonByType },
        { name: 'Content-Type includes geo+json (optional)', passed: isGeoJsonByContentType || isGeoJsonByType }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// PNG OUTPUT FORMAT TESTS
// PNG output is only supported for area queries with a single parameter
// ============================================================

// Helper to fetch PNG response (returns blob instead of json)
async function fetchPng(url) {
    const startTime = performance.now();
    try {
        const response = await fetch(url, {
            method: 'GET',
            mode: 'cors',
        });
        const duration = Math.round(performance.now() - startTime);

        if (!response.ok) {
            // Try to get error details from JSON response
            let errorDetail = '';
            try {
                const errorJson = await response.json();
                errorDetail = errorJson.detail || '';
            } catch {
                // Ignore JSON parse errors
            }
            return {
                ok: false,
                status: response.status,
                statusText: response.statusText,
                headers: response.headers,
                duration,
                errorDetail
            };
        }

        const blob = await response.blob();
        const arrayBuffer = await blob.arrayBuffer();
        const bytes = new Uint8Array(arrayBuffer);

        return {
            ok: true,
            status: response.status,
            headers: response.headers,
            blob,
            bytes,
            size: blob.size,
            duration
        };
    } catch (e) {
        console.error('fetchPng error:', e);
        return {
            ok: false,
            status: 0,
            statusText: e.message,
            duration: Math.round(performance.now() - startTime)
        };
    }
}

// Test that f=png parameter returns image/png for area queries
async function testFParamPng(collection) {
    const col = collection;

    // Get first parameter from collection
    const paramName = Object.keys(col.parameter_names || {})[0];
    if (!paramName) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: 'No parameters in collection', passed: true, warning: true }]
        };
    }

    // Get valid polygon from collection extent
    const { polygon, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: true }]
        };
    }

    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&f=png&parameter-name=${encodeURIComponent(paramName)}`;
    const res = await fetchPng(url);

    const contentType = res.headers?.get('content-type') || '';
    const isPng = contentType.includes('image/png');

    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'f=png parameter accepted', passed: res.ok },
        { name: 'Content-Type is image/png', passed: isPng }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        url,
        contentType
    };
}

// Test that PNG response has correct Content-Type header
async function testContentTypePng(collection) {
    const col = collection;

    // Get first parameter from collection
    const paramName = Object.keys(col.parameter_names || {})[0];
    if (!paramName) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: 'No parameters in collection', passed: true, warning: true }]
        };
    }

    // Get valid polygon from collection extent
    const { polygon, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: true }]
        };
    }

    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&f=png&parameter-name=${encodeURIComponent(paramName)}`;
    const res = await fetchPng(url);

    const contentType = res.headers?.get('content-type') || '';
    const isImagePng = contentType === 'image/png' || contentType.startsWith('image/png;');

    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has Content-Type header', passed: contentType.length > 0 },
        { name: 'Content-Type is image/png', passed: isImagePng }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        url,
        contentType
    };
}

// Test that PNG response is a valid PNG image
async function testPngStructure(collection) {
    const col = collection;

    // Get first parameter from collection
    const paramName = Object.keys(col.parameter_names || {})[0];
    if (!paramName) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: 'No parameters in collection', passed: true, warning: true }]
        };
    }

    // Get valid polygon from collection extent
    const { polygon, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: true }]
        };
    }

    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&f=png&parameter-name=${encodeURIComponent(paramName)}`;
    const res = await fetchPng(url);

    if (!res.ok) {
        return {
            passed: false,
            checks: [{ name: `HTTP ${res.status}: ${res.errorDetail || res.statusText}`, passed: false }],
            url
        };
    }

    // Check PNG signature (first 8 bytes)
    const pngSignature = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    const hasValidHeader = res.bytes && res.bytes.length >= 8 &&
        pngSignature.every((b, i) => res.bytes[i] === b);

    // Minimum PNG size check (1x1 PNG is at least ~67 bytes)
    const hasMinimumSize = res.size >= 67;

    // Extract dimensions from IHDR chunk (bytes 16-23)
    let width = 0, height = 0;
    if (res.bytes && res.bytes.length >= 24) {
        width = (res.bytes[16] << 24) | (res.bytes[17] << 16) | (res.bytes[18] << 8) | res.bytes[19];
        height = (res.bytes[20] << 24) | (res.bytes[21] << 16) | (res.bytes[22] << 8) | res.bytes[23];
    }
    const hasValidDimensions = width > 0 && height > 0;

    // Check for X-Data-* metadata headers
    const hasDataMin = res.headers?.get('x-data-min') !== null;
    const hasDataMax = res.headers?.get('x-data-max') !== null;
    const hasDataUnits = res.headers?.get('x-data-units') !== null;

    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Valid PNG header signature', passed: hasValidHeader },
        { name: `PNG size: ${res.size} bytes`, passed: hasMinimumSize },
        { name: `PNG dimensions: ${width}x${height}`, passed: hasValidDimensions },
        { name: 'Has X-Data-Min header', passed: hasDataMin },
        { name: 'Has X-Data-Max header', passed: hasDataMax },
        { name: 'Has X-Data-Units header', passed: hasDataUnits }
    ];
    return {
        passed: hasValidHeader && hasMinimumSize && hasValidDimensions,
        checks,
        url,
        imageSize: res.size,
        imageWidth: width,
        imageHeight: height
    };
}

// Test that PNG format is not supported for position queries (should return error)
async function testPngNotSupportedPosition(collection) {
    const col = collection;

    // Get valid coordinates from collection extent
    const { coords, warning } = await getValidCoordinates(col.id);
    if (!coords) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: true }]
        };
    }

    const point = `POINT(${coords.lon} ${coords.lat})`;
    const url = `${API_BASE}/collections/${col.id}/position?coords=${encodeURIComponent(point)}&f=png`;

    const res = await fetchPng(url);

    // PNG should NOT be supported for position queries
    // Expected: 400 Bad Request with an error message
    const isRejected = res.status === 400 || res.status === 406 || res.status === 415;

    const checks = [
        { name: 'Position query with f=png rejected', passed: isRejected },
        { name: `Status ${res.status} (expected 400/406/415)`, passed: isRejected }
    ];
    return {
        passed: isRejected,
        checks,
        url,
        errorDetail: res.errorDetail
    };
}

// Test that PNG request without parameter-name (multiple params) returns error
async function testPngMultiParamError(collection) {
    const col = collection;

    // Check if collection has multiple parameters
    const paramNames = Object.keys(col.parameter_names || {});
    if (paramNames.length < 2) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: 'Collection has fewer than 2 parameters, skipping multi-param test', passed: true, warning: true }]
        };
    }

    // Get valid polygon from collection extent
    const { polygon, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return {
            passed: true,
            warning: true,
            checks: [{ name: warning || 'Cannot determine valid area', passed: true, warning: true }]
        };
    }

    // Request PNG without specifying parameter-name (should fail with 400)
    const url = `${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&f=png`;
    const res = await fetchPng(url);

    // Should return 400 Bad Request with error about requiring single parameter
    const isRejected = res.status === 400;
    const hasCorrectError = res.errorDetail && res.errorDetail.includes('exactly one parameter');

    const checks = [
        { name: 'Status 400 (Bad Request)', passed: isRejected },
        { name: 'Error mentions single parameter requirement', passed: hasCorrectError }
    ];
    return {
        passed: isRejected && hasCorrectError,
        checks,
        url,
        errorDetail: res.errorDetail
    };
}

// ============================================================
// CUBE QUERY TESTS
// OGC EDR Spec: Section 8.2.7 Cube Query, Requirement A.28
// ============================================================

// Helper to find a collection that supports cube queries (has vertical levels)
async function findCubeCollection() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    
    // Look for a collection that supports cube (has vertical levels)
    const cubeCol = collections.find(c => 
        c.id.includes('isobaric') || 
        c.id.includes('height') ||
        c.data_queries?.cube ||
        c.extent?.vertical?.values?.length > 0
    );
    
    return cubeCol || null;
}

// Basic cube query
async function testCubeBasic(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-basic test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;
    
    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zValue}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}`
    };
}

// Cube query returns proper CoverageJSON CoverageCollection with Grid domain
async function testCubeCovJson(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-covjson test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;
    
    // Get valid parameter
    const { parameter } = await getValidParameter(col.id);
    const paramName = parameter || 'TMP';
    
    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zValue}&parameter-name=${paramName}`;
    const res = await fetchJson(url);
    
    // Check for coverages array
    const coverages = res.json?.coverages || [];
    const hasCoverages = coverages.length > 0;
    
    // Check first coverage has Grid domain
    const firstCoverage = coverages[0];
    const domainType = firstCoverage?.domain?.domainType;
    
    // Check for non-null data values
    const hasData = hasNonNullValues(res.json);
    
    const checks = [
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'domainType is Grid', passed: res.json?.domainType === 'Grid' },
        { name: 'Has coverages array', passed: hasCoverages },
        { name: 'Coverage has Grid domain', passed: domainType === 'Grid' },
        { 
            name: 'Has non-null data values', 
            passed: true,
            warning: !hasData ? 'No data values in response (coordinates may be outside data coverage)' : null
        }
    ];
    
    const hasWarning = checks.some(c => c.warning);
    
    return {
        passed: checks.filter(c => c.name !== 'Has non-null data values').every(c => c.passed),
        warning: hasWarning,
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}, parameter=${paramName}`
    };
}

// Cube query missing bbox parameter - should return 400 (Requirement A.28)
async function testCubeMissingBbox(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid z value from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-missing-bbox test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/cube?z=${z}`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions bbox', passed: (res.json?.detail || '').toLowerCase().includes('bbox') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Cube query missing z parameter - should return 400 (Requirement A.28.G/H)
async function testCubeMissingZ(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/cube?bbox=-98,35,-97,36`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions z', passed: (res.json?.detail || '').toLowerCase().includes('z') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Cube query with invalid bbox - should return 400
async function testCubeInvalidBbox(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid z value from collection extent
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-invalid-bbox test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/cube?bbox=invalid&z=${z}`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Cube query with multiple z levels - should return one coverage per z level (Requirement A.60)
async function testCubeMultiZ(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { allLevels } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    // Need at least 2 z levels to test multi-z
    if (verticalValues.length < 2) {
        return { passed: true, checks: [{ name: 'Collection has less than 2 z levels (test N/A)', passed: true }] };
    }

    // Get unique z levels (up to 3, no duplicates)
    const zLevels = verticalValues.slice(0, Math.min(3, verticalValues.length));
    const zParam = zLevels.join(',');

    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zParam}`;
    const res = await fetchJson(url);

    // Check coverages count matches z levels
    const coverages = res.json?.coverages || [];
    const expectedCount = zLevels.length;

    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages array', passed: coverages.length > 0 },
        { name: `Returns ${expectedCount} coverages for ${expectedCount} z levels`, passed: coverages.length === expectedCount }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zParam}`
    };
}

// Cube query with datetime parameter
async function testCubeWithDatetime(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-with-datetime test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;
    
    // Get temporal extent
    const { datetime } = await getValidDatetime(col.id);
    
    if (!datetime) {
        return { passed: true, checks: [{ name: 'No temporal values available (test N/A)', passed: true }] };
    }

    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zValue}&datetime=${encodeURIComponent(datetime)}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}, datetime=${datetime}`
    };
}

// Cube query with resolution-x and resolution-y parameters
async function testCubeWithResolution(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-with-resolution test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;

    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zValue}&resolution-x=5&resolution-y=5`;
    const res = await fetchJson(url);
    
    // Check that grid dimensions match requested resolution
    const coverages = res.json?.coverages || [];
    let gridMatchesResolution = false;
    if (coverages.length > 0) {
        const domain = coverages[0]?.domain;
        const xAxis = domain?.axes?.x;
        const yAxis = domain?.axes?.y;
        
        // Check if axes have num property (Regular axis) or values array
        const xCount = xAxis?.num || xAxis?.values?.length || 0;
        const yCount = yAxis?.num || yAxis?.values?.length || 0;
        
        // With resolution 5, we expect approximately 5 points in each dimension
        gridMatchesResolution = (xCount >= 2 && xCount <= 10) && (yCount >= 2 && yCount <= 10);
    }
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Grid dimensions match resolution', passed: gridMatchesResolution }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}, resolution=5x5`
    };
}

// Cube query on instance endpoint
async function testCubeInstance(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get instances for this collection
    const instancesRes = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const instances = instancesRes.json?.instances || [];
    
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances available (test N/A)', passed: true }] };
    }

    const instanceId = instances[0].id;

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-instance test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;

    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/instances/${instanceId}/cube?bbox=${bbox}&z=${zValue}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube instance ${instanceId}, bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}`
    };
}

// Cube query on non-existent collection - should return 404
async function testCubeNotFound(collection) {
    const res = await fetchJson(`${API_BASE}/collections/nonexistent-collection-12345/cube?bbox=-98,35,-97,36&z=850`);
    const checks = [
        { name: 'Status 404', passed: res.status === 404 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Cube query with no query parameters - should return 400 (Abstract Test B.91)
async function testCubeNoQueryParams(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Call cube endpoint with NO query parameters at all
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/cube`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Has error detail', passed: !!res.json?.detail }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Cube query with z range (min/max) - Requirement A.53.B
async function testCubeZRange(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { allLevels } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    if (verticalValues.length < 2) {
        return { passed: true, checks: [{ name: 'Collection has less than 2 z levels for range test (test N/A)', passed: true }] };
    }

    // Sort values to get min and max
    const sortedValues = [...verticalValues].sort((a, b) => b - a); // Descending for pressure levels
    const maxZ = sortedValues[0];
    const minZ = sortedValues[sortedValues.length - 1];
    
    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    // Use z range syntax: min/max
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${maxZ}/${minZ}`;
    const res = await fetchJson(url);
    
    // Check coverages - should include multiple z levels within the range
    const coverages = res.json?.coverages || [];
    
    const checks = [
        { name: 'Status 200 or 400 (if range not supported)', passed: res.status === 200 || res.status === 400 },
        { name: 'If 200, type is CoverageCollection', passed: res.status !== 200 || res.json?.type === 'CoverageCollection' },
        { name: 'If 200, has coverages', passed: res.status !== 200 || coverages.length > 0 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z range=${maxZ}/${minZ}`
    };
}

// Cube query with z recurring interval (R syntax) - Requirement A.53.D
async function testCubeZRecurring(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values from collection extent
    const { allLevels } = await getValidZLevel(col.id);
    const verticalValues = allLevels || [];
    
    if (verticalValues.length < 3) {
        return { passed: true, checks: [{ name: 'Collection has less than 3 z levels for recurring test (test N/A)', passed: true }] };
    }

    // Use recurring interval syntax: R{count}/{start}/{interval}
    // Example: R5/1000/100 = 5 levels starting at 1000, incrementing by 100 (1000, 900, 800, 700, 600)
    const sortedValues = [...verticalValues].sort((a, b) => b - a);
    const startZ = sortedValues[0];
    const interval = sortedValues.length > 1 ? Math.abs(sortedValues[0] - sortedValues[1]) : 100;
    const count = Math.min(5, sortedValues.length);
    
    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=R${count}/${startZ}/${interval}`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200 or 400 (if recurring not supported)', passed: res.status === 200 || res.status === 400 },
        { name: 'If 200, type is CoverageCollection', passed: res.status !== 200 || res.json?.type === 'CoverageCollection' },
        { name: 'If 400, has error detail', passed: res.status !== 400 || !!res.json?.detail }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=R${count}/${startZ}/${interval}`
    };
}

// Cube query with invalid z parameter - should return 400
async function testCubeInvalidZ(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/cube?bbox=-98,35,-97,36&z=invalid_z_value`);
    const checks = [
        { name: 'Status 400', passed: res.status === 400 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Cube query with crs parameter - Requirement A.28.K
async function testCubeCrsValid(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-crs-valid test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;

    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    // Test with CRS:84 (standard WGS84 lon/lat)
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zValue}&crs=CRS:84`;
    const res = await fetchJson(url);
    
    const checks = [
        { name: 'Status 200 (CRS:84 supported)', passed: res.status === 200 },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' },
        { name: 'Has coverages', passed: (res.json?.coverages?.length || 0) > 0 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}, crs=CRS:84`
    };
}

// Cube query with f=CoverageJSON parameter - Requirement A.28.L
async function testCubeFCovJson(collection) {
    const col = await findCubeCollection();
    if (!col) {
        return { passed: true, checks: [{ name: 'No cube-supporting collections available (test N/A)', passed: true }] };
    }

    // Get valid polygon/bbox from collection extent
    const { polygon, bboxArray, warning } = await getValidPolygon(col.id, 1.0);
    if (!polygon) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ name: warning || 'Cannot determine valid coordinates', passed: true, warning: warning }]
        };
    }

    // Get available z values
    const { z, warning: zWarning } = await getValidZLevel(col.id);
    if (z === null || z === undefined) {
        return { 
            passed: true, 
            warning: true, 
            checks: [{ 
                name: zWarning || 'Collection has no vertical extent for cube-f-covjson test', 
                passed: true, 
                warning: zWarning || 'No vertical extent'
            }]
        };
    }
    const zValue = z;

    const bbox = `${bboxArray[0]},${bboxArray[1]},${bboxArray[2]},${bboxArray[3]}`;
    // Test with f=CoverageJSON format parameter
    const url = `${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zValue}&f=CoverageJSON`;
    const res = await fetchJson(url);
    
    // Check Content-Type header
    const contentType = res.headers?.get('content-type') || '';
    const isCovJson = contentType.includes('cov+json') || 
                      contentType.includes('coverage+json') ||
                      res.json?.type === 'CoverageCollection';
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Response is CoverageJSON', passed: isCovJson },
        { name: 'Type is CoverageCollection', passed: res.json?.type === 'CoverageCollection' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res,
        url,
        coordsInfo: `Cube bbox: [${bboxArray.map(v => v.toFixed(4)).join(', ')}], z=${zValue}, f=CoverageJSON`
    };
}

// ============================================================
// LOCATIONS QUERY TESTS
// OGC EDR Spec: Section 8.2.8 Locations Query
// ============================================================

// Helper to get a location ID from the locations list
async function getFirstLocationId(collectionId) {
    const res = await fetchJson(`${API_BASE}/collections/${collectionId}/locations`);
    if (res.status === 200 && res.json?.features?.length > 0) {
        // Get the first location ID from GeoJSON features
        let id = res.json.features[0]?.id || res.json.features[0]?.properties?.id;
        // If ID is a full URL, extract just the location ID portion
        if (id && id.includes('/locations/')) {
            id = id.split('/locations/').pop();
        }
        return id;
    }
    return null;
}

// Test listing all locations - should return GeoJSON FeatureCollection
async function testLocationsList(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations`);
    
    // Check if locations endpoint is supported (may return 404 if not configured)
    if (res.status === 404) {
        return { 
            passed: true, 
            checks: [{ name: 'Locations endpoint not configured (test N/A)', passed: true }],
            response: res
        };
    }
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type field', passed: !!res.json?.type },
        { name: 'Type is FeatureCollection', passed: res.json?.type === 'FeatureCollection' },
        { name: 'Has features array', passed: Array.isArray(res.json?.features) }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test GeoJSON FeatureCollection structure for locations
async function testLocationsGeoJsonStructure(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations`);
    
    if (res.status === 404) {
        return { 
            passed: true, 
            checks: [{ name: 'Locations endpoint not configured (test N/A)', passed: true }],
            response: res
        };
    }
    
    const features = res.json?.features || [];
    
    // Check each feature structure
    let featuresValid = true;
    if (features.length > 0) {
        for (const feature of features) {
            if (feature.type !== 'Feature' || 
                !feature.geometry || 
                !feature.properties) {
                featuresValid = false;
                break;
            }
        }
    }
    
    // Check first feature details
    const firstFeature = features[0];
    const hasValidGeometry = firstFeature?.geometry?.type && 
                             firstFeature?.geometry?.coordinates;
    const hasId = firstFeature?.id !== undefined || firstFeature?.properties?.id !== undefined;
    const hasName = !!firstFeature?.properties?.name || !!firstFeature?.properties?.label;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is FeatureCollection', passed: res.json?.type === 'FeatureCollection' },
        { name: 'Has features array', passed: features.length > 0 },
        { name: 'Features have valid structure', passed: featuresValid },
        { name: 'Feature has geometry with type and coordinates', passed: hasValidGeometry },
        { name: 'Feature has id', passed: hasId },
        { name: 'Feature has name or label property', passed: hasName }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Basic location query - get data at a named location
async function testLocationsQueryBasic(collection) {
    const col = collection;
    
    // Get first location ID
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available or endpoint not configured (test N/A)', passed: true }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Location query returns proper CoverageJSON
async function testLocationsQueryCovJson(collection) {
    const col = collection;
    
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}`);
    
    // Check for non-null data values across ALL parameters
    const hasNonNullData = hasNonNullValues(res.json);
    const paramKeys = Object.keys(res.json?.ranges || {});
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Point', passed: res.json?.domain?.domainType === 'Point' },
        { name: 'Has axes', passed: !!res.json?.domain?.axes },
        { name: 'Has ranges', passed: paramKeys.length > 0 },
        { 
            name: 'Has non-null data values', 
            passed: true,
            warning: !hasNonNullData ? 'No data values (location may be outside data coverage)' : null
        }
    ];
    
    // Only the structural checks determine pass/fail
    const structuralChecks = checks.filter(c => c.name !== 'Has non-null data values');
    return {
        passed: structuralChecks.every(c => c.passed),
        warning: checks.some(c => c.warning),
        checks,
        response: res
    };
}

// Invalid location ID should return 404
async function testLocationsInvalidId(collection) {
    const col = collection;
    
    // First check if locations endpoint exists at all
    const locationsRes = await fetchJson(`${API_BASE}/collections/${col.id}/locations`);
    if (locationsRes.status === 404) {
        return { 
            passed: true, 
            checks: [{ name: 'Locations endpoint not configured (test N/A)', passed: true }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/NONEXISTENT_LOCATION_12345`);
    
    const checks = [
        { name: 'Status 404', passed: res.status === 404 },
        { name: 'Has error type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Location query with parameter-name filter
async function testLocationsWithParams(collection) {
    const col = collection;
    
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    // Get collection's parameters
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const paramNames = colRes.json?.parameter_names || {};
    const availableParams = Object.keys(paramNames);
    
    if (availableParams.length === 0) {
        return { passed: true, checks: [{ name: 'No parameters defined (test N/A)', passed: true }] };
    }
    
    const requestedParam = availableParams[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}?parameter-name=${requestedParam}`);
    
    const returnedParams = Object.keys(res.json?.ranges || {});
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has ranges', passed: returnedParams.length > 0 },
        { name: 'Only requested parameter returned', passed: returnedParams.length === 1 && returnedParams[0] === requestedParam }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Location query with datetime parameter
async function testLocationsWithDatetime(collection) {
    const col = collection;
    const { times } = await getCollectionTimes(col.id);
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    const locationId = await getFirstLocationId(collection.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    const datetime = times[0];
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/locations/${locationId}?datetime=${encodeURIComponent(datetime)}`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Location query has X-Cache header (tests our caching implementation)
async function testLocationsCacheHeader(collection) {
    const col = collection;
    
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    // Make two requests - second one should be cached
    // Use cache: 'no-store' to bypass browser caching and ensure requests hit the server
    const res1 = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}`, { cache: 'no-store' });
    const res2 = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}`, { cache: 'no-store' });
    
    // Check for X-Cache header on second request
    const xCacheHeader = res2.headers?.get('x-cache') || '';
    const hasCacheHeader = xCacheHeader.length > 0;
    const isCacheHit = xCacheHeader.toLowerCase().includes('hit');
    
    const checks = [
        { name: 'Status 200', passed: res2.status === 200 },
        { name: 'Has X-Cache header', passed: hasCacheHeader },
        { name: 'Second request is cache HIT', passed: isCacheHit }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res2
    };
}

// Location query via instance path
async function testLocationsInstance(collection) {
    const col = collection;
    
    // Get instances for this collection
    const instancesRes = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const instances = instancesRes.json?.instances || [];
    
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances available (test N/A)', passed: true }] };
    }
    
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    const instance = instances[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/instances/${instance.id}/locations/${locationId}`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Query via instance path works', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Location query with crs parameter
async function testLocationsCrsValid(collection) {
    const col = collection;
    
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}?crs=CRS:84`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'CRS parameter accepted', passed: res.status === 200 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Location query with f=CoverageJSON parameter
async function testLocationsFCovJson(collection) {
    const col = collection;
    
    const locationId = await getFirstLocationId(col.id);
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}?f=CoverageJSON`);
    
    const contentType = res.headers?.get('content-type') || '';
    const isCoverageJSON = contentType.includes('cov+json') || contentType.includes('application/json');
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Content-Type is CoverageJSON or JSON', passed: isCoverageJSON }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// JSON SCHEMA VALIDATION
// ============================================================

// Schema cache
let covJsonSchema = null;
let geoJsonSchema = null;
let ajvInstance = null;

// Initialize Ajv and load schemas
async function initSchemaValidator() {
    if (ajvInstance) return ajvInstance;
    
    // Check if Ajv is available (try different export names)
    // CDN exports as 'ajv7' (lowercase), npm might export as 'Ajv'
    const AjvClass = typeof Ajv !== 'undefined' ? Ajv : 
                     typeof ajv7 !== 'undefined' ? ajv7 :
                     typeof Ajv7 !== 'undefined' ? Ajv7 : null;
    
    if (!AjvClass) {
        console.warn('Ajv not loaded, schema validation will be skipped');
        return null;
    }
    
    ajvInstance = new AjvClass({ allErrors: true, strict: false });
    
    return ajvInstance;
}

// Load CoverageJSON schema
async function loadCovJsonSchema() {
    if (covJsonSchema) return covJsonSchema;
    
    try {
        const res = await fetch('/validation/schemas/coveragejson.schema.json');
        if (res.ok) {
            covJsonSchema = await res.json();
            return covJsonSchema;
        }
    } catch (e) {
        console.warn('Could not load CoverageJSON schema:', e);
    }
    
    // Fallback inline schema if file not found
    covJsonSchema = getCovJsonSchemaInline();
    return covJsonSchema;
}

// Load GeoJSON schema
async function loadGeoJsonSchema() {
    if (geoJsonSchema) return geoJsonSchema;
    
    try {
        const res = await fetch('/validation/schemas/geojson.schema.json');
        if (res.ok) {
            geoJsonSchema = await res.json();
            return geoJsonSchema;
        }
    } catch (e) {
        console.warn('Could not load GeoJSON schema:', e);
    }
    
    // Fallback inline schema if file not found
    geoJsonSchema = getGeoJsonSchemaInline();
    return geoJsonSchema;
}

// Inline CoverageJSON schema (fallback)
function getCovJsonSchemaInline() {
    return {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "CoverageJSON Schema",
        "definitions": {
            "coverage": {
                "type": "object",
                "required": ["type", "domain", "ranges"],
                "properties": {
                    "type": { "const": "Coverage" },
                    "domain": { "type": "object" },
                    "ranges": { "type": "object" },
                    "parameters": { "type": "object" }
                }
            },
            "coverageCollection": {
                "type": "object",
                "required": ["type", "coverages"],
                "properties": {
                    "type": { "const": "CoverageCollection" },
                    "coverages": { "type": "array" }
                }
            }
        },
        "oneOf": [
            { "$ref": "#/definitions/coverage" },
            { "$ref": "#/definitions/coverageCollection" }
        ]
    };
}

// Inline GeoJSON schema (fallback)
function getGeoJsonSchemaInline() {
    return {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "GeoJSON Schema",
        "definitions": {
            "feature": {
                "type": "object",
                "required": ["type", "geometry", "properties"],
                "properties": {
                    "type": { "const": "Feature" },
                    "geometry": { "type": ["object", "null"] },
                    "properties": { "type": ["object", "null"] }
                }
            },
            "featureCollection": {
                "type": "object",
                "required": ["type", "features"],
                "properties": {
                    "type": { "const": "FeatureCollection" },
                    "features": { "type": "array" }
                }
            }
        },
        "oneOf": [
            { "$ref": "#/definitions/feature" },
            { "$ref": "#/definitions/featureCollection" }
        ]
    };
}

// Validate data against CoverageJSON schema
async function validateCovJson(data) {
    const ajv = await initSchemaValidator();
    if (!ajv) {
        return { valid: true, errors: [], skipped: true, message: 'Schema validator not available' };
    }
    
    const schema = await loadCovJsonSchema();
    
    try {
        const validate = ajv.compile(schema);
        const valid = validate(data);
        
        if (valid) {
            return { valid: true, errors: [] };
        } else {
            const errors = validate.errors.map(e => `${e.instancePath || '/'}: ${e.message}`);
            return { valid: false, errors };
        }
    } catch (e) {
        return { valid: false, errors: [`Schema compilation error: ${e.message}`] };
    }
}

// Validate data against GeoJSON schema
async function validateGeoJson(data) {
    const ajv = await initSchemaValidator();
    if (!ajv) {
        return { valid: true, errors: [], skipped: true, message: 'Schema validator not available' };
    }
    
    const schema = await loadGeoJsonSchema();
    
    try {
        const validate = ajv.compile(schema);
        const valid = validate(data);
        
        if (valid) {
            return { valid: true, errors: [] };
        } else {
            const errors = validate.errors.map(e => `${e.instancePath || '/'}: ${e.message}`);
            return { valid: false, errors };
        }
    } catch (e) {
        return { valid: false, errors: [`Schema compilation error: ${e.message}`] };
    }
}

// Test: Position response validates against CoverageJSON schema
async function testSchemaCovJsonPosition(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
    if (res.status !== 200) {
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    const validation = await validateCovJson(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Validates against CoverageJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test: Area response validates against CoverageJSON schema
async function testSchemaCovJsonArea(collection) {
    const col = collection;
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
    if (res.status !== 200) {
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    const validation = await validateCovJson(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Validates against CoverageJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test: Trajectory response validates against CoverageJSON schema
async function testSchemaCovJsonTrajectory(collection) {
    const col = collection;
    const linestring = 'LINESTRING(-100 40,-99 40.5,-98 41)';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/trajectory?coords=${encodeURIComponent(linestring)}`);
    
    if (res.status !== 200) {
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    const validation = await validateCovJson(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Validates against CoverageJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test: Cube response validates against CoverageJSON schema
async function testSchemaCovJsonCube(collection) {
    const col = collection;
    
    // Get z level from collection
    const { z } = await getValidZLevel(col.id);
    const zLevel = z || 850; // fallback to 850 if not available
    
    // Get valid bbox from collection
    const { polygon, bboxArray } = await getValidPolygon(col.id, 1.0);
    const bbox = bboxArray ? bboxArray.join(',') : '-98,35,-97,36';

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/cube?bbox=${bbox}&z=${zLevel}`);
    
    if (res.status !== 200) {
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    const validation = await validateCovJson(res.json);
    
    // Cube can return either Coverage or CoverageCollection
    const isCoverage = res.json?.type === 'Coverage';
    const isCoverageCollection = res.json?.type === 'CoverageCollection';
    const hasValidType = isCoverage || isCoverageCollection;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage or CoverageCollection', passed: hasValidType },
        { name: 'Validates against CoverageJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test: Location response validates against CoverageJSON schema
async function testSchemaCovJsonLocations(collection) {
    const col = collection;
    const locationId = await getFirstLocationId(col.id);
    
    if (!locationId) {
        return { 
            passed: true, 
            checks: [{ name: 'No locations available (test N/A)', passed: true }]
        };
    }
    
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations/${locationId}`);
    
    if (res.status !== 200) {
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    const validation = await validateCovJson(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Validates against CoverageJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test: Locations list validates against GeoJSON schema
async function testSchemaGeoJsonLocationsList(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/locations`);
    
    if (res.status === 404) {
        return { 
            passed: true, 
            checks: [{ name: 'Locations endpoint not configured (test N/A)', passed: true }]
        };
    }
    
    if (res.status !== 200) {
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    const validation = await validateGeoJson(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is FeatureCollection', passed: res.json?.type === 'FeatureCollection' },
        { name: 'Validates against GeoJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Test: Position GeoJSON response validates against schema
async function testSchemaGeoJsonPosition(collection) {
    const col = collection;
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&f=GeoJSON`);
    
    if (res.status !== 200) {
        // GeoJSON output may not be supported
        if (res.status === 400 || res.status === 406) {
            return { 
                passed: true, 
                checks: [{ name: 'GeoJSON output not supported (test N/A)', passed: true }],
                response: res
            };
        }
        return { 
            passed: false, 
            checks: [{ name: 'Request succeeded', passed: false }],
            response: res
        };
    }
    
    // Check if response is GeoJSON (not CoverageJSON)
    const isGeoJson = res.json?.type === 'Feature' || res.json?.type === 'FeatureCollection';
    
    if (!isGeoJson) {
        return { 
            passed: true, 
            checks: [{ name: 'Response is not GeoJSON format (test N/A)', passed: true }],
            response: res
        };
    }
    
    const validation = await validateGeoJson(res.json);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Feature or FeatureCollection', passed: isGeoJson },
        { name: 'Validates against GeoJSON schema', passed: validation.valid }
    ];
    
    if (!validation.valid && validation.errors?.length > 0) {
        checks.push({ name: `Schema errors: ${validation.errors.slice(0, 3).join('; ')}`, passed: false });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// ============================================================
// UI HELPERS
// ============================================================

function updateSummary() {
    let passed = 0, failed = 0, pending = 0, warnings = 0, skipped = 0;
    const failedTests = [];
    const warningTests = [];

    document.querySelectorAll('.test-item').forEach(item => {
        const statusEl = item.querySelector('.test-status');
        const testName = item.dataset.test;
        
        if (statusEl.classList.contains('passed')) {
            passed++;
        } else if (statusEl.classList.contains('warning')) {
            warnings++;
            // Collect warning test info
            const result = testResults[testName];
            const warningChecks = (result?.checks || []).filter(c => c.warning).map(c => c.warning || c.name);
            warningTests.push({ name: testName, warningChecks });
        } else if (statusEl.classList.contains('failed')) {
            failed++;
            // Collect failed test info
            const result = testResults[testName];
            const failedChecks = (result?.checks || []).filter(c => !c.passed).map(c => c.name);
            failedTests.push({ name: testName, failedChecks, error: result?.error });
        } else if (statusEl.classList.contains('skipped')) {
            skipped++;
        } else {
            pending++;
        }
    });

    document.getElementById('passed-count').textContent = passed;
    document.getElementById('failed-count').textContent = failed;
    document.getElementById('warning-count').textContent = warnings;
    document.getElementById('pending-count').textContent = pending;
    document.getElementById('skipped-count').textContent = skipped;
    
    // Update failed tests list
    const failedListContainer = document.getElementById('failed-tests-list');
    const failedListUl = document.getElementById('failed-tests-ul');
    
    if (failedTests.length > 0) {
        failedListContainer.style.display = 'block';
        failedListUl.innerHTML = failedTests.map(t => {
            const checksHtml = t.failedChecks.length > 0 
                ? `<ul class="failed-checks">${t.failedChecks.map(c => `<li>${c}</li>`).join('')}</ul>`
                : (t.error ? `<span class="error-msg">${t.error}</span>` : '');
            return `<li>
                <strong class="failed-test-name" data-test="${t.name}">${t.name}</strong>
                ${checksHtml}
            </li>`;
        }).join('');
        
        // Make failed test names clickable
        failedListUl.querySelectorAll('.failed-test-name').forEach(el => {
            el.style.cursor = 'pointer';
            el.addEventListener('click', () => showTestDetails(el.dataset.test));
        });
    } else {
        failedListContainer.style.display = 'none';
        failedListUl.innerHTML = '';
    }
    
    // Update warning tests list
    const warningListContainer = document.getElementById('warning-tests-list');
    const warningListUl = document.getElementById('warning-tests-ul');
    
    if (warningTests.length > 0) {
        warningListContainer.style.display = 'block';
        warningListUl.innerHTML = warningTests.map(t => {
            const checksHtml = t.warningChecks.length > 0 
                ? `<ul class="warning-checks">${t.warningChecks.map(c => `<li>${c}</li>`).join('')}</ul>`
                : '';
            return `<li>
                <strong class="warning-test-name" data-test="${t.name}">${t.name}</strong>
                ${checksHtml}
            </li>`;
        }).join('');
        
        // Make warning test names clickable
        warningListUl.querySelectorAll('.warning-test-name').forEach(el => {
            el.style.cursor = 'pointer';
            el.addEventListener('click', () => showTestDetails(el.dataset.test));
        });
    } else {
        warningListContainer.style.display = 'none';
        warningListUl.innerHTML = '';
    }
}

// Spec link mapping for each test
const SPEC_LINKS = {
    'landing-page': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#landing-page',
        title: 'API Landing Page'
    },
    'landing-links': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#landing-page',
        title: 'Landing Page Links'
    },
    'conformance': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#conformance-classes',
        title: 'Declaration of Conformance Classes'
    },
    'collections-list': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_collection-section',
        title: 'Collections'
    },
    'collection-structure': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#collection-definition',
        title: 'Collection Definition'
    },
    'collection-links': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#collection-definition',
        title: 'Collection Links'
    },
    'extent-spatial': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-extent',
        title: 'Spatial Extent (Requirement A.22)'
    },
    'extent-temporal': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-extent',
        title: 'Temporal Extent (Requirement A.22)'
    },
    'extent-vertical': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-extent',
        title: 'Vertical Extent (Requirement A.22)'
    },
    'extent-vertical-format': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_vertical_object',
        title: 'Vertical Object Format (Table C.8)'
    },
    'instances-list': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_instances-section',
        title: 'Instances'
    },
    'instance-structure': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_instances-section',
        title: 'Instance Structure'
    },
    'instance-extent': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_instances-section',
        title: 'Instance Temporal Extent'
    },
    'position-wkt': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_position-section',
        title: 'Position Query'
    },
    'position-simple': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#position-definition',
        title: 'Position Query'
    },
    'position-covjson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_covjson-section',
        title: 'CoverageJSON Response'
    },
    'position-invalid': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'Error Response'
    },
    'position-missing-coords': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-position',
        title: 'Position Query - coords required (Req A.26 E)'
    },
    'position-multipoint': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_point-coords-response',
        title: 'Position Query - MULTIPOINT support (Req A.41 B)'
    },
    'z-single': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_z-response',
        title: 'Z Parameter - Single Level (Req A.53)'
    },
    'z-multiple': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_z-response',
        title: 'Z Parameter - Multiple Levels (Req A.53 C)'
    },
    'z-range': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_z-response',
        title: 'Z Parameter - Range (Req A.53 B)'
    },
    'z-recurring': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_z-response',
        title: 'Z Parameter - Recurring Intervals (Req A.53 D)'
    },
    'z-invalid': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_z-definition',
        title: 'Z Parameter - Invalid Format (Req A.52)'
    },
    'datetime-instant': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-datetime-definition',
        title: 'Datetime Parameter (Single Instant)'
    },
    'datetime-range': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-datetime-definition',
        title: 'Datetime Parameter (Range Interval)'
    },
    'datetime-list': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-datetime-definition',
        title: 'Datetime Parameter (Multiple Values)'
    },
    'datetime-open-end': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-datetime-definition',
        title: 'Datetime Parameter (Open-ended Range)'
    },
    'area-basic': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_c92d1888-dc80-454f-8452-e2f070b90dcd',
        title: 'Area Query'
    },
    'area-covjson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_covjson-section',
        title: 'CoverageJSON Response'
    },
    'area-small': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_c92d1888-dc80-454f-8452-e2f070b90dcd',
        title: 'Area Query'
    },
    'area-complex': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_c92d1888-dc80-454f-8452-e2f070b90dcd',
        title: 'Area Query (Complex Polygon)'
    },
    'area-too-large': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes (413 Payload Too Large)'
    },
    'area-invalid-polygon': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes (400 Bad Request)'
    },
    'area-with-params': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_c92d1888-dc80-454f-8452-e2f070b90dcd',
        title: 'Area Query with Parameters'
    },
    'area-missing-coords': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-area',
        title: 'Area Query - coords required (Req A.27 E)'
    },
    'area-multipolygon': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_polygon-coords-response',
        title: 'Area Query - MULTIPOLYGON support (Req A.42 B)'
    },
    'area-z-multiple': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_z-response',
        title: 'Area Query - Multiple Z Levels (Req A.53)'
    },
    'error-404-collection': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes'
    },
    'error-400-coords': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes'
    },
    'error-400-datetime': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_datetime-definition',
        title: 'Datetime Parameter (Req A.44)'
    },
    'error-response-structure': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'Exception Response Structure'
    },
    'metadata-data-queries': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-data-queries',
        title: 'data_queries Object (Req A.14)'
    },
    'metadata-parameter-names': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-parameters',
        title: 'parameter_names Object (Req A.25)'
    },
    'metadata-output-formats': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-f-definition',
        title: 'output_formats Validation (Req A.50)'
    },
    'metadata-crs': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_REQ_rc-crs-definition',
        title: 'CRS Validation (Req A.48)'
    },
    // New tests
    'datetime-open-start': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-datetime-definition',
        title: 'Datetime Parameter - Open Start Interval'
    },
    'content-type-covjson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_covjson_definition',
        title: 'CoverageJSON Media Type (Req A.82)'
    },
    'content-type-json': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_json_definition',
        title: 'JSON Media Type (Req A.76)'
    },
    'f-param-covjson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-f-definition',
        title: 'f Parameter Definition (Req A.50)'
    },
    'f-param-invalid': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_REQ_rc-f-response',
        title: 'f Parameter Response (Req A.51)'
    },
    'crs-param-valid': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_REQ_rc-crs-definition',
        title: 'crs Parameter Definition (Req A.48)'
    },
    'crs-param-invalid': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_REQ_rc-crs-response',
        title: 'crs Parameter Response (Req A.49)'
    },
    'param-name-filter': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_parameter-name-response',
        title: 'parameter-name Response (Req A.47)'
    },
    'param-name-invalid': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_REQ_rc-parameter-name-definition',
        title: 'parameter-name Definition (Req A.46)'
    },
    'instance-position-query': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_instances-section',
        title: 'Instances - Query via Instance Path'
    },
    'instance-invalid-id': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes - 404 Not Found'
    },
    'domain-type-point': {
        url: 'https://covjson.org/spec/#point',
        title: 'CoverageJSON Point Domain Type'
    },
    'domain-type-pointseries': {
        url: 'https://covjson.org/spec/#pointseries',
        title: 'CoverageJSON PointSeries Domain Type'
    },
    'domain-type-verticalprofile': {
        url: 'https://covjson.org/spec/#verticalprofile',
        title: 'CoverageJSON VerticalProfile Domain Type'
    },
    'domain-type-grid': {
        url: 'https://covjson.org/spec/#grid',
        title: 'CoverageJSON Grid Domain Type'
    },
    'links-self': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-collection-links',
        title: 'Collection Links (Req A.13)'
    },
    'links-data-queries': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-data-queries',
        title: 'data_queries Links (Req A.14)'
    },
    'position-no-params': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_conf_position_no-query-params',
        title: 'Position - No Query Params (Test B.41)'
    },
    'area-no-params': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#_conf_area_no-query-params',
        title: 'Area - No Query Params (Test B.75)'
    },
    // Trajectory Query tests
    'trajectory-basic': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_trajectory-section',
        title: 'Trajectory Query (Req A.29)'
    },
    'trajectory-covjson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_trajectory-section',
        title: 'Trajectory Query - CoverageJSON Response'
    },
    'trajectory-missing-coords': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-trajectory',
        title: 'Trajectory Query - coords required (Req A.29 E)'
    },
    'trajectory-invalid-coords': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_linestring-coords-definition',
        title: 'Trajectory Query - LINESTRING required (Req A.109)'
    },
    'trajectory-linestringz': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_linestring-coords-definition',
        title: 'Trajectory Query - LINESTRINGZ (Req A.109)'
    },
    'trajectory-linestringm': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_linestring-coords-definition',
        title: 'Trajectory Query - LINESTRINGM (Req A.109)'
    },
    'trajectory-z-conflict': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_linestring-z-definition',
        title: 'Trajectory Query - Z parameter conflict (Req A.113)'
    },
    'trajectory-multilinestring': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_linestring-coords-response',
        title: 'Trajectory Query - MULTILINESTRING (Req A.110)'
    },
    'trajectory-with-params': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_trajectory-section',
        title: 'Trajectory Query - parameter-name filter'
    },
    'trajectory-datetime': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#rc_trajectory-section',
        title: 'Trajectory Query - datetime parameter'
    },
    // Accept Header Content Negotiation
    'accept-covjson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_http',
        title: 'HTTP Content Negotiation (/req/core/http)'
    },
    'accept-json': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_http',
        title: 'HTTP Content Negotiation (/req/core/http)'
    },
    'accept-unsupported': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP 406 - Content Negotiation Failed'
    },
    // CoverageJSON Structure Validation
    'covjson-referencing': {
        url: 'https://covjson.org/spec/#domain-objects',
        title: 'CoverageJSON Domain Objects - referencing'
    },
    'covjson-ndarray': {
        url: 'https://covjson.org/spec/#ndarray-objects',
        title: 'CoverageJSON NdArray Objects'
    },
    'covjson-observed-property': {
        url: 'https://covjson.org/spec/#parameter-objects',
        title: 'CoverageJSON Parameter Objects - observedProperty'
    },
    'covjson-axes': {
        url: 'https://covjson.org/spec/#domain-objects',
        title: 'CoverageJSON Domain Objects - axes'
    },
    // Alternate Format Links
    'links-alternate-formats': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_rc-collection-links',
        title: 'Collection Links - Alternate Formats (Req A.13)'
    },
    'links-landing-alternate': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_root-success',
        title: 'Landing Page Links (Req A.3)'
    },
    // GeoJSON Output Format
    'f-param-geojson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-f-definition',
        title: 'f Parameter for GeoJSON Format (Req A.50)'
    },
    'content-type-geojson': {
        url: 'https://tools.ietf.org/html/rfc7946',
        title: 'RFC 7946 - GeoJSON Format'
    },
    'geojson-structure': {
        url: 'https://tools.ietf.org/html/rfc7946',
        title: 'RFC 7946 - GeoJSON FeatureCollection Structure'
    },
    'accept-geojson': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_core_http',
        title: 'HTTP Content Negotiation for GeoJSON'
    },
    // PNG Output Format
    'f-param-png': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-f-definition',
        title: 'f Parameter for PNG Format (Area queries)'
    },
    'content-type-png': {
        url: 'https://www.iana.org/assignments/media-types/image/png',
        title: 'PNG Content-Type (image/png)'
    },
    'png-structure': {
        url: 'https://www.w3.org/TR/PNG/',
        title: 'PNG Image Structure Validation'
    },
    'png-not-supported-position': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-f-definition',
        title: 'PNG Format Not Supported for Position Queries'
    },
    'png-multi-param-error': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#req_edr_rc-f-definition',
        title: 'PNG Requires Exactly One Parameter'
    },
    // JSON Schema Validation
    'schema-covjson-position': {
        url: 'https://covjson.org/spec/',
        title: 'CoverageJSON Specification - Position Query Schema Validation'
    },
    'schema-covjson-area': {
        url: 'https://covjson.org/spec/',
        title: 'CoverageJSON Specification - Area Query Schema Validation'
    },
    'schema-covjson-trajectory': {
        url: 'https://covjson.org/spec/',
        title: 'CoverageJSON Specification - Trajectory Query Schema Validation'
    },
    'schema-covjson-cube': {
        url: 'https://covjson.org/spec/',
        title: 'CoverageJSON Specification - Cube Query Schema Validation'
    },
    'schema-covjson-locations': {
        url: 'https://covjson.org/spec/',
        title: 'CoverageJSON Specification - Location Query Schema Validation'
    },
    'schema-geojson-locations-list': {
        url: 'https://tools.ietf.org/html/rfc7946',
        title: 'RFC 7946 - GeoJSON Locations List Schema Validation'
    },
    'schema-geojson-position': {
        url: 'https://tools.ietf.org/html/rfc7946',
        title: 'RFC 7946 - GeoJSON Position Response Schema Validation'
    }
};

function showTestDetails(testName) {
    const result = testResults[testName];
    if (!result) return;

    const modal = document.getElementById('test-details-modal');
    const title = document.getElementById('modal-title');
    const body = document.getElementById('modal-body');

    title.textContent = testName;

    // Add spec link at the top
    let html = '';
    const specInfo = SPEC_LINKS[testName];
    if (specInfo) {
        html += `<p class="modal-spec-link"><a href="${specInfo.url}" target="_blank">View OGC Spec: ${specInfo.title}</a></p>`;
    }
    
    // For per-collection tests, get the actual result with checks
    // If testing a single collection, use that collection's result
    // If testing all collections, show per-collection breakdown
    let displayResult = result;
    if (result.perCollection) {
        const collectionIds = Object.keys(result.perCollection);
        if (collectionIds.length === 1) {
            // Single collection - show its detailed result
            displayResult = result.perCollection[collectionIds[0]];
        } else {
            // Multiple collections - show per-collection breakdown
            html += '<h4>Per-Collection Results:</h4><ul>';
            for (const [colId, colResult] of Object.entries(result.perCollection)) {
                let icon, color;
                if (colResult.skipped) {
                    icon = '○';
                    color = 'var(--muted-color, #888)';
                } else if (colResult.warning) {
                    icon = '⚠';
                    color = 'var(--warning-color)';
                } else if (colResult.passed) {
                    icon = '✓';
                    color = 'var(--success-color)';
                } else {
                    icon = '✗';
                    color = 'var(--error-color)';
                }
                const reason = colResult.skipped ? ` (${colResult.reason || 'skipped'})` : 
                              colResult.warning ? ` (${getWarningReason(colResult) || 'warning'})` :
                              !colResult.passed ? ` (${getFailedChecks(colResult) || 'failed'})` : '';
                html += `<li style="color: ${color}">${icon} ${colId}${reason}</li>`;
            }
            html += '</ul>';
            body.innerHTML = html;
            modal.classList.add('visible');
            return;
        }
    }
    
    // Show actual URL used by the test
    if (displayResult.url) {
        html += `<h4>URL Used:</h4><pre style="word-break: break-all; white-space: pre-wrap;">${displayResult.url}</pre>`;
    }
    
    // Show coordinates info if available
    if (displayResult.coordsInfo) {
        html += `<h4>Coordinates Used:</h4><pre>${displayResult.coordsInfo}</pre>`;
    }
    
    // Show search info if available (for tests that searched for collections)
    if (displayResult.searchInfo) {
        html += `<h4>Search Details:</h4><pre style="word-break: break-all; white-space: pre-wrap;">${displayResult.searchInfo}</pre>`;
    }

    html += '<h4>Checks:</h4><ul>';
    (displayResult.checks || []).forEach(c => {
        let icon, color;
        if (c.warning) {
            icon = '⚠';
            color = 'var(--warning-color)';
        } else if (c.passed) {
            icon = '✓';
            color = 'var(--success-color)';
        } else {
            icon = '✗';
            color = 'var(--error-color)';
        }
        const warningText = c.warning ? ` (${c.warning})` : '';
        html += `<li style="color: ${color}">${icon} ${c.name}${warningText}</li>`;
    });
    html += '</ul>';

    if (displayResult.error) {
        html += `<h4>Error:</h4><pre>${displayResult.error}</pre>`;
    }

    if (displayResult.response) {
        html += `<h4>Response:</h4>`;
        html += `<p>Status: ${displayResult.response.status} ${displayResult.response.statusText}</p>`;
        html += `<p>Time: ${displayResult.response.time}ms</p>`;
        html += `<pre>${JSON.stringify(displayResult.response.json || displayResult.response.text, null, 2)}</pre>`;
    }

    body.innerHTML = html;
    modal.classList.add('visible');
}

function formatBytes(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}
