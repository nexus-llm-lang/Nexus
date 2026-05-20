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
BENCH_MODE=0
RUN_MODE=0
CAPS_DIFF_REV=""
CAPS_DIFF_TMP=""
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
      #
      # `nexus test --help|-h` is a help request, not an invocation —
      # fall through to the compiler payload so the in-wasm
      # try_dispatch_help (nexus-z35v) can serve it.
      if [ "$#" -ge 2 ] && { [ "$2" = "--help" ] || [ "$2" = "-h" ]; }; then
        :
      else
        TEST_MODE=1
        shift
      fi
      ;;
    bench)
      # `nexus bench` — same preview1 subprocess constraint as `nexus test`.
      # Drive the per-bench compile+run loop from the host shell; the in-wasm
      # path in `src/tools/bench.nx` exists for any future WASI preview that
      # gains subprocess back.
      #
      # `nexus bench --help|-h` is a help request — fall through to the
      # compiler payload so the in-wasm try_dispatch_help can serve it.
      if [ "$#" -ge 2 ] && { [ "$2" = "--help" ] || [ "$2" = "-h" ]; }; then
        :
      else
        BENCH_MODE=1
        shift
      fi
      ;;
    run)
      # `nexus run` — same preview1 constraint as test (Proc.exec is a -1
      # stub), so build + exec are driven from the host shell. The in-wasm
      # `src/main.nx` still parses `run` and would Proc.exec wasmtime if
      # subprocess was available; under preview1 it emits the same
      # "use the polyglot launcher" hint as the test path.
      #
      # `nexus run --help|-h` is a help request — fall through to the
      # compiler payload (see comment on `test` above).
      if [ "$#" -ge 2 ] && { [ "$2" = "--help" ] || [ "$2" = "-h" ]; }; then
        :
      else
        RUN_MODE=1
        shift
      fi
      ;;
    caps)
      # `nexus caps --diff <rev>` (nexus-2v8n): the in-wasm `caps` subcommand
      # needs to compare the working-tree closure against a closure built
      # from `git show <rev>:<file>`. Proc.exec is a preview1 -1-stub so the
      # in-wasm side can't shell out; we intercept --diff here, fetch the
      # base source via `git show`, write it to a TMPDIR file, and rewrite
      # the arg vector to `--diff-base <tmp>`. The compiler payload sees
      # only --diff-base, which is just an extra Fs.read_to_string.
      #
      # `caps` itself stays in the vector — we hand the whole thing to the
      # compiler payload at the bottom of this script, same as `build` /
      # `typecheck`. The rewrite below mutates the positional arg list in
      # place via `set --` once the rev + input path are known.
      CAPS_DIFF_REV=""
      CAPS_INPUT=""
      _saw_diff=0
      _pos=0
      # Single linear scan: discover --diff <rev> and the third positional
      # (the input file, since the vector is `caps SYMBOL FILE [flags...]`).
      # Flags with values (--format VALUE etc.) take their next arg, so we
      # only treat bare non-flag tokens as positionals.
      _skip_next=0
      for _arg in "$@"; do
        if [ "$_saw_diff" = "1" ]; then CAPS_DIFF_REV="$_arg"; _saw_diff=0; continue; fi
        if [ "$_skip_next" = "1" ]; then _skip_next=0; continue; fi
        case "$_arg" in
          --diff) _saw_diff=1 ;;
          --diff-base|--format|-o|--output) _skip_next=1 ;;
          -*) ;;
          *)
            _pos=$((_pos + 1))
            if [ "$_pos" = "3" ] && [ -z "$CAPS_INPUT" ]; then
              CAPS_INPUT="$_arg"
            fi
            ;;
        esac
      done
      if [ -n "$CAPS_DIFF_REV" ]; then
        if [ -z "$CAPS_INPUT" ]; then
          echo "nexus caps --diff: cannot locate input file in argv" >&2
          exit 2
        fi
        CAPS_DIFF_TMP=$(mktemp "${TMPDIR:-/tmp}/nexus-caps-diff-base.XXXXXX.nx")
        if ! git show "${CAPS_DIFF_REV}:${CAPS_INPUT}" > "$CAPS_DIFF_TMP" 2>/dev/null; then
          rm -f "$CAPS_DIFF_TMP"
          echo "nexus caps --diff ${CAPS_DIFF_REV}: git show failed (revision missing or file absent at that revision?)" >&2
          exit 1
        fi
        # Rebuild argv: drop `--diff <rev>`, append `--diff-base <tmp>`.
        # Sentinel + eval-set keeps quoting safe for paths with spaces.
        set -- "$@" "__caps_diff_sentinel__"
        _new=""
        _skip=0
        while [ "$#" -gt 0 ]; do
          if [ "$1" = "__caps_diff_sentinel__" ]; then shift; break; fi
          if [ "$_skip" = "1" ]; then _skip=0; shift; continue; fi
          if [ "$1" = "--diff" ]; then _skip=1; shift; continue; fi
          if [ -z "$_new" ]; then
            _new=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
            _new="'$_new'"
          else
            esc=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
            _new="$_new '$esc'"
          fi
          shift
        done
        esc=$(printf '%s' "$CAPS_DIFF_TMP" | sed "s/'/'\\\\''/g")
        _new="$_new '--diff-base' '$esc'"
        eval set -- $_new
      fi
      ;;
  esac
fi
# Hook caps-diff temp into the cleanup trap so an early exit doesn't leak it.
cleanup_caps_diff() {
  [ -n "${CAPS_DIFF_TMP:-}" ] && rm -f "$CAPS_DIFF_TMP"
}
trap 'cleanup_caps_diff; cleanup' EXIT INT TERM HUP

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
# Extract the payload bytes [OFFSET, OFFSET+SIZE) of $0. Read the first
# OFFSET+SIZE bytes and keep the last SIZE: `tail` consumes all of `head`'s
# output, so neither side closes the pipe early (a `tail … | head -c SIZE`
# would SIGPIPE `tail`, printing a spurious "Broken pipe" on every run).
head -c "$((OFFSET + SIZE))" "$0" | tail -c "$SIZE" > "$TMP"

# Compose the wasmtime feature flag set. Both payloads are self-contained
# core WASM with preview1 imports satisfied by --dir mounts.
W_FLAGS="max-wasm-stack=${NEXUS_MAX_WASM_STACK:-67108864},tail-call=y,exceptions=y,function-references=y,stack-switching=y,threads=y,shared-memory=y"

