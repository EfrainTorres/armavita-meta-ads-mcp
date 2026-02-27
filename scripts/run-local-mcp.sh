#!/usr/bin/env bash
set -euo pipefail

# Local helper script for launching the MCP server.
# Optional auth env vars:
# - META_ACCESS_TOKEN
# - META_APP_ID + META_APP_SECRET

echo "Starting Armavita Meta Ads MCP..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PYTHONPATH="$REPO_ROOT/src${PYTHONPATH:+:$PYTHONPATH}" python -m armavita_meta_ads_mcp "$@"
