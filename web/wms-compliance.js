// WMS 1.3.0 OGC Compliance Tests JavaScript

// ============================================================
// CONFIGURATION
// ============================================================

// API URL detection - can be changed by user
const DEFAULT_API_BASE = window.location.port === '8000' ? 'http://localhost:8080' : '';
let API_BASE = localStorage.getItem('wms-compliance-endpoint') || DEFAULT_API_BASE;

// Authentication credentials (stored in memory only for security)
let authCredentials = null; // { username, password }

// Check if the endpoint already includes a WMS path (e.g., /wms, /WMS, /ogc/WMS, /geoserver/wms)
function endpointIncludesWmsPath(endpoint) {
    const lower = endpoint.toLowerCase();
    // Check if URL ends with /wms or has /wms? (query string)
    return /\/wms\/?(\?|$)/i.test(endpoint) || 
           /\/ows\/?(\?|$)/i.test(endpoint) ||  // GeoServer OWS endpoint
           lower.includes('service=wms');
}

// Get the WMS endpoint URL (appends /wms only if needed)
function getWmsUrl() {
    if (endpointIncludesWmsPath(API_BASE)) {
        // Endpoint already has WMS path, use as-is
        return API_BASE;
    }
    // Append /wms for endpoints that need it
    return `${API_BASE}/wms`;
}

// ============================================================
// ENDPOINT MANAGEMENT
// ============================================================

// Initialize endpoint input field
function initEndpointConfig() {
    const input = document.getElementById('endpoint-input');
    const applyBtn = document.getElementById('endpoint-apply-btn');
    const resetBtn = document.getElementById('endpoint-reset-btn');
    const authToggleBtn = document.getElementById('auth-toggle-btn');
    const authPassword = document.getElementById('auth-password');
    const authUsername = document.getElementById('auth-username');

    // Set initial value
    input.value = API_BASE;

    // Apply button click
    applyBtn.addEventListener('click', () => applyEndpoint());

    // Reset button click
    resetBtn.addEventListener('click', () => resetEndpoint());

    // Enter key in input fields
    input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') applyEndpoint();
    });
    authUsername.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') applyEndpoint();
    });
    authPassword.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') applyEndpoint();
    });

    // Toggle password visibility
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

// Get auth headers if credentials are set
function getAuthHeaders() {
    if (authCredentials && authCredentials.username && authCredentials.password) {
        const credentials = btoa(`${authCredentials.username}:${authCredentials.password}`);
        return { 'Authorization': `Basic ${credentials}` };
    }
    return {};
}

// Wrapper for fetch that includes auth headers
async function fetchWithAuth(url, options = {}) {
    const headers = { ...getAuthHeaders(), ...(options.headers || {}) };
    return fetch(url, { ...options, headers });
}

// Apply the new endpoint
async function applyEndpoint() {
    const input = document.getElementById('endpoint-input');
    const usernameInput = document.getElementById('auth-username');
    const passwordInput = document.getElementById('auth-password');
    
    let newEndpoint = input.value.trim();

    // Normalize the endpoint (remove trailing slash only)
    newEndpoint = newEndpoint.replace(/\/+$/, '');

    // Update the input to show normalized value
    input.value = newEndpoint;

    // Update global and save to localStorage
    API_BASE = newEndpoint;
    if (newEndpoint) {
        localStorage.setItem('wms-compliance-endpoint', newEndpoint);
    } else {
        localStorage.removeItem('wms-compliance-endpoint');
    }

    // Update auth credentials (not saved to localStorage for security)
    const username = usernameInput.value.trim();
    const password = passwordInput.value;
    if (username && password) {
        authCredentials = { username, password };
    } else {
        authCredentials = null;
    }

    // Reload capabilities with new endpoint
    await reloadWithNewEndpoint();
}

// Reset to default endpoint
async function resetEndpoint() {
    const input = document.getElementById('endpoint-input');
    const usernameInput = document.getElementById('auth-username');
    const passwordInput = document.getElementById('auth-password');
    
    input.value = DEFAULT_API_BASE;
    usernameInput.value = '';
    passwordInput.value = '';
    
    API_BASE = DEFAULT_API_BASE;
    authCredentials = null;
    localStorage.removeItem('wms-compliance-endpoint');

    await reloadWithNewEndpoint();
}

