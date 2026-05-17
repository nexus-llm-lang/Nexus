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
    run)
      # `nexus run` — same preview1 constraint as test (Proc.exec is a -1
      # stub), so build + exec are driven from the host shell. The in-wasm
      # `src/driver.nx` still parses `run` and would Proc.exec wasmtime if
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
      *) FILES="";;
    esac
  else
    POS_FILES=$(find "$TEST_PATH" -type f -name '*_test.nx' 2>/dev/null)
    NEG_FILES=$(find "$TEST_PATH" -type f -path '*/negative/*.nx' 2>/dev/null)
    FILES=$(printf '%s\n%s\n' "$POS_FILES" "$NEG_FILES" | grep -v '^$' | sort)
  fi
  N=$(printf '%s\n' "$FILES" | grep -c '^.' || true)
  if [ "$N" = "0" ]; then
    echo "nexus test: no *_test.nx files (or tests/negative/*.nx fixtures) found under $TEST_PATH" >&2
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

    # Classify by file path. Anything under a `negative/` directory uses
    # the inverted-semantics path; the header parse below picks between
    # the compile-fail and runtime-throw flavors.
    case "$f" in
      */negative/*) NEG=1;;
      *)            NEG=0;;
    esac

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
      if wasmtime run -W "$W_FLAGS" -S threads --dir=. --dir="$TMPDIR_REAL" \
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

# shellcheck disable=SC2086  # NEXUS_WASMTIME_ARGS is intentionally word-split.
exec wasmtime run \
  -W "$W_FLAGS" \
  -S threads \
  --dir=. --dir="${TMPDIR:-/tmp}" \
  ${NEXUS_WASMTIME_ARGS:-} \
  "$TMP" "$@"
#__NEXUS_PAYLOAD_MANIFEST__
