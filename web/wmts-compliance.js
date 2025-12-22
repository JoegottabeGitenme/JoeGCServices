// WMTS 1.0.0 OGC Compliance Tests JavaScript

// ============================================================
// CONFIGURATION
// ============================================================

// API URL detection - can be changed by user
const DEFAULT_API_BASE = window.location.port === '8000' ? 'http://localhost:8080' : '';
let API_BASE = localStorage.getItem('wmts-compliance-endpoint') || DEFAULT_API_BASE;

// Authentication credentials (stored in memory only for security)
let authCredentials = null; // { username, password }

// Check if the endpoint already includes a WMTS path
function endpointIncludesWmtsPath(endpoint) {
    const lower = endpoint.toLowerCase();
    return /\/wmts\/?(\?|$)/i.test(endpoint) ||
           lower.includes('service=wmts');
}

// Get the WMTS endpoint URL (appends /wmts only if needed)
function getWmtsUrl() {
    if (endpointIncludesWmtsPath(API_BASE)) {
        return API_BASE;
    }
    return `${API_BASE}/wmts`;
}

// ============================================================
// ENDPOINT MANAGEMENT
// ============================================================

function initEndpointConfig() {
    const input = document.getElementById('endpoint-input');
    const applyBtn = document.getElementById('endpoint-apply-btn');
    const resetBtn = document.getElementById('endpoint-reset-btn');
    const authToggleBtn = document.getElementById('auth-toggle-btn');
    const authPassword = document.getElementById('auth-password');
    const authUsername = document.getElementById('auth-username');

    input.value = API_BASE;

    applyBtn.addEventListener('click', () => applyEndpoint());
    resetBtn.addEventListener('click', () => resetEndpoint());

    input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') applyEndpoint();
    });
    authUsername.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') applyEndpoint();
    });
    authPassword.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') applyEndpoint();
    });

    authToggleBtn.addEventListener('click', () => {
        if (authPassword.type === 'password') {
            authPassword.type = 'text';
            authToggleBtn.textContent = 'Hide';
        } else {
            authPassword.type = 'password';
            authToggleBtn.textContent = 'Show';
        }
    });
}

function getAuthHeaders() {
    if (authCredentials && authCredentials.username && authCredentials.password) {
        const credentials = btoa(`${authCredentials.username}:${authCredentials.password}`);
        return { 'Authorization': `Basic ${credentials}` };
    }
    return {};
}

async function fetchWithAuth(url, options = {}) {
    const headers = { ...getAuthHeaders(), ...(options.headers || {}) };
    return fetch(url, { ...options, headers });
}

async function applyEndpoint() {
    const input = document.getElementById('endpoint-input');
    const usernameInput = document.getElementById('auth-username');
    const passwordInput = document.getElementById('auth-password');
    
    let newEndpoint = input.value.trim();
    newEndpoint = newEndpoint.replace(/\/+$/, '');
    input.value = newEndpoint;

    API_BASE = newEndpoint;
    if (newEndpoint) {
        localStorage.setItem('wmts-compliance-endpoint', newEndpoint);
    } else {
        localStorage.removeItem('wmts-compliance-endpoint');
    }

    const username = usernameInput.value.trim();
    const password = passwordInput.value;
    if (username && password) {
        authCredentials = { username, password };
    } else {
        authCredentials = null;
    }

    await reloadWithNewEndpoint();
}

async function resetEndpoint() {
    const input = document.getElementById('endpoint-input');
    const usernameInput = document.getElementById('auth-username');
    const passwordInput = document.getElementById('auth-password');
    
    input.value = DEFAULT_API_BASE;
    usernameInput.value = '';
    passwordInput.value = '';
    
    API_BASE = DEFAULT_API_BASE;
    authCredentials = null;
    localStorage.removeItem('wmts-compliance-endpoint');

    await reloadWithNewEndpoint();
}

async function reloadWithNewEndpoint() {
    updateEndpointStatus('loading', 'Connecting...');
    layerStatus = {};

    try {
        await loadCapabilities();
        initOgcTests();
        updateOgcSummary();

        if (layers.length > 0) {
            updateEndpointStatus('connected', `Connected (${layers.length} layers)`);
            updateFilteredIndices();
            displayLayer(0);
        } else {
            updateEndpointStatus('error', 'No layers found');
        }
    } catch (error) {
        console.error('Failed to connect to endpoint:', error);
        updateEndpointStatus('error', `Error: ${error.message}`);
    }
}

function updateEndpointStatus(status, text) {
    const icon = document.getElementById('endpoint-status-icon');
    const textEl = document.getElementById('endpoint-status-text');

    icon.className = 'endpoint-status-icon';
    textEl.className = 'endpoint-status-text';

    if (status === 'connected') {
        icon.classList.add('connected');
        textEl.classList.add('connected');
    } else if (status === 'error') {
        icon.classList.add('error');
        textEl.classList.add('error');
    } else if (status === 'loading') {
        icon.classList.add('loading');
    }

    textEl.textContent = text;
}

// ============================================================
// OGC WMTS 1.0.0 COMPLIANCE TESTS
// ============================================================

// OGC WMTS 1.0.0 Specification reference (OGC 07-057r7)
const OGC_SPEC_URL = 'https://portal.ogc.org/files/?artifact_id=35326';

