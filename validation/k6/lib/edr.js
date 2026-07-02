// ============================================================================
// EDR request builders — runtime collection discovery + weighted query mixes
// ============================================================================
import http from "k6/http";
import { check } from "k6";
import {
  target,
  weightedPick,
  randLon,
  randLat,
  edrPosition,
  edrRadius,
  edrArea,
  edrCube,
  edrItems,
} from "./common.js";

// Gridded collections queried with position/radius/area/cube.
// Storm-event feature collections queried with radius/items.
// Override with -e EDR_COLLECTIONS=..., -e EDR_EVENT_COLLECTIONS=...
const DEFAULT_GRID_COLLECTIONS = [
  "gfs-surface",
  "gfs-isobaric",
  "hrrr-surface",
  "mrms-single-level",
];


const DEFAULT_EVENT_COLLECTIONS = ["hail", "wind", "tornado"];

// Query-type mix for gridded collections (weights ~ real client patterns:
// point queries dominate, cubes are rare and expensive)
const GRID_QUERY_MIX = [
  { type: "position", weight: 6 },
  { type: "radius", weight: 2 },
  { type: "area", weight: 2 },
  { type: "cube", weight: 1 },
];
const EVENT_QUERY_MIX = [
  { type: "radius", weight: 3 },
  { type: "items", weight: 2 },
];

// ---- Discovery (call from setup()) -----------------------------------------
// Fetches parameter names + temporal extent per collection so every request
// generated is currently valid.
export function discoverEdr() {
  const base = target().edr;
  const gridIds = (__ENV.EDR_COLLECTIONS || DEFAULT_GRID_COLLECTIONS.join(","))
    .split(",")
    .map((s) => s.trim());
  const eventIds = (
    __ENV.EDR_EVENT_COLLECTIONS || DEFAULT_EVENT_COLLECTIONS.join(",")
  )
    .split(",")
    .map((s) => s.trim());

  const collections = { grid: [], event: [] };

  for (const id of gridIds) {
    const res = http.get(`${base}/edr/collections/${id}`, { timeout: "30s" });
    if (res.status !== 200) {
      console.warn(`discovery: collection ${id} -> ${res.status}, skipping`);
      continue;
    }
    try {
      const doc = JSON.parse(res.body);
      const params = Object.keys(doc.parameter_names || {}).slice(0, 6);
      // Temporal extents expose DISCRETE valid instants in .values —
      // random instants inside the interval land in gaps and 400.
      let times = [];
      if (doc.extent && doc.extent.temporal) {
        times = (doc.extent.temporal.values || []).slice(-24); // recent N
      }
      // Which query types does this collection actually support?
      // (e.g. cube requires numeric vertical levels — surface collections 400)
      const supports = Object.keys(doc.data_queries || {});
      // Valid vertical levels (like datetime, must come from the extent —
      // hardcoded pressure levels 400 on "outside vertical extent")
      let levels = [];
      if (doc.extent && doc.extent.vertical) {
        levels = (doc.extent.vertical.values || []).map(Number).filter(isFinite);
      }
      if (params.length > 0) {
        collections.grid.push({
          id: id,
          params: params,
          times: times,
          supports: supports,
          levels: levels,
        });
      }
    } catch (e) {
      console.warn(`discovery: collection ${id} parse failed`);
    }
  }

  for (const id of eventIds) {
    const res = http.get(`${base}/edr/collections/${id}`, { timeout: "30s" });
    if (res.status === 200) {
      collections.event.push({ id: id });
    } else {
      console.warn(`discovery: event collection ${id} -> ${res.status}, skipping`);
    }
  }

  if (collections.grid.length === 0 && collections.event.length === 0) {
    throw new Error("EDR discovery found no usable collections");
  }
  return collections;
}

// Random datetime from the collection's discrete valid instants,
// biased toward recent ones (like real users). Returns null (omit
// datetime -> server default/latest) when no values are available.
function randDatetime(times) {
  if (!times || times.length === 0) return null;
  // Bias: 70% of picks from the most recent third
  const recentStart = Math.floor(times.length * 0.67);
  const idx =
    Math.random() < 0.7
      ? recentStart + Math.floor(Math.random() * (times.length - recentStart))
      : Math.floor(Math.random() * times.length);
  return times[idx];
}

// Random closed year range for storm-event queries (server requires closed
// ranges; all-time scans time out — see docs/storm-events-api.md)
function randYearRange() {
  const y = 1996 + Math.floor(Math.random() * 28);
  const span = 1 + Math.floor(Math.random() * 3);
  return `${y}-01-01T00:00:00Z/${Math.min(y + span, 2026)}-01-01T00:00:00Z`;
}

