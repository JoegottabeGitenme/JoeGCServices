/**
 * EDR Coverage Validation Tool
 * 
 * Systematically validates that every piece of data advertised by the EDR API
 * can actually be retrieved.
 */

// Smart endpoint detection
const IS_LOCAL_DEV = window.location.port === '8000';
const DEFAULT_EDR_ENDPOINT = IS_LOCAL_DEV ? 'http://localhost:8083/edr' : `${window.location.origin}/edr`;

class EDRCoverageValidator {
    constructor() {
        this.endpoint = DEFAULT_EDR_ENDPOINT;
        this.mode = 'full';
        this.queryType = 'position'; // 'position', 'area', 'radius', 'trajectory', 'corridor', 'cube'
        this.outputFormat = 'covjson'; // 'covjson', 'geojson', or 'both'
        this.testPoint = { lon: -100, lat: 40 };
        this.testArea = { lon: -100, lat: 40 }; // Center of 1x1 degree test polygon
        this.testRadius = { lon: -100, lat: 40, within: 50, units: 'km' }; // Radius query config
        this.testTrajectory = { coords: 'LINESTRING(-100 40,-99 40.5,-98 41)' }; // Trajectory query config
        this.testCorridor = { 
            coords: 'LINESTRING(-100 40,-99 40.5,-98 41)', 
            corridorWidth: 10, 
            widthUnits: 'km',
            corridorHeight: 1000,
            heightUnits: 'm'
        }; // Corridor query config
        this.testCube = {
            bbox: '-101,39,-100,40',
            z: '850',
            resolutionX: 10,
            resolutionY: 10
        }; // Cube query config
        this.testLocations = {
            locationId: 'KJFK',
            availableLocations: [] // Populated from API
        }; // Locations query config
        this.randomizeLocations = false; // When true, pick random coords within collection bbox
        this.running = false;
        this.abortController = null;
        this.concurrency = 20; // Max concurrent requests
        
        // Results tracking
        this.results = {
            advertised: {},
            catalog: {},
            checks: [],
            summary: { pass: 0, fail: 0, warn: 0, skip: 0 }
        };
        
        // Request log
        this.requestLog = [];
        
        // Progress tracking
        this.totalChecks = 0;
        this.completedChecks = 0;
        
        // Selected item for details panel
        this.selectedItem = null;
        
        // Whether catalog-check endpoint is available (our custom extension)
        this.hasCatalogCheck = false;
        
        // Collection filter - array of collection IDs to test (empty = all)
        this.collectionFilter = [];
        
        // Available collections (loaded from API)
        this.availableCollections = [];
    }

    /**
     * Initialize the validator and bind event listeners
     */
    init() {
        // Load saved endpoint or use default
        const savedEndpoint = localStorage.getItem('edr-coverage-endpoint');
        if (savedEndpoint) {
            document.getElementById('endpoint-input').value = savedEndpoint;
            this.endpoint = savedEndpoint;
        } else {
            // Set default value in the input
            document.getElementById('endpoint-input').value = this.endpoint;
        }

        // Bind buttons
        document.getElementById('apply-btn').addEventListener('click', () => this.applyEndpoint());
        document.getElementById('run-btn').addEventListener('click', () => this.run());
        document.getElementById('stop-btn').addEventListener('click', () => this.stop());
        document.getElementById('clear-btn').addEventListener('click', () => this.clear());
        document.getElementById('export-btn').addEventListener('click', () => this.exportJSON());

        // Bind mode select
        document.getElementById('mode-select').addEventListener('change', (e) => {
            this.mode = e.target.value;
        });

        // Bind format select
        document.getElementById('format-select').addEventListener('change', (e) => {
            this.outputFormat = e.target.value;
        });

        // Bind query type select
        document.getElementById('query-type-select').addEventListener('change', (e) => {
            this.queryType = e.target.value;
            // Toggle visibility of point vs area vs radius vs trajectory vs corridor vs cube vs locations config
            document.getElementById('test-point-config').style.display = 
                this.queryType === 'position' ? '' : 'none';
            document.getElementById('test-area-config').style.display = 
                this.queryType === 'area' ? '' : 'none';
            document.getElementById('test-radius-config').style.display = 
                this.queryType === 'radius' ? '' : 'none';
            document.getElementById('test-trajectory-config').style.display = 
                this.queryType === 'trajectory' ? '' : 'none';
            document.getElementById('test-corridor-config').style.display = 
                this.queryType === 'corridor' ? '' : 'none';
            document.getElementById('test-cube-config').style.display = 
                this.queryType === 'cube' ? '' : 'none';
            document.getElementById('test-locations-config').style.display = 
                this.queryType === 'locations' ? '' : 'none';
            
            // Load available locations when switching to locations query type
            if (this.queryType === 'locations') {
                this.loadAvailableLocations();
            }
        });

        // Bind test point inputs
        document.getElementById('test-lon').addEventListener('change', (e) => {
            this.testPoint.lon = parseFloat(e.target.value) || -100;
        });
        document.getElementById('test-lat').addEventListener('change', (e) => {
            this.testPoint.lat = parseFloat(e.target.value) || 40;
        });

        // Bind test area inputs
        document.getElementById('test-area-lon').addEventListener('change', (e) => {
            this.testArea.lon = parseFloat(e.target.value) || -100;
        });
        document.getElementById('test-area-lat').addEventListener('change', (e) => {
            this.testArea.lat = parseFloat(e.target.value) || 40;
        });

        // Bind test radius inputs
        document.getElementById('test-radius-lon').addEventListener('change', (e) => {
            this.testRadius.lon = parseFloat(e.target.value) || -100;
        });
        document.getElementById('test-radius-lat').addEventListener('change', (e) => {
            this.testRadius.lat = parseFloat(e.target.value) || 40;
        });
        document.getElementById('test-radius-within').addEventListener('change', (e) => {
            this.testRadius.within = parseFloat(e.target.value) || 50;
        });
        document.getElementById('test-radius-units').addEventListener('change', (e) => {
            this.testRadius.units = e.target.value || 'km';
        });

        // Bind test trajectory input
        document.getElementById('test-trajectory-coords').addEventListener('change', (e) => {
            this.testTrajectory.coords = e.target.value || 'LINESTRING(-100 40,-99 40.5,-98 41)';
        });

        // Bind test corridor inputs
        document.getElementById('test-corridor-coords').addEventListener('change', (e) => {
            this.testCorridor.coords = e.target.value || 'LINESTRING(-100 40,-99 40.5,-98 41)';
        });
        document.getElementById('test-corridor-width').addEventListener('change', (e) => {
            this.testCorridor.corridorWidth = parseFloat(e.target.value) || 10;
        });
        document.getElementById('test-corridor-width-units').addEventListener('change', (e) => {
            this.testCorridor.widthUnits = e.target.value || 'km';
        });
        document.getElementById('test-corridor-height').addEventListener('change', (e) => {
            this.testCorridor.corridorHeight = parseFloat(e.target.value) || 1000;
        });
        document.getElementById('test-corridor-height-units').addEventListener('change', (e) => {
            this.testCorridor.heightUnits = e.target.value || 'm';
        });

        // Bind test cube inputs
        document.getElementById('test-cube-bbox').addEventListener('change', (e) => {
            this.testCube.bbox = e.target.value || '-101,39,-100,40';
        });
        document.getElementById('test-cube-z').addEventListener('change', (e) => {
            this.testCube.z = e.target.value || '850';
        });
        document.getElementById('test-cube-resolution-x').addEventListener('change', (e) => {
            this.testCube.resolutionX = parseInt(e.target.value) || 10;
        });
        document.getElementById('test-cube-resolution-y').addEventListener('change', (e) => {
            this.testCube.resolutionY = parseInt(e.target.value) || 10;
        });

        // Bind test locations inputs
        document.getElementById('test-location-id').addEventListener('change', (e) => {
            this.testLocations.locationId = e.target.value || 'KJFK';
        });
        document.getElementById('refresh-locations-btn').addEventListener('click', () => {
            this.loadAvailableLocations();
        });
        document.getElementById('location-select').addEventListener('change', (e) => {
            if (e.target.value) {
                this.testLocations.locationId = e.target.value;
                document.getElementById('test-location-id').value = e.target.value;
            }
        });

        // Bind randomize toggle
        document.getElementById('randomize-toggle').addEventListener('change', (e) => {
            this.randomizeLocations = e.target.checked;
            // Disable/enable manual coordinate inputs when randomize is on
            const pointInputs = document.querySelectorAll('#test-point-config input');
            const areaInputs = document.querySelectorAll('#test-area-config input');
            const radiusInputs = document.querySelectorAll('#test-radius-config input:not(#test-radius-within)');
            const trajectoryInputs = document.querySelectorAll('#test-trajectory-config input');
            const corridorCoordInputs = document.querySelectorAll('#test-corridor-config input[type="text"]');
            pointInputs.forEach(input => input.disabled = this.randomizeLocations);
            areaInputs.forEach(input => input.disabled = this.randomizeLocations);
            radiusInputs.forEach(input => input.disabled = this.randomizeLocations);
            trajectoryInputs.forEach(input => input.disabled = this.randomizeLocations);
            corridorCoordInputs.forEach(input => input.disabled = this.randomizeLocations);
        });

        // Bind collection filter controls
        document.getElementById('select-all-btn').addEventListener('click', () => this.selectAllCollections());
        document.getElementById('select-none-btn').addEventListener('click', () => this.selectNoCollections());
        document.getElementById('refresh-collections-btn').addEventListener('click', () => this.loadCollectionList());
        document.getElementById('collection-filter-select').addEventListener('change', (e) => {
            this.updateCollectionFilter();
        });

        // Bind summary stat clicks to filter the tree view
        document.querySelectorAll('.summary-stats .stat').forEach(stat => {
            stat.style.cursor = 'pointer';
            stat.addEventListener('click', (e) => {
                const statEl = e.currentTarget;
                if (statEl.classList.contains('pass')) {
                    this.setFilter('pass');
                } else if (statEl.classList.contains('fail')) {
                    this.setFilter('fail');
                } else if (statEl.classList.contains('warn')) {
                    this.setFilter('warn');
                } else if (statEl.classList.contains('skip')) {
                    this.setFilter('all'); // Skip shows all (no specific skip filter)
                }
            });
        });

        // Load collection list on init
        this.loadCollectionList();
    }

