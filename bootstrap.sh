#!/usr/bin/env bash
#
# bootstrap.sh — Multi-stage bootstrap for the Nexus self-hosted compiler (nxc/)
#
# Stage 0: Locate a seed nxc_driver.wasm (pre-built, cached, or via NXC_SEED)
# Stage 1: Run stage0 to compile nxc/driver.nx → stage1.wasm
# Stage 2: Run stage1 to compile nxc/driver.nx → stage2.wasm
# Verify:  stage1.wasm == stage2.wasm (fixed point)
#
# No Rust compiler needed — all stages use wasmtime directly.
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

NXC_ENTRY="nxc/driver.nx"
BUILD_DIR="bootstrap_out"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

info()  { printf "${CYAN}[bootstrap]${RESET} %s\n" "$*"; }
ok()    { printf "${GREEN}[bootstrap]${RESET} %s\n" "$*"; }
warn()  { printf "${YELLOW}[bootstrap]${RESET} %s\n" "$*"; }
fail()  { printf "${RED}[bootstrap]${RESET} %s\n" "$*" >&2; exit 1; }

# ─── Ensure tools are available ──────────────────────────────────────────────

command -v wasmtime >/dev/null 2>&1 || fail "wasmtime not found in PATH"
command -v wasm-tools >/dev/null 2>&1 || fail "wasm-tools not found in PATH"

# ─── Locate seed nxc_driver.wasm (stage 0) ───────────────────────────────────

find_seed() {
  if [[ -n "${NXC_SEED:-}" ]] && [[ -f "$NXC_SEED" ]]; then
    echo "$NXC_SEED"
    return
  fi

  if [[ -f "nxc_driver.wasm" ]]; then
    echo "nxc_driver.wasm"
    return
  fi

  if [[ -f "target/nxc/nxc_driver.wasm" ]]; then
    echo "target/nxc/nxc_driver.wasm"
    return
  fi

  return 1
}

SEED=$(find_seed) || fail "No seed nxc_driver.wasm found.
  Provide one via NXC_SEED=/path/to/nxc_driver.wasm
  or place nxc_driver.wasm in the project root."

info "Using seed: $SEED"

# ─── Ensure seed has _start export ───────────────────────────────────────────
# The Rust-built nxc_driver.wasm exports "main" but not "_start".
# wasmtime run needs _start, so we patch it via WAT roundtrip if needed.

ensure_start_export() {
  local wasm="$1"
  if wasm-tools print "$wasm" 2>/dev/null | grep -q 'export "_start"'; then
    return 0
  fi

  info "Patching $wasm: adding _start export..."
  local wat
  wat=$(mktemp /tmp/nxc_patch_XXXXXX.wat)
  wasm-tools print "$wasm" > "$wat"
  # Add _start as alias for main (both are (func) → void)
  perl -pi -e 's/\(export "main" \(func (\d+)\)\)/(export "main" (func $1))\n  (export "_start" (func $1))/' "$wat"
  wasm-tools parse "$wat" -o "$wasm"
  rm -f "$wat"
  ok "Patched $wasm with _start export"
}

ensure_start_export "$SEED"

# ─── Helper to run a wasm compiler stage ─────────────────────────────────────

run_stage() {
  local wasm="$1"
  local input="$2"
  local output="$3"
  wasmtime run -S preview2=n --dir=. "$wasm" "$input" "$output"
}

mkdir -p "$BUILD_DIR"

# ─── Stage 0 → Stage 1 ──────────────────────────────────────────────────────

STAGE0="$SEED"
STAGE1="$BUILD_DIR/stage1.wasm"

info "Stage 1: $STAGE0 compiles $NXC_ENTRY → $STAGE1"
run_stage "$STAGE0" "$NXC_ENTRY" "$STAGE1"
ok "Stage 1 complete: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"

# ─── Stage 1 → Stage 2 ──────────────────────────────────────────────────────

STAGE2="$BUILD_DIR/stage2.wasm"

info "Stage 2: $STAGE1 compiles $NXC_ENTRY → $STAGE2"
if run_stage "$STAGE1" "$NXC_ENTRY" "$STAGE2" 2>&1; then
  ok "Stage 2 complete: $STAGE2 ($(wc -c < "$STAGE2" | tr -d ' ') bytes)"
else
  if [[ "$CI_MODE" == true ]]; then
    fail "Stage 2 failed — nxc-produced WASM is not self-executable."
  fi
  warn "Stage 2 failed — nxc-produced WASM not yet self-executable."
  warn "Stage 1 output is still valid (compiled by the seed)."
  cp "$STAGE1" nxc_driver.wasm
  ok "Installed (stage 1): nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
  exit 0
fi

# ─── Verify fixed point ─────────────────────────────────────────────────────

info "Verifying fixed point: stage1 == stage2"
if cmp -s "$STAGE1" "$STAGE2"; then
  ok "Fixed point reached! stage1.wasm and stage2.wasm are identical."
  ok "The self-hosted compiler is verified."

  cp "$STAGE1" nxc_driver.wasm
  ok "Installed: nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
else
  if [[ "$CI_MODE" == true ]]; then
    S1_SIZE=$(wc -c < "$STAGE1" | tr -d ' ')
    S2_SIZE=$(wc -c < "$STAGE2" | tr -d ' ')
    info "stage1: $S1_SIZE bytes, stage2: $S2_SIZE bytes"
    fail "Fixed point NOT reached — stage1.wasm and stage2.wasm differ."
  fi
  warn "stage1.wasm and stage2.wasm differ — not yet at fixed point."
  warn "This is expected while nxc codegen is still maturing."
  cp "$STAGE1" nxc_driver.wasm
  ok "Installed (stage 1): nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
fi
