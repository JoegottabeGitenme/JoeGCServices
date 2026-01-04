#!/bin/bash
# =============================================================================
# Rate Limiting Verification
# =============================================================================
# Tests that rate limiting is properly configured and enforced by sending
# a burst of requests to trigger the rate limiter.
# =============================================================================

set -uo pipefail

TARGET="$1"
OUTPUT_DIR="$2"

RESULTS_JSON="${OUTPUT_DIR}/raw/rate-limit-check.json"

echo "[RATE] Testing rate limiting enforcement..."

# Test parameters
MAX_REQUESTS=100
TEST_ENDPOINT="/health"
TEST_URL="${TARGET}${TEST_ENDPOINT}"

echo "[RATE] Sending ${MAX_REQUESTS} rapid requests to ${TEST_ENDPOINT}..."

count=0
count_200=0
count_429=0
count_other=0
rate_limited=false
rate_limit_at=0
start_time=$(date +%s)

while [[ $count -lt $MAX_REQUESTS ]]; do
    status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "$TEST_URL" 2>/dev/null) || status="000"
    ((count++)) || true
    
    case "$status" in
        200|204)
            ((count_200++)) || true
            ;;
        429)
            ((count_429++)) || true
            if [[ "$rate_limited" == "false" ]]; then
                rate_limited=true
                rate_limit_at=$count
                echo "[RATE] Rate limit triggered at request #${count}"
            fi
            ;;
        *)
            ((count_other++)) || true
            ;;
    esac
done

end_time=$(date +%s)
duration=$((end_time - start_time))

# Determine result
result="pass"
severity="info"
message=""

if [[ "$rate_limited" == "true" ]]; then
    result="pass"
    message="Rate limiting active - triggered after ${rate_limit_at} requests"
    echo "[RATE] SUCCESS: $message"
else
    result="warning"
    severity="medium"
    message="Rate limiting NOT triggered after ${MAX_REQUESTS} requests"
    echo "[RATE] WARNING: $message"
fi

# Generate JSON report
cat > "$RESULTS_JSON" << EOF
{
    "scan_type": "rate_limit",
    "target": "${TARGET}",
    "timestamp": "$(date -Iseconds)",
    "config": {
        "max_requests": ${MAX_REQUESTS},
        "test_endpoint": "${TEST_ENDPOINT}"
    },
    "results": {
        "result": "${result}",
        "severity": "${severity}",
        "message": "${message}",
        "total_requests": ${count},
        "rate_limit_triggered": ${rate_limited},
        "rate_limit_at": ${rate_limit_at},
        "status_200": ${count_200},
        "status_429": ${count_429},
        "status_other": ${count_other},
        "duration_seconds": ${duration}
    }
}
EOF

echo "[RATE] Results saved to ${RESULTS_JSON}"
