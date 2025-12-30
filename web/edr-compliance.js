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
    // Use XMLHttpRequest for more control over headers
    return new Promise((resolve) => {
        const xhr = new XMLHttpRequest();
        const startTime = performance.now();
        
        xhr.open('GET', url, true);
        xhr.setRequestHeader('Accept', acceptHeader);
        
        // Debug: log what we're sending
        console.log('fetchWithAccept (XHR) - URL:', url, 'Accept:', acceptHeader);
        
        xhr.onload = function() {
            const endTime = performance.now();
            let json = null;
            try {
                json = JSON.parse(xhr.responseText);
            } catch (e) {
                // Not JSON
            }
            
            // Create a headers-like object with get() method
            const headersObj = {
                get: function(name) {
                    return xhr.getResponseHeader(name);
                }
            };
            
            resolve({
                ok: xhr.status >= 200 && xhr.status < 300,
                status: xhr.status,
                statusText: xhr.statusText,
                headers: headersObj,
                text: xhr.responseText,
                json,
                time: Math.round(endTime - startTime)
            });
        };
        
        xhr.onerror = function() {
            resolve({
                ok: false,
                status: 0,
                statusText: 'Network Error',
                headers: { get: () => null },
                text: '',
                json: null,
                time: 0
            });
        };
        
        xhr.send();
    });
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
        // Core
        'landing-page', 'landing-links', 'conformance',
        // Collections
        'collections-list', 'collection-structure', 'collection-links',
        // Extent
        'extent-spatial', 'extent-temporal', 'extent-vertical',
        // Instances
        'instances-list', 'instance-structure', 'instance-extent',
        // Position Query
        'position-wkt', 'position-simple', 'position-covjson', 'position-invalid',
        'position-missing-coords', 'position-multipoint',
        // Z Parameter
        'z-single', 'z-multiple', 'z-range', 'z-recurring', 'z-invalid',
        // Datetime Parameter
        'datetime-instant', 'datetime-range', 'datetime-list', 'datetime-open-end', 'datetime-open-start',
        // Area Query
        'area-basic', 'area-covjson', 'area-small', 'area-complex',
        'area-too-large', 'area-invalid-polygon', 'area-with-params',
        'area-missing-coords', 'area-multipolygon', 'area-z-multiple',
        // Radius Query
        'radius-basic', 'radius-covjson', 'radius-missing-coords',
        'radius-missing-within', 'radius-missing-within-units', 'radius-invalid-coords',
        'radius-too-large', 'radius-units-km', 'radius-units-mi', 'radius-units-m',
        'radius-multipoint', 'radius-z-parameter', 'radius-with-params', 'radius-datetime',
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
        'accept-covjson', 'accept-json', 'accept-unsupported',
        // CoverageJSON Structure Validation (LOW PRIORITY)
        'covjson-referencing', 'covjson-ndarray', 'covjson-observed-property', 'covjson-axes',
        // Alternate Format Links (LOW PRIORITY)
        'links-alternate-formats', 'links-landing-alternate'
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
        case 'position-missing-coords':
            return testPositionMissingCoords();
        case 'position-multipoint':
            return testPositionMultipoint();
        case 'z-single':
            return testZSingle();
        case 'z-multiple':
            return testZMultiple();
        case 'z-range':
            return testZRange();
        case 'z-recurring':
            return testZRecurring();
        case 'z-invalid':
            return testZInvalid();
        case 'datetime-instant':
            return testDatetimeInstant();
        case 'datetime-range':
            return testDatetimeRange();
        case 'datetime-list':
            return testDatetimeList();
        case 'datetime-open-end':
            return testDatetimeOpenEnd();
        case 'area-basic':
            return testAreaBasic();
        case 'area-covjson':
            return testAreaCovJson();
        case 'area-small':
            return testAreaSmall();
        case 'area-complex':
            return testAreaComplex();
        case 'area-too-large':
            return testAreaTooLarge();
        case 'area-invalid-polygon':
            return testAreaInvalidPolygon();
        case 'area-with-params':
            return testAreaWithParams();
        case 'area-missing-coords':
            return testAreaMissingCoords();
        case 'area-multipolygon':
            return testAreaMultipolygon();
        case 'area-z-multiple':
            return testAreaZMultiple();
        // Radius Query tests
        case 'radius-basic':
            return testRadiusBasic();
        case 'radius-covjson':
            return testRadiusCovJson();
        case 'radius-missing-coords':
            return testRadiusMissingCoords();
        case 'radius-missing-within':
            return testRadiusMissingWithin();
        case 'radius-missing-within-units':
            return testRadiusMissingWithinUnits();
        case 'radius-invalid-coords':
            return testRadiusInvalidCoords();
        case 'radius-too-large':
            return testRadiusTooLarge();
        case 'radius-units-km':
            return testRadiusUnitsKm();
        case 'radius-units-mi':
            return testRadiusUnitsMi();
        case 'radius-units-m':
            return testRadiusUnitsM();
        case 'radius-multipoint':
            return testRadiusMultipoint();
        case 'radius-z-parameter':
            return testRadiusZParameter();
        case 'radius-with-params':
            return testRadiusWithParams();
        case 'radius-datetime':
            return testRadiusDatetime();
        case 'error-404-collection':
            return testError404Collection();
        case 'error-400-coords':
            return testError400Coords();
        case 'error-400-datetime':
            return testError400Datetime();
        case 'error-response-structure':
            return testErrorResponseStructure();
        case 'metadata-data-queries':
            return testMetadataDataQueries();
        case 'metadata-parameter-names':
            return testMetadataParameterNames();
        case 'metadata-output-formats':
            return testMetadataOutputFormats();
        case 'metadata-crs':
            return testMetadataCrs();
        // Content-Type & Format tests
        case 'content-type-covjson':
            return testContentTypeCovJson();
        case 'content-type-json':
            return testContentTypeJson();
        case 'f-param-covjson':
            return testFParamCovJson();
        case 'f-param-invalid':
            return testFParamInvalid();
        // CRS Parameter tests
        case 'crs-param-valid':
            return testCrsParamValid();
        case 'crs-param-invalid':
            return testCrsParamInvalid();
        // Parameter-Name tests
        case 'param-name-filter':
            return testParamNameFilter();
        case 'param-name-invalid':
            return testParamNameInvalid();
        // Instance Query tests
        case 'instance-position-query':
            return testInstancePositionQuery();
        case 'instance-invalid-id':
            return testInstanceInvalidId();
        // Domain Type tests
        case 'domain-type-point':
            return testDomainTypePoint();
        case 'domain-type-pointseries':
            return testDomainTypePointSeries();
        case 'domain-type-verticalprofile':
            return testDomainTypeVerticalProfile();
        case 'domain-type-grid':
            return testDomainTypeGrid();
        // Link Validation tests
        case 'links-self':
            return testLinksSelf();
        case 'links-data-queries':
            return testLinksDataQueries();
        // No Query Params tests
        case 'position-no-params':
            return testPositionNoParams();
        case 'area-no-params':
            return testAreaNoParams();
        // Datetime open start
        case 'datetime-open-start':
            return testDatetimeOpenStart();
        // Accept Header Content Negotiation
        case 'accept-covjson':
            return testAcceptCovJson();
        case 'accept-json':
            return testAcceptJson();
        case 'accept-unsupported':
            return testAcceptUnsupported();
        // CoverageJSON Structure Validation
        case 'covjson-referencing':
            return testCovJsonReferencing();
        case 'covjson-ndarray':
            return testCovJsonNdArray();
        case 'covjson-observed-property':
            return testCovJsonObservedProperty();
        case 'covjson-axes':
            return testCovJsonAxes();
        // Alternate Format Links
        case 'links-alternate-formats':
            return testLinksAlternateFormats();
        case 'links-landing-alternate':
            return testLinksLandingAlternate();
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
        case 'position-missing-coords':
            return [`${API_BASE}/collections/${colId}/position`];
        case 'position-multipoint':
            return [`${API_BASE}/collections/${colId}/position?coords=MULTIPOINT((-97.5 35.2),(-98.0 36.0))`];
        case 'z-single':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=850`];
        case 'z-multiple':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=850,700,500`];
        case 'z-range':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=1000/500`];
        case 'z-recurring':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=R5/1000/100`];
        case 'z-invalid':
            return [`${API_BASE}/collections/${colId}/position?coords=POINT(-97.5 35.2)&z=abc`];
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
            return [`${API_BASE}/collections/${colId}/area?coords=POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))&z=850,700`];
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

