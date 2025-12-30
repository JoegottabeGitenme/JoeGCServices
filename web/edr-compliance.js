// OGC EDR API Compliance Test Suite JavaScript

// ============================================================
// CONFIGURATION
// ============================================================

const DEFAULT_API_BASE = 'http://localhost:8083/edr';
let API_BASE = localStorage.getItem('edr-compliance-endpoint') || DEFAULT_API_BASE;

// Test state
let testResults = {};
let collections = [];
let map = null;
let marker = null;

// ============================================================
// INITIALIZATION
// ============================================================

document.addEventListener('DOMContentLoaded', () => {
    initEndpointConfig();
    initMap();
    initTestSections();
    initQueryForm();
    initModal();
    loadCollections();
});

function initEndpointConfig() {
    const input = document.getElementById('endpoint-input');
    const applyBtn = document.getElementById('endpoint-apply-btn');
    const resetBtn = document.getElementById('endpoint-reset-btn');

    input.value = API_BASE;

    applyBtn.addEventListener('click', () => {
        API_BASE = input.value.trim().replace(/\/+$/, '');
        localStorage.setItem('edr-compliance-endpoint', API_BASE);
        loadCollections();
        clearAllResults();
    });

    resetBtn.addEventListener('click', () => {
        API_BASE = DEFAULT_API_BASE;
        input.value = API_BASE;
        localStorage.removeItem('edr-compliance-endpoint');
        loadCollections();
        clearAllResults();
    });

    // Run all tests button
    document.getElementById('run-all-btn').addEventListener('click', runAllTests);
    document.getElementById('clear-results-btn').addEventListener('click', clearAllResults);
}

function initMap() {
    map = L.map('map').setView([39.8283, -98.5795], 4); // Center of US

    L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
        attribution: '&copy; OpenStreetMap contributors'
    }).addTo(map);

    // Click handler to set coordinates
    map.on('click', (e) => {
        const { lat, lng } = e.latlng;
        setCoordinates(lng, lat);
    });
}