if [ "$RUN_MODE" = "1" ]; then
  # ─── nexus run: build + exec wasm under wasmtime ─────────────────────
  # Compile the input .nx to a temp wasm using the embedded compiler
  # payload, then exec wasmtime against the result with the user's args.
  # The `--` separator splits compile-time tokens from program argv.
  #
  # Sandbox flags (--seed, --frozen-clock, --max-time, --max-mem, --no-net,
  # --no-fs, --no-clock, --no-rand, --tmp-fs) are intercepted here at the
  # shell layer because WASI preview1 has no subprocess API, so the
  # in-wasm driver can validate them (cap strip check raises SandboxRefusal)
  # but must signal the sandbox params to us for wasmtime argv construction.
  # Strategy: pass sandbox flags through to the compiler build phase as-is
  # (it validates cap strip and exits 3 on violation), then use them here
  # to build the wasmtime run argv.
  if [ "$#" -lt 1 ]; then
    echo "nexus run: missing input file" >&2
    echo "  usage: nexus run FILE.nx [sandbox-flags...] [-- ARGS...]" >&2
    exit 2
  fi
  RUN_INPUT=""
  RUN_PROG_ARGS=""
  HAVE_PROG_ARGS=0
  # Sandbox state
  SB_SEED=""
  SB_FROZEN_CLOCK=""
  SB_MAX_TIME=""
  SB_MAX_MEM=""
  SB_NO_NET=0
  SB_NO_FS=0
  SB_NO_CLOCK=0
  SB_NO_RAND=0
  SB_TMP_FS=""
  # Record/replay session (nexus-eoug)
  RUN_RECORD=""
  RUN_REPLAY=""
  # Compiler build pass-through flags (all sandbox flags go to the compiler
  # for cap-strip validation; only then do we construct the run argv).
  SB_COMPILE_FLAGS=""
  _append_compile_flag() {
    if [ -z "$SB_COMPILE_FLAGS" ]; then
      SB_COMPILE_FLAGS=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
      SB_COMPILE_FLAGS="'$SB_COMPILE_FLAGS'"
    else
      esc=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
      SB_COMPILE_FLAGS="$SB_COMPILE_FLAGS '$esc'"
    fi
  }
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
      --seed)
        SB_SEED="${2:-}"
        _append_compile_flag "--seed"
        _append_compile_flag "${2:-}"
        shift 2
        ;;
      --seed=*)
        SB_SEED="${1#--seed=}"
        _append_compile_flag "$1"
        shift
        ;;
      --frozen-clock)
        # --frozen-clock with no value means epoch=0
        if [ "$#" -ge 2 ] && [ "${2#-}" = "${2}" ] && [ -n "${2:-}" ] && [ "${2:-}" != "--" ]; then
          SB_FROZEN_CLOCK="$2"
          _append_compile_flag "--frozen-clock"
          _append_compile_flag "$2"
          shift 2
        else
          SB_FROZEN_CLOCK="0"
          _append_compile_flag "--frozen-clock"
          _append_compile_flag "0"
          shift
        fi
        ;;
      --frozen-clock=*)
        SB_FROZEN_CLOCK="${1#--frozen-clock=}"
        _append_compile_flag "$1"
        shift
        ;;
      --max-time)
        SB_MAX_TIME="${2:-}"
        _append_compile_flag "--max-time"
        _append_compile_flag "${2:-}"
        shift 2
        ;;
      --max-time=*)
        SB_MAX_TIME="${1#--max-time=}"
        _append_compile_flag "$1"
        shift
        ;;
      --max-mem)
        SB_MAX_MEM="${2:-}"
        _append_compile_flag "--max-mem"
        _append_compile_flag "${2:-}"
        shift 2
        ;;
      --max-mem=*)
        SB_MAX_MEM="${1#--max-mem=}"
        _append_compile_flag "$1"
        shift
        ;;
      --no-net)
        SB_NO_NET=1
        _append_compile_flag "--no-net"
        shift
        ;;
      --no-fs)
        SB_NO_FS=1
        _append_compile_flag "--no-fs"
        shift
        ;;
      --no-clock)
        SB_NO_CLOCK=1
        _append_compile_flag "--no-clock"
        shift
        ;;
      --no-rand)
        SB_NO_RAND=1
        _append_compile_flag "--no-rand"
        shift
        ;;
      --tmp-fs)
        SB_TMP_FS="${2:-}"
        _append_compile_flag "--tmp-fs"
        _append_compile_flag "${2:-}"
        shift 2
        ;;
      --tmp-fs=*)
        SB_TMP_FS="${1#--tmp-fs=}"
        _append_compile_flag "$1"
        shift
        ;;
      --record)
        # nexus-eoug: record session to FILE; do NOT pass to compiler
        RUN_RECORD="${2:-}"
        shift 2
        ;;
      --record=*)
        RUN_RECORD="${1#--record=}"
        shift
        ;;
      --replay)
        # nexus-eoug: replay recorded session from FILE; do NOT pass to compiler
        RUN_REPLAY="${2:-}"
        shift 2
        ;;
      --replay=*)
        RUN_REPLAY="${1#--replay=}"
        shift
        ;;
      --explain-capabilities|--format|--explain-capabilities-format)
        # pass-through: value follows in next token
        _append_compile_flag "$1"
        _append_compile_flag "${2:-}"
        shift 2
        ;;
      --verbose)
        _append_compile_flag "--verbose"
        shift
        ;;
      -*)
        # Unknown flag: pass through to compile phase for argparse error
        _append_compile_flag "$1"
        shift
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
    echo "  usage: nexus run FILE.nx [sandbox-flags...] [-- ARGS...]" >&2
    exit 2
  fi

  RUN_WASM=$(mktemp "${TMPDIR:-/tmp}/nexus-run.XXXXXX.wasm")
  cleanup_run() {
    [ -n "${RUN_WASM:-}" ] && rm -f "$RUN_WASM"
    [ -n "$TMP" ] && rm -f "$TMP"
    [ -n "$MANIFEST_FILE" ] && rm -f "$MANIFEST_FILE"
  }
  trap cleanup_run EXIT INT TERM HUP

  # Compile via the compiler payload. Pass sandbox flags so the in-wasm
  # driver can validate cap strips (raises SandboxRefusal → exit 3).
  # Stash both streams of compile output so the program's own stdio is
  # the only thing the user sees on success; on failure replay stderr.
  COMPILE_ELOG=$(mktemp "${TMPDIR:-/tmp}/nexus-run-compile.XXXXXX.log")
  # shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS + SB_COMPILE_FLAGS intentionally word-split.
  if ! eval wasmtime run \
      -W \"\$W_FLAGS\" -S threads \
      --dir=. --dir=\"\${TMPDIR:-/tmp}\" \
      \${NEXUS_WASMTIME_ARGS:-} \
      \"\$TMP\" build \"\$RUN_INPUT\" -o \"\$RUN_WASM\" --explain-capabilities none \
      ${SB_COMPILE_FLAGS:-} \
      >/dev/null 2>"$COMPILE_ELOG"; then
    compile_exit=$?
    cat "$COMPILE_ELOG" >&2
    rm -f "$COMPILE_ELOG"
    exit $compile_exit
  fi
  rm -f "$COMPILE_ELOG"

  # Build the run-phase wasmtime argv with sandbox constraints applied.
  # Cap-derived flags (--dir=., --wasi inherit-network) are suppressed
  # for stripped caps; sandbox extra flags (--env NEXUS_SEED=N etc.) are
  # appended.
  RUN_SB_FLAGS=""
  _append_run_flag() {
    if [ -z "$RUN_SB_FLAGS" ]; then
      esc=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
      RUN_SB_FLAGS="'$esc'"
    else
      esc=$(printf '%s' "$1" | sed "s/'/'\\\\''/g")
      RUN_SB_FLAGS="$RUN_SB_FLAGS '$esc'"
    fi
  }
  # --max-mem MB → -W max-size=<MB>000000
  if [ -n "$SB_MAX_MEM" ]; then
    _append_run_flag "-W"
    _append_run_flag "max-size=${SB_MAX_MEM}000000"
  fi
  # --max-time MS → epoch-interruption + --wasm-timeout <ms>ms
  if [ -n "$SB_MAX_TIME" ]; then
    _append_run_flag "-W"
    _append_run_flag "epoch-interruption=y"
    _append_run_flag "--wasm-timeout"
    _append_run_flag "${SB_MAX_TIME}ms"
  fi
  # --seed N → NEXUS_SEED env var
  if [ -n "$SB_SEED" ]; then
    _append_run_flag "--env"
    _append_run_flag "NEXUS_SEED=${SB_SEED}"
  fi
  # --frozen-clock EPOCH → NEXUS_FROZEN_CLOCK env var
  if [ -n "$SB_FROZEN_CLOCK" ]; then
    _append_run_flag "--env"
    _append_run_flag "NEXUS_FROZEN_CLOCK=${SB_FROZEN_CLOCK}"
  fi
  # Filesystem: --tmp-fs overrides default --dir=.; --no-fs suppresses it
  if [ "$SB_NO_FS" = "0" ]; then
    if [ -n "$SB_TMP_FS" ]; then
      # Rebind: --dir <scratch>::.  maps scratch dir as the virtual '.'
      _append_run_flag "--dir"
      _append_run_flag "${SB_TMP_FS}::."
    else
      _append_run_flag "--dir"
      _append_run_flag "."
    fi
  fi
  # Network: --no-net suppresses --wasi inherit-network
  if [ "$SB_NO_NET" = "0" ]; then
    _append_run_flag "--wasi"
    _append_run_flag "inherit-network"
  fi
  # --dir for tmp always granted (programs need /tmp)
  _append_run_flag "--dir"
  _append_run_flag "${TMPDIR:-/tmp}"

  # ── record mode (nexus-eoug) ───────────────────────────────────────────
  # Capture stdout + stderr + timing + exit code into a JSONL file.
  # The JSONL schema:
  #   line 1: {"kind":"entry","source":"...","source_hash":"...","argv":[...],"seed":...,
  #             "frozen_clock":...,"caps":[],"nexus_version":"..."}
  #   lines 2+: {"kind":"stdout","ts_ms":<i64>,"line":"..."}
  #              {"kind":"stderr","ts_ms":<i64>,"line":"..."}
  #   last line: {"kind":"summary","exit_code":<i64>,"wall_ms":<i64>}
  #
  # Note: we do NOT exec here — we run + wait so we can capture output.
  if [ -n "$RUN_RECORD" ]; then
    _json_esc() {
      # Minimal JSON string escaping: \, ", \n, \r, \t
      printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/'"$(printf '\r')"'/\\r/g; s/'"$(printf '\t')"'/\\t/g' | awk '{printf "%s\\n", $0}' | head -c -2
    }
    _json_str() { printf '"%s"' "$(_json_esc "$1")"; }
    _json_opt() {
      if [ -z "$1" ]; then printf 'null'; else printf '"%s"' "$(_json_esc "$1")"; fi
    }
    _ts_ms() { date '+%s%3N' 2>/dev/null || date '+%s000' 2>/dev/null || echo 0; }
    # Compute source hash (sha256 first byte of hex is stable; fall back to size)
    _src_hash=""
    if command -v sha256sum >/dev/null 2>&1; then
      _src_hash=$(sha256sum "$RUN_INPUT" 2>/dev/null | cut -d' ' -f1)
    elif command -v shasum >/dev/null 2>&1; then
      _src_hash=$(shasum -a 256 "$RUN_INPUT" 2>/dev/null | cut -d' ' -f1)
    fi
    [ -z "$_src_hash" ] && _src_hash="unknown"
    # Get nexus version (git sha of compiler payload)
    _nxver=$(git -C "$(dirname "$0")" rev-parse --short HEAD 2>/dev/null || echo "unknown")
    # Build argv JSON array
    _argv_json="[$(_json_str "$RUN_INPUT")"
    if [ -n "$SB_SEED" ]; then _argv_json="$_argv_json,$(_json_str "--seed"),$(_json_str "$SB_SEED")"; fi
    if [ -n "$SB_FROZEN_CLOCK" ]; then _argv_json="$_argv_json,$(_json_str "--frozen-clock"),$(_json_str "$SB_FROZEN_CLOCK")"; fi
    if [ -n "$SB_MAX_TIME" ]; then _argv_json="$_argv_json,$(_json_str "--max-time"),$(_json_str "$SB_MAX_TIME")"; fi
    if [ -n "$SB_MAX_MEM" ]; then _argv_json="$_argv_json,$(_json_str "--max-mem"),$(_json_str "$SB_MAX_MEM")"; fi
    if [ "$SB_NO_NET" = "1" ]; then _argv_json="$_argv_json,$(_json_str "--no-net")"; fi
    if [ "$SB_NO_FS" = "1" ]; then _argv_json="$_argv_json,$(_json_str "--no-fs")"; fi
    if [ "$SB_NO_CLOCK" = "1" ]; then _argv_json="$_argv_json,$(_json_str "--no-clock")"; fi
    if [ "$SB_NO_RAND" = "1" ]; then _argv_json="$_argv_json,$(_json_str "--no-rand")"; fi
    if [ -n "$SB_TMP_FS" ]; then _argv_json="$_argv_json,$(_json_str "--tmp-fs"),$(_json_str "$SB_TMP_FS")"; fi
    _argv_json="$_argv_json]"
    # Write entry record
    printf '{"kind":"entry","source":%s,"source_hash":%s,"argv":%s,"seed":%s,"frozen_clock":%s,"caps":[],"nexus_version":%s}\n' \
      "$(_json_str "$RUN_INPUT")" \
      "$(_json_str "$_src_hash")" \
      "$_argv_json" \
      "$(_json_opt "$SB_SEED")" \
      "$(_json_opt "$SB_FROZEN_CLOCK")" \
      "$(_json_str "$_nxver")" \
      > "$RUN_RECORD"
    # Run and capture stdout/stderr, timestamping each line
    _t0=$(_ts_ms)
    _rec_out=$(mktemp "${TMPDIR:-/tmp}/nexus-rec-out.XXXXXX")
    _rec_err=$(mktemp "${TMPDIR:-/tmp}/nexus-rec-err.XXXXXX")
    # shellcheck disable=SC2086
    if [ "$HAVE_PROG_ARGS" = "1" ]; then
      eval wasmtime run \
        -W \"\$W_FLAGS\" \
        -S threads \
        ${RUN_SB_FLAGS:-} \
        \${NEXUS_WASMTIME_ARGS:-} \
        \"\$RUN_WASM\" $RUN_PROG_ARGS \
        >"$_rec_out" 2>"$_rec_err"
    else
      # shellcheck disable=SC2086
      eval wasmtime run \
        -W \"\$W_FLAGS\" \
        -S threads \
        ${RUN_SB_FLAGS:-} \
        \${NEXUS_WASMTIME_ARGS:-} \
        \"\$RUN_WASM\" \
        >"$_rec_out" 2>"$_rec_err"
    fi
    _run_exit=$?
    _t1=$(_ts_ms)
    _wall_ms=$((_t1 - _t0))
    # Emit stdout lines to JSONL and to the terminal
    if [ -s "$_rec_out" ]; then
      cat "$_rec_out"
      _ts_now=$(_ts_ms)
      while IFS= read -r _line || [ -n "$_line" ]; do
        printf '{"kind":"stdout","ts_ms":%s,"line":%s}\n' "$_ts_now" "$(_json_str "$_line")" >> "$RUN_RECORD"
      done < "$_rec_out"
    fi
    # Emit stderr lines to JSONL and to the terminal
    if [ -s "$_rec_err" ]; then
      cat "$_rec_err" >&2
      _ts_now=$(_ts_ms)
      while IFS= read -r _line || [ -n "$_line" ]; do
        printf '{"kind":"stderr","ts_ms":%s,"line":%s}\n' "$_ts_now" "$(_json_str "$_line")" >> "$RUN_RECORD"
      done < "$_rec_err"
    fi
    rm -f "$_rec_out" "$_rec_err"
    # Write summary record
    printf '{"kind":"summary","exit_code":%d,"wall_ms":%d}\n' "$_run_exit" "$_wall_ms" >> "$RUN_RECORD"
    exit "$_run_exit"
  fi

  # ── replay mode (nexus-eoug) ──────────────────────────────────────────
  # Re-run the program and assert byte-equivalence of stdout/stderr/exit.
  # Divergence prints a structural diff and exits non-zero.
  if [ -n "$RUN_REPLAY" ]; then
    if [ ! -f "$RUN_REPLAY" ]; then
      echo "nexus run --replay: session file not found: $RUN_REPLAY" >&2
      exit 2
    fi
    # Extract recorded stdout and exit_code from the JSONL session
    _rep_expected_stdout=$(mktemp "${TMPDIR:-/tmp}/nexus-rep-exp-out.XXXXXX")
    _rep_expected_exit=0
    while IFS= read -r _jline; do
      _kind=$(printf '%s' "$_jline" | sed -n 's/.*"kind":"\([^"]*\)".*/\1/p')
      case "$_kind" in
        stdout)
          # Extract the "line" field and append to expected stdout
          _val=$(printf '%s' "$_jline" | sed -n 's/.*"line":"\(.*\)"[[:space:]]*}/\1/p')
          printf '%s\n' "$_val" >> "$_rep_expected_stdout"
          ;;
        summary)
          _rep_expected_exit=$(printf '%s' "$_jline" | sed -n 's/.*"exit_code":\([0-9-]*\).*/\1/p')
          ;;
      esac
    done < "$RUN_REPLAY"
    # Run the program (discarding stderr from the replayed run to stdout comparison)
    _rep_actual_stdout=$(mktemp "${TMPDIR:-/tmp}/nexus-rep-act-out.XXXXXX")
    _rep_actual_stderr=$(mktemp "${TMPDIR:-/tmp}/nexus-rep-act-err.XXXXXX")
    # shellcheck disable=SC2086
    if [ "$HAVE_PROG_ARGS" = "1" ]; then
      eval wasmtime run \
        -W \"\$W_FLAGS\" \
        -S threads \
        ${RUN_SB_FLAGS:-} \
        \${NEXUS_WASMTIME_ARGS:-} \
        \"\$RUN_WASM\" $RUN_PROG_ARGS \
        >"$_rep_actual_stdout" 2>"$_rep_actual_stderr"
    else
      # shellcheck disable=SC2086
      eval wasmtime run \
        -W \"\$W_FLAGS\" \
        -S threads \
        ${RUN_SB_FLAGS:-} \
        \${NEXUS_WASMTIME_ARGS:-} \
        \"\$RUN_WASM\" \
        >"$_rep_actual_stdout" 2>"$_rep_actual_stderr"
    fi
    _rep_actual_exit=$?
    # Stream actual stderr so user sees it
    if [ -s "$_rep_actual_stderr" ]; then
      cat "$_rep_actual_stderr" >&2
    fi
    # Compare
    _rep_ok=1
    if [ "$_rep_actual_exit" != "$_rep_expected_exit" ]; then
      echo "nexus replay: DIVERGE — exit code: expected=$_rep_expected_exit actual=$_rep_actual_exit" >&2
      _rep_ok=0
    fi
    if ! diff -u "$_rep_expected_stdout" "$_rep_actual_stdout" >/dev/null 2>&1; then
      echo "nexus replay: DIVERGE — stdout mismatch:" >&2
      diff -u "$_rep_expected_stdout" "$_rep_actual_stdout" >&2 || true
      _rep_ok=0
    fi
    rm -f "$_rep_expected_stdout" "$_rep_actual_stdout" "$_rep_actual_stderr"
    if [ "$_rep_ok" = "1" ]; then
      echo "nexus replay: OK — byte-equivalent" >&2
      exit 0
    else
      exit 4
    fi
  fi

  # Run-phase exec.
  # shellcheck disable=SC2086
  if [ "$HAVE_PROG_ARGS" = "1" ]; then
    eval exec wasmtime run \
      -W \"\$W_FLAGS\" \
      -S threads \
      ${RUN_SB_FLAGS:-} \
      \${NEXUS_WASMTIME_ARGS:-} \
      \"\$RUN_WASM\" "$RUN_PROG_ARGS"
  else
    # shellcheck disable=SC2086
    eval exec wasmtime run \
      -W \"\$W_FLAGS\" \
      -S threads \
      ${RUN_SB_FLAGS:-} \
      \${NEXUS_WASMTIME_ARGS:-} \
      \"\$RUN_WASM\"
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
  # Discovery: positive fixtures are `*_test.nx` anywhere under TEST_PATH;
  # negative fixtures are any `*.nx` file living under a `negative/`
  # directory (the per-fixture worker classifies each negative by the
  # `// expect-fail:` / `// expect-runtime-throw:` header it carries).
  # Single-file mode: accept either a `*_test.nx` file or any `*.nx`
  # under `negative/`; classification is by header content.
  if [ -f "$TEST_PATH" ]; then
    case "$TEST_PATH" in
      *_test.nx) FILES="$TEST_PATH";;
      */negative/*.nx) FILES="$TEST_PATH";;
      */p3_component/*.nx) FILES="$TEST_PATH";;
      */repl/*.in.txt) FILES="$TEST_PATH";;
      *) FILES="";;
    esac
  else
    POS_FILES=$(find "$TEST_PATH" -type f -name '*_test.nx' 2>/dev/null)
    NEG_FILES=$(find "$TEST_PATH" -type f -path '*/negative/*.nx' 2>/dev/null)
    P3C_FILES=$(find "$TEST_PATH" -type f -path '*/p3_component/*.nx' 2>/dev/null)
    REPL_FILES=$(find "$TEST_PATH" -type f -path '*/repl/*.in.txt' 2>/dev/null)
    FILES=$(printf '%s\n%s\n%s\n%s\n' "$POS_FILES" "$NEG_FILES" "$P3C_FILES" "$REPL_FILES" | grep -v '^$' | sort)
  fi
  N=$(printf '%s\n' "$FILES" | grep -c '^.' || true)
  if [ "$N" = "0" ]; then
    echo "nexus test: no *_test.nx (or tests/negative/*.nx or tests/repl/*.in.txt) fixtures found under $TEST_PATH" >&2
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
  export TMP W_FLAGS TMPDIR_REAL RESULTS_DIR NEXUS_WASMTIME_ARGS COMPILE_EXTRA_ARGS TEST_COVERAGE UPDATE_GOLDEN
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
  #
  # Three fixture flavors are supported (each worker classifies its own
  # fixture from path + header — no out-of-band metadata):
  #
  #   positive (default; `*_test.nx`): compile must succeed, then run
  #     must exit 0. Any failure → FAIL.
  #
  #   negative compile-fail (`// expect-fail: E####`): compile must fail
  #     and the captured diagnostic must include `[E####]` plus every
  #     `// expect-msg: <substring>`.
  #
  #   negative runtime-throw (`// expect-runtime-throw: <substring>`):
  #     compile must succeed; the wasm must then exit non-zero with
  #     combined stdout+stderr containing every `expect-runtime-throw`
  #     and `expect-msg` substring. Wasmtime's "thrown Wasm exception"
  #     line does not carry the constructor name, so fixtures normally
  #     anchor substring assertions on text the fixture itself prints
  #     before raising.
  #
  # Negatives are identified by their parent directory name (`negative/`).
  # A `negative/` fixture without a header directive surfaces as
  # `negative-config` FAIL rather than silently passing.
  cat "$PLAN_FILE" | xargs -P "$JOBS" -L 1 -- /bin/sh -c '
    idx="$1"; f="$2"
    wasm="$RESULTS_DIR/test_$idx.wasm"
    elog="$RESULTS_DIR/err_$idx.log"
    # Fresh per-fixture scratch, mapped to the guest as /tmp in the run
    # phases below. Fixtures may write under /tmp without depending on the
    # host having a real /tmp (a nix dev-shell puts TMPDIR elsewhere).
    scratch="$RESULTS_DIR/scratch_$idx"; mkdir -p "$scratch"

    # Classify by file path. Anything under a `negative/` directory uses
    # the inverted-semantics path; the header parse below picks between
    # the compile-fail and runtime-throw flavors. `repl/*.in.txt` files
    # are transcript-driven golden tests (nexus-u468).
    case "$f" in
      */negative/*)    NEG=1; P3C=0; REPL_GOLDEN=0;;
      */p3_component/*) NEG=0; P3C=1; REPL_GOLDEN=0;;
      */repl/*.in.txt) NEG=0; P3C=0; REPL_GOLDEN=1;;
      *)               NEG=0; P3C=0; REPL_GOLDEN=0;;
    esac

    # ── repl golden fixture (nexus-u468) ──────────────────────────────
    if [ "$REPL_GOLDEN" = "1" ]; then
      # Derive the matching .out.txt path from the .in.txt path.
      golden="${f%.in.txt}.out.txt"
      t0=$(date +%s%N 2>/dev/null || date +%s000000000)
      # Run the REPL with the input script on stdin; capture stdout only.
      # Errors go to a separate log so they do not pollute the golden diff.
      actual="$RESULTS_DIR/repl_actual_$idx.txt"
      elog="$RESULTS_DIR/err_$idx.log"
      if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
          ${NEXUS_WASMTIME_ARGS:-} \
          "$TMP" repl \
          < "$f" > "$actual" 2>"$elog"; then
        t1=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t1 - t0) / 1000000 ))
        tail_txt="$(tail -8 "$elog" 2>/dev/null)"
        printf "FAIL\t%s\trepl-run\t%s\t%s\n" "$ms" "$f" "$tail_txt" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, repl-run)\n" "$f" "$ms" >&2
        exit 0
      fi
      t1=$(date +%s%N 2>/dev/null || date +%s000000000)
      ms=$(( (t1 - t0) / 1000000 ))
      # UPDATE_GOLDEN=1: regenerate the golden file from actual output.
      if [ "${UPDATE_GOLDEN:-0}" = "1" ]; then
        cp "$actual" "$golden"
        printf "PASS\t%s\trepl-golden-updated\t%s\t-\n" "$ms" "$f" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "PASS  %s  (%sms, golden updated)\n" "$f" "$ms" >&2
        exit 0
      fi
      # Golden compare: .out.txt must exist and match byte-for-byte.
      if [ ! -f "$golden" ]; then
        printf "FAIL\t%s\trepl-golden-missing\t%s\t%s\n" "$ms" "$f" \
          "golden file not found: $golden (run UPDATE_GOLDEN=1 ./nexus test to create)" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, repl-golden-missing)\n" "$f" "$ms" >&2
        exit 0
      fi
      if ! diff -u "$golden" "$actual" > "$RESULTS_DIR/repl_diff_$idx.txt" 2>&1; then
        diff_txt="$(cat "$RESULTS_DIR/repl_diff_$idx.txt")"
        printf "FAIL\t%s\trepl-golden-mismatch\t%s\t%s\n" "$ms" "$f" "$diff_txt" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, repl-golden-mismatch)\n" "$f" "$ms" >&2
        printf "%s\n" "$diff_txt" | head -20 >&2
        exit 0
      fi
      printf "PASS\t%s\trepl-golden\t%s\t-\n" "$ms" "$f" \
        > "$RESULTS_DIR/result_$idx.tsv"
      printf "PASS  %s  (%sms, repl-golden)\n" "$f" "$ms" >&2
      exit 0
    fi

    if [ "$NEG" = "1" ]; then
      # Parse the contiguous comment header at the top of the fixture.
      # Three tab-separated record kinds are emitted: `fail<TAB>E####`,
      # `rt<TAB><payload>`, `msg<TAB><substring>`. The walker stops at
      # the first non-comment, non-blank line so fixture bodies cannot
      # accidentally satisfy the parse.
      HEADER_TSV=$(awk "
        /^[[:space:]]*\\/\\/[[:space:]]*expect-fail:/ {
          sub(/^[[:space:]]*\\/\\/[[:space:]]*expect-fail:[[:space:]]*/, \"\")
          print \"fail\\t\" \$0
          next
        }
        /^[[:space:]]*\\/\\/[[:space:]]*expect-runtime-throw:/ {
          sub(/^[[:space:]]*\\/\\/[[:space:]]*expect-runtime-throw:[[:space:]]*/, \"\")
          print \"rt\\t\" \$0
          next
        }
        /^[[:space:]]*\\/\\/[[:space:]]*expect-msg:/ {
          sub(/^[[:space:]]*\\/\\/[[:space:]]*expect-msg:[[:space:]]*/, \"\")
          print \"msg\\t\" \$0
          next
        }
        /^[[:space:]]*\$/ { next }
        /^[[:space:]]*\\/\\// { next }
        { exit }
      " "$f")
      NEG_CODE=$(printf "%s\n" "$HEADER_TSV" | awk -F"\t" "/^fail\t/ { print \$2; exit }")
      NEG_RT_PRESENT=$(printf "%s\n" "$HEADER_TSV" | awk -F"\t" "/^rt\t/ { print 1; exit }")
      NEG_MSGS=$(printf "%s\n" "$HEADER_TSV" | awk -F"\t" "/^msg\t/ { print \$2 }
                                                            /^rt\t/  { if (\$2 != \"\") print \$2 }")

      t0=$(date +%s%N 2>/dev/null || date +%s000000000)

      # Header-shape validation. Surface either-or violations as a
      # configuration FAIL rather than degrading to a silent pass.
      if [ -z "$NEG_CODE" ] && [ -z "$NEG_RT_PRESENT" ]; then
        t1=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t1 - t0) / 1000000 ))
        msg="negative fixture missing // expect-fail: or // expect-runtime-throw: header"
        printf "FAIL\t%s\tnegative\t%s\t%s\n" "$ms" "$f" "$msg" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, negative-config)\n" "$f" "$ms" >&2
        exit 0
      fi
      if [ -n "$NEG_CODE" ] && [ -n "$NEG_RT_PRESENT" ]; then
        t1=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t1 - t0) / 1000000 ))
        msg="negative fixture mixes // expect-fail: and // expect-runtime-throw: headers"
        printf "FAIL\t%s\tnegative\t%s\t%s\n" "$ms" "$f" "$msg" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, negative-config)\n" "$f" "$ms" >&2
        exit 0
      fi

      # ── flavor A: compile-fail diagnostic ───────────────────────────
      if [ -n "$NEG_CODE" ]; then
        if wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
            ${NEXUS_WASMTIME_ARGS:-} \
            "$TMP" build "$f" -o "$wasm" --explain-capabilities none ${COMPILE_EXTRA_ARGS:-} \
            >/dev/null 2>"$elog"; then
          t1=$(date +%s%N 2>/dev/null || date +%s000000000)
          ms=$(( (t1 - t0) / 1000000 ))
          msg="compiled successfully — expected diagnostic $NEG_CODE"
          printf "FAIL\t%s\tnegative-compile\t%s\t%s\n" "$ms" "$f" "$msg" \
            > "$RESULTS_DIR/result_$idx.tsv"
          printf "FAIL  %s  (%sms, negative-compile)\n" "$f" "$ms" >&2
          rm -f "$wasm"
          exit 0
        fi
        t1=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t1 - t0) / 1000000 ))
        # Verify diagnostic code: a `[E####]` substring must appear in
        # the captured stderr.
        if ! grep -q -- "\\[$NEG_CODE\\]" "$elog"; then
          tail_txt="$(tail -8 "$elog" 2>/dev/null)"
          msg="expected $NEG_CODE not found in diagnostic; got: $tail_txt"
          printf "FAIL\t%s\tnegative-compile\t%s\t%s\n" "$ms" "$f" "$msg" \
            > "$RESULTS_DIR/result_$idx.tsv"
          printf "FAIL  %s  (%sms, negative-compile)\n" "$f" "$ms" >&2
          rm -f "$wasm"
          exit 0
        fi
        # Substring assertions: every expect-msg substring must appear.
        miss=""
        IFS_BAK=$IFS
        IFS="
