#!/bin/bash
#
# EDR Product Dump Script
#
# Queries every parameter/level/time combination advertised by the EDR API
# and dumps the results to JSON files for manual verification.
#
# Usage:
#   ./edr-product-dump.sh [EDR_ENDPOINT] [OUTPUT_DIR]
#
# Defaults:
#   EDR_ENDPOINT: http://localhost:8083/edr
#   OUTPUT_DIR: ./edr-dump-$(date +%Y%m%d-%H%M%S)
#

set -e

EDR_ENDPOINT="${1:-http://localhost:8083/edr}"
OUTPUT_DIR="${2:-./edr-dump-$(date +%Y%m%d-%H%M%S)}"

echo "=============================================="
echo "EDR Product Dump"
echo "=============================================="
echo "Endpoint: $EDR_ENDPOINT"
echo "Output:   $OUTPUT_DIR"
echo ""

# Create output directories
mkdir -p "$OUTPUT_DIR/covjson"
mkdir -p "$OUTPUT_DIR/geojson"
mkdir -p "$OUTPUT_DIR/metadata"

# Summary file
SUMMARY_FILE="$OUTPUT_DIR/summary.txt"
echo "EDR Product Dump Summary" > "$SUMMARY_FILE"
echo "========================" >> "$SUMMARY_FILE"
echo "Endpoint: $EDR_ENDPOINT" >> "$SUMMARY_FILE"
echo "Timestamp: $(date -Iseconds)" >> "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"

# Results tracking
TOTAL=0
SUCCESS=0
FAILED=0
EMPTY=0

# Get collections
echo "Fetching collections..."
COLLECTIONS_FILE="$OUTPUT_DIR/metadata/collections.json"
if ! curl -s "$EDR_ENDPOINT/collections" > "$COLLECTIONS_FILE"; then
    echo "ERROR: Failed to fetch collections"
    exit 1
fi

# Parse collection IDs
COLLECTION_IDS=$(jq -r '.collections[].id' "$COLLECTIONS_FILE" 2>/dev/null)
if [ -z "$COLLECTION_IDS" ]; then
    echo "ERROR: No collections found"
    exit 1
fi

COLLECTION_COUNT=$(echo "$COLLECTION_IDS" | wc -l)
echo "Found $COLLECTION_COUNT collections"
echo "" >> "$SUMMARY_FILE"
echo "Collections: $COLLECTION_COUNT" >> "$SUMMARY_FILE"
echo "" >> "$SUMMARY_FILE"

