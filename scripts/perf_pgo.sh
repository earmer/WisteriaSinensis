#!/usr/bin/env bash
# Build and run `mutil-engine`'s `perf_survey` with PGO, `target-cpu=native`,
# and `panic=abort` on top of `[profile.release]` (`lto = "fat"`, `codegen-units = 1`)
# and `mimalloc`.
#
# Usage:
#   ./scripts/perf_pgo.sh [output_json]
#
# Defaults to writing `mutil-engine/target/perf_data.json`.
# Requires the `llvm-tools` rustup component (`rustup component add llvm-tools`).

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ENGINE_DIR="$ROOT_DIR/mutil-engine"
OUT_JSON="${1:-$ENGINE_DIR/target/perf_data.json}"

SYSROOT="$(rustc --print sysroot)"
HOST="$(rustc -vV | awk '/^host:/ { print $2 }')"
LLVM_PROFDATA="$SYSROOT/lib/rustlib/$HOST/bin/llvm-profdata"

if [[ ! -x "$LLVM_PROFDATA" ]]; then
    echo "error: llvm-profdata not found at $LLVM_PROFDATA" >&2
    echo "hint: run 'rustup component add llvm-tools'" >&2
    exit 1
fi

PGO_DATA="$ENGINE_DIR/target/pgo-data"
PGO_GEN_DIR="$ENGINE_DIR/target/pgo-gen"
PGO_USE_DIR="$ENGINE_DIR/target/pgo-use"
MERGED_PROFDATA="$PGO_DATA/merged.profdata"

BASE_FLAGS="-Ctarget-cpu=native -Cpanic=abort"

rm -rf "$PGO_DATA"
mkdir -p "$PGO_DATA" "$(dirname "$OUT_JSON")"

echo "[1/4] Building PGO-instrumented perf_survey..." >&2
(
    cd "$ENGINE_DIR"
    RUSTFLAGS="$BASE_FLAGS -Cprofile-generate=$PGO_DATA" \
        cargo build --release --example perf_survey --target-dir "$PGO_GEN_DIR"
)

echo "[2/4] Collecting profile data..." >&2
(
    cd "$ENGINE_DIR"
    "$PGO_GEN_DIR/release/examples/perf_survey" > /dev/null
)
"$LLVM_PROFDATA" merge -o "$MERGED_PROFDATA" "$PGO_DATA"

echo "[3/4] Building PGO-optimized perf_survey..." >&2
(
    cd "$ENGINE_DIR"
    RUSTFLAGS="$BASE_FLAGS -Cprofile-use=$MERGED_PROFDATA" \
        cargo build --release --example perf_survey --target-dir "$PGO_USE_DIR"
)

echo "[4/4] Running PGO-optimized perf_survey -> $OUT_JSON" >&2
(
    cd "$ENGINE_DIR"
    "$PGO_USE_DIR/release/examples/perf_survey" > "$OUT_JSON"
)

echo "Done: $OUT_JSON" >&2
