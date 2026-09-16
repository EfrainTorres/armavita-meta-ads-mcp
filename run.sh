#!/usr/bin/env bash
set -euo pipefail

SERVER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY_NAME="armavita-meta-ads-mcp"

if [[ -n "${ARMAVITA_META_ADS_MCP_BIN:-}" ]]; then
  if [[ ! -x "$ARMAVITA_META_ADS_MCP_BIN" ]]; then
    echo "ARMAVITA_META_ADS_MCP_BIN is not an executable file" >&2
    exit 127
  fi
  exec "$ARMAVITA_META_ADS_MCP_BIN" "$@"
fi

for candidate in \
  "$SERVER_DIR/target/release/$BINARY_NAME" \
  "$SERVER_DIR/../../target/release/$BINARY_NAME"
do
  if [[ -x "$candidate" ]]; then
    exec "$candidate" "$@"
  fi
done

if command -v "$BINARY_NAME" >/dev/null 2>&1; then
  exec "$BINARY_NAME" "$@"
fi

echo "$BINARY_NAME is not built or installed; run the README's one-time release build first" >&2
exit 127
