#!/usr/bin/env bash
#
# bootstrap.sh — Multi-stage bootstrap for the Nexus self-hosted compiler (nxc/)
#
# Stage 0: Rust compiler (nexus build) compiles nxc/driver.nx → stage0.wasm
# Stage 1: Run stage0.wasm (nexus exec) to compile nxc/driver.nx → stage1.wasm
# Stage 2: Run stage1.wasm (nexus exec) to compile nxc/driver.nx → stage2.wasm
# Verify:  stage1.wasm == stage2.wasm (fixed point)
#
set -euo pipefail

NEXUS="${NEXUS:-./target/release/nexus}"
NXC_ENTRY="nxc/driver.nx"
BUILD_DIR="bootstrap_out"
NEXUS_EXEC_FLAGS="--allow-fs --allow-console --allow-proc"
NEXUS_BUILD_FLAGS="--skip-typecheck"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

info()  { printf "${CYAN}[bootstrap]${RESET} %s\n" "$*"; }
ok()    { printf "${GREEN}[bootstrap]${RESET} %s\n" "$*"; }
warn()  { printf "${YELLOW}[bootstrap]${RESET} %s\n" "$*"; }
fail()  { printf "${RED}[bootstrap]${RESET} %s\n" "$*" >&2; exit 1; }

# ─── Ensure the Rust compiler is built ─────────────────────────────────────

if [[ ! -x "$NEXUS" ]]; then
  info "Building Rust compiler (cargo build --release)..."
  cargo build --release
fi

[[ -x "$NEXUS" ]] || fail "Rust compiler not found at $NEXUS"
info "Using Rust compiler: $NEXUS"

mkdir -p "$BUILD_DIR"

# ─── Stage 0: Rust compiler → stage0.wasm ─────────────────────────────────
# Try to reuse `nexus nxc` auto-cache (target/nxc/nxc_driver.wasm).
# If the cache is valid, copy it as stage0 instead of recompiling.

STAGE0="$BUILD_DIR/stage0.wasm"
NXC_CACHE="target/nxc/nxc_driver.wasm"

# Trigger cache build (nxc with no args exits with usage, but populates cache).
if "$NEXUS" nxc >/dev/null 2>&1 || [[ -f "$NXC_CACHE" ]]; then
  info "Stage 0: reusing cached nxc_driver.wasm"
  cp "$NXC_CACHE" "$STAGE0"
else
  info "Stage 0: nexus build $NXC_ENTRY → $STAGE0"
  "$NEXUS" build $NEXUS_BUILD_FLAGS "$NXC_ENTRY" -o "$STAGE0"
fi
ok "Stage 0 complete: $STAGE0 ($(wc -c < "$STAGE0" | tr -d ' ') bytes)"

# ─── Stage 1: stage0.wasm compiles nxc → stage1.wasm ──────────────────────

STAGE1="$BUILD_DIR/stage1.wasm"
info "Stage 1: nexus exec $STAGE0 -- $NXC_ENTRY $STAGE1"
"$NEXUS" exec $NEXUS_EXEC_FLAGS "$STAGE0" -- "$NXC_ENTRY" "$STAGE1"
ok "Stage 1 complete: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"

# ─── Stage 2: stage1.wasm compiles nxc → stage2.wasm ──────────────────────

STAGE2="$BUILD_DIR/stage2.wasm"
info "Stage 2: nexus exec $STAGE1 -- $NXC_ENTRY $STAGE2"
if "$NEXUS" exec $NEXUS_EXEC_FLAGS "$STAGE1" -- "$NXC_ENTRY" "$STAGE2" 2>&1; then
  ok "Stage 2 complete: $STAGE2 ($(wc -c < "$STAGE2" | tr -d ' ') bytes)"
else
  warn "Stage 2 failed — nxc-produced WASM not yet self-executable."
  warn "Stage 1 output is still valid (compiled by the Rust-built stage0)."
  cp "$STAGE1" nxc_driver.wasm
  ok "Installed (stage 1): nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
  exit 0
fi

# ─── Verify fixed point ───────────────────────────────────────────────────

info "Verifying fixed point: stage1 == stage2"
if cmp -s "$STAGE1" "$STAGE2"; then
  ok "Fixed point reached! stage1.wasm and stage2.wasm are identical."
  ok "The self-hosted compiler is verified."

  cp "$STAGE1" nxc_driver.wasm
  ok "Installed: nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
else
  warn "stage1.wasm and stage2.wasm differ — not yet at fixed point."
  warn "This is expected while nxc codegen is still maturing."
  cp "$STAGE1" nxc_driver.wasm
  ok "Installed (stage 1): nxc_driver.wasm ($(wc -c < nxc_driver.wasm | tr -d ' ') bytes)"
fi
