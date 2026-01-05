#!/bin/bash
# =============================================================================
# OGC Endpoint Discovery Script
# =============================================================================
# Dynamically discovers WMS layers, WMTS layers, and EDR collections/parameters
# from the target server. Results are cached for 24 hours to speed up repeated
# scans.
#
# Usage:
#   ./discover-ogc-endpoints.sh <target_url> <output_dir> [--refresh]
#
# Outputs:
#   - wms-layers.txt: List of WMS layer names
#   - wmts-layers.txt: List of WMTS layer identifiers
#   - edr-collections.txt: List of EDR collection IDs
#   - edr-parameters.txt: List of EDR parameter names (deduplicated)
#   - ogc-targets.txt: Complete list of target URLs for scanning
# =============================================================================

set -euo pipefail

TARGET="${1:-}"
OUTPUT_DIR="${2:-}"
REFRESH="${3:-}"

if [[ -z "$TARGET" || -z "$OUTPUT_DIR" ]]; then
    echo "Usage: $0 <target_url> <output_dir> [--refresh]"
    exit 1
fi

# Cache settings
CACHE_DIR="${OUTPUT_DIR}/cache"
CACHE_MAX_AGE=$((24 * 60 * 60))  # 24 hours in seconds
CACHE_TIMESTAMP_FILE="${CACHE_DIR}/.cache_timestamp"

mkdir -p "$CACHE_DIR"

# Check if cache is valid
cache_is_valid() {
    if [[ "$REFRESH" == "--refresh" ]]; then
        return 1
    fi
    
    if [[ ! -f "$CACHE_TIMESTAMP_FILE" ]]; then
        return 1
    fi
    
    local cache_time=$(cat "$CACHE_TIMESTAMP_FILE")
    local current_time=$(date +%s)
    local age=$((current_time - cache_time))
    
    if [[ $age -gt $CACHE_MAX_AGE ]]; then
        return 1
    fi
    
    # Check if all cache files exist
    if [[ ! -f "${CACHE_DIR}/wms-layers.txt" ]] || \
       [[ ! -f "${CACHE_DIR}/wmts-layers.txt" ]] || \
       [[ ! -f "${CACHE_DIR}/edr-collections.txt" ]]; then
        return 1
    fi
    
    return 0
}

# Update cache timestamp
update_cache_timestamp() {
    date +%s > "$CACHE_TIMESTAMP_FILE"
}

echo "[DISCOVERY] Target: ${TARGET}"

if cache_is_valid; then
    echo "[DISCOVERY] Using cached endpoint data (less than 24 hours old)"
    echo "[DISCOVERY] Use --refresh to force re-discovery"
    
    # Copy cached files to output
    cp "${CACHE_DIR}/wms-layers.txt" "${OUTPUT_DIR}/" 2>/dev/null || true
    cp "${CACHE_DIR}/wmts-layers.txt" "${OUTPUT_DIR}/" 2>/dev/null || true
    cp "${CACHE_DIR}/edr-collections.txt" "${OUTPUT_DIR}/" 2>/dev/null || true
    cp "${CACHE_DIR}/edr-parameters.txt" "${OUTPUT_DIR}/" 2>/dev/null || true
else
    echo "[DISCOVERY] Discovering OGC endpoints..."

    # ==========================================================================
    # WMS Layer Discovery
    # ==========================================================================
    echo "[DISCOVERY] Fetching WMS GetCapabilities..."
    WMS_CAPS=$(curl -s --max-time 30 "${TARGET}/wms?SERVICE=WMS&REQUEST=GetCapabilities" 2>/dev/null || echo "")
    
    if [[ -n "$WMS_CAPS" ]]; then
        # Extract layer names (skip parent layer names that are just categories)
        echo "$WMS_CAPS" | grep -oP '(?<=<Name>)[^<]+' | sort -u > "${CACHE_DIR}/wms-layers.txt"
        WMS_COUNT=$(wc -l < "${CACHE_DIR}/wms-layers.txt")
        echo "[DISCOVERY] Found ${WMS_COUNT} WMS layers"
    else
        echo "[DISCOVERY] WARNING: Could not fetch WMS capabilities"
        touch "${CACHE_DIR}/wms-layers.txt"
    fi

    # ==========================================================================
    # WMTS Layer Discovery
    # ==========================================================================
    echo "[DISCOVERY] Fetching WMTS GetCapabilities..."
    WMTS_CAPS=$(curl -s --max-time 30 "${TARGET}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities" 2>/dev/null || echo "")
    
    if [[ -n "$WMTS_CAPS" ]]; then
        # Extract layer identifiers
        echo "$WMTS_CAPS" | grep -oP '(?<=<ows:Identifier>)[^<]+' | \
            grep -v -E '^(WebMercatorQuad|WorldCRS84Quad|time|run|forecast|elevation|standard|enhanced|gradient|isolines|wind_speed)' | \
            sort -u > "${CACHE_DIR}/wmts-layers.txt"
        WMTS_COUNT=$(wc -l < "${CACHE_DIR}/wmts-layers.txt")
        echo "[DISCOVERY] Found ${WMTS_COUNT} WMTS layers"
    else
        echo "[DISCOVERY] WARNING: Could not fetch WMTS capabilities"
        touch "${CACHE_DIR}/wmts-layers.txt"
    fi

    # ==========================================================================
    # EDR Collection Discovery
    # ==========================================================================
    echo "[DISCOVERY] Fetching EDR collections..."
    EDR_COLLECTIONS=$(curl -s --max-time 30 "${TARGET}/edr/collections" 2>/dev/null || echo "")
    
    if [[ -n "$EDR_COLLECTIONS" ]] && echo "$EDR_COLLECTIONS" | jq -e '.collections' >/dev/null 2>&1; then
        echo "$EDR_COLLECTIONS" | jq -r '.collections[].id' 2>/dev/null | sort -u > "${CACHE_DIR}/edr-collections.txt"
        EDR_COUNT=$(wc -l < "${CACHE_DIR}/edr-collections.txt")
        echo "[DISCOVERY] Found ${EDR_COUNT} EDR collections"
        
        # Discover parameters for each collection
        echo "[DISCOVERY] Fetching EDR parameters for each collection..."
        > "${CACHE_DIR}/edr-parameters.txt"
        
        while IFS= read -r collection; do
            if [[ -n "$collection" ]]; then
                COLLECTION_DATA=$(curl -s --max-time 10 "${TARGET}/edr/collections/${collection}" 2>/dev/null || echo "")
                if [[ -n "$COLLECTION_DATA" ]]; then
                    echo "$COLLECTION_DATA" | jq -r '.parameter_names | keys[]' 2>/dev/null >> "${CACHE_DIR}/edr-parameters.txt" || true
                fi
            fi
        done < "${CACHE_DIR}/edr-collections.txt"
        
        # Deduplicate parameters
        sort -u "${CACHE_DIR}/edr-parameters.txt" -o "${CACHE_DIR}/edr-parameters.txt"
        PARAM_COUNT=$(wc -l < "${CACHE_DIR}/edr-parameters.txt")
        echo "[DISCOVERY] Found ${PARAM_COUNT} unique EDR parameters"
    else
        echo "[DISCOVERY] WARNING: Could not fetch EDR collections"
        touch "${CACHE_DIR}/edr-collections.txt"
        touch "${CACHE_DIR}/edr-parameters.txt"
    fi

    # Update cache timestamp
    update_cache_timestamp
    
    # Copy to output directory
    cp "${CACHE_DIR}/wms-layers.txt" "${OUTPUT_DIR}/"
    cp "${CACHE_DIR}/wmts-layers.txt" "${OUTPUT_DIR}/"
    cp "${CACHE_DIR}/edr-collections.txt" "${OUTPUT_DIR}/"
    cp "${CACHE_DIR}/edr-parameters.txt" "${OUTPUT_DIR}/"