    /**
     * Load the list of available collections from the API
     */
    async loadCollectionList() {
        const select = document.getElementById('collection-filter-select');
        select.innerHTML = '<option value="" disabled>Loading...</option>';
        
        try {
            const response = await fetch(`${this.endpoint}/collections`);
            if (!response.ok) {
                throw new Error(`HTTP ${response.status}`);
            }
            const data = await response.json();
            this.availableCollections = data.collections || [];
            
            // Populate select
            select.innerHTML = '';
            this.availableCollections.forEach(col => {
                const option = document.createElement('option');
                option.value = col.id;
                option.textContent = col.title || col.id;
                option.title = col.description || col.id;
                select.appendChild(option);
            });
            
            if (this.availableCollections.length === 0) {
                select.innerHTML = '<option value="" disabled>No collections found</option>';
            }
        } catch (e) {
            console.error('Failed to load collections:', e);
            select.innerHTML = '<option value="" disabled>Error loading collections</option>';
        }
    }

    /**
     * Select all collections in the filter
     */
    selectAllCollections() {
        const select = document.getElementById('collection-filter-select');
        Array.from(select.options).forEach(opt => opt.selected = true);
        this.updateCollectionFilter();
    }

    /**
     * Clear collection filter selection
     */
    selectNoCollections() {
        const select = document.getElementById('collection-filter-select');
        Array.from(select.options).forEach(opt => opt.selected = false);
        this.updateCollectionFilter();
    }

    /**
     * Update the collection filter from the select element
     */
    updateCollectionFilter() {
        const select = document.getElementById('collection-filter-select');
        this.collectionFilter = Array.from(select.selectedOptions).map(opt => opt.value);
    }

    /**
     * Apply the endpoint URL
     */
    applyEndpoint() {
        const input = document.getElementById('endpoint-input');
        this.endpoint = input.value.replace(/\/$/, ''); // Remove trailing slash
        localStorage.setItem('edr-coverage-endpoint', this.endpoint);
        this.updateStatus('Endpoint updated');
    }

    /**
     * Main run method - orchestrates the validation
     */
    async run() {
        if (this.running) return;

        this.running = true;
        this.abortController = new AbortController();
        this.requestLog = [];
        this.results = {
            advertised: {},
            catalog: {},
            checks: [],
            summary: { pass: 0, fail: 0, warn: 0, skip: 0 }
        };

        // Update UI
        document.getElementById('run-btn').disabled = true;
        document.getElementById('stop-btn').disabled = false;
        document.body.classList.add('running');
        this.updateProgress(0, 'Starting validation...');
        this.updateSummary();

        try {
            // Phase 1: Discovery
            this.updateStatus('Phase 1: Discovering advertised data...');
            await this.discoverAdvertised();

            // Phase 2: Try catalog check (optional - our custom extension)
            this.updateStatus('Phase 2: Checking catalog inventory...');
            await this.getCatalogInventory();

            // Phase 3: Compare (only if catalog check succeeded)
            if (this.hasCatalogCheck) {
                this.updateStatus('Phase 3: Comparing advertised vs. available...');
                this.compareAdvertisedVsCatalog();
            } else {
                // Initialize parameter checks without catalog data
                this.initializeParameterChecksWithoutCatalog();
            }

            // Phase 4: HTTP verification (skip only in quick mode when we have catalog data)
            const skipHttp = this.mode === 'quick' && this.hasCatalogCheck;
            if (!skipHttp && this.running) {
                const phaseNum = this.hasCatalogCheck ? 4 : 3;
                this.updateStatus(`Phase ${phaseNum}: Verifying data retrieval...`);
                await this.verifyDataRetrieval();
            }

            // Build results tree
            this.renderTree();
            this.renderGapSummary();

            const progressClass = this.results.summary.fail > 0 ? 'error' : 'complete';
            document.getElementById('progress-fill').className = 'progress-fill ' + progressClass;
            this.updateStatus(`Complete: ${this.results.summary.pass} pass, ${this.results.summary.fail} fail, ${this.results.summary.warn} warn`);

        } catch (error) {
            if (error.name !== 'AbortError') {
                console.error('Validation error:', error);
                this.updateStatus('Error: ' + error.message);
                document.getElementById('progress-fill').className = 'progress-fill error';
            }
        } finally {
            this.running = false;
            document.getElementById('run-btn').disabled = false;
            document.getElementById('stop-btn').disabled = true;
            document.body.classList.remove('running');
        }
    }

    /**
     * Stop the validation
     */
    stop() {
        if (this.abortController) {
            this.abortController.abort();
        }
        this.running = false;
        this.updateStatus('Stopped');
    }

    /**
     * Clear all results
     */
    clear() {
        this.results = {
            advertised: {},
            catalog: {},
            checks: [],
            summary: { pass: 0, fail: 0, warn: 0, skip: 0 }
        };
        this.requestLog = [];
        this.totalChecks = 0;
        this.completedChecks = 0;
        this.selectedItem = null;
        this.hasCatalogCheck = false;

        document.getElementById('collections-tree').innerHTML = '<div class="loading">Click "Run Validation" to start...</div>';
        document.getElementById('details-content').innerHTML = '<p class="placeholder">Select an item from the tree to see details</p>';
        document.getElementById('log-content').innerHTML = '<div class="log-empty">No requests logged yet</div>';
        document.getElementById('log-count').textContent = '0 requests';
        document.getElementById('gap-summary-panel').style.display = 'none';
        document.getElementById('progress-fill').style.width = '0%';
        document.getElementById('progress-fill').className = 'progress-fill';
        this.updateSummary();
        this.updateStatus('Ready to validate');
    }

