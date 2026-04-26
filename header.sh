#!/bin/sh
# nexus — self-extracting POSIX sh + wasm polyglot launcher.
# Header is concatenated with nexus.wasm at build time. At runtime we extract
# the embedded payload to a tempfile and exec wasmtime against it.
#
# Environment overrides:
#   NEXUS_MAX_WASM_STACK   Bytes for -W max-wasm-stack=. Default: 67108864 (64 MiB).
#   NEXUS_WASMTIME_ARGS    Extra args appended to `wasmtime run` (whitespace-split).
set -eu

TMP=$(mktemp) || exit 1
trap 'rm -f "$TMP"' EXIT INT TERM HUP

# Locate the payload marker line, count bytes up to and including it,
# then stream everything after that byte to the tempfile. The marker is a
# comment line so the shell parser never treats it as a command.
LINE=$(awk '/^#__NEXUS_WASM_PAYLOAD__$/ { print NR; exit }' "$0")
HEAD_BYTES=$(head -n "$LINE" "$0" | wc -c | tr -d ' ')
tail -c +$((HEAD_BYTES + 1)) "$0" > "$TMP"

# shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS is intentionally word-split.
exec wasmtime run \
  -W "max-wasm-stack=${NEXUS_MAX_WASM_STACK:-67108864},tail-call=y,exceptions=y,function-references=y,stack-switching=y" \
  --dir=. --dir="${TMPDIR:-/tmp}" \
  ${NEXUS_WASMTIME_ARGS:-} \
  "$TMP" "$@"
#__NEXUS_WASM_PAYLOAD__