fi

# =============================================================================
# Generate Target URLs for Nuclei
# =============================================================================
echo "[DISCOVERY] Generating target URLs for scanning..."

TARGETS_FILE="${OUTPUT_DIR}/ogc-targets.txt"
> "$TARGETS_FILE"

# Add base endpoints
cat >> "$TARGETS_FILE" << EOF
${TARGET}/
${TARGET}/wms?SERVICE=WMS&REQUEST=GetCapabilities
${TARGET}/wmts?SERVICE=WMTS&REQUEST=GetCapabilities
${TARGET}/edr
${TARGET}/edr/collections
${TARGET}/edr/conformance
${TARGET}/edr/api
EOF

# Add WMS GetMap URLs for each layer
if [[ -f "${OUTPUT_DIR}/wms-layers.txt" ]]; then
    while IFS= read -r layer; do
        if [[ -n "$layer" && "$layer" != "WMS" ]]; then
            echo "${TARGET}/wms?SERVICE=WMS&REQUEST=GetMap&LAYERS=${layer}&STYLES=&CRS=EPSG:4326&BBOX=-180,-90,180,90&WIDTH=256&HEIGHT=256&FORMAT=image/png" >> "$TARGETS_FILE"
        fi
    done < "${OUTPUT_DIR}/wms-layers.txt"
fi

# Add WMTS GetTile URLs for each layer
if [[ -f "${OUTPUT_DIR}/wmts-layers.txt" ]]; then
    while IFS= read -r layer; do
        if [[ -n "$layer" ]]; then
            echo "${TARGET}/wmts?SERVICE=WMTS&REQUEST=GetTile&LAYER=${layer}&STYLE=default&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=0&TILEROW=0&TILECOL=0&FORMAT=image/png" >> "$TARGETS_FILE"
        fi
    done < "${OUTPUT_DIR}/wmts-layers.txt"
fi

# Add EDR position query URLs for each collection
if [[ -f "${OUTPUT_DIR}/edr-collections.txt" ]]; then
    while IFS= read -r collection; do
        if [[ -n "$collection" ]]; then
            echo "${TARGET}/edr/collections/${collection}" >> "$TARGETS_FILE"
            echo "${TARGET}/edr/collections/${collection}/position?coords=POINT(-104.5%2039.5)&f=CoverageJSON" >> "$TARGETS_FILE"
        fi
    done < "${OUTPUT_DIR}/edr-collections.txt"
fi

TOTAL_TARGETS=$(wc -l < "$TARGETS_FILE")
echo "[DISCOVERY] Generated ${TOTAL_TARGETS} target URLs"

# Create summary JSON
cat > "${OUTPUT_DIR}/discovery-summary.json" << EOF
{
    "target": "${TARGET}",
    "timestamp": "$(date -Iseconds)",
    "cached": $(cache_is_valid && echo "true" || echo "false"),
    "counts": {
        "wms_layers": $(wc -l < "${OUTPUT_DIR}/wms-layers.txt" 2>/dev/null || echo 0),
        "wmts_layers": $(wc -l < "${OUTPUT_DIR}/wmts-layers.txt" 2>/dev/null || echo 0),
        "edr_collections": $(wc -l < "${OUTPUT_DIR}/edr-collections.txt" 2>/dev/null || echo 0),
        "edr_parameters": $(wc -l < "${OUTPUT_DIR}/edr-parameters.txt" 2>/dev/null || echo 0),
        "total_targets": ${TOTAL_TARGETS}
    }
}
EOF

echo "[DISCOVERY] Complete. Results saved to ${OUTPUT_DIR}/"