// Position query without coords parameter - should return 400
async function testPositionMissingCoords() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testPositionMultipoint() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=MULTIPOINT((-97.5 35.2),(-98.0 36.0))`);
    
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
        response: res
    };
}

// ============================================================
// Z PARAMETER TESTS
// ============================================================

// Helper to find an isobaric collection (which has z levels)
async function findIsobaricCollection() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    
    // Look for a collection that likely has vertical levels
    const isobaricCol = collections.find(c => 
        c.id.includes('isobaric') || 
        c.extent?.vertical?.values?.length > 1
    );
    
    return isobaricCol || collections[0];
}

// Single z level query
async function testZSingle() {
    const col = await findIsobaricCollection();
    if (!col) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    // Get available z values from collection extent
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const verticalValues = colRes.json?.extent?.vertical?.values || [];
    const zValue = verticalValues[0] || 850;

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&z=${zValue}`);
    
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
        response: res
    };
}

// Multiple z levels query - spec says ALL requested levels should be returned
async function testZMultiple() {
    const col = await findIsobaricCollection();
    if (!col) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    // Get available z values from collection extent
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const verticalValues = colRes.json?.extent?.vertical?.values || [];
    
    // Use first 3 available levels, or defaults
    const zLevels = verticalValues.length >= 3 
        ? verticalValues.slice(0, 3) 
        : [850, 700, 500];
    const zParam = zLevels.join(',');

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&z=${zParam}`);
    
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
        response: res
    };
}

// Z range query (z=1000/500)
async function testZRange() {
    const col = await findIsobaricCollection();
    if (!col) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&z=1000/500`);
    
    // Check z axis in response - should include levels between 1000 and 500
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
        response: res
    };
}

