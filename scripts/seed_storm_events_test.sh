#!/bin/bash
# Seed the storm-events pipeline with test data — no internet download required.
#
# POSTs a small batch of realistic events (hail/wind/tornado) directly to the
# ingester's /ingest/storm-events endpoint, then triggers a county-aggregate
# refresh. Lets you exercise the full ingester→PostGIS→EDR flow in ~10 seconds
# without touching the downloader at all.
#
# Prerequisites:
#   - postgres running and storm-events schema migrated (ingester startup does this)
#   - TIGER counties loaded (scripts/load_tiger_counties.sh — or events will have
#     null county_fips which is fine for basic endpoint testing)
#   - ingester running (default http://localhost:8082)
#
# Usage:
#   ./scripts/seed_storm_events_test.sh                    # use defaults
#   INGESTER_URL=http://localhost:8082 ./scripts/seed_storm_events_test.sh

set -euo pipefail

INGESTER_URL="${INGESTER_URL:-http://localhost:8082}"

echo "==> Seeding storm events test data → ${INGESTER_URL}"
echo "    (25 events: 10 hail, 8 wind, 7 tornado)"

# ---------------------------------------------------------------------------
# POST the test event batch
# ---------------------------------------------------------------------------
curl -sf -o /dev/null -w "    /ingest/storm-events: HTTP %{http_code}\n" \
  -X POST "${INGESTER_URL}/ingest/storm-events" \
  -H "Content-Type: application/json" \
  -d '{
  "events": [
    {
      "event_id": 900001, "event_type": "hail",
      "begin_time": "2023-04-28T18:30:00Z",
      "begin_lat": 35.467, "begin_lon": -97.517,
      "magnitude": 1.75, "magnitude_unit": "in",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900002, "event_type": "hail",
      "begin_time": "2022-06-12T21:00:00Z",
      "begin_lat": 35.221, "begin_lon": -97.444,
      "magnitude": 2.0, "magnitude_unit": "in",
      "state": "OKLAHOMA", "cz_name": "CLEVELAND"
    },
    {
      "event_id": 900003, "event_type": "hail",
      "begin_time": "2021-05-04T20:15:00Z",
      "begin_lat": 36.154, "begin_lon": -95.994,
      "magnitude": 3.0, "magnitude_unit": "in",
      "state": "OKLAHOMA", "cz_name": "TULSA"
    },
    {
      "event_id": 900004, "event_type": "hail",
      "begin_time": "2020-08-19T22:00:00Z",
      "begin_lat": 35.467, "begin_lon": -97.517,
      "magnitude": 1.0, "magnitude_unit": "in",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900005, "event_type": "hail",
      "begin_time": "2019-05-21T19:45:00Z",
      "begin_lat": 39.768, "begin_lon": -86.158,
      "magnitude": 1.5, "magnitude_unit": "in",
      "state": "INDIANA", "cz_name": "MARION"
    },
    {
      "event_id": 900006, "event_type": "hail",
      "begin_time": "2018-03-13T16:30:00Z",
      "begin_lat": 33.749, "begin_lon": -84.388,
      "magnitude": 2.5, "magnitude_unit": "in",
      "state": "GEORGIA", "cz_name": "FULTON"
    },
    {
      "event_id": 900007, "event_type": "hail",
      "begin_time": "2017-06-01T18:00:00Z",
      "begin_lat": 41.878, "begin_lon": -87.630,
      "magnitude": 0.75, "magnitude_unit": "in",
      "state": "ILLINOIS", "cz_name": "COOK"
    },
    {
      "event_id": 900008, "event_type": "hail",
      "begin_time": "2016-09-14T17:00:00Z",
      "begin_lat": 29.763, "begin_lon": -95.363,
      "magnitude": 2.0, "magnitude_unit": "in",
      "state": "TEXAS", "cz_name": "HARRIS"
    },
    {
      "event_id": 900009, "event_type": "hail",
      "begin_time": "2015-04-28T14:30:00Z",
      "begin_lat": 35.467, "begin_lon": -97.517,
      "magnitude": 4.0, "magnitude_unit": "in",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900010, "event_type": "hail",
      "begin_time": "2014-07-04T20:00:00Z",
      "begin_lat": 44.978, "begin_lon": -93.265,
      "magnitude": 1.25, "magnitude_unit": "in",
      "state": "MINNESOTA", "cz_name": "HENNEPIN"
    },
    {
      "event_id": 900011, "event_type": "wind",
      "begin_time": "2023-07-16T23:45:00Z",
      "begin_lat": 35.467, "begin_lon": -97.517,
      "magnitude": 65.0, "magnitude_unit": "kt",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900012, "event_type": "wind",
      "begin_time": "2022-08-10T02:00:00Z",
      "begin_lat": 41.878, "begin_lon": -87.630,
      "magnitude": 80.0, "magnitude_unit": "kt",
      "state": "ILLINOIS", "cz_name": "COOK"
    },
    {
      "event_id": 900013, "event_type": "wind",
      "begin_time": "2021-06-25T19:00:00Z",
      "begin_lat": 39.768, "begin_lon": -86.158,
      "magnitude": 55.0, "magnitude_unit": "kt",
      "state": "INDIANA", "cz_name": "MARION"
    },
    {
      "event_id": 900014, "event_type": "wind",
      "begin_time": "2020-08-10T21:00:00Z",
      "begin_lat": 42.358, "begin_lon": -71.060,
      "magnitude": 70.0, "magnitude_unit": "kt",
      "state": "MASSACHUSETTS", "cz_name": "SUFFOLK"
    },
    {
      "event_id": 900015, "event_type": "wind",
      "begin_time": "2019-09-11T14:00:00Z",
      "begin_lat": 29.763, "begin_lon": -95.363,
      "magnitude": 60.0, "magnitude_unit": "kt",
      "state": "TEXAS", "cz_name": "HARRIS"
    },
    {
      "event_id": 900016, "event_type": "wind",
      "begin_time": "2018-04-03T22:30:00Z",
      "begin_lat": 33.749, "begin_lon": -84.388,
      "magnitude": 50.0, "magnitude_unit": "kt",
      "state": "GEORGIA", "cz_name": "FULTON"
    },
    {
      "event_id": 900017, "event_type": "wind",
      "begin_time": "2017-05-27T18:45:00Z",
      "begin_lat": 44.978, "begin_lon": -93.265,
      "magnitude": 65.0, "magnitude_unit": "kt",
      "state": "MINNESOTA", "cz_name": "HENNEPIN"
    },
    {
      "event_id": 900018, "event_type": "wind",
      "begin_time": "2016-07-22T20:00:00Z",
      "begin_lat": 36.154, "begin_lon": -95.994,
      "magnitude": 75.0, "magnitude_unit": "kt",
      "state": "OKLAHOMA", "cz_name": "TULSA"
    },
    {
      "event_id": 900019, "event_type": "tornado",
      "begin_time": "2023-04-19T22:00:00Z",
      "begin_lat": 35.310, "begin_lon": -97.823,
      "end_lat": 35.510, "end_lon": -97.520,
      "tor_f_scale": 3, "magnitude": 3.0, "magnitude_unit": "EF",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900020, "event_type": "tornado",
      "begin_time": "2022-03-30T20:15:00Z",
      "begin_lat": 33.210, "begin_lon": -97.130,
      "end_lat": 33.320, "end_lon": -96.980,
      "tor_f_scale": 1, "magnitude": 1.0, "magnitude_unit": "EF",
      "state": "TEXAS", "cz_name": "DENTON"
    },
    {
      "event_id": 900021, "event_type": "tornado",
      "begin_time": "2021-12-10T21:00:00Z",
      "begin_lat": 36.720, "begin_lon": -89.120,
      "end_lat": 37.010, "end_lon": -88.780,
      "tor_f_scale": 4, "magnitude": 4.0, "magnitude_unit": "EF",
      "state": "KENTUCKY", "cz_name": "GRAVES"
    },
    {
      "event_id": 900022, "event_type": "tornado",
      "begin_time": "2020-04-12T22:30:00Z",
      "begin_lat": 35.150, "begin_lon": -90.050,
      "end_lat": 35.230, "end_lon": -89.920,
      "tor_f_scale": 2, "magnitude": 2.0, "magnitude_unit": "EF",
      "state": "TENNESSEE", "cz_name": "SHELBY"
    },
    {
      "event_id": 900023, "event_type": "tornado",
      "begin_time": "2019-05-20T21:45:00Z",
      "begin_lat": 35.410, "begin_lon": -97.700,
      "end_lat": 35.550, "end_lon": -97.430,
      "tor_f_scale": 2, "magnitude": 2.0, "magnitude_unit": "EF",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900024, "event_type": "tornado",
      "begin_time": "2018-11-30T16:30:00Z",
      "begin_lat": 35.467, "begin_lon": -97.517,
      "tor_f_scale": 0, "magnitude": 0.0, "magnitude_unit": "EF",
      "state": "OKLAHOMA", "cz_name": "OKLAHOMA"
    },
    {
      "event_id": 900025, "event_type": "tornado",
      "begin_time": "2013-05-20T19:56:00Z",
      "begin_lat": 35.262, "begin_lon": -97.676,
      "end_lat": 35.449, "end_lon": -97.351,
      "tor_f_scale": 5, "magnitude": 5.0, "magnitude_unit": "EF",
      "state": "OKLAHOMA", "cz_name": "CLEVELAND"
    }
  ]
}'

