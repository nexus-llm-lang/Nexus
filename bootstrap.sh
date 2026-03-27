#!/usr/bin/env bash
#
# bootstrap.sh — Multi-stage bootstrap for the Nexus self-hosted compiler (nxc/)
#
# Stage 0: Rust compiler (nexus build) compiles nxc/driver.nx → stage0.wasm
# Stage 1: Run stage0.wasm (nexus exec) to compile nxc/driver.nx → stage1.wasm
# Stage 2: Run stage1.wasm (nexus exec) to compile nxc/driver.nx → stage2.wasm
# Verify:  stage1.wasm == stage2.wasm (fixed point)
#
# Usage: ./bootstrap.sh [--ci]
#   --ci    Strict mode for CI: fail on stage2 failure or non-identical output
#
set -euo pipefail

CI_MODE=false
for arg in "$@"; do
  case "$arg" in
    --ci) CI_MODE=true ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

NEXUS="${NEXUS:-./target/release/nexus}"
NXC_ENTRY="nxc/driver.nx"
BUILD_DIR="bootstrap_out"
NEXUS_EXEC_FLAGS="--allow-fs --allow-console --allow-proc --allow-random --allow-clock"
NEXUS_BUILD_FLAGS=""

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

info()  { printf "${CYAN}[bootstrap]${RESET} %s\n" "$*"; }
ok()    { printf "${GREEN}[bootstrap]${RESET} %s\n" "$*"; }
warn()  { printf "${YELLOW}[bootstrap]${RESET} %s\n" "$*"; }
fail()  { printf "${RED}[bootstrap]${RESET} %s\n" "$*" >&2; exit 1; }

# ─── Patch wasmparser function body size limit ────────────────────────────
# nxc-produced WASM has functions up to ~9MB; wasmparser's default limit
# (7654321) prevents wasmtime from loading them. Vendor a patched copy.

WASMPARSER_VENDOR="vendor/wasmparser"
WASMPARSER_LIMITS="$WASMPARSER_VENDOR/src/limits.rs"

if [[ ! -f "$WASMPARSER_LIMITS" ]] || ! grep -q '128_000_000' "$WASMPARSER_LIMITS" 2>/dev/null; then
  # Find the wasmparser 0.243 source in cargo registry
  WASMPARSER_SRC="$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" \
    -maxdepth 2 -type d -name 'wasmparser-0.243.*' 2>/dev/null | head -1)"
  if [[ -z "$WASMPARSER_SRC" ]]; then
    # Fetch it by building once (populates cargo registry)
    cargo check --quiet 2>/dev/null || true
    WASMPARSER_SRC="$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" \
      -maxdepth 2 -type d -name 'wasmparser-0.243.*' 2>/dev/null | head -1)"
  fi
  if [[ -n "$WASMPARSER_SRC" ]]; then
    info "Patching wasmparser MAX_WASM_FUNCTION_SIZE (7.6MB → 128MB)..."
    rm -rf "$WASMPARSER_VENDOR"
    cp -r "$WASMPARSER_SRC" "$WASMPARSER_VENDOR"
    sed -i.bak 's/7_654_321/128_000_000/g' "$WASMPARSER_LIMITS"
    rm -f "$WASMPARSER_LIMITS.bak"
  else
    warn "Could not find wasmparser source to patch — stage2 may fail to load"
  fi
fi

# ─── Ensure the Rust compiler is built and up to date ─────────────────────

CURRENT_COMMIT="$(git rev-parse HEAD)"
COMMIT_FILE="target/release/.nexus_commit"

needs_rebuild() {
  [[ ! -x "$NEXUS" ]] && return 0
  [[ ! -f "$COMMIT_FILE" ]] && return 0
  [[ "$(cat "$COMMIT_FILE")" != "$CURRENT_COMMIT" ]] && return 0
  return 1
}

if needs_rebuild; then
  if [[ -x "$NEXUS" && -f "$COMMIT_FILE" ]]; then
    local_hash="$(cat "$COMMIT_FILE")"
    info "Rust compiler is stale (built from ${local_hash:0:7}, current ${CURRENT_COMMIT:0:7})"
  fi
  info "Building Rust compiler (cargo build --release)..."
  cargo build --release
  mkdir -p "$(dirname "$COMMIT_FILE")"
  echo "$CURRENT_COMMIT" > "$COMMIT_FILE"
fi

[[ -x "$NEXUS" ]] || fail "Rust compiler not found at $NEXUS"
info "Using Rust compiler: $NEXUS (commit ${CURRENT_COMMIT:0:7})"

mkdir -p "$BUILD_DIR"

# ─── Stage 0: Rust compiler → stage0.wasm ─────────────────────────────────
# Try to reuse `nexus nxc` auto-cache (target/nxc/nxc_driver.wasm).
# If the cache is valid, copy it as stage0 instead of recompiling.

STAGE0="$BUILD_DIR/stage0.wasm"
NXC_CACHE="target/nxc/nxc_driver.wasm"
NXC_CACHE_COMMIT="target/nxc/.nxc_commit"

