#!/usr/bin/env bash
# Builds the engine harness to wasm and puts it where the page loads it.
set -euo pipefail
cd "$(dirname "$0")"

# Prefer the rustup shim when it exists: a Homebrew `cargo` in PATH often has
# no wasm32 standard library, while the rustup toolchain can install one.
if [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]; then
  CARGO="${CARGO_HOME:-$HOME/.cargo}/bin/cargo"
else
  CARGO=cargo
fi

if ! "$CARGO" build --release --target wasm32-unknown-unknown; then
  echo "" >&2
  echo "If the target is missing: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi

mkdir -p web/pkg
cp "${CARGO_TARGET_DIR:-target}"/wasm32-unknown-unknown/release/mutil_demo.wasm web/pkg/
echo "built web/pkg/mutil_demo.wasm"