# Process each collection
for COLLECTION_ID in $COLLECTION_IDS; do
    echo ""
    echo "Processing collection: $COLLECTION_ID"
    echo "----------------------------------------"
    
    # Get collection details
    COLL_FILE="$OUTPUT_DIR/metadata/${COLLECTION_ID}.json"
    curl -s "$EDR_ENDPOINT/collections/$COLLECTION_ID" > "$COLL_FILE"
    
    # Extract parameters
    PARAMS=$(jq -r '.parameter_names | keys[]' "$COLL_FILE" 2>/dev/null)
    if [ -z "$PARAMS" ]; then
        echo "  No parameters found, skipping"
        continue
    fi
    
    # Extract vertical levels (if any)
    LEVELS=$(jq -r '.extent.vertical.interval[]?[0] // empty' "$COLL_FILE" 2>/dev/null | sort -u)
    
    # Extract latest time value
    LATEST_TIME=$(jq -r '.extent.temporal.values[0] // empty' "$COLL_FILE" 2>/dev/null)
    
    echo "  Parameters: $(echo "$PARAMS" | wc -w)"
    echo "  Levels: $(echo "$LEVELS" | wc -w)"
    echo "  Latest time: ${LATEST_TIME:-none}"
    
    # Create collection output dir
    mkdir -p "$OUTPUT_DIR/covjson/$COLLECTION_ID"
    mkdir -p "$OUTPUT_DIR/geojson/$COLLECTION_ID"
    
    # Query each parameter
    for PARAM in $PARAMS; do
        # Determine levels to query
        if [ -n "$LEVELS" ]; then
            LEVEL_LIST="$LEVELS"
        else
            LEVEL_LIST="none"
        fi
        
        for LEVEL in $LEVEL_LIST; do
            TOTAL=$((TOTAL + 1))
            
            # Build query URL
            # Use a point in the center of CONUS for testing
            COORDS="POINT(-100 40)"
            
            if [ "$LEVEL" = "none" ]; then
                QUERY_PARAMS="coords=$(echo "$COORDS" | sed 's/ /%20/g')&parameter-name=$PARAM"
                FILENAME="${PARAM}"
            else
                QUERY_PARAMS="coords=$(echo "$COORDS" | sed 's/ /%20/g')&parameter-name=$PARAM&z=$LEVEL"
                FILENAME="${PARAM}_z${LEVEL}"
            fi
            
            # Add datetime if available
            if [ -n "$LATEST_TIME" ]; then
                QUERY_PARAMS="$QUERY_PARAMS&datetime=$(echo "$LATEST_TIME" | sed 's/:/%3A/g')"
            fi
            
            # Query CoverageJSON
            COVJSON_URL="$EDR_ENDPOINT/collections/$COLLECTION_ID/position?$QUERY_PARAMS"
            COVJSON_FILE="$OUTPUT_DIR/covjson/$COLLECTION_ID/${FILENAME}.json"
            
            echo -n "  $PARAM"
            [ "$LEVEL" != "none" ] && echo -n " @ z=$LEVEL"
            echo -n "..."
            
            HTTP_CODE=$(curl -s -w "%{http_code}" -o "$COVJSON_FILE" "$COVJSON_URL")
            
            if [ "$HTTP_CODE" = "200" ]; then
                # Check if response has data
                VALUES=$(jq -r ".ranges.$PARAM.values | length" "$COVJSON_FILE" 2>/dev/null || echo "0")
                NON_NULL=$(jq -r "[.ranges.$PARAM.values[] | select(. != null)] | length" "$COVJSON_FILE" 2>/dev/null || echo "0")
                
                if [ "$NON_NULL" -gt 0 ]; then
                    echo " OK ($NON_NULL values)"
                    SUCCESS=$((SUCCESS + 1))
                else
                    echo " EMPTY (0 non-null values)"
                    EMPTY=$((EMPTY + 1))
                fi
            else
                echo " FAILED (HTTP $HTTP_CODE)"
                FAILED=$((FAILED + 1))
            fi
            
            # Also query GeoJSON format
            GEOJSON_URL="$EDR_ENDPOINT/collections/$COLLECTION_ID/position?$QUERY_PARAMS&f=geojson"
            GEOJSON_FILE="$OUTPUT_DIR/geojson/$COLLECTION_ID/${FILENAME}.json"
            curl -s -o "$GEOJSON_FILE" "$GEOJSON_URL"
        done
    done
    
    echo "" >> "$SUMMARY_FILE"
    echo "Collection: $COLLECTION_ID" >> "$SUMMARY_FILE"
    echo "  Parameters: $(echo "$PARAMS" | wc -w)" >> "$SUMMARY_FILE"
    echo "  Levels: $(echo "$LEVELS" | wc -w)" >> "$SUMMARY_FILE"
done

# Final summary
echo ""
echo "=============================================="
echo "Summary"
echo "=============================================="
echo "Total queries:  $TOTAL"
echo "Success:        $SUCCESS"
echo "Empty:          $EMPTY"
echo "Failed:         $FAILED"
echo ""
echo "Results saved to: $OUTPUT_DIR"

echo "" >> "$SUMMARY_FILE"
echo "============================================" >> "$SUMMARY_FILE"
echo "Results" >> "$SUMMARY_FILE"
echo "============================================" >> "$SUMMARY_FILE"
echo "Total queries:  $TOTAL" >> "$SUMMARY_FILE"
echo "Success:        $SUCCESS" >> "$SUMMARY_FILE"
echo "Empty:          $EMPTY" >> "$SUMMARY_FILE"
echo "Failed:         $FAILED" >> "$SUMMARY_FILE"

