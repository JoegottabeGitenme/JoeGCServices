// ============================================================================
// Daytime probe — short mixed-traffic health/trend check through the gateway
//
// Runs sporadically throughout the day (systemd timer with jitter). Rates are
// deliberately light and stay far below the gateway's 10000r/m limit.
// Thresholds make the run pass/fail so the report can flag degradation.
//
// Env: TARGET (default gateway), RATE (default 15), DURATION (default 3m)
// ============================================================================
import { layerPool, fetchLayerTimes, wmtsRequest, wmsGetMapRequest } from "../lib/wms.js";
import { discoverEdr, edrGridRequest, edrEventRequest } from "../lib/edr.js";
import { summaryHandler } from "../lib/common.js";

const RATE = parseFloat(__ENV.RATE || "15");
const DURATION = __ENV.DURATION || "3m";

export const options = {
  scenarios: {
    wmts: {
      executor: "constant-arrival-rate",
      exec: "runWmts",
      rate: Math.max(1, Math.round(RATE * 0.6)),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 10,
      maxVUs: 60,
    },
    wms: {
      executor: "constant-arrival-rate",
      exec: "runWms",
      rate: Math.max(1, Math.round(RATE * 0.15)),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 5,
      maxVUs: 30,
    },
    edr: {
      executor: "constant-arrival-rate",
      exec: "runEdr",
      rate: Math.max(1, Math.round(RATE * 0.25)),
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 5,
      maxVUs: 30,
    },
  },
  thresholds: {
    // Probe SLOs — tune as baselines accumulate
    http_req_failed: ["rate<0.02"],
    wmts_tile_ms: ["p(95)<2000"],
    wms_getmap_ms: ["p(95)<6000"],
    edr_position_ms: ["p(95)<3000"],
  },
};

export function setup() {
  return {
    pool: layerPool(),
    times: fetchLayerTimes("mrms_REFL", 5),
    edr: discoverEdr(),
  };
}

export function runWmts(data) {
  wmtsRequest(data.pool, data.times);
}
export function runWms(data) {
  wmsGetMapRequest(data.pool);
}
export function runEdr(data) {
  // 80/20 grid vs storm-event queries
  if (Math.random() < 0.8) {
    edrGridRequest(data.edr);
  } else {
    edrEventRequest(data.edr);
  }
}

export const handleSummary = summaryHandler({
  kind: "probe",
  rate: RATE,
  duration: DURATION,
});