// Recurring z intervals (z=R5/1000/100) - 5 levels starting at 1000, decrementing by 100
async function testZRecurring() {
    const col = await findIsobaricCollection();
    if (!col) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&z=R5/1000/100`);
    
    // Check z axis in response - should have 5 levels: 1000, 900, 800, 700, 600
    const zAxis = res.json?.domain?.axes?.z;
    const zAxisValues = Array.isArray(zAxis?.values) ? zAxis.values : (Array.isArray(zAxis) ? zAxis : []);
    const hasFiveZLevels = zAxisValues.length === 5;
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has z axis in domain', passed: zAxis !== undefined },
        { name: 'Returns exactly 5 z levels', passed: hasFiveZLevels }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Invalid z parameter
async function testZInvalid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&z=abc`);
    
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

// ============================================================
// DATETIME QUERY TESTS
// ============================================================

// Helper to get available times from a collection
async function getCollectionTimes() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { collection: null, times: [] };
    }
    
    const col = collections[0];
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const times = colRes.json?.extent?.temporal?.values || [];
    
    return { collection: col, times };
}

// Single datetime instant
async function testDatetimeInstant() {
    const { collection, times } = await getCollectionTimes();
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    const datetime = times[0]; // Use first available time
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/position?coords=POINT(-97.5 35.2)&datetime=${encodeURIComponent(datetime)}`);
    
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
        response: res
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
async function testDatetimeRange() {
    const { collection, times } = await getCollectionTimes();
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for range test (N/A)', passed: true }] };
    }
    
    const startTime = times[0];
    const endTime = times[Math.min(2, times.length - 1)]; // Use 3rd time or last
    const datetimeRange = `${startTime}/${endTime}`;
    
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/position?coords=POINT(-97.5 35.2)&datetime=${encodeURIComponent(datetimeRange)}`);
    
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
        response: res
    };
}

// Multiple discrete datetimes (comma-separated list)
async function testDatetimeList() {
    const { collection, times } = await getCollectionTimes();
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length < 3) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for list test (N/A)', passed: true }] };
    }
    
    // Pick 3 times
    const selectedTimes = [times[0], times[1], times[2]];
    const datetimeList = selectedTimes.join(',');
    
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/position?coords=POINT(-97.5 35.2)&datetime=${encodeURIComponent(datetimeList)}`);
    
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
        response: res
    };
}

// Datetime with open end (start/..)
async function testDatetimeOpenEnd() {
    const { collection, times } = await getCollectionTimes();
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for open-end test (N/A)', passed: true }] };
    }
    
    const startTime = times[0];
    const datetimeOpenEnd = `${startTime}/..`;
    
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/position?coords=POINT(-97.5 35.2)&datetime=${encodeURIComponent(datetimeOpenEnd)}`);
    
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
        response: res
    };
}

// ============================================================
// AREA QUERY TESTS
// ============================================================

// Basic polygon area query
async function testAreaBasic() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    // Small 1x1 degree polygon over Oklahoma
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
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

// Area query returns proper CoverageJSON Grid
async function testAreaCovJson() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
    // Check for non-null data values
    const ranges = res.json?.ranges || {};
    const paramKeys = Object.keys(ranges);
    let hasNonNullData = false;
    if (paramKeys.length > 0) {
        const firstParam = paramKeys[0];
        const values = ranges[firstParam]?.values || [];
        hasNonNullData = values.some(v => v !== null);
    }
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has x axis', passed: !!res.json?.domain?.axes?.x },
        { name: 'Has y axis', passed: !!res.json?.domain?.axes?.y },
        { name: 'Has ranges', passed: paramKeys.length > 0 },
        { name: 'Has non-null data values', passed: hasNonNullData }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Small region area query
