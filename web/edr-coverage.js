/**
 * EDR Coverage Validation Tool
 * 
 * Systematically validates that every piece of data advertised by the EDR API
 * can actually be retrieved.
 */

class EDRCoverageValidator {
    constructor() {
        this.endpoint = 'http://localhost:8083/edr';
        this.mode = 'full';
        this.testPoint = { lon: -100, lat: 40 };
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
    }

    /**
     * Initialize the validator and bind event listeners
     */
    init() {
        // Load saved endpoint
        const savedEndpoint = localStorage.getItem('edr-coverage-endpoint');
        if (savedEndpoint) {
            document.getElementById('endpoint-input').value = savedEndpoint;
            this.endpoint = savedEndpoint;
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

        // Bind filter buttons
        document.querySelectorAll('.filter-btn').forEach(btn => {
            btn.addEventListener('click', (e) => this.setFilter(e.target.dataset.filter));
        });

        // Bind test point inputs
        document.getElementById('test-lon').addEventListener('change', (e) => {
            this.testPoint.lon = parseFloat(e.target.value) || -100;
        });
        document.getElementById('test-lat').addEventListener('change', (e) => {
            this.testPoint.lat = parseFloat(e.target.value) || 40;
        });
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
            testPoint: this.testPoint,
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
        
        for (const coll of collectionsResp.collections || []) {
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
     */
    async verifyDataRetrieval() {
        const tasks = [];

        for (const [collId, coll] of Object.entries(this.results.advertised)) {
            for (const param of coll.parameters) {
                const paramCheck = coll.parameterChecks[param];
                
                // When we have catalog data: skip if parameter not found (unless thorough mode)
                // When we don't have catalog data (inCatalog === null): always test
                if (paramCheck.inCatalog === false && this.mode !== 'thorough') {
                    continue;
                }

                // Determine which levels to test
                const levels = Object.keys(paramCheck.levels).length > 0 
                    ? Object.keys(paramCheck.levels)
                    : [null]; // null means no level (surface collections)

                for (const level of levels) {
                    const levelCheck = level !== null ? paramCheck.levels[level] : null;
                    
                    // When we have catalog data: skip if level not found (unless thorough mode)
                    // When we don't have catalog data (inCatalog === null): always test
                    if (levelCheck && levelCheck.inCatalog === false && this.mode !== 'thorough') {
                        levelCheck.queryResult = 'skipped';
                        continue;
                    }

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
                            levelCheck
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
        const { collId, param, level, time, levelCheck } = task;
        const result = await this.testPositionQuery(collId, param, level, time);

        // Update the check result
        if (levelCheck) {
            levelCheck.queryResult = result.status;
            levelCheck.queryMessage = result.message;
            levelCheck.queryValues = result.values;
            levelCheck.queryDuration = result.duration;
            levelCheck.queryUrl = result.url;
        }

        // Track in results
        this.results.checks.push(result);

        // Update summary
        this.results.summary[result.status]++;
        this.updateSummary();

        // Update progress
        this.completedChecks++;
        const percent = Math.round((this.completedChecks / this.totalChecks) * 100);
        this.updateProgress(percent, `Testing ${collId}/${param} (${this.completedChecks}/${this.totalChecks})`);

        // Log request
        this.logRequest(result);

        return result;
    }

    /**
     * Make a single position query and check result
     */
    async testPositionQuery(collectionId, parameter, level, time) {
        const coords = `POINT(${this.testPoint.lon} ${this.testPoint.lat})`;
        let url = `${this.endpoint}/collections/${collectionId}/position?coords=${encodeURIComponent(coords)}&parameter-name=${parameter}`;

        if (level !== null) {
            url += `&z=${level}`;
        }
        if (time) {
            url += `&datetime=${encodeURIComponent(time)}`;
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
                    status: 'fail',
                    message: `HTTP ${response.status}: ${response.statusText}`,
                    duration,
                    url
                };
            }

            const data = await response.json();
            const values = data.ranges?.[parameter]?.values || [];
            const hasData = values.some(v => v !== null);

            return {
                collection: collectionId,
                parameter,
                level,
                time,
                status: hasData ? 'pass' : 'warn',
                message: hasData ? 'Data retrieved' : 'Query OK but null values',
                values: values.slice(0, 5),
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
                status: 'fail',
                message: error.message,
                duration: Math.round(performance.now() - startTime),
                url
            };
        }
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
        entry.innerHTML = `
            <span class="time">${new Date().toLocaleTimeString()}</span>
            <span class="status ${result.status}">${result.status.toUpperCase()}</span>
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
        const inCatalogClass = paramCheck.inCatalog ? 'pass' : 'fail';

        el.innerHTML = `
            <div class="parameter-header">
                <span class="toggle">▶</span>
                <span class="name">${param}</span>
                <span class="status-badge ${inCatalogClass}">${paramCheck.inCatalog ? 'In Catalog' : 'Missing'}</span>
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
            // No levels - show single status
            const statusEl = document.createElement('div');
            statusEl.className = 'tree-level';
            statusEl.innerHTML = `
                <span class="level-name">surface</span>
                <span class="level-status">
                    <span class="status-badge ${inCatalogClass}">DB: ${paramCheck.inCatalog ? 'exists' : 'MISSING'}</span>
                </span>
            `;
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

        const dbStatus = levelCheck.inCatalog ? 'pass' : 'fail';
        const dbLabel = levelCheck.inCatalog ? 'exists' : 'MISSING';

        let queryStatus = 'skip';
        let queryLabel = 'skipped';
        if (levelCheck.queryResult) {
            queryStatus = levelCheck.queryResult;
            queryLabel = levelCheck.queryResult === 'pass' ? 'OK' : 
                        levelCheck.queryResult === 'warn' ? 'null' : 'ERROR';
        }

        el.innerHTML = `
            <span class="level-name">${level} ${collId.includes('isobaric') ? 'hPa' : ''}</span>
            <span class="level-status">
                <span class="status-badge ${dbStatus}">DB: ${dbLabel}</span>
                <span class="status-badge ${queryStatus}">Query: ${queryLabel}</span>
            </span>
        `;

        el.addEventListener('click', () => {
            this.showDetails(collId, param, level, levelCheck);
        });

        return el;
    }

    /**
     * Calculate collection coverage percentage
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
                    if (paramCheck.levels[level].inCatalog) available++;
                }
            } else {
                total++;
                if (paramCheck.inCatalog) available++;
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
                    <span class="value">${check.inCatalog ? 'Yes' : 'No'}</span>
                </div>
                <div class="detail-item">
                    <span class="label">Message</span>
                    <span class="value">${check.message || check.queryMessage || '-'}</span>
                </div>
            </div>
        `;

        if (check.queryUrl) {
            html += `
                <div class="detail-section">
                    <h3>HTTP Request</h3>
                    <div class="detail-item">
                        <span class="label">Status</span>
                        <span class="value status-badge ${check.queryResult}">${check.queryResult?.toUpperCase()}</span>
                    </div>
                    <div class="detail-item">
                        <span class="label">Duration</span>
                        <span class="value">${check.queryDuration}ms</span>
                    </div>
                    <div class="detail-request">${check.queryUrl}</div>
                </div>
            `;

            if (check.queryValues) {
                html += `
                    <div class="detail-section">
                        <h3>Response Values</h3>
                        <div class="detail-response">${JSON.stringify(check.queryValues, null, 2)}</div>
                    </div>
                `;
            }
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
        // Update button states
        document.querySelectorAll('.filter-btn').forEach(btn => {
            btn.classList.toggle('active', btn.dataset.filter === filter);
        });

        // Apply filter
        document.querySelectorAll('.tree-collection').forEach(collEl => {
            const collId = collEl.dataset.collId;
            const coll = this.results.advertised[collId];

            if (!coll) return;

            let show = filter === 'all';

            if (!show) {
                // Check if collection has any items matching the filter
                for (const param of coll.parameters) {
                    const paramCheck = coll.parameterChecks[param];
                    
                    if (filter === 'fail' && !paramCheck.inCatalog) {
                        show = true;
                        break;
                    }

                    for (const levelCheck of Object.values(paramCheck.levels || {})) {
                        if (filter === 'fail' && !levelCheck.inCatalog) show = true;
                        if (filter === 'warn' && levelCheck.queryResult === 'warn') show = true;
                        if (filter === 'pass' && levelCheck.inCatalog) show = true;
                    }
                }
            }

            collEl.style.display = show ? '' : 'none';
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