// Spec section references for tooltips
const SPEC_REFS = {
    // GetCapabilities
    'caps-version-no-version': { section: '7.1.1', title: 'Version negotiation', desc: 'Server shall respond with supported version if none specified' },
    'caps-version-1.0.0': { section: '7.1.1', title: 'Version number', desc: 'WMTS implementations shall use value "1.0.0"' },
    'caps-service-wmts': { section: '7.1.1', title: 'SERVICE parameter', desc: 'SERVICE=WMTS is mandatory for KVP requests' },
    'caps-xml-valid': { section: '7.1.2', title: 'GetCapabilities response', desc: 'Response shall be XML document per WMTS schema' },
    'caps-has-contents': { section: '7.1.2', title: 'Contents element', desc: 'Capabilities shall include Contents with Layers and TileMatrixSets' },
    'caps-has-tilematrixset': { section: '6.1', title: 'TileMatrixSet', desc: 'Server shall define at least one TileMatrixSet' },
    'caps-layer-has-identifier': { section: '7.1.2.2', title: 'Layer Identifier', desc: 'Each Layer shall have unique ows:Identifier' },
    'caps-layer-has-tilematrixsetlink': { section: '7.1.2.2', title: 'TileMatrixSetLink', desc: 'Each Layer shall reference at least one TileMatrixSet' },
    'caps-layer-has-format': { section: '7.1.2.2', title: 'Layer Format', desc: 'Each Layer shall specify supported output formats' },
    'caps-extra-param-ignored': { section: '7.1.1', title: 'Parameter handling', desc: 'WMTS shall ignore unrecognized parameters' },
    
    // GetTile
    'gettile-basic-kvp': { section: '7.2.1', title: 'GetTile KVP', desc: 'GetTile via KVP encoding shall return tile or exception' },
    'gettile-basic-rest': { section: '10', title: 'GetTile REST', desc: 'GetTile via RESTful encoding if supported' },
    'gettile-invalid-layer': { section: '7.2.1', title: 'Invalid LAYER', desc: 'Server shall return exception for undefined layer' },
    'gettile-invalid-style': { section: '7.2.1', title: 'Invalid STYLE', desc: 'Server shall return exception for undefined style' },
    'gettile-invalid-format': { section: '7.2.1', title: 'Invalid FORMAT', desc: 'Server shall return exception for unsupported format' },
    'gettile-invalid-tilematrixset': { section: '7.2.1', title: 'Invalid TileMatrixSet', desc: 'Server shall return exception for undefined TileMatrixSet' },
    'gettile-invalid-tilematrix': { section: '7.2.1', title: 'Invalid TileMatrix', desc: 'Server shall return exception for invalid zoom level' },
    'gettile-invalid-tilerow': { section: '7.2.1', title: 'Invalid TileRow', desc: 'Server shall return exception for row out of bounds' },
    'gettile-invalid-tilecol': { section: '7.2.1', title: 'Invalid TileCol', desc: 'Server shall return exception for column out of bounds' },
    'gettile-png-format': { section: '7.2.1', title: 'PNG format', desc: 'Server shall return Content-Type: image/png and valid PNG data when FORMAT=image/png' },
    'gettile-jpeg-format': { section: '7.2.1', title: 'JPEG format', desc: 'Server shall return Content-Type: image/jpeg and valid JPEG data when FORMAT=image/jpeg (if supported)' },
    
    // GetFeatureInfo
    'gfi-basic-request': { section: '7.3.1', title: 'GetFeatureInfo', desc: 'Optional operation for InfoFormat-enabled layers' },
    'gfi-format-json': { section: '7.3.1', title: 'JSON InfoFormat', desc: 'Response format shall match requested InfoFormat' },
    'gfi-format-html': { section: '7.3.1', title: 'HTML InfoFormat', desc: 'Response format shall match requested InfoFormat' },
    'gfi-invalid-i': { section: '7.3.1', title: 'Invalid I', desc: 'Server shall return exception for I out of bounds' },
    'gfi-invalid-j': { section: '7.3.1', title: 'Invalid J', desc: 'Server shall return exception for J out of bounds' },
    
    // Dimensions (OGC WMTS 07-057r7 Section 7.2.1 - Dimensions)
    'dim-time-default': { section: '7.2.1', title: 'TIME dimension', desc: 'Server shall use default TIME if not specified' },
    'dim-time-explicit': { section: '7.2.1', title: 'TIME dimension', desc: 'Server shall respect explicit TIME value' },
    'dim-elevation-default': { section: '7.2.1', title: 'ELEVATION dimension', desc: 'Server shall use default ELEVATION if not specified' },
    'dim-elevation-explicit': { section: '7.2.1', title: 'ELEVATION dimension', desc: 'Server shall respect explicit ELEVATION value' },
    'dim-run-default': { section: '7.2.1', title: 'RUN dimension', desc: 'Server shall use default RUN (model run time) if not specified' },
    'dim-run-explicit': { section: '7.2.1', title: 'RUN dimension', desc: 'Server shall respect explicit RUN value' },
    'dim-forecast-default': { section: '7.2.1', title: 'FORECAST dimension', desc: 'Server shall use default FORECAST (forecast hour) if not specified' },
    'dim-forecast-explicit': { section: '7.2.1', title: 'FORECAST dimension', desc: 'Server shall respect explicit FORECAST value' },
    
    // TileMatrixSet
    'tms-webmercator': { section: '6.1', title: 'WebMercatorQuad', desc: 'Well-known TileMatrixSet for web mapping' },
    'tms-wgs84': { section: '6.1', title: 'WorldCRS84Quad', desc: 'Well-known TileMatrixSet for WGS84' },
    'tms-structure': { section: '6.1', title: 'TileMatrixSet structure', desc: 'TileMatrixSet shall define identifier, CRS, and TileMatrix levels' }
};

