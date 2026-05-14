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
#   nexus lsp [args...]              -> run the embedded `lsp` payload
#   nexus test [args...]             -> host-shell-driven test loop (see below)
#   nexus run FILE.nx [-- ARGS...]   -> compile FILE.nx and exec the wasm here
#                                       (the in-wasm driver's `run` path returns
#                                       a Proc.exec stub diagnostic under
#                                       preview1, so we intercept it at the
#                                       shell layer for the same reason `test`
#                                       is shell-driven)
#   nexus <anything else>            -> run the embedded `compiler` payload
#                                       (build/typecheck/etc.)
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
RUN_MODE=0
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
      # compile. The in-Nexus `src/tools/test_runner.nx` stays as the source of
      # truth for the report format and is the path that will light up if
      # a future WASI preview gives us subprocess back.
      TEST_MODE=1
      shift
      ;;
    run)
      # `nexus run` — same preview1 constraint as test (Proc.exec is a -1
      # stub), so build + exec are driven from the host shell. The in-wasm
      # `src/driver.nx` still parses `run` and would Proc.exec wasmtime if
      # subprocess was available; under preview1 it emits the same
      # "use the polyglot launcher" hint as the test path.
      RUN_MODE=1
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
#
# Two manifest line shapes are recognized:
#   <name>:<size>             -> payload entry (recorded with computed offset)
#   sha:<name>:<sha256-hex>   -> hash of the payload bytes named <name>
# Entries with 3 fields where the first is literally `sha` are written as
# `__sha__ <name> <hex>` to the manifest file so the staleness check below
# can look them up without re-parsing.
MANIFEST_FILE=$(mktemp)
awk -v head_bytes="$HEAD_BYTES" '
  /^#__NEXUS_PAYLOAD_BEGIN__$/ { exit }
  in_manifest && /^#/ { next }
  in_manifest && NF > 0 {
    n = split($0, parts, ":")
    if (n == 2) {
      print parts[1] " " parts[2] " " offset
      offset += parts[2]
    } else if (n == 3 && parts[1] == "sha") {
      print "__sha__ " parts[2] " " parts[3]
    }
  }
  /^#__NEXUS_PAYLOAD_MANIFEST__$/ {
    in_manifest = 1
    offset = head_bytes + 0
  }
' "$0" > "$MANIFEST_FILE"

