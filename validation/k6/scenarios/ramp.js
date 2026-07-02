// ============================================================================
// Breakpoint ramp — find max sustainable RPS for one workload class
//
// Ramps arrival rate from START_RATE to MAX_RATE over DURATION. Aborts when
// error rate or p95 breaches limits — the abort time lets the report
// interpolate the achieved breakpoint RPS. dropped_iterations also signals
// saturation (system can't keep up with the arrival schedule).
//
// Env:
//   WORKLOAD    wmts | wms | edr        (required)
//   START_RATE  default 10
//   MAX_RATE    default: wmts=800, wms=200, edr=150
//   DURATION    ramp length, default 10m
//   P95_LIMIT   ms, default: wmts=3000, wms=10000, edr=5000
//   FAIL_LIMIT  error rate, default 0.05
// ============================================================================
import { layerPool, fetchLayerTimes, wmtsRequest, wmsGetMapRequest } from "../lib/wms.js";
import { discoverEdr, edrGridRequest, edrEventRequest } from "../lib/edr.js";
import { summaryHandler } from "../lib/common.js";

const WORKLOAD = __ENV.WORKLOAD;
if (!["wmts", "wms", "edr"].includes(WORKLOAD)) {
  throw new Error("WORKLOAD must be wmts|wms|edr");
}

const DEFAULTS = {
  wmts: { max: 800, p95: 3000, vus: 400 },
  wms: { max: 200, p95: 10000, vus: 200 },
  edr: { max: 150, p95: 5000, vus: 150 },
};

const START_RATE = parseInt(__ENV.START_RATE || "10", 10);
const MAX_RATE = parseInt(__ENV.MAX_RATE || String(DEFAULTS[WORKLOAD].max), 10);
const DURATION = __ENV.DURATION || "10m";
const P95_LIMIT = parseInt(__ENV.P95_LIMIT || String(DEFAULTS[WORKLOAD].p95), 10);
const FAIL_LIMIT = parseFloat(__ENV.FAIL_LIMIT || "0.05");

export const options = {
  scenarios: {
    ramp: {
      executor: "ramping-arrival-rate",
      exec: "run",
      startRate: START_RATE,
      timeUnit: "1s",
      stages: [{ target: MAX_RATE, duration: DURATION }],
      preAllocatedVUs: 50,
      maxVUs: DEFAULTS[WORKLOAD].vus,
    },
  },
  thresholds: {
    http_req_failed: [
      { threshold: `rate<${FAIL_LIMIT}`, abortOnFail: true, delayAbortEval: "30s" },
    ],
    http_req_duration: [
      { threshold: `p(95)<${P95_LIMIT}`, abortOnFail: true, delayAbortEval: "30s" },
    ],
  },
};

export function setup() {
  const data = { pool: layerPool(), times: [], edr: null };
  if (WORKLOAD === "wmts") {
    data.times = fetchLayerTimes("mrms_REFL", 10);
  }
  if (WORKLOAD === "edr") {
    data.edr = discoverEdr();
  }
  return data;
}

export function run(data) {
  if (WORKLOAD === "wmts") {
    wmtsRequest(data.pool, data.times);
  } else if (WORKLOAD === "wms") {
    wmsGetMapRequest(data.pool);
  } else {
    // 85/15 grid vs storm-event mix for EDR breakpoint
    if (Math.random() < 0.85) {
      edrGridRequest(data.edr);
    } else {
      edrEventRequest(data.edr);
    }
  }
}

// ramp config embedded so the report can interpolate breakpoint RPS from
// the actual test duration at abort time
export const handleSummary = summaryHandler({
  kind: "ramp",
  workload: WORKLOAD,
  start_rate: START_RATE,
  max_rate: MAX_RATE,
  ramp_duration: DURATION,
  p95_limit_ms: P95_LIMIT,
  fail_limit: FAIL_LIMIT,
});
