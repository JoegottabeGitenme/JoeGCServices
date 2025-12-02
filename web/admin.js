// Admin Dashboard JavaScript
// Handles loading and displaying admin data from the WMS API

const API_BASE_URL = 'http://localhost:8080';
const REFRESH_INTERVAL = 10000; // 10 seconds

let refreshIntervalId = null;
let selectedModel = null;
let originalYaml = null; // Store original YAML for reset

// Initialize dashboard on load
document.addEventListener('DOMContentLoaded', () => {
    console.log('Admin Dashboard initializing...');
    
    // Set up event listeners
    document.getElementById('refresh-btn').addEventListener('click', () => {
        console.log('Manual refresh triggered');
        loadAllData();
    });
    
    // Tab navigation
    document.querySelectorAll('.tab-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            const tabId = e.target.dataset.tab;
            switchTab(tabId);
        });
    });
    
    // Config editor buttons
    document.getElementById('validate-btn')?.addEventListener('click', validateConfig);
    document.getElementById('reset-btn')?.addEventListener('click', resetConfig);
    document.getElementById('save-btn')?.addEventListener('click', saveConfig);
    
    // Initial load
    loadAllData();
    
    // Auto-refresh every 10 seconds
    startAutoRefresh();
});

// Start auto-refresh
function startAutoRefresh() {
    if (refreshIntervalId) {
        clearInterval(refreshIntervalId);
    }
    
    refreshIntervalId = setInterval(() => {
        console.log('Auto-refresh triggered');
        loadAllData();
    }, REFRESH_INTERVAL);
}

// Load all dashboard data
async function loadAllData() {
    await Promise.all([
        loadSystemStatus(),
        loadCatalogSummary(),
        loadModelConfigs(),
        loadIngestionLog(),
        loadCleanupStatus()
    ]);
}

// Load system status
async function loadSystemStatus() {
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/ingestion/status`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        const systemInfo = data.system_info || {};
        
        // Update service status
        const statusBadge = document.getElementById('service-status');
        statusBadge.innerHTML = `<span class="status-badge success">Online</span>`;
        
        // Update CPU cores
        document.getElementById('cpu-cores').textContent = systemInfo.cpu_cores || '--';
        
        // Update worker threads
        document.getElementById('worker-threads').textContent = systemInfo.worker_threads || '--';
        
        // Update uptime
        const uptime = systemInfo.uptime_seconds ? formatUptime(systemInfo.uptime_seconds) : '--';
        document.getElementById('system-uptime').textContent = uptime;
        
    } catch (error) {
        console.error('Error loading system status:', error);
        const statusBadge = document.getElementById('service-status');
        statusBadge.innerHTML = `<span class="status-badge error">Error</span>`;
    }
}

// Load catalog summary
async function loadCatalogSummary() {
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/ingestion/status`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        
        // Update catalog stats
        document.getElementById('total-datasets').textContent = data.catalog_summary?.total_datasets || '0';
        document.getElementById('total-models').textContent = data.models?.length || '0';
        
        // Display storage breakdown from API
        const totalSizeBytes = data.catalog_summary?.total_size_bytes || 0;
        const rawSizeBytes = data.catalog_summary?.raw_size_bytes || 0;
        const shreddedSizeBytes = data.catalog_summary?.shredded_size_bytes || 0;
        
        document.getElementById('catalog-size').textContent = formatBytes(totalSizeBytes);
        document.getElementById('raw-size').textContent = formatBytes(rawSizeBytes);
        document.getElementById('shredded-size').textContent = formatBytes(shreddedSizeBytes);
        
        // Find the most recent ingest time across all models
        const modelsWithIngest = (data.models || []).filter(m => m.last_ingest);
        if (modelsWithIngest.length > 0) {
            // Sort by timestamp descending and get the most recent
            modelsWithIngest.sort((a, b) => b.last_ingest.localeCompare(a.last_ingest));
            document.getElementById('latest-ingest').textContent = modelsWithIngest[0].last_ingest;
        } else {
            document.getElementById('latest-ingest').textContent = 'Never';
        }
        
    } catch (error) {
        console.error('Error loading catalog summary:', error);
        document.getElementById('total-datasets').textContent = '--';
        document.getElementById('total-models').textContent = '--';
        document.getElementById('catalog-size').textContent = '--';
        document.getElementById('raw-size').textContent = '--';
        document.getElementById('shredded-size').textContent = '--';
        document.getElementById('latest-ingest').textContent = '--';
    }
}