# ─── Stale-launcher check ────────────────────────────────────────────────
# Detect the common gotcha where `./nexus` was built by a prior bootstrap
# and now embeds older wasm bytes than the on-disk `nexus.wasm` (or
# `lsp.wasm`) sitting next to the launcher. This happens after `git
# checkout` or worktree creation: the tracked `nexus.wasm` advances with
# the source, but `./nexus` is gitignored and only rebuilt by
# `./bootstrap.sh`. Running the stale launcher silently against new
# source bytes wastes hours of debugging (see nexus-r9ga, nexus-xnob).
#
# We compare the sha256 embedded in the launcher manifest against the
# sha256 of the sibling file `<launcher-dir>/<name>.wasm`. Mismatch =>
# fail loud. The check is skipped when:
#   * the embedded sha is empty (launcher built without a sha tool), OR
#   * no sibling wasm exists (launcher installed standalone, e.g. via
#     `cp ./nexus /usr/local/bin/`), OR
#   * no local sha256 tool is available (we cannot verify), OR
#   * NEXUS_SKIP_STALE_CHECK is set non-empty (escape hatch).
if [ -z "${NEXUS_SKIP_STALE_CHECK:-}" ]; then
  EXPECTED_SHA=$(awk -v want="$PAYLOAD_NAME" '$1 == "__sha__" && $2 == want { print $3; exit }' "$MANIFEST_FILE")
  LAUNCHER_DIR=$(dirname -- "$0")
  case "$PAYLOAD_NAME" in
    compiler) SIBLING="$LAUNCHER_DIR/nexus.wasm" ;;
    lsp)      SIBLING="$LAUNCHER_DIR/lsp.wasm" ;;
    *)        SIBLING="" ;;
  esac
  if [ -n "$EXPECTED_SHA" ] && [ -n "$SIBLING" ] && [ -f "$SIBLING" ]; then
    if command -v sha256sum >/dev/null 2>&1; then
      ACTUAL_SHA=$(sha256sum "$SIBLING" | awk '{ print $1 }')
    elif command -v shasum >/dev/null 2>&1; then
      ACTUAL_SHA=$(shasum -a 256 "$SIBLING" | awk '{ print $1 }')
    else
      ACTUAL_SHA=""
    fi
    if [ -n "$ACTUAL_SHA" ] && [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
      echo "nexus: stale launcher detected." >&2
      echo "nexus:   embedded $PAYLOAD_NAME sha: $EXPECTED_SHA" >&2
      echo "nexus:   on-disk  $SIBLING sha: $ACTUAL_SHA" >&2
      echo "nexus: ./nexus was built by a prior bootstrap and no longer matches the" >&2
      echo "nexus: wasm sitting next to it. Re-bootstrap to refresh the launcher:" >&2
      echo "nexus:   ./bootstrap.sh" >&2
      echo "nexus: (or set NEXUS_SKIP_STALE_CHECK=1 to bypass this check)" >&2
      exit 3
    fi
  fi
fi

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

if [ "$RUN_MODE" = "1" ]; then
  # ─── nexus run: build + exec wasm under wasmtime ─────────────────────
  # Compile the input .nx to a temp wasm using the embedded compiler
  # payload, then exec wasmtime against the result with the user's args.
  # The `--` separator splits compile-time tokens from program argv.
  if [ "$#" -lt 1 ]; then
    echo "nexus run: missing input file" >&2
    echo "  usage: nexus run FILE.nx [-- ARGS...]" >&2
    exit 2
  fi
  RUN_INPUT=""
  RUN_PROG_ARGS=""
  HAVE_PROG_ARGS=0
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --)
        shift
        HAVE_PROG_ARGS=1
        # Remaining args go to the program. Quote so paths with spaces survive
        # the eventual `eval`-equivalent re-expansion below.
        while [ "$#" -gt 0 ]; do
          if [ -z "$RUN_PROG_ARGS" ]; then
            RUN_PROG_ARGS=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
            RUN_PROG_ARGS="'$RUN_PROG_ARGS'"
          else
            esc=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
            RUN_PROG_ARGS="$RUN_PROG_ARGS '$esc'"
          fi
          shift
        done
        ;;
      *)
        if [ -z "$RUN_INPUT" ]; then
          RUN_INPUT="$1"
        else
          echo "nexus run: unexpected argument: $1" >&2
          echo "  (forward program args after a literal '--')" >&2
          exit 2
        fi
        shift
        ;;
    esac
  done
  if [ -z "$RUN_INPUT" ]; then
    echo "nexus run: missing input file" >&2
    echo "  usage: nexus run FILE.nx [-- ARGS...]" >&2
    exit 2
  fi

  RUN_WASM=$(mktemp "${TMPDIR:-/tmp}/nexus-run.XXXXXX.wasm")
  cleanup_run() {
    [ -n "${RUN_WASM:-}" ] && rm -f "$RUN_WASM"
    [ -n "$TMP" ] && rm -f "$TMP"
    [ -n "$MANIFEST_FILE" ] && rm -f "$MANIFEST_FILE"
  }
  trap cleanup_run EXIT INT TERM HUP

  # Compile via the compiler payload. Stash both streams of compile output so
  # the program's own stdio is the only thing the user sees on success; on
  # failure replay the captured stderr (where the compiler writes its
  # diagnostics).
  COMPILE_ELOG=$(mktemp "${TMPDIR:-/tmp}/nexus-run-compile.XXXXXX.log")
  # shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS is intentionally word-split.
  if ! wasmtime run \
      -W "$W_FLAGS" -S threads \
      --dir=. --dir="${TMPDIR:-/tmp}" \
      ${NEXUS_WASMTIME_ARGS:-} \
      "$TMP" build "$RUN_INPUT" -o "$RUN_WASM" --explain-capabilities none \
      >/dev/null 2>"$COMPILE_ELOG"; then
    cat "$COMPILE_ELOG" >&2
    rm -f "$COMPILE_ELOG"
    exit 1
  fi
  rm -f "$COMPILE_ELOG"

  # Run-phase: same flag set as the compiler payload. The cap-derived
  # `--wasi inherit-network` is omitted because we'd need to parse the
  # program's `require` row; wasmtime treats missing `--dir`/`--wasi`
  # as deny-by-default, so over-granting `--dir=.` matches what
  # `./nexus` already exposes to the compiler payload itself.
  if [ "$HAVE_PROG_ARGS" = "1" ]; then
    # shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS + RUN_PROG_ARGS both word-split.
    eval exec wasmtime run \
      -W \"\$W_FLAGS\" \
      -S threads \
      --dir=. --dir=\"\${TMPDIR:-/tmp}\" \
      \${NEXUS_WASMTIME_ARGS:-} \
      \"\$RUN_WASM\" "$RUN_PROG_ARGS"
  else
    # shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS is intentionally word-split.
    exec wasmtime run \
      -W "$W_FLAGS" -S threads \
      --dir=. --dir="${TMPDIR:-/tmp}" \
      ${NEXUS_WASMTIME_ARGS:-} \
      "$RUN_WASM"
  fi
