#!/bin/sh
# nexus — self-extracting POSIX sh + wasm polyglot launcher.
#
# Header is concatenated with a payload at build time:
#
#   header.sh + manifest line(s) + #__NEXUS_PAYLOAD_BEGIN__\n + bytes
#
# The manifest format is one or more `name:size` lines (decimal byte size),
# enclosed between `#__NEXUS_PAYLOAD_MANIFEST__` and `#__NEXUS_PAYLOAD_BEGIN__`
# markers. Both markers are shell comments so the parser never reads them as
# commands. After the BEGIN marker the wasm bytes are concatenated in the
# manifest order.
#
# Subcommand routing:
#   nexus lsp [args...]    -> run the embedded `lsp` payload
#   nexus <anything else>  -> run the embedded `compiler` payload (build/exec/etc.)
#
# When no `lsp` payload is embedded (unbundled compiler-only build), `nexus lsp`
# fails with a clear diagnostic rather than executing the compiler with
# unexpected args.
#
# Environment overrides:
#   NEXUS_MAX_WASM_STACK   Bytes for -W max-wasm-stack=. Default: 67108864 (64 MiB).
#   NEXUS_WASMTIME_ARGS    Extra args appended to `wasmtime run` (whitespace-split).
set -eu

TMP=""
MANIFEST_FILE=""
cleanup() {
  [ -n "$TMP" ] && rm -f "$TMP"
  [ -n "$MANIFEST_FILE" ] && rm -f "$MANIFEST_FILE"
}
trap cleanup EXIT INT TERM HUP

# Determine which payload the user asked for. `nexus lsp` selects the
# language-server payload; everything else (build / exec / no-arg) selects
# the compiler payload.
PAYLOAD_NAME="compiler"
if [ "$#" -gt 0 ]; then
  case "$1" in
    lsp)
      PAYLOAD_NAME="lsp"
      shift
      # The `--stdio` argument is the conventional LSP transport selector.
      # We always run on stdio (no other transport implemented), so consume
      # and discard it if present.
      if [ "$#" -gt 0 ] && [ "$1" = "--stdio" ]; then
        shift
      fi
      ;;
  esac
fi

# Locate the byte offset of the first payload byte (one past the
# BEGIN marker line). `head -n LINE | wc -c` gives the cumulative bytes
# through the marker line; payload starts at byte offset HEAD_BYTES + 1.
BEGIN_LINE=$(awk '/^#__NEXUS_PAYLOAD_BEGIN__$/ { print NR; exit }' "$0")
if [ -z "${BEGIN_LINE:-}" ]; then
  echo "nexus: launcher is missing the payload BEGIN marker (corrupt build)" >&2
  exit 2
fi
HEAD_BYTES=$(head -n "$BEGIN_LINE" "$0" | wc -c | tr -d ' ')

# Walk the manifest block: lines between `#__NEXUS_PAYLOAD_MANIFEST__` and
# `#__NEXUS_PAYLOAD_BEGIN__` (exclusive). Compute each entry's offset as
# HEAD_BYTES plus the cumulative size of preceding entries. The result is
# written to a temp file because `while` runs in a subshell and cannot
# export variables back.
MANIFEST_FILE=$(mktemp)
awk -v head_bytes="$HEAD_BYTES" '
  /^#__NEXUS_PAYLOAD_BEGIN__$/ { exit }
  in_manifest && /^#/ { next }
  in_manifest && NF > 0 {
    n = split($0, parts, ":")
    if (n == 2) {
      print parts[1] " " parts[2] " " offset
      offset += parts[2]
    }
  }
  /^#__NEXUS_PAYLOAD_MANIFEST__$/ {
    in_manifest = 1
    offset = head_bytes + 0
  }
' "$0" > "$MANIFEST_FILE"

# Locate the requested entry by name.
ENTRY=$(awk -v want="$PAYLOAD_NAME" '$1 == want { print $2, $3; exit }' "$MANIFEST_FILE")
if [ -z "$ENTRY" ]; then
  AVAILABLE=$(awk '{ print $1 }' "$MANIFEST_FILE" | tr '\n' ' ')
  echo "nexus: no embedded payload named '$PAYLOAD_NAME'" >&2
  echo "nexus: this build only contains: $AVAILABLE" >&2
  exit 2
fi

SIZE=$(echo "$ENTRY" | awk '{ print $1 }')
OFFSET=$(echo "$ENTRY" | awk '{ print $2 }')

TMP=$(mktemp) || exit 1
# Extract: skip OFFSET bytes from the head of $0, then take SIZE bytes.
tail -c +$((OFFSET + 1)) "$0" | head -c "$SIZE" > "$TMP"

# Compose the wasmtime feature flag set. The compiler payload is core
# WASM (preview1 imports satisfied by --dir mounts). The lsp payload is
# a WASI Preview2 component that pulls in wasi:http/* via the stdlib
# bundle even when network isn't used at runtime, so the component-model
# host needs to be available; we declare component-model and pass
# `-S http,inherit-network` to satisfy the http instance imports.
W_FLAGS="max-wasm-stack=${NEXUS_MAX_WASM_STACK:-67108864},tail-call=y,exceptions=y,function-references=y,stack-switching=y"
if [ "$PAYLOAD_NAME" = "lsp" ]; then
  W_FLAGS="$W_FLAGS,component-model=y,max-memory-size=8589934592"
  S_FLAGS="-S http,inherit-network"
else
  S_FLAGS=""
fi

# shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS / S_FLAGS are intentionally word-split.
exec wasmtime run \
  -W "$W_FLAGS" \
  $S_FLAGS \
  --dir=. --dir="${TMPDIR:-/tmp}" \
  ${NEXUS_WASMTIME_ARGS:-} \
  "$TMP" "$@"
#__NEXUS_PAYLOAD_MANIFEST__
