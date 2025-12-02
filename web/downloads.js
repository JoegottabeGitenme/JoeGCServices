// Downloads Dashboard JavaScript
// Handles loading and displaying downloader status data

// Configurable URLs - downloader runs on port 8081
const DOWNLOADER_URL = 'http://localhost:8081';
const REFRESH_INTERVAL = 5000; // 5 seconds

let refreshIntervalId = null;
let downloadsChart = null;
let bytesChart = null;

// Initialize dashboard on load
document.addEventListener('DOMContentLoaded', () => {
    console.log('Downloads Dashboard initializing...');
    
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
    
    // Initialize charts
    initCharts();
    
    // Initial load
    loadAllData();
    
    // Auto-refresh every 5 seconds
    startAutoRefresh();
});

// Start auto-refresh
function startAutoRefresh() {
    if (refreshIntervalId) {
        clearInterval(refreshIntervalId);
    }
    
    refreshIntervalId = setInterval(() => {
        loadAllData();
    }, REFRESH_INTERVAL);
}

// Load all dashboard data
async function loadAllData() {
    await Promise.all([
        loadStatus(),
        loadDownloads(),
        loadSchedule(),
        loadTimeSeries()
    ]);
}

// Load overall status
async function loadStatus() {
    try {
        const response = await fetch(`${DOWNLOADER_URL}/status`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        
        // Update service status
        document.getElementById('service-dot').className = 'status-dot online';
        document.getElementById('service-status-text').textContent = `${data.status}`;
        
        // Update stats
        document.getElementById('stat-pending').textContent = data.stats.pending;
        document.getElementById('stat-in-progress').textContent = data.stats.in_progress;
        document.getElementById('stat-completed').textContent = data.stats.completed;
        document.getElementById('stat-failed').textContent = data.stats.failed;
        document.getElementById('stat-bytes').textContent = formatBytes(data.stats.total_bytes_downloaded);
        document.getElementById('stat-pending-ingest').textContent = data.pending_ingestion;
        
    } catch (error) {
        console.error('Error loading status:', error);
        document.getElementById('service-dot').className = 'status-dot offline';
        document.getElementById('service-status-text').textContent = 'Offline';
        
        // Clear stats
        ['stat-pending', 'stat-in-progress', 'stat-completed', 'stat-failed', 'stat-bytes', 'stat-pending-ingest'].forEach(id => {
            document.getElementById(id).textContent = '--';
        });
    }
}

// Load downloads list
async function loadDownloads() {
    try {
        const response = await fetch(`${DOWNLOADER_URL}/downloads?limit=50`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        
        // Render active downloads
        renderDownloadList('active-downloads', data.active, 'active');
        
        // Render pending downloads
        renderDownloadList('pending-downloads', data.pending, 'pending');
        
        // Render completed downloads
        renderCompletedList('completed-downloads', data.recent_completed);
        
        // Render failed downloads
        renderDownloadList('failed-downloads', data.failed, 'failed');
        
    } catch (error) {
        console.error('Error loading downloads:', error);
        ['active-downloads', 'pending-downloads', 'completed-downloads', 'failed-downloads'].forEach(id => {
            document.getElementById(id).innerHTML = '<div class="error-message">Failed to load downloads</div>';
        });
    }
}

// Render download list
function renderDownloadList(containerId, downloads, type) {
    const container = document.getElementById(containerId);
    
    if (!downloads || downloads.length === 0) {
        container.innerHTML = `<div class="empty-state">
            <div class="empty-state-icon">${type === 'active' ? 'downloading' : type === 'pending' ? 'queue' : 'error'}</div>
            <div>No ${type} downloads</div>
        </div>`;
        return;
    }
    
    container.innerHTML = downloads.map(d => `
        <div class="download-item">
            <div class="download-header">
                <span class="download-filename">${d.filename}</span>
                ${d.model ? `<span class="download-model">${d.model}</span>` : ''}
            </div>
            <div class="download-meta">
                <span>Status: <span class="status-badge ${getStatusClass(d.status)}">${d.status}</span></span>
                ${d.total_bytes ? `<span>Size: ${formatBytes(d.total_bytes)}</span>` : ''}
                <span>Retries: ${d.retry_count}</span>
            </div>
            ${d.progress_percent !== null && d.progress_percent !== undefined ? `
            <div class="download-progress">
                <div class="progress-bar">
                    <div class="progress-fill" style="width: ${d.progress_percent}%"></div>
                </div>
                <div class="progress-text">
                    <span>${formatBytes(d.downloaded_bytes)} / ${d.total_bytes ? formatBytes(d.total_bytes) : 'Unknown'}</span>
                    <span>${d.progress_percent.toFixed(1)}%</span>
                </div>
            </div>
            ` : ''}
            ${d.error_message ? `<div class="error-message" style="margin-top: 0.5rem; font-size: 0.85rem">${d.error_message}</div>` : ''}
            ${type === 'failed' ? `<button class="retry-btn" onclick="retryDownload('${encodeURIComponent(d.url)}')">Retry</button>` : ''}
        </div>
    `).join('');
}

// Render completed downloads list
function renderCompletedList(containerId, downloads) {
    const container = document.getElementById(containerId);
    
    if (!downloads || downloads.length === 0) {
        container.innerHTML = `<div class="empty-state">
            <div class="empty-state-icon">check</div>
            <div>No recent completed downloads</div>
        </div>`;
        return;
    }
    
    container.innerHTML = downloads.map(d => `
        <div class="download-item">
            <div class="download-header">
                <span class="download-filename">${d.filename}</span>
                <div>
                    ${d.model ? `<span class="download-model">${d.model}</span>` : ''}
                    <span class="status-badge ${d.ingested ? 'success' : 'warning'}">${d.ingested ? 'Ingested' : 'Pending'}</span>
                </div>
            </div>
            <div class="download-meta">
                ${d.total_bytes ? `<span>Size: ${formatBytes(d.total_bytes)}</span>` : ''}
                <span>Completed: ${formatTime(d.completed_at)}</span>
            </div>
        </div>
    `).join('');
}

// Load schedule
async function loadSchedule() {
    try {
        const response = await fetch(`${DOWNLOADER_URL}/schedule`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        
        // Render schedule table
        renderScheduleTable(data.models);
        
        // Render next checks
        renderNextChecks(data.next_checks);
        
    } catch (error) {
        console.error('Error loading schedule:', error);
        document.getElementById('schedule-body').innerHTML = '<tr><td colspan="7" class="error-message">Failed to load schedule</td></tr>';
        document.getElementById('next-checks').innerHTML = '<div class="error-message">Failed to load upcoming checks</div>';
    }
}

// Render schedule table
function renderScheduleTable(models) {
    const tbody = document.getElementById('schedule-body');
    
    if (!models || models.length === 0) {
        tbody.innerHTML = '<tr><td colspan="7">No models configured</td></tr>';
        return;
    }
    
    tbody.innerHTML = models.map(m => `
        <tr>
            <td><strong>${m.id}</strong></td>
            <td>
                <span class="status-badge ${m.enabled ? 'success' : 'warning'}">
                    ${m.enabled ? 'Enabled' : 'Disabled'}
                </span>
            </td>
            <td>${m.bucket}</td>
            <td>
                <div class="cycles-list">
                    ${m.cycles.map(c => `<span class="cycle-badge">${String(c).padStart(2, '0')}Z</span>`).join('')}
                </div>
            </td>
            <td>${m.delay_hours}h</td>
            <td>${formatDuration(m.poll_interval_secs)}</td>
            <td>${m.forecast_hours.length} hours (${m.forecast_hours.slice(0, 5).join(', ')}${m.forecast_hours.length > 5 ? '...' : ''})</td>
        </tr>
    `).join('');
}

// Render next checks
function renderNextChecks(checks) {
    const container = document.getElementById('next-checks');
    
    if (!checks || checks.length === 0) {
        container.innerHTML = '<div class="empty-state">No upcoming checks</div>';
        return;
    }
    
    container.innerHTML = `
        <div class="stats-grid" style="grid-template-columns: repeat(auto-fit, minmax(200px, 1fr))">
            ${checks.map(c => `
                <div class="stat-card">
                    <div class="stat-value info">${c.next_cycle}</div>
                    <div class="stat-label">
                        <strong>${c.model}</strong><br>
                        Available ${c.expected_available}<br>
                        ${c.files_expected} files expected
                    </div>
                </div>
            `).join('')}
        </div>
    `;
}

// Load time series data
async function loadTimeSeries() {
    try {
        const response = await fetch(`${DOWNLOADER_URL}/timeseries?hours=24`);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        
        const data = await response.json();
        
        // Update charts
        updateCharts(data.hourly);
        
    } catch (error) {
        console.error('Error loading time series:', error);
    }
}

// Initialize charts
function initCharts() {
    const chartOptions = {
        responsive: true,
        maintainAspectRatio: false,
        plugins: {
            legend: {
                display: false
            }
        },
        scales: {
            x: {
                grid: {
                    color: 'rgba(255, 255, 255, 0.1)'
                },
                ticks: {
                    color: 'rgba(255, 255, 255, 0.6)'
                }
            },
            y: {
                grid: {
                    color: 'rgba(255, 255, 255, 0.1)'
                },
                ticks: {
                    color: 'rgba(255, 255, 255, 0.6)'
                },
                beginAtZero: true
            }
        }
    };
    
    // Downloads chart
    const downloadsCtx = document.getElementById('downloads-chart').getContext('2d');
    downloadsChart = new Chart(downloadsCtx, {
        type: 'bar',
        data: {
            labels: [],
            datasets: [{
                label: 'Downloads',
                data: [],
                backgroundColor: 'rgba(100, 181, 246, 0.7)',
                borderColor: '#64b5f6',
                borderWidth: 1
            }]
        },
        options: chartOptions
    });
    
    // Bytes chart
    const bytesCtx = document.getElementById('bytes-chart').getContext('2d');
    bytesChart = new Chart(bytesCtx, {
        type: 'line',
        data: {
            labels: [],
            datasets: [{
                label: 'Bytes Downloaded',
                data: [],
                backgroundColor: 'rgba(76, 175, 80, 0.2)',
                borderColor: '#4caf50',
                borderWidth: 2,
                fill: true,
                tension: 0.4
            }]
        },
        options: {
            ...chartOptions,
            scales: {
                ...chartOptions.scales,
                y: {
                    ...chartOptions.scales.y,
                    ticks: {
                        ...chartOptions.scales.y.ticks,
                        callback: function(value) {
                            return formatBytes(value);
                        }
                    }
                }
            }
        }
    });
}

// Update charts with new data
function updateCharts(hourly) {
    if (!hourly || hourly.length === 0) {
        return;
    }
    
    const labels = hourly.map(h => h.hour.split(' ')[1] || h.hour);
    const downloadCounts = hourly.map(h => h.download_count);
    const bytesDownloaded = hourly.map(h => h.bytes_downloaded);
    
    // Update downloads chart
    downloadsChart.data.labels = labels;
    downloadsChart.data.datasets[0].data = downloadCounts;
    downloadsChart.update();
    
    // Update bytes chart
    bytesChart.data.labels = labels;
    bytesChart.data.datasets[0].data = bytesDownloaded;
    bytesChart.update();
}

// Retry a failed download
async function retryDownload(encodedUrl) {
    const url = decodeURIComponent(encodedUrl);
    
    try {
        const response = await fetch(`${DOWNLOADER_URL}/retry?url=${encodedUrl}`);
        const data = await response.json();
        
        if (data.success) {
            alert('Download queued for retry');
            loadAllData();
        } else {
            alert(`Retry failed: ${data.message}`);
        }
    } catch (error) {
        alert(`Error: ${error.message}`);
    }
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

// Helper: Get status class
function getStatusClass(status) {
    switch (status) {
        case 'completed': return 'success';
        case 'in_progress': return 'info';
        case 'pending': return 'info';
        case 'retrying': return 'warning';
        case 'failed': return 'error';
        default: return '';
    }
}

// Helper: Format bytes
function formatBytes(bytes) {
    if (bytes === 0 || bytes === null || bytes === undefined) return '0 B';
    
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

// Helper: Format duration
function formatDuration(seconds) {
    if (seconds < 60) return `${seconds}s`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
    return `${Math.floor(seconds / 3600)}h`;
}

// Helper: Format time
function formatTime(timestamp) {
    try {
        const date = new Date(timestamp);
        return date.toLocaleTimeString();
    } catch {
        return timestamp;
    }
}

// Cleanup on unload
window.addEventListener('beforeunload', () => {
    if (refreshIntervalId) {
        clearInterval(refreshIntervalId);
    }
});
