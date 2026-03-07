#!/bin/bash
# =============================================================================
# Weather WMS - Kubernetes Deploy Script
# =============================================================================
# Syncs config files into the Helm chart and runs helm upgrade --install.
#
# Usage:
#   ./scripts/deploy-k8s.sh                    # Dev install/upgrade
#   ./scripts/deploy-k8s.sh --production       # Production with values overlay
#   ./scripts/deploy-k8s.sh --dry-run          # Preview without applying
#   ./scripts/deploy-k8s.sh --template         # Render templates only
#   ./scripts/deploy-k8s.sh --uninstall        # Remove the release
#
# Prerequisites:
#   - kubectl configured for the target k3s cluster
#   - helm 3 installed
#   - Gitea registry accessible from the cluster (for image pulls)
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CHART_DIR="${PROJECT_ROOT}/deploy/k8s/weather-wms"
RELEASE_NAME="weather-wms"
NAMESPACE="weather-wms"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Parse arguments
PRODUCTION=false
DRY_RUN=""
TEMPLATE_ONLY=false
UNINSTALL=false
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --production)
            PRODUCTION=true
            shift
            ;;
        --dry-run)
            DRY_RUN="--dry-run"
            shift
            ;;
        --template)
            TEMPLATE_ONLY=true
            shift
            ;;
        --uninstall)
            UNINSTALL=true
            shift
            ;;
        --namespace)
            NAMESPACE="$2"
            shift 2
            ;;
        --set|--set-string)
            EXTRA_ARGS+=("$1" "$2")
            shift 2
            ;;
        -f|--values)
            EXTRA_ARGS+=("$1" "$2")
            shift 2
            ;;
        *)
            echo -e "${RED}Unknown argument: $1${NC}"
            echo "Usage: $0 [--production] [--dry-run] [--template] [--uninstall] [--namespace NS]"
            exit 1
            ;;
    esac
done

# Check prerequisites
check_prereqs() {
    local missing=false

    if ! command -v helm &>/dev/null; then
        echo -e "${RED}Error: helm is not installed${NC}"
        missing=true
    fi

    if ! command -v kubectl &>/dev/null; then
        echo -e "${RED}Error: kubectl is not installed${NC}"
        missing=true
    fi

    if [ "$missing" = true ]; then
        exit 1
    fi

    # Check kubectl connectivity
    if ! kubectl cluster-info &>/dev/null; then
        echo -e "${RED}Error: Cannot connect to Kubernetes cluster${NC}"
        echo "Make sure kubectl is configured and the cluster is running."
        exit 1
    fi
}

# Sync config files into the chart directory
sync_config() {
    echo -e "${YELLOW}Syncing config files into chart...${NC}"
    rsync -a --delete \
        --exclude='*.py' \
        --exclude='__pycache__' \
        --exclude='README.md' \
        "${PROJECT_ROOT}/config/" "${CHART_DIR}/config/"
    echo -e "${GREEN}Config synced ($(du -sh "${CHART_DIR}/config" | cut -f1))${NC}"
}

# Uninstall the release
do_uninstall() {
    echo -e "${YELLOW}Uninstalling ${RELEASE_NAME} from namespace ${NAMESPACE}...${NC}"
    helm uninstall "${RELEASE_NAME}" -n "${NAMESPACE}" 2>/dev/null || true
    echo -e "${GREEN}Release uninstalled.${NC}"
    echo -e "${YELLOW}Note: PVCs are NOT deleted. To remove all data:${NC}"
    echo "  kubectl delete pvc --all -n ${NAMESPACE}"
    echo "  kubectl delete namespace ${NAMESPACE}"
}

# Build values arguments
build_values_args() {
    local args=("-f" "${CHART_DIR}/values.yaml")

    if [ "$PRODUCTION" = true ]; then
        if [ ! -f "${CHART_DIR}/values-production.yaml" ]; then
            echo -e "${RED}Error: values-production.yaml not found${NC}"
            exit 1
        fi
        args+=("-f" "${CHART_DIR}/values-production.yaml")
        echo -e "${YELLOW}Using production values overlay${NC}"
    fi

    # Append any extra --set or -f args
    args+=("${EXTRA_ARGS[@]}")

    echo "${args[@]}"
}

# Main
main() {
    echo "=========================================="
    echo " Weather WMS - Kubernetes Deploy"
    echo "=========================================="
    echo ""

    if [ "$UNINSTALL" = true ]; then
        check_prereqs
        do_uninstall
        exit 0
    fi

    if [ "$TEMPLATE_ONLY" = true ]; then
        sync_config
        local values_args
        values_args=$(build_values_args)
        echo -e "${YELLOW}Rendering templates...${NC}"
        helm template "${RELEASE_NAME}" "${CHART_DIR}" \
            --namespace "${NAMESPACE}" \
            ${values_args} \
            "${EXTRA_ARGS[@]}" 2>&1
        exit 0
    fi

    check_prereqs
    sync_config

    # Lint first
    echo -e "${YELLOW}Linting chart...${NC}"
    if ! helm lint "${CHART_DIR}" 2>&1 | grep -q "0 chart(s) failed"; then
        echo -e "${RED}Chart lint failed!${NC}"
        helm lint "${CHART_DIR}"
        exit 1
    fi
    echo -e "${GREEN}Lint passed${NC}"

    # Build values args
    local values_args
    values_args=$(build_values_args)

    # Install or upgrade
    echo ""
    if [ -n "$DRY_RUN" ]; then
        echo -e "${YELLOW}Dry run (no changes will be applied)...${NC}"
    else
        echo -e "${YELLOW}Installing/upgrading ${RELEASE_NAME} in namespace ${NAMESPACE}...${NC}"
    fi

    helm upgrade --install "${RELEASE_NAME}" "${CHART_DIR}" \
        --namespace "${NAMESPACE}" --create-namespace \
        ${values_args} \
        ${DRY_RUN}

    if [ -z "$DRY_RUN" ]; then
        echo ""
        echo -e "${GREEN}Deploy initiated!${NC}"
        echo -e "${YELLOW}Waiting for rollout...${NC}"

        # Wait for critical deployments
        kubectl -n "${NAMESPACE}" rollout status deployment/"${RELEASE_NAME}-data-pipeline" --timeout=300s 2>/dev/null || true
        kubectl -n "${NAMESPACE}" rollout status deployment/"${RELEASE_NAME}-wms-api" --timeout=300s 2>/dev/null || true
        kubectl -n "${NAMESPACE}" rollout status deployment/"${RELEASE_NAME}-edr-api" --timeout=300s 2>/dev/null || true

        echo ""
        echo -e "${GREEN}Deploy complete!${NC}"
        echo ""
        echo "Pod status:"
        kubectl get pods -n "${NAMESPACE}" -o wide
    fi
}

main
