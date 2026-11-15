#!/usr/bin/env bash
# Serves the page. A static server is required: ES modules and wasm do not
# load from file:// URLs.
set -euo pipefail
cd "$(dirname "$0")"
port="${1:-8000}"
echo "http://127.0.0.1:$port/"
exec python3 -m http.server "$port" --bind 127.0.0.1
