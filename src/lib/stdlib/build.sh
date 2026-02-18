#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../../.." && pwd)"
STDIO_DIR="$REPO_ROOT/src/lib/stdio"

# Build pure stdlib wasm.
cargo build \
  --manifest-path "$SCRIPT_DIR/Cargo.toml" \
  --target wasm32-wasip1 \
  --release

# Build stdio wasm.
cargo build \
  --manifest-path "$STDIO_DIR/Cargo.toml" \
  --target wasm32-wasip1 \
  --release

# stdio.wasm exports IO-facing functions.
cp \
  "$STDIO_DIR/target/wasm32-wasip1/release/nexus_stdio_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/stdio.wasm"

# stdlib.wasm exports pure helper intrinsics.
cp \
  "$SCRIPT_DIR/target/wasm32-wasip1/release/nexus_stdlib_wasm.wasm" \
  "$REPO_ROOT/nxlib/stdlib/stdlib.wasm"

echo "Wrote $REPO_ROOT/nxlib/stdlib/{stdio,stdlib}.wasm"