// Load ingestion log
async function loadIngestionLog() {
    const container = document.getElementById('log-container');
    
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/ingestion/log?limit=50`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        renderIngestionLog(data.entries || []);
        
    } catch (error) {
        console.error('Error loading ingestion log:', error);
        container.innerHTML = `<div class="loading">No recent ingestion activity</div>`;
    }
}

// Render ingestion log entries
function renderIngestionLog(entries) {
    const container = document.getElementById('log-container');
    
    if (entries.length === 0) {
        container.innerHTML = '<div class="loading">No recent ingestion activity</div>';
        return;
    }
    
    container.innerHTML = entries.map(entry => `
        <div class="log-entry">
            <span class="timestamp">${formatTimestamp(entry.timestamp)}</span>
            <span class="model">${entry.model}</span>
            <span class="param">${entry.parameter}</span>
            <span class="level">${entry.level}</span>
            <span class="path" title="${entry.storage_path}">${shortenPath(entry.storage_path)}</span>
        </div>
    `).join('');
}

// Format timestamp for log display
function formatTimestamp(timestamp) {
    // Extract just time portion
    const parts = timestamp.split(' ');
    return parts[1] || timestamp;
}

// Shorten storage path for display
function shortenPath(path) {
    if (path.length > 50) {
        return '...' + path.slice(-47);
    }
    return path;
}

// Load cleanup/retention status
async function loadCleanupStatus() {
    const container = document.getElementById('cleanup-status-container');
    
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/cleanup/status`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        renderCleanupStatus(data);
        
    } catch (error) {
        console.error('Error loading cleanup status:', error);
        container.innerHTML = `<div class="error-message">Failed to load cleanup status: ${error.message}</div>`;
    }
}

// Render cleanup status
function renderCleanupStatus(data) {
    const container = document.getElementById('cleanup-status-container');
    
    // Calculate next run time
    const nextRunMins = Math.round(data.next_run_in_secs / 60);
    const intervalMins = Math.round(data.interval_secs / 60);
    
    // Build overview stats
    const overviewHtml = `
        <div class="cleanup-overview">
            <div class="cleanup-stat">
                <div class="cleanup-stat-value">${data.enabled ? '✓ Active' : '✗ Disabled'}</div>
                <div class="cleanup-stat-label">Cleanup Status</div>
            </div>
            <div class="cleanup-stat">
                <div class="cleanup-stat-value">${intervalMins} min</div>
                <div class="cleanup-stat-label">Cleanup Interval</div>
            </div>
            <div class="cleanup-stat">
                <div class="cleanup-stat-value">${nextRunMins} min</div>
                <div class="cleanup-stat-label">Next Run In</div>
            </div>
            <div class="cleanup-stat">
                <div class="cleanup-stat-value ${data.expired_count > 0 ? 'warning' : ''}">${data.expired_count}</div>
                <div class="cleanup-stat-label">Expired Datasets</div>
            </div>
            <div class="cleanup-stat">
                <div class="cleanup-stat-value ${data.total_purge_size_bytes > 0 ? 'warning' : ''}">${formatBytes(data.total_purge_size_bytes)}</div>
                <div class="cleanup-stat-label">To Be Purged</div>
            </div>
        </div>
    `;
    
    // Build retention table
    let tableHtml = `
        <table class="retention-table">
            <thead>
                <tr>
                    <th>Model</th>
                    <th>Retention</th>
                    <th>Cutoff Time</th>
                    <th>Oldest Data</th>
                    <th>Next Purge In</th>
                    <th>Files to Purge</th>
                    <th>Size</th>
                </tr>
            </thead>
            <tbody>
    `;
    
    // Sort by model name
    const previews = data.purge_preview || [];
    previews.sort((a, b) => a.model.localeCompare(b.model));
    
    for (const preview of previews) {
        const purgeClass = preview.dataset_count === 0 ? 'none' : 
                          preview.dataset_count < 10 ? 'some' : 'many';
        
        tableHtml += `
            <tr>
                <td><strong>${preview.model}</strong></td>
                <td>${preview.retention_hours} hours</td>
                <td class="time-until">${preview.cutoff_time || 'N/A'}</td>
                <td class="time-until">${preview.oldest_data || 'No data'}</td>
                <td class="time-until">${preview.next_purge_in || 'N/A'}</td>
                <td>
                    <span class="purge-count ${purgeClass}">
                        ${preview.dataset_count} files
                    </span>
                </td>
                <td>${formatBytes(preview.total_size_bytes)}</td>
            </tr>
        `;
    }
    
    tableHtml += `
            </tbody>
        </table>
    `;
    
    // Add manual cleanup button
    const actionsHtml = `
        <div class="cleanup-actions">
            <button class="btn btn-secondary" onclick="runManualCleanup()">🗑️ Run Cleanup Now</button>
        </div>
    `;
    
    container.innerHTML = overviewHtml + tableHtml + actionsHtml;
}

