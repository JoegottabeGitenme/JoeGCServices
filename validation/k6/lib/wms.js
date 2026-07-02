// ============================================================================
// WMS GetMap + WMTS tile request builders
// ============================================================================
import http from "k6/http";
import { check } from "k6";
import {
  target,
  weightedPick,
  randTile,
  randZoom,
  randBbox3857,
  wmtsTile,
  wmtsL1,
  wmtsL2,
  wmtsMiss,
  wmsMap,
  wmsMiss,
  recordCacheTier,
} from "./common.js";

// Known-good layer:style pairs (kept in sync with validation/load-test
// scenarios + config/models). Omitting TIME/RUN returns the latest frame,
// which is both valid and what real users predominantly request.
// Override with -e LAYERS='layer:style:weight,layer:style:weight'.
// Styles verified against live GetCapabilities (per-layer style ids differ:
// model layers use gradient/isolines, MRMS uses standard/enhanced; "default"
// is always accepted and resolves to the layer's first style).
const DEFAULT_LAYERS = [
  { layer: "gfs_TMP", style: "gradient", weight: 3 },
  { layer: "gfs_WIND_BARBS", style: "default", weight: 1 },
  { layer: "gfs_PRMSL", style: "isolines", weight: 1 },
  { layer: "hrrr_TMP", style: "gradient", weight: 2 },
  { layer: "mrms_REFL", style: "enhanced", weight: 3 },
  { layer: "mrms_PRECIP_RATE", style: "default", weight: 1 },
  { layer: "goes19_CMI_C13", style: "default", weight: 2 },
  { layer: "goes18_CMI_C02", style: "default", weight: 1 },
];

export function layerPool() {
  if (__ENV.LAYERS) {
    return __ENV.LAYERS.split(",").map((s) => {
      const parts = s.split(":");
      return {
        layer: parts[0],
        style: parts[1] || "default",
        weight: parseFloat(parts[2] || "1"),
      };
    });
  }
  return DEFAULT_LAYERS;
}

// Fetch available times for a layer from WMS GetCapabilities (regex parse).
// Returns [] on any failure — callers fall back to latest-only requests.
export function fetchLayerTimes(layerName, maxTimes) {
  try {
    const res = http.get(
      `${target().wms}/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetCapabilities`,
      { timeout: "30s", tags: { name: "wms_capabilities" } }
    );
    if (res.status !== 200) return [];
    const body = res.body;
    // Locate the layer block, then its time dimension values
    const nameIdx = body.indexOf(`<Name>${layerName}</Name>`);
    if (nameIdx < 0) return [];
    const tail = body.substring(nameIdx, nameIdx + 20000);
    const dimMatch = tail.match(
      /<Dimension[^>]*name="time"[^>]*>([^<]+)<\/Dimension>/
    );
    if (!dimMatch) return [];
    const times = dimMatch[1]
      .split(",")
      .map((t) => t.trim())
      .filter((t) => t.length > 0 && !t.includes("/")); // skip interval syntax
    return times.slice(-(maxTimes || 10)); // most recent N
  } catch (e) {
    return [];
  }
}

// ---- WMTS tile request (RESTful path) ---------------------------------------
// times: optional array of ISO times; TIME_VARIETY fraction of requests
// will use a random one (exercises cache-miss/temporal paths).
export function wmtsRequest(pool, times) {
  const pick = weightedPick(pool);
  const t = randTile(randZoom());
  // WMTS registers style ids separately from WMS style names — only
  // "default" is universally valid (WMS GetMap uses the richer names).
  const style = __ENV.WMTS_STYLE || "default";
  let url = `${target().wms}/wmts/rest/${pick.layer}/${style}/WebMercatorQuad/${t.z}/${t.x}/${t.y}.png`;

  const variety = parseFloat(__ENV.TIME_VARIETY || "0");
  if (times && times.length > 0 && Math.random() < variety) {
    const time = times[Math.floor(Math.random() * times.length)];
    url += `?TIME=${encodeURIComponent(time)}`;
  }

  const res = http.get(url, {
    timeout: __ENV.REQ_TIMEOUT || "30s",
    tags: { name: "wmts_tile", layer: pick.layer },
  });

  check(res, {
    "wmts status 200": (r) => r.status === 200,
    "wmts is png": (r) =>
      (r.headers["Content-Type"] || "").includes("image"),
  });
  wmtsTile.add(res.timings.duration);
  recordCacheTier(res, wmtsL2, wmtsMiss, wmtsL1);
  return res;
}

// ---- WMS GetMap request --------------------------------------------------------
// Random bbox spans + viewport sizes; unlike tiles these rarely repeat
// exactly, so they exercise the render path much harder.
const SPANS = [4, 8, 15, 30]; // degrees
const SIZES = [
  [256, 256],
  [512, 512],
  [768, 512],
  [1024, 768],
];

export function wmsGetMapRequest(pool) {
  const pick = weightedPick(pool);
  const span = SPANS[Math.floor(Math.random() * SPANS.length)];
  const size = SIZES[Math.floor(Math.random() * SIZES.length)];
  const bbox = randBbox3857(span);

  const url =
    `${target().wms}/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap` +
    `&LAYERS=${pick.layer}&STYLES=${pick.style}&CRS=EPSG:3857` +
    `&BBOX=${bbox}&WIDTH=${size[0]}&HEIGHT=${size[1]}&FORMAT=image/png&TRANSPARENT=true`;

  const res = http.get(url, {
    timeout: __ENV.REQ_TIMEOUT || "60s",
    tags: { name: "wms_getmap", layer: pick.layer },
  });

  check(res, {
    "getmap status 200": (r) => r.status === 200,
    "getmap is png": (r) =>
      (r.headers["Content-Type"] || "").includes("image"),
  });
  wmsMap.add(res.timings.duration);
  recordCacheTier(res, null, wmsMiss, null);
  return res;
}
