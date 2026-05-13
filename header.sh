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
# the compiler payload. `nexus test` is special-cased — see below.
PAYLOAD_NAME="compiler"
TEST_MODE=0
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
    test)
      # `nexus test` cannot be implemented inside the wasm: WASI preview1
      # (forced by `-S threads`) has no subprocess-spawn API, so the
      # in-wasm runner cannot invoke the per-test `nexus build` /
      # `wasmtime run`. Drive the loop from the shell instead, reusing
      # this same launcher (the compiler payload) for every per-test
      # compile. The in-Nexus `src/test_runner.nx` stays as the source of
      # truth for the report format and is the path that will light up if
      # a future WASI preview gives us subprocess back.
      TEST_MODE=1
      shift
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

# Compose the wasmtime feature flag set. Both payloads are self-contained
# core WASM with preview1 imports satisfied by --dir mounts.
W_FLAGS="max-wasm-stack=${NEXUS_MAX_WASM_STACK:-67108864},tail-call=y,exceptions=y,function-references=y,stack-switching=y,threads=y,shared-memory=y"

if [ "$TEST_MODE" = "1" ]; then
  # ─── nexus test: host-driven loop ─────────────────────────────────────
  # Discover *_test.nx under the given path, then for each fixture
  # `compile + run` in turn — each step is a wasmtime invocation, which
  # is the subprocess primitive the in-wasm runner cannot get to. Parallel
  # by default via xargs -P; `--sequential` collapses to one worker.
  TEST_PATH="tests"
  TEST_SEQ=0
  TEST_JUNIT=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --sequential) TEST_SEQ=1; shift ;;
      --junit) TEST_JUNIT="${2:-}"; shift 2 ;;
      --nexus-bin|--wasmtime-bin) shift 2 ;;  # legacy, accepted + ignored
      --*) shift ;;
      *) TEST_PATH="$1"; shift ;;
    esac
  done

  TMPDIR_REAL="${TMPDIR:-/tmp}"
  if ! [ -e "$TEST_PATH" ]; then
    echo "nexus test: path not found: $TEST_PATH" >&2
    exit 1
  fi
  if [ -f "$TEST_PATH" ]; then
    case "$TEST_PATH" in *_test.nx) FILES="$TEST_PATH";; *) FILES="";; esac
  else
    FILES=$(find "$TEST_PATH" -type f -name '*_test.nx' 2>/dev/null | sort)
  fi
  N=$(printf '%s\n' "$FILES" | grep -c '^.' || true)
  if [ "$N" = "0" ]; then
    echo "nexus test: no *_test.nx files found under $TEST_PATH" >&2
    exit 1
  fi

  if [ "$TEST_SEQ" = "1" ]; then
    JOBS=1
    LABEL="sequential"
  else
    JOBS="${NEXUS_TEST_JOBS:-$( (nproc 2>/dev/null) || echo 4 )}"
    LABEL="parallel x$JOBS"
  fi
  echo "nexus test: discovered $N file(s) under $TEST_PATH [$LABEL]" >&2

  RESULTS_DIR=$(mktemp -d "$TMPDIR_REAL/nexus_test.XXXXXX")
  trap 'rm -rf "$RESULTS_DIR"; cleanup' EXIT INT TERM HUP

  # The per-fixture runner runs inline under xargs below — `xargs -L 1 sh -c
  # '...'` reads (idx, file) pairs from the plan file and invokes wasmtime
  # twice (compile, then run). Each worker prints its own PASS/FAIL line to
  # stderr as soon as it finishes (so the user sees parallel progress live)
  # and also writes a TSV record to RESULTS_DIR/result_$idx for the
  # plan-ordered final replay (summary + JUnit XML).
  export TMP W_FLAGS TMPDIR_REAL RESULTS_DIR NEXUS_WASMTIME_ARGS
  IDX=0
  PLAN_FILE="$RESULTS_DIR/plan.tsv"
  : > "$PLAN_FILE"
  for f in $FILES; do
    IDX=$((IDX + 1))
    printf '%d\t%s\n' "$IDX" "$f" >> "$PLAN_FILE"
  done

  # The inline runner uses positional args from `xargs -L 1`: $1=idx, $2=file.
  # The streamed PASS/FAIL line goes to fd 2 (stderr); fd 1 is reserved for
  # silent / further pipeline use.
  cat "$PLAN_FILE" | xargs -P "$JOBS" -L 1 -- /bin/sh -c '
    idx="$1"; f="$2"
    wasm="$RESULTS_DIR/test_$idx.wasm"
    elog="$RESULTS_DIR/err_$idx.log"
    t0=$(date +%s%N 2>/dev/null || date +%s000000000)
    if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
        ${NEXUS_WASMTIME_ARGS:-} \
        "$TMP" build "$f" -o "$wasm" --explain-capabilities none \
        >/dev/null 2>"$elog"; then
      t1=$(date +%s%N 2>/dev/null || date +%s000000000)
      ms=$(( (t1 - t0) / 1000000 ))
      tail_txt="$(tail -8 "$elog" 2>/dev/null)"
      printf "FAIL\t%s\tcompile\t%s\t%s\n" "$ms" "$f" "$tail_txt" \
        > "$RESULTS_DIR/result_$idx.tsv"
      printf "FAIL  %s  (%sms, compile)\n" "$f" "$ms" >&2
      rm -f "$wasm"
      exit 0
    fi
    t1=$(date +%s%N 2>/dev/null || date +%s000000000)
    if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
        ${NEXUS_WASMTIME_ARGS:-} \
        "$wasm" \
        >/dev/null 2>"$elog"; then
      t2=$(date +%s%N 2>/dev/null || date +%s000000000)
      ms=$(( (t2 - t1) / 1000000 ))
      tail_txt="$(tail -16 "$elog" 2>/dev/null)"
      printf "FAIL\t%s\trun\t%s\t%s\n" "$ms" "$f" "$tail_txt" \
        > "$RESULTS_DIR/result_$idx.tsv"
      printf "FAIL  %s  (%sms, run)\n" "$f" "$ms" >&2
      rm -f "$wasm"
      exit 0
    fi
    t2=$(date +%s%N 2>/dev/null || date +%s000000000)
    ms=$(( (t2 - t1) / 1000000 ))
    printf "PASS\t%s\t-\t%s\t-\n" "$ms" "$f" \
      > "$RESULTS_DIR/result_$idx.tsv"
    printf "PASS  %s  (%sms)\n" "$f" "$ms" >&2
    rm -f "$wasm"
  ' _

  # Replay results in plan order to accumulate totals. The PASS/FAIL lines
  # were already streamed live by the workers above; this loop is silent
  # except for the summary at the bottom.
  PASSED=0; FAILED=0; TOTAL_MS=0
  IDX=0
  for f in $FILES; do
    IDX=$((IDX + 1))
    rf="$RESULTS_DIR/result_$IDX.tsv"
    if [ ! -f "$rf" ]; then
      printf 'FAIL  %s  (0ms, missing-result)\n' "$f" >&2
      FAILED=$((FAILED + 1))
      continue
    fi
    status=$(awk -F'\t' '{ print $1; exit }' "$rf")
    ms=$(awk -F'\t' '{ print $2; exit }' "$rf")
    TOTAL_MS=$((TOTAL_MS + ms))
    if [ "$status" = "PASS" ]; then
      PASSED=$((PASSED + 1))
    else
      FAILED=$((FAILED + 1))
    fi
  done
  TOTAL=$((PASSED + FAILED))
  echo "" >&2
  echo "test summary: $PASSED passed, $FAILED failed, $TOTAL total (${TOTAL_MS}ms)" >&2

  if [ -n "$TEST_JUNIT" ]; then
    {
      echo '<?xml version="1.0" encoding="UTF-8"?>'
      printf '<testsuites>\n'
      printf '  <testsuite name="nexus" tests="%d" failures="%d" time="%d.%03d">\n' \
        "$TOTAL" "$FAILED" "$((TOTAL_MS / 1000))" "$((TOTAL_MS % 1000))"
      IDX=0
      for f in $FILES; do
        IDX=$((IDX + 1))
        rf="$RESULTS_DIR/result_$IDX.tsv"
        [ -f "$rf" ] || continue
        status=$(awk -F'\t' '{ print $1; exit }' "$rf")
        ms=$(awk -F'\t' '{ print $2; exit }' "$rf")
        stage=$(awk -F'\t' '{ print $3; exit }' "$rf")
        tail_txt=$(awk -F'\t' '{ for (i = 5; i < NF; i++) printf "%s\t", $i; print $NF }' "$rf")
        # XML-escape minimally.
        esc=$(printf '%s' "$tail_txt" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g')
        f_esc=$(printf '%s' "$f" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g')
        if [ "$status" = "PASS" ]; then
          printf '    <testcase classname="nexus" name="%s" time="%d.%03d"/>\n' \
            "$f_esc" "$((ms / 1000))" "$((ms % 1000))"
        else
          printf '    <testcase classname="nexus" name="%s" time="%d.%03d">\n' \
            "$f_esc" "$((ms / 1000))" "$((ms % 1000))"
          printf '      <failure message="%s failed" type="%s">%s</failure>\n' \
            "$stage" "$stage" "$esc"
          printf '    </testcase>\n'
        fi
      done
      printf '  </testsuite>\n'
      printf '</testsuites>\n'
    } > "$TEST_JUNIT"
    echo "nexus test: wrote JUnit XML to $TEST_JUNIT" >&2
  fi

  [ "$FAILED" = "0" ] && exit 0 || exit 1
fi

# shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS is intentionally word-split.
exec wasmtime run \
  -W "$W_FLAGS" \
  -S threads \
  --dir=. --dir="${TMPDIR:-/tmp}" \
  ${NEXUS_WASMTIME_ARGS:-} \
  "$TMP" "$@"
#__NEXUS_PAYLOAD_MANIFEST__