    /**
     * Export results as JSON
     */
    exportJSON() {
        const exportData = {
            timestamp: new Date().toISOString(),
            endpoint: this.endpoint,
            mode: this.mode,
            queryType: this.queryType,
            collectionFilter: this.collectionFilter.length > 0 ? this.collectionFilter : 'all',
            testPoint: this.queryType === 'position' ? this.testPoint : null,
            testArea: this.queryType === 'area' ? this.testArea : null,
            testTrajectory: this.queryType === 'trajectory' ? this.testTrajectory : null,
            testCorridor: this.queryType === 'corridor' ? this.testCorridor : null,
            testCube: this.queryType === 'cube' ? this.testCube : null,
            testLocations: this.queryType === 'locations' ? { locationId: this.testLocations.locationId } : null,
            hasCatalogCheck: this.hasCatalogCheck,
            summary: this.results.summary,
            advertised: this.results.advertised,
            catalog: this.hasCatalogCheck ? this.results.catalog : null,
            checks: this.results.checks,
            requestLog: this.requestLog
        };

        const blob = new Blob([JSON.stringify(exportData, null, 2)], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `edr-coverage-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
    }

    /**
     * Phase 1: Discover advertised data from EDR API
     */
    async discoverAdvertised() {
        const collectionsResp = await this.fetchJson(`${this.endpoint}/collections`);
        
        // Get list of collections to process
        let collectionsToProcess = collectionsResp.collections || [];
        
        // Apply collection filter if set
        if (this.collectionFilter.length > 0) {
            collectionsToProcess = collectionsToProcess.filter(
                coll => this.collectionFilter.includes(coll.id)
            );
            this.updateStatus(`Phase 1: Discovering ${collectionsToProcess.length} selected collection(s)...`);
        }
        
        for (const coll of collectionsToProcess) {
            if (!this.running) break;

            // Get full collection details
            const collDetail = await this.fetchJson(`${this.endpoint}/collections/${coll.id}`);
            
            // Get instances
            let instances = [];
            try {
                const instancesResp = await this.fetchJson(`${this.endpoint}/collections/${coll.id}/instances`);
                instances = (instancesResp.instances || []).map(i => i.id);
            } catch (e) {
                // Instances might not be supported
            }

            this.results.advertised[coll.id] = {
                id: coll.id,
                title: coll.title || coll.id,
                description: coll.description || '',
                parameters: Object.keys(collDetail.parameter_names || {}),
                levels: this.extractLevels(collDetail.extent?.vertical),
                temporalValues: collDetail.extent?.temporal?.values || [],
                instances: instances,
                bbox: collDetail.extent?.spatial?.bbox?.[0],
                model: this.extractModelFromCollectionId(coll.id)
            };
        }
    }

    /**
     * Extract numeric levels from vertical extent
     */
    extractLevels(vertical) {
        if (!vertical || !vertical.interval) return [];
        
        const levels = [];
        for (const interval of vertical.interval) {
            if (interval[0] !== null) {
                levels.push(interval[0]);
            }
        }
        return levels;
    }

    /**
     * Extract model name from collection ID (e.g., "hrrr-isobaric" -> "hrrr")
     */
    extractModelFromCollectionId(collId) {
        return collId.split('-')[0];
    }

    /**
     * Phase 2: Get catalog inventory (optional - our custom extension)
     * This endpoint is specific to our weather-wms server; other EDR servers won't have it.
     */
    async getCatalogInventory() {
        try {
            this.results.catalog = await this.fetchJson(`${this.endpoint}/catalog-check`);
            this.hasCatalogCheck = true;
            this.updateStatus('Phase 2: Catalog check available - comparing data...');
        } catch (e) {
            // Catalog-check endpoint not available (expected for standard EDR servers)
            this.hasCatalogCheck = false;
            this.results.catalog = {};
            console.log('Catalog-check endpoint not available (this is normal for standard EDR servers)');
            this.updateStatus('Phase 2: Catalog check not available - will verify via HTTP requests...');
        }
    }

    /**
     * Initialize parameter checks when catalog-check is not available
     * Mark everything as needing HTTP verification
     */
    initializeParameterChecksWithoutCatalog() {
        for (const [collId, coll] of Object.entries(this.results.advertised)) {
            coll.parameterChecks = {};

            for (const param of coll.parameters) {
                coll.parameterChecks[param] = {
                    inCatalog: null, // Unknown - needs HTTP verification
                    catalogLevels: [],
                    catalogCount: null,
                    levels: {},
                    message: 'Will verify via HTTP'
                };

                // Mark all levels as needing verification
                if (coll.levels && coll.levels.length > 0) {
                    for (const level of coll.levels) {
                        coll.parameterChecks[param].levels[level] = {
                            inCatalog: null, // Unknown
                            queryResult: null,
                            message: 'Pending HTTP verification'
                        };
                    }
                }
            }
        }
    }

    /**
     * Phase 3: Compare advertised vs. catalog
     */
    compareAdvertisedVsCatalog() {
        for (const [collId, coll] of Object.entries(this.results.advertised)) {
            const modelName = coll.model;
            const modelCatalog = this.results.catalog.models?.[modelName];

            // Initialize collection checks
            coll.parameterChecks = {};

            if (!modelCatalog) {
                // No data for this model at all
                for (const param of coll.parameters) {
                    coll.parameterChecks[param] = {
                        inCatalog: false,
                        levels: {},
                        message: `Model "${modelName}" has no data in catalog`
                    };
                    
                    // Mark all levels as missing
                    if (coll.levels && coll.levels.length > 0) {
                        for (const level of coll.levels) {
                            coll.parameterChecks[param].levels[level] = {
                                inCatalog: false,
                                queryResult: null,
                                message: 'Model missing'
                            };
                        }
                    }
                }
                continue;
            }

            // Check each parameter
            for (const param of coll.parameters) {
                const paramCatalog = modelCatalog.parameters[param];
                
                coll.parameterChecks[param] = {
                    inCatalog: paramCatalog && paramCatalog.count > 0,
                    catalogLevels: paramCatalog?.levels || [],
                    catalogCount: paramCatalog?.count || 0,
                    levels: {},
                    message: paramCatalog ? 'Found in catalog' : 'Not found in catalog'
                };

                // Check levels if applicable
                if (coll.levels && coll.levels.length > 0) {
                    for (const level of coll.levels) {
                        const levelStr = this.formatLevelForComparison(level, collId);
                        const hasLevel = paramCatalog?.levels?.some(l => 
                            this.levelsMatch(l, levelStr)
                        ) || false;

                        coll.parameterChecks[param].levels[level] = {
                            inCatalog: hasLevel,
                            queryResult: null,
                            message: hasLevel ? 'Level found' : 'Level not found'
                        };
                    }
                }
            }
        }
    }

    /**
     * Format level value for comparison with catalog
     */
    formatLevelForComparison(level, collId) {
        // Isobaric levels should match "XXX mb" format
        if (collId.includes('isobaric')) {
            return `${level} mb`;
        }
        // Height above ground
        if (collId.includes('height') || collId.includes('agl')) {
            return `${level} m above ground`;
        }
        return String(level);
    }

    /**
     * Check if two level strings match
     */
    levelsMatch(catalogLevel, targetLevel) {
        // Normalize both strings
        const normalize = s => s.toLowerCase().replace(/\s+/g, ' ').trim();
        return normalize(catalogLevel).includes(normalize(targetLevel)) ||
               normalize(targetLevel).includes(normalize(catalogLevel));
    }

    /**
     * Phase 4: Verify data retrieval with HTTP requests
     * Now queries ALL parameters regardless of catalog status to confirm data exists
     */
    async verifyDataRetrieval() {
        const tasks = [];

        for (const [collId, coll] of Object.entries(this.results.advertised)) {
            for (const param of coll.parameters) {
                const paramCheck = coll.parameterChecks[param];
                
                // Initialize parameter-level query tracking
                paramCheck.queryResults = [];

                // Determine which levels to test
                const levels = Object.keys(paramCheck.levels).length > 0 
                    ? Object.keys(paramCheck.levels)
                    : [null]; // null means no level (surface collections)

                for (const level of levels) {
                    const levelCheck = level !== null ? paramCheck.levels[level] : null;

                    // Determine times to test
                    let timesToTest = [];
                    if (this.mode === 'thorough') {
                        timesToTest = coll.temporalValues.slice(0, 10); // Test up to 10 times
                    } else if (this.mode === 'full') {
                        timesToTest = coll.temporalValues.slice(0, 3); // Test latest 3
                    } else {
                        timesToTest = coll.temporalValues.slice(0, 1); // Just latest
                    }

                    // If no times, test without datetime param
                    if (timesToTest.length === 0) {
                        timesToTest = [null];
                    }

                    for (const time of timesToTest) {
                        tasks.push({
                            collId,
                            param,
                            level,
                            time,
                            levelCheck,
                            paramCheck,
                            coll // Pass collection for bbox access
                        });
                    }
                }
            }
        }

        this.totalChecks = tasks.length;
        this.completedChecks = 0;

        // Process tasks with concurrency limit
        await this.processTasksWithConcurrency(tasks, this.concurrency);
    }

    /**
     * Process tasks with limited concurrency
     */
    async processTasksWithConcurrency(tasks, limit) {
        const results = [];
        const executing = [];

        for (const task of tasks) {
            if (!this.running) break;

            const p = this.executeTask(task).then(result => {
                executing.splice(executing.indexOf(p), 1);
                return result;
            });

            results.push(p);
            executing.push(p);

            if (executing.length >= limit) {
                await Promise.race(executing);
            }
        }

        return Promise.all(results);
    }

    /**
     * Execute a single validation task
     */
    async executeTask(task) {
        const { collId, param, level, time, levelCheck, paramCheck, coll } = task;
        
        // Determine which formats to test
        const formats = this.outputFormat === 'both' 
            ? ['covjson', 'geojson'] 
            : [this.outputFormat];
        
        const results = [];
        
        for (const format of formats) {
            let result;
            if (this.queryType === 'area') {
                result = await this.testAreaQuery(collId, param, level, time, coll, format);
            } else if (this.queryType === 'radius') {
                result = await this.testRadiusQuery(collId, param, level, time, coll, format);
            } else if (this.queryType === 'trajectory') {
                result = await this.testTrajectoryQuery(collId, param, level, time, coll, format);
            } else if (this.queryType === 'corridor') {
                result = await this.testCorridorQuery(collId, param, level, time, coll, format);
            } else if (this.queryType === 'cube') {
                result = await this.testCubeQuery(collId, param, level, time, coll, format);
            } else if (this.queryType === 'locations') {
                result = await this.testLocationsQuery(collId, param, level, time, coll, format);
            } else {
                result = await this.testPositionQuery(collId, param, level, time, coll, format);
            }
            
            results.push(result);
            
            // Track in results
            this.results.checks.push(result);
            
            // Update summary
            this.results.summary[result.status]++;
            this.updateSummary();
            
            // Log request
            this.logRequest(result);
        }
        
        // Use the first result (or best result if testing both formats) for level/param tracking
        const primaryResult = results.find(r => r.status === 'pass') || results[0];

        // Update the level check result
        if (levelCheck) {
            levelCheck.queryResult = primaryResult.status;
            levelCheck.queryMessage = primaryResult.message;
            levelCheck.queryValues = primaryResult.values;
            levelCheck.queryDuration = primaryResult.duration;
            levelCheck.queryUrl = primaryResult.url;
            // Store all format results for this level
            levelCheck.formatResults = results;
        }

        // Track at parameter level for summary display
        if (paramCheck) {
            paramCheck.queryResults.push(...results);
            // Update parameter-level summary
            const passCount = paramCheck.queryResults.filter(r => r.status === 'pass').length;
            const totalCount = paramCheck.queryResults.length;
            paramCheck.queryPassCount = passCount;
            paramCheck.queryTotalCount = totalCount;
            paramCheck.queryStatus = passCount > 0 ? 'pass' : 
                                     paramCheck.queryResults.some(r => r.status === 'warn') ? 'warn' : 'fail';
        }

        // Update progress
        this.completedChecks++;
        const percent = Math.round((this.completedChecks / this.totalChecks) * 100);
        this.updateProgress(percent, `Testing ${collId}/${param} (${this.completedChecks}/${this.totalChecks})`);

        return primaryResult;
    }

    /**
     * Make a single position query and check result
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testPositionQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        const { lon, lat } = this.getQueryCoordinates(coll);
        const coords = `POINT(${lon} ${lat})`;
        let url = `${this.endpoint}/collections/${collectionId}/position?coords=${encodeURIComponent(coords)}&parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'position',
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                return this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
            } else {
                const values = data.ranges?.[parameter]?.values || [];
                const hasData = values.some(v => v !== null);

                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format: 'covjson',
                    queryType: 'position',
                    status: hasData ? 'pass' : 'warn',
                    message: hasData ? 'Data retrieved (CovJSON)' : 'Query OK but null values',
                    values: values.slice(0, 5),
                    duration,
                    url,
                    response: data
                };
            }
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'position',
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }
    
    /**
     * Validate GeoJSON response structure and extract data
     */
    validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url) {
        const isFeatureCollection = data.type === 'FeatureCollection';
        const isFeature = data.type === 'Feature';
        
        if (!isFeatureCollection && !isFeature) {
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'geojson',
                queryType: this.queryType,
                status: 'fail',
                message: `Invalid GeoJSON: type is "${data.type}", expected FeatureCollection or Feature`,
                duration,
                url,
                response: data
            };
        }
        
        // Extract features
        const features = isFeatureCollection ? (data.features || []) : [data];
        
        if (features.length === 0) {
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'geojson',
                queryType: this.queryType,
                status: 'warn',
                message: 'GeoJSON valid but no features',
                duration,
                url,
                response: data
            };
        }
        
        // Check for parameter values in properties
        let hasData = false;
        let sampleValues = [];
        
        for (const feature of features) {
            const props = feature.properties || {};
            // Check for parameter value (could be under 'parameters' or direct property)
            const paramData = props.parameters?.[parameter] || props[parameter];
            if (paramData !== undefined && paramData !== null) {
                hasData = true;
                if (typeof paramData === 'object' && paramData.values) {
                    sampleValues.push(...paramData.values.slice(0, 3));
                } else {
                    sampleValues.push(paramData);
                }
            }
        }
        
        // Check geometry validity
        const hasValidGeometry = features.every(f => 
            f.geometry && f.geometry.type && f.geometry.coordinates
        );
        
        return {
            collection: collectionId,
            parameter,
            level,
            time,
            format: 'geojson',
            queryType: this.queryType,
            status: hasData ? 'pass' : 'warn',
            message: hasData 
                ? `GeoJSON valid (${features.length} features)` 
                : `GeoJSON valid but no data for ${parameter}`,
            values: sampleValues.slice(0, 5),
            featureCount: features.length,
            hasValidGeometry,
            duration,
            url,
            response: data
        };
    }

    /**
     * Make a single area query and check result
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testAreaQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        // Create a 1x1 degree polygon centered on the test area (or random point)
        const { lon, lat } = this.getQueryCoordinates(coll);
        const halfDeg = 0.5;
        const minLon = lon - halfDeg;
        const maxLon = lon + halfDeg;
        const minLat = lat - halfDeg;
        const maxLat = lat + halfDeg;
        
        // WKT POLYGON format: POLYGON((lon1 lat1, lon2 lat2, lon3 lat3, lon4 lat4, lon1 lat1))
        const polygon = `POLYGON((${minLon} ${minLat},${maxLon} ${minLat},${maxLon} ${maxLat},${minLon} ${maxLat},${minLon} ${minLat}))`;
        
        let url = `${this.endpoint}/collections/${collectionId}/area?coords=${encodeURIComponent(polygon)}&parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'area',
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                return this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
            }
            
            const values = data.ranges?.[parameter]?.values || [];
            const nonNullValues = values.filter(v => v !== null);
            const hasData = nonNullValues.length > 0;

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'covjson',
                queryType: 'area',
                status: hasData ? 'pass' : 'warn',
                message: hasData 
                    ? `Area data retrieved (${nonNullValues.length}/${values.length} non-null)` 
                    : 'Query OK but all values null',
                values: nonNullValues.slice(0, 10), // Show first 10 non-null values
                valueCount: values.length,
                nonNullCount: nonNullValues.length,
                duration,
                url,
                response: data
            };
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'area',
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }

    /**
     * Make a single radius query and check result
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testRadiusQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        // Get coordinates (either from fixed config or random within collection bbox)
        const { lon, lat } = this.getQueryCoordinates(coll);
        const coords = `POINT(${lon} ${lat})`;
        const within = this.testRadius.within;
        const units = this.testRadius.units;
        
        let url = `${this.endpoint}/collections/${collectionId}/radius?coords=${encodeURIComponent(coords)}&within=${within}&within-units=${units}&parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'radius',
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                return this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
            }
            
            const values = data.ranges?.[parameter]?.values || [];
            const nonNullValues = values.filter(v => v !== null);
            const hasData = nonNullValues.length > 0;

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'covjson',
                queryType: 'radius',
                status: hasData ? 'pass' : 'warn',
                message: hasData 
                    ? `Radius data retrieved (${nonNullValues.length}/${values.length} non-null)` 
                    : 'Query OK but all values null',
                values: nonNullValues.slice(0, 10), // Show first 10 non-null values
                valueCount: values.length,
                nonNullCount: nonNullValues.length,
                duration,
                url,
                response: data
            };
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'radius',
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }

    /**
     * Make a single trajectory query and check result
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testTrajectoryQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        // Get trajectory coordinates - either fixed or generate random within bbox
        const coords = this.getTrajectoryCoordinates(coll);
        
        let url = `${this.endpoint}/collections/${collectionId}/trajectory?coords=${encodeURIComponent(coords)}&parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'trajectory',
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                return this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
            }
            
            const values = data.ranges?.[parameter]?.values || [];
            const nonNullValues = values.filter(v => v !== null);
            const hasData = nonNullValues.length > 0;

            // Verify domain type is Trajectory
            const domainType = data.domain?.domainType;
            const isTrajectory = domainType === 'Trajectory';

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'covjson',
                queryType: 'trajectory',
                status: hasData ? 'pass' : 'warn',
                message: hasData 
                    ? `Trajectory data retrieved (${nonNullValues.length}/${values.length} non-null, domainType: ${domainType})` 
                    : 'Query OK but all values null',
                values: nonNullValues.slice(0, 10), // Show first 10 non-null values
                valueCount: values.length,
                nonNullCount: nonNullValues.length,
                domainType: domainType,
                isValidDomainType: isTrajectory,
                duration,
                url,
                response: data
            };
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'trajectory',
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }

    /**
     * Make a single corridor query and check result
     * Corridor returns a CoverageCollection with multiple trajectories
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testCorridorQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        // Get corridor coordinates - either fixed or generate random within bbox
        const coords = this.getCorridorCoordinates(coll);
        const corridorWidth = this.testCorridor.corridorWidth;
        const widthUnits = this.testCorridor.widthUnits;
        const corridorHeight = this.testCorridor.corridorHeight;
        const heightUnits = this.testCorridor.heightUnits;
        
        let url = `${this.endpoint}/collections/${collectionId}/corridor?coords=${encodeURIComponent(coords)}&corridor-width=${corridorWidth}&width-units=${widthUnits}&corridor-height=${corridorHeight}&height-units=${heightUnits}&parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'corridor',
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                return this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
            }
            
            // Corridor returns CoverageCollection with multiple coverages (trajectories)
            const isCoverageCollection = data.type === 'CoverageCollection';
            const coverages = data.coverages || [];
            const numCoverages = coverages.length;
            
            // Collect values from all coverages
            let totalValues = 0;
            let totalNonNull = 0;
            for (const cov of coverages) {
                const values = cov.ranges?.[parameter]?.values || [];
                totalValues += values.length;
                totalNonNull += values.filter(v => v !== null).length;
            }
            
            const hasData = totalNonNull > 0;

            // Verify domain type is Trajectory at the collection level
            const domainType = data.domainType;
            const isTrajectory = domainType === 'Trajectory';

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'covjson',
                queryType: 'corridor',
                status: hasData ? 'pass' : 'warn',
                message: hasData 
                    ? `Corridor data retrieved (${numCoverages} trajectories, ${totalNonNull}/${totalValues} non-null, domainType: ${domainType})` 
                    : 'Query OK but all values null',
                values: coverages[0]?.ranges?.[parameter]?.values?.filter(v => v !== null).slice(0, 10) || [],
                valueCount: totalValues,
                nonNullCount: totalNonNull,
                numCoverages: numCoverages,
                domainType: domainType,
                isValidDomainType: isTrajectory,
                isCoverageCollection: isCoverageCollection,
                duration,
                url,
                response: data
            };
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'corridor',
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }

    /**
     * Make a single cube query and check result
     * Cube returns a CoverageCollection with one Coverage per z-level
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testCubeQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        // Get cube parameters - either fixed or generate random bbox within collection bounds
        const bbox = this.getCubeBbox(coll);
        const z = this.getCubeZ(coll, level);
        const resX = this.testCube.resolutionX;
        const resY = this.testCube.resolutionY;
        
        let url = `${this.endpoint}/collections/${collectionId}/cube?bbox=${bbox}&z=${z}&parameter-name=${parameter}`;
        
        // Add resolution if specified
        if (resX > 0) {
            url += `&resolution-x=${resX}`;
        }
        if (resY > 0) {
            url += `&resolution-y=${resY}`;
        }
        
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'cube',
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                return this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
            }
            
            // Cube returns CoverageCollection with one coverage per z-level
            const isCoverageCollection = data.type === 'CoverageCollection';
            const coverages = data.coverages || [];
            const numCoverages = coverages.length;
            
            // Collect values from all coverages
            let totalValues = 0;
            let totalNonNull = 0;
            for (const cov of coverages) {
                const values = cov.ranges?.[parameter]?.values || [];
                totalValues += values.length;
                totalNonNull += values.filter(v => v !== null).length;
            }
            
            const hasData = totalNonNull > 0;

            // Verify domain type is Grid at the collection level
            const domainType = data.domainType;
            const isGrid = domainType === 'Grid';

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'covjson',
                queryType: 'cube',
                status: hasData ? 'pass' : 'warn',
                message: hasData 
                    ? `Cube data retrieved (${numCoverages} z-levels, ${totalNonNull}/${totalValues} non-null, domainType: ${domainType})` 
                    : 'Query OK but all values null',
                values: coverages[0]?.ranges?.[parameter]?.values?.filter(v => v !== null).slice(0, 10) || [],
                valueCount: totalValues,
                nonNullCount: totalNonNull,
                numCoverages: numCoverages,
                domainType: domainType,
                isValidDomainType: isGrid,
                isCoverageCollection: isCoverageCollection,
                duration,
                url,
                response: data
            };
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'cube',
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }

    /**
     * Make a single locations query and check result
     * Uses a named location ID instead of coordinates
     * @param {string} format - 'covjson' or 'geojson'
     */
    async testLocationsQuery(collectionId, parameter, level, time, coll, format = 'covjson') {
        // Get location ID - either fixed or pick random from available locations
        const locationId = this.getLocationId(collectionId);
        
        let url = `${this.endpoint}/collections/${collectionId}/locations/${locationId}?parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
        }
        
        // Add format parameter
        if (format === 'geojson') {
            url += '&f=geojson';
        }

        const startTime = performance.now();

        try {
            const response = await fetch(url, { 
                signal: this.abortController?.signal 
            });
            const duration = Math.round(performance.now() - startTime);

            if (!response.ok) {
                return {
                    collection: collectionId,
                    parameter,
                    level,
                    time,
                    format,
                    queryType: 'locations',
                    locationId,
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            
            // Validate response based on format
            if (format === 'geojson') {
                const result = this.validateGeoJsonResponse(data, collectionId, parameter, level, time, duration, url);
                result.locationId = locationId;
                return result;
            }
            
            const values = data.ranges?.[parameter]?.values || [];
            const nonNullValues = values.filter(v => v !== null);
            const hasData = nonNullValues.length > 0;

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format: 'covjson',
                queryType: 'locations',
                locationId,
                status: hasData ? 'pass' : 'warn',
                message: hasData 
                    ? `Location "${locationId}" data retrieved (${nonNullValues.length}/${values.length} non-null)` 
                    : `Query OK but all values null for location "${locationId}"`,
                values: nonNullValues.slice(0, 10), // Show first 10 non-null values
                valueCount: values.length,
                nonNullCount: nonNullValues.length,
                duration,
                url,
                response: data
            };
        } catch (error) {
            if (error.name === 'AbortError') {
                throw error;
            }
            return {
                collection: collectionId,
                parameter,
                level,
                time,
                format,
                queryType: 'locations',
                locationId,
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
    }

    /**
     * Get the location ID to use for a query
     * If randomize is enabled, picks a random location from available ones
     * @param {string} collectionId - Collection ID (used for per-collection location caching in future)
     * @returns {string} - Location ID
     */
    getLocationId(collectionId) {
        if (this.randomizeLocations && this.testLocations.availableLocations.length > 0) {
            const randomIndex = Math.floor(Math.random() * this.testLocations.availableLocations.length);
            return this.testLocations.availableLocations[randomIndex];
        }
        return this.testLocations.locationId;
    }

    /**
     * Load available locations from the first collection
     * Populates the location select dropdown
     */
    async loadAvailableLocations() {
        const select = document.getElementById('location-select');
        select.innerHTML = '<option value="">Loading...</option>';
        
        try {
            // Get the first collection to query its locations
            const collectionsResp = await fetch(`${this.endpoint}/collections`);
            if (!collectionsResp.ok) {
                throw new Error(`HTTP ${collectionsResp.status}`);
            }
            const collectionsData = await collectionsResp.json();
            const collections = collectionsData.collections || [];
            
            if (collections.length === 0) {
                select.innerHTML = '<option value="">No collections</option>';
                return;
            }
            
            // Try to get locations from the first collection
            const firstCollId = collections[0].id;
            const locationsResp = await fetch(`${this.endpoint}/collections/${firstCollId}/locations`);
            
            if (!locationsResp.ok) {
                throw new Error(`HTTP ${locationsResp.status}`);
            }
            
            const locationsData = await locationsResp.json();
            const features = locationsData.features || [];
            
            // Extract location IDs from features
            // Note: Per OGC EDR spec, feature IDs are URI-style (e.g., "http://.../locations/HOU")
            // We need to extract just the location code (last path segment) for use in queries
            const locationIds = features.map(f => {
                const rawId = f.id || f.properties?.id;
                if (!rawId) return null;
                // If it's a URI, extract the last path segment
                if (rawId.includes('/locations/')) {
                    return rawId.split('/locations/').pop();
                }
                return rawId;
            }).filter(Boolean);
            this.testLocations.availableLocations = locationIds;
            
            // Populate select
            select.innerHTML = '<option value="">-- or select --</option>';
            locationIds.forEach(id => {
                const option = document.createElement('option');
                option.value = id;
                // Try to get a nice label from feature properties
                // Match by checking if the feature ID ends with our extracted ID
                const feature = features.find(f => {
                    const fid = f.id || f.properties?.id;
                    return fid === id || (fid && fid.endsWith(`/locations/${id}`));
                });
                const name = feature?.properties?.name || id;
                option.textContent = name !== id ? `${id} - ${name}` : id;
                select.appendChild(option);
            });
            
            if (locationIds.length === 0) {
                select.innerHTML = '<option value="">No locations found</option>';
            }
        } catch (e) {
            console.error('Failed to load locations:', e);
            select.innerHTML = '<option value="">Error loading</option>';
        }
    }

    /**
     * Get cube bbox - either fixed or generate random within collection bounds
     * @param {Object} coll - Collection object with bbox
     * @returns {string} - bbox string "minLon,minLat,maxLon,maxLat"
     */
    getCubeBbox(coll) {
        if (!this.randomizeLocations) {
            return this.testCube.bbox;
        }
        
        // Generate a random 1x1 degree bbox within the collection's bounds
        if (!coll.bbox || coll.bbox.length < 4) {
            return this.testCube.bbox; // Fallback to default
        }
        
        const [minLon, minLat, maxLon, maxLat] = coll.bbox;
        
        // Add padding (10% inward from edges) and ensure 1 degree for bbox size
        const lonPadding = (maxLon - minLon) * 0.1;
        const latPadding = (maxLat - minLat) * 0.1;
        const bboxSize = 1.0; // 1 degree
        
        // Calculate valid range for bbox corner
        const validMinLon = minLon + lonPadding;
        const validMaxLon = maxLon - lonPadding - bboxSize;
        const validMinLat = minLat + latPadding;
        const validMaxLat = maxLat - latPadding - bboxSize;
        
        if (validMaxLon <= validMinLon || validMaxLat <= validMinLat) {
            return this.testCube.bbox; // Fallback if collection too small
        }
        
        const cornerLon = validMinLon + Math.random() * (validMaxLon - validMinLon);
        const cornerLat = validMinLat + Math.random() * (validMaxLat - validMinLat);
        
        const bboxMinLon = Math.round(cornerLon * 100) / 100;
        const bboxMinLat = Math.round(cornerLat * 100) / 100;
        const bboxMaxLon = Math.round((cornerLon + bboxSize) * 100) / 100;
        const bboxMaxLat = Math.round((cornerLat + bboxSize) * 100) / 100;
        
        return `${bboxMinLon},${bboxMinLat},${bboxMaxLon},${bboxMaxLat}`;
    }

    /**
     * Get cube z parameter - use level from task if provided, otherwise use config
     * @param {Object} coll - Collection object
     * @param {string|number|null} level - Level from task
     * @returns {string} - z parameter value
     */
    getCubeZ(coll, level) {
        // If a specific level is being tested, use it
        if (level !== null && level !== undefined) {
            return String(level);
        }
        // Otherwise use the configured z value
        return this.testCube.z;
    }

    /**
     * Get corridor coordinates - either fixed or generate random path within bbox
     * @param {Object} coll - Collection object with bbox
     * @returns {string} - WKT LINESTRING
     */
    getCorridorCoordinates(coll) {
        if (!this.randomizeLocations) {
            return this.testCorridor.coords;
        }
        
        // Generate a random corridor path within the collection's bbox
        if (!coll.bbox || coll.bbox.length < 4) {
            return this.testCorridor.coords; // Fallback to default
        }
        
        const [minLon, minLat, maxLon, maxLat] = coll.bbox;
        
        // Add padding (10% inward from edges)
        const lonPadding = (maxLon - minLon) * 0.1;
        const latPadding = (maxLat - minLat) * 0.1;
        
        // Generate 3 waypoints for the corridor centerline
        const waypoints = [];
        for (let i = 0; i < 3; i++) {
            const lon = minLon + lonPadding + Math.random() * (maxLon - minLon - 2 * lonPadding);
            const lat = minLat + latPadding + Math.random() * (maxLat - minLat - 2 * latPadding);
            waypoints.push(`${Math.round(lon * 10000) / 10000} ${Math.round(lat * 10000) / 10000}`);
        }
        
        return `LINESTRING(${waypoints.join(',')})`;
    }

    /**
     * Get trajectory coordinates - either fixed or generate random path within bbox
     * @param {Object} coll - Collection object with bbox
     * @returns {string} - WKT LINESTRING
     */
    getTrajectoryCoordinates(coll) {
        if (!this.randomizeLocations) {
            return this.testTrajectory.coords;
        }
        
        // Generate a random trajectory within the collection's bbox
        if (!coll.bbox || coll.bbox.length < 4) {
            return this.testTrajectory.coords; // Fallback to default
        }
        
        const [minLon, minLat, maxLon, maxLat] = coll.bbox;
        
        // Add padding (10% inward from edges)
        const lonPadding = (maxLon - minLon) * 0.1;
        const latPadding = (maxLat - minLat) * 0.1;
        
        // Generate 3 waypoints for the trajectory
        const waypoints = [];
        for (let i = 0; i < 3; i++) {
            const lon = minLon + lonPadding + Math.random() * (maxLon - minLon - 2 * lonPadding);
            const lat = minLat + latPadding + Math.random() * (maxLat - minLat - 2 * latPadding);
            waypoints.push(`${Math.round(lon * 10000) / 10000} ${Math.round(lat * 10000) / 10000}`);
        }
        
        return `LINESTRING(${waypoints.join(',')})`;
    }

    /**
     * Fetch JSON from URL
     */
    async fetchJson(url) {
        const response = await fetch(url, { 
            signal: this.abortController?.signal 
        });
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }
        return response.json();
    }

    /**
     * Log a request
     */
    logRequest(result) {
        this.requestLog.push({
            timestamp: new Date().toISOString(),
            ...result
        });

        // Update log UI
        const logContent = document.getElementById('log-content');
        const logCount = document.getElementById('log-count');

        // Remove empty message if present
        const emptyMsg = logContent.querySelector('.log-empty');
        if (emptyMsg) emptyMsg.remove();

        // Add log entry
        const entry = document.createElement('div');
        entry.className = 'log-entry';
        const formatBadge = result.format === 'geojson' ? '<span class="format-badge geojson">GeoJSON</span>' : '';
        entry.innerHTML = `
            <span class="time">${new Date().toLocaleTimeString()}</span>
            <span class="status ${result.status}">${result.status.toUpperCase()}</span>
            ${formatBadge}
            <span class="url" title="${result.url}">${result.url}</span>
            <span class="duration">${result.duration}ms</span>
        `;
        logContent.appendChild(entry);

        // Auto-scroll to bottom
        logContent.scrollTop = logContent.scrollHeight;

        // Update count
        logCount.textContent = `${this.requestLog.length} requests`;
    }

    /**
     * Render the collections tree
     */
    renderTree() {
        const container = document.getElementById('collections-tree');
        container.innerHTML = '';

        for (const [collId, coll] of Object.entries(this.results.advertised)) {
            const collEl = this.createCollectionElement(collId, coll);
            container.appendChild(collEl);
        }
    }

    /**
     * Create a collection element for the tree
     */
    createCollectionElement(collId, coll) {
        const el = document.createElement('div');
        el.className = 'tree-collection';
        el.dataset.collId = collId;

        // Calculate coverage
        const coverage = this.calculateCollectionCoverage(coll);
        const coverageClass = coverage >= 80 ? 'high' : coverage >= 50 ? 'medium' : 'low';

        el.innerHTML = `
            <div class="collection-header">
                <span class="toggle">▶</span>
                <span class="name">${coll.title}</span>
                <span class="coverage ${coverageClass}">${coverage}%</span>
            </div>
            <div class="collection-content">
                <div class="collection-stats">
                    <span>Params: ${coll.parameters.length}</span>
                    <span>Levels: ${coll.levels?.length || 0}</span>
                    <span>Times: ${coll.temporalValues?.length || 0}</span>
                </div>
            </div>
        `;

        // Add parameters
        const content = el.querySelector('.collection-content');
        for (const param of coll.parameters) {
            const paramEl = this.createParameterElement(collId, param, coll);
            content.appendChild(paramEl);
        }

        // Toggle expand/collapse
        el.querySelector('.collection-header').addEventListener('click', () => {
            el.classList.toggle('expanded');
        });

        return el;
    }

    /**
     * Create a parameter element for the tree
     */
    createParameterElement(collId, param, coll) {
        const paramCheck = coll.parameterChecks[param];
        const el = document.createElement('div');
        el.className = 'tree-parameter';
        el.dataset.param = param;

        // Calculate parameter coverage
        const levels = Object.keys(paramCheck.levels || {});
        let passCount = 0;
        let totalCount = levels.length || 1;

        if (levels.length > 0) {
            for (const level of levels) {
                if (paramCheck.levels[level].inCatalog) passCount++;
            }
        } else {
            if (paramCheck.inCatalog) passCount = 1;
        }

        const paramCoverage = Math.round((passCount / totalCount) * 100);
        const inCatalogClass = paramCheck.inCatalog ? 'pass' : (paramCheck.inCatalog === null ? 'skip' : 'fail');
        const inCatalogLabel = paramCheck.inCatalog ? 'In Catalog' : (paramCheck.inCatalog === null ? 'Unknown' : 'Missing');

        // Query status at parameter level
        let queryBadge = '';
        if (paramCheck.queryResults && paramCheck.queryResults.length > 0) {
            const queryClass = paramCheck.queryStatus || 'skip';
            const queryPassCount = paramCheck.queryPassCount || 0;
            const queryTotal = paramCheck.queryTotalCount || 0;
            queryBadge = `<span class="status-badge ${queryClass}">Query: ${queryPassCount}/${queryTotal} OK</span>`;
        }

        el.innerHTML = `
            <div class="parameter-header">
                <span class="toggle">▶</span>
                <span class="name">${param}</span>
                <span class="status-badge ${inCatalogClass}">${inCatalogLabel}</span>
                ${queryBadge}
                <span class="stats">${passCount}/${totalCount} levels</span>
            </div>
            <div class="parameter-content"></div>
        `;

        // Add levels if any
        const content = el.querySelector('.parameter-content');
        if (levels.length > 0) {
            for (const level of levels) {
                const levelEl = this.createLevelElement(collId, param, level, paramCheck.levels[level]);
                content.appendChild(levelEl);
            }
        } else {
            // No levels - show single status for surface parameters
            const statusEl = document.createElement('div');
            statusEl.className = 'tree-level';
            
            // DB status
            let dbStatus = 'skip';
            let dbLabel = 'unknown';
            if (paramCheck.inCatalog === true) {
                dbStatus = 'pass';
                dbLabel = 'exists';
            } else if (paramCheck.inCatalog === false) {
                dbStatus = 'fail';
                dbLabel = 'MISSING';
            }
            
            // Query status for surface params
            let queryStatus = 'skip';
            let queryLabel = 'pending';
            let queryUrl = null;
            if (paramCheck.queryResults && paramCheck.queryResults.length > 0) {
                const firstResult = paramCheck.queryResults[0];
                queryStatus = firstResult.status;
                queryLabel = firstResult.status === 'pass' ? 'OK' : 
                            firstResult.status === 'warn' ? 'null' : 'ERROR';
                queryUrl = firstResult.url;
            }
            
            // Copy URL button
            const copyBtn = queryUrl 
                ? `<button class="copy-url-btn" title="Copy query URL to clipboard">Copy URL</button>`
                : '';
            
            statusEl.innerHTML = `
                <span class="level-name">surface</span>
                <span class="level-status">
                    <span class="status-badge ${dbStatus}">DB: ${dbLabel}</span>
                    <span class="status-badge ${queryStatus}">Query: ${queryLabel}</span>
                    ${copyBtn}
                </span>
            `;
            
            // Bind copy button
            const copyBtnEl = statusEl.querySelector('.copy-url-btn');
            if (copyBtnEl && queryUrl) {
                copyBtnEl.addEventListener('click', (e) => {
                    e.stopPropagation();
                    this.copyToClipboard(queryUrl, copyBtnEl);
                });
            }
            
            statusEl.addEventListener('click', () => {
                this.showDetails(collId, param, null, paramCheck);
            });
            content.appendChild(statusEl);
        }

        // Toggle expand/collapse
        el.querySelector('.parameter-header').addEventListener('click', (e) => {
            e.stopPropagation();
            el.classList.toggle('expanded');
        });

        return el;
    }

    /**
     * Create a level element for the tree
     */
    createLevelElement(collId, param, level, levelCheck) {
        const el = document.createElement('div');
        el.className = 'tree-level';
        el.dataset.level = level;

        // DB status - may be unknown (null) if catalog check not available
        let dbStatus = 'skip';
        let dbLabel = 'unknown';
        if (levelCheck.inCatalog === true) {
            dbStatus = 'pass';
            dbLabel = 'exists';
        } else if (levelCheck.inCatalog === false) {
            dbStatus = 'fail';
            dbLabel = 'MISSING';
        }

        // Query status
        let queryStatus = 'skip';
        let queryLabel = 'pending';
        if (levelCheck.queryResult) {
            queryStatus = levelCheck.queryResult;
            if (levelCheck.queryResult === 'pass') {
                queryLabel = 'OK';
            } else if (levelCheck.queryResult === 'warn') {
                queryLabel = 'null';
            } else if (levelCheck.queryResult === 'fail') {
                queryLabel = 'ERROR';
            } else if (levelCheck.queryResult === 'skipped') {
                queryLabel = 'skipped';
            }
        }

        // Copy URL button (only show if we have a URL)
        const copyBtn = levelCheck.queryUrl 
            ? `<button class="copy-url-btn" title="Copy query URL to clipboard">Copy URL</button>`
            : '';

        el.innerHTML = `
            <span class="level-name">${level} ${collId.includes('isobaric') ? 'hPa' : ''}</span>
            <span class="level-status">
                <span class="status-badge ${dbStatus}">DB: ${dbLabel}</span>
                <span class="status-badge ${queryStatus}">Query: ${queryLabel}</span>
                ${copyBtn}
            </span>
        `;

        // Bind copy button
        const copyBtnEl = el.querySelector('.copy-url-btn');
        if (copyBtnEl) {
            copyBtnEl.addEventListener('click', (e) => {
                e.stopPropagation();
                this.copyToClipboard(levelCheck.queryUrl, copyBtnEl);
            });
        }

        el.addEventListener('click', () => {
            this.showDetails(collId, param, level, levelCheck);
        });

        return el;
    }

    /**
     * Copy text to clipboard and show feedback
     */
    copyToClipboard(text, buttonEl) {
        navigator.clipboard.writeText(text).then(() => {
            const originalText = buttonEl.textContent;
            buttonEl.textContent = 'Copied!';
            buttonEl.classList.add('copied');
            setTimeout(() => {
                buttonEl.textContent = originalText;
                buttonEl.classList.remove('copied');
            }, 1500);
        }).catch(err => {
            console.error('Failed to copy:', err);
            buttonEl.textContent = 'Failed';
            setTimeout(() => {
                buttonEl.textContent = 'Copy URL';
            }, 1500);
        });
    }

    /**
     * Get random coordinates within a bounding box
     * @param {Array} bbox - [minLon, minLat, maxLon, maxLat]
     * @returns {Object} - { lon, lat }
     */
    getRandomPointInBbox(bbox) {
        if (!bbox || bbox.length < 4) {
            // Fallback to default test point if no bbox
            return { lon: this.testPoint.lon, lat: this.testPoint.lat };
        }
        
        const [minLon, minLat, maxLon, maxLat] = bbox;
        
        // Add some padding to avoid edge cases (5% inward from edges)
        const lonPadding = (maxLon - minLon) * 0.05;
        const latPadding = (maxLat - minLat) * 0.05;
        
        const lon = minLon + lonPadding + Math.random() * (maxLon - minLon - 2 * lonPadding);
        const lat = minLat + latPadding + Math.random() * (maxLat - minLat - 2 * latPadding);
        
        // Round to 4 decimal places for cleaner URLs
        return {
            lon: Math.round(lon * 10000) / 10000,
            lat: Math.round(lat * 10000) / 10000
        };
    }

    /**
     * Get coordinates for a query - either fixed or randomized
     * @param {Object} coll - Collection object with bbox
     * @returns {Object} - { lon, lat }
     */
    getQueryCoordinates(coll) {
        if (this.randomizeLocations && coll.bbox) {
            return this.getRandomPointInBbox(coll.bbox);
        }
        
        // Use fixed test point/area/radius center
        if (this.queryType === 'area') {
            return { lon: this.testArea.lon, lat: this.testArea.lat };
        } else if (this.queryType === 'radius') {
            return { lon: this.testRadius.lon, lat: this.testRadius.lat };
        }
        return { lon: this.testPoint.lon, lat: this.testPoint.lat };
    }

    /**
     * Calculate collection coverage percentage based on query results
     * Now uses query results as the primary indicator, falling back to catalog status
     */
    calculateCollectionCoverage(coll) {
        let total = 0;
        let available = 0;

        for (const param of coll.parameters) {
            const paramCheck = coll.parameterChecks[param];
            const levels = Object.keys(paramCheck.levels || {});

            if (levels.length > 0) {
                for (const level of levels) {
                    total++;
                    const levelCheck = paramCheck.levels[level];
                    // Prefer query result over catalog status
                    if (levelCheck.queryResult === 'pass') {
                        available++;
                    } else if (levelCheck.queryResult === null && levelCheck.inCatalog) {
                        // Query not run yet, fall back to catalog
                        available++;
                    }
                }
            } else {
                total++;
                // For surface params, check query results
                if (paramCheck.queryStatus === 'pass') {
                    available++;
                } else if (!paramCheck.queryResults?.length && paramCheck.inCatalog) {
                    // Query not run yet, fall back to catalog
                    available++;
                }
            }
        }

        return total > 0 ? Math.round((available / total) * 100) : 0;
    }

    /**
     * Show details for selected item
     */
    showDetails(collId, param, level, check) {
        this.selectedItem = { collId, param, level, check };
        const coll = this.results.advertised[collId];
        const container = document.getElementById('details-content');

        // For surface params (level === null), check is paramCheck which has queryResults array
        // For level params, check is levelCheck which has queryUrl directly
        const isSurfaceParam = level === null && check.queryResults;
        
        // Get query info - either from levelCheck or first result in paramCheck.queryResults
        let queryUrl = check.queryUrl;
        let queryResult = check.queryResult;
        let queryDuration = check.queryDuration;
        let queryValues = check.queryValues;
        let queryMessage = check.queryMessage;
        
        if (isSurfaceParam && check.queryResults.length > 0) {
            const firstResult = check.queryResults[0];
            queryUrl = firstResult.url;
            queryResult = firstResult.status;
            queryDuration = firstResult.duration;
            queryValues = firstResult.values;
            queryMessage = firstResult.message;
        }

        let html = `
            <div class="detail-section">
                <h3>Item</h3>
                <div class="detail-item">
                    <span class="label">Collection</span>
                    <span class="value">${coll.title}</span>
                </div>
                <div class="detail-item">
                    <span class="label">Parameter</span>
                    <span class="value">${param}</span>
                </div>
                ${level !== null ? `
                <div class="detail-item">
                    <span class="label">Level</span>
                    <span class="value">${level}</span>
                </div>
                ` : ''}
            </div>

            <div class="detail-section">
                <h3>Catalog Status</h3>
                <div class="detail-item">
                    <span class="label">In Database</span>
                    <span class="value">${check.inCatalog ? 'Yes' : (check.inCatalog === false ? 'No' : 'Unknown')}</span>
                </div>
                <div class="detail-item">
                    <span class="label">Message</span>
                    <span class="value">${check.message || queryMessage || '-'}</span>
                </div>
            </div>
        `;

        if (queryUrl) {
            html += `
                <div class="detail-section">
                    <h3>HTTP Request</h3>
                    <div class="detail-item">
                        <span class="label">Status</span>
                        <span class="value status-badge ${queryResult}">${queryResult?.toUpperCase()}</span>
                    </div>
                    <div class="detail-item">
                        <span class="label">Duration</span>
                        <span class="value">${queryDuration}ms</span>
                    </div>
                    <div class="detail-request">${queryUrl}</div>
                </div>
            `;

            if (queryValues) {
                html += `
                    <div class="detail-section">
                        <h3>Response Values</h3>
                        <div class="detail-response">${JSON.stringify(queryValues, null, 2)}</div>
                    </div>
                `;
            }
        }

        // For surface params with multiple query results, show all of them
        if (isSurfaceParam && check.queryResults && check.queryResults.length > 1) {
            html += `
                <div class="detail-section">
                    <h3>All Query Results (${check.queryResults.length})</h3>
            `;
            for (let i = 0; i < check.queryResults.length; i++) {
                const result = check.queryResults[i];
                html += `
                    <div class="detail-item">
                        <span class="label">#${i + 1} ${result.time || 'no datetime'}</span>
                        <span class="value">
                            <span class="status-badge ${result.status}">${result.status.toUpperCase()}</span>
                            ${result.duration}ms
                        </span>
                    </div>
                `;
            }
            html += `</div>`;
        }

        container.innerHTML = html;
    }

    /**
     * Render gap summary
     */
    renderGapSummary() {
        const gaps = [];

        for (const [collId, coll] of Object.entries(this.results.advertised)) {
            for (const param of coll.parameters) {
                const paramCheck = coll.parameterChecks[param];

                if (!paramCheck.inCatalog) {
                    gaps.push({
                        collection: collId,
                        parameter: param,
                        type: 'parameter_missing',
                        message: `Parameter "${param}" has no data in catalog`
                    });
                    continue;
                }

                // Check levels
                for (const [level, levelCheck] of Object.entries(paramCheck.levels || {})) {
                    if (!levelCheck.inCatalog) {
                        gaps.push({
                            collection: collId,
                            parameter: param,
                            level: level,
                            type: 'level_missing',
                            message: `Level ${level} not available for ${param}`
                        });
                    }
                }
            }
        }

        if (gaps.length === 0) {
            document.getElementById('gap-summary-panel').style.display = 'none';
            return;
        }

        document.getElementById('gap-summary-panel').style.display = 'block';
        const content = document.getElementById('gap-content');
        content.innerHTML = gaps.map(gap => `
            <div class="gap-item">
                <div class="gap-type">${gap.type.replace('_', ' ').toUpperCase()}</div>
                <div class="gap-details">
                    <strong>${gap.collection}</strong>: ${gap.message}
                </div>
            </div>
        `).join('');
    }

    /**
     * Set the filter for the tree view
     */
    setFilter(filter) {
        // Apply filter
        document.querySelectorAll('.tree-collection').forEach(collEl => {
            const collId = collEl.dataset.collId;
            const coll = this.results.advertised[collId];

            if (!coll) return;

            let showCollection = filter === 'all';
            let shouldExpand = false;

            // Process each parameter
            collEl.querySelectorAll('.tree-parameter').forEach(paramEl => {
                const paramName = paramEl.dataset.param;
                const paramCheck = coll.parameterChecks[paramName];
                
                let showParam = filter === 'all';
                let paramHasMatch = false;
                
                // Check parameter-level status
                if (filter === 'fail') {
                    if (paramCheck.inCatalog === false || paramCheck.queryStatus === 'fail') {
                        showParam = true;
                        paramHasMatch = true;
                    }
                    if (paramCheck.queryResults?.some(r => r.status === 'fail')) {
                        showParam = true;
                        paramHasMatch = true;
                    }
                }
                if (filter === 'warn') {
                    if (paramCheck.queryStatus === 'warn') {
                        showParam = true;
                        paramHasMatch = true;
                    }
                    if (paramCheck.queryResults?.some(r => r.status === 'warn')) {
                        showParam = true;
                        paramHasMatch = true;
                    }
                }
                if (filter === 'pass' && paramCheck.queryStatus === 'pass') {
                    showParam = true;
                }

                // Process each level within the parameter
                const levels = Object.keys(paramCheck.levels || {});
                paramEl.querySelectorAll('.tree-level').forEach(levelEl => {
                    const levelName = levelEl.dataset.level;
                    const levelCheck = levelName ? paramCheck.levels[levelName] : null;
                    
                    let showLevel = filter === 'all';
                    
                    if (levelCheck) {
                        if (filter === 'fail') {
                            if (levelCheck.inCatalog === false || levelCheck.queryResult === 'fail') {
                                showLevel = true;
                                showParam = true;
                                paramHasMatch = true;
                            }
                        }
                        if (filter === 'warn' && levelCheck.queryResult === 'warn') {
                            showLevel = true;
                            showParam = true;
                            paramHasMatch = true;
                        }
                        if (filter === 'pass' && (levelCheck.queryResult === 'pass' || levelCheck.inCatalog === true)) {
                            showLevel = true;
                            showParam = true;
                        }
                    } else {
                        // Surface parameter (no level) - use param-level checks
                        if (filter === 'fail') {
                            if (paramCheck.inCatalog === false || paramCheck.queryStatus === 'fail') {
                                showLevel = true;
                            }
                            if (paramCheck.queryResults?.some(r => r.status === 'fail')) {
                                showLevel = true;
                            }
                        }
                        if (filter === 'warn') {
                            if (paramCheck.queryStatus === 'warn') {
                                showLevel = true;
                            }
                            if (paramCheck.queryResults?.some(r => r.status === 'warn')) {
                                showLevel = true;
                            }
                        }
                        if (filter === 'pass' && paramCheck.queryStatus === 'pass') {
                            showLevel = true;
                        }
                    }
                    
                    levelEl.style.display = showLevel ? '' : 'none';
                });

                // Show/hide parameter and expand if it has matches
                paramEl.style.display = showParam ? '' : 'none';
                if (showParam) {
                    showCollection = true;
                    if (paramHasMatch && (filter === 'fail' || filter === 'warn')) {
                        paramEl.classList.add('expanded');
                        shouldExpand = true;
                    }
                }
                
                // Collapse when showing all
                if (filter === 'all') {
                    paramEl.classList.remove('expanded');
                }
            });

            collEl.style.display = showCollection ? '' : 'none';
            
            // Auto-expand collections that have matching items for fail/warn filters
            if (showCollection && shouldExpand && (filter === 'fail' || filter === 'warn')) {
                collEl.classList.add('expanded');
            } else if (filter === 'all') {
                // When showing all, collapse everything
                collEl.classList.remove('expanded');
            }
        });
    }

    /**
     * Update progress bar and text
     */
    updateProgress(percent, text) {
        document.getElementById('progress-fill').style.width = percent + '%';
        document.getElementById('progress-text').textContent = text;
    }

    /**
     * Update status text
     */
    updateStatus(text) {
        document.getElementById('progress-text').textContent = text;
    }

    /**
     * Update summary counts
     */
    updateSummary() {
        document.getElementById('pass-count').textContent = this.results.summary.pass;
        document.getElementById('fail-count').textContent = this.results.summary.fail;
        document.getElementById('warn-count').textContent = this.results.summary.warn;
        document.getElementById('skip-count').textContent = this.results.summary.skip;
    }
}

/**
 * Toggle log panel
 */
function toggleLog() {
    document.getElementById('log-panel').classList.toggle('expanded');
}

/**
 * Copy gap summary to clipboard
 */
function copyGapSummary() {
    const content = document.getElementById('gap-content').innerText;
    navigator.clipboard.writeText(content).then(() => {
        alert('Gap summary copied to clipboard');
    });
}

// Initialize on page load
document.addEventListener('DOMContentLoaded', () => {
    window.validator = new EDRCoverageValidator();
    window.validator.init();
});
