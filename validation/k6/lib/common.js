// ============================================================================
// StormHagl / weather-wms load testing — shared helpers
// ============================================================================
import { Trend, Counter } from "k6/metrics";

// ---- Targets ---------------------------------------------------------------
// Direct-to-service (bypasses gateway rate limits; used for stress stages).
// Gateway path (realistic full stack; used for probes + overhead stage).
export const TARGETS = {
  direct: {
    wms: __ENV.WMS_BASE || "http://wms-api:8080",
    edr: __ENV.EDR_BASE || "http://edr-api:8083",
  },
  gateway: {
    wms: __ENV.GATEWAY_BASE || "http://gateway-nginx",
    edr: __ENV.GATEWAY_BASE || "http://gateway-nginx",
  },
};

export function target() {
  const mode = __ENV.TARGET || "direct";
  if (!TARGETS[mode]) {
    throw new Error(`Unknown TARGET '${mode}' (use direct|gateway)`);
  }
  return TARGETS[mode];
}

// ---- Cache-tier segmented latency metrics ----------------------------------
// WMS/WMTS responses carry X-Cache: L1-HIT | L2-HIT | MISS
export const wmtsTile = new Trend("wmts_tile_ms", true);
export const wmtsL1 = new Trend("wmts_tile_l1_ms", true);
export const wmtsL2 = new Trend("wmts_tile_l2_ms", true);
export const wmtsMiss = new Trend("wmts_tile_miss_ms", true);
export const wmsMap = new Trend("wms_getmap_ms", true);
export const wmsMiss = new Trend("wms_getmap_miss_ms", true);
export const edrPosition = new Trend("edr_position_ms", true);
export const edrRadius = new Trend("edr_radius_ms", true);
export const edrArea = new Trend("edr_area_ms", true);
export const edrCube = new Trend("edr_cube_ms", true);
export const edrItems = new Trend("edr_items_ms", true);
// Explicit per-tier counters: k6 Trend summaries don't include counts,
// so the report reads these for the cache-tier distribution table.
export const cacheL1Count = new Counter("cache_l1_total");
export const cacheL2Count = new Counter("cache_l2_total");
export const cacheMissCount = new Counter("cache_miss_total");

export function recordCacheTier(res, hitTrend2, missTrend, l1Trend) {
  const tier = res.headers["X-Cache"] || "UNKNOWN";
  if (tier === "L1-HIT") {
    cacheL1Count.add(1);
    if (l1Trend) l1Trend.add(res.timings.duration);
  } else if (tier === "L2-HIT") {
    cacheL2Count.add(1);
    if (hitTrend2) hitTrend2.add(res.timings.duration);
  } else if (tier === "MISS") {
    cacheMissCount.add(1);
    if (missTrend) missTrend.add(res.timings.duration);
  }
}

// ---- Weighted random selection ----------------------------------------------
export function weightedPick(items) {
  const total = items.reduce((s, it) => s + (it.weight || 1), 0);
  let r = Math.random() * total;
  for (const it of items) {
    r -= it.weight || 1;
    if (r <= 0) return it;
  }
  return items[items.length - 1];
}

// ---- Geographic helpers ------------------------------------------------------
// CONUS-ish bbox for realistic request areas
export const CONUS = { minLon: -125, minLat: 24, maxLon: -66, maxLat: 50 };

export function randLon() {
  return CONUS.minLon + Math.random() * (CONUS.maxLon - CONUS.minLon);
}
export function randLat() {
  return CONUS.minLat + Math.random() * (CONUS.maxLat - CONUS.minLat);
}

// Random WebMercator tile coords at zoom z, constrained to CONUS
export function randTile(z) {
  const n = Math.pow(2, z);
  const lon = randLon();
  const lat = randLat();
  const x = Math.floor(((lon + 180) / 360) * n);
  const latRad = (lat * Math.PI) / 180;
  const y = Math.floor(
    ((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n
  );
  return { z: z, x: x, y: Math.max(0, Math.min(n - 1, y)) };
}

// Zoom distribution: mostly regional zooms like real map users
export function randZoom() {
  const r = Math.random();
  if (r < 0.2) return 4 + Math.floor(Math.random() * 2); // 4-5 overview
  if (r < 0.75) return 6 + Math.floor(Math.random() * 3); // 6-8 regional
  return 9 + Math.floor(Math.random() * 3); // 9-11 local
}

// Random EPSG:3857 bbox of roughly deg x deg size within CONUS
export function randBbox3857(degSpan) {
  const lon = CONUS.minLon + Math.random() * (CONUS.maxLon - CONUS.minLon - degSpan);
  const lat = CONUS.minLat + Math.random() * (CONUS.maxLat - CONUS.minLat - degSpan);
  return [
    merc(lon),
    mercLat(lat),
    merc(lon + degSpan),
    mercLat(lat + degSpan),
  ].join(",");
}

function merc(lon) {
  return (lon * 20037508.34) / 180;
}
function mercLat(lat) {
  const y = Math.log(Math.tan(((90 + lat) * Math.PI) / 360)) / (Math.PI / 180);
  return (y * 20037508.34) / 180;
}

// ---- Summary helper -----------------------------------------------------------
// Standard handleSummary: write full JSON to SUMMARY_PATH (mounted volume).
export function summaryHandler(extra) {
  return function (data) {
    const out = {};
    const path = __ENV.SUMMARY_PATH;
    data.loadtest_meta = Object.assign(
      {
        scenario: __ENV.SCENARIO_NAME || "unnamed",
        target: __ENV.TARGET || "direct",
        finished_at: new Date().toISOString(),
      },
      extra || {}
    );
    if (path) {
      out[path] = JSON.stringify(data, null, 2);
    }
    out.stdout = textSummaryLine(data);
    return out;
  };
}

function textSummaryLine(data) {
  const reqs = data.metrics.http_reqs ? data.metrics.http_reqs.values.count : 0;
  const fail = data.metrics.http_req_failed
    ? (data.metrics.http_req_failed.values.rate * 100).toFixed(2)
    : "?";
  const p95 = data.metrics.http_req_duration
    ? data.metrics.http_req_duration.values["p(95)"].toFixed(1)
    : "?";
  return `\n[${__ENV.SCENARIO_NAME || "run"}] requests=${reqs} failed=${fail}% p95=${p95}ms\n`;
}