fi

if [ "$TEST_MODE" = "1" ]; then
  # ─── nexus test: host-driven loop ─────────────────────────────────────
  # Discover *_test.nx under the given path, then for each fixture
  # `compile + run` in turn — each step is a wasmtime invocation, which
  # is the subprocess primitive the in-wasm runner cannot get to. Parallel
  # by default via xargs -P; `--sequential` collapses to one worker.
  TEST_PATH="tests"
  TEST_SEQ=0
  TEST_JUNIT=""
  TEST_COVERAGE=0
  TEST_LCOV=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --sequential) TEST_SEQ=1; shift ;;
      --junit) TEST_JUNIT="${2:-}"; shift 2 ;;
      --coverage) TEST_COVERAGE=1; shift ;;
      --lcov) TEST_LCOV="${2:-}"; shift 2 ;;
      --nexus-bin|--wasmtime-bin) shift 2 ;;  # legacy, accepted + ignored
      --*) shift ;;
      *) TEST_PATH="$1"; shift ;;
    esac
  done
  # --lcov implies --coverage; warn if --coverage absent so users notice.
  if [ -n "$TEST_LCOV" ] && [ "$TEST_COVERAGE" = "0" ]; then
    TEST_COVERAGE=1
  fi

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
  # When --coverage is on, pass `--coverage` through to each per-fixture build
  # so the compiler emits the `nx.coverage.functions` custom section and the
  # per-function atomic-counter prologues (nexus-gycd). The compiled wasms are
  # kept (not `rm -f`'d after run) so the post-run pass can dump the custom
  # section out of each.
  COMPILE_EXTRA_ARGS=""
  if [ "$TEST_COVERAGE" = "1" ]; then
    COMPILE_EXTRA_ARGS="--coverage"
  fi
  export TMP W_FLAGS TMPDIR_REAL RESULTS_DIR NEXUS_WASMTIME_ARGS COMPILE_EXTRA_ARGS TEST_COVERAGE
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
        "$TMP" build "$f" -o "$wasm" --explain-capabilities none ${COMPILE_EXTRA_ARGS:-} \
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
      if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
      exit 0
    fi
    t2=$(date +%s%N 2>/dev/null || date +%s000000000)
    ms=$(( (t2 - t1) / 1000000 ))
    printf "PASS\t%s\t-\t%s\t-\n" "$ms" "$f" \
      > "$RESULTS_DIR/result_$idx.tsv"
    printf "PASS  %s  (%sms)\n" "$f" "$ms" >&2
    if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
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

  # ─── Coverage report (nexus-gycd) ────────────────────────────────────────
  # For each test wasm, decode the `nx.coverage.functions` custom section to
  # list the instrumented functions. V1 prints function-level instrumentation
  # totals; per-function runtime call counts require an exit-time fd_write
  # dump path which is filed as nexus-gycd.1 (the static custom section is
  # the foundation that path will read alongside the runtime counter array).
  if [ "$TEST_COVERAGE" = "1" ]; then
    echo "" >&2
    echo "nexus test: coverage (nx.coverage.functions custom section)" >&2
    TOTAL_INSTRUMENTED=0
    TOTAL_FIXTURES_WITH_COV=0
    IDX=0
    for f in $FILES; do
      IDX=$((IDX + 1))
      wasm="$RESULTS_DIR/test_$IDX.wasm"
      [ -f "$wasm" ] || continue
      # wasm-tools dumps custom sections as escaped strings; we count by
      # parsing the first ULEB byte (count <= 127 fits in one byte, which
      # covers any realistic test fixture).
      cov_count=$(wasm-tools print "$wasm" 2>/dev/null \
        | awk '/@custom "nx.coverage.functions"/ {
            # Extract content between the first and last quote on the line.
            line = $0
            sub(/^[^"]*"nx.coverage.functions"[^"]*"/, "", line)
            # The first byte (escaped or raw) encodes the function count.
            if (substr(line, 1, 1) == "\\") {
              # \xx hex form
              hex = substr(line, 2, 2)
              print strtonum("0x" hex)
            } else {
              # raw byte (printable). Convert via printf - awk lacks ord().
              printf "%d\n", 0
            }
            exit
          }')
      cov_count="${cov_count:-0}"
      TOTAL_INSTRUMENTED=$((TOTAL_INSTRUMENTED + cov_count))
      if [ "$cov_count" -gt 0 ]; then
        TOTAL_FIXTURES_WITH_COV=$((TOTAL_FIXTURES_WITH_COV + 1))
      fi
      printf "  %s: %s function(s) instrumented\n" "$f" "$cov_count" >&2
    done
    echo "coverage summary: $TOTAL_INSTRUMENTED function(s) instrumented across $TOTAL_FIXTURES_WITH_COV fixture(s)" >&2
    echo "  (runtime call counts will be extracted in V2 — see issue nexus-gycd.1)" >&2

    if [ -n "$TEST_LCOV" ]; then
      # Emit a function-shaped lcov.info. Since V1 has no runtime counts,
      # every FNDA defaults to 0. Tools that consume lcov.info (codecov,
      # github coverage) will accept it and report 0% function coverage —
      # which is honest for now.
      {
        IDX=0
        for f in $FILES; do
          IDX=$((IDX + 1))
          wasm="$RESULTS_DIR/test_$IDX.wasm"
          [ -f "$wasm" ] || continue
          # wasm-tools 1.x prints the custom section payload as a quoted
          # backslash-escaped string. We extract function names + line by
          # walking the bytes; the simpler path is to dump the section to a
          # file and rely on wasm-tools to parse — but for V1 we keep this
          # in-shell. The format is: ULEB128 count, then for each function
          # ULEB128 id, ULEB128 line, length-prefixed UTF-8 name. We treat
          # every byte sequence after the first as opaque and emit a single
          # FN line for the source file with line=0 + name=fixture so the
          # consumer at least knows the fixture was instrumented.
          printf 'SF:%s\n' "$f"
          printf 'FN:0,_instrumented\n'
          printf 'FNDA:0,_instrumented\n'
          printf 'FNF:1\n'
          printf 'FNH:0\n'
          printf 'end_of_record\n'
        done
      } > "$TEST_LCOV"
      echo "nexus test: wrote lcov to $TEST_LCOV" >&2
    fi

    # Clean up the kept wasms now that the coverage extraction has run.
    rm -f "$RESULTS_DIR"/test_*.wasm
  fi

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