// Run manual cleanup
async function runManualCleanup() {
    if (!confirm('This will permanently delete expired datasets. Continue?')) {
        return;
    }
    
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/cleanup/run`, {
            method: 'POST'
        });
        
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        alert(`Cleanup complete!\n\nMarked expired: ${data.marked_expired}\nFiles deleted: ${data.files_deleted}\nRecords removed: ${data.records_removed}`);
        
        // Refresh the status
        loadCleanupStatus();
        loadCatalogSummary();
        
    } catch (error) {
        console.error('Error running cleanup:', error);
        alert(`Failed to run cleanup: ${error.message}`);
    }
}

// Load model configurations
async function loadModelConfigs() {
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/config/models`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        renderModelList(data.models || []);
        
    } catch (error) {
        console.error('Error loading model configs:', error);
        const container = document.getElementById('models-container');
        container.innerHTML = `<div class="error-message">Failed to load model configurations: ${error.message}</div>`;
    }
}

// Render model list
function renderModelList(models) {
    const container = document.getElementById('models-container');
    
    if (models.length === 0) {
        container.innerHTML = '<div class="loading">No model configurations found</div>';
        return;
    }
    
    container.innerHTML = '';
    const modelList = document.createElement('div');
    modelList.className = 'model-list';
    
    models.forEach(model => {
        const modelItem = document.createElement('div');
        modelItem.className = 'model-item';
        modelItem.dataset.modelId = model.id;
        
        if (selectedModel === model.id) {
            modelItem.classList.add('selected');
        }
        
        const sourceType = model.source_type || 'unknown';
        const sourceBadge = getSourceTypeBadge(sourceType);
        
        modelItem.innerHTML = `
            <div class="model-header">
                <div class="model-name">${model.name || model.id}</div>
                ${sourceBadge}
            </div>
            <div class="model-info">
                <div><strong>ID:</strong> ${model.id}</div>
                <div><strong>Source:</strong> ${sourceType}</div>
                <div><strong>Projection:</strong> ${model.projection || 'N/A'}</div>
                <div><strong>Parameters:</strong> ${model.parameter_count || 0}</div>
            </div>
        `;
        
        modelItem.addEventListener('click', () => {
            selectModel(model.id);
        });
        
        modelList.appendChild(modelItem);
    });
    
    container.appendChild(modelList);
}

// Select a model and load its details
async function selectModel(modelId) {
    // Update selection UI
    document.querySelectorAll('.model-item').forEach(item => {
        item.classList.toggle('selected', item.dataset.modelId === modelId);
    });
    
    selectedModel = modelId;
    
    // Show details panel
    const detailsPanel = document.getElementById('model-details');
    detailsPanel.style.display = 'block';
    
    // Update title
    document.getElementById('config-model-name').textContent = `${modelId} Configuration`;
    
    // Load all tabs content
    await Promise.all([
        loadModelYaml(modelId),
        loadShredPreview(modelId)
    ]);
    
    // Scroll to details
    detailsPanel.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
}

// Load model YAML
async function loadModelYaml(modelId) {
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/config/models/${modelId}`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        const yaml = data.yaml || 'No configuration available';
        
        // Update view tab
        document.getElementById('config-yaml').textContent = yaml;
        
        // Update editor tab
        document.getElementById('config-editor-textarea').value = yaml;
        
        // Store original for reset
        originalYaml = yaml;
        
        // Clear validation feedback
        document.getElementById('validation-feedback').innerHTML = '';
        
    } catch (error) {
        console.error('Error loading model YAML:', error);
        document.getElementById('config-yaml').textContent = `Error loading configuration: ${error.message}`;
    }
}

// Load shred preview
async function loadShredPreview(modelId) {
    const container = document.getElementById('shred-preview-container');
    container.innerHTML = '<div class="loading">Loading preview...</div>';
    
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/preview-shred?model=${modelId}`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        renderShredPreview(data);
        
    } catch (error) {
        console.error('Error loading shred preview:', error);
        container.innerHTML = `<div class="error-message">Failed to load preview: ${error.message}</div>`;
    }
}

// Render shred preview
function renderShredPreview(data) {
    const container = document.getElementById('shred-preview-container');
    
    const params = data.parameters_to_extract || [];
    
    container.innerHTML = `
        <div class="shred-preview">
            <div class="shred-header">
                <div>
                    <strong>${data.model_name}</strong>
                    <div style="font-size: 0.85rem; color: rgba(255,255,255,0.6);">
                        Source: ${data.source_type}
                    </div>
                </div>
                <div class="shred-summary">
                    <div class="shred-stat">
                        <div class="shred-stat-value">${params.length}</div>
                        <div class="shred-stat-label">Parameters</div>
                    </div>
                    <div class="shred-stat">
                        <div class="shred-stat-value">${data.total_extractions}</div>
                        <div class="shred-stat-label">Total Extractions</div>
                    </div>
                </div>
            </div>
            <div class="shred-params">
                ${params.map(param => `
                    <div class="shred-param">
                        <div class="shred-param-header">
                            <span class="shred-param-name">${param.name}</span>
                            <span class="status-badge info">${param.style}</span>
                        </div>
                        <div class="shred-param-levels">
                            ${param.description ? `<div style="margin-bottom: 0.25rem;">${param.description}</div>` : ''}
                            <div class="shred-levels-list">
                                ${param.levels.map(level => `
                                    <span class="shred-level-badge" title="${level.storage_path_template}">
                                        ${level.display}
                                    </span>
                                `).join('')}
                            </div>
                        </div>
                    </div>
                `).join('')}
            </div>
        </div>
    `;
}

