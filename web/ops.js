// Ops page: actions only. Monitoring lives in Grafana.
//
// Same URL rule as app.js: localhost:8000 (docker-compose) talks to the service ports,
// anything else uses relative URLs behind the gateway.
const IS_LOCAL_DEV = window.location.port === '8000';
const API_BASE = IS_LOCAL_DEV ? 'http://localhost:8080' : '';
const DOWNLOADER_URL = IS_LOCAL_DEV ? 'http://localhost:8081' : '/downloader';

// The API rejects non-GET requests without this header (CSRF guard).
async function post(url) {
    const response = await fetch(url, { method: 'POST', headers: { 'X-Admin-Request': '1' } });
    const text = await response.text();
    if (!response.ok) throw new Error(`HTTP ${response.status}: ${text}`);
    try { return JSON.parse(text); } catch { return text; }
}

async function get(url) {
    const response = await fetch(url, { cache: 'no-cache' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.json();
}

const card = id => document.getElementById(id);
const role = (id, name) => card(id).querySelector(`[data-role="${name}"]`);
const show = (el, value) => { el.textContent = typeof value === 'string' ? value : JSON.stringify(value, null, 2); };

// Run an action: disable its button, show the outcome (or the error) in the output element.
async function run(button, out, fn) {
    button.disabled = true;
    show(out, 'Working...');
    try {
        show(out, await fn());
    } catch (error) {
        show(out, `Failed: ${error.message}`);
    } finally {
        button.disabled = false;
    }
}

// ---- Retention cleanup ----

async function loadCleanupStatus() {
    const out = role('cleanup', 'status');
    try {
        const s = await get(`${API_BASE}/api/admin/cleanup/status`);
        const lines = [
            `Enabled: ${s.enabled}`,
            `Interval: ${s.interval_secs}s, next run in: ${s.next_run_in_secs ?? 'unknown'}s`,
            `Last run: ${s.last_run ?? 'never'}`,
            `Expired datasets waiting: ${s.expired_count}`,
            `Total to purge: ${(s.total_purge_size_bytes / 1048576).toFixed(1)} MiB`,
            '',
            ...(s.purge_preview || []).map(p =>
                `${p.model}: keep ${p.retention_hours}h, ${p.dataset_count} datasets to purge, oldest ${p.oldest_data ?? 'none'}`),
        ];
        show(out, lines.join('\n'));
    } catch (error) {
        show(out, `Could not load status: ${error.message}`);
    }
}

// ---- Catalog and storage sync ----

async function loadSyncStatus() {
    const out = role('sync', 'status');
    try {
        const s = await get(`${API_BASE}/api/admin/sync/status`);
        show(out, [
            `Orphan database records: ${s.orphan_db_records} (of ${s.db_records_checked} checked)`,
            `Orphan storage objects: ${s.orphan_minio_objects} (of ${s.minio_objects_checked} checked)`,
            s.message,
            ...(s.errors || []),
        ].join('\n'));
    } catch (error) {
        show(out, `Could not load status: ${error.message}`);
    }
}

function describeSyncPreview(p) {
    const list = (title, paths) => `${title} (${paths.length}):\n${paths.slice(0, 50).join('\n')}${paths.length > 50 ? `\n... and ${paths.length - 50} more` : ''}`;
    return [
        `Checked ${p.db_records_checked} database records and ${p.minio_objects_checked} storage objects.`,
        list('Orphan database records', p.orphan_db_paths || []),
        list('Orphan storage objects', p.orphan_minio_paths || []),
    ].join('\n\n');
}

// ---- Failed downloads ----

async function loadFailed() {
    const list = role('failed', 'list');
    list.textContent = 'Loading...';
    try {
        const data = await get(`${DOWNLOADER_URL}/downloads?limit=100`);
        const failed = data.failed || [];
        if (failed.length === 0) { list.textContent = 'No failed downloads.'; return; }
        const table = document.createElement('table');
        const head = table.createTHead().insertRow();
        ['File', 'Model', 'Retries', 'Error', ''].forEach(h => { head.insertCell().textContent = h; });
        const body = table.createTBody();
        for (const d of failed) {
            const row = body.insertRow();
            row.insertCell().textContent = d.filename;
            row.insertCell().textContent = d.model ?? '';
            row.insertCell().textContent = d.retry_count;
            row.insertCell().textContent = d.error_message ?? '';
            const button = document.createElement('button');
            button.textContent = 'Retry';
            button.addEventListener('click', () => run(button, role('failed', 'result'), async () => {
                const r = await post(`${DOWNLOADER_URL}/retry?url=${encodeURIComponent(d.url)}`);
                loadFailed();
                return r.message || r;
            }));
            row.insertCell().appendChild(button);
        }
        list.replaceChildren(table);
    } catch (error) {
        list.textContent = `Could not load downloads: ${error.message}`;
    }
}

// ---- Validation ----

function describeValidation(v) {
    return ['wms', 'wmts'].map(name => {
        const s = v[name] || {};
        const checks = Object.entries(s.checks || {});
        const failing = checks.filter(([, c]) => c && c.status !== 'pass').map(([k]) => k);
        return `${name.toUpperCase()}: ${s.status ?? 'unknown'}, ${checks.length - failing.length}/${checks.length} checks passed${failing.length ? `\n  not passing: ${failing.join(', ')}` : ''}`;
    }).join('\n');
}

// ---- Model config (read-only) ----

async function loadModels() {
    const select = card('model-select');
    const out = card('model-yaml');
    try {
        const data = await get(`${API_BASE}/api/admin/config/models`);
        for (const m of data.models || []) {
            const option = document.createElement('option');
            option.value = m.id;
            option.textContent = `${m.id} (${m.model_type}, ${m.parameter_count} parameters)`;
            select.appendChild(option);
        }
        const showYaml = async () => {
            if (!select.value) { out.textContent = 'No models.'; return; }
            out.textContent = 'Loading...';
            try {
                out.textContent = (await get(`${API_BASE}/api/admin/config/models/${encodeURIComponent(select.value)}`)).yaml;
            } catch (error) {
                out.textContent = `Could not load config: ${error.message}`;
            }
        };
        select.addEventListener('change', showYaml);
        showYaml();
    } catch (error) {
        out.textContent = `Could not load models: ${error.message}`;
    }
}

// ---- Wiring ----

document.addEventListener('click', event => {
    const button = event.target.closest('button[data-act]');
    if (!button) return;
    const section = button.closest('.ops-card').id;
    const result = role(section, 'result');
    switch (button.dataset.act) {
        case 'cleanup-run':
            if (!confirm('Delete expired data now? Files and database records past retention are removed and cannot be restored.')) return;
            run(button, result, async () => {
                const r = await post(`${API_BASE}/api/admin/cleanup/run`);
                loadCleanupStatus();
                return r;
            });
            break;
        case 'sync-preview':
            run(button, result, async () => describeSyncPreview(await get(`${API_BASE}/api/admin/sync/preview`)));
            break;
        case 'sync-run':
            if (!confirm('Delete orphan database records and orphan storage objects? Use Preview first. This cannot be undone.')) return;
            run(button, result, async () => {
                const r = await post(`${API_BASE}/api/admin/sync/run`);
                loadSyncStatus();
                return r;
            });
            break;
        case 'failed-refresh':
            loadFailed();
            break;
        case 'post':
            if (button.dataset.confirm && !confirm(button.dataset.confirm)) return;
            run(button, result, () => post(`${API_BASE}${button.dataset.path}`));
            break;
        case 'validate':
            run(button, result, async () => describeValidation(await post(`${API_BASE}/api/validation/run`)));
            break;
    }
});

loadCleanupStatus();
loadSyncStatus();
loadFailed();
loadModels();