// ---- Request generators -------------------------------------------------------
export function edrGridRequest(collections) {
  if (collections.grid.length === 0) return null;
  const col =
    collections.grid[Math.floor(Math.random() * collections.grid.length)];
  // Only pick query types the collection supports (per its data_queries)
  const mix =
    col.supports && col.supports.length > 0
      ? GRID_QUERY_MIX.filter((q) => col.supports.includes(q.type))
      : GRID_QUERY_MIX.filter((q) => q.type !== "cube");
  if (mix.length === 0) return null;
  const qtype = weightedPick(mix).type;
  const base = target().edr;
  // Collections with a vertical extent need a valid discovered level
  const zParam =
    col.levels && col.levels.length > 0
      ? `&z=${col.levels[Math.floor(Math.random() * col.levels.length)]}`
      : "";
  const params = col.params
    .slice(0, 1 + Math.floor(Math.random() * 2))
    .join(",");
  const dt = randDatetime(col.times);
  const dtParam = dt ? `&datetime=${encodeURIComponent(dt)}` : "";
  const lon = randLon().toFixed(3);
  const lat = randLat().toFixed(3);

  // NOTE: coords MUST be percent-encoded — k6's HTTP client sends raw
  // spaces as-is and axum rejects the URI with an empty 400.
  const point = encodeURIComponent(`POINT(${lon} ${lat})`);

  let url, trend;
  switch (qtype) {
    case "position":
      url =
        `${base}/edr/collections/${col.id}/position?coords=${point}` +
        `&parameter-name=${params}${dtParam}${zParam}`;
      trend = edrPosition;
      break;
    case "radius":
      url =
        `${base}/edr/collections/${col.id}/radius?coords=${point}` +
        `&within=50&within-units=km&parameter-name=${params}${dtParam}${zParam}`;
      trend = edrRadius;
      break;
    case "area": {
      const w = (2 + Math.random() * 4).toFixed(2);
      const x0 = parseFloat(lon);
      const y0 = parseFloat(lat);
      const poly = `POLYGON((${x0} ${y0},${x0 + parseFloat(w)} ${y0},${
        x0 + parseFloat(w)
      } ${y0 + parseFloat(w)},${x0} ${y0 + parseFloat(w)},${x0} ${y0}))`;
      url =
        `${base}/edr/collections/${col.id}/area?coords=${encodeURIComponent(poly)}` +
        `&parameter-name=${params}${dtParam}${zParam}`;
      trend = edrArea;
      break;
    }
    case "cube": {
      const w = 3;
      const x0 = parseFloat(lon);
      const y0 = parseFloat(lat);
      url =
        `${base}/edr/collections/${col.id}/cube?bbox=${x0},${y0},${x0 + w},${y0 + w}` +
        `&parameter-name=${params.split(",")[0]}${dtParam}${zParam}`;
      trend = edrCube;
      break;
    }
  }

  const res = http.get(url, {
    timeout: __ENV.REQ_TIMEOUT || "60s",
    tags: { name: `edr_${qtype}`, collection: col.id },
  });

  // Cubes may legitimately 4xx on response-size limits; count 2xx/4xx as
  // non-failures, 5xx and timeouts as failures.
  check(res, {
    "edr status ok": (r) =>
      qtype === "cube" ? r.status < 500 && r.status !== 0 : r.status === 200,
  });
  trend.add(res.timings.duration);
  return res;
}

export function edrEventRequest(collections) {
  if (collections.event.length === 0) return null;
  const col =
    collections.event[Math.floor(Math.random() * collections.event.length)];
  const qtype = weightedPick(EVENT_QUERY_MIX).type;
  const base = target().edr;

  let url, trend;
  if (qtype === "radius") {
    const point = encodeURIComponent(
      `POINT(${randLon().toFixed(3)} ${randLat().toFixed(3)})`
    );
    url =
      `${base}/edr/collections/${col.id}/radius?coords=${point}` +
      `&within=75&within-units=km&datetime=${encodeURIComponent(randYearRange())}&f=GeoJSON`;
    trend = edrRadius;
  } else {
    const x0 = randLon();
    const y0 = randLat();
    url =
      `${base}/edr/collections/${col.id}/items?bbox=${x0.toFixed(2)},${y0.toFixed(2)},${(x0 + 4).toFixed(2)},${(y0 + 4).toFixed(2)}` +
      `&datetime=${encodeURIComponent(randYearRange())}&limit=500`;
    trend = edrItems;
  }

  const res = http.get(url, {
    timeout: __ENV.REQ_TIMEOUT || "60s",
    tags: { name: `edr_events_${qtype}`, collection: col.id },
  });
  check(res, { "edr events status 200": (r) => r.status === 200 });
  trend.add(res.timings.duration);
  return res;
}