// Tab switching
function switchTab(tabId) {
    // Update tab buttons
    document.querySelectorAll('.tab-btn').forEach(btn => {
        btn.classList.toggle('active', btn.dataset.tab === tabId);
    });
    
    // Update tab content
    document.querySelectorAll('.tab-content').forEach(content => {
        content.classList.toggle('active', content.id === tabId);
    });
}

// Validate configuration
async function validateConfig() {
    const yaml = document.getElementById('config-editor-textarea').value;
    const feedback = document.getElementById('validation-feedback');
    
    // Basic YAML syntax check (client-side)
    try {
        // We'll send to server for validation
        const response = await fetch(`${API_BASE_URL}/api/admin/config/models/${selectedModel}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ yaml, validate_only: true })
        });
        
        const data = await response.json();
        
        if (data.validation_errors && data.validation_errors.length > 0) {
            feedback.innerHTML = `
                <div class="validation-errors">
                    <strong>Validation Errors:</strong>
                    <ul>
                        ${data.validation_errors.map(e => `<li>${e}</li>`).join('')}
                    </ul>
                </div>
            `;
        } else {
            feedback.innerHTML = `
                <div class="success-message">
                    Configuration is valid!
                </div>
            `;
        }
    } catch (error) {
        // Try basic YAML parse
        feedback.innerHTML = `
            <div class="validation-errors">
                <strong>Error:</strong> ${error.message}
            </div>
        `;
    }
}

// Reset configuration to original
function resetConfig() {
    if (originalYaml) {
        document.getElementById('config-editor-textarea').value = originalYaml;
        document.getElementById('validation-feedback').innerHTML = '';
    }
}

// Save configuration
async function saveConfig() {
    if (!selectedModel) return;
    
    const yaml = document.getElementById('config-editor-textarea').value;
    const feedback = document.getElementById('validation-feedback');
    
    try {
        const response = await fetch(`${API_BASE_URL}/api/admin/config/models/${selectedModel}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ yaml })
        });
        
        const data = await response.json();
        
        if (data.success) {
            feedback.innerHTML = `
                <div class="success-message">
                    ${data.message}
                </div>
            `;
            
            // Update original
            originalYaml = yaml;
            
            // Refresh view tab
            document.getElementById('config-yaml').textContent = yaml;
            
            // Reload shred preview
            loadShredPreview(selectedModel);
        } else {
            feedback.innerHTML = `
                <div class="validation-errors">
                    <strong>${data.message}</strong>
                    ${data.validation_errors?.length > 0 ? `
                        <ul>
                            ${data.validation_errors.map(e => `<li>${e}</li>`).join('')}
                        </ul>
                    ` : ''}
                </div>
            `;
        }
    } catch (error) {
        feedback.innerHTML = `
            <div class="validation-errors">
                <strong>Error saving:</strong> ${error.message}
            </div>
        `;
    }
}

// Get source type badge
function getSourceTypeBadge(sourceType) {
    const badges = {
        'aws_s3': '<span class="status-badge info">AWS S3</span>',
        'aws_s3_goes': '<span class="status-badge info">GOES</span>',
        'aws_s3_grib2': '<span class="status-badge info">GRIB2</span>',
        'http': '<span class="status-badge warning">HTTP</span>',
        'local': '<span class="status-badge success">Local</span>'
    };
    
    return badges[sourceType] || `<span class="status-badge">${sourceType}</span>`;
}

// Format bytes to human-readable
function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

// Format uptime seconds to human-readable
function formatUptime(seconds) {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    
    const parts = [];
    if (days > 0) parts.push(`${days}d`);
    if (hours > 0) parts.push(`${hours}h`);
    if (minutes > 0) parts.push(`${minutes}m`);
    
    return parts.length > 0 ? parts.join(' ') : '< 1m';
}

// Get time ago from timestamp
function getTimeAgo(date) {
    const now = new Date();
    const seconds = Math.floor((now - date) / 1000);
    
    if (seconds < 60) return `${seconds}s ago`;
    
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
}

// Cleanup on unload
window.addEventListener('beforeunload', () => {
    if (refreshIntervalId) {
        clearInterval(refreshIntervalId);
    }
});