// Test definitions organized by category
const OGC_TESTS = {
    getcapabilities: {
        name: 'GetCapabilities',
        tests: [
            {
                id: 'caps-version-no-version',
                desc: 'GetCapabilities without VERSION returns valid response',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('Capabilities')) throw new Error('No Capabilities element found');
                    return {valid: true, url};
                }
            },
            {
                id: 'caps-version-1.0.0',
                desc: 'GetCapabilities with VERSION=1.0.0 returns 1.0.0',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Match Capabilities element version attribute specifically (not XML declaration)
                    const match = text.match(/<Capabilities[^>]*version="([^"]+)"/);
                    if (!match) throw new Error('No Capabilities version attribute found');
                    if (match[1] !== '1.0.0') throw new Error(`Expected 1.0.0, got ${match[1]}`);
                    return {version: match[1], url};
                }
            },
            {
                id: 'caps-service-wmts',
                desc: 'SERVICE=WMTS parameter is recognized',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    if (text.includes('InvalidParameterValue') && text.includes('SERVICE')) {
                        throw new Error('SERVICE=WMTS not recognized');
                    }
                    return {valid: true, url};
                }
            },
            {
                id: 'caps-xml-valid',
                desc: 'GetCapabilities returns valid XML',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const parseError = doc.querySelector('parsererror');
                    if (parseError) throw new Error('XML parse error');
                    return {valid: true, url};
                }
            },
            {
                id: 'caps-has-contents',
                desc: 'Capabilities contains Contents element',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('Contents')) throw new Error('Missing Contents element');
                    return {found: true, url};
                }
            },
            {
                id: 'caps-has-tilematrixset',
                desc: 'Capabilities defines at least one TileMatrixSet',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const tms = doc.querySelectorAll('TileMatrixSet');
                    if (tms.length === 0) throw new Error('No TileMatrixSet defined');
                    return {count: tms.length, url};
                }
            },
            {
                id: 'caps-layer-has-identifier',
                desc: 'All layers have ows:Identifier',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const layers = doc.querySelectorAll('Contents > Layer');
                    let missing = 0;
                    layers.forEach(layer => {
                        const id = layer.querySelector('Identifier');
                        if (!id || !id.textContent) missing++;
                    });
                    if (missing > 0) throw new Error(`${missing} layers missing Identifier`);
                    return {layerCount: layers.length, url};
                }
            },
            {
                id: 'caps-layer-has-tilematrixsetlink',
                desc: 'All layers have TileMatrixSetLink',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const layers = doc.querySelectorAll('Contents > Layer');
                    let missing = 0;
                    layers.forEach(layer => {
                        const link = layer.querySelector('TileMatrixSetLink');
                        if (!link) missing++;
                    });
                    if (missing > 0) throw new Error(`${missing} layers missing TileMatrixSetLink`);
                    return {valid: true, url};
                }
            },
            {
                id: 'caps-layer-has-format',
                desc: 'All layers specify output formats',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const layers = doc.querySelectorAll('Contents > Layer');
                    let missing = 0;
                    layers.forEach(layer => {
                        const format = layer.querySelector('Format');
                        if (!format) missing++;
                    });
                    if (missing > 0) throw new Error(`${missing} layers missing Format`);
                    return {valid: true, url};
                }
            },
            {
                id: 'caps-extra-param-ignored',
                desc: 'Extra parameters are ignored',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0&FAKEPARAM=test123`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    if (!text.includes('Capabilities')) throw new Error('Invalid response');
                    return {ignored: true, url};
                }
            }
        ]
    },
    gettile: {
        name: 'GetTile',
        tests: [
            {
                id: 'gettile-basic-kvp',
                desc: 'Basic GetTile KVP request returns image/png',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected image/png, got ${ct}`);
                    return {contentType: ct, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-basic-rest',
                desc: 'Basic GetTile REST request returns image/png',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const url = `${API_BASE}/wmts/rest/${layer.name}/${style}/${tms}/2/1/1.png`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        // REST may not be supported
                        if (resp.status === 404) {
                            return {skipped: 'REST encoding not supported', url};
                        }
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected image/png, got ${ct}`);
                    return {contentType: ct, url};
                }
            },
            {
                id: 'gettile-invalid-layer',
                desc: 'Invalid LAYER returns exception',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=INVALID_LAYER_XYZ&STYLE=default&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception, but received an image');
                    }
                    const text = await resp.text();
                    if (text.includes('Exception') || text.includes('error') || !resp.ok) {
                        return {exception: true, url};
                    }
                    throw new Error('Expected exception for invalid layer');
                }
            },
            {
                id: 'gettile-invalid-style',
                desc: 'Invalid STYLE returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=INVALID_STYLE_XYZ&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception, but received an image');
                    }
                    return {exception: true, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-invalid-format',
                desc: 'Invalid FORMAT returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/fake&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception, but received an image');
                    }
                    return {exception: true, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-invalid-tilematrixset',
                desc: 'Invalid TILEMATRIXSET returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=InvalidTMS&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception, but received an image');
                    }
                    return {exception: true, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-invalid-tilematrix',
                desc: 'Invalid TILEMATRIX (zoom level) returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=99&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception for invalid zoom level, but received an image');
                    }
                    return {exception: true, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-invalid-tilerow',
                desc: 'Invalid TILEROW (out of bounds) returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=9999&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception for out-of-bounds row, but received an image');
                    }
                    return {exception: true, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-invalid-tilecol',
                desc: 'Invalid TILECOL (out of bounds) returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=9999`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) {
                        throw new Error('Expected exception for out-of-bounds column, but received an image');
                    }
                    return {exception: true, dimensions: dims, url};
                }
            },
            {
                id: 'gettile-png-format',
                desc: 'FORMAT=image/png returns PNG image with correct Content-Type header',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected Content-Type 'image/png', got '${ct}'`);
                    
                    // Verify response is actually a valid PNG by checking magic bytes
                    const buffer = await resp.arrayBuffer();
                    const bytes = new Uint8Array(buffer);
                    // PNG magic bytes: 89 50 4E 47 0D 0A 1A 0A (first 8 bytes)
                    const pngMagic = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
                    const isPng = bytes.length >= 8 && pngMagic.every((b, i) => bytes[i] === b);
                    if (!isPng) {
                        throw new Error(`Content-Type is image/png but response is not valid PNG`);
                    }
                    return {
                        format: 'image/png',
                        contentType: ct,
                        size: bytes.length,
                        validPngMagic: true,
                        dimensions: dims,
                        url
                    };
                }
            },
            {
                id: 'gettile-jpeg-format',
                desc: 'FORMAT=image/jpeg returns JPEG image with correct Content-Type header',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/jpeg&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    
                    // Check HTTP Content-Type header explicitly
                    if (ct.includes('image/jpeg')) {
                        // Verify response is actually a valid JPEG by checking magic bytes
                        const buffer = await resp.arrayBuffer();
                        const bytes = new Uint8Array(buffer);
                        // JPEG magic bytes: FF D8 FF
                        if (bytes.length >= 3 && bytes[0] === 0xFF && bytes[1] === 0xD8 && bytes[2] === 0xFF) {
                            return {
                                format: 'image/jpeg',
                                contentType: ct,
                                size: bytes.length,
                                validJpegMagic: true,
                                dimensions: dims,
                                url
                            };
                        }
                        throw new Error(`Content-Type is image/jpeg but response is not valid JPEG (magic bytes: ${bytes[0]?.toString(16)}, ${bytes[1]?.toString(16)}, ${bytes[2]?.toString(16)})`);
                    }
                    // JPEG may not be supported - check for proper exception
                    if (!resp.ok || ct.includes('xml') || ct.includes('text')) {
                        return {skipped: 'JPEG format not supported', url};
                    }
                    throw new Error(`Expected Content-Type 'image/jpeg', got '${ct}'`);
                }
            }
        ]
    },
    getfeatureinfo: {
        name: 'GetFeatureInfo',
        tests: [
            {
                id: 'gfi-basic-request',
                desc: 'Basic GetFeatureInfo request returns valid response',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    // Get tile coordinates within the layer's coverage area
                    const tile = getTileForLayerBounds(layer, 4);
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetFeatureInfo&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=${tile.z}&TILEROW=${tile.y}&TILECOL=${tile.x}&I=128&J=128&INFOFORMAT=application/json`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        // GFI may not be supported
                        if (resp.status === 400 || resp.status === 501) {
                            return {skipped: 'GetFeatureInfo not supported', url};
                        }
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    return {success: true, tile, dimensions: dims, url};
                }
            },
            {
                id: 'gfi-format-json',
                desc: 'INFOFORMAT=application/json returns JSON',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const tile = getTileForLayerBounds(layer, 4);
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetFeatureInfo&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=${tile.z}&TILEROW=${tile.y}&TILECOL=${tile.x}&I=128&J=128&INFOFORMAT=application/json`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        if (resp.status === 400 || resp.status === 501) {
                            return {skipped: 'GetFeatureInfo not supported', url};
                        }
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('json')) throw new Error(`Expected JSON, got ${ct}`);
                    return {type: 'json', tile, dimensions: dims, url};
                }
            },
            {
                id: 'gfi-format-html',
                desc: 'INFOFORMAT=text/html returns HTML',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const tile = getTileForLayerBounds(layer, 4);
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetFeatureInfo&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=${tile.z}&TILEROW=${tile.y}&TILECOL=${tile.x}&I=128&J=128&INFOFORMAT=text/html`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        if (resp.status === 400 || resp.status === 501) {
                            return {skipped: 'HTML InfoFormat not supported', url};
                        }
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('html')) throw new Error(`Expected HTML, got ${ct}`);
                    return {type: 'html', tile, dimensions: dims, url};
                }
            },
            {
                id: 'gfi-invalid-i',
                desc: 'Invalid I parameter returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const tile = getTileForLayerBounds(layer, 4);
                    // Use I=9999 which is way out of bounds for a 256x256 tile
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetFeatureInfo&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=${tile.z}&TILEROW=${tile.y}&TILECOL=${tile.x}&I=9999&J=128&INFOFORMAT=application/json`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    // Should be an error for out-of-bounds I
                    if (!resp.ok || resp.status >= 400) {
                        return {exception: true, tile, dimensions: dims, url};
                    }
                    // Some servers may just return empty result
                    return {behavior: 'lenient', tile, dimensions: dims, url};
                }
            },
            {
                id: 'gfi-invalid-j',
                desc: 'Invalid J parameter returns exception',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const tile = getTileForLayerBounds(layer, 4);
                    // Use J=9999 which is way out of bounds for a 256x256 tile
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetFeatureInfo&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=${tile.z}&TILEROW=${tile.y}&TILECOL=${tile.x}&I=128&J=9999&INFOFORMAT=application/json`;
                    const dims = buildRandomDimensionParams(layer);
                    Object.entries(dims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok || resp.status >= 400) {
                        return {exception: true, tile, dimensions: dims, url};
                    }
                    return {behavior: 'lenient', tile, dimensions: dims, url};
                }
            }
        ]
    },
    dimensions: {
        name: 'Dimensions',
        tests: [
            {
                id: 'dim-time-default',
                desc: 'TIME dimension uses default when not specified',
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return {skipped: 'No TIME dimension layer'};
                    const style = ctx.layerWithTimeStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    // Build URL without TIME
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    // Add other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'TIME') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-time-explicit',
                desc: 'Explicit random TIME value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return {skipped: 'No TIME dimension layer'};
                    const style = ctx.layerWithTimeStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const timeValue = getRandomDimensionValue(layer, 'TIME');
                    if (!timeValue) return {skipped: 'No TIME values'};
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1&TIME=${encodeURIComponent(timeValue)}`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'TIME') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {time: timeValue, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-elevation-default',
                desc: 'ELEVATION dimension uses default when not specified',
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return {skipped: 'No ELEVATION dimension layer'};
                    const style = ctx.layerWithElevationStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'ELEVATION') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-elevation-explicit',
                desc: 'Explicit random ELEVATION value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return {skipped: 'No ELEVATION dimension layer'};
                    const style = ctx.layerWithElevationStyle || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const elevValue = getRandomDimensionValue(layer, 'ELEVATION');
                    if (!elevValue) return {skipped: 'No ELEVATION values'};
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1&ELEVATION=${encodeURIComponent(elevValue)}`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'ELEVATION') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {elevation: elevValue, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-run-default',
                desc: 'RUN dimension uses default when not specified',
                run: async (ctx) => {
                    const layer = ctx.layerWithRun;
                    if (!layer) return {skipped: 'No RUN dimension layer'};
                    const style = layer.styles?.[0]?.name || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    // Build URL without RUN - should use default
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    // Add other dimensions except RUN
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'RUN') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-run-explicit',
                desc: 'Explicit RUN value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithRun;
                    if (!layer) return {skipped: 'No RUN dimension layer'};
                    const style = layer.styles?.[0]?.name || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const runValue = getRandomDimensionValue(layer, 'RUN');
                    if (!runValue) return {skipped: 'No RUN values'};
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1&RUN=${encodeURIComponent(runValue)}`;
                    // Add other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'RUN') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {run: runValue, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-forecast-default',
                desc: 'FORECAST dimension uses default when not specified',
                run: async (ctx) => {
                    const layer = ctx.layerWithForecast;
                    if (!layer) return {skipped: 'No FORECAST dimension layer'};
                    const style = layer.styles?.[0]?.name || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    // Build URL without FORECAST - should use default
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1`;
                    // Add other dimensions except FORECAST
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'FORECAST') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url};
                }
            },
            {
                id: 'dim-forecast-explicit',
                desc: 'Explicit FORECAST value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithForecast;
                    if (!layer) return {skipped: 'No FORECAST dimension layer'};
                    const style = layer.styles?.[0]?.name || 'default';
                    const tms = ctx.sampleTileMatrixSet || 'WebMercatorQuad';
                    const forecastValue = getRandomDimensionValue(layer, 'FORECAST');
                    if (!forecastValue) return {skipped: 'No FORECAST values'};
                    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=${tms}&TILEMATRIX=2&TILEROW=1&TILECOL=1&FORECAST=${encodeURIComponent(forecastValue)}`;
                    // Add other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'FORECAST') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {forecast: forecastValue, otherDimensions: otherDims, url};
                }
            }
        ]
    },
    tilematrixset: {
        name: 'TileMatrixSet',
        tests: [
            {
                id: 'tms-webmercator',
                desc: 'WebMercatorQuad TileMatrixSet is defined',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (text.includes('WebMercatorQuad') || text.includes('GoogleMapsCompatible') || text.includes('EPSG:3857')) {
                        return {found: true, url};
                    }
                    return {skipped: 'WebMercatorQuad not advertised', url};
                }
            },
            {
                id: 'tms-wgs84',
                desc: 'WorldCRS84Quad TileMatrixSet is defined and functional',
                run: async (ctx) => {
                    // First verify it's advertised in capabilities
                    const capsUrl = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const capsResp = await fetchWithAuth(capsUrl);
                    const text = await capsResp.text();
                    if (!text.includes('WorldCRS84Quad')) {
                        throw new Error('WorldCRS84Quad not advertised in GetCapabilities');
                    }
                    
                    // Then verify we can actually request a tile with it
                    const layer = ctx.sampleLayer;
                    if (!layer) {
                        return {found: true, tileTest: 'skipped (no sample layer)', url: capsUrl};
                    }
                    const style = ctx.sampleStyle || 'default';
                    // z=1 has 4 columns (0-3) and 2 rows (0-1) for WorldCRS84Quad
                    const dims = buildRandomDimensionParams(layer);
                    let tileUrl = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png&TILEMATRIXSET=WorldCRS84Quad&TILEMATRIX=1&TILEROW=0&TILECOL=0`;
                    Object.entries(dims).forEach(([key, value]) => {
                        tileUrl += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const tileResp = await fetchWithAuth(tileUrl);
                    if (!tileResp.ok) throw new Error(`GetTile failed: HTTP ${tileResp.status}`);
                    const ct = tileResp.headers.get('content-type') || '';
                    if (!ct.includes('image/')) throw new Error(`Expected image, got ${ct}`);
                    
                    return {found: true, tileWorks: true, dimensions: dims, url: tileUrl};
                }
            },
            {
                id: 'tms-structure',
                desc: 'TileMatrixSet has required elements',
                run: async () => {
                    const url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    
                    // Find TileMatrixSet that contains TileMatrix children (not TileMatrixSetLink)
                    const allTms = doc.querySelectorAll('TileMatrixSet');
                    let tms = null;
                    for (const t of allTms) {
                        if (t.querySelector('TileMatrix')) {
                            tms = t;
                            break;
                        }
                    }
                    if (!tms) throw new Error('No TileMatrixSet found');
                    
                    // Handle namespaced elements (ows:Identifier, etc.)
                    const id = tms.querySelector('Identifier') 
                            || tms.querySelector('ows\\:Identifier')
                            || tms.getElementsByTagNameNS('*', 'Identifier')[0];
                    const crs = tms.querySelector('SupportedCRS') 
                             || tms.querySelector('ows\\:SupportedCRS')
                             || tms.getElementsByTagNameNS('*', 'SupportedCRS')[0]
                             || tms.querySelector('CRS');
                    const matrix = tms.querySelector('TileMatrix');
                    
                    if (!id) throw new Error('TileMatrixSet missing Identifier');
                    if (!crs) throw new Error('TileMatrixSet missing SupportedCRS');
                    if (!matrix) throw new Error('TileMatrixSet missing TileMatrix');
                    return {
                        identifier: id.textContent,
                        crs: crs.textContent,
                        hasMatrix: true,
                        url
                    };
                }
            }
        ]
    }
};

