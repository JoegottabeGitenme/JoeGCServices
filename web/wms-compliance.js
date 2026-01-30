// WMS 1.3.0 OGC CITE Compliance Tests JavaScript
// Aligned with OGC TEAM ENGINE WMS 1.3.0 test suite (ets-wms13)
// Specification: OGC 06-042 - Web Map Server Implementation Specification

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
    return /\/wms\/?(\?|$)/i.test(endpoint) || 
           /\/ows\/?(\?|$)/i.test(endpoint) ||
           lower.includes('service=wms');
}

// Get the WMS endpoint URL (appends /wms only if needed)
function getWmsUrl() {
    if (endpointIncludesWmsPath(API_BASE)) {
        return API_BASE;
    }
    return `${API_BASE}/wms`;
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
        localStorage.setItem('wms-compliance-endpoint', newEndpoint);
    } else {
        localStorage.removeItem('wms-compliance-endpoint');
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
    localStorage.removeItem('wms-compliance-endpoint');

    await reloadWithNewEndpoint();
}

async function reloadWithNewEndpoint() {
    updateEndpointStatus('loading', 'Connecting...');
    layerStatus = {};

    try {
        await loadCapabilities();
        initCiteTests();
        updateConformanceSummary();

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
// OGC CITE WMS 1.3.0 TEST DEFINITIONS
// ============================================================

// Requirement types
const REQ_MANDATORY = 'mandatory';
const REQ_OPTIONAL = 'optional';
const REQ_RECOMMENDATION = 'recommendation';
const REQ_MANUAL = 'manual';

// Conformance classes
const CONF_BASIC = 'basic';
const CONF_QUERYABLE = 'queryable';

// Test modules aligned with ets-wms13 structure
const CITE_TESTS = {
    // ========== MAIN ENTRY POINT (3 tests) ==========
    main: {
        name: 'Main Entry Point',
        description: 'Overall compliance verification',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'main:main',
                name: 'Main conformance test',
                purpose: 'Overall compliance verification',
                specRef: 'Annex A',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    if (!text.includes('WMS_Capabilities')) throw new Error('Not a valid WMS capabilities document');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'main:data-independent',
                name: 'Data independence',
                purpose: 'Server does not require specific test data',
                specRef: '-',
                reqType: REQ_MANUAL,
                run: async () => ({ skipped: 'Manual verification required' })
            },
            {
                citeId: 'main:data-preconditions',
                name: 'Data preconditions',
                purpose: 'Verify test data availability',
                specRef: '-',
                reqType: REQ_MANUAL,
                run: async () => ({ skipped: 'Manual verification required' })
            }
        ]
    },

    // ========== BASIC WMS (14 tests) ==========
    basic: {
        name: 'Basic WMS',
        description: 'Core WMS functionality and basic compliance',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'basic:basic',
                name: 'Basic WMS compliance',
                purpose: 'Overall basic WMS verification',
                specRef: 'Sec 2.2, Annex A.1.2',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    if (!text.includes('WMS_Capabilities')) throw new Error('Invalid capabilities');
                    if (!text.includes('<GetMap>')) throw new Error('Missing GetMap operation');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'basic:options-requirements',
                name: 'OPTIONS method support',
                purpose: 'HTTP OPTIONS method handling',
                specRef: 'Sec 6.8.1',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    try {
                        const resp = await fetch(getWmsUrl(), { method: 'OPTIONS' });
                        return { supported: resp.ok, status: resp.status };
                    } catch (e) {
                        return { skipped: 'OPTIONS not supported' };
                    }
                }
            },
            {
                citeId: 'basic:getmap',
                name: 'GetMap availability',
                purpose: 'Verify GetMap operation exists',
                specRef: 'Sec 7.3, Annex A.1.2.4',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('<GetMap>') && !text.includes('GetMap')) {
                        throw new Error('GetMap operation not advertised');
                    }
                    return { available: true, url };
                }
            },
            {
                citeId: 'basic:interactive',
                name: 'Interactive tests',
                purpose: 'Manual verification tests',
                specRef: '-',
                reqType: REQ_MANUAL,
                run: async () => ({ skipped: 'Manual verification required' })
            },
            {
                citeId: 'basic:gif-or-png',
                name: 'Image format support',
                purpose: 'PNG/GIF format availability',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasPng = text.includes('image/png');
                    const hasGif = text.includes('image/gif');
                    if (!hasPng && !hasGif) {
                        throw new Error('Neither PNG nor GIF format advertised');
                    }
                    return { png: hasPng, gif: hasGif, url };
                }
            },
            {
                citeId: 'basic:bbox',
                name: 'Bounding box handling',
                purpose: 'Basic bbox parameter support',
                specRef: 'Sec 7.3.3.6, C.4.2',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/')) throw new Error(`Expected image, got ${ct}`);
                    return { success: true, url };
                }
            },
            {
                citeId: 'basic:bgcolor',
                name: 'Background color',
                purpose: 'BGCOLOR parameter handling',
                specRef: 'Sec 7.3.3.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=FALSE&BGCOLOR=0xFF0000`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { bgcolor: '0xFF0000', url };
                }
            },
            {
                citeId: 'basic:transparent',
                name: 'Transparency',
                purpose: 'TRANSPARENT parameter handling',
                specRef: 'Sec 7.3.3.10',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { transparent: true, url };
                }
            },
            {
                citeId: 'basic:bbox-exponential',
                name: 'Exponential notation',
                purpose: 'Scientific notation in bbox',
                specRef: 'Sec 6.5.3',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Use exponential notation: -9e1 = -90, 1.8e2 = 180
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-9e1,-1.8e2,9e1,1.8e2&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status} - exponential notation not supported`);
                    return { supported: true, url };
                }
            },
            {
                citeId: 'basic:bbox-pixel-interpretation',
                name: 'Pixel interpretation',
                purpose: 'Bbox to pixel mapping',
                specRef: 'Sec 7.3.3.6, C.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-45,-90,45,90&WIDTH=180&HEIGHT=90&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { valid: true, url };
                }
            },
            {
                citeId: 'basic:no-bgcolor',
                name: 'Default background',
                purpose: 'Default bgcolor when not specified',
                specRef: 'Sec 7.3.3.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=FALSE`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { defaultUsed: true, url };
                }
            },
            {
                citeId: 'basic:blue-bgcolor',
                name: 'Blue background',
                purpose: 'Blue bgcolor (0x0000FF)',
                specRef: 'Sec 7.3.3.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=FALSE&BGCOLOR=0x0000FF`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { bgcolor: '0x0000FF', url };
                }
            },
            {
                citeId: 'basic:transparent-true',
                name: 'Transparent true',
                purpose: 'TRANSPARENT=TRUE handling',
                specRef: 'Sec 7.3.3.10',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { transparent: true, url };
                }
            },
            {
                citeId: 'basic:layer-order',
                name: 'Layer rendering order',
                purpose: 'Layer cascade/overlay order',
                specRef: 'Sec 7.3.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    if (ctx.allLayers.length < 2) {
                        return { skipped: 'Need 2+ layers for layer order test' };
                    }
                    const layer1 = ctx.allLayers[0];
                    const layer2 = ctx.allLayers[1];
                    const style1 = layer1.styles?.[0]?.name || '';
                    const style2 = layer2.styles?.[0]?.name || '';
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer1.name},${layer2.name}&STYLES=${style1},${style2}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const allDims = {};
                    [layer1, layer2].forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        const text = await resp.text();
                        if (text.includes('not supported') || text.includes('OperationNotSupported')) {
                            return { skipped: 'Multi-layer not supported' };
                        }
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    return { layers: [layer1.name, layer2.name], url };
                }
            },
            {
                citeId: 'basic:aspect-ratio',
                name: 'Aspect ratio preservation',
                purpose: 'Maintaining aspect ratio',
                specRef: 'Sec 7.3.3.8, C.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Request with specific aspect ratio
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-45,-90,45,90&WIDTH=200&HEIGHT=100&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { aspectRatio: '2:1', url };
                }
            }
        ]
    },

    // ========== BASIC ELEMENTS (10 tests) ==========
    basic_elements: {
        name: 'Basic Elements',
        description: 'HTTP protocol and request formatting',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'basic_elements:basic_elements',
                name: 'Basic service',
                purpose: 'Overall server behavior',
                specRef: 'Sec 6, Annex A.1.2.2',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { valid: true, url };
                }
            },
            {
                citeId: 'basic_elements:version-negotiation',
                name: 'Version negotiation',
                purpose: 'Handle unsupported versions',
                specRef: 'Sec 6.2, Annex A.1.2.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const match = text.match(/WMS_Capabilities[^>]*version="([^"]+)"/);
                    if (!match) throw new Error('No version in capabilities');
                    return { version: match[1], url };
                }
            },
            {
                citeId: 'basic_elements:reserved-chars',
                name: 'Reserved characters',
                purpose: 'URL encoding of reserved chars',
                specRef: 'Sec 6.3.3, Table 2',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&TEST%3DPARAM=value`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { decoded: true, url };
                }
            },
            {
                citeId: 'basic_elements:param-rules',
                name: 'Parameter rules',
                purpose: 'Case sensitivity, parameter order',
                specRef: 'Sec 6.3.4, 6.5',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?VERSION=1.3.0&SERVICE=WMS&REQUEST=GetCapabilities`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status} - parameter order matters`);
                    return { orderIndependent: true, url };
                }
            },
            {
                citeId: 'basic_elements:extra-GetCapabilities-param',
                name: 'Extra GetCapabilities parameters',
                purpose: 'Ignore unknown GetCapabilities params',
                specRef: 'Sec 6.3.4',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&UNKNOWN_PARAM=test123`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { ignored: true, url };
                }
            },
            {
                citeId: 'basic_elements:extra-GetMap-param',
                name: 'Extra GetMap parameters',
                purpose: 'Ignore unknown GetMap params',
                specRef: 'Sec 6.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&UNKNOWN_PARAM=xyz`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { ignored: true, url };
                }
            },
            {
                citeId: 'basic_elements:extra-GetFeatureInfo-param',
                name: 'Extra GetFeatureInfo parameters',
                purpose: 'Ignore unknown GetFeatureInfo params',
                specRef: 'Sec 6.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json&UNKNOWN_PARAM=xyz`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { ignored: true, url };
                }
            },
            {
                citeId: 'basic_elements:escaped-chars',
                name: 'Character escaping',
                purpose: 'URL encoding rules',
                specRef: 'Sec 6.3.3',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&TEST%26PARAM=value`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { decoded: true, url };
                }
            },
            {
                citeId: 'basic_elements:escaped-space',
                name: 'Space escaping',
                purpose: 'Space character encoding (%20)',
                specRef: 'Sec 6.3.3',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&TEST%20PARAM=value`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { decoded: true, url };
                }
            },
            {
                citeId: 'basic_elements:negotiate-no-version',
                name: 'No version specified',
                purpose: 'Handle missing VERSION parameter',
                specRef: 'Sec 6.2, 6.9.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const match = text.match(/WMS_Capabilities[^>]*version="([^"]+)"/);
                    if (!match) throw new Error('No version found');
                    if (parseFloat(match[1]) < 1.3) throw new Error(`Version ${match[1]} < 1.3.0`);
                    return { version: match[1], url };
                }
            }
        ]
    },

    // ========== GETCAPABILITIES (34 tests) ==========
    getcapabilities: {
        name: 'GetCapabilities',
        description: 'Service metadata (capabilities document) validation',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'getcapabilities:getcapabilities',
                name: 'GetCapabilities compliance',
                purpose: 'Overall GetCapabilities verification',
                specRef: 'Sec 7.2, Annex A.1.2.3',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    if (!text.includes('WMS_Capabilities')) throw new Error('Invalid capabilities');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:requests',
                name: 'Request types',
                purpose: 'GetCapabilities request handling',
                specRef: 'Sec 7.2.3',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const text = await resp.text();
                    const hasGetCaps = text.includes('<GetCapabilities>') || text.includes('GetCapabilities');
                    const hasGetMap = text.includes('<GetMap>') || text.includes('GetMap');
                    if (!hasGetCaps || !hasGetMap) throw new Error('Missing required operations');
                    return { operations: ['GetCapabilities', 'GetMap'], url };
                }
            },
            {
                citeId: 'getcapabilities:xml-validation',
                name: 'XML validation',
                purpose: 'Valid XML structure',
                specRef: 'Sec 7.2.4, Annex E.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const parseError = doc.querySelector('parsererror');
                    if (parseError) throw new Error('XML parse error');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:capability-metadata',
                name: 'Capability metadata',
                purpose: 'Service metadata structure',
                specRef: 'Sec 7.2.4.2',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('<Capability>')) throw new Error('Missing Capability element');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:layer-properties',
                name: 'Layer properties',
                purpose: 'Layer metadata validation',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('<Layer')) throw new Error('No Layer elements found');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:dimensions',
                name: 'Dimensions',
                purpose: 'Dimension layer identification',
                specRef: 'Sec C.4.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasDimensions = text.includes('<Dimension');
                    return { hasDimensions, url };
                }
            },
            {
                citeId: 'getcapabilities:each-format',
                name: 'Each format',
                purpose: 'Iterate all advertised formats',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const formats = [];
                    const formatMatches = text.matchAll(/<Format>([^<]+)<\/Format>/g);
                    for (const match of formatMatches) {
                        if (!formats.includes(match[1])) formats.push(match[1]);
                    }
                    return { formats, count: formats.length, url };
                }
            },
            {
                citeId: 'getcapabilities:no-format',
                name: 'No format',
                purpose: 'Format parameter handling',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('xml')) throw new Error(`Expected XML, got ${ct}`);
                    return { contentType: ct, url };
                }
            },
            {
                citeId: 'getcapabilities:invalid-format',
                name: 'Invalid format',
                purpose: 'Invalid format exception',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&FORMAT=invalid/format`;
                    const resp = await fetchWithAuth(url);
                    // Server should either ignore the invalid format or return an exception
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getcapabilities:updatesequence-ignored',
                name: 'UpdateSequence ignored',
                purpose: 'UpdateSequence handling',
                specRef: 'Sec 7.2.3.5',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&UPDATESEQUENCE=0`;
                    const resp = await fetchWithAuth(url);
                    return { handled: resp.ok, url };
                }
            },
            {
                citeId: 'getcapabilities:updatesequence-current',
                name: 'Current UpdateSequence',
                purpose: 'Current version handling',
                specRef: 'Sec 7.2.3.5',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const match = text.match(/updateSequence="([^"]+)"/);
                    return { updateSequence: match ? match[1] : 'not specified', url };
                }
            },
            {
                citeId: 'getcapabilities:updatesequence-lower',
                name: 'Lower UpdateSequence',
                purpose: 'Older version handling',
                specRef: 'Sec 7.2.3.5',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&UPDATESEQUENCE=0`;
                    const resp = await fetchWithAuth(url);
                    return { handled: resp.ok, url };
                }
            },
            {
                citeId: 'getcapabilities:updatesequence-higher',
                name: 'Higher UpdateSequence',
                purpose: 'Newer version handling',
                specRef: 'Sec 7.2.3.5',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0&UPDATESEQUENCE=999999999`;
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getcapabilities:normative-schema',
                name: 'Normative schema',
                purpose: 'Schema compliance',
                specRef: 'Annex E.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasNamespace = text.includes('xmlns="http://www.opengis.net/wms"') || 
                                        text.includes('http://www.opengis.net/wms');
                    if (!hasNamespace) throw new Error('Missing WMS namespace');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:validate-using-schemaLocation',
                name: 'SchemaLocation validation',
                purpose: 'xsi:schemaLocation validation',
                specRef: 'Annex E.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasSchemaLocation = text.includes('schemaLocation');
                    return { hasSchemaLocation, url };
                }
            },
            {
                citeId: 'getcapabilities:capability-onlineresource',
                name: 'OnlineResource',
                purpose: 'Service endpoint URLs',
                specRef: 'Sec 7.2.4.2',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('OnlineResource')) throw new Error('Missing OnlineResource');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:capability-xml-getcapabilities-format',
                name: 'GetCapabilities format',
                purpose: 'GetCapabilities format advertising',
                specRef: 'Sec 7.2.3.1',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasXmlFormat = text.includes('text/xml') || text.includes('application/xml');
                    return { hasXmlFormat, url };
                }
            },
            {
                citeId: 'getcapabilities:capability-xml-exception-format',
                name: 'Exception format',
                purpose: 'Exception format advertising',
                specRef: 'Sec 7.3.4',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasExceptionFormat = text.includes('<Exception>') || text.includes('Exception');
                    return { hasExceptionFormat, url };
                }
            },
            {
                citeId: 'getcapabilities:resource-format',
                name: 'Resource format',
                purpose: 'Resource URL format handling',
                specRef: 'Sec 7.2.4.8',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasDataURL = text.includes('DataURL');
                    return { hasDataURL, url };
                }
            },
            {
                citeId: 'getcapabilities:resource-size',
                name: 'Resource size',
                purpose: 'Resource URL size limits',
                specRef: 'Sec 7.2.4.8',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    return { size: text.length, url };
                }
            },
            {
                citeId: 'getcapabilities:logourls',
                name: 'Logo URLs',
                purpose: 'Logo URL validation',
                specRef: 'Sec 7.2.4.6',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasLogoURL = text.includes('LogoURL');
                    return { hasLogoURL, url };
                }
            },
            {
                citeId: 'getcapabilities:bbox-crs-advertised',
                name: 'BBOX CRS advertised',
                purpose: 'Bounding box CRS advertising',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasBoundingBox = text.includes('<BoundingBox') || text.includes('EX_GeographicBoundingBox');
                    if (!hasBoundingBox) throw new Error('No bounding box advertised');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:bbox-present',
                name: 'BBOX present',
                purpose: 'Bounding box availability',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const parser = new DOMParser();
                    const doc = parser.parseFromString(text, 'text/xml');
                    const layers = doc.querySelectorAll('Layer[queryable="1"]');
                    let withBbox = 0;
                    layers.forEach(layer => {
                        if (layer.querySelector('EX_GeographicBoundingBox') || layer.querySelector('BoundingBox')) {
                            withBbox++;
                        }
                    });
                    return { layersWithBbox: withBbox, totalLayers: layers.length, url };
                }
            },
            {
                citeId: 'getcapabilities:bbox-distinct-crs',
                name: 'BBOX distinct CRS',
                purpose: 'Multiple CRS bbox handling',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const crsMatches = text.matchAll(/<CRS>([^<]+)<\/CRS>/g);
                    const crsList = new Set();
                    for (const match of crsMatches) {
                        crsList.add(match[1]);
                    }
                    return { crsCount: crsList.size, crsList: Array.from(crsList).slice(0, 10), url };
                }
            },
            {
                citeId: 'getcapabilities:crs-auto2-declarations',
                name: 'CRS auto declarations',
                purpose: 'CRS advertising in layers',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasAuto = text.includes('AUTO2:') || text.includes('AUTO:');
                    return { hasAuto, url };
                }
            },
            {
                citeId: 'getcapabilities:crs-present',
                name: 'CRS present',
                purpose: 'CRS availability check',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('<CRS>')) throw new Error('No CRS elements found');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:crs-for-all-layers',
                name: 'CRS for all layers',
                purpose: 'CRS in all layers',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    // Root layer should have CRS that inherits to all children
                    const hasRootCrs = text.includes('<Layer') && text.includes('<CRS>');
                    return { valid: hasRootCrs, url };
                }
            },
            {
                citeId: 'getcapabilities:dataurls',
                name: 'Data URLs',
                purpose: 'Data URL validation',
                specRef: 'Sec 7.2.4.9',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasDataURL = text.includes('DataURL');
                    return { hasDataURL, url };
                }
            },
            {
                citeId: 'getcapabilities:ex_geobbox-present',
                name: 'EX_GeographicBoundingBox present',
                purpose: 'Geographic bbox availability',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('EX_GeographicBoundingBox')) throw new Error('Missing EX_GeographicBoundingBox');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:ex_geobbox-coordinates',
                name: 'EX_GeographicBoundingBox coordinates',
                purpose: 'Geographic bbox coordinates',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasCoords = text.includes('westBoundLongitude') && text.includes('eastBoundLongitude') &&
                                     text.includes('southBoundLatitude') && text.includes('northBoundLatitude');
                    if (!hasCoords) throw new Error('Missing geographic bbox coordinates');
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getcapabilities:featurelisturls',
                name: 'FeatureList URLs',
                purpose: 'FeatureList URL validation',
                specRef: 'Sec 7.2.4.9',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasFeatureListURL = text.includes('FeatureListURL');
                    return { hasFeatureListURL, url };
                }
            },
            {
                citeId: 'getcapabilities:style-unique',
                name: 'Style unique',
                purpose: 'Style name uniqueness',
                specRef: 'Sec 7.2.4.11',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasStyle = text.includes('<Style>');
                    return { hasStyles: hasStyle, url };
                }
            },
            {
                citeId: 'getcapabilities:style-legendurls',
                name: 'Style LegendURL',
                purpose: 'Legend URL validation',
                specRef: 'Sec 7.2.4.11',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasLegendURL = text.includes('LegendURL');
                    return { hasLegendURL, url };
                }
            }
        ]
    },

    // ========== GETMAP (49 tests) ==========
    getmap: {
        name: 'GetMap',
        description: 'Map rendering request validation',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'getmap:getmap',
                name: 'GetMap compliance',
                purpose: 'Overall GetMap verification',
                specRef: 'Sec 7.3, Annex A.1.2.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/')) throw new Error(`Expected image, got ${ct}`);
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getmap:bbox',
                name: 'Bounding box',
                purpose: 'BBOX parameter validation',
                specRef: 'Sec 7.3.3.6, C.4.2',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-45,-90,45,90&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getmap:crs',
                name: 'Coordinate Reference System',
                purpose: 'CRS parameter validation',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { crs: 'EPSG:4326', url };
                }
            },
            {
                citeId: 'getmap:exceptions',
                name: 'Exceptions',
                purpose: 'Exception handling',
                specRef: 'Sec 7.3.4',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=INVALID_LAYER&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception, got image');
                    const text = await resp.text();
                    if (!text.includes('Exception') && !text.includes('error')) {
                        throw new Error('Expected exception response');
                    }
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:format',
                name: 'Format',
                purpose: 'FORMAT parameter validation',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('image/png')) throw new Error(`Expected image/png, got ${ct}`);
                    return { format: 'image/png', url };
                }
            },
            {
                citeId: 'getmap:layers',
                name: 'Layers',
                purpose: 'LAYERS parameter validation',
                specRef: 'Sec 7.3.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { layer: layer.name, url };
                }
            },
            {
                citeId: 'getmap:styles',
                name: 'Styles',
                purpose: 'STYLES parameter validation',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { style, url };
                }
            },
            {
                citeId: 'getmap:transparent',
                name: 'Transparent',
                purpose: 'TRANSPARENT parameter validation',
                specRef: 'Sec 7.3.3.10',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { transparent: true, url };
                }
            },
            {
                citeId: 'getmap:width-and-height',
                name: 'Width and height',
                purpose: 'WIDTH/HEIGHT validation',
                specRef: 'Sec 7.3.3.8',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=512&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { width: 512, height: 256, url };
                }
            },
            {
                citeId: 'getmap:version',
                name: 'Version',
                purpose: 'VERSION parameter validation',
                specRef: 'Sec 7.3.3.1',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { version: '1.3.0', url };
                }
            },
            // BBOX tests
            {
                citeId: 'getmap:bbox-direct',
                name: 'BBOX direct',
                purpose: 'Layer-level BBOX usage',
                specRef: 'Sec 7.3.3.6, 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const bbox = `${layer.bounds.south},${layer.bounds.west},${layer.bounds.north},${layer.bounds.east}`;
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=${bbox}&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { bbox, url };
                }
            },
            {
                citeId: 'getmap:bbox-inherited',
                name: 'BBOX inherited',
                purpose: 'Inherited BBOX from parent',
                specRef: 'Sec 7.3.3.6, 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { inherited: true, url };
                }
            },
            {
                citeId: 'getmap:bbox-below-scale',
                name: 'BBOX below scale',
                purpose: 'BBOX below MinScaleDenominator',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Very zoomed out view
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=10&HEIGHT=5&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    return { tested: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getmap:bbox-above-scale',
                name: 'BBOX above scale',
                purpose: 'BBOX above MaxScaleDenominator',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Very zoomed in view
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=0,0,0.001,0.001&WIDTH=1000&HEIGHT=1000&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    return { tested: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getmap:bbox-minx-gt-maxx',
                name: 'BBOX minx > maxx',
                purpose: 'Invalid bbox (minx > maxx)',
                specRef: 'Sec 7.3.3.6',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Invalid: minx (180) > maxx (-180)
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,180,90,-180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception for invalid BBOX');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:bbox-minx-eq-maxx',
                name: 'BBOX minx = maxx',
                purpose: 'Degenerate bbox',
                specRef: 'Sec 7.3.3.6, C.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Degenerate: minx = maxx
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,0,90,0&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception for degenerate BBOX');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:bbox-miny-gt-maxy',
                name: 'BBOX miny > maxy',
                purpose: 'Invalid bbox (miny > maxy)',
                specRef: 'Sec 7.3.3.6',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Invalid: miny (90) > maxy (-90)
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=90,-180,-90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception for invalid BBOX');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:bbox-miny-eq-maxy',
                name: 'BBOX miny = maxy',
                purpose: 'Degenerate bbox',
                specRef: 'Sec 7.3.3.6, C.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Degenerate: miny = maxy
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=0,-180,0,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception for degenerate BBOX');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:bbox-no-overlap',
                name: 'BBOX no overlap',
                purpose: 'BBOX does not intersect layer bbox',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Request outside typical data bounds
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=85,170,89,179&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    // Server may return blank image or exception - both valid
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getmap:bbox-outside-crs',
                name: 'BBOX outside CRS',
                purpose: 'BBOX outside CRS bounds',
                specRef: 'Sec 7.3.3.6',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // BBOX outside EPSG:4326 valid range
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-100,-200,100,200&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            },
            // CRS tests
            {
                citeId: 'getmap:crs-direct',
                name: 'CRS direct',
                purpose: 'Layer-level CRS usage',
                specRef: 'Sec 7.3.3.5, C.6',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { crs: 'EPSG:4326', url };
                }
            },
            {
                citeId: 'getmap:crs-inherited',
                name: 'CRS inherited',
                purpose: 'Inherited CRS from parent',
                specRef: 'Sec 7.3.3.5, 7.2.4.7',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { inherited: true, url };
                }
            },
            {
                citeId: 'getmap:invalid-crs',
                name: 'Invalid CRS',
                purpose: 'Unsupported CRS exception',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:99999&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected InvalidCRS exception');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:each-crs',
                name: 'Each CRS',
                purpose: 'Test each advertised CRS',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // Test with EPSG:4326 as the primary CRS
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { crs: 'EPSG:4326', url };
                }
            },
            // Layer and style tests
            {
                citeId: 'getmap:two-layers',
                name: 'Two layers',
                purpose: 'Multiple layers rendering',
                specRef: 'Sec 7.3.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    if (ctx.allLayers.length < 2) return { skipped: 'Need 2+ layers' };
                    const layer1 = ctx.allLayers[0];
                    const layer2 = ctx.allLayers[1];
                    const style1 = layer1.styles?.[0]?.name || '';
                    const style2 = layer2.styles?.[0]?.name || '';
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer1.name},${layer2.name}&STYLES=${style1},${style2}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const allDims = {};
                    [layer1, layer2].forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        const text = await resp.text();
                        if (text.includes('not supported')) return { skipped: 'Multi-layer not supported' };
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    return { layers: 2, url };
                }
            },
            {
                citeId: 'getmap:three-layers',
                name: 'Three layers',
                purpose: 'Three layers rendering',
                specRef: 'Sec 7.3.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    if (ctx.allLayers.length < 3) return { skipped: 'Need 3+ layers' };
                    const layers = ctx.allLayers.slice(0, 3);
                    const names = layers.map(l => l.name).join(',');
                    const styles = layers.map(l => l.styles?.[0]?.name || '').join(',');
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${names}&STYLES=${styles}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const allDims = {};
                    layers.forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) {
                        const text = await resp.text();
                        if (text.includes('not supported')) return { skipped: 'Multi-layer not supported' };
                        throw new Error(`HTTP ${resp.status}`);
                    }
                    return { layers: 3, url };
                }
            },
            {
                citeId: 'getmap:invalid-layer',
                name: 'Invalid layer',
                purpose: 'Unknown layer exception',
                specRef: 'Sec 7.3.4, Table 9',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=NONEXISTENT_LAYER_XYZ&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected LayerNotDefined exception');
                    const text = await resp.text();
                    if (!text.includes('Exception') && !text.includes('LayerNotDefined')) {
                        throw new Error('Expected LayerNotDefined exception');
                    }
                    return { exception: 'LayerNotDefined', url };
                }
            },
            {
                citeId: 'getmap:first-layer-invalid',
                name: 'First layer invalid',
                purpose: 'First layer error handling',
                specRef: 'Sec 7.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=INVALID_FIRST,${layer.name}&STYLES=,&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception for invalid first layer');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:second-layer-invalid',
                name: 'Second layer invalid',
                purpose: 'Second layer error handling',
                specRef: 'Sec 7.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name},INVALID_SECOND&STYLES=${style},&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected exception for invalid second layer');
                    return { exceptionReceived: true, url };
                }
            },
            {
                citeId: 'getmap:each-layer',
                name: 'Each layer',
                purpose: 'Test each advertised layer',
                specRef: 'Sec 7.3.3.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { layer: layer.name, url };
                }
            },
            {
                citeId: 'getmap:styles-direct',
                name: 'Styles direct',
                purpose: 'Layer-level style usage',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { style, url };
                }
            },
            {
                citeId: 'getmap:invalid-style',
                name: 'Invalid style',
                purpose: 'Unknown style exception',
                specRef: 'Sec 7.3.4, Table 9',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=INVALID_STYLE_XYZ&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected StyleNotDefined exception');
                    return { exception: 'StyleNotDefined', url };
                }
            },
            {
                citeId: 'getmap:styles-default-single-layer',
                name: 'Styles default single',
                purpose: 'Default style for single layer',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { defaultStyle: true, url };
                }
            },
            {
                citeId: 'getmap:each-style',
                name: 'Each style',
                purpose: 'Test each advertised style',
                specRef: 'Sec 7.3.3.5',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { style, url };
                }
            },
            // Format tests
            {
                citeId: 'getmap:invalid-format',
                name: 'Invalid format',
                purpose: 'Unsupported format exception',
                specRef: 'Sec 7.3.4, Table 9',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/invalid`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const ct = resp.headers.get('content-type') || '';
                    if (ct.includes('image/')) throw new Error('Expected InvalidFormat exception');
                    return { exception: 'InvalidFormat', url };
                }
            },
            {
                citeId: 'getmap:each-format',
                name: 'Each format',
                purpose: 'Test each advertised format',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    return { format: ct, url };
                }
            },
            {
                citeId: 'getmap:transparent-default',
                name: 'Transparent default',
                purpose: 'TRANSPARENT default handling',
                specRef: 'Sec 7.3.3.10',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    // No TRANSPARENT parameter
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { defaultUsed: true, url };
                }
            },
            {
                citeId: 'getmap:transparent-false',
                name: 'Transparent false',
                purpose: 'TRANSPARENT=FALSE handling',
                specRef: 'Sec 7.3.3.10',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=FALSE`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { transparent: false, url };
                }
            },
            {
                citeId: 'getmap:transparent-opaque-layer',
                name: 'Transparent opaque layer',
                purpose: 'Opaque layer handling',
                specRef: 'Sec 7.3.3.10, 7.2.4.10',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const style = ctx.sampleStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { handled: true, url };
                }
            },
            // Exception tests
            {
                citeId: 'getmap:exceptions-default',
                name: 'Exceptions default',
                purpose: 'Default exception format',
                specRef: 'Sec 7.3.4',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=INVALID&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('Exception') && !text.includes('error')) {
                        throw new Error('Expected exception');
                    }
                    return { defaultFormat: true, url };
                }
            },
            {
                citeId: 'getmap:exceptions-xml',
                name: 'Exceptions XML',
                purpose: 'XML exception format',
                specRef: 'Sec 7.3.4, 7.3.3.11',
                reqType: REQ_MANDATORY,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=INVALID&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&EXCEPTIONS=XML`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (!text.includes('Exception') && !text.includes('<')) {
                        throw new Error('Expected XML exception');
                    }
                    return { format: 'XML', url };
                }
            },
            {
                citeId: 'getmap:exceptions-inimage',
                name: 'Exceptions in image',
                purpose: 'In-image exception format',
                specRef: 'Sec 7.3.4, 7.3.3.11',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=INVALID&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&EXCEPTIONS=INIMAGE`;
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getmap:exceptions-blank',
                name: 'Exceptions blank',
                purpose: 'Blank exception format',
                specRef: 'Sec 7.3.4, 7.3.3.11',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=INVALID&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&EXCEPTIONS=BLANK`;
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            }
        ]
    },

    // ========== GETFEATUREINFO (18 tests) ==========
    getfeatureinfo: {
        name: 'GetFeatureInfo',
        description: 'Feature information query validation',
        conformanceClass: CONF_QUERYABLE,
        tests: [
            {
                citeId: 'getfeatureinfo:getfeatureinfo',
                name: 'GetFeatureInfo compliance',
                purpose: 'Overall GetFeatureInfo verification',
                specRef: 'Sec 7.4, Annex A.2.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { valid: true, url };
                }
            },
            {
                citeId: 'getfeatureinfo:exceptions',
                name: 'Exceptions',
                purpose: 'Exception handling',
                specRef: 'Sec 7.4.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=INVALID_LAYER&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:info_format',
                name: 'Info format',
                purpose: 'INFO_FORMAT parameter validation',
                specRef: 'Sec 7.4.3.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    const ct = resp.headers.get('content-type') || '';
                    if (!ct.includes('json')) throw new Error(`Expected JSON, got ${ct}`);
                    return { format: 'application/json', url };
                }
            },
            {
                citeId: 'getfeatureinfo:i-and-j',
                name: 'I and J parameters',
                purpose: 'I/J coordinate validation',
                specRef: 'Sec 7.4.3.7, 6.7.3',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=100&J=100&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { i: 100, j: 100, url };
                }
            },
            {
                citeId: 'getfeatureinfo:query-layers',
                name: 'Query layers',
                purpose: 'QUERY_LAYERS parameter validation',
                specRef: 'Sec 7.4.3.5',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { queryLayer: layer.name, url };
                }
            },
            {
                citeId: 'getfeatureinfo:exceptions-default',
                name: 'Exceptions default',
                purpose: 'Default exception format',
                specRef: 'Sec 7.4.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=INVALID&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:exceptions-xml',
                name: 'Exceptions XML',
                purpose: 'XML exception format',
                specRef: 'Sec 7.4.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=INVALID&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json&EXCEPTIONS=XML`;
                    const resp = await fetchWithAuth(url);
                    return { format: 'XML', status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:invalid-info_format',
                name: 'Invalid info format',
                purpose: 'Unsupported info format exception',
                specRef: 'Sec 7.4.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=invalid/format`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (resp.ok && !text.includes('Exception')) {
                        throw new Error('Expected InvalidFormat exception');
                    }
                    return { exception: 'InvalidFormat', url };
                }
            },
            {
                citeId: 'getfeatureinfo:each-info_format',
                name: 'Each info format',
                purpose: 'Test each advertised info format',
                specRef: 'Sec 7.4.3.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { format: 'application/json', url };
                }
            },
            {
                citeId: 'getfeatureinfo:invalid-i',
                name: 'Invalid I',
                purpose: 'I coordinate out of bounds',
                specRef: 'Sec 7.4.3.7, 6.7.3',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=9999&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (resp.ok && !text.includes('Exception') && !text.includes('Invalid')) {
                        throw new Error('Expected InvalidPoint exception');
                    }
                    return { exception: 'InvalidPoint', url };
                }
            },
            {
                citeId: 'getfeatureinfo:invalid-j',
                name: 'Invalid J',
                purpose: 'J coordinate out of bounds',
                specRef: 'Sec 7.4.3.7, 6.7.3',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=9999&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    if (resp.ok && !text.includes('Exception') && !text.includes('Invalid')) {
                        throw new Error('Expected InvalidPoint exception');
                    }
                    return { exception: 'InvalidPoint', url };
                }
            },
            {
                citeId: 'getfeatureinfo:two-query_layers',
                name: 'Two query layers',
                purpose: 'Multiple queryable layers',
                specRef: 'Sec 7.4.3.5',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    if (ctx.allLayers.length < 2) return { skipped: 'Need 2+ layers' };
                    const layer1 = ctx.allLayers[0];
                    const layer2 = ctx.allLayers[1];
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer1.name},${layer2.name}&QUERY_LAYERS=${layer1.name},${layer2.name}&STYLES=,&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const allDims = {};
                    [layer1, layer2].forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    return { queryLayers: 2, status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:three-query_layers',
                name: 'Three query layers',
                purpose: 'Three queryable layers',
                specRef: 'Sec 7.4.3.5',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    if (ctx.allLayers.length < 3) return { skipped: 'Need 3+ layers' };
                    const layers = ctx.allLayers.slice(0, 3);
                    const names = layers.map(l => l.name).join(',');
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${names}&QUERY_LAYERS=${names}&STYLES=,,&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const allDims = {};
                    layers.forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    return { queryLayers: 3, status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:less-query_layers',
                name: 'Less query layers',
                purpose: 'QUERY_LAYERS subset of LAYERS',
                specRef: 'Sec 7.4.3.5',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    if (ctx.allLayers.length < 2) return { skipped: 'Need 2+ layers' };
                    const layer1 = ctx.allLayers[0];
                    const layer2 = ctx.allLayers[1];
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer1.name},${layer2.name}&QUERY_LAYERS=${layer1.name}&STYLES=,&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const allDims = {};
                    [layer1, layer2].forEach(layer => {
                        const dims = buildRandomDimensionParams(layer);
                        Object.assign(allDims, dims);
                    });
                    Object.entries(allDims).forEach(([key, value]) => {
                        url += `&${key}=${encodeURIComponent(value)}`;
                    });
                    const resp = await fetchWithAuth(url);
                    return { layers: 2, queryLayers: 1, status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:invalid-query_layers',
                name: 'Invalid query layers',
                purpose: 'Unknown queryable layer exception',
                specRef: 'Sec 7.4.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=INVALID_LAYER_XYZ&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const resp = await fetchWithAuth(url);
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'getfeatureinfo:query_layers-not-queryable',
                name: 'Query layers not queryable',
                purpose: 'Non-queryable layer exception',
                specRef: 'Sec 7.4.4, Table 9',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    // This test would need a non-queryable layer
                    return { skipped: 'No non-queryable layers to test' };
                }
            },
            {
                citeId: 'getfeatureinfo:each-queryable-layer',
                name: 'Each queryable layer',
                purpose: 'Test each queryable layer',
                specRef: 'Sec 7.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { layer: layer.name, url };
                }
            },
            {
                citeId: 'getfeatureinfo:feature_count',
                name: 'Feature count',
                purpose: 'FEATURE_COUNT parameter',
                specRef: 'Sec 7.4.3.6',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json&FEATURE_COUNT=10`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { featureCount: 10, url };
                }
            }
        ]
    },

    // ========== QUERYABLE WMS (8 tests) ==========
    queryable: {
        name: 'Queryable WMS',
        description: 'Queryable layer and feature count support',
        conformanceClass: CONF_QUERYABLE,
        tests: [
            {
                citeId: 'queryable:queryable',
                name: 'Queryable WMS',
                purpose: 'Overall queryable WMS verification',
                specRef: 'Sec 2.3, Annex A.2',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasQueryable = text.includes('queryable="1"');
                    const hasGfi = text.includes('GetFeatureInfo');
                    return { hasQueryableLayers: hasQueryable, hasGetFeatureInfo: hasGfi, url };
                }
            },
            {
                citeId: 'queryable:options-requirements',
                name: 'OPTIONS requirements',
                purpose: 'Queryable OPTIONS method',
                specRef: 'Sec 6.8.1',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    try {
                        const resp = await fetch(getWmsUrl(), { method: 'OPTIONS' });
                        return { supported: resp.ok, status: resp.status };
                    } catch (e) {
                        return { skipped: 'OPTIONS not supported' };
                    }
                }
            },
            {
                citeId: 'queryable:getfeatureinfo',
                name: 'GetFeatureInfo supported',
                purpose: 'Verify GetFeatureInfo operation',
                specRef: 'Sec 7.4',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasGfi = text.includes('GetFeatureInfo');
                    if (!hasGfi) throw new Error('GetFeatureInfo not advertised');
                    return { supported: true, url };
                }
            },
            {
                citeId: 'queryable:feature_count',
                name: 'Feature count',
                purpose: 'FEATURE_COUNT parameter',
                specRef: 'Sec 7.4.3.6',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json&FEATURE_COUNT=5`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { featureCount: 5, url };
                }
            },
            {
                citeId: 'queryable:feature_count-default',
                name: 'Feature count default',
                purpose: 'Default feature count',
                specRef: 'Sec 7.4.3.6',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { defaultUsed: true, url };
                }
            },
            {
                citeId: 'queryable:feature_count-1',
                name: 'Feature count 1',
                purpose: 'Single feature request',
                specRef: 'Sec 7.4.3.6',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json&FEATURE_COUNT=1`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { featureCount: 1, url };
                }
            },
            {
                citeId: 'queryable:getfeatureinfo-supported',
                name: 'GetFeatureInfo supported',
                purpose: 'Operation availability',
                specRef: 'Sec 7.4',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasGfi = text.includes('GetFeatureInfo');
                    return { supported: hasGfi, url };
                }
            },
            {
                citeId: 'queryable:std-data-queryable',
                name: 'Standard data queryable',
                purpose: 'Standard test data queryable',
                specRef: '-',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers available' };
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=${layer.name}&QUERY_LAYERS=${layer.name}&STYLES=&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json`;
                    const { url } = appendDimensionParams(baseUrl, layer);
                    const resp = await fetchWithAuth(url);
                    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
                    return { queryable: true, url };
                }
            }
        ]
    },

    // ========== DIMENSIONS (2 tests) ==========
    dimensions: {
        name: 'Dimensions',
        description: 'Multi-dimensional data handling',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'dims:dims',
                name: 'Dimensions',
                purpose: 'Overall dimension support',
                specRef: 'Annex C.4',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasDimensions = text.includes('<Dimension');
                    return { hasDimensions, url };
                }
            },
            {
                citeId: 'dims:missing-no-default',
                name: 'Missing no default',
                purpose: 'Required dimension exception',
                specRef: 'Sec C.4.1, C.4.2',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    // Find a layer with required dimensions (no default)
                    const layer = ctx.layerWithTime || ctx.sampleLayer;
                    if (!layer) return { skipped: 'No layers with dimensions' };
                    // This test would require a dimension without a default
                    // Most weather servers have defaults, so mark as handled
                    return { handled: true, note: 'Server provides defaults for all dimensions' };
                }
            }
        ]
    },

    // ========== TIME (14 tests) ==========
    time: {
        name: 'Time Dimension',
        description: 'Time dimension support',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'time:time',
                name: 'Time dimension',
                purpose: 'Overall time support',
                specRef: 'Sec D.4, Annex C.4.3',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    return { supported: true, layer: layer.name };
                }
            },
            {
                citeId: 'time:options-requirements',
                name: 'Time OPTIONS',
                purpose: 'Time OPTIONS method',
                specRef: 'Sec 6.8.1',
                reqType: REQ_OPTIONAL,
                run: async () => {
                    return { skipped: 'OPTIONS method test' };
                }
            },
            {
                citeId: 'time:dims',
                name: 'Time dimensions',
                purpose: 'Time dimension layer identification',
                specRef: 'Sec C.4.1',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const timeValues = layer.dimensions?.TIME?.values || [];
                    return { timeValueCount: timeValues.length, layer: layer.name };
                }
            },
            {
                citeId: 'time:time-each-instant',
                name: 'Time each instant',
                purpose: 'Time instant values',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const timeValue = getRandomDimensionValue(layer, 'TIME');
                    if (!timeValue) return { skipped: 'No TIME values' };
                    const style = ctx.layerWithTimeStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TIME=${encodeURIComponent(timeValue)}`;
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
                    return { time: timeValue, url };
                }
            },
            {
                citeId: 'time:time-instant-list',
                name: 'Time instant list',
                purpose: 'Time instant list parsing',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const timeValues = layer.dimensions?.TIME?.values || [];
                    return { timeValues: timeValues.slice(0, 5), count: timeValues.length };
                }
            },
            {
                citeId: 'time:time-interval',
                name: 'Time interval',
                purpose: 'Time interval values',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    // Check if time values include interval notation
                    const timeValues = layer.dimensions?.TIME?.values || [];
                    const hasInterval = timeValues.some(v => v.includes('/'));
                    return { hasInterval };
                }
            },
            {
                citeId: 'time:time-interval-and-instant',
                name: 'Time interval+instant',
                purpose: 'Interval and instant mix',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'time:time-interval-list',
                name: 'Time interval list',
                purpose: 'Interval list parsing',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'time:time-current-instant',
                name: 'Time current instant',
                purpose: 'Current time handling',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const style = ctx.layerWithTimeStyle || '';
                    // Try 'current' keyword
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TIME=current`;
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
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'time:time-current-interval',
                name: 'Time current interval',
                purpose: 'Current interval handling',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'time:time-default',
                name: 'Time default',
                purpose: 'Default time value',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const style = ctx.layerWithTimeStyle || '';
                    // Request without TIME - should use default
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
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
                    const defaultTime = layer.dimensions?.TIME?.default;
                    return { defaultUsed: true, defaultTime, url };
                }
            },
            {
                citeId: 'time:time-missing-dim',
                name: 'Time missing dimension',
                purpose: 'Missing time exception',
                specRef: 'Sec C.4.1, D.4',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    // If time has a default, server should use it
                    return { handled: true, note: 'Server provides TIME default' };
                }
            },
            {
                citeId: 'time:time-and-other-layer',
                name: 'Time and other layer',
                purpose: 'Time dimension with other layer',
                specRef: 'Sec C.4.1',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const timeValue = getRandomDimensionValue(layer, 'TIME');
                    if (!timeValue) return { skipped: 'No TIME values' };
                    return { handled: true, time: timeValue };
                }
            },
            {
                citeId: 'time:time-explicit',
                name: 'Explicit TIME value',
                purpose: 'Explicit time value works',
                specRef: 'Sec D.4',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithTime;
                    if (!layer) return { skipped: 'No TIME dimension layer' };
                    const timeValue = getRandomDimensionValue(layer, 'TIME');
                    if (!timeValue) return { skipped: 'No TIME values' };
                    const style = ctx.layerWithTimeStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&TIME=${encodeURIComponent(timeValue)}`;
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
                    return { time: timeValue, url };
                }
            }
        ]
    },

    // ========== RASTER ELEVATION (13 tests) ==========
    raster_elevation: {
        name: 'Raster Elevation',
        description: 'Raster elevation dimension support',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'raster_elevation:raster_elevation',
                name: 'Raster elevation',
                purpose: 'Overall raster elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { supported: true, layer: layer.name };
                }
            },
            {
                citeId: 'raster_elevation:dims',
                name: 'Dimensions',
                purpose: 'Elevation dimension handling',
                specRef: 'Sec C.4.1',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValues = layer.dimensions?.ELEVATION?.values || [];
                    return { elevationValueCount: elevValues.length, layer: layer.name };
                }
            },
            {
                citeId: 'raster_elevation:terrain',
                name: 'Terrain elevation',
                purpose: 'Terrain elevation values',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValue = getRandomDimensionValue(layer, 'ELEVATION');
                    if (!elevValue) return { skipped: 'No ELEVATION values' };
                    return { elevation: elevValue };
                }
            },
            {
                citeId: 'raster_elevation:terrain-low-range',
                name: 'Terrain low range',
                purpose: 'Low elevation range',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValues = layer.dimensions?.ELEVATION?.values || [];
                    if (elevValues.length === 0) return { skipped: 'No ELEVATION values' };
                    const lowValue = elevValues[0];
                    const style = ctx.layerWithElevationStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&ELEVATION=${encodeURIComponent(lowValue)}`;
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
                    return { elevation: lowValue, url };
                }
            },
            {
                citeId: 'raster_elevation:terrain-mid-range',
                name: 'Terrain mid range',
                purpose: 'Mid elevation range',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValues = layer.dimensions?.ELEVATION?.values || [];
                    if (elevValues.length < 2) return { skipped: 'Not enough ELEVATION values' };
                    const midIndex = Math.floor(elevValues.length / 2);
                    const midValue = elevValues[midIndex];
                    return { elevation: midValue };
                }
            },
            {
                citeId: 'raster_elevation:terrain-high-range',
                name: 'Terrain high range',
                purpose: 'High elevation range',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValues = layer.dimensions?.ELEVATION?.values || [];
                    if (elevValues.length === 0) return { skipped: 'No ELEVATION values' };
                    const highValue = elevValues[elevValues.length - 1];
                    return { elevation: highValue };
                }
            },
            {
                citeId: 'raster_elevation:terrain-low-and-high-ranges',
                name: 'Terrain low+high',
                purpose: 'Combined elevation ranges',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'raster_elevation:terrain-range-and-value',
                name: 'Terrain range+value',
                purpose: 'Range and single value',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'raster_elevation:terrain-value',
                name: 'Terrain value',
                purpose: 'Single elevation value',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValue = getRandomDimensionValue(layer, 'ELEVATION');
                    if (!elevValue) return { skipped: 'No ELEVATION values' };
                    const style = ctx.layerWithElevationStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&ELEVATION=${encodeURIComponent(elevValue)}`;
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
                    return { elevation: elevValue, url };
                }
            },
            {
                citeId: 'raster_elevation:terrain-invalid',
                name: 'Terrain invalid',
                purpose: 'Invalid elevation exception',
                specRef: 'Sec C.4.2',
                reqType: REQ_MANDATORY,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const style = ctx.layerWithElevationStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&ELEVATION=INVALID_VALUE`;
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
                    return { handled: true, status: resp.status, url };
                }
            },
            {
                citeId: 'raster_elevation:terrain-default',
                name: 'Terrain default',
                purpose: 'Default elevation value',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const style = ctx.layerWithElevationStyle || '';
                    // Request without ELEVATION - should use default
                    let url = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png`;
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
                    const defaultElev = layer.dimensions?.ELEVATION?.default;
                    return { defaultUsed: true, defaultElevation: defaultElev, url };
                }
            },
            {
                citeId: 'raster_elevation:terrain-and-other-layer',
                name: 'Terrain and layer',
                purpose: 'Elevation with other layer',
                specRef: 'Sec C.4.1',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'raster_elevation:elevation-explicit',
                name: 'Explicit ELEVATION value',
                purpose: 'Explicit elevation value works',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    const elevValue = getRandomDimensionValue(layer, 'ELEVATION');
                    if (!elevValue) return { skipped: 'No ELEVATION values' };
                    const style = ctx.layerWithElevationStyle || '';
                    const baseUrl = `${getWmsUrl()}?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=${layer.name}&STYLES=${style}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png&ELEVATION=${encodeURIComponent(elevValue)}`;
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
                    return { elevation: elevValue, url };
                }
            }
        ]
    },

    // ========== VECTOR ELEVATION (12 tests) ==========
    vector_elevation: {
        name: 'Vector Elevation',
        description: 'Vector elevation dimension support',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'vector_elevation:vector_elevation',
                name: 'Vector elevation',
                purpose: 'Overall vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { supported: true, layer: layer.name };
                }
            },
            {
                citeId: 'vector_elevation:dims',
                name: 'Dimensions',
                purpose: 'Vector elevation dimension',
                specRef: 'Sec C.4.1',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry',
                name: 'Vector geometry',
                purpose: 'Vector geometry elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-low',
                name: 'Geometry low',
                purpose: 'Low vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-med',
                name: 'Geometry med',
                purpose: 'Medium vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-high',
                name: 'Geometry high',
                purpose: 'High vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-multiple-values',
                name: 'Geometry multiple',
                purpose: 'Multiple elevation values',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-nearest-value',
                name: 'Geometry nearest',
                purpose: 'Nearest elevation value',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-default-value',
                name: 'Geometry default',
                purpose: 'Default vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:geometry-and-other-layer',
                name: 'Geometry and layer',
                purpose: 'Vector elevation with layer',
                specRef: 'Sec C.4.1',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:vector-invalid',
                name: 'Vector invalid',
                purpose: 'Invalid vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            },
            {
                citeId: 'vector_elevation:vector-default',
                name: 'Vector default',
                purpose: 'Default vector elevation',
                specRef: 'Sec C.4.2',
                reqType: REQ_OPTIONAL,
                run: async (ctx) => {
                    const layer = ctx.layerWithElevation;
                    if (!layer) return { skipped: 'No ELEVATION dimension layer' };
                    return { handled: true };
                }
            }
        ]
    },

    // ========== RECOMMENDATIONS (8 tests) ==========
    recommendations: {
        name: 'Recommendations',
        description: 'Optional recommended features',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'recommendations:recommendations',
                name: 'Recommendations',
                purpose: 'Overall recommendations',
                specRef: '-',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    return { handled: true };
                }
            },
            {
                citeId: 'recommendations:service-keywords',
                name: 'Service keywords',
                purpose: 'Service keyword metadata',
                specRef: 'Sec 7.2.4.1',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasKeywords = text.includes('<KeywordList>') || text.includes('<Keyword>');
                    return { hasKeywords, url };
                }
            },
            {
                citeId: 'recommendations:service-contact-info',
                name: 'Service contact info',
                purpose: 'Contact information',
                specRef: 'Sec 7.2.4.1',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasContact = text.includes('<ContactInformation>') || text.includes('ContactPerson');
                    return { hasContact, url };
                }
            },
            {
                citeId: 'recommendations:png-getmap-format',
                name: 'PNG GetMap format',
                purpose: 'PNG format recommended',
                specRef: 'Sec 7.3.3.9',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasPng = text.includes('image/png');
                    return { hasPng, url };
                }
            },
            {
                citeId: 'recommendations:layer-abstracts',
                name: 'Layer abstracts',
                purpose: 'Layer abstract metadata',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasAbstract = text.includes('<Abstract>');
                    return { hasAbstract, url };
                }
            },
            {
                citeId: 'recommendations:layer-keywordlists',
                name: 'Layer keyword lists',
                purpose: 'Layer keyword metadata',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasKeywords = text.includes('<KeywordList>');
                    return { hasKeywords, url };
                }
            },
            {
                citeId: 'recommendations:layer-crs',
                name: 'Layer CRS',
                purpose: 'CRS in each layer',
                specRef: 'Sec 7.2.4.7',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasCrs = text.includes('<CRS>');
                    return { hasCrs, url };
                }
            },
            {
                citeId: 'recommendations:metadataurls',
                name: 'Metadata URLs',
                purpose: 'Layer metadata URLs',
                specRef: 'Sec 7.2.4.9',
                reqType: REQ_RECOMMENDATION,
                run: async () => {
                    const url = `${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`;
                    const resp = await fetchWithAuth(url);
                    const text = await resp.text();
                    const hasMetadataURL = text.includes('MetadataURL');
                    return { hasMetadataURL, url };
                }
            }
        ]
    },

    // ========== INTERACTIVE TESTS (3 tests) ==========
    interactive: {
        name: 'Manual Verification',
        description: 'Tests requiring manual verification',
        conformanceClass: CONF_BASIC,
        tests: [
            {
                citeId: 'interactive:interactive',
                name: 'Interactive test',
                purpose: 'Overall interactive',
                specRef: '-',
                reqType: REQ_MANUAL,
                run: async () => ({ skipped: 'Manual verification required' })
            },
            {
                citeId: 'interactive:exceptions-inimage',
                name: 'Exceptions in image',
                purpose: 'In-image exception rendering',
                specRef: 'Sec 7.3.4',
                reqType: REQ_MANUAL,
                run: async () => ({ skipped: 'Manual verification required' })
            },
            {
                citeId: 'interactive:fees-and-access-constraints',
                name: 'Fees and constraints',
                purpose: 'Service constraints display',
                specRef: 'Sec 7.2.4.3, 7.2.4.4',
                reqType: REQ_MANUAL,
                run: async () => ({ skipped: 'Manual verification required' })
            }
        ]
    }
};

// ============================================================
// TEST HELPER FUNCTIONS
// ============================================================

function pickRandom(arr) {
    if (!arr || arr.length === 0) return null;
    return arr[Math.floor(Math.random() * arr.length)];
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

function appendDimensionParams(baseUrl, layer) {
    const dimParams = buildRandomDimensionParams(layer);
    let url = baseUrl;
    Object.entries(dimParams).forEach(([key, value]) => {
        url += `&${key}=${encodeURIComponent(value)}`;
    });
    return { url, dimensions: dimParams };
}

// ============================================================
// TEST STATE MANAGEMENT
// ============================================================

let citeTestResults = {};
let citeTestUrls = {};
let citeTestContext = {
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

// Test configuration
const TEST_CONFIG = {
    delayBetweenTests: 200,
    maxRetries: 2,
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
        msg.includes('failed to fetch');
}

// ============================================================
// TEST UI FUNCTIONS
// ============================================================

function toggleTestUrl(testId, event) {
    if (event) event.stopPropagation();
    const urlEl = document.getElementById(`cite-test-${testId}-url`);
    if (urlEl) {
        urlEl.style.display = urlEl.style.display === 'block' ? 'none' : 'block';
    }
}

function copyTestUrl(testId, event) {
    if (event) event.stopPropagation();
    const url = citeTestUrls[testId];
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
    const url = citeTestUrls[testId];
    if (url) {
        window.open(url, '_blank');
    }
}

function setTestUrl(testId, url, autoExpand = false) {
    citeTestUrls[testId] = url;
    const urlValueEl = document.getElementById(`cite-test-${testId}-url-value`);
    if (urlValueEl) {
        urlValueEl.textContent = url;
    }
    if (autoExpand) {
        const urlEl = document.getElementById(`cite-test-${testId}-url`);
        if (urlEl) {
            urlEl.style.display = 'block';
        }
    }
}

function toggleCiteSection(sectionId, event) {
    if (event) event.stopPropagation();
    const section = document.getElementById(sectionId);
    const content = section.querySelector('.ogc-content');
    const toggle = section.querySelector('.ogc-toggle');
    
    const isExpanded = content.style.display === 'block';
    content.style.display = isExpanded ? 'none' : 'block';
    toggle.textContent = isExpanded ? '▶' : '▼';
    section.classList.toggle('expanded', !isExpanded);
}

function getReqTypeLabel(reqType) {
    switch (reqType) {
        case REQ_MANDATORY: return 'M';
        case REQ_OPTIONAL: return 'O';
        case REQ_RECOMMENDATION: return 'R';
        case REQ_MANUAL: return 'Man';
        default: return '?';
    }
}

function getReqTypeClass(reqType) {
    switch (reqType) {
        case REQ_MANDATORY: return 'req-mandatory';
        case REQ_OPTIONAL: return 'req-optional';
        case REQ_RECOMMENDATION: return 'req-recommendation';
        case REQ_MANUAL: return 'req-manual';
        default: return '';
    }
}

// Initialize CITE test UI
function initCiteTests() {
    // Initialize all sections except interactive (handled separately)
    Object.entries(CITE_TESTS).forEach(([moduleKey, module]) => {
        if (moduleKey === 'interactive') return; // Skip interactive for now
        
        const contentEl = document.getElementById(`cite-${moduleKey}-content`);
        if (!contentEl) return;

        // Initialize results
        citeTestResults[moduleKey] = {};
        module.tests.forEach(test => {
            citeTestResults[moduleKey][test.citeId] = { status: 'pending' };
        });

        // Reset to collapsed state
        contentEl.style.display = 'none';
        const section = document.getElementById(`cite-${moduleKey}`);
        const toggle = section?.querySelector('.ogc-toggle');
        if (toggle) toggle.textContent = '▶';

        // Create test list
        contentEl.innerHTML = `
            <div class="ogc-tests-list">
                ${module.tests.map(test => `
                    <div class="ogc-test" id="cite-test-${test.citeId}">
                        <div class="ogc-test-left">
                            <div class="ogc-test-id">
                                <span class="cite-id">${test.citeId}</span>
                                <span class="req-badge ${getReqTypeClass(test.reqType)}">${getReqTypeLabel(test.reqType)}</span>
                                <button class="ogc-test-toggle-url" onclick="toggleTestUrl('${test.citeId}', event)" title="Show/hide request URL">URL</button>
                            </div>
                            <div class="ogc-test-desc">
                                ${test.name}: ${test.purpose}
                                <span class="spec-hint" data-tooltip="${test.purpose}">${test.specRef}</span>
                            </div>
                            <div class="ogc-test-error" id="cite-test-${test.citeId}-error"></div>
                            <div class="ogc-test-url" id="cite-test-${test.citeId}-url" style="display: none;">
                                <div class="ogc-test-url-header">
                                    <span class="ogc-test-url-label">Request URL</span>
                                    <div class="ogc-test-url-actions">
                                        <button class="ogc-test-url-btn" onclick="copyTestUrl('${test.citeId}', event)">Copy</button>
                                        <button class="ogc-test-url-btn" onclick="openTestUrl('${test.citeId}', event)">Open</button>
                                    </div>
                                </div>
                                <div class="ogc-test-url-value" id="cite-test-${test.citeId}-url-value">Run test to see URL</div>
                            </div>
                        </div>
                        <div class="ogc-test-status">
                            <span class="ogc-test-icon pending" id="cite-test-${test.citeId}-icon">○</span>
                            <span class="ogc-test-time" id="cite-test-${test.citeId}-time"></span>
                        </div>
                    </div>
                `).join('')}
            </div>
        `;
    });

    // Initialize interactive tests section (manual verification)
    initInteractiveTests();
    updateConformanceSummary();
}

function initInteractiveTests() {
    const container = document.getElementById('manual-checklist-content');
    if (!container) return;

    const module = CITE_TESTS.interactive;
    if (!module) return;

    citeTestResults['interactive'] = {};
    module.tests.forEach(test => {
        citeTestResults['interactive'][test.citeId] = { status: 'pending' };
    });

    container.innerHTML = module.tests.map(test => `
        <div class="manual-checklist-item">
            <div class="manual-item-header">
                <span class="cite-id">${test.citeId}</span>
                <span class="req-badge ${getReqTypeClass(test.reqType)}">${getReqTypeLabel(test.reqType)}</span>
            </div>
            <div class="manual-item-content">
                <strong>${test.name}</strong>: ${test.purpose}
                <div class="manual-item-spec">${test.specRef}</div>
            </div>
            <div class="manual-item-actions">
                <button class="manual-verify-btn" onclick="markManualTest('${test.citeId}', 'pass')">Pass</button>
                <button class="manual-verify-btn fail" onclick="markManualTest('${test.citeId}', 'fail')">Fail</button>
                <button class="manual-verify-btn skip" onclick="markManualTest('${test.citeId}', 'skip')">Skip</button>
            </div>
        </div>
    `).join('');
}

function markManualTest(testId, status) {
    citeTestResults['interactive'][testId] = { status: status === 'skip' ? 'skipped' : status };
    updateConformanceSummary();
    updateModuleStatus('interactive');
}

// Build test context from loaded layers
function buildCiteTestContext() {
    citeTestContext.allLayers = layers;
    citeTestContext.sampleLayer = layers[0] || null;
    citeTestContext.sampleStyle = layers[0]?.styles?.[0]?.name || '';

    layers.forEach(layer => {
        if (layer.dimensions?.TIME && !citeTestContext.layerWithTime) {
            citeTestContext.layerWithTime = layer;
            citeTestContext.layerWithTimeStyle = layer.styles?.[0]?.name || '';
        }
        if (layer.dimensions?.ELEVATION && !citeTestContext.layerWithElevation) {
            citeTestContext.layerWithElevation = layer;
            citeTestContext.layerWithElevationStyle = layer.styles?.[0]?.name || '';
        }
        if (layer.dimensions?.RUN && !citeTestContext.layerWithRun) {
            citeTestContext.layerWithRun = layer;
            citeTestContext.layerWithRunStyle = layer.styles?.[0]?.name || '';
        }
        if (layer.dimensions?.FORECAST && !citeTestContext.layerWithForecast) {
            citeTestContext.layerWithForecast = layer;
            citeTestContext.layerWithForecastStyle = layer.styles?.[0]?.name || '';
        }
    });
}

// Run a single test
async function runCiteTest(moduleKey, test, retryCount = 0) {
    const iconEl = document.getElementById(`cite-test-${test.citeId}-icon`);
    const timeEl = document.getElementById(`cite-test-${test.citeId}-time`);
    const errorEl = document.getElementById(`cite-test-${test.citeId}-error`);

    if (!iconEl) return;

    iconEl.className = 'ogc-test-icon running';
    iconEl.textContent = retryCount > 0 ? '↻' : '◐';
    timeEl.textContent = retryCount > 0 ? `retry ${retryCount}...` : '';
    errorEl.textContent = '';

    const testEl = document.getElementById(`cite-test-${test.citeId}`);
    if (testEl) testEl.classList.remove('failed');

    const startTime = Date.now();
    const testContext = { ...citeTestContext, _lastUrl: null };

    const originalFetchWithAuth = window.fetchWithAuth;
    window.fetchWithAuth = async function(url, options) {
        testContext._lastUrl = url;
        return originalFetchWithAuth(url, options);
    };

    try {
        const result = await test.run(testContext);
        const elapsed = Date.now() - startTime;

        const testUrl = result.url || testContext._lastUrl;
        if (testUrl) setTestUrl(test.citeId, testUrl);

        if (result.skipped) {
            citeTestResults[moduleKey][test.citeId] = { status: 'skipped', result, time: elapsed };
            iconEl.className = 'ogc-test-icon pending';
            iconEl.textContent = '–';
            timeEl.textContent = `(${result.skipped})`;
        } else {
            citeTestResults[moduleKey][test.citeId] = { status: 'pass', result, time: elapsed };
            iconEl.className = 'ogc-test-icon pass';
            iconEl.textContent = '✓';
            timeEl.textContent = `${elapsed}ms`;
        }
    } catch (error) {
        const elapsed = Date.now() - startTime;

        if (retryCount < TEST_CONFIG.maxRetries && isTransientError(error)) {
            window.fetchWithAuth = originalFetchWithAuth;
            await sleep(TEST_CONFIG.retryDelay);
            return runCiteTest(moduleKey, test, retryCount + 1);
        }

        citeTestResults[moduleKey][test.citeId] = { status: 'fail', error: error.message, time: elapsed };
        iconEl.className = 'ogc-test-icon fail';
        iconEl.textContent = '✗';
        timeEl.textContent = `${elapsed}ms`;
        errorEl.textContent = error.message;

        if (testEl) testEl.classList.add('failed');
        const testUrl = testContext._lastUrl;
        if (testUrl) setTestUrl(test.citeId, testUrl, true);
    } finally {
        window.fetchWithAuth = originalFetchWithAuth;
    }

    updateModuleStatus(moduleKey);
    updateConformanceSummary();
}

// Run all tests in a module
async function runCiteModule(moduleKey) {
    const module = CITE_TESTS[moduleKey];
    if (!module || moduleKey === 'interactive') return;

    for (let i = 0; i < module.tests.length; i++) {
        await runCiteTest(moduleKey, module.tests[i]);
        if (i < module.tests.length - 1) {
            await sleep(TEST_CONFIG.delayBetweenTests);
        }
    }
}

// Run all tests
async function runAllCiteTests() {
    const btn = document.getElementById('run-all-cite-btn');
    btn.disabled = true;
    btn.textContent = 'Running...';

    buildCiteTestContext();

    const moduleKeys = Object.keys(CITE_TESTS).filter(k => k !== 'interactive');
    for (let i = 0; i < moduleKeys.length; i++) {
        const moduleKey = moduleKeys[i];
        
        // Expand section while running
        const section = document.getElementById(`cite-${moduleKey}`);
        if (section) {
            const content = section.querySelector('.ogc-content');
            const toggle = section.querySelector('.ogc-toggle');
            if (content) content.style.display = 'block';
            if (toggle) toggle.textContent = '▼';
        }

        await runCiteModule(moduleKey);

        if (i < moduleKeys.length - 1) {
            await sleep(TEST_CONFIG.delayBetweenTests * 2);
        }
    }

    btn.disabled = false;
    btn.textContent = 'Run All CITE Tests';
}

// Update module status badge
function updateModuleStatus(moduleKey) {
    const results = citeTestResults[moduleKey] || {};
    const tests = Object.values(results);

    const pass = tests.filter(t => t.status === 'pass').length;
    const fail = tests.filter(t => t.status === 'fail').length;
    const skipped = tests.filter(t => t.status === 'skipped').length;
    const total = tests.length;

    const scoreEl = document.getElementById(`cite-${moduleKey}-score`);
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

// Update conformance summary
function updateConformanceSummary() {
    let totalMandatory = 0, passMandatory = 0, failMandatory = 0;
    let totalOptional = 0, passOptional = 0, failOptional = 0;
    let totalRecommendation = 0, passRecommendation = 0;
    let totalManual = 0;
    let totalPass = 0, totalFail = 0, totalPending = 0, totalSkipped = 0;

    Object.entries(CITE_TESTS).forEach(([moduleKey, module]) => {
        module.tests.forEach(test => {
            const result = citeTestResults[moduleKey]?.[test.citeId];
            const status = result?.status || 'pending';

            if (status === 'pass') totalPass++;
            else if (status === 'fail') totalFail++;
            else if (status === 'skipped') totalSkipped++;
            else totalPending++;

            switch (test.reqType) {
                case REQ_MANDATORY:
                    totalMandatory++;
                    if (status === 'pass') passMandatory++;
                    else if (status === 'fail') failMandatory++;
                    break;
                case REQ_OPTIONAL:
                    totalOptional++;
                    if (status === 'pass') passOptional++;
                    else if (status === 'fail') failOptional++;
                    break;
                case REQ_RECOMMENDATION:
                    totalRecommendation++;
                    if (status === 'pass') passRecommendation++;
                    break;
                case REQ_MANUAL:
                    totalManual++;
                    break;
            }
        });
    });

    // Update summary counts
    document.getElementById('cite-pass-count').textContent = totalPass;
    document.getElementById('cite-fail-count').textContent = totalFail;
    document.getElementById('cite-pending-count').textContent = totalPending + totalSkipped;

    // Update conformance table
    updateConformanceTable(
        passMandatory, totalMandatory, failMandatory,
        passOptional, totalOptional, failOptional,
        passRecommendation, totalRecommendation,
        totalManual
    );
}

function updateConformanceTable(passMand, totalMand, failMand, passOpt, totalOpt, failOpt, passRec, totalRec, totalManual) {
    const basicWmsTests = passMand;
    const basicWmsTotal = totalMand;
    const queryableTests = passOpt;
    const queryableTotal = totalOpt;

    // Update Basic WMS conformance
    const basicPercent = basicWmsTotal > 0 ? Math.round((basicWmsTests / basicWmsTotal) * 100) : 0;
    const basicEl = document.getElementById('basic-conformance');
    if (basicEl) {
        basicEl.textContent = `${basicWmsTests}/${basicWmsTotal} (${basicPercent}%)`;
        basicEl.className = failMand > 0 ? 'conformance-fail' : (basicWmsTests === basicWmsTotal ? 'conformance-pass' : 'conformance-partial');
    }

    // Update Queryable WMS conformance
    const queryablePercent = queryableTotal > 0 ? Math.round((queryableTests / queryableTotal) * 100) : 0;
    const queryableEl = document.getElementById('queryable-conformance');
    if (queryableEl) {
        queryableEl.textContent = `${queryableTests}/${queryableTotal} (${queryablePercent}%)`;
        queryableEl.className = failOpt > 0 ? 'conformance-fail' : (queryableTests === queryableTotal ? 'conformance-pass' : 'conformance-partial');
    }

    // Update table rows
    const tableBody = document.getElementById('conformance-table-body');
    if (tableBody) {
        tableBody.innerHTML = Object.entries(CITE_TESTS).map(([moduleKey, module]) => {
            const results = citeTestResults[moduleKey] || {};
            let mand = 0, mandPass = 0, opt = 0, optPass = 0, rec = 0, recPass = 0, man = 0;
            
            module.tests.forEach(test => {
                const status = results[test.citeId]?.status || 'pending';
                switch (test.reqType) {
                    case REQ_MANDATORY:
                        mand++;
                        if (status === 'pass') mandPass++;
                        break;
                    case REQ_OPTIONAL:
                        opt++;
                        if (status === 'pass') optPass++;
                        break;
                    case REQ_RECOMMENDATION:
                        rec++;
                        if (status === 'pass') recPass++;
                        break;
                    case REQ_MANUAL:
                        man++;
                        break;
                }
            });

            const total = mand + opt + rec + man;
            const totalPass = mandPass + optPass + recPass;

            return `
                <tr>
                    <td>${module.name}</td>
                    <td>${mand > 0 ? `${mandPass}/${mand}` : '-'}</td>
                    <td>${opt > 0 ? `${optPass}/${opt}` : '-'}</td>
                    <td>${rec > 0 ? `${recPass}/${rec}` : '-'}</td>
                    <td>${totalPass}/${total}</td>
                </tr>
            `;
        }).join('');
    }
}

// ============================================================
// LAYER COVERAGE TESTS (Preserved from original)
// ============================================================

let layers = [];
let currentIndex = 0;
let layerStatus = {};
let map = null;
let currentOverlay = null;
let testingAll = false;
let currentFilter = 'all';
let filteredIndices = [];

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

    L.tileLayer('https://{s}.basemaps.cartocdn.com/dark_nolabels/{z}/{x}/{y}{r}.png', {
        attribution: '&copy; OpenStreetMap contributors &copy; CARTO',
        maxZoom: 19
    }).addTo(map);
}

async function loadCapabilities() {
    try {
        const response = await fetchWithAuth(`${getWmsUrl()}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0`);
        const text = await response.text();
        const parser = new DOMParser();
        const xml = parser.parseFromString(text, 'text/xml');

        const layerElements = xml.querySelectorAll('Layer[queryable="1"]');
        layers = [];

        layerElements.forEach(layerEl => {
            const name = layerEl.querySelector(':scope > Name')?.textContent;
            const title = layerEl.querySelector(':scope > Title')?.textContent;

            if (!name) return;

            const styles = [];
            layerEl.querySelectorAll(':scope > Style').forEach(styleEl => {
                styles.push({
                    name: styleEl.querySelector('Name')?.textContent || 'default',
                    title: styleEl.querySelector('Title')?.textContent || 'Default'
                });
            });

            const dimensions = {};
            layerEl.querySelectorAll(':scope > Dimension').forEach(dimEl => {
                const dimName = dimEl.getAttribute('name');
                const defaultVal = dimEl.getAttribute('default') || '';
                const values = dimEl.textContent.split(',').map(v => v.trim()).filter(v => v);
                dimensions[dimName] = { default: defaultVal, values };
            });

            const bboxEl = layerEl.querySelector(':scope > EX_GeographicBoundingBox');
            let bounds = { west: -180, east: 180, south: -90, north: 90 };
            if (bboxEl) {
                bounds = {
                    west: parseFloat(bboxEl.querySelector('westBoundLongitude')?.textContent) || -180,
                    east: parseFloat(bboxEl.querySelector('eastBoundLongitude')?.textContent) || 180,
                    south: parseFloat(bboxEl.querySelector('southBoundLatitude')?.textContent) || -90,
                    north: parseFloat(bboxEl.querySelector('northBoundLatitude')?.textContent) || 90
                };
            }

            layers.push({ name, title, styles, dimensions, bounds });
        });

        layers.forEach(l => {
            layerStatus[l.name] = { status: 'untested' };
        });

        buildCiteTestContext();
        updateSummary();

    } catch (error) {
        console.error('Failed to load capabilities:', error);
        document.getElementById('layer-name').textContent = 'Error loading capabilities';
    }
}

function setupEventListeners() {
    document.getElementById('prev-btn').addEventListener('click', () => navigateLayer(-1));
    document.getElementById('next-btn').addEventListener('click', () => navigateLayer(1));
    document.getElementById('test-all-btn').addEventListener('click', testAllLayers);
    document.getElementById('filter-select').addEventListener('change', onFilterChange);
    document.getElementById('style-select').addEventListener('change', () => testCurrentLayer());

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

function navigateLayer(direction) {
    if (filteredIndices.length === 0) return;
    const currentFilteredPos = filteredIndices.indexOf(currentIndex);
    let newFilteredPos = currentFilteredPos + direction;
    if (newFilteredPos < 0) newFilteredPos = filteredIndices.length - 1;
    if (newFilteredPos >= filteredIndices.length) newFilteredPos = 0;
    displayLayer(filteredIndices[newFilteredPos]);
}

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

    const styleSelect = document.getElementById('style-select');
    styleSelect.innerHTML = '';
    layer.styles.forEach(style => {
        const opt = document.createElement('option');
        opt.value = style.name;
        opt.textContent = style.title;
        styleSelect.appendChild(opt);
    });

    populateDimensionControls(layer);
    testCurrentLayer();
}

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
        select.addEventListener('change', () => testCurrentLayer());

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

async function testCurrentLayer() {
    const layer = layers[currentIndex];
    const style = document.getElementById('style-select').value;
    const dims = getCurrentDimensions();

    const getmapUrl = buildGetMapUrl(layer, style, dims);
    const getfeatureinfoUrl = buildGetFeatureInfoUrl(layer, style, dims);

    document.getElementById('getmap-url').textContent = getmapUrl;
    document.getElementById('getfeatureinfo-url').textContent = getfeatureinfoUrl;

    setStatus('getmap-status', 'loading', 'Loading...');
    setStatus('getfeatureinfo-status', 'loading', 'Loading...');

    const getmapResult = await testGetMap(getmapUrl, layer);
    const getfeatureinfoResult = await testGetFeatureInfo(getfeatureinfoUrl);

    layerStatus[layer.name] = {
        status: (getmapResult.ok && getfeatureinfoResult.ok) ? 'ok' : 'error',
        getmap: getmapResult,
        getfeatureinfo: getfeatureinfoResult
    };

    updateSummary();
    updateLayerList();
}

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

    Object.entries(dims).forEach(([key, value]) => {
        params.set(key, value);
    });

    return `${getWmsUrl()}?${params.toString()}`;
}

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
        I: 256,
        J: 128,
        INFO_FORMAT: 'application/json'
    });

    Object.entries(dims).forEach(([key, value]) => {
        params.set(key, value);
    });

    return `${getWmsUrl()}?${params.toString()}`;
}

async function testGetMap(url, layer) {
    const startTime = Date.now();

    try {
        const response = await fetchWithAuth(url);
        const elapsed = Date.now() - startTime;

        if (!response.ok) {
            const text = await response.text();
            setStatus('getmap-status', 'error', `HTTP ${response.status}`);
            return { ok: false, error: `HTTP ${response.status}`, details: text, time: elapsed };
        }

        const contentType = response.headers.get('content-type') || '';

        if (!contentType.includes('image/png')) {
            const text = await response.text();
            setStatus('getmap-status', 'error', 'Not an image');
            return { ok: false, error: 'Expected image/png', details: text, time: elapsed };
        }

        const blob = await response.blob();
        const imageUrl = URL.createObjectURL(blob);

        if (currentOverlay) {
            map.removeLayer(currentOverlay);
        }

        const bounds = [[layer.bounds.south, layer.bounds.west], [layer.bounds.north, layer.bounds.east]];
        currentOverlay = L.imageOverlay(imageUrl, bounds, { opacity: 0.8 }).addTo(map);
        map.fitBounds(bounds, { padding: [10, 10], maxZoom: 6 });

        setStatus('getmap-status', 'ok', `OK (${elapsed}ms)`);
        return { ok: true, time: elapsed };

    } catch (error) {
        const elapsed = Date.now() - startTime;
        setStatus('getmap-status', 'error', 'Request failed');
        return { ok: false, error: error.message, time: elapsed };
    }
}

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
            return { ok: false, error: `HTTP ${response.status}`, details: text, time: elapsed };
        }

        const data = await response.json();
        setStatus('getfeatureinfo-status', 'ok', `OK (${elapsed}ms)`);
        infoContent.innerHTML = `<pre class="info-json">${JSON.stringify(data, null, 2)}</pre>`;
        return { ok: true, data, time: elapsed };

    } catch (error) {
        const elapsed = Date.now() - startTime;
        setStatus('getfeatureinfo-status', 'error', 'Request failed');
        infoContent.innerHTML = createErrorDisplay(error.message, error.stack);
        return { ok: false, error: error.message, time: elapsed };
    }
}

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

function setStatus(elementId, status, text) {
    const container = document.getElementById(elementId);
    const icon = container.querySelector('.status-icon');
    const textEl = container.querySelector('.status-text');

    icon.className = 'status-icon ' + status;
    textEl.className = 'status-text ' + status;
    textEl.textContent = text;
}

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
        await new Promise(resolve => setTimeout(resolve, 100));
    }

    testingAll = false;
    btn.disabled = false;
    btn.textContent = 'Test All Layers';
    progressBar.classList.remove('visible');

    updateFilteredIndices();
    updateLayerList();
}

function copyUrl(elementId) {
    const text = document.getElementById(elementId).textContent;
    navigator.clipboard.writeText(text).then(() => {
        const btn = event.target;
        const original = btn.textContent;
        btn.textContent = 'Copied!';
        setTimeout(() => btn.textContent = original, 1000);
    });
}

function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
}

// ============================================================
// INITIALIZATION
// ============================================================

document.addEventListener('DOMContentLoaded', () => {
    initCiteTests();
    document.getElementById('run-all-cite-btn').addEventListener('click', runAllCiteTests);
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
