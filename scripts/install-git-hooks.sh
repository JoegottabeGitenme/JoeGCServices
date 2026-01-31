#!/usr/bin/env bash
#
# Install git hooks for the project
#
# Usage:
#   ./scripts/install-git-hooks.sh
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOKS_SRC="$PROJECT_ROOT/scripts/git-hooks"
HOOKS_DST="$PROJECT_ROOT/.git/hooks"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "Installing git hooks..."
echo ""

# Ensure destination exists
mkdir -p "$HOOKS_DST"

# Install each hook
for hook in "$HOOKS_SRC"/*; do
    if [[ -f "$hook" ]]; then
        hook_name=$(basename "$hook")
        dst="$HOOKS_DST/$hook_name"
        
        # Check if hook already exists
        if [[ -f "$dst" ]]; then
            echo -e "${YELLOW}  $hook_name: already exists (backing up to $hook_name.bak)${NC}"
            mv "$dst" "$dst.bak"
        fi
        
        # Copy and make executable
        cp "$hook" "$dst"
        chmod +x "$dst"
        echo -e "${GREEN}  $hook_name: installed${NC}"
    fi
done

echo ""
echo -e "${GREEN}Git hooks installed successfully!${NC}"
echo ""
echo "Hooks installed:"
for hook in "$HOOKS_SRC"/*; do
    if [[ -f "$hook" ]]; then
        echo "  - $(basename "$hook")"
    fi
done