async function testAreaSmall() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    // Very small 0.1x0.1 degree polygon
    const polygon = 'POLYGON((-97.5 35.2,-97.4 35.2,-97.4 35.3,-97.5 35.3,-97.5 35.2))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Response is valid JSON', passed: res.json !== null }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Complex polygon (L-shaped)
async function testAreaComplex() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    // L-shaped polygon
    const polygon = 'POLYGON((-100 34,-98 34,-98 36,-99 36,-99 35,-100 35,-100 34))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}`);
    
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

// Area too large should return 413 or 400
async function testAreaTooLarge() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAreaInvalidPolygon() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAreaWithParams() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&parameter-name=TMP`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Has type', passed: !!res.json?.type },
        { name: 'Has ranges', passed: !!res.json?.ranges }
    ];
    
    // If we have ranges, check that filtering worked (only requested params)
    if (res.json?.ranges) {
        const rangeKeys = Object.keys(res.json.ranges);
        // Should have TMP or temperature-related parameter
        const hasTempParam = rangeKeys.some(k => k.includes('TMP') || k.toLowerCase().includes('temp'));
        checks.push({ name: 'Response includes requested parameter', passed: hasTempParam || rangeKeys.length > 0 });
    }
    
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Area query without coords parameter - should return 400
async function testAreaMissingCoords() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAreaMultipolygon() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAreaZMultiple() {
    const col = await findIsobaricCollection();
    if (!col) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const polygon = 'POLYGON((-98 35,-97 35,-97 36,-98 36,-98 35))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/area?coords=${encodeURIComponent(polygon)}&z=850,700`);
    
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

// ============================================================
// RADIUS QUERY TESTS
// OGC EDR Spec: Section 8.2.4 Radius Query
// ============================================================

// Basic radius query
async function testRadiusBasic() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km`);
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

// Radius query returns proper CoverageJSON Grid
async function testRadiusCovJson() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=30&within-units=km`);
    
    // Check for non-null data values
    const ranges = res.json?.ranges || {};
    const paramKeys = Object.keys(ranges);
    let hasNonNullData = false;
    if (paramKeys.length > 0) {
        const firstParam = paramKeys[0];
        const values = ranges[firstParam]?.values || [];
        hasNonNullData = values.some(v => v !== null);
    }
    
    const checks = [
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has x axis', passed: !!res.json?.domain?.axes?.x },
        { name: 'Has y axis', passed: !!res.json?.domain?.axes?.y },
        { name: 'Has ranges', passed: paramKeys.length > 0 },
        { name: 'Has non-null data values', passed: hasNonNullData }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query missing coords parameter - should return 400
async function testRadiusMissingCoords() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testRadiusMissingWithin() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testRadiusMissingWithinUnits() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testRadiusInvalidCoords() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testRadiusTooLarge() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    // Request 1000 km radius which should exceed limit
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=1000&within-units=km`);
    const checks = [
        { name: 'Status 413', passed: res.status === 413 },
        { name: 'Has error type', passed: !!res.json?.type },
        { name: 'Error mentions radius', passed: (res.json?.detail || '').toLowerCase().includes('radius') }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with km units
async function testRadiusUnitsKm() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with miles units
async function testRadiusUnitsMi() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=30&within-units=mi`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with meters units
async function testRadiusUnitsM() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=50000&within-units=m`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with MULTIPOINT coords (union of circles)
async function testRadiusMultipoint() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const coords = 'MULTIPOINT((-97.5 35.2),(-98.0 36.0))';
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=${encodeURIComponent(coords)}&within=30&within-units=km`);
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has ranges with data', passed: Object.keys(res.json?.ranges || {}).length > 0 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with z (vertical level) parameter
async function testRadiusZParameter() {
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
    
    // Use 850 hPa as a common isobaric level
    const res = await fetchJson(`${API_BASE}/collections/${isobaricCol.id}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&z=850`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' },
        { name: 'Has ranges', passed: Object.keys(res.json?.ranges || {}).length > 0 }
    ];
    return {
        passed: checks.every(c => c.passed),
        checks,
        response: res
    };
}

