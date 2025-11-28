#!/bin/bash
# Update Grafana dashboard via API

GRAFANA_URL="http://localhost:3000"
GRAFANA_USER="admin"
GRAFANA_PASS="admin"
DASHBOARD_FILE="grafana-enhanced-dashboard.json"

echo "Updating Grafana dashboard..."

# Read the dashboard JSON
DASHBOARD_JSON=$(cat "$DASHBOARD_FILE")

# Wrap it in the required API format
API_PAYLOAD=$(cat <<PAYLOAD
{
  "dashboard": $DASHBOARD_JSON,
  "overwrite": true,
  "message": "Updated to enhanced dashboard with all Phase 4.1 metrics"
}
PAYLOAD
)

# Post to Grafana API
RESPONSE=$(curl -s -X POST \
  -H "Content-Type: application/json" \
  -u "$GRAFANA_USER:$GRAFANA_PASS" \
  -d "$API_PAYLOAD" \
  "$GRAFANA_URL/api/dashboards/db")

# Check result
if echo "$RESPONSE" | grep -q '"status":"success"'; then
  echo "✅ Dashboard updated successfully!"
  echo "   View at: $GRAFANA_URL/d/wms-perf/wms-performance"
else
  echo "❌ Failed to update dashboard"
  echo "Response: $RESPONSE"
  exit 1
fi