function setCoordinates(lon, lat) {
    const coordsInput = document.getElementById('coords-input');
    coordsInput.value = `POINT(${lon.toFixed(4)} ${lat.toFixed(4)})`;

    // Update marker
    if (marker) {
        marker.setLatLng([lat, lon]);
    } else {
        marker = L.marker([lat, lon]).addTo(map);
    }
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

function initQueryForm() {
    document.getElementById('execute-query-btn').addEventListener('click', executeQuery);
    document.getElementById('copy-url-btn').addEventListener('click', copyQueryUrl);
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

async function fetchJson(url) {
    const startTime = performance.now();
    const response = await fetch(url);
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

async function loadCollections() {
    try {
        const response = await fetchJson(`${API_BASE}/collections`);
        if (response.ok && response.json?.collections) {
            collections = response.json.collections;
            updateCollectionSelect();
        }
    } catch (e) {
        console.error('Failed to load collections:', e);
        collections = [];
    }
}

function updateCollectionSelect() {
    const select = document.getElementById('collection-select');
    select.innerHTML = '<option value="">Select a collection...</option>';

    collections.forEach(col => {
        const option = document.createElement('option');
        option.value = col.id;
        option.textContent = col.title || col.id;
        select.appendChild(option);
    });
}

// ============================================================
// TEST FUNCTIONS
// ============================================================

async function runAllTests() {
    const tests = [
        'landing-page', 'landing-links', 'conformance',
        'collections-list', 'collection-structure', 'collection-links',
        'extent-spatial', 'extent-temporal', 'extent-vertical',
        'instances-list', 'instance-structure', 'instance-extent',
        'position-wkt', 'position-simple', 'position-covjson', 'position-invalid',
        'error-404-collection', 'error-400-coords'
    ];

    for (const test of tests) {
        await runTest(test);
    }
}

function clearAllResults() {
    testResults = {};
    document.querySelectorAll('.test-status').forEach(el => {
        el.className = 'test-status pending';
        el.textContent = 'Pending';
    });
    updateSummary();
}

async function runTest(testName) {
    setTestStatus(testName, 'running', 'Running...');

    try {
        const result = await executeTest(testName);
        testResults[testName] = result;

        if (result.passed) {
            setTestStatus(testName, 'passed', 'Passed');
        } else {
            setTestStatus(testName, 'failed', 'Failed');
        }
    } catch (e) {
        testResults[testName] = { passed: false, error: e.message };
        setTestStatus(testName, 'failed', 'Error');
    }

    updateSummary();
}

function setTestStatus(testName, status, text) {
    const testItem = document.querySelector(`[data-test="${testName}"]`);
    if (testItem) {
        const statusEl = testItem.querySelector('.test-status');
        statusEl.className = `test-status ${status}`;
        statusEl.textContent = text;
    }
}

async function executeTest(testName) {
    switch (testName) {
        case 'landing-page':
            return testLandingPage();
        case 'landing-links':
            return testLandingLinks();
        case 'conformance':
            return testConformance();
        case 'collections-list':
            return testCollectionsList();
        case 'collection-structure':
            return testCollectionStructure();
        case 'collection-links':
            return testCollectionLinks();
        case 'extent-spatial':
            return testExtentSpatial();
        case 'extent-temporal':
            return testExtentTemporal();
        case 'extent-vertical':
            return testExtentVertical();
        case 'instances-list':
            return testInstancesList();
        case 'instance-structure':
            return testInstanceStructure();
        case 'instance-extent':
            return testInstanceExtent();
        case 'position-wkt':
            return testPositionWkt();
        case 'position-simple':
            return testPositionSimple();
        case 'position-covjson':
            return testPositionCovJson();
        case 'position-invalid':
            return testPositionInvalid();
        case 'error-404-collection':
            return testError404Collection();
        case 'error-400-coords':
            return testError400Coords();
        default:
            return { passed: false, error: 'Unknown test' };
    }
}

// Get the URL(s) used by a test
function getTestUrls(testName) {
    const col = collections.length > 0 ? collections[0] : null;
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
        case 'error-404-collection':
            return [`${API_BASE}/collections/nonexistent-collection-12345`];
        case 'error-400-coords':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-999 999)`];
        default:
            return [];
    }
}

function copyTestUrl(testName, btn) {
    const urls = getTestUrls(testName);
    if (urls.length === 0) {
        showToast('No URL available for this test', 'error');
        return;
    }

    const textToCopy = urls.length === 1 ? urls[0] : urls.join('\n');
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
        { name: 'Includes position', passed: conformsTo.some(c => c.includes('conf/position')) }
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

async function testCollectionStructure() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const checks = [
        { name: 'Has id', passed: !!col.id },
        { name: 'Has links', passed: Array.isArray(col.links) },
        { name: 'Has extent or data_queries', passed: !!col.extent || !!col.data_queries }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: listRes
    };
}

async function testCollectionLinks() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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

async function testExtentSpatial() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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

async function testExtentTemporal() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
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
        response: colRes
    };
}

async function testExtentVertical() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    
    // Find a collection that should have vertical extent (isobaric)
    const isobaricCol = collections.find(c => c.id.includes('isobaric'));
    if (!isobaricCol) {
        // If no isobaric collection, this test is N/A
        return { 
            passed: true, 
            checks: [{ name: 'No isobaric collection (test N/A)', passed: true }],
            response: listRes
        };
    }

    const colRes = await fetchJson(`${API_BASE}/collections/${isobaricCol.id}`);
    const extent = colRes.json?.extent;
    const vertical = extent?.vertical;
    
    const checks = [
        { name: 'Has extent object', passed: !!extent },
        { name: 'Has vertical extent', passed: !!vertical },
        { name: 'Has interval or values', passed: !!(vertical?.interval || vertical?.values) },
        { name: 'Has VRS (vertical ref system)', passed: !!vertical?.vrs }
    ];
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: colRes
    };
}

// ============================================================
// INSTANCES TESTS
// ============================================================

async function testInstancesList() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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

async function testInstanceStructure() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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

async function testInstanceExtent() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const instRes = await fetchJson(`${API_BASE}/collections/${col.id}/instances`);
    const instances = instRes.json?.instances || [];
    if (instances.length === 0) {
        return { passed: true, checks: [{ name: 'No instances to test (ok)', passed: true }], response: instRes };
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
    
    // Check if the range makes sense (end > start)
    let hasValidRange = false;
    if (hasCompleteRange) {
        const start = new Date(interval[0][0]);
        const end = new Date(interval[0][1]);
        hasValidRange = end > start;
    }
    
    const checks = [
        { name: 'Instance has extent', passed: !!extent },
        { name: 'Has temporal extent', passed: !!temporal },
        { name: 'Has interval array', passed: hasValidInterval },
        { name: 'Interval has start AND end', passed: hasCompleteRange },
        { name: 'End time > start time', passed: hasValidRange || !hasCompleteRange }
    ];
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: instRes
    };
}

async function testPositionWkt() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has domain', passed: !!res.json?.domain }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testPositionSimple() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=-97.5,35.2`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testPositionCovJson() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Point', passed: res.json?.domain?.domainType === 'Point' },
        { name: 'Has axes', passed: !!res.json?.domain?.axes }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

async function testPositionInvalid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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

async function testError400Coords() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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

// ============================================================
// QUERY BUILDER
// ============================================================

async function executeQuery() {
    const collectionId = document.getElementById('collection-select').value;
    const coords = document.getElementById('coords-input').value.trim();
    const params = document.getElementById('params-input').value.trim();
    const datetime = document.getElementById('datetime-input').value.trim();
    const z = document.getElementById('z-input').value.trim();

    if (!collectionId) {
        alert('Please select a collection');
        return;
    }

    if (!coords) {
        alert('Please enter coordinates');
        return;
    }

    const url = buildQueryUrl(collectionId, coords, params, datetime, z);

    const responseBody = document.getElementById('response-body');
    const responseStatus = document.getElementById('response-status');
    const responseTime = document.getElementById('response-time');
    const responseSize = document.getElementById('response-size');

    responseBody.textContent = 'Loading...';

    try {
        const res = await fetchJson(url);
        responseStatus.textContent = `Status: ${res.status} ${res.statusText}`;
        responseTime.textContent = `Time: ${res.time}ms`;
        responseSize.textContent = `Size: ${formatBytes(res.text.length)}`;

        if (res.json) {
            responseBody.textContent = JSON.stringify(res.json, null, 2);
        } else {
            responseBody.textContent = res.text;
        }
    } catch (e) {
        responseStatus.textContent = 'Error';
        responseTime.textContent = '';
        responseSize.textContent = '';
        responseBody.textContent = `Error: ${e.message}`;
    }
}

function buildQueryUrl(collectionId, coords, params, datetime, z) {
    let url = `${API_BASE}/collections/${collectionId}/position?coords=${encodeURIComponent(coords)}`;

    if (params) {
        url += `&parameter-name=${encodeURIComponent(params)}`;
    }
    if (datetime) {
        url += `&datetime=${encodeURIComponent(datetime)}`;
    }
    if (z) {
        url += `&z=${encodeURIComponent(z)}`;
    }

    return url;
}

function copyQueryUrl() {
    const collectionId = document.getElementById('collection-select').value;
    const coords = document.getElementById('coords-input').value.trim();
    const params = document.getElementById('params-input').value.trim();
    const datetime = document.getElementById('datetime-input').value.trim();
    const z = document.getElementById('z-input').value.trim();

    if (!collectionId || !coords) {
        alert('Please select a collection and enter coordinates');
        return;
    }

    const url = buildQueryUrl(collectionId, coords, params, datetime, z);
    navigator.clipboard.writeText(url).then(() => {
        alert('URL copied to clipboard');
    });
}

// ============================================================
// UI HELPERS
// ============================================================

function updateSummary() {
    let passed = 0, failed = 0, pending = 0;

    document.querySelectorAll('.test-status').forEach(el => {
        if (el.classList.contains('passed')) passed++;
        else if (el.classList.contains('failed')) failed++;
        else pending++;
    });

    document.getElementById('passed-count').textContent = passed;
    document.getElementById('failed-count').textContent = failed;
    document.getElementById('pending-count').textContent = pending;
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
    'error-404-collection': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes'
    },
    'error-400-coords': {
        url: 'https://docs.ogc.org/is/19-086r6/19-086r6.html#http-status-codes',
        title: 'HTTP Status Codes'
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

    html += '<h4>Checks:</h4><ul>';
    (result.checks || []).forEach(c => {
        const icon = c.passed ? '✓' : '✗';
        const color = c.passed ? 'var(--success-color)' : 'var(--error-color)';
        html += `<li style="color: ${color}">${icon} ${c.name}</li>`;
    });
    html += '</ul>';

    if (result.error) {
        html += `<h4>Error:</h4><pre>${result.error}</pre>`;
    }

    if (result.response) {
        html += `<h4>Response:</h4>`;
        html += `<p>Status: ${result.response.status} ${result.response.statusText}</p>`;
        html += `<p>Time: ${result.response.time}ms</p>`;
        html += `<pre>${JSON.stringify(result.response.json || result.response.text, null, 2)}</pre>`;
    }

    body.innerHTML = html;
    modal.classList.add('visible');
}

function formatBytes(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}
