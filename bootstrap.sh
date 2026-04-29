#!/usr/bin/env bash
#
# bootstrap.sh — Multi-stage bootstrap for the Nexus self-hosted compiler (src/)
#
# Stage 0: Rust compiler (nexus build) compiles src/driver.nx → stage0.wasm
# Stage 1: Run stage0.wasm (nexus exec) to compile src/driver.nx → stage1.wasm
# Stage 2: Run stage1.wasm (nexus exec) to compile src/driver.nx → stage2.wasm
# Verify:  stage1.wasm == stage2.wasm (fixed point)
#
# Usage: ./bootstrap.sh [--ci]
#   --ci    Strict mode for CI: fail on stage2 failure or non-identical output
#
set -euo pipefail

for arg in "$@"; do
  case "$arg" in
    --ci) ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

NEXUS="${NEXUS:-./bootstrap/target/release/nexus}"
NEXUS_ENTRY="src/driver.nx"
WASMTIME="${WASMTIME:-wasmtime}"
# shellcheck disable=SC2054  # commas inside -W and -S are wasmtime delimiters, not array separators
WASMTIME_FLAGS_COMPONENT=(
  -W tail-call=y,exceptions=y,function-references=y,stack-switching=y,component-model=y,max-memory-size=8589934592
  -S http,inherit-network
  --dir=. --dir="${TMPDIR:-/tmp}"
)
# shellcheck disable=SC2054
WASMTIME_FLAGS_CORE=(
  -W tail-call=y,exceptions=y,function-references=y,stack-switching=y,max-memory-size=8589934592
  --dir=. --dir="${TMPDIR:-/tmp}"
)

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
info "Building Nexus compiler (cargo build --release --manifest-path bootstrap/Cargo.toml)..."
cargo build --release --manifest-path bootstrap/Cargo.toml
[[ -x "$NEXUS" ]] || fail "Nexus compiler not found at $NEXUS"
info "Using Nexus compiler: $NEXUS (commit ${CURRENT_COMMIT:0:7})"

# ─── Stage 0: Rust compiler → stage0.wasm ─────────────────────────────────
# Reuse `nexus build` auto-cache (target/nexus/nexus.wasm) only when both:
#   (a) .commit stamp matches HEAD, and
#   (b) the working tree has no uncommitted changes in src/ or stdlib/.
# A plain commit match is unsafe: `nexus build` reads the working tree, so
# any uncommitted edit poisons the cache and subsequent matching-commit
# runs silently reuse the wrong stage0 (stale cache-by-wrong-key bug).
# When the cache is suspect, rebuild from the Rust compiler directly.

STAGE0="$BUILD_DIR/stage0.wasm"
NEXUS_CACHE="target/nexus/nexus.wasm"
NEXUS_CACHE_COMMIT="target/nexus/.commit"

nexus_cache_valid() {
  [[ -f "$NEXUS_CACHE" ]] || return 1
  [[ -f "$NEXUS_CACHE_COMMIT" ]] || return 1
  [[ "$(cat "$NEXUS_CACHE_COMMIT")" == "$CURRENT_COMMIT" ]] || return 1
  git diff --quiet HEAD -- src/ stdlib/ nxlib/ 2>/dev/null || return 1
  return 0
}

if nexus_cache_valid; then
  info "Stage 0: reusing cached nexus.wasm (commit ${CURRENT_COMMIT:0:7})"
  cp "$NEXUS_CACHE" "$STAGE0"
else
  info "Stage 0: nexus build $NEXUS_ENTRY → $STAGE0"
  "$NEXUS" build "$NEXUS_ENTRY" -o "$STAGE0"
  mkdir -p "$(dirname "$NEXUS_CACHE_COMMIT")"
  if git diff --quiet HEAD -- src/ stdlib/ nxlib/ 2>/dev/null; then
    echo "$CURRENT_COMMIT" > "$NEXUS_CACHE_COMMIT"
  else
    rm -f "$NEXUS_CACHE_COMMIT"
  fi
fi
ok "Stage 0 complete: $STAGE0 ($(wc -c < "$STAGE0" | tr -d ' ') bytes)"

# ─── Stage 1: stage0.wasm compiles src → stage1.wasm ──────────────────────
# Stage0 may be a stub (no merge). If stage1 is core WASM with unresolved imports,
# compose it with stdlib. Once stage1 has the merge code, stage2+ are self-contained.

STAGE1_RAW="$BUILD_DIR/stage1_raw.wasm"
STAGE1="$BUILD_DIR/stage1.wasm"
info "Stage 1: wasmtime run $STAGE0 $NEXUS_ENTRY $STAGE1_RAW"
"$WASMTIME" run "${WASMTIME_FLAGS_COMPONENT[@]}" "$STAGE0" "$NEXUS_ENTRY" --verbose "$STAGE1_RAW"

# Check if stage1 is self-contained (no nexus:std imports) or needs compose
if wasm-tools print "$STAGE1_RAW" 2>/dev/null | grep -q 'import "nexus:std/'; then
  info "Stage 1 has unresolved stdlib imports — composing..."
  "$NEXUS" compose "$STAGE1_RAW" -o "$STAGE1"
  ok "Stage 1 composed: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"
  STAGE1_FLAGS=("${WASMTIME_FLAGS_COMPONENT[@]}")
