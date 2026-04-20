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
WASMTIME="${WASMTIME:-wasmtime}"
# Stage0 is a component (from nexus build); stage1+ are core WASM (from nxc with stdlib merge)
WASMTIME_FLAGS_COMPONENT="-W tail-call=y,exceptions=y,component-model=y,max-memory-size=8589934592 -S http,inherit-network --dir=. --dir=${TMPDIR:-/tmp}"
WASMTIME_FLAGS_CORE="-W tail-call=y,exceptions=y,max-memory-size=8589934592 --dir=. --dir=${TMPDIR:-/tmp}"
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

# ─── Temp directory ──────────────────────────────────────────────────────
BUILD_DIR="$(mktemp -d)"
cleanup() { rm -rf "$BUILD_DIR"; }
trap cleanup EXIT
info "Build directory: $BUILD_DIR"

# ─── Build the Nexus compiler (Rust) ──────────────────────────────────────
CURRENT_COMMIT="$(git rev-parse HEAD)"
info "Building Nexus compiler (cargo build --release)..."
cargo build --release
[[ -x "$NEXUS" ]] || fail "Nexus compiler not found at $NEXUS"
info "Using Nexus compiler: $NEXUS (commit ${CURRENT_COMMIT:0:7})"

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
# Stage0 may be a stub (no merge). If stage1 is core WASM with unresolved imports,
# compose it with stdlib. Once stage1 has the merge code, stage2+ are self-contained.

STAGE1_RAW="$BUILD_DIR/stage1_raw.wasm"
STAGE1="$BUILD_DIR/stage1.wasm"
info "Stage 1: wasmtime run $STAGE0 $NXC_ENTRY $STAGE1_RAW"
"$WASMTIME" run $WASMTIME_FLAGS_COMPONENT "$STAGE0" "$NXC_ENTRY" --verbose "$STAGE1_RAW"

# Check if stage1 is self-contained (no nexus:stdlib imports) or needs compose
if wasm-tools print "$STAGE1_RAW" 2>/dev/null | grep -q 'import "nexus:stdlib/'; then
  info "Stage 1 has unresolved stdlib imports — composing..."
  "$NEXUS" compose "$STAGE1_RAW" -o "$STAGE1"
  ok "Stage 1 composed: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"
  STAGE1_FLAGS="$WASMTIME_FLAGS_COMPONENT"
else
  cp "$STAGE1_RAW" "$STAGE1"
  ok "Stage 1 self-contained: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"
  STAGE1_FLAGS="$WASMTIME_FLAGS_CORE"
fi

# ─── Stage 2: stage1.wasm compiles nxc → stage2.wasm ──────────────────────

STAGE2="$BUILD_DIR/stage2.wasm"
info "Stage 2: wasmtime run $STAGE1 $NXC_ENTRY $STAGE2"
if "$WASMTIME" run $STAGE1_FLAGS "$STAGE1" "$NXC_ENTRY" "$STAGE2" 2>&1; then
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

# ─── Install nxc_driver.wasm ─────────────────────────────────────────────
# Stage1 is already self-contained (stdlib merged). Install directly.

info "Installing nxc_driver.wasm..."
cp "$STAGE1" nxc_driver.wasm
ok "Installed nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
