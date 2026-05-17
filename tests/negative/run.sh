#!/bin/sh
# tests/negative/run.sh — drive the negative-fixture corpus.
#
# Two flavors of negative fixture are supported. Each is selected by the
# header directive present in the file.
#
# 1. Compile-time diagnostic (default flavor)
#
#      // expect-fail: <code>        — required, e.g. E2007
#      // expect-msg:  <substring>   — optional, repeatable; every listed
#                                      substring must appear in the diagnostic
#
#    The driver runs `nexus build` and asserts:
#      a. The compile *fails* (nonzero exit).
#      b. The reported diagnostic carries the declared `expect-fail` code.
#      c. Every `expect-msg` substring appears in the diagnostic body.
#
# 2. Runtime exception (new in fl9t.1)
#
#      // expect-runtime-throw: <substring>  — required header (substring
#                                              may be empty to assert
#                                              only "non-zero exit").
#                                              Repeatable; every listed
#                                              substring must appear in
#                                              the combined run output.
#      // expect-msg:           <substring>  — optional, alias of the
#                                              above (kept so a fixture
#                                              with multiple substring
#                                              expectations stays
#                                              readable).
#
#    The driver runs `nexus build` (which must *succeed*), then `nexus
#    run` and asserts:
#      a. The compile succeeds.
#      b. The run exits with a non-zero status.
#      c. Every `expect-runtime-throw` and `expect-msg` substring appears
#         in the combined run stdout+stderr.
#
#    Runtime-throw fixtures are how stdlib modules whose error path is
#    surfaced via `raise <Exn>` (Option.unwrap, math.mod_i64-by-zero,
#    json.parse on malformed input, regexp.compile on a bad pattern,
#    bytebuffer.read_binary_file on a missing path, ...) get their
#    per-module negative coverage. wasmtime's "thrown Wasm exception"
#    line does not carry the exception constructor, so substring
#    assertions usually anchor on text the fixture itself prints
#    before raising (or on the wasmtime header itself).
#
# Exit codes: 0 = all fixtures behaved as advertised; 1 = at least one
# fixture failed an assertion. The launcher walks the directory listed
# as $1 (default: the dir holding this script).
#
# Usage:
#   ./tests/negative/run.sh                # walks tests/negative/
#   ./tests/negative/run.sh path/to/fixtures
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

# Helper: parse the contiguous header-comment block at the top of a
# fixture and emit lines of the form `KIND:VALUE` where KIND is one of
# `fail`, `runtime-throw`, `msg`. Stops at the first non-comment,
# non-blank line so fixtures can interleave body code right after the
# directive header.
parse_headers() {
  awk '
    /^[[:space:]]*\/\/[[:space:]]*expect-fail:/ {
      sub(/^[[:space:]]*\/\/[[:space:]]*expect-fail:[[:space:]]*/, "")
      print "fail:" $0
      next
    }
    /^[[:space:]]*\/\/[[:space:]]*expect-runtime-throw:/ {
      sub(/^[[:space:]]*\/\/[[:space:]]*expect-runtime-throw:[[:space:]]*/, "")
      print "runtime-throw:" $0
      next
    }
    /^[[:space:]]*\/\/[[:space:]]*expect-msg:/ {
      sub(/^[[:space:]]*\/\/[[:space:]]*expect-msg:[[:space:]]*/, "")
      print "msg:" $0
      next
    }
    /^[[:space:]]*$/ { next }
    /^[[:space:]]*\/\// { next }
    { exit }
  ' "$1"
}

for f in $FILES; do
  TOTAL=$((TOTAL + 1))

  HEADERS=$(parse_headers "$f")
  CODE=$(printf '%s\n' "$HEADERS" | awk -F: '/^fail:/ { sub(/^fail:/, ""); print; exit }')
  RUNTIME_PRESENT=$(printf '%s\n' "$HEADERS" | awk -F: '/^runtime-throw:/ { print "1"; exit }')
  RUNTIME_MSGS=$(printf '%s\n' "$HEADERS" | awk '/^runtime-throw:/ { sub(/^runtime-throw:/, ""); print }')
  MSGS=$(printf '%s\n' "$HEADERS" | awk '/^msg:/ { sub(/^msg:/, ""); print }')

  if [ -z "$CODE" ] && [ -z "$RUNTIME_PRESENT" ]; then
    echo "FAIL  $f  (missing 'expect-fail:' or 'expect-runtime-throw:' header)" >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  if [ -n "$CODE" ] && [ -n "$RUNTIME_PRESENT" ]; then
    echo "FAIL  $f  (cannot mix 'expect-fail:' and 'expect-runtime-throw:')" >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  # ── flavor A: compile-time diagnostic ────────────────────────────────
  if [ -n "$CODE" ]; then
    # Capture both streams; on success or failure the compiler writes its
    # diagnostic to stderr. Redirect to one file so we can grep regardless
    # of which side the implementation currently uses.
    OUT=$("$NEXUS_BIN" build "$f" -o "$WASM_OUT" --explain-capabilities none 2>&1 || true)
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

    if ! printf '%s\n' "$OUT" | grep -q -- "\\[$CODE\\]"; then
      echo "FAIL  $f  (expected $CODE not found in diagnostic)" >&2
      printf '%s\n' "$OUT" | sed 's/^/        /' >&2
      FAIL=$((FAIL + 1))
      continue
    fi

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
    continue
  fi

  # ── flavor B: runtime exception ──────────────────────────────────────
  # Build must succeed; the failure is expected only at run time.
  if ! "$NEXUS_BIN" build "$f" -o "$WASM_OUT" --explain-capabilities none >/dev/null 2>&1; then
    echo "FAIL  $f  (compile failed — runtime-throw fixture must build cleanly)" >&2
    "$NEXUS_BIN" build "$f" -o "$WASM_OUT" --explain-capabilities none 2>&1 | sed 's/^/        /' >&2 || true
    FAIL=$((FAIL + 1))
    continue
  fi

  RUN_OUT=$("$NEXUS_BIN" run "$f" 2>&1 || true)
  if "$NEXUS_BIN" run "$f" >/dev/null 2>&1; then
    RUN_OK=1
  else
    RUN_OK=0
  fi

  if [ "$RUN_OK" = "1" ]; then
    echo "FAIL  $f  (run exited 0 — expected a runtime exception)" >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  # Substring assertions: union of runtime-throw values (excluding empty)
  # and any expect-msg lines. Each non-empty substring must appear in the
  # combined run output.
  MISS=""
  IFS_BAK=$IFS
  IFS='
'
  for msg in $RUNTIME_MSGS; do
    [ -z "$msg" ] && continue
    if ! printf '%s\n' "$RUN_OUT" | grep -q -F -- "$msg"; then
      MISS="$MISS|$msg"
    fi
  done
  for msg in $MSGS; do
    [ -z "$msg" ] && continue
    if ! printf '%s\n' "$RUN_OUT" | grep -q -F -- "$msg"; then
      MISS="$MISS|$msg"
    fi
  done
  IFS=$IFS_BAK

  if [ -n "$MISS" ]; then
    echo "FAIL  $f  (missing substring(s) in run output: ${MISS#|})" >&2
    printf '%s\n' "$RUN_OUT" | sed 's/^/        /' >&2
    FAIL=$((FAIL + 1))
    continue
  fi

  echo "PASS  $f  (runtime-throw)" >&2
  PASS=$((PASS + 1))
done

echo "" >&2
echo "negative-runner: $PASS passed, $FAIL failed, $TOTAL total" >&2
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