# ---------------------------------------------------------------------------
# Trigger county aggregate refresh
# ---------------------------------------------------------------------------
echo ""
echo "==> Refreshing county aggregate..."
HTTP_CODE=$(curl -sf -o /tmp/se_refresh.json -w "%{http_code}" \
  -X POST "${INGESTER_URL}/ingest/storm-events/refresh-counties" 2>&1) || true
echo "    /ingest/storm-events/refresh-counties: HTTP ${HTTP_CODE}"
cat /tmp/se_refresh.json 2>/dev/null && echo ""

# ---------------------------------------------------------------------------
# Quick sanity query via ingester health
# ---------------------------------------------------------------------------
echo ""
echo "==> Ingester health check..."
curl -sf "${INGESTER_URL}/health" | python3 -m json.tool 2>/dev/null || echo "    (non-JSON response)"

echo ""
echo "✓ Done. Test data seeded."
echo ""
echo "Now hit the EDR API (default http://localhost:8083):"
echo ""
echo "  # List collections (should show hail/wind/tornado once data is present):"
echo "  curl 'http://localhost:8083/edr/collections' | python3 -m json.tool | grep -E '\"id\"|\"title\"'"
echo ""
echo "  # Hail events within 50 km of Oklahoma City:"
echo "  curl 'http://localhost:8083/edr/collections/hail/radius?coords=POINT(-97.5+35.5)&within=50km&datetime=2015/2024' | python3 -m json.tool"
echo ""
echo "  # Tornado tracks intersecting an OKC-area bbox:"
echo "  curl 'http://localhost:8083/edr/collections/tornado/items?bbox=-98.5,34.8,-96.5,36.2' | python3 -m json.tool"
echo ""
echo "  # County aggregate — hail in Oklahoma 2015-2023:"
echo "  curl 'http://localhost:8083/edr/collections/hail/counties?state=OK&datetime=2015/2023&geometry=false' | python3 -m json.tool"
echo ""
echo "  # Wind events as area query:"
echo "  curl 'http://localhost:8083/edr/collections/wind/area?coords=-100,34,-94,37&datetime=2019/2024' | python3 -m json.tool"
