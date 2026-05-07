#!/usr/bin/env bash
#
# bootstrap.sh — Multi-stage bootstrap for the Nexus self-hosted compiler (src/)
#
# Stage 0: Run committed ./nexus.wasm (seed) to compile src/driver.nx → stage0.wasm
# Stage 1: Run stage0.wasm to compile src/driver.nx → stage1.wasm
# Stage 2: Run stage1.wasm to compile src/driver.nx → stage2.wasm
# Verify:  stage1.wasm == stage2.wasm (fixed point)
#
# The committed ./nexus.wasm is the Stage 0 source of truth; this script
# no longer rebuilds the seed from Rust.
#
# Usage: ./bootstrap.sh [--ci]
#   --ci    Strict mode for CI: fail on stage2 failure or non-identical output
#
set -euo pipefail

CI_MODE=0
for arg in "$@"; do
  case "$arg" in
    --ci) CI_MODE=1 ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

NEXUS_SEED="${NEXUS_SEED:-./nexus.wasm}"
NEXUS_ENTRY="src/driver.nx"
WASMTIME="${WASMTIME:-wasmtime}"
# shellcheck disable=SC2054  # commas inside -W are wasmtime delimiters, not array separators
WASMTIME_FLAGS_CORE=(
  -W tail-call=y,exceptions=y,function-references=y,stack-switching=y,max-memory-size=8589934592,max-wasm-stack=33554432
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

# ─── Verify Stage 0 seed exists ──────────────────────────────────────────
[[ -f "$NEXUS_SEED" ]] || fail "Stage0 seed $NEXUS_SEED not found at repo root."

# ─── Temp directory ──────────────────────────────────────────────────────
BUILD_DIR="$(mktemp -d)"
cleanup() { rm -rf "$BUILD_DIR"; }
trap cleanup EXIT
info "Build directory: $BUILD_DIR"

CURRENT_COMMIT="$(git rev-parse HEAD)"
info "Using Stage 0 seed: $NEXUS_SEED (commit ${CURRENT_COMMIT:0:7})"

run_seed_compile() {
  # Run the committed Stage 0 seed to compile <input.nx> → <output.wasm>.
  local input="$1"
  local output="$2"
  "$WASMTIME" run "${WASMTIME_FLAGS_CORE[@]}" "$NEXUS_SEED" build "$input" -o "$output"
}

# ─── Stage 0: committed seed → stage0.wasm ───────────────────────────────

STAGE0="$BUILD_DIR/stage0.wasm"
info "Stage 0: wasmtime run $NEXUS_SEED $NEXUS_ENTRY → $STAGE0"
run_seed_compile "$NEXUS_ENTRY" "$STAGE0"
ok "Stage 0 complete: $STAGE0 ($(wc -c < "$STAGE0" | tr -d ' ') bytes)"

# ─── Stage 1: stage0.wasm compiles src → stage1.wasm ──────────────────────
# Stage0 may be a stub (no merge). If stage1 is core WASM with unresolved imports,
# compose it with stdlib. Once stage1 has the merge code, stage2+ are self-contained.

STAGE1_RAW="$BUILD_DIR/stage1_raw.wasm"
STAGE1="$BUILD_DIR/stage1.wasm"
info "Stage 1: wasmtime run $STAGE0 $NEXUS_ENTRY $STAGE1_RAW"
"$WASMTIME" run "${WASMTIME_FLAGS_CORE[@]}" "$STAGE0" build "$NEXUS_ENTRY" -o "$STAGE1_RAW"

# The committed seed produces self-contained core wasm; stage1 must too.
if wasm-tools print "$STAGE1_RAW" 2>/dev/null | grep -q 'import "nexus:std/'; then
  fail "Stage 1 emitted unresolved nexus:std imports. Self-hosted compose is not wired."
fi
cp "$STAGE1_RAW" "$STAGE1"
ok "Stage 1 self-contained: $STAGE1 ($(wc -c < "$STAGE1" | tr -d ' ') bytes)"

# ─── Stage 2: stage1.wasm compiles src → stage2.wasm ──────────────────────

STAGE2="$BUILD_DIR/stage2.wasm"
info "Stage 2: wasmtime run $STAGE1 $NEXUS_ENTRY $STAGE2"
if ! "$WASMTIME" run "${WASMTIME_FLAGS_CORE[@]}" "$STAGE1" build "$NEXUS_ENTRY" -o "$STAGE2" 2>&1; then
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

# ─── CI-only: committed seed must match freshly-built stage1.wasm ────────
# Enforces the seed-policy: any src/** or nxlib/** change must come with a
# regenerated nexus.wasm. Skipped for local dev so editing src/ doesn't
# immediately error.
if [[ "$CI_MODE" -eq 1 ]]; then
  # The committed ./nexus.wasm is the artifact under policy, regardless of
  # what $NEXUS_SEED is overridden to (e.g. negative tests pointing elsewhere).
  info "Verifying committed seed: ./nexus.wasm == stage1.wasm"
  if [[ ! -f ./nexus.wasm ]]; then
    fail "./nexus.wasm not present at repo root; cannot verify seed match."
  fi
  if ! cmp -s ./nexus.wasm "$STAGE1"; then
    SEED_SIZE=$(wc -c < ./nexus.wasm | tr -d ' ')
    S1_SIZE=$(wc -c < "$STAGE1" | tr -d ' ')
    info "./nexus.wasm: $SEED_SIZE bytes, stage1.wasm: $S1_SIZE bytes"
    fail "Seed mismatch: ./nexus.wasm differs from stage1.wasm.
[bootstrap] The committed seed is stale. Regenerate it:
[bootstrap]   ./bootstrap.sh && cp <build_dir>/stage1.wasm nexus.wasm
[bootstrap] (the bootstrap installs nexus.wasm for you on success — just commit it)
[bootstrap] Then commit nexus.wasm alongside the source change."
  fi
  ok "Committed seed matches stage1.wasm."
fi

# ─── Install nexus.wasm and build polyglot launcher ──────────────────────

info "Installing nexus.wasm..."
cp "$STAGE1" nexus.wasm
ok "Installed nexus.wasm ($(wc -c < nexus.wasm | tr -d ' ') bytes)"

# ─── Stage L: build lsp.wasm ─────────────────────────────────────────────
#
# `src/lsp/main.nx` wires the LSP scaffold to the Nexus pipeline; running
# it produces a standalone wasm that speaks JSON-RPC on stdio. We compile
# it through the self-hosted `nexus.wasm` produced by Stage 1, mirroring
# the Stage 1/2 self-host invocation. The handler-vtable record literal
# in `nxlib/lsp/server.nx` stores arity-2 top-level fns as record fields;
# the MIR pass lifts each into a `__closure_wrap_<target>` thunk so the
# closure machinery (closure_table + arity-N type dedup) carries them.

LSP_RAW="$BUILD_DIR/lsp_raw.wasm"
LSP_OUT="$BUILD_DIR/lsp.wasm"
info "Stage L: wasmtime run nexus.wasm src/lsp/main.nx → $LSP_RAW"
"$WASMTIME" run "${WASMTIME_FLAGS_CORE[@]}" nexus.wasm build src/lsp/main.nx -o "$LSP_RAW"

if wasm-tools print "$LSP_RAW" 2>/dev/null | grep -q 'import "nexus:std/'; then
  fail "lsp.wasm emitted unresolved nexus:std imports. Self-hosted compose is not wired."
fi
cp "$LSP_RAW" "$LSP_OUT"
ok "lsp.wasm self-contained: $LSP_OUT ($(wc -c < "$LSP_OUT" | tr -d ' ') bytes)"

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
