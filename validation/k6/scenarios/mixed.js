// ============================================================================
// Mixed realistic traffic — WMTS tiles + WMS GetMap + EDR queries
//
// Used for: cold-cache stage, warm steady-state, soak, gateway-overhead stage.
//
// Env:
//   RATE       total requests/sec across all workloads (default 20)
//   DURATION   e.g. "5m" (default)
//   TARGET     direct|gateway
//   TIME_VARIETY  fraction of WMTS requests using explicit TIME (default 0.2)
//   SCENARIO_NAME, SUMMARY_PATH  (set by orchestration scripts)
// ============================================================================
import { layerPool, fetchLayerTimes, wmtsRequest, wmsGetMapRequest } from "../lib/wms.js";
import { discoverEdr, edrGridRequest, edrEventRequest } from "../lib/edr.js";
import { summaryHandler } from "../lib/common.js";

const RATE = parseFloat(__ENV.RATE || "20");
const DURATION = __ENV.DURATION || "5m";

// Traffic mix: tiles dominate (map panning), then EDR point data, then GetMap
const MIX = { wmts: 0.6, wms: 0.15, edr_grid: 0.2, edr_events: 0.05 };

function rateFor(share) {
  return Math.max(1, Math.round(RATE * share));
}

export const options = {
  discardResponseBodies: false,
  scenarios: {
    wmts: {
      executor: "constant-arrival-rate",
      exec: "runWmts",
      rate: rateFor(MIX.wmts),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 20,
      maxVUs: 200,
    },
    wms: {
      executor: "constant-arrival-rate",
      exec: "runWms",
      rate: rateFor(MIX.wms),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 10,
      maxVUs: 100,
    },
    edr_grid: {
      executor: "constant-arrival-rate",
      exec: "runEdrGrid",
      rate: rateFor(MIX.edr_grid),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 10,
      maxVUs: 100,
    },
    edr_events: {
      executor: "constant-arrival-rate",
      exec: "runEdrEvents",
      rate: rateFor(MIX.edr_events),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 5,
      maxVUs: 50,
    },
  },
};

export function setup() {
  const pool = layerPool();
  // Grab recent MRMS times to give WMTS temporal variety (best-effort)
  const times = fetchLayerTimes("mrms_REFL", 10);
  const edr = discoverEdr();
  return { pool: pool, times: times, edr: edr };
}

export function runWmts(data) {
  wmtsRequest(data.pool, data.times);
}
export function runWms(data) {
  wmsGetMapRequest(data.pool);
}
export function runEdrGrid(data) {
  edrGridRequest(data.edr);
}
export function runEdrEvents(data) {
  edrEventRequest(data.edr);
}

export const handleSummary = summaryHandler({
  kind: "mixed",
  rate: RATE,
  duration: DURATION,
});