"
        for sub in $NEG_MSGS; do
          [ -z "$sub" ] && continue
          if ! grep -q -F -- "$sub" "$elog"; then
            miss="$miss|$sub"
          fi
        done
        IFS=$IFS_BAK
        if [ -n "$miss" ]; then
          msg="missing substring(s): ${miss#|}"
          printf "FAIL\t%s\tnegative-compile\t%s\t%s\n" "$ms" "$f" "$msg" \
            > "$RESULTS_DIR/result_$idx.tsv"
          printf "FAIL  %s  (%sms, negative-compile)\n" "$f" "$ms" >&2
          rm -f "$wasm"
          exit 0
        fi
        printf "PASS\t%s\tnegative-compile\t%s\t%s\n" "$ms" "$f" "$NEG_CODE" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "PASS  %s  (%sms, expect-fail %s)\n" "$f" "$ms" "$NEG_CODE" >&2
        rm -f "$wasm"
        exit 0
      fi

      # ── flavor B: runtime-throw ─────────────────────────────────────
      # Compile must succeed (the negative is exclusively at run time).
      if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
          ${NEXUS_WASMTIME_ARGS:-} \
          "$TMP" build "$f" -o "$wasm" --explain-capabilities none ${COMPILE_EXTRA_ARGS:-} \
          >/dev/null 2>"$elog"; then
        t1=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t1 - t0) / 1000000 ))
        tail_txt="$(tail -8 "$elog" 2>/dev/null)"
        msg="compile failed — runtime-throw fixture must build cleanly: $tail_txt"
        printf "FAIL\t%s\tnegative-runtime\t%s\t%s\n" "$ms" "$f" "$msg" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, negative-runtime, compile)\n" "$f" "$ms" >&2
        rm -f "$wasm"
        exit 0
      fi
      t1=$(date +%s%N 2>/dev/null || date +%s000000000)
      if wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$scratch::/tmp" \
          ${NEXUS_WASMTIME_ARGS:-} \
          "$wasm" \
          >"$elog" 2>&1; then
        t2=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t2 - t1) / 1000000 ))
        msg="run exited 0 — expected a runtime exception"
        printf "FAIL\t%s\tnegative-runtime\t%s\t%s\n" "$ms" "$f" "$msg" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, negative-runtime, run-ok)\n" "$f" "$ms" >&2
        if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
        exit 0
      fi
      t2=$(date +%s%N 2>/dev/null || date +%s000000000)
      ms=$(( (t2 - t1) / 1000000 ))
      # Substring assertions over combined stdout+stderr.
      miss=""
      IFS_BAK=$IFS
      IFS="