else
  cp "$STAGE1_RAW" "$STAGE1"
  ok "Stage 1 self-contained: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"
  STAGE1_FLAGS=("${WASMTIME_FLAGS_CORE[@]}")
fi

# ─── Stage 2: stage1.wasm compiles src → stage2.wasm ──────────────────────

STAGE2="$BUILD_DIR/stage2.wasm"
info "Stage 2: wasmtime run $STAGE1 $NEXUS_ENTRY $STAGE2"
if ! "$WASMTIME" run "${STAGE1_FLAGS[@]}" "$STAGE1" "$NEXUS_ENTRY" "$STAGE2" 2>&1; then
  fail "Stage 2 failed — src-produced WASM is not self-executable."
fi
ok "Stage 2 complete: $STAGE2 ($(wc -c < "$STAGE2" | tr -d ' ') bytes)"

# Validate stage2 as a well-formed wasm module. wasmtime run exit 0 only means
# stage1 ran to completion; it does not check the bytes stage1 emitted.
if ! wasm-tools validate "$STAGE2" 2>&1; then
  fail "Stage 2 failed validation — src produced an ill-formed wasm module."
fi
ok "Stage 2 passes wasm-tools validate"

# ─── Verify fixed point ───────────────────────────────────────────────────

info "Verifying fixed point: stage1 == stage2"
if ! cmp -s "$STAGE1" "$STAGE2"; then
  S1_SIZE=$(wc -c < "$STAGE1" | tr -d ' ')
  S2_SIZE=$(wc -c < "$STAGE2" | tr -d ' ')
  info "stage1: $S1_SIZE bytes, stage2: $S2_SIZE bytes"
  fail "Fixed point NOT reached — stage1.wasm and stage2.wasm differ."
fi
ok "Fixed point reached! stage1.wasm and stage2.wasm are identical."
ok "The self-hosted compiler is verified."

# ─── Install nexus.wasm and build polyglot launcher ──────────────────────

info "Installing nexus.wasm..."
cp "$STAGE1" nexus.wasm
ok "Installed nexus.wasm ($(wc -c < nexus.wasm | tr -d ' ') bytes)"

# ─── Stage L: build lsp.wasm ─────────────────────────────────────────────
#
# `src/lsp/main.nx` wires the LSP scaffold to the Nexus pipeline; running
# it produces a standalone wasm that speaks JSON-RPC on stdio. We compile
# it through the Rust-built `$NEXUS` binary (the same binary that produced
# stage 0). The self-hosted `nexus.wasm` cannot currently compile
# `src/lsp/main.nx`: the codegen pass raises E3001 "closure arity '2' not
# found" on the handler-vtable record, an issue tracked separately from
# this hw47 epic. Once that codegen path is fixed, this can switch to
# `wasmtime run "${STAGE1_FLAGS[@]}" nexus.wasm src/lsp/main.nx ...` for
# full self-hosting parity.

LSP_RAW="$BUILD_DIR/lsp_raw.wasm"
LSP_OUT="$BUILD_DIR/lsp.wasm"
info "Stage L: $NEXUS build src/lsp/main.nx → $LSP_RAW"
"$NEXUS" build src/lsp/main.nx -o "$LSP_RAW"

if wasm-tools print "$LSP_RAW" 2>/dev/null | grep -q 'import "nexus:std/'; then
  info "lsp.wasm has unresolved stdlib imports — composing..."
  "$NEXUS" compose "$LSP_RAW" -o "$LSP_OUT"
  ok "lsp.wasm composed: $LSP_OUT ($(wc -c < "$LSP_OUT" | tr -d ' ') bytes)"
else
  cp "$LSP_RAW" "$LSP_OUT"
  ok "lsp.wasm self-contained: $LSP_OUT ($(wc -c < "$LSP_OUT" | tr -d ' ') bytes)"
fi

info "Installing lsp.wasm..."
cp "$LSP_OUT" lsp.wasm
ok "Installed lsp.wasm ($(wc -c < lsp.wasm | tr -d ' ') bytes)"

# ─── Build polyglot launcher with both payloads embedded ────────────────
#
# The launcher format is:
#
#     header.sh + manifest line(s) + #__NEXUS_PAYLOAD_BEGIN__\n + payload bytes
#
# Manifest entries are `name:size`, one per line, between the
# `#__NEXUS_PAYLOAD_MANIFEST__` marker (last line of header.sh) and the
# `#__NEXUS_PAYLOAD_BEGIN__` marker. Payload bytes are concatenated in the
# manifest order: compiler first, then lsp.

info "Building polyglot launcher: header.sh + nexus.wasm + lsp.wasm → nexus"
COMPILER_SIZE=$(wc -c < nexus.wasm | tr -d ' ')
LSP_SIZE=$(wc -c < lsp.wasm | tr -d ' ')
{
  cat header.sh
  printf 'compiler:%s\n' "$COMPILER_SIZE"
  printf 'lsp:%s\n' "$LSP_SIZE"
  printf '#__NEXUS_PAYLOAD_BEGIN__\n'
  cat nexus.wasm
  cat lsp.wasm
} > nexus
chmod +x nexus
ok "Installed nexus polyglot ($(wc -c < nexus | tr -d ' ') bytes; compiler=$COMPILER_SIZE, lsp=$LSP_SIZE)"