# Generate index HTML for easy viewing
INDEX_FILE="$OUTPUT_DIR/index.html"
cat > "$INDEX_FILE" << 'HTMLEOF'
<!DOCTYPE html>
<html>
<head>
    <title>EDR Product Dump</title>
    <style>
        body { font-family: monospace; margin: 20px; background: #1a1a2e; color: #eee; }
        h1, h2 { color: #00d4ff; }
        .collection { margin: 20px 0; padding: 15px; background: #16213e; border-radius: 8px; }
        .param { margin: 10px 0; padding: 10px; background: #0f3460; border-radius: 4px; }
        .param-name { font-weight: bold; color: #e94560; }
        .links a { margin-right: 15px; color: #00d4ff; }
        pre { background: #0a0a15; padding: 15px; border-radius: 4px; overflow-x: auto; max-height: 400px; }
        .success { color: #4ade80; }
        .empty { color: #fbbf24; }
        .failed { color: #f87171; }
        #viewer { margin-top: 20px; }
        #json-content { white-space: pre-wrap; }
    </style>
</head>
<body>
    <h1>EDR Product Dump</h1>
    <div id="summary"></div>
    <div id="collections"></div>
    <div id="viewer">
        <h2>JSON Viewer</h2>
        <pre id="json-content">Click a file link to view its contents</pre>
    </div>
    
    <script>
        async function loadFile(path) {
            try {
                const resp = await fetch(path);
                if (!resp.ok) {
                    document.getElementById('json-content').textContent = 'HTTP Error: ' + resp.status + ' ' + resp.statusText;
                    return;
                }
                const text = await resp.text();
                try {
                    const data = JSON.parse(text);
                    document.getElementById('json-content').textContent = JSON.stringify(data, null, 2);
                } catch (parseErr) {
                    // Not valid JSON - show raw text
                    document.getElementById('json-content').textContent = text || '(empty file)';
                }
            } catch (e) {
                document.getElementById('json-content').textContent = 'Error loading file: ' + e.message;
            }
        }
        
        async function init() {
            // Load summary
            try {
                const summaryResp = await fetch('summary.txt');
                const summaryText = await summaryResp.text();
                document.getElementById('summary').innerHTML = '<pre>' + summaryText + '</pre>';
            } catch (e) {
                console.error('Could not load summary:', e);
            }
            
            // Load collections metadata
            try {
                const collectionsResp = await fetch('metadata/collections.json');
                const collectionsData = await collectionsResp.json();
                
                let html = '';
                for (const coll of collectionsData.collections || []) {
                    html += `<div class="collection">`;
                    html += `<h2>${coll.title || coll.id}</h2>`;
                    html += `<p>${coll.description || ''}</p>`;
                    
                    // List parameters
                    const params = Object.keys(coll.parameter_names || {});
                    for (const param of params) {
                        html += `<div class="param">`;
                        html += `<span class="param-name">${param}</span>`;
                        html += `<div class="links">`;
                        html += `<a href="#" onclick="loadFile('covjson/${coll.id}/${param}.json'); return false;">CovJSON</a>`;
                        html += `<a href="#" onclick="loadFile('geojson/${coll.id}/${param}.json'); return false;">GeoJSON</a>`;
                        html += `</div>`;
                        html += `</div>`;
                    }
                    
                    html += `</div>`;
                }
                
                document.getElementById('collections').innerHTML = html;
            } catch (e) {
                document.getElementById('collections').innerHTML = '<p>Error loading collections: ' + e.message + '</p>';
            }
        }
        
        init();
    </script>
</body>
</html>
HTMLEOF

echo "View results: open $OUTPUT_DIR/index.html"
echo "(or serve with: python3 -m http.server -d $OUTPUT_DIR 8000)"