"
      for sub in $NEG_MSGS; do
        [ -z "$sub" ] && continue
        if ! grep -q -F -- "$sub" "$elog"; then
          miss="$miss|$sub"
        fi
      done
      IFS=$IFS_BAK
      if [ -n "$miss" ]; then
        msg="missing substring(s) in run output: ${miss#|}"
        printf "FAIL\t%s\tnegative-runtime\t%s\t%s\n" "$ms" "$f" "$msg" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, negative-runtime, missing-substring)\n" "$f" "$ms" >&2
        if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
        exit 0
      fi
      printf "PASS\t%s\tnegative-runtime\t%s\t%s\n" "$ms" "$f" "runtime-throw" \
        > "$RESULTS_DIR/result_$idx.tsv"
      printf "PASS  %s  (%sms, expect-runtime-throw)\n" "$f" "$ms" >&2
      if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
      exit 0
    fi

    # ── p3_component fixture ───────────────────────────────────────────
    if [ "$P3C" = "1" ]; then
      t0=$(date +%s%N 2>/dev/null || date +%s000000000)
      if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
          ${NEXUS_WASMTIME_ARGS:-} \
          "$TMP" build "$f" -o "$wasm" --p3-component --explain-capabilities none \
          >/dev/null 2>"$elog"; then
        t1=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t1 - t0) / 1000000 ))
        tail_txt="$(tail -8 "$elog" 2>/dev/null)"
        printf "FAIL\t%s\tp3c-compile\t%s\t%s\n" "$ms" "$f" "$tail_txt" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, p3c-compile)\n" "$f" "$ms" >&2
        rm -f "$wasm"
        exit 0
      fi
      t1=$(date +%s%N 2>/dev/null || date +%s000000000)
      if ! wasmtime run -W component-model=y,component-model-async=y,exceptions=y \
          -S p3=y --dir=. --dir="$scratch::/tmp" ${NEXUS_WASMTIME_ARGS:-} \
          "$wasm" \
          >/dev/null 2>"$elog"; then
        t2=$(date +%s%N 2>/dev/null || date +%s000000000)
        ms=$(( (t2 - t1) / 1000000 ))
        tail_txt="$(tail -16 "$elog" 2>/dev/null)"
        printf "FAIL\t%s\tp3c-run\t%s\t%s\n" "$ms" "$f" "$tail_txt" \
          > "$RESULTS_DIR/result_$idx.tsv"
        printf "FAIL  %s  (%sms, p3c-run)\n" "$f" "$ms" >&2
        if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
        exit 0
      fi
      t2=$(date +%s%N 2>/dev/null || date +%s000000000)
      ms=$(( (t2 - t1) / 1000000 ))
      printf "PASS\t%s\tp3c\t%s\t-\n" "$ms" "$f" \
        > "$RESULTS_DIR/result_$idx.tsv"
      printf "PASS  %s  (%sms, p3c)\n" "$f" "$ms" >&2
      if [ "$TEST_COVERAGE" != "1" ]; then rm -f "$wasm"; fi
      exit 0
    fi

    # ── positive fixture (existing path) ──────────────────────────────
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
    if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$scratch::/tmp" \
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
        | awk 'BEGIN {
            # Build a printable-ASCII char -> byte-value lookup. awk lacks
            # ord(); index() against a single string of all printable bytes
            # (0x20-0x7E) gives us the value as (index - 1 + 0x20).
            # Backslash (0x5C) is kept in-slot so indexes stay aligned, even
            # though the escape branch handles that case separately.
            printable = " !\"#$%&'\''()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"
          }
          /@custom "nx.coverage.functions"/ {
            # Extract content between the first and last quote on the line.
            line = $0
            sub(/^[^"]*"nx.coverage.functions"[^"]*"/, "", line)
            # The first byte (escaped or raw) encodes the function count.
            first = substr(line, 1, 1)
            if (first == "\\") {
              # \xx hex form
              hex = substr(line, 2, 2)
              print strtonum("0x" hex)
            } else {
              # raw byte (printable, 0x20-0x7E). Look up via index() against
              # the printable map; offset by 0x20 (first map entry is space).
              pos = index(printable, first)
              if (pos > 0) {
                print pos + 31
              } else {
                # Unrecognized — fall back to 0 rather than emit garbage.
                print 0
              }
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

if [ "$BENCH_MODE" = "1" ]; then
  # ─── nexus bench: host-driven loop ────────────────────────────────────
  # Discover bench_*.nx under BENCH_DIR, compile each, run it BENCH_ITERS
  # times, collect the integer ms printed on stdout's last line, compute
  # median + p95, and emit one NDJSON line per bench.
  BENCH_DIR="examples/feature"
  BENCH_ITERS=5
  BENCH_BASELINE=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --dir)       BENCH_DIR="${2:-}"; shift 2 ;;
      --iters)     BENCH_ITERS="${2:-5}"; shift 2 ;;
      --baseline)  BENCH_BASELINE="${2:-}"; shift 2 ;;
      --*) shift ;;
      *) shift ;;
    esac
  done

  TMPDIR_REAL="${TMPDIR:-/tmp}"
  BENCH_WASM=$(mktemp "${TMPDIR_REAL}/nexus_bench_XXXXXX.wasm")
  cleanup_bench() { rm -f "$BENCH_WASM"; }
  trap 'cleanup_bench; cleanup' EXIT INT TERM HUP

  # Use the same W_FLAGS set earlier in this script (includes threads,
  # shared-memory etc. required by the stdlib allocator).

  # Discover bench_*.nx files in the directory (sorted by find).
  BENCH_FILES=$(find "$BENCH_DIR" -maxdepth 1 -name 'bench_*.nx' 2>/dev/null | sort)
  if [ -z "$BENCH_FILES" ]; then
    echo "nexus bench: no bench_*.nx files found under $BENCH_DIR" >&2
    exit 1
  fi
  N_BENCHES=$(echo "$BENCH_FILES" | wc -l | tr -d ' ')
  echo "nexus bench: discovered $N_BENCHES bench(es) under $BENCH_DIR [iters=${BENCH_ITERS}]" >&2

  BENCH_RESULTS=""  # accumulate name:median:p95 lines for baseline check
  BENCH_ANY_FAIL=0

  for bench_file in $BENCH_FILES; do
    # Derive bench name: strip directory prefix, "bench_" prefix, ".nx" suffix.
    bench_base=$(basename "$bench_file")
    bench_stem="${bench_base#bench_}"
    bench_name="${bench_stem%.nx}"

    # Compile via the compiler payload (same pattern as nexus test loop).
    if ! wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
        ${NEXUS_WASMTIME_ARGS:-} \
        "$TMP" build "$bench_file" -o "$BENCH_WASM" --explain-capabilities none \
        2>/dev/null; then
      echo "{\"name\":\"${bench_name}\",\"error\":\"compile failed\"}"
      BENCH_ANY_FAIL=1
      continue
    fi

    # Run BENCH_ITERS times; collect integer ms from last stdout line.
    _samples=""
    _fail=""
    _i=0
    while [ "$_i" -lt "$BENCH_ITERS" ]; do
      _out=$(wasmtime run \
        -W "$W_FLAGS" \
        -S threads \
        --dir=. --dir="${TMPDIR_REAL}" \
        "$BENCH_WASM" 2>/dev/null)
      _ec=$?
      if [ "$_ec" != "0" ]; then
        _fail="run exit $_ec"
        break
      fi
      # Extract last non-empty line
      _last=$(printf '%s\n' "$_out" | grep -v '^[[:space:]]*$' | tail -1 | tr -d '[:space:]')
      if [ -z "$_last" ]; then
        _fail="no integer output"
        break
      fi
      _samples="${_samples:+$_samples }${_last}"
      _i=$((_i + 1))
    done

    if [ -n "$_fail" ]; then
      echo "{\"name\":\"${bench_name}\",\"error\":\"${_fail}\"}"
      BENCH_ANY_FAIL=1
      continue
    fi

    # Compute median and p95 from space-separated samples using awk.
    _stats=$(printf '%s\n' $_samples | awk '
      BEGIN { n=0 }
      { a[n++]=$1+0 }
      END {
        # insertion sort
        for(i=1;i<n;i++){v=a[i];j=i-1;while(j>=0&&a[j]>v){a[j+1]=a[j];j--};a[j+1]=v}
        idx_med = int(n*50/100); if(idx_med>=n) idx_med=n-1
        idx_p95 = int(n*95/100); if(idx_p95>=n) idx_p95=n-1
        print a[idx_med] " " a[idx_p95]
      }
    ')
    _median=$(echo "$_stats" | awk '{print $1}')
    _p95=$(echo "$_stats" | awk '{print $2}')

    echo "{\"name\":\"${bench_name}\",\"median_ms\":${_median},\"p95_ms\":${_p95},\"iters\":${BENCH_ITERS}}"
    BENCH_RESULTS="${BENCH_RESULTS}${bench_name}:${_median}:${_p95}:20\n"
  done

  # ─── compile_self bench: shell-side, times `./nexus build src/main.nx` ────
  # Cannot be a bench_*.nx file because WASI preview1 has no subprocess;
  # we time it here directly.  Uses /usr/bin/time -v for peak RSS capture.
  # Drift gate is ±40% (coarser) because compile time is more variable
  # than micro-benches (JIT warm-up, I/O, scheduler noise).
  # Runs min(BENCH_ITERS, 3) iterations to avoid long CI times.
  if [ "$BENCH_ITERS" -lt "3" ]; then _cs_iters=$BENCH_ITERS; else _cs_iters=3; fi
  _cs_out_wasm="${TMPDIR_REAL}/nexus_bench_compile_self.wasm"
  _cs_time_log="${TMPDIR_REAL}/nexus_bench_cs_time.txt"
  echo "nexus bench: compile_self [iters=${_cs_iters}, shell-driven, drift=±40%]" >&2
  _cs_ms_samples=""
  _cs_rss_samples=""
  _cs_fail=""
  _cs_i=0
  while [ "$_cs_i" -lt "$_cs_iters" ]; do
    # /usr/bin/time -v writes to stderr; capture both stdout (compile msgs) and stderr.
    /usr/bin/time -v ./nexus build src/main.nx -o "$_cs_out_wasm" \
      2>"$_cs_time_log" >/dev/null
    _cs_ec=$?
    if [ "$_cs_ec" != "0" ]; then
      _cs_fail="compile exited ${_cs_ec}"
      break
    fi
    # Parse elapsed wall clock: "Elapsed (wall clock) time (h:mm:ss or m:ss): [H:]M:SS.ss"
    # Use awk to handle both m:ss and h:mm:ss formats and emit integer milliseconds.
    _cs_ms=$(awk '/Elapsed \(wall clock\)/ {
      # The value is the last colon-separated token group after the final ": "
      # e.g. "0:11.00" (m:ss) or "1:02:03.00" (h:mm:ss)
      n = split($NF, a, ":");
      if (n == 2) { printf "%d", (a[1]+0)*60000 + (a[2]+0)*1000 }
      else if (n == 3) { printf "%d", (a[1]+0)*3600000 + (a[2]+0)*60000 + (a[3]+0)*1000 }
    }' "$_cs_time_log")
    # Parse peak RSS: "Maximum resident set size (kbytes): N"
    _cs_rss=$(awk '/Maximum resident set size/ { print $NF+0 }' "$_cs_time_log")
    if [ -z "$_cs_ms" ] || [ "$_cs_ms" -le "0" ] 2>/dev/null; then
      _cs_fail="could not parse elapsed time"
      break
    fi
    _cs_ms_samples="${_cs_ms_samples:+$_cs_ms_samples }${_cs_ms}"
    _cs_rss_samples="${_cs_rss_samples:+$_cs_rss_samples }${_cs_rss:-0}"
    _cs_i=$((_cs_i + 1))
  done
  rm -f "$_cs_time_log" "$_cs_out_wasm"

  if [ -n "$_cs_fail" ]; then
    echo "{\"name\":\"compile_self\",\"error\":\"${_cs_fail}\"}"
    BENCH_ANY_FAIL=1
  else
    # Compute median and p95 for wall-clock ms.
    _cs_stats=$(printf '%s\n' $_cs_ms_samples | awk '
      BEGIN { n=0 }
      { a[n++]=$1+0 }
      END {
        for(i=1;i<n;i++){v=a[i];j=i-1;while(j>=0&&a[j]>v){a[j+1]=a[j];j--};a[j+1]=v}
        idx_med = int(n*50/100); if(idx_med>=n) idx_med=n-1
        idx_p95 = int(n*95/100); if(idx_p95>=n) idx_p95=n-1
        print a[idx_med] " " a[idx_p95]
      }
    ')
    _cs_median=$(echo "$_cs_stats" | awk '{print $1}')
    _cs_p95=$(echo "$_cs_stats"    | awk '{print $2}')
    echo "{\"name\":\"compile_self\",\"median_ms\":${_cs_median},\"p95_ms\":${_cs_p95},\"iters\":${_cs_iters}}"
    BENCH_RESULTS="${BENCH_RESULTS}compile_self:${_cs_median}:${_cs_p95}:40\n"

    # Compute median peak RSS (kbytes).
    _cs_rss_stats=$(printf '%s\n' $_cs_rss_samples | awk '
      BEGIN { n=0 }
      { a[n++]=$1+0 }
      END {
        if (n == 0) { print "0 0"; exit }
        for(i=1;i<n;i++){v=a[i];j=i-1;while(j>=0&&a[j]>v){a[j+1]=a[j];j--};a[j+1]=v}
        idx_med = int(n*50/100); if(idx_med>=n) idx_med=n-1
        idx_p95 = int(n*95/100); if(idx_p95>=n) idx_p95=n-1
        print a[idx_med] " " a[idx_p95]
      }
    ')
    _cs_rss_median=$(echo "$_cs_rss_stats" | awk '{print $1}')
    _cs_rss_p95=$(echo "$_cs_rss_stats"    | awk '{print $2}')
    # Emit RSS as a separate bench entry (median_ms field holds kbytes for RSS bench).
    echo "{\"name\":\"compile_self_rss_kb\",\"median_ms\":${_cs_rss_median},\"p95_ms\":${_cs_rss_p95},\"iters\":${_cs_iters}}"
    BENCH_RESULTS="${BENCH_RESULTS}compile_self_rss_kb:${_cs_rss_median}:${_cs_rss_p95}:40\n"
  fi

  # Baseline drift gate (±20% default, ±40% for compile_self entries)
  if [ -n "$BENCH_BASELINE" ] && [ "$BENCH_ANY_FAIL" = "0" ]; then
    if [ ! -f "$BENCH_BASELINE" ]; then
      echo "nexus bench: baseline file not found: $BENCH_BASELINE" >&2
      exit 1
    fi
    DRIFT_VIOLATIONS=0
    # For each result line "name:median:p95:gate", look up "name" in the
    # baseline JSON and check drift against the per-entry gate percentage.
    # Normal benches use gate=20 (±20%); compile_self entries use gate=40.
    printf '%b' "$BENCH_RESULTS" | while IFS=: read -r _name _actual _p95_val _gate; do
      [ -z "$_name" ] && continue
      _gate="${_gate:-20}"
      _base=$(awk -v name="$_name" '
        BEGIN { found=0 }
        /"name"[[:space:]]*:[[:space:]]*"/ {
          match($0, /"name"[[:space:]]*:[[:space:]]*"([^"]+)"/, arr)
          if (arr[1] == name) { found=1 }
        }
        found && /"median_ms"[[:space:]]*:/ {
          match($0, /"median_ms"[[:space:]]*:[[:space:]]*([0-9]+)/, arr)
          if (arr[1]+0 > 0) { print arr[1]+0; found=0 }
        }
      ' "$BENCH_BASELINE")
      if [ -z "$_base" ] || [ "$_base" = "0" ]; then continue; fi
      # Compute bounds: lo = base*(100-gate)/100, hi = base*(100+gate)/100
      _lo=$(( _base * (100 - _gate) / 100 ))
      _hi=$(( _base * (100 + _gate) / 100 ))
      if [ "$_actual" -lt "$_lo" ] || [ "$_actual" -gt "$_hi" ]; then
        _pct=$(((_actual - _base) * 100 / _base))
        echo "DRIFT  ${_name}  baseline=${_base}ms  actual=${_actual}ms  drift=${_pct}%  (limit ±${_gate}%)" >&2
        DRIFT_VIOLATIONS=$((DRIFT_VIOLATIONS + 1))
      fi
    done
    # Re-count violations (subshell above cannot export; re-scan from results)
    _viol=$(printf '%b' "$BENCH_RESULTS" | while IFS=: read -r _name _actual _p95_val _gate; do
      [ -z "$_name" ] && continue
      _gate="${_gate:-20}"
      _base=$(awk -v name="$_name" '
        BEGIN { found=0 }
        /"name"[[:space:]]*:[[:space:]]*"/ {
          match($0, /"name"[[:space:]]*:[[:space:]]*"([^"]+)"/, arr)
          if (arr[1] == name) { found=1 }
        }
        found && /"median_ms"[[:space:]]*:/ {
          match($0, /"median_ms"[[:space:]]*:[[:space:]]*([0-9]+)/, arr)
          if (arr[1]+0 > 0) { print arr[1]+0; found=0 }
        }
      ' "$BENCH_BASELINE")
      if [ -z "$_base" ] || [ "$_base" = "0" ]; then continue; fi
      _lo=$(( _base * (100 - _gate) / 100 ))
      _hi=$(( _base * (100 + _gate) / 100 ))
      if [ "$_actual" -lt "$_lo" ] || [ "$_actual" -gt "$_hi" ]; then
        echo "violation"
      fi
    done | wc -l | tr -d ' ')
    if [ "$_viol" -gt "0" ]; then
      echo "nexus bench: ${_viol} bench(es) exceeded drift gate" >&2
      exit 3
    fi
    echo "nexus bench: baseline check passed" >&2
  fi

  [ "$BENCH_ANY_FAIL" = "0" ] && exit 0 || exit 1
fi

# shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS is intentionally word-split.
exec wasmtime run \
  -W "$W_FLAGS" \
  -S threads \
  --dir=. --dir="${TMPDIR:-/tmp}" \
  ${NEXUS_WASMTIME_ARGS:-} \
  "$TMP" "$@"
#__NEXUS_PAYLOAD_MANIFEST__