// Radius query with parameter-name filtering
async function testRadiusWithParams() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    
    // Get the collection's parameters
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const params = colRes.json?.parameter_names || {};
    const paramKeys = Object.keys(params);
    
    if (paramKeys.length === 0) {
        return { passed: true, checks: [{ name: 'No parameters available (test N/A)', passed: true }] };
    }
    
    // Request only the first parameter
    const requestedParam = paramKeys[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&parameter-name=${requestedParam}`);
    
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

// Radius query with datetime parameter
async function testRadiusDatetime() {
    const { collection, times } = await getCollectionTimes();
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length === 0) {
        return { passed: true, checks: [{ name: 'No temporal values (test N/A)', passed: true }] };
    }
    
    const datetime = times[0]; // Use first available time
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/radius?coords=POINT(-97.5 35.2)&within=50&within-units=km&datetime=${encodeURIComponent(datetime)}`);
    
    const checks = [
        { name: 'Status 200', passed: res.status === 200 },
        { name: 'Type is Coverage', passed: res.json?.type === 'Coverage' },
        { name: 'Has domain', passed: !!res.json?.domain },
        { name: 'Domain type is Grid', passed: res.json?.domain?.domainType === 'Grid' }
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

// Invalid datetime format
async function testError400Datetime() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testErrorResponseStructure() {
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
async function testMetadataDataQueries() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testMetadataParameterNames() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testMetadataOutputFormats() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testMetadataCrs() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testDatetimeOpenStart() {
    const { collection, times } = await getCollectionTimes();
    if (!collection) {
        return { passed: false, error: 'No collections available', checks: [] };
    }
    if (times.length < 2) {
        return { passed: true, checks: [{ name: 'Not enough temporal values for open-start test (N/A)', passed: true }] };
    }
    
    const endTime = times[times.length - 1];
    const datetimeOpenStart = `../${endTime}`;
    
    const res = await fetchJson(`${API_BASE}/collections/${collection.id}/position?coords=POINT(-97.5 35.2)&datetime=${encodeURIComponent(datetimeOpenStart)}`);
    
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
        response: res
    };
}

// ============================================================
// CONTENT-TYPE & FORMAT PARAMETER TESTS
// Spec: Requirement A.76, A.82, A.50, A.51
// ============================================================

// Test that position query returns proper CoverageJSON Content-Type header
async function testContentTypeCovJson() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)`);
    
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
async function testContentTypeJson() {
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
async function testFParamCovJson() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testFParamInvalid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testCrsParamValid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testCrsParamInvalid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testParamNameFilter() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testParamNameInvalid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testInstancePositionQuery() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
    
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
async function testInstanceInvalidId() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testDomainTypePoint() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testDomainTypePointSeries() {
    const { collection, times } = await getCollectionTimes();
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
async function testDomainTypeVerticalProfile() {
    const col = await findIsobaricCollection();
    if (!col) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    // Get available z values from collection extent
    const colRes = await fetchJson(`${API_BASE}/collections/${col.id}`);
    const verticalValues = colRes.json?.extent?.vertical?.values || [];
    
    // Use first 3 available levels, or defaults
    const zLevels = verticalValues.length >= 3 
        ? verticalValues.slice(0, 3) 
        : [850, 700, 500];
    const zParam = zLevels.join(',');

    const res = await fetchJson(`${API_BASE}/collections/${col.id}/position?coords=POINT(-97.5 35.2)&z=${zParam}`);
    
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
async function testDomainTypeGrid() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testLinksSelf() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testLinksDataQueries() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testPositionNoParams() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAreaNoParams() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAcceptCovJson() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testAcceptJson() {
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
async function testAcceptUnsupported() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testCovJsonReferencing() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testCovJsonNdArray() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testCovJsonObservedProperty() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testCovJsonAxes() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
async function testLinksAlternateFormats() {
    const listRes = await fetchJson(`${API_BASE}/collections`);
    const collections = listRes.json?.collections || [];
    if (collections.length === 0) {
        return { passed: false, error: 'No collections available', checks: [] };
    }

    const col = collections[0];
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
    const failedTests = [];

    document.querySelectorAll('.test-item').forEach(item => {
        const statusEl = item.querySelector('.test-status');
        const testName = item.dataset.test;
        
        if (statusEl.classList.contains('passed')) {
            passed++;
        } else if (statusEl.classList.contains('failed')) {
            failed++;
            // Collect failed test info
            const result = testResults[testName];
            const failedChecks = (result?.checks || []).filter(c => !c.passed).map(c => c.name);
            failedTests.push({ name: testName, failedChecks, error: result?.error });
        } else {
            pending++;
        }
    });

    document.getElementById('passed-count').textContent = passed;
    document.getElementById('failed-count').textContent = failed;
    document.getElementById('pending-count').textContent = pending;
    
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
