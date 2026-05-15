#!/bin/sh
# examples/negative/run.sh — drive the negative-example corpus.
#
# Each `examples/negative/*.nx` fixture begins with one or more header
# directives:
#
#   // expect-fail: <code>        — required, e.g. E2007
#   // expect-msg:  <substring>   — optional, repeatable; every listed
#                                   substring must appear in the diagnostic
#
# For each fixture this driver runs `nexus build` (because some checks
# only fire at the build pass, not `typecheck`) and asserts:
#
#   1. The compile *fails* (nonzero exit).
#   2. The reported diagnostic carries the declared `expect-fail` code.
#   3. Every `expect-msg` substring appears in the diagnostic body.
#
# Exit codes: 0 = all fixtures behaved as advertised; 1 = at least one
# fixture failed an assertion (either it built when it shouldn't, or it
# raised the wrong diagnostic). The launcher walks the directory listed
# as $1 (default: the dir holding this script).
#
# Usage:
#   ./examples/negative/run.sh                # walks examples/negative/
#   ./examples/negative/run.sh path/to/fixtures
#
# Env overrides:
#   NEXUS_BIN      — path to the nexus launcher (default: ./nexus)
#   NEXUS_SKIP_STALE_CHECK=1 — set automatically (worktree-friendly)
set -eu

DIR="${1:-$(dirname "$0")}"
NEXUS_BIN="${NEXUS_BIN:-./nexus}"
export NEXUS_SKIP_STALE_CHECK=1

if [ ! -x "$NEXUS_BIN" ]; then
  echo "negative-runner: launcher not found or not executable: $NEXUS_BIN" >&2
  exit 2
fi
if [ ! -d "$DIR" ]; then
  echo "negative-runner: directory not found: $DIR" >&2
  exit 2
fi

FILES=$(find "$DIR" -type f -name '*.nx' 2>/dev/null | sort)
if [ -z "$FILES" ]; then
  echo "negative-runner: no .nx fixtures found under $DIR" >&2
  exit 2
fi

PASS=0
FAIL=0
TOTAL=0
WASM_OUT=$(mktemp "${TMPDIR:-/tmp}/neg.XXXXXX.wasm")
trap 'rm -f "$WASM_OUT"' EXIT INT TERM HUP

for f in $FILES; do
  TOTAL=$((TOTAL + 1))

  # Parse header directives. Tolerant of leading whitespace; stops at the
  # first non-comment, non-blank line so headers can be followed by code.
  CODE=$(awk '
    /^[[:space:]]*\/\/[[:space:]]*expect-fail:/ {
      sub(/^[[:space:]]*\/\/[[:space:]]*expect-fail:[[:space:]]*/, "")
      print
      exit
    }
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*\/\// { next }
    { exit }
  ' "$f")
  MSGS=$(awk '
    /^[[:space:]]*\/\/[[:space:]]*expect-msg:/ {
      sub(/^[[:space:]]*\/\/[[:space:]]*expect-msg:[[:space:]]*/, "")
      print
      next
    }
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*\/\// { next }
    { exit }
  ' "$f")

  if [ -z "$CODE" ]; then
    echo "FAIL  $f  (missing 'expect-fail:' header)" >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  # Capture both streams; on success or failure the compiler writes its
  # diagnostic to stderr (success = empty). Redirect both to one file so
  # we can grep for the code regardless of which side the implementation
  # currently uses.
  OUT=$("$NEXUS_BIN" build "$f" -o "$WASM_OUT" --explain-capabilities none 2>&1 || true)
  STATUS=$?
  # Re-run for the actual exit code (the above is always 0 because of `|| true`).
  if "$NEXUS_BIN" build "$f" -o "$WASM_OUT" --explain-capabilities none >/dev/null 2>&1; then
    BUILD_OK=1
  else
    BUILD_OK=0
  fi

  if [ "$BUILD_OK" = "1" ]; then
    echo "FAIL  $f  (compiled successfully — expected diagnostic $CODE)" >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  # Assert the declared code is present.
  if ! printf '%s\n' "$OUT" | grep -q -- "\\[$CODE\\]"; then
    echo "FAIL  $f  (expected $CODE not found in diagnostic)" >&2
    printf '%s\n' "$OUT" | sed 's/^/        /' >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  # Assert every expect-msg substring is present.
  MISS=""
  IFS_BAK=$IFS
  IFS='
'
  for msg in $MSGS; do
    [ -z "$msg" ] && continue
    if ! printf '%s\n' "$OUT" | grep -q -F -- "$msg"; then
      MISS="$MISS|$msg"
    fi
  done
  IFS=$IFS_BAK

  if [ -n "$MISS" ]; then
    echo "FAIL  $f  (missing substring(s): ${MISS#|})" >&2
    printf '%s\n' "$OUT" | sed 's/^/        /' >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  echo "PASS  $f  ($CODE)" >&2
  PASS=$((PASS + 1))
done

echo "" >&2
echo "negative-runner: $PASS passed, $FAIL failed, $TOTAL total" >&2
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