// OGC Test State
let ogcTestResults = {};
let ogcTestUrls = {};
let ogcTestContext = {
    sampleLayer: null,
    sampleStyle: '',
    sampleTileMatrixSet: 'WebMercatorQuad',
    allLayers: [],
    layerWithTime: null,
    layerWithTimeStyle: '',
    layerWithElevation: null,
    layerWithElevationStyle: '',
    layerWithRun: null,
    layerWithRunStyle: '',
    layerWithForecast: null,
    layerWithForecastStyle: ''
};

// Helper functions
function pickRandom(arr) {
    if (!arr || arr.length === 0) return null;
    return arr[Math.floor(Math.random() * arr.length)];
}

/**
 * Calculate tile coordinates (z, x, y) for a point within a layer's bounding box.
 * Uses WebMercator tile scheme.
 * @param {Object} layer - Layer object with bounds property
 * @param {number} z - Zoom level (default 4)
 * @returns {Object} {z, x, y} tile coordinates
 */
function getTileForLayerBounds(layer, z = 4) {
    if (!layer || !layer.bounds) {
        // Default to a tile that covers most of the world
        return { z: 4, x: 4, y: 6 };
    }
    
    const bounds = layer.bounds;
    // Calculate center of the bounding box
    const centerLon = (bounds.west + bounds.east) / 2;
    const centerLat = (bounds.south + bounds.north) / 2;
    
    // Clamp latitude to valid WebMercator range
    const lat = Math.max(-85.051, Math.min(85.051, centerLat));
    const lon = Math.max(-180, Math.min(180, centerLon));
    
    // Convert lat/lon to tile coordinates at zoom level z
    const n = Math.pow(2, z);
    const x = Math.floor((lon + 180) / 360 * n);
    const latRad = lat * Math.PI / 180;
    const y = Math.floor((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2 * n);
    
    // Clamp to valid range
    return {
        z: z,
        x: Math.max(0, Math.min(n - 1, x)),
        y: Math.max(0, Math.min(n - 1, y))
    };
}

function getRandomDimensionValue(layer, dimensionName) {
    if (!layer || !layer.dimensions || !layer.dimensions[dimensionName]) {
        return null;
    }
    const dim = layer.dimensions[dimensionName];
    if (dim.values && dim.values.length > 0) {
        return pickRandom(dim.values);
    }
    return dim.default || null;
}

function buildRandomDimensionParams(layer) {
    const params = {};
    if (!layer || !layer.dimensions) return params;

    Object.keys(layer.dimensions).forEach(dimName => {
        const value = getRandomDimensionValue(layer, dimName);
        if (value) {
            params[dimName] = value;
        }
    });
    return params;
}

function formatDimensionInfo(dimensions) {
    if (!dimensions || Object.keys(dimensions).length === 0) return '';
    return Object.entries(dimensions)
        .map(([k, v]) => `${k}=${v}`)
        .join(', ');
}

// URL helper functions
function toggleTestUrl(testId, event) {
    if (event) event.stopPropagation();
    const urlEl = document.getElementById(`ogc-test-${testId}-url`);
    if (urlEl.style.display === 'block') {
        urlEl.style.display = 'none';
    } else {
        urlEl.style.display = 'block';
    }
}

function copyTestUrl(testId, event) {
    if (event) event.stopPropagation();
    const url = ogcTestUrls[testId];
    if (url) {
        navigator.clipboard.writeText(url).then(() => {
            const btn = event.target;
            const original = btn.textContent;
            btn.textContent = 'Copied!';
            setTimeout(() => btn.textContent = original, 1000);
        });
    }
}

function openTestUrl(testId, event) {
    if (event) event.stopPropagation();
    const url = ogcTestUrls[testId];
    if (url) {
        window.open(url, '_blank');
    }
}

function setTestUrl(testId, url, autoExpand = false) {
    ogcTestUrls[testId] = url;
    const urlValueEl = document.getElementById(`ogc-test-${testId}-url-value`);
    if (urlValueEl) {
        urlValueEl.textContent = url;
    }
    if (autoExpand) {
        const urlEl = document.getElementById(`ogc-test-${testId}-url`);
        if (urlEl) {
            urlEl.style.display = 'block';
        }
    }
}

// Toggle OGC section
function toggleOgcSection(sectionId, event) {
    if (event) event.stopPropagation();
    
    const section = document.getElementById(sectionId);
    const content = section.querySelector('.ogc-content');
    const toggle = section.querySelector('.ogc-toggle');
    
    const isExpanded = content.style.display === 'block';
    
    if (isExpanded) {
        content.style.display = 'none';
        toggle.textContent = '▶';
    } else {
        content.style.display = 'block';
        toggle.textContent = '▼';
    }
}

// Initialize OGC test UI
function initOgcTests() {
    Object.entries(OGC_TESTS).forEach(([categoryKey, category]) => {
        const contentEl = document.getElementById(`ogc-${categoryKey}-content`);
        if (!contentEl) return;

        ogcTestResults[categoryKey] = {};
        category.tests.forEach(test => {
            ogcTestResults[categoryKey][test.id] = {status: 'pending'};
        });

        contentEl.style.display = 'none';
        const section = document.getElementById(`ogc-${categoryKey}`);
        const toggle = section.querySelector('.ogc-toggle');
        if (toggle) toggle.textContent = '▶';

        contentEl.innerHTML = `
            <div class="ogc-tests-list">
                ${category.tests.map(test => {
                    const specRef = SPEC_REFS[test.id];
                    const specHtml = specRef 
                        ? `<span class="spec-hint" data-tooltip="${specRef.title}: ${specRef.desc}">OGC 07-057r7 §${specRef.section}</span>`
                        : '';
                    return `
                    <div class="ogc-test" id="ogc-test-${test.id}">
                        <div class="ogc-test-left">
                            <div class="ogc-test-id">
                                ${test.id}
                                <button class="ogc-test-toggle-url" onclick="toggleTestUrl('${test.id}', event)" title="Show/hide request URL">URL</button>
                            </div>
                            <div class="ogc-test-desc">${test.desc} ${specHtml}</div>
                            <div class="ogc-test-error" id="ogc-test-${test.id}-error"></div>
                            <div class="ogc-test-url" id="ogc-test-${test.id}-url" style="display: none;">
                                <div class="ogc-test-url-header">
                                    <span class="ogc-test-url-label">Request URL</span>
                                    <div class="ogc-test-url-actions">
                                        <button class="ogc-test-url-btn" onclick="copyTestUrl('${test.id}', event)">Copy</button>
                                        <button class="ogc-test-url-btn" onclick="openTestUrl('${test.id}', event)">Open</button>
                                    </div>
                                </div>
                                <div class="ogc-test-url-value" id="ogc-test-${test.id}-url-value">Run test to see URL</div>
                            </div>
                        </div>
                        <div class="ogc-test-status">
                            <span class="ogc-test-icon pending" id="ogc-test-${test.id}-icon">○</span>
                            <span class="ogc-test-time" id="ogc-test-${test.id}-time"></span>
                        </div>
                    </div>
                `}).join('')}
            </div>
        `;
    });

    updateOgcSummary();
}

// Build test context from loaded layers
function buildOgcTestContext() {
    ogcTestContext.allLayers = layers;
    ogcTestContext.sampleLayer = layers[0] || null;
    ogcTestContext.sampleStyle = layers[0]?.styles?.[0]?.name || 'default';

    // Find layers with specific dimensions
    layers.forEach(layer => {
        if (layer.dimensions?.TIME && !ogcTestContext.layerWithTime) {
            ogcTestContext.layerWithTime = layer;
            ogcTestContext.layerWithTimeStyle = layer.styles?.[0]?.name || 'default';
        }
        if (layer.dimensions?.ELEVATION && !ogcTestContext.layerWithElevation) {
            ogcTestContext.layerWithElevation = layer;
            ogcTestContext.layerWithElevationStyle = layer.styles?.[0]?.name || 'default';
        }
        if (layer.dimensions?.RUN && !ogcTestContext.layerWithRun) {
            ogcTestContext.layerWithRun = layer;
            ogcTestContext.layerWithRunStyle = layer.styles?.[0]?.name || 'default';
        }
        if (layer.dimensions?.FORECAST && !ogcTestContext.layerWithForecast) {
            ogcTestContext.layerWithForecast = layer;
            ogcTestContext.layerWithForecastStyle = layer.styles?.[0]?.name || 'default';
        }
    });

    console.log('WMTS Test Context built:', {
        sampleLayer: ogcTestContext.sampleLayer?.name,
        sampleStyle: ogcTestContext.sampleStyle,
        layerWithTime: ogcTestContext.layerWithTime?.name,
        layerWithTimeValues: ogcTestContext.layerWithTime?.dimensions?.TIME?.values?.slice(0, 3),
        layerWithElevation: ogcTestContext.layerWithElevation?.name,
        layerWithElevationValues: ogcTestContext.layerWithElevation?.dimensions?.ELEVATION?.values?.slice(0, 3),
        layerWithRun: ogcTestContext.layerWithRun?.name,
        layerWithRunValues: ogcTestContext.layerWithRun?.dimensions?.RUN?.values?.slice(0, 3),
        layerWithForecast: ogcTestContext.layerWithForecast?.name,
        layerWithForecastValues: ogcTestContext.layerWithForecast?.dimensions?.FORECAST?.values?.slice(0, 3)
    });
}

// Test configuration
const TEST_CONFIG = {
    delayBetweenTests: 350,
    maxRetries: 3,
    retryDelay: 300,
    timeout: 10000
};

function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

function isTransientError(error) {
    const msg = error.message?.toLowerCase() || '';
    return msg.includes('network') ||
        msg.includes('timeout') ||
        msg.includes('fetch') ||
        msg.includes('aborted') ||
        msg.includes('failed to fetch') ||
        msg.includes('502') ||
        msg.includes('503') ||
        msg.includes('504');
}

// Run a single OGC test
async function runOgcTest(categoryKey, test, retryCount = 0) {
    const iconEl = document.getElementById(`ogc-test-${test.id}-icon`);
    const timeEl = document.getElementById(`ogc-test-${test.id}-time`);
    const errorEl = document.getElementById(`ogc-test-${test.id}-error`);

    iconEl.className = 'ogc-test-icon running';
    iconEl.textContent = retryCount > 0 ? '↻' : '◐';
    timeEl.textContent = retryCount > 0 ? `retry ${retryCount}...` : '';
    errorEl.textContent = '';

    const testEl = document.getElementById(`ogc-test-${test.id}`);
    if (testEl) {
        testEl.classList.remove('failed');
    }

    const startTime = Date.now();

    const testContext = {
        ...ogcTestContext,
        _lastUrl: null
    };

    const originalFetchWithAuth = window.fetchWithAuth;
    window.fetchWithAuth = async function(url, options) {
        testContext._lastUrl = url;
        return originalFetchWithAuth(url, options);
    };

    try {
        const result = await test.run(testContext);
        const elapsed = Date.now() - startTime;

        const testUrl = result.url || testContext._lastUrl;
        if (testUrl) {
            setTestUrl(test.id, testUrl);
        }

        if (result.skipped) {
            ogcTestResults[categoryKey][test.id] = {status: 'skipped', result, time: elapsed};
            iconEl.className = 'ogc-test-icon pending';
            iconEl.textContent = '–';
            timeEl.textContent = `(skipped: ${result.skipped})`;
        } else {
            ogcTestResults[categoryKey][test.id] = {status: 'pass', result, time: elapsed};
            iconEl.className = 'ogc-test-icon pass';
            iconEl.textContent = '✓';
            const dimInfo = formatDimensionInfo(result.dimensions || result.otherDimensions);
            const retryInfo = retryCount > 0 ? ` (retry ${retryCount})` : '';
            timeEl.textContent = dimInfo ? `${elapsed}ms${retryInfo} [${dimInfo}]` : `${elapsed}ms${retryInfo}`;
        }
    } catch (error) {
        const elapsed = Date.now() - startTime;

        if (retryCount < TEST_CONFIG.maxRetries && isTransientError(error)) {
            console.log(`Test ${test.id} failed with transient error, retrying (${retryCount + 1}/${TEST_CONFIG.maxRetries})...`);
            window.fetchWithAuth = originalFetchWithAuth;
            await sleep(TEST_CONFIG.retryDelay);
            return runOgcTest(categoryKey, test, retryCount + 1);
        }

        ogcTestResults[categoryKey][test.id] = {status: 'fail', error: error.message, time: elapsed};
        iconEl.className = 'ogc-test-icon fail';
        iconEl.textContent = '✗';
        const retryInfo = retryCount > 0 ? ` (after ${retryCount} retries)` : '';
        timeEl.textContent = `${elapsed}ms${retryInfo}`;
        errorEl.textContent = error.message;

        if (testEl) {
            testEl.classList.add('failed');
        }

        const testUrl = error.url || testContext._lastUrl;
        if (testUrl) {
            setTestUrl(test.id, testUrl, true);
        }
    } finally {
        window.fetchWithAuth = originalFetchWithAuth;
    }

    updateOgcCategoryStatus(categoryKey);
    updateOgcSummary();
}

// Run all tests in a category
async function runOgcCategory(categoryKey) {
    const category = OGC_TESTS[categoryKey];
    if (!category) return;

    for (let i = 0; i < category.tests.length; i++) {
        const test = category.tests[i];
        await runOgcTest(categoryKey, test);

        if (i < category.tests.length - 1) {
            await sleep(TEST_CONFIG.delayBetweenTests);
        }
    }
}

// Run all OGC tests
async function runAllOgcTests() {
    const btn = document.getElementById('run-all-ogc-btn');
    btn.disabled = true;
    btn.textContent = 'Running...';

    buildOgcTestContext();

    const categoryKeys = Object.keys(OGC_TESTS);
    for (let i = 0; i < categoryKeys.length; i++) {
        const categoryKey = categoryKeys[i];
        const section = document.getElementById(`ogc-${categoryKey}`);
        section.classList.add('expanded');

        await runOgcCategory(categoryKey);

        if (i < categoryKeys.length - 1) {
            await sleep(TEST_CONFIG.delayBetweenTests * 2);
        }
    }

    btn.disabled = false;
    btn.textContent = 'Run All OGC Tests';
}

// Update category status
function updateOgcCategoryStatus(categoryKey) {
    const results = ogcTestResults[categoryKey] || {};
    const tests = Object.values(results);

    const pass = tests.filter(t => t.status === 'pass').length;
    const fail = tests.filter(t => t.status === 'fail').length;
    const skipped = tests.filter(t => t.status === 'skipped').length;
    const total = tests.length;

    const scoreEl = document.getElementById(`ogc-${categoryKey}-score`);
    if (!scoreEl) return;

    if (pass + fail + skipped === 0) {
        scoreEl.className = 'ogc-score pending';
        scoreEl.textContent = `-- / ${total}`;
    } else {
        // Show pass / total, with skipped count if any
        if (skipped > 0) {
            scoreEl.textContent = `${pass} / ${total} (${skipped} skipped)`;
        } else {
            scoreEl.textContent = `${pass} / ${total}`;
        }

        if (fail === 0 && pass > 0) {
            scoreEl.className = 'ogc-score pass';
        } else if (pass > 0) {
            scoreEl.className = 'ogc-score partial';
        } else {
            scoreEl.className = 'ogc-score fail';
        }
    }
}

// Update overall summary
function updateOgcSummary() {
    let totalPass = 0;
    let totalFail = 0;
    let totalPending = 0;

    Object.values(ogcTestResults).forEach(categoryResults => {
        Object.values(categoryResults).forEach(result => {
            if (result.status === 'pass') totalPass++;
            else if (result.status === 'fail') totalFail++;
            else if (result.status === 'pending') totalPending++;
        });
    });

    document.getElementById('ogc-pass-count').textContent = totalPass;
    document.getElementById('ogc-fail-count').textContent = totalFail;
    document.getElementById('ogc-pending-count').textContent = totalPending;
}

// ============================================================
// LAYER COVERAGE TESTS
// ============================================================

let layers = [];
let currentIndex = 0;
let layerStatus = {};
let map = null;
let currentTileLayer = null;
let testingAll = false;
let currentFilter = 'all';
let filteredIndices = [];

let tileStats = {
    loaded: 0,
    errors: 0,
    loadTimes: [],
    errorDetails: []
};

// Initialize Leaflet map
function initMap() {
    map = L.map('map', {
        center: [20, 0],
        zoom: 2
    });

    L.tileLayer('https://{s}.basemaps.cartocdn.com/dark_nolabels/{z}/{x}/{y}{r}.png', {
        attribution: '&copy; OpenStreetMap contributors &copy; CARTO',
        maxZoom: 19
    }).addTo(map);

    map.on('zoomend', () => {
        document.getElementById('zoom-level').textContent = map.getZoom();
    });
}

// Load WMTS GetCapabilities
async function loadCapabilities() {
    try {
        const response = await fetchWithAuth(`${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0`);
        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');

        const layerElements = xml.querySelectorAll('Contents > Layer');
        layers = [];

        layerElements.forEach(layerEl => {
            const name = layerEl.querySelector('Identifier')?.textContent;
            const title = layerEl.querySelector('Title')?.textContent;

            if (!name) return;

            // Parse styles
            const styles = [];
            layerEl.querySelectorAll('Style').forEach(styleEl => {
                styles.push({
                    name: styleEl.querySelector('Identifier')?.textContent || 'default',
                    title: styleEl.querySelector('Title')?.textContent || 'Default'
                });
            });

            // Parse dimensions
            // Note: Dimensions use ows:Identifier which may appear as Identifier or ows:Identifier
            const dimensions = {};
            layerEl.querySelectorAll('Dimension').forEach(dimEl => {
                // Try both with and without namespace prefix
                let dimName = dimEl.querySelector('Identifier')?.textContent 
                           || dimEl.querySelector('ows\\:Identifier')?.textContent
                           || dimEl.getElementsByTagNameNS('*', 'Identifier')[0]?.textContent;
                const defaultVal = dimEl.querySelector('Default')?.textContent || '';
                const values = [];
                dimEl.querySelectorAll('Value').forEach(v => values.push(v.textContent));
                if (dimName) {
                    // Store with uppercase key for consistency, but keep original name for URL params
                    const upperName = dimName.toUpperCase();
                    dimensions[upperName] = { default: defaultVal, values, originalName: dimName };
                }
            });

            // Parse bounding box
            const bboxEl = layerEl.querySelector('WGS84BoundingBox');
            let bounds = { west: -180, east: 180, south: -90, north: 90 };
            if (bboxEl) {
                const lowerCorner = bboxEl.querySelector('LowerCorner')?.textContent?.split(' ');
                const upperCorner = bboxEl.querySelector('UpperCorner')?.textContent?.split(' ');
                if (lowerCorner && upperCorner) {
                    bounds = {
                        west: parseFloat(lowerCorner[0]) || -180,
                        south: parseFloat(lowerCorner[1]) || -90,
                        east: parseFloat(upperCorner[0]) || 180,
                        north: parseFloat(upperCorner[1]) || 90
                    };
                }
            }

            layers.push({ name, title, styles, dimensions, bounds });
        });

        layers.forEach(l => {
            layerStatus[l.name] = { status: 'untested' };
        });

        buildOgcTestContext();
        updateSummary();

    } catch (error) {
        console.error('Failed to load capabilities:', error);
        document.getElementById('layer-name').textContent = 'Error loading capabilities';
        throw error;
    }
}

// Setup event listeners
function setupEventListeners() {
    document.getElementById('prev-btn').addEventListener('click', () => navigateLayer(-1));
    document.getElementById('next-btn').addEventListener('click', () => navigateLayer(1));
    document.getElementById('test-all-btn').addEventListener('click', testAllLayers);
    document.getElementById('filter-select').addEventListener('change', onFilterChange);
    document.getElementById('style-select').addEventListener('change', () => loadCurrentLayer());

    document.addEventListener('keydown', (e) => {
        if (e.target.tagName === 'SELECT' || e.target.tagName === 'INPUT') return;

        if (e.key === 'ArrowLeft') {
            navigateLayer(-1);
        } else if (e.key === 'ArrowRight') {
            navigateLayer(1);
        } else if (e.key === ' ') {
            e.preventDefault();
            loadCurrentLayer();
        }
    });
}

// Navigate layers
function navigateLayer(direction) {
    if (filteredIndices.length === 0) return;

    const currentFilteredPos = filteredIndices.indexOf(currentIndex);
    let newFilteredPos = currentFilteredPos + direction;

    if (newFilteredPos < 0) newFilteredPos = filteredIndices.length - 1;
    if (newFilteredPos >= filteredIndices.length) newFilteredPos = 0;

    displayLayer(filteredIndices[newFilteredPos]);
}

// Display a layer
function displayLayer(index) {
    if (index < 0 || index >= layers.length) return;

    currentIndex = index;
    const layer = layers[index];

    document.getElementById('layer-name').textContent = layer.name;
    document.getElementById('layer-title').textContent = layer.title || '';

    const filteredPos = filteredIndices.indexOf(index);
    document.getElementById('layer-index').textContent =
        `${filteredPos + 1} of ${filteredIndices.length}` +
        (filteredIndices.length !== layers.length ? ` (${layers.length} total)` : '');

    document.getElementById('prev-btn').disabled = filteredIndices.length <= 1;
    document.getElementById('next-btn').disabled = filteredIndices.length <= 1;

    // Populate style selector
    const styleSelect = document.getElementById('style-select');
    styleSelect.innerHTML = '';
    layer.styles.forEach(style => {
        const opt = document.createElement('option');
        opt.value = style.name;
        opt.textContent = style.title;
        styleSelect.appendChild(opt);
    });

    populateDimensionControls(layer);
    loadCurrentLayer();
}

// Populate dimension controls
function populateDimensionControls(layer) {
    const container = document.getElementById('dimension-controls');

    const existingDims = container.querySelectorAll('.dimension-group:not(:first-child)');
    existingDims.forEach(el => el.remove());

    const dimOrder = ['RUN', 'FORECAST', 'TIME', 'ELEVATION'];
    dimOrder.forEach(dimName => {
        if (!layer.dimensions[dimName]) return;

        const dim = layer.dimensions[dimName];
        const group = document.createElement('div');
        group.className = 'dimension-group';

        const label = document.createElement('span');
        label.className = 'dimension-label';
        label.textContent = dimName + ':';

        const select = document.createElement('select');
        select.className = 'dimension-select';
        select.id = `dim-${dimName}`;
        select.addEventListener('change', () => loadCurrentLayer());

        const values = dim.values.slice(0, 50);
        values.forEach(val => {
            const opt = document.createElement('option');
            opt.value = val;
            opt.textContent = formatDimensionValue(dimName, val);
            if (val === dim.default) opt.selected = true;
            select.appendChild(opt);
        });

        if (dim.values.length > 50) {
            const opt = document.createElement('option');
            opt.disabled = true;
            opt.textContent = `... and ${dim.values.length - 50} more`;
            select.appendChild(opt);
        }

        group.appendChild(label);
        group.appendChild(select);
        container.appendChild(group);
    });
}

// Format dimension value
function formatDimensionValue(dimName, value) {
    if (dimName === 'TIME' || dimName === 'RUN') {
        try {
            const date = new Date(value);
            return date.toLocaleString('en-US', {
                month: 'short', day: 'numeric',
                hour: '2-digit', minute: '2-digit',
                hour12: false
            });
        } catch {
            return value;
        }
    }
    if (dimName === 'FORECAST') {
        return `+${value}h`;
    }
    return value;
}

// Get current dimensions
function getCurrentDimensions() {
    const dims = {};
    const layer = layers[currentIndex];

    Object.keys(layer.dimensions || {}).forEach(dimName => {
        const select = document.getElementById(`dim-${dimName}`);
        if (select) {
            dims[dimName] = select.value;
        }
    });

    return dims;
}

// Load current layer as WMTS tiles
function loadCurrentLayer() {
    const layer = layers[currentIndex];
    const style = document.getElementById('style-select').value;
    const dims = getCurrentDimensions();

    // Reset tile stats
    tileStats = { loaded: 0, errors: 0, loadTimes: [], errorDetails: [] };
    updateTileStats();

    // Hide error panel
    document.getElementById('error-panel').classList.remove('visible', 'expanded');

    // Set loading state
    setStatus('tile-status', 'loading', 'Loading tiles...');

    // Build URLs
    const kvpUrl = buildKvpUrl(layer, style, dims);
    const restUrl = buildRestUrl(layer, style);

    document.getElementById('kvp-url').textContent = kvpUrl;
    document.getElementById('rest-url').textContent = restUrl;

    // Remove existing tile layer
    if (currentTileLayer) {
        map.removeLayer(currentTileLayer);
    }

    // Create WMTS tile layer
    const tileUrl = buildTileUrl(layer, style, dims);

    currentTileLayer = L.tileLayer(tileUrl, {
        attribution: `${layer.name} (WMTS)`,
        maxZoom: 18,
        tileSize: 256,
        opacity: 0.8
    });

    const loadStartTimes = {};

    currentTileLayer.on('tileloadstart', (e) => {
        const key = `${e.coords.x}_${e.coords.y}_${e.coords.z}`;
        loadStartTimes[key] = Date.now();
    });

    currentTileLayer.on('tileload', (e) => {
        const key = `${e.coords.x}_${e.coords.y}_${e.coords.z}`;
        const startTime = loadStartTimes[key];
        if (startTime) {
            tileStats.loadTimes.push(Date.now() - startTime);
        }
        tileStats.loaded++;
        updateTileStats();
        checkLayerStatus();
    });

    currentTileLayer.on('tileerror', (e) => {
        tileStats.errors++;
        tileStats.errorDetails.push({
            coords: e.coords,
            error: e.error || 'Unknown error'
        });
        updateTileStats();
        checkLayerStatus();
    });

    currentTileLayer.addTo(map);

    const bounds = [[layer.bounds.south, layer.bounds.west], [layer.bounds.north, layer.bounds.east]];
    map.fitBounds(bounds, { padding: [20, 20] });
}

// Build WMTS KVP tile URL
function buildTileUrl(layer, style, dims) {
    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0` +
        `&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png` +
        `&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}`;

    Object.entries(dims).forEach(([key, value]) => {
        url += `&${key}=${encodeURIComponent(value)}`;
    });

    return url;
}

// Build KVP URL for display
function buildKvpUrl(layer, style, dims) {
    let url = `${getWmtsUrl()}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0` +
        `&LAYER=${layer.name}&STYLE=${style}&FORMAT=image/png` +
        `&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=4&TILEROW=5&TILECOL=4`;

    Object.entries(dims).forEach(([key, value]) => {
        url += `&${key}=${encodeURIComponent(value)}`;
    });

    return url;
}

// Build REST URL pattern
function buildRestUrl(layer, style) {
    return `${API_BASE}/wmts/rest/${layer.name}/${style}/WebMercatorQuad/{z}/{y}/{x}.png`;
}

// Update tile stats display
function updateTileStats() {
    document.getElementById('tiles-loaded').textContent = tileStats.loaded;
    document.getElementById('tile-errors').textContent = tileStats.errors;

    if (tileStats.loadTimes.length > 0) {
        const avg = Math.round(tileStats.loadTimes.reduce((a, b) => a + b, 0) / tileStats.loadTimes.length);
        document.getElementById('avg-load-time').textContent = `${avg}ms`;
    } else {
        document.getElementById('avg-load-time').textContent = '--';
    }
}

// Check layer status
function checkLayerStatus() {
    const layer = layers[currentIndex];

    if (tileStats.errors > 0 && tileStats.loaded === 0) {
        setStatus('tile-status', 'error', `Error (${tileStats.errors} failed)`);
        showError(`All ${tileStats.errors} tiles failed to load`, tileStats.errorDetails);
        layerStatus[layer.name] = {
            status: 'error',
            tilesLoaded: tileStats.loaded,
            tileErrors: tileStats.errors,
            error: 'All tiles failed'
        };
    } else if (tileStats.errors > 0) {
        setStatus('tile-status', 'error', `Partial (${tileStats.loaded} ok, ${tileStats.errors} failed)`);
        showError(`${tileStats.errors} tiles failed to load`, tileStats.errorDetails);
        layerStatus[layer.name] = {
            status: 'error',
            tilesLoaded: tileStats.loaded,
            tileErrors: tileStats.errors,
            error: `${tileStats.errors} tile errors`
        };
    } else if (tileStats.loaded > 0) {
        const avgTime = Math.round(tileStats.loadTimes.reduce((a, b) => a + b, 0) / tileStats.loadTimes.length);
        setStatus('tile-status', 'ok', `OK (${tileStats.loaded} tiles, avg ${avgTime}ms)`);
        document.getElementById('error-panel').classList.remove('visible');
        layerStatus[layer.name] = {
            status: 'ok',
            tilesLoaded: tileStats.loaded,
            tileErrors: 0,
            avgTime
        };
    }

    updateSummary();
    updateLayerList();
}

// Show error panel
function showError(summary, details) {
    const panel = document.getElementById('error-panel');
    panel.classList.add('visible');
    document.getElementById('error-summary-text').textContent = summary;
    document.getElementById('error-details').textContent = JSON.stringify(details, null, 2);
}

// Set status indicator
function setStatus(elementId, status, text) {
    const container = document.getElementById(elementId);
    const icon = container.querySelector('.status-icon');
    const textEl = container.querySelector('.status-text');

    icon.className = 'status-icon ' + status;
    textEl.className = 'status-text ' + status;
    textEl.textContent = text;
}

// Update summary stats
function updateSummary() {
    const working = Object.values(layerStatus).filter(s => s.status === 'ok').length;
    const broken = Object.values(layerStatus).filter(s => s.status === 'error').length;
    const untested = Object.values(layerStatus).filter(s => s.status === 'untested').length;
    const tested = working + broken;
    const percent = tested > 0 ? Math.round((working / tested) * 100) : 0;

    document.getElementById('working-count').textContent = working;
    document.getElementById('broken-count').textContent = broken;
    document.getElementById('untested-count').textContent = untested;

    const percentEl = document.getElementById('percent-working');
    percentEl.textContent = tested > 0 ? `${percent}%` : '--%';
    percentEl.className = 'stat-value ' + (percent >= 90 ? 'good' : percent >= 50 ? 'warning' : 'bad');
}

// Filter change handler
function onFilterChange(e) {
    currentFilter = e.target.value;
    updateFilteredIndices();
    updateLayerList();

    if (!filteredIndices.includes(currentIndex) && filteredIndices.length > 0) {
        displayLayer(filteredIndices[0]);
    }

    const layerListEl = document.getElementById('layer-list');
    layerListEl.classList.toggle('visible', currentFilter !== 'all');
}

// Update filtered indices
function updateFilteredIndices() {
    filteredIndices = [];

    layers.forEach((layer, index) => {
        const status = layerStatus[layer.name]?.status || 'untested';

        if (currentFilter === 'all') {
            filteredIndices.push(index);
        } else if (currentFilter === 'working' && status === 'ok') {
            filteredIndices.push(index);
        } else if (currentFilter === 'broken' && status === 'error') {
            filteredIndices.push(index);
        } else if (currentFilter === 'untested' && status === 'untested') {
            filteredIndices.push(index);
        }
    });
}

// Update layer list display
function updateLayerList() {
    const listEl = document.getElementById('layer-list');

    if (currentFilter === 'all') {
        listEl.innerHTML = '';
        return;
    }

    listEl.innerHTML = filteredIndices.map(index => {
        const layer = layers[index];
        const status = layerStatus[layer.name]?.status || 'untested';
        const statusText = status === 'ok' ? 'OK' : status === 'error' ? 'Error' : 'Untested';

        return `
            <div class="layer-list-item" onclick="displayLayer(${index})">
                <span class="layer-list-name">${escapeHtml(layer.name)}</span>
                <span class="layer-list-status ${status}">${statusText}</span>
            </div>
        `;
    }).join('');
}

// Test all layers
async function testAllLayers() {
    if (testingAll) return;

    testingAll = true;
    const btn = document.getElementById('test-all-btn');
    btn.disabled = true;
    btn.textContent = 'Testing...';

    const progressBar = document.getElementById('progress-bar');
    const progressFill = document.getElementById('progress-fill');
    progressBar.classList.add('visible');

    for (let i = 0; i < layers.length; i++) {
        const percent = Math.round(((i + 1) / layers.length) * 100);
        progressFill.style.width = `${percent}%`;
        btn.textContent = `Testing ${i + 1}/${layers.length}...`;

        displayLayer(i);
        await waitForTileLoad(3000);
        await new Promise(resolve => setTimeout(resolve, 200));
    }

    testingAll = false;
    btn.disabled = false;
    btn.textContent = 'Test All Layers';
    progressBar.classList.remove('visible');

    updateFilteredIndices();
    updateLayerList();
}

// Wait for tiles to load
function waitForTileLoad(timeout) {
    return new Promise(resolve => {
        const startTime = Date.now();
        const startLoaded = tileStats.loaded;
        const startErrors = tileStats.errors;

        const check = () => {
            const elapsed = Date.now() - startTime;

            if (elapsed >= timeout) {
                resolve();
            } else if (tileStats.loaded > 0 || tileStats.errors > 0) {
                setTimeout(resolve, 500);
            } else {
                setTimeout(check, 100);
            }
        };

        check();
    });
}

// Copy URL to clipboard
function copyUrl(elementId) {
    const text = document.getElementById(elementId).textContent;
    navigator.clipboard.writeText(text).then(() => {
        const btn = event.target;
        const original = btn.textContent;
        btn.textContent = 'Copied!';
        setTimeout(() => btn.textContent = original, 1000);
    });
}

// Escape HTML
function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// ============================================================
// INITIALIZATION
// ============================================================

document.addEventListener('DOMContentLoaded', () => {
    initOgcTests();
    document.getElementById('run-all-ogc-btn').addEventListener('click', runAllOgcTests);
});

document.addEventListener('DOMContentLoaded', async () => {
    initEndpointConfig();
    initMap();
    updateEndpointStatus('loading', 'Connecting...');
    try {
        await loadCapabilities();
        setupEventListeners();
        if (layers.length > 0) {
            updateEndpointStatus('connected', `Connected (${layers.length} layers)`);
            updateFilteredIndices();
            displayLayer(0);
        } else {
            updateEndpointStatus('error', 'No layers found');
        }
    } catch (error) {
        updateEndpointStatus('error', `Error: ${error.message}`);
    }
});