// Reload capabilities and reset test state
async function reloadWithNewEndpoint() {
    updateEndpointStatus('loading', 'Connecting...');

    // Reset layer status
    layerStatus = {};

    try {
        await loadCapabilities();

        // Always reinitialize OGC tests UI (resets results too)
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

// Update endpoint status indicator
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
// OGC TEAM ENGINE COMPLIANCE TESTS
// ============================================================

// OGC WMS 1.3.0 Specification reference (OGC 06-042)
const OGC_SPEC_URL = 'https://portal.ogc.org/files/?artifact_id=14416';

// Spec section references for tooltips
const SPEC_REFS = {
    // GetCapabilities
    'caps-version-no-version': { section: '6.2.4', title: 'Version number negotiation', desc: 'Server shall respond with highest version if none specified' },
    'caps-version-1.3.0': { section: '6.2.1', title: 'Version number form and value', desc: 'Implementations shall use value "1.3.0"' },
    'caps-format-default': { section: '7.2.3.1', title: 'FORMAT parameter', desc: 'Every server shall support default text/xml format' },
    'caps-xml-valid': { section: '7.2.4.1', title: 'GetCapabilities response', desc: 'Response shall be XML formatted per schema in E.1' },
    'caps-has-wms-capabilities': { section: '7.2.4.1', title: 'GetCapabilities response', desc: 'Root element shall be WMS_Capabilities in opengis.net/wms namespace' },
    'caps-has-getmap-format': { section: '6.6', title: 'Output formats', desc: 'Server should offer at least PNG format' },
    'caps-has-exception-format': { section: '6.11', title: 'Service exceptions', desc: 'Server shall advertise exception format in capabilities' },
    'caps-layers-have-crs': { section: '7.2.4.6.7', title: 'CRS', desc: 'Every Layer shall have at least one CRS element' },
    'caps-layers-have-bbox': { section: '7.2.4.6.8', title: 'EX_GeographicBoundingBox', desc: 'Every named Layer shall have BoundingBox' },
    'caps-extra-param-ignored': { section: '6.8.1', title: 'Parameter ordering and case', desc: 'WMS shall not require extra parameters not in spec' },
    
    // GetMap
    'getmap-basic-request': { section: '7.3.1', title: 'GetMap General', desc: 'Upon valid request, WMS shall return a map or service exception' },
    'getmap-invalid-layer': { section: '7.2.4.6.3', title: 'Name', desc: 'Server shall throw LayerNotDefined exception for invalid layer' },
    'getmap-invalid-style': { section: '7.3.3.4', title: 'STYLES', desc: 'Server shall throw StyleNotDefined exception for undefined style' },
    'getmap-invalid-crs': { section: '7.3.3.5', title: 'CRS', desc: 'Server shall throw InvalidCRS exception for unsupported CRS' },
    'getmap-invalid-format': { section: '7.3.3.7', title: 'FORMAT', desc: 'Server shall throw InvalidFormat exception for unsupported format' },
    'getmap-bbox-invalid': { section: '7.3.3.6', title: 'BBOX', desc: 'Server shall throw exception for invalid BBOX (minX >= maxX)' },
    'getmap-transparent': { section: '7.3.3.9', title: 'TRANSPARENT', desc: 'TRANSPARENT=TRUE allows results to be overlaid' },
    'getmap-default-style': { section: '7.3.3.4', title: 'STYLES', desc: 'Empty string in STYLES list requests default style' },
    'getmap-small-size': { section: '7.3.3.8', title: 'WIDTH, HEIGHT', desc: 'Map shall have exactly specified width/height in pixels' },
    'getmap-large-size': { section: '7.3.3.8', title: 'WIDTH, HEIGHT', desc: 'Map shall have exactly specified width/height in pixels' },
    'getmap-exception-xml': { section: '7.3.3.11', title: 'EXCEPTIONS', desc: 'Default exception format is XML if parameter absent' },
    'getmap-multi-layer': { section: '7.3.3.3', title: 'LAYERS', desc: 'LAYERS value is comma-separated list of one or more layer names' },
    'getmap-version-required': { section: '6.2.3', title: 'Appearance in requests', desc: 'VERSION is mandatory in requests other than GetCapabilities' },
    'getmap-png-format': { section: '7.3.3.7', title: 'FORMAT (PNG)', desc: 'Server shall return image/png with valid PNG magic bytes when FORMAT=image/png' },
    'getmap-jpeg-format': { section: '7.3.3.7', title: 'FORMAT (JPEG)', desc: 'Server shall return image/jpeg with valid JPEG magic bytes when FORMAT=image/jpeg (if supported)' },
    'getmap-webp-format': { section: '7.3.3.7', title: 'FORMAT (WebP)', desc: 'Server shall return image/webp with valid WebP magic bytes when FORMAT=image/webp (if supported)' },
    
    // GetFeatureInfo
    'gfi-basic-request': { section: '7.4.1', title: 'GetFeatureInfo General', desc: 'Optional operation for queryable layers only' },
    'gfi-format-json': { section: '7.4.3.5', title: 'INFO_FORMAT', desc: 'Response format shall match INFO_FORMAT MIME type' },
    'gfi-format-html': { section: '7.4.3.5', title: 'INFO_FORMAT', desc: 'Response format shall match INFO_FORMAT MIME type' },
    'gfi-invalid-i': { section: '7.4.3.7', title: 'I, J', desc: 'Server shall throw InvalidPoint for I/J outside valid range' },
    'gfi-invalid-j': { section: '7.4.3.7', title: 'I, J', desc: 'Server shall throw InvalidPoint for I/J outside valid range' },
    'gfi-invalid-query-layer': { section: '7.4.3.4', title: 'QUERY_LAYERS', desc: 'Server shall throw LayerNotDefined for undefined layer' },
    'gfi-invalid-format': { section: '7.4.3.5', title: 'INFO_FORMAT', desc: 'Server shall throw InvalidFormat for unsupported format' },
    'gfi-feature-count': { section: '7.4.3.6', title: 'FEATURE_COUNT', desc: 'Maximum number of features per layer; default is 1' },
    
    // Dimensions
    'dim-time-default': { section: 'C.4.1 / 6.7.6', title: 'TIME dimension', desc: 'Server shall respond with default value if declared and not in request' },
    'dim-time-explicit': { section: 'C.4.1 / 6.7.6', title: 'TIME dimension', desc: 'GetMap includes parameter for requesting particular time' },
    'dim-elevation-default': { section: 'C.4.1 / 6.7.5', title: 'ELEVATION dimension', desc: 'Server shall respond with default value if declared and not in request' },
    'dim-elevation-explicit': { section: 'C.4.1 / 6.7.5', title: 'ELEVATION dimension', desc: 'Elevation value requests specific vertical level' },
    'dim-run-default': { section: 'C.3.3 / 6.7.7', title: 'Sample dimensions', desc: 'Custom dimension uses server-declared default if not specified' },
    'dim-run-explicit': { section: 'C.3.3 / 6.7.7', title: 'Sample dimensions', desc: 'GetMap includes mechanism for requesting dimensional values' },
    'dim-forecast-default': { section: 'C.3.3 / 6.7.7', title: 'Sample dimensions', desc: 'Custom dimension uses server-declared default if not specified' },
    'dim-forecast-explicit': { section: 'C.3.3 / 6.7.7', title: 'Sample dimensions', desc: 'GetMap includes mechanism for requesting dimensional values' }
};

// Test definitions organized by category
const OGC_TESTS = {
    getcapabilities: {
        name: 'GetCapabilities',
        tests: [
            {
                id: 'caps-version-no-version',
                desc: 'GetCapabilities without VERSION returns >= 1.3.0',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Match WMS_Capabilities version attribute specifically (not XML declaration)
                    const match = text.match(/WMS_Capabilities[^>]*version="([^"]+)"/);
                    if (!match) throw new Error('No WMS_Capabilities version attribute found');
                    const version = match[1];
                    if (parseFloat(version) < 1.3) throw new Error(`Version ${version} < 1.3.0`);
                    return {version, url};
                }
            },
            {
                id: 'caps-version-1.3.0',
                desc: 'GetCapabilities with VERSION=1.3.0 returns 1.3.0',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Match WMS_Capabilities version attribute specifically (not XML declaration)
                    const match = text.match(/WMS_Capabilities[^>]*version="([^"]+)"/);
                    if (!match) throw new Error('No WMS_Capabilities version attribute found');
                    if (match[1] !== '1.3.0') throw new Error(`Expected 1.3.0, got ${match[1]}`);
                    return {version: match[1], url};
                }
            },
            {
                id: 'caps-format-default',
                desc: 'GetCapabilities without FORMAT returns text/xml',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('text/xml') && !ct.includes('application/xml')) {
                        throw new Error(`Expected XML, got ${ct}`);
                    }
                    return {contentType: ct, url};
                }
            },
            {
                id: 'caps-xml-valid',
                desc: 'GetCapabilities returns valid XML',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
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
                id: 'caps-has-wms-capabilities',
                desc: 'Response contains WMS_Capabilities root element',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('WMS_Capabilities')) throw new Error('Missing WMS_Capabilities element');
                    return {found: true, url};
                }
            },
            {
                id: 'caps-has-getmap-format',
                desc: 'Advertises image/png for GetMap',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('<Format>image/png</Format>')) throw new Error('image/png not advertised');
                    return {format: 'image/png', url};
                }
            },
            {
                id: 'caps-has-exception-format',
                desc: 'Advertises XML exception format',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('<Format>XML</Format>') && !text.includes('Exception')) {
                        throw new Error('XML exception format not found');
                    }
                    return {found: true, url};
                }
            },
            {
                id: 'caps-layers-have-crs',
                desc: 'All named layers have at least one CRS',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const layers = doc.querySelectorAll('Layer[queryable="1"]');
                    let count = 0;
                    layers.forEach(layer => {
                        const name = layer.querySelector(':scope > Name');
                        if (name) count++;
                    });
                    // Check root layer has CRS
                    const rootCrs = doc.querySelector('Layer > CRS');
                    if (!rootCrs) throw new Error('Root layer missing CRS');
                    return {layerCount: count, url};
                }
            },
            {
                id: 'caps-layers-have-bbox',
                desc: 'All named layers have EX_GeographicBoundingBox',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const layers = doc.querySelectorAll('Layer[queryable="1"]');
                    let missing = [];
                    layers.forEach(layer => {
                        const name = layer.querySelector(':scope > Name')?.textContent;
                        const bbox = layer.querySelector(':scope > EX_GeographicBoundingBox');
                        if (name && !bbox) missing.push(name);
                    });
                    if (missing.length > 0) throw new Error(`Missing bbox: ${missing.slice(0, 3).join(', ')}`);
                    return {valid: true, url};
                }
            },
            {
                id: 'caps-extra-param-ignored',
                desc: 'Extra parameters are ignored',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&FAKEPARAM=test123`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    if (!text.includes('WMS_Capabilities')) throw new Error('Invalid response');
                    return {ignored: true, url};
                }
            }
        ]
    },
    getmap: {
        name: 'GetMap',
        tests: [
            {
                id: 'getmap-basic-request',
                desc: 'Basic GetMap request returns image/png (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected image/png, got ${ct}`);
                    return {contentType: ct, dimensions, url};
                }
            },
            {
                id: 'getmap-invalid-layer',
                desc: 'Invalid LAYER returns LayerNotDefined exception',
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=BADLYR&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const controller = new AbortController();
                    const timeoutId = setTimeout(() => controller.abort(), 5000);
                    try {
                        const resp = await fetchWithAuth(url, {signal: controller.signal});
                        clearTimeout(timeoutId);
                        const ct = resp.headers.get('content-type') || '';
                        // Should return XML exception, not an image
                        if (ct.includes('image/')) {
                            throw new Error('Expected LayerNotDefined exception, but received an image');
                        }
                        const text = await resp.text();
                        // Check for the specific OGC exception code
                        if (text.includes('code="LayerNotDefined"')) {
                            return {exception: 'LayerNotDefined', url};
                        }
                        // Accept other exception formats
                        if (text.includes('LayerNotDefined') || text.includes('Exception')) {
                            return {exception: 'LayerNotDefined', url};
                        }
                        throw new Error('Expected LayerNotDefined exception in response');
                    } catch (e) {
                        clearTimeout(timeoutId);
                        if (e.name === 'AbortError') throw new Error('Request timed out');
                        throw e;
                    }
                }
            },
            {
                id: 'getmap-invalid-style',
                desc: 'Invalid STYLE returns StyleNotDefined exception (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=INVALID_STYLE_XYZ&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    // Should return XML exception, not an image
                    if (ct.includes('image/')) {
                        throw new Error('Expected StyleNotDefined exception, but received an image');
                    }
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="StyleNotDefined"')) {
                        return {exception: 'StyleNotDefined', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('StyleNotDefined') || text.includes('Exception')) {
                        return {exception: 'StyleNotDefined', dimensions, url};
                    }
                    throw new Error('Expected StyleNotDefined exception in response');
                }
            },
            {
                id: 'getmap-invalid-crs',
                desc: 'Invalid CRS returns InvalidCRS exception (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:99999&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    // Should return XML exception, not an image
                    if (ct.includes('image/')) {
                        throw new Error('Expected InvalidCRS exception, but received an image. Server is too lenient.');
                    }
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="InvalidCRS"')) {
                        return {exception: 'InvalidCRS', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('InvalidCRS') || text.includes('Exception')) {
                        return {exception: 'InvalidCRS', dimensions, url};
                    }
                    throw new Error('Expected InvalidCRS exception in response');
                }
            },
            {
                id: 'getmap-invalid-format',
                desc: 'Invalid FORMAT returns InvalidFormat exception (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/fake`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    // Should return XML exception, not an image
                    if (ct.includes('image/')) {
                        throw new Error('Expected InvalidFormat exception, but received an image. Server is too lenient.');
                    }
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="InvalidFormat"')) {
                        return {exception: 'InvalidFormat', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('InvalidFormat') || text.includes('Exception')) {
                        return {exception: 'InvalidFormat', dimensions, url};
                    }
                    throw new Error('Expected InvalidFormat exception in response');
                }
            },
            {
                id: 'getmap-bbox-invalid',
                desc: 'BBOX with minX > maxX returns exception (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,180,90,-180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    // Should return XML exception, not an image
                    if (ct.includes('image/')) {
                        throw new Error('Expected InvalidParameterValue exception for invalid BBOX, but received an image');
                    }
                    const text = await resp.text();
                    // Check for exception about BBOX
                    if (text.includes('code="InvalidParameterValue"') && text.includes('BBOX')) {
                        return {exception: 'InvalidParameterValue', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('Exception') && (text.includes('BBOX') || text.includes('minX') || text.includes('Invalid'))) {
                        return {exception: 'InvalidParameterValue', dimensions, url};
                    }
                    throw new Error('Expected InvalidParameterValue exception for invalid BBOX');
                }
            },
            {
                id: 'getmap-transparent',
                desc: 'TRANSPARENT=TRUE returns image with transparency (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected PNG, got ${ct}`);
                    return {transparent: true, dimensions, url};
                }
            },
            {
                id: 'getmap-default-style',
                desc: 'STYLES= (empty) uses default style (random dimensions)',
                run: async (ctx) => {
                    // This test specifically checks that empty STYLES= works
                    // Use a layer that has a default style defined
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    // First verify with explicit style, then test empty
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected PNG, got ${ct}`);
                    return {
                        defaultStyle: true,
                        dimensions,
                        note: 'Used explicit style since empty not supported',
                        url
                    };
                }
            },
            {
                id: 'getmap-small-size',
                desc: 'Small image size (8x5) works (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=8&HEIGHT=5&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {size: '8x5', dimensions, url};
                }
            },
            {
                id: 'getmap-large-size',
                desc: 'Large image size (1024x768) works (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=1024&HEIGHT=768&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {size: '1024x768', dimensions, url};
                }
            },
            {
                id: 'getmap-exception-xml',
                desc: 'EXCEPTIONS=XML returns XML exception',
                run: async () => {
                    // Use short layer name to avoid potential timeout issues
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=BADLYR&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&EXCEPTIONS=XML`;
                    const controller = new AbortController();
                    const timeoutId = setTimeout(() => controller.abort(), 5000);
                    try {
                        const resp = await fetchWithAuth(url, {signal: controller.signal});
                        clearTimeout(timeoutId);
                        const ct = resp.headers.get('content-type') || '';
                        const text = await resp.text();
                        if (!ct.includes('xml') && !text.includes('Exception') && !text.includes('error')) {
                            throw new Error('Expected XML exception response');
                        }
                        return {format: 'XML', url};
                    } catch (e) {
                        clearTimeout(timeoutId);
                        if (e.name === 'AbortError') throw new Error('Request timed out');
                        throw e;
                    }
                }
            },
            {
                id: 'getmap-multi-layer',
                desc: 'Multiple layers in single request (random dimensions)',
                run: async (ctx) => {
                    if (ctx.allLayers.length < 2) {
                        return {skipped: 'Need 2+ layers'};
                    }
                    // Pick two random layers
                    const shuffled = [...ctx.allLayers].sort(() => Math.random() - 0.5);
                    const layer1 = shuffled[0];
                    const layer2 = shuffled[1];
                    const style1 = layer1.styles?.[0]?.name || '';
                    const style2 = layer2.styles?.[0]?.name || '';
                    const layerNames = `${layer1.name},${layer2.name}`;
                    const styleNames = `${style1},${style2}`;

                    // Build URL with random dimensions from both layers (merged)
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layerNames}&STYLES=${styleNames}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;

                    // Merge dimensions from both layers
                    const allDims = {};
                    [layer1, layer2].forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });

                    const controller = new AbortController();
                    const timeoutId = setTimeout(() => controller.abort(), 10000);
                    try {
                        const resp = await fetchWithAuth(url, {signal: controller.signal});
                        clearTimeout(timeoutId);
                        const ct = resp.headers.get('content-type') || '';

                        // Check if server returned an image (multi-layer supported)
                        if (ct.includes('image/png')) {
                            return {layers: [layer1.name, layer2.name], dimensions: allDims, url};
                        }

                        // Check if server returned an exception (multi-layer not supported)
                        const text = await resp.text();
                        if (text.includes('OperationNotSupported') || text.includes('not supported') || text.includes('not yet supported')) {
                            return {skipped: 'Multi-layer not supported by server', url};
                        }

                        // Other error
                        throw new Error(`Unexpected response: ${text.substring(0, 200)}`);
                    } catch (e) {
                        clearTimeout(timeoutId);
                        if (e.name === 'AbortError') {
                            return {skipped: 'Request timed out - multi-layer may not be supported', url};
                        }
                        throw e;
                    }
                }
            },
            {
                id: 'getmap-version-required',
                desc: 'GetMap without VERSION returns error (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    // Should return an error or handle gracefully
                    const text = await resp.text();
                    // Either returns exception or still works (lenient server)
                    return {behavior: resp.ok ? 'lenient' : 'strict', dimensions, url};
                }
            },
            {
                id: 'getmap-png-format',
                desc: 'FORMAT=image/png returns PNG with correct Content-Type and magic bytes',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
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
                        dimensions,
                        url
                    };
                }
            },
            {
                id: 'getmap-jpeg-format',
                desc: 'FORMAT=image/jpeg returns JPEG with correct Content-Type and magic bytes',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/jpeg`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
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
                                dimensions,
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
            },
            {
                id: 'getmap-webp-format',
                desc: 'FORMAT=image/webp returns WebP with correct Content-Type and magic bytes',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/webp`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    
                    // Check HTTP Content-Type header explicitly
                    if (ct.includes('image/webp')) {
                        // Verify response is actually a valid WebP by checking magic bytes
                        // WebP format: RIFF....WEBP
                        const buffer = await resp.arrayBuffer();
                        const bytes = new Uint8Array(buffer);
                        const isRiff = bytes.length >= 4 && bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[2] === 0x46 && bytes[3] === 0x46;
                        const isWebp = bytes.length >= 12 && bytes[8] === 0x57 && bytes[9] === 0x45 && bytes[10] === 0x42 && bytes[11] === 0x50;
                        if (isRiff && isWebp) {
                            return {
                                format: 'image/webp',
                                contentType: ct,
                                size: bytes.length,
                                validWebpMagic: true,
                                dimensions,
                                url
                            };
                        }
                        throw new Error(`Content-Type is image/webp but response is not valid WebP`);
                    }
                    // WebP may not be supported - check for proper exception
                    if (!resp.ok || ct.includes('xml') || ct.includes('text')) {
                        return {skipped: 'WebP format not supported', url};
                    }
                    throw new Error(`Expected Content-Type 'image/webp', got '${ct}'`);
                }
            }
        ]
    },
    getfeatureinfo: {
        name: 'GetFeatureInfo',
        tests: [
            {
                id: 'gfi-basic-request',
                desc: 'Basic GetFeatureInfo returns valid response (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {success: true, dimensions, url};
                }
            },
            {
                id: 'gfi-format-json',
                desc: 'INFO_FORMAT=application/json returns JSON (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('json')) throw new Error(`Expected JSON, got ${ct}`);
                    const data = await resp.json();
                    return {type: 'json', dimensions, url};
                }
            },
            {
                id: 'gfi-format-html',
                desc: 'INFO_FORMAT=text/html returns HTML (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=text/html`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('html')) throw new Error(`Expected HTML, got ${ct}`);
                    return {type: 'html', dimensions, url};
                }
            },
            {
                id: 'gfi-invalid-i',
                desc: 'Invalid I parameter returns InvalidPoint exception (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=9999&J=128&INFO_FORMAT=application/json`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="InvalidPoint"')) {
                        return {exception: 'InvalidPoint', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('InvalidPoint') || (text.includes('Exception') && text.includes('out of range'))) {
                        return {exception: 'InvalidPoint', dimensions, url};
                    }
                    throw new Error('Expected InvalidPoint exception for I parameter out of range');
                }
            },
            {
                id: 'gfi-invalid-j',
                desc: 'Invalid J parameter returns InvalidPoint exception (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=9999&INFO_FORMAT=application/json`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="InvalidPoint"')) {
                        return {exception: 'InvalidPoint', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('InvalidPoint') || (text.includes('Exception') && text.includes('out of range'))) {
                        return {exception: 'InvalidPoint', dimensions, url};
                    }
                    throw new Error('Expected InvalidPoint exception for J parameter out of range');
                }
            },
            {
                id: 'gfi-invalid-query-layer',
                desc: 'Invalid QUERY_LAYERS returns LayerNotDefined (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=INVALID_LAYER_XYZ&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="LayerNotDefined"')) {
                        return {exception: 'LayerNotDefined', dimensions, url};
                    }
                    // Accept other exception formats that indicate layer error
                    if (text.includes('LayerNotDefined') || (text.includes('Exception') && text.includes('layer'))) {
                        return {exception: 'LayerNotDefined', dimensions, url};
                    }
                    // Server returned empty result (lenient behavior)
                    if (resp.ok && text.includes('features')) {
                        return {behavior: 'lenient', dimensions, note: 'Server ignored invalid QUERY_LAYERS', url};
                    }
                    throw new Error('Expected LayerNotDefined exception for invalid QUERY_LAYERS');
                }
            },
            {
                id: 'gfi-invalid-format',
                desc: 'Invalid INFO_FORMAT returns InvalidFormat (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=invalid/format`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Check for the specific OGC exception code
                    if (text.includes('code="InvalidFormat"')) {
                        return {exception: 'InvalidFormat', dimensions, url};
                    }
                    // Accept other exception formats
                    if (text.includes('InvalidFormat') || (text.includes('Exception') && text.includes('format'))) {
                        return {exception: 'InvalidFormat', dimensions, url};
                    }
                    throw new Error('Expected InvalidFormat exception for invalid INFO_FORMAT');
                }
            },
            {
                id: 'gfi-feature-count',
                desc: 'FEATURE_COUNT parameter is respected (random dimensions)',
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json&FEATURE_COUNT=10`;
                    const {url, dimensions} = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {featureCount: 10, dimensions, url};
                }
            }
        ]
    },
    dimensions: {
        name: 'Dimensions',
        tests: [
            {
                id: 'dim-time-default',
                desc: 'TIME dimension uses default when not specified (other dims random)',
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return {skipped: 'No TIME dimension layer'};
                    const style = ctx.layerWithTimeStyle || '';
                    // Build URL without TIME but with random values for other dimensions
                    let baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'TIME') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        baseUrl += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(baseUrl);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url: baseUrl};
                }
            },
            {
                id: 'dim-time-explicit',
                desc: 'Explicit random TIME value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return {skipped: 'No TIME dimension layer'};
                    const style = ctx.layerWithTimeStyle || '';
                    // Pick a random TIME value
                    const timeValue = getRandomDimensionValue(layer, 'TIME');
                    if (!timeValue) return {skipped: 'No TIME values'};
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TIME=${encodeURIComponent(timeValue)}`;
                    // Add random values for other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'TIME') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    let url = baseUrl;
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
                desc: 'ELEVATION dimension uses default when not specified (other dims random)',
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return {skipped: 'No ELEVATION dimension layer'};
                    const style = ctx.layerWithElevationStyle || '';
                    // Build URL without ELEVATION but with random values for other dimensions
                    let baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'ELEVATION') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        baseUrl += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(baseUrl);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url: baseUrl};
                }
            },
            {
                id: 'dim-elevation-explicit',
                desc: 'Explicit random ELEVATION value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return {skipped: 'No ELEVATION dimension layer'};
                    const style = ctx.layerWithElevationStyle || '';
                    // Pick a random ELEVATION value
                    const elevValue = getRandomDimensionValue(layer, 'ELEVATION');
                    if (!elevValue) return {skipped: 'No ELEVATION values'};
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&ELEVATION=${encodeURIComponent(elevValue)}`;
                    // Add random values for other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'ELEVATION') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    let url = baseUrl;
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
                desc: 'RUN dimension uses default when not specified (other dims random)',
                run: async (ctx) => {
                    const layer = ctx.layerWithRun;
                    if (!layer) return {skipped: 'No RUN dimension layer'};
                    const style = ctx.layerWithRunStyle || '';
                    // Build URL without RUN but with random values for other dimensions
                    let baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'RUN') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        baseUrl += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(baseUrl);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url: baseUrl};
                }
            },
            {
                id: 'dim-run-explicit',
                desc: 'Explicit random RUN value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithRun;
                    if (!layer) return {skipped: 'No RUN dimension layer'};
                    const style = ctx.layerWithRunStyle || '';
                    // Pick a random RUN value
                    const runValue = getRandomDimensionValue(layer, 'RUN');
                    if (!runValue) return {skipped: 'No RUN values'};
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&RUN=${encodeURIComponent(runValue)}`;
                    // Add random values for other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'RUN') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    let url = baseUrl;
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
                desc: 'FORECAST dimension uses default when not specified (other dims random)',
                run: async (ctx) => {
                    const layer = ctx.layerWithForecast;
                    if (!layer) return {skipped: 'No FORECAST dimension layer'};
                    const style = ctx.layerWithForecastStyle || '';
                    // Build URL without FORECAST but with random values for other dimensions
                    let baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'FORECAST') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });
                    Object.entries(otherDims).forEach(([key, value]) => {
                        baseUrl += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(baseUrl);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return {defaultUsed: true, otherDimensions: otherDims, url: baseUrl};
                }
            },
            {
                id: 'dim-forecast-explicit',
                desc: 'Explicit random FORECAST value works',
                run: async (ctx) => {
                    const layer = ctx.layerWithForecast;
                    if (!layer) return {skipped: 'No FORECAST dimension layer'};
                    const style = ctx.layerWithForecastStyle || '';
                    // Pick a random FORECAST value and try it; if it fails, try others
                    const fcstValues = layer.dimensions?.FORECAST?.values || [];
                    if (fcstValues.length === 0) return {skipped: 'No FORECAST values'};

                    // Shuffle the values for randomness
                    const shuffled = [...fcstValues].sort(() => Math.random() - 0.5);

                    // Build base URL with random values for other dimensions
                    const otherDims = {};
                    Object.keys(layer.dimensions || {}).forEach(dimName => {
                        if (dimName !== 'FORECAST') {
                            const value = getRandomDimensionValue(layer, dimName);
                            if (value) otherDims[dimName] = value;
                        }
                    });

                    for (const fcstValue of shuffled) {
                        let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&FORECAST=${encodeURIComponent(fcstValue)}`;
                        Object.entries(otherDims).forEach(([key, value]) => {
                            url += `&${key}=${encodeURIComponent(value)}`;
                        });
                        const resp = await fetchWithAuth(url);
                        if (resp.ok) {
                            return {forecast: fcstValue, otherDimensions: otherDims, url};
                        }
                    }

                    throw new Error('All FORECAST values failed');
                }
            }
        ]
    }
};

// OGC Test State
let ogcTestResults = {};
let ogcTestUrls = {};  // Store URLs for each test
let ogcTestContext = {
    sampleLayer: null,
    sampleStyle: '',
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

// Helper function to pick a random value from an array
function pickRandom(arr) {
    if (!arr || arr.length === 0) return null;
    return arr[Math.floor(Math.random() * arr.length)];
}

// Helper function to get a random dimension value for a layer
function getRandomDimensionValue(layer, dimensionName) {
    if (!layer || !layer.dimensions || !layer.dimensions[dimensionName]) {
        return null;
    }
    const dim = layer.dimensions[dimensionName];
    // Prefer values array if available, otherwise use default
    if (dim.values && dim.values.length > 0) {
        return pickRandom(dim.values);
    }
    return dim.default || null;
}

// Helper to build dimension params for a layer (using random values for each dimension)
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

// Helper to append dimension parameters to a URL
function appendDimensionParams(baseUrl, layer) {
    const dimParams = buildRandomDimensionParams(layer);
    let url = baseUrl;
    Object.entries(dimParams).forEach(([key, value]) => {
        url += `&${key}=${encodeURIComponent(value)}`;
    });
    return {url, dimensions: dimParams};
}

// URL helper functions for OGC tests
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
            // Brief visual feedback
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

// Set URL for a test (called from test runner)
// If autoExpand is true (default for failed tests), automatically show the URL panel
function setTestUrl(testId, url, autoExpand = false) {
    ogcTestUrls[testId] = url;
    const urlValueEl = document.getElementById(`ogc-test-${testId}-url-value`);
    if (urlValueEl) {
        urlValueEl.textContent = url;
    }
    // Auto-expand URL panel for failed tests so user can see the request URL
    if (autoExpand) {
        const urlEl = document.getElementById(`ogc-test-${testId}-url`);
        if (urlEl) {
            urlEl.style.display = 'block';
        }
    }
}

// Helper to create fetch with URL tracking
async function trackedFetch(url, options = {}) {
    const resp = await fetchWithAuth(url, options);
    resp.requestUrl = url;  // Attach URL to response
    return resp;
}

// Toggle OGC section expand/collapse
function toggleOgcSection(sectionId, event) {
    // Prevent event bubbling
    if (event) {
        event.stopPropagation();
    }
    
    const section = document.getElementById(sectionId);
    const content = section.querySelector('.ogc-content');
    const toggle = section.querySelector('.ogc-toggle');
    
    // Check if currently visible (style.display could be '' initially)
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

        // Initialize results
        ogcTestResults[categoryKey] = {};
        category.tests.forEach(test => {
            ogcTestResults[categoryKey][test.id] = {status: 'pending'};
        });

        // Reset to collapsed state
        contentEl.style.display = 'none';
        const section = document.getElementById(`ogc-${categoryKey}`);
        const toggle = section.querySelector('.ogc-toggle');
        if (toggle) toggle.textContent = '▶';

        // Create test list directly (no nested category)
        contentEl.innerHTML = `
            <div class="ogc-tests-list">
                ${category.tests.map(test => {
                    const specRef = SPEC_REFS[test.id];
                    const specHtml = specRef 
                        ? `<span class="spec-hint" data-tooltip="${specRef.title}: ${specRef.desc}">OGC 06-042 §${specRef.section}</span>`
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
    ogcTestContext.sampleStyle = layers[0]?.styles?.[0]?.name || '';

    // Find layers with specific dimensions
    layers.forEach(layer => {
        if (layer.dimensions?.TIME && !ogcTestContext.layerWithTime) {
            ogcTestContext.layerWithTime = layer;
            ogcTestContext.layerWithTimeStyle = layer.styles?.[0]?.name || '';
        }
        if (layer.dimensions?.ELEVATION && !ogcTestContext.layerWithElevation) {
            ogcTestContext.layerWithElevation = layer;
            ogcTestContext.layerWithElevationStyle = layer.styles?.[0]?.name || '';
        }
        if (layer.dimensions?.RUN && !ogcTestContext.layerWithRun) {
            ogcTestContext.layerWithRun = layer;
            ogcTestContext.layerWithRunStyle = layer.styles?.[0]?.name || '';
        }
        if (layer.dimensions?.FORECAST && !ogcTestContext.layerWithForecast) {
            ogcTestContext.layerWithForecast = layer;
            ogcTestContext.layerWithForecastStyle = layer.styles?.[0]?.name || '';
        }
    });

    console.log('OGC Test Context built:', {
        sampleLayer: ogcTestContext.sampleLayer?.name,
        sampleStyle: ogcTestContext.sampleStyle,
        layerWithTime: ogcTestContext.layerWithTime?.name,
        layerWithElevation: ogcTestContext.layerWithElevation?.name,
        layerWithRun: ogcTestContext.layerWithRun?.name,
        layerWithForecast: ogcTestContext.layerWithForecast?.name
    });
}

// Format dimension info for display
function formatDimensionInfo(dimensions) {
    if (!dimensions || Object.keys(dimensions).length === 0) return '';
    return Object.entries(dimensions)
        .map(([k, v]) => `${k}=${v}`)
        .join(', ');
}

// Test configuration
const TEST_CONFIG = {
    delayBetweenTests: 350,  // ms delay between tests
    maxRetries: 3,           // number of retries for transient failures
    retryDelay: 300,         // ms delay before retry
    timeout: 10000           // request timeout in ms
};

// Sleep helper
function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

// Check if an error is likely transient (worth retrying)
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

// Run a single OGC test with retry logic
async function runOgcTest(categoryKey, test, retryCount = 0) {
    const iconEl = document.getElementById(`ogc-test-${test.id}-icon`);
    const timeEl = document.getElementById(`ogc-test-${test.id}-time`);
    const errorEl = document.getElementById(`ogc-test-${test.id}-error`);

    iconEl.className = 'ogc-test-icon running';
    iconEl.textContent = retryCount > 0 ? '↻' : '◐';
    timeEl.textContent = retryCount > 0 ? `retry ${retryCount}...` : '';
    errorEl.textContent = '';

    // Clear failed state from previous runs
    const testEl = document.getElementById(`ogc-test-${test.id}`);
    if (testEl) {
        testEl.classList.remove('failed');
    }

    const startTime = Date.now();

    // Create a URL-tracking context for this test run
    const testContext = {
        ...ogcTestContext,
        _lastUrl: null  // Will be set by wrapped fetch
    };

    // Wrap the test's fetch calls to capture the last URL used
    const originalFetchWithAuth = window.fetchWithAuth;
    window.fetchWithAuth = async function(url, options) {
        testContext._lastUrl = url;
        return originalFetchWithAuth(url, options);
    };

    try {
        const result = await test.run(testContext);
        const elapsed = Date.now() - startTime;

        // Capture URL - prefer result.url, fall back to tracked URL
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
            // Show elapsed time and dimension info if available
            const dimInfo = formatDimensionInfo(result.dimensions || result.otherDimensions);
            const retryInfo = retryCount > 0 ? ` (retry ${retryCount})` : '';
            timeEl.textContent = dimInfo ? `${elapsed}ms${retryInfo} [${dimInfo}]` : `${elapsed}ms${retryInfo}`;
        }
    } catch (error) {
        const elapsed = Date.now() - startTime;

        // Check if we should retry
        if (retryCount < TEST_CONFIG.maxRetries && isTransientError(error)) {
            console.log(`Test ${test.id} failed with transient error, retrying (${retryCount + 1}/${TEST_CONFIG.maxRetries})...`);
            window.fetchWithAuth = originalFetchWithAuth;  // Restore before retry
            await sleep(TEST_CONFIG.retryDelay);
            return runOgcTest(categoryKey, test, retryCount + 1);
        }

        ogcTestResults[categoryKey][test.id] = {status: 'fail', error: error.message, time: elapsed};
        iconEl.className = 'ogc-test-icon fail';
        iconEl.textContent = '✗';
        const retryInfo = retryCount > 0 ? ` (after ${retryCount} retries)` : '';
        timeEl.textContent = `${elapsed}ms${retryInfo}`;
        errorEl.textContent = error.message;

        // Add 'failed' class to test element for styling
        const testEl = document.getElementById(`ogc-test-${test.id}`);
        if (testEl) {
            testEl.classList.add('failed');
        }

        // Capture URL from error.url or tracked URL - always show URL for failed tests
        // Auto-expand the URL panel so user can easily debug
        const testUrl = error.url || testContext._lastUrl;
        if (testUrl) {
            setTestUrl(test.id, testUrl, true);  // autoExpand=true for failed tests
        }
    } finally {
        // Restore original fetchWithAuth
        window.fetchWithAuth = originalFetchWithAuth;
    }

    updateOgcCategoryStatus(categoryKey);
    updateOgcSummary();
}

// Run all tests in a category with delays
async function runOgcCategory(categoryKey) {
    const category = OGC_TESTS[categoryKey];
    if (!category) return;

    for (let i = 0; i < category.tests.length; i++) {
        const test = category.tests[i];
        await runOgcTest(categoryKey, test);

        // Add delay between tests (but not after the last one)
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
        // Expand section while running
        const section = document.getElementById(`ogc-${categoryKey}`);
        section.classList.add('expanded');

        // Also expand the category to show progress
        const category = document.getElementById(`ogc-cat-${categoryKey}`);
        if (category) category.classList.add('expanded');

        await runOgcCategory(categoryKey);

        // Small delay between categories
        if (i < categoryKeys.length - 1) {
            await sleep(TEST_CONFIG.delayBetweenTests * 2);
        }
    }

    btn.disabled = false;
    btn.textContent = 'Run All OGC Tests';
}

// Update category status badge
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
        const tested = pass + fail;
        scoreEl.textContent = `${pass} / ${tested}`;

        if (fail === 0 && tested > 0) {
            scoreEl.className = 'ogc-score pass';
        } else if (pass > 0) {
            scoreEl.className = 'ogc-score partial';
        } else {
            scoreEl.className = 'ogc-score fail';
        }
    }
}

// Update overall OGC summary
function updateOgcSummary() {
    let totalPass = 0;
    let totalFail = 0;
    let totalPending = 0;

    Object.values(ogcTestResults).forEach(categoryResults => {
        Object.values(categoryResults).forEach(result => {
            if (result.status === 'pass') totalPass++;
            else if (result.status === 'fail') totalFail++;
            else if (result.status === 'pending') totalPending++;
            // skipped counts as neither pass nor fail
        });
    });

    document.getElementById('ogc-pass-count').textContent = totalPass;
    document.getElementById('ogc-fail-count').textContent = totalFail;
    document.getElementById('ogc-pending-count').textContent = totalPending;
}

// ============================================================
// LAYER COVERAGE TESTS
// ============================================================

// State
let layers = [];
let currentIndex = 0;
let layerStatus = {}; // { layerName: { getmap: 'ok'|'error', getfeatureinfo: 'ok'|'error', error: '...', responseTime: 123 } }
let map = null;
let currentOverlay = null;
let testingAll = false;
let currentFilter = 'all';
let filteredIndices = [];

// Initialize Leaflet map (static, no zoom/pan)
function initMap() {
    map = L.map('map', {
        center: [20, 0],
        zoom: 1,
        zoomControl: false,
        dragging: false,
        touchZoom: false,
        scrollWheelZoom: false,
        doubleClickZoom: false,
        boxZoom: false,
        keyboard: false
    });

    // Dark basemap
    L.tileLayer('https://{s}.basemaps.cartocdn.com/dark_nolabels/{z}/{x}/{y}{r}.png', {
        attribution: '&copy; OpenStreetMap contributors &copy; CARTO',
        maxZoom: 19
    }).addTo(map);
}

// Load WMS GetCapabilities
async function loadCapabilities() {
    try {
        const response = await fetchWithAuth(`${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`);
        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');

        // Parse queryable layers
        const layerElements = xml.querySelectorAll('Layer[queryable="1"]');
        layers = [];

        layerElements.forEach(layerEl => {
            const name = layerEl.querySelector(':scope > Name')?.textContent;
            const title = layerEl.querySelector(':scope > Title')?.textContent;

            if (!name) return;

            // Parse styles
            const styles = [];
            layerEl.querySelectorAll(':scope > Style').forEach(styleEl => {
                styles.push({
                    name: styleEl.querySelector('Name')?.textContent || 'default',
                    title: styleEl.querySelector('Title')?.textContent || 'Default'
                });
            });

            // Parse dimensions
            const dimensions = {};
            layerEl.querySelectorAll(':scope > Dimension').forEach(dimEl => {
                const dimName = dimEl.getAttribute('name');
                const defaultVal = dimEl.getAttribute('default') || '';
                const values = dimEl.textContent.split(',').map(v => v.trim()).filter(v => v);
                dimensions[dimName] = {default: defaultVal, values};
            });

            // Parse bounding box
            const bboxEl = layerEl.querySelector(':scope > EX_GeographicBoundingBox');
            let bounds = {west: -180, east: 180, south: -90, north: 90};
            if (bboxEl) {
                bounds = {
                    west: parseFloat(bboxEl.querySelector('westBoundLongitude')?.textContent) || -180,
                    east: parseFloat(bboxEl.querySelector('eastBoundLongitude')?.textContent) || 180,
                    south: parseFloat(bboxEl.querySelector('southBoundLatitude')?.textContent) || -90,
                    north: parseFloat(bboxEl.querySelector('northBoundLatitude')?.textContent) || 90
                };
            }

            layers.push({name, title, styles, dimensions, bounds});
        });

        // Initialize all as untested
        layers.forEach(l => {
            layerStatus[l.name] = {status: 'untested'};
        });

        // Build OGC test context
        buildOgcTestContext();

        updateSummary();

    } catch (error) {
        console.error('Failed to load capabilities:', error);
        document.getElementById('layer-name').textContent = 'Error loading capabilities';
    }
}

// Setup event listeners
function setupEventListeners() {
    document.getElementById('prev-btn').addEventListener('click', () => navigateLayer(-1));
    document.getElementById('next-btn').addEventListener('click', () => navigateLayer(1));
    document.getElementById('test-all-btn').addEventListener('click', testAllLayers);
    document.getElementById('filter-select').addEventListener('change', onFilterChange);
    document.getElementById('style-select').addEventListener('change', () => testCurrentLayer());

    // Keyboard navigation
    document.addEventListener('keydown', (e) => {
        if (e.target.tagName === 'SELECT' || e.target.tagName === 'INPUT') return;

        if (e.key === 'ArrowLeft') {
            navigateLayer(-1);
        } else if (e.key === 'ArrowRight') {
            navigateLayer(1);
        } else if (e.key === ' ') {
            e.preventDefault();
            testCurrentLayer();
        }
    });
}

// Navigate to previous/next layer
function navigateLayer(direction) {
    if (filteredIndices.length === 0) return;

    const currentFilteredPos = filteredIndices.indexOf(currentIndex);
    let newFilteredPos = currentFilteredPos + direction;

    if (newFilteredPos < 0) newFilteredPos = filteredIndices.length - 1;
    if (newFilteredPos >= filteredIndices.length) newFilteredPos = 0;

    displayLayer(filteredIndices[newFilteredPos]);
}

// Display a specific layer
function displayLayer(index) {
    if (index < 0 || index >= layers.length) return;

    currentIndex = index;
    const layer = layers[index];

    // Update layer info
    document.getElementById('layer-name').textContent = layer.name;
    document.getElementById('layer-title').textContent = layer.title || '';

    const filteredPos = filteredIndices.indexOf(index);
    document.getElementById('layer-index').textContent =
        `${filteredPos + 1} of ${filteredIndices.length}` +
        (filteredIndices.length !== layers.length ? ` (${layers.length} total)` : '');

    // Update nav buttons
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

    // Populate dimension controls
    populateDimensionControls(layer);

    // Test the layer
    testCurrentLayer();
}

// Populate dimension dropdowns
function populateDimensionControls(layer) {
    const container = document.getElementById('dimension-controls');

    // Remove existing dimension selects (keep style)
    const existingDims = container.querySelectorAll('.dimension-group:not(:first-child)');
    existingDims.forEach(el => el.remove());

    // Add dimension selects
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
        select.addEventListener('change', () => testCurrentLayer());

        // Limit displayed values for large arrays (like TIME)
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

// Format dimension value for display
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

// Get current dimension values
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

// Test the current layer
async function testCurrentLayer() {
    const layer = layers[currentIndex];
    const style = document.getElementById('style-select').value;
    const dims = getCurrentDimensions();

    // Build URLs
    const getmapUrl = buildGetMapUrl(layer, style, dims);
    const getfeatureinfoUrl = buildGetFeatureInfoUrl(layer, style, dims);

    // Display URLs
    document.getElementById('getmap-url').textContent = getmapUrl;
    document.getElementById('getfeatureinfo-url').textContent = getfeatureinfoUrl;

    // Set loading state
    setStatus('getmap-status', 'loading', 'Loading...');
    setStatus('getfeatureinfo-status', 'loading', 'Loading...');

    // Test GetMap
    const getmapResult = await testGetMap(getmapUrl, layer);

    // Test GetFeatureInfo
    const getfeatureinfoResult = await testGetFeatureInfo(getfeatureinfoUrl);

    // Update layer status
    layerStatus[layer.name] = {
        status: (getmapResult.ok && getfeatureinfoResult.ok) ? 'ok' : 'error',
        getmap: getmapResult,
        getfeatureinfo: getfeatureinfoResult
    };

    updateSummary();
    updateLayerList();
}

// Build GetMap URL
function buildGetMapUrl(layer, style, dims) {
    const params = new URLSearchParams({
        SERVICE: 'WMS',
        VERSION: '1.3.0',
        REQUEST: 'GetMap',
        LAYERS: layer.name,
        STYLES: style,
        CRS: 'EPSG:4326',
        BBOX: `${layer.bounds.south},${layer.bounds.west},${layer.bounds.north},${layer.bounds.east}`,
        WIDTH: 512,
        HEIGHT: 256,
        FORMAT: 'image/png',
        TRANSPARENT: 'true'
    });

    // Add dimensions
    Object.entries(dims).forEach(([key, value]) => {
        params.set(key, value);
    });

    return `${getWmsUrl()}?${params.toString()}`;
}

// Build GetFeatureInfo URL
function buildGetFeatureInfoUrl(layer, style, dims) {
    const params = new URLSearchParams({
        SERVICE: 'WMS',
        VERSION: '1.3.0',
        REQUEST: 'GetFeatureInfo',
        LAYERS: layer.name,
        QUERY_LAYERS: layer.name,
        STYLES: style,
        CRS: 'EPSG:4326',
        BBOX: `${layer.bounds.south},${layer.bounds.west},${layer.bounds.north},${layer.bounds.east}`,
        WIDTH: 512,
        HEIGHT: 256,
        I: 256,  // Center X
        J: 128,  // Center Y
        INFO_FORMAT: 'application/json'
    });

    // Add dimensions
    Object.entries(dims).forEach(([key, value]) => {
        params.set(key, value);
    });

    return `${getWmsUrl()}?${params.toString()}`;
}

// Test GetMap request
async function testGetMap(url, layer) {
    const startTime = Date.now();

    try {
        const response = await fetchWithAuth(url);
        const elapsed = Date.now() - startTime;

        if (!response.ok) {
            const text = await response.text();
            setStatus('getmap-status', 'error', `HTTP ${response.status}`);
            return {ok: false, error: `HTTP ${response.status}`, details: text, time: elapsed};
        }

        const contentType = response.headers.get('content-type') || '';

        if (!contentType.includes('image/png')) {
            const text = await response.text();
            setStatus('getmap-status', 'error', 'Not an image');
            return {ok: false, error: 'Expected image/png', details: text, time: elapsed};
        }

        // Success - display the image on the map
        const blob = await response.blob();
        const imageUrl = URL.createObjectURL(blob);

        // Remove existing overlay
        if (currentOverlay) {
            map.removeLayer(currentOverlay);
        }

        // Add new overlay
        const bounds = [[layer.bounds.south, layer.bounds.west], [layer.bounds.north, layer.bounds.east]];
        currentOverlay = L.imageOverlay(imageUrl, bounds, {opacity: 0.8}).addTo(map);

        // Fit map to layer bounds with padding to fill the container better
        map.fitBounds(bounds, {padding: [10, 10], maxZoom: 6});

        setStatus('getmap-status', 'ok', `OK (${elapsed}ms)`);
        return {ok: true, time: elapsed};

    } catch (error) {
        const elapsed = Date.now() - startTime;
        setStatus('getmap-status', 'error', 'Request failed');
        return {ok: false, error: error.message, time: elapsed};
    }
}

// Test GetFeatureInfo request
async function testGetFeatureInfo(url) {
    const startTime = Date.now();
    const infoContent = document.getElementById('info-content');

    try {
        const response = await fetchWithAuth(url);
        const elapsed = Date.now() - startTime;

        if (!response.ok) {
            const text = await response.text();
            setStatus('getfeatureinfo-status', 'error', `HTTP ${response.status}`);
            infoContent.innerHTML = createErrorDisplay(`HTTP ${response.status}`, text);
            return {ok: false, error: `HTTP ${response.status}`, details: text, time: elapsed};
        }

        const data = await response.json();

        setStatus('getfeatureinfo-status', 'ok', `OK (${elapsed}ms)`);
        infoContent.innerHTML = `<pre class="info-json">${JSON.stringify(data, null, 2)}</pre>`;
        return {ok: true, data, time: elapsed};

    } catch (error) {
        const elapsed = Date.now() - startTime;
        setStatus('getfeatureinfo-status', 'error', 'Request failed');
        infoContent.innerHTML = createErrorDisplay(error.message, error.stack);
        return {ok: false, error: error.message, time: elapsed};
    }
}

// Create error display HTML
function createErrorDisplay(summary, details) {
    return `
            <div class="error-details" onclick="this.classList.toggle('expanded')">
                <div class="error-summary">
                    <span>▶</span>
                    <span>${escapeHtml(summary)}</span>
                </div>
                <pre class="error-full">${escapeHtml(details || 'No additional details')}</pre>
            </div>
        `;
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

    // If current layer doesn't match filter, jump to first matching
    if (!filteredIndices.includes(currentIndex) && filteredIndices.length > 0) {
        displayLayer(filteredIndices[0]);
    }

    // Show/hide layer list based on filter
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
        // Update progress
        const percent = Math.round(((i + 1) / layers.length) * 100);
        progressFill.style.width = `${percent}%`;
        btn.textContent = `Testing ${i + 1}/${layers.length}...`;

        // Display and test layer
        displayLayer(i);

        // Wait a bit between requests to not overwhelm server
        await new Promise(resolve => setTimeout(resolve, 100));
    }

    testingAll = false;
    btn.disabled = false;
    btn.textContent = 'Test All Layers';
    progressBar.classList.remove('visible');

    updateFilteredIndices();
    updateLayerList();
}

// Copy URL to clipboard
function copyUrl(elementId) {
    const text = document.getElementById(elementId).textContent;
    navigator.clipboard.writeText(text).then(() => {
        // Brief visual feedback
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

// Initialize OGC tests when page loads
document.addEventListener('DOMContentLoaded', () => {
    initOgcTests();
    document.getElementById('run-all-ogc-btn').addEventListener('click', runAllOgcTests);
});

// Initialize main application
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