# Reuse cache only if it was built from the current commit.
nxc_cache_valid() {
  [[ -f "$NXC_CACHE" ]] || return 1
  [[ -f "$NXC_CACHE_COMMIT" ]] || return 1
  [[ "$(cat "$NXC_CACHE_COMMIT")" == "$CURRENT_COMMIT" ]] || return 1
  return 0
}

if nxc_cache_valid; then
  info "Stage 0: reusing cached nxc_driver.wasm (commit ${CURRENT_COMMIT:0:7})"
  cp "$NXC_CACHE" "$STAGE0"
else
  info "Stage 0: nexus build $NXC_ENTRY → $STAGE0"
  # Trigger nxc cache rebuild
  "$NEXUS" nxc >/dev/null 2>&1 || true
  if [[ -f "$NXC_CACHE" ]]; then
    cp "$NXC_CACHE" "$STAGE0"
    mkdir -p "$(dirname "$NXC_CACHE_COMMIT")"
    echo "$CURRENT_COMMIT" > "$NXC_CACHE_COMMIT"
  else
    "$NEXUS" build $NEXUS_BUILD_FLAGS "$NXC_ENTRY" -o "$STAGE0"
  fi
fi
ok "Stage 0 complete: $STAGE0 ($(wc -c < "$STAGE0" | tr -d ' ') bytes)"

# ─── Stage 1: stage0.wasm compiles nxc → stage1.wasm ──────────────────────

STAGE1="$BUILD_DIR/stage1.wasm"
info "Stage 1: nexus exec $STAGE0 -- $NXC_ENTRY $STAGE1"
"$NEXUS" exec $NEXUS_EXEC_FLAGS "$STAGE0" -- "$NXC_ENTRY" --verbose "$STAGE1"
ok "Stage 1 complete: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"

# ─── Bundle stage1 with stdlib ─────────────────────────────────────────────
# Stage1 is a core WASM that imports stdlib/stdlib.wasm.
# Bundle it so stage2 can run without external dependencies.

WASM_MERGE="${NEXUS_WASM_MERGE:-wasm-merge}"
STAGE1_BUNDLED="$BUILD_DIR/stage1_bundled.wasm"
HOST_STUB_WAT="$(pwd)/nxlib/stdlib/nexus_host_stub.wat"
HOST_STUB_WASM="$(pwd)/stdlib/nexus_host_stub.wasm"
if [[ -f "$HOST_STUB_WAT" && ! -f "$HOST_STUB_WASM" ]]; then
  if command -v wat2wasm >/dev/null 2>&1; then
    info "Compiling nexus_host_stub.wat → .wasm"
    wat2wasm "$HOST_STUB_WAT" -o "$HOST_STUB_WASM"
  fi
fi
if command -v "$WASM_MERGE" >/dev/null 2>&1 || [[ -x "$WASM_MERGE" ]]; then
  info "Bundling stage1 with stdlib..."
  "$WASM_MERGE" "$STAGE1" __main \
    "$(pwd)/stdlib/stdlib.wasm" "stdlib/stdlib.wasm" \
    "$(pwd)/stdlib/nexus_host_stub.wasm" "nexus:cli/nexus-host" \
    --all-features --enable-tail-call --rename-export-conflicts \
    -o "$STAGE1_BUNDLED"
  ok "Bundled: $STAGE1_BUNDLED ($(wc -c < "$STAGE1_BUNDLED" | tr -d ' ') bytes)"
else
  warn "wasm-merge not found — skipping bundle, stage2 may fail"
  cp "$STAGE1" "$STAGE1_BUNDLED"
fi

# ─── Stage 2: stage1.wasm compiles nxc → stage2.wasm ──────────────────────

STAGE2="$BUILD_DIR/stage2.wasm"
info "Stage 2: nexus exec $STAGE1_BUNDLED -- $NXC_ENTRY $STAGE2"
if "$NEXUS" exec $NEXUS_EXEC_FLAGS "$STAGE1_BUNDLED" -- "$NXC_ENTRY" --verbose "$STAGE2" 2>&1; then
  ok "Stage 2 complete: $STAGE2 ($(wc -c < "$STAGE2" | tr -d ' ') bytes)"
else
  if [[ "$CI_MODE" == true ]]; then
    fail "Stage 2 failed — nxc-produced WASM is not self-executable."
  fi
  warn "Stage 2 failed — nxc-produced WASM not yet self-executable."
  warn "Stage 1 output is still valid (compiled by the Rust-built stage0)."
  exit 1
fi

# ─── Verify fixed point ───────────────────────────────────────────────────

info "Verifying fixed point: stage1 == stage2"
if cmp -s "$STAGE1" "$STAGE2"; then
  ok "Fixed point reached! stage1.wasm and stage2.wasm are identical."
  ok "The self-hosted compiler is verified."
else
  if [[ "$CI_MODE" == true ]]; then
    S1_SIZE=$(wc -c < "$STAGE1" | tr -d ' ')
    S2_SIZE=$(wc -c < "$STAGE2" | tr -d ' ')
    info "stage1: $S1_SIZE bytes, stage2: $S2_SIZE bytes"
    fail "Fixed point NOT reached — stage1.wasm and stage2.wasm differ."
  fi
  warn "stage1.wasm and stage2.wasm differ — not yet at fixed point."
  warn "This is expected while nxc codegen is still maturing."
fi
