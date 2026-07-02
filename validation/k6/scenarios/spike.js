// ============================================================================
// Spike test — instant burst to SPIKE_RATE, hold, drop. Measures degradation
// under sudden load and recovery afterwards (the report inspects the tail).
//
// Env: SPIKE_RATE (default 200), HOLD (default 60s), TARGET
// ============================================================================
import { layerPool, fetchLayerTimes, wmtsRequest, wmsGetMapRequest } from "../lib/wms.js";
import { discoverEdr, edrGridRequest } from "../lib/edr.js";
import { summaryHandler } from "../lib/common.js";

const SPIKE_RATE = parseInt(__ENV.SPIKE_RATE || "200", 10);
const HOLD = __ENV.HOLD || "60s";

export const options = {
  scenarios: {
    spike: {
      executor: "ramping-arrival-rate",
      exec: "run",
      startRate: 5,
      timeUnit: "1s",
      stages: [
        { target: SPIKE_RATE, duration: "10s" }, // near-instant ramp
        { target: SPIKE_RATE, duration: HOLD }, // hold at spike
        { target: 5, duration: "30s" }, // drop + recovery window
        { target: 5, duration: "60s" }, // observe recovery
      ],
      preAllocatedVUs: 100,
      maxVUs: 400,
    },
  },
};

export function setup() {
  return {
    pool: layerPool(),
    times: fetchLayerTimes("mrms_REFL", 5),
    edr: discoverEdr(),
  };
}

export function run(data) {
  const r = Math.random();
  if (r < 0.65) {
    wmtsRequest(data.pool, data.times);
  } else if (r < 0.8) {
    wmsGetMapRequest(data.pool);
  } else {
    edrGridRequest(data.edr);
  }
}

export const handleSummary = summaryHandler({
  kind: "spike",
  spike_rate: SPIKE_RATE,
  hold: HOLD,
});
