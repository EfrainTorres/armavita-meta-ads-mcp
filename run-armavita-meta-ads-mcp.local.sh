#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec env \
  PYTHONDONTWRITEBYTECODE=1 \
  UV_PROJECT="$REPO_DIR" \
  uv run --project "$REPO_DIR" armavita-meta-ads-mcp "$@"
