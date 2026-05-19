# ADR 0003 — Polyglot Runner: wasm-direct → Host-Shell-Driven Test Runner

**Status**: Accepted  
**Date**: 2026-05-13  
**Issue**: nexus-f90i (prerequisite), commit 57ba69cf  

## Context

Nexus programs run under WASI preview1 when the threads proposal is active
(`-S threads`).  WASI preview1 has no subprocess primitive: `Proc.exec`
cannot be implemented in-wasm because there is no `proc_exec` system call in
the preview1 ABI.

The test runner (`src/test_runner.nx`) operates by compiling each fixture with
`nexus build` and then running the compiled wasm — both of which require
launching a child process.  Under in-wasm execution this means calling
`Proc.exec` inside `nexus.wasm`, which returns -1 unconditionally.  Every
fixture therefore reported `compile exit=-1` regardless of whether the test
passed or failed.

This was not a theoretical problem.  After parallel `@` (nexus-f90i, commit
`7efcc668`) required `-S threads` in `header.sh`, the entire `nexus test`
surface became non-functional: 66 fixtures, 0 results.

### Options considered

| # | Option | Summary |
|---|--------|---------|
| 1 | Implement Proc.exec via WASI preview2 or p3 | Requires abandoning `-S threads` (preview2) or migrating to p3 (large scope) |
| 2 | Wrap each fixture in a separate wasmtime invocation from host shell | Test runner becomes a polyglot shell+wasm hybrid |
| 3 | Port test runner entirely to shell script | Lose Nexus-language test framework; hard to maintain |
| 4 | Spawn subprocesses via host-side embedder | Requires custom wasmtime embedder, not portable |

## Decision

**Option 2: polyglot launcher intercept in `header.sh`.**

When `./nexus test [path]` is invoked, the polyglot launcher (`header.sh`)
intercepts the `test` subcommand before passing control to `nexus.wasm`.  The
launcher:

1. Discovers `*_test.nx` fixtures under the given path using the host shell.
2. Drives a per-fixture compile + run loop via `xargs -P` (parallel by
   default; `NEXUS_TEST_JOBS` overrides parallelism; `--sequential` forces
   one worker).
3. Each step is a separate `wasmtime run …` invocation — the real subprocess
   primitive the in-wasm runner cannot reach.
4. Aggregates results; `--junit FILE` emits XML for CI.

`src/test_runner.nx` is **unchanged in behaviour**.  It is retained as the
eventual self-host path.  When invoked wasm-direct (bypassing the launcher),
it probes `Proc.exec` at entry and exits with a diagnostic pointing at
`./nexus test` (commit `57ba69cf`).

Baseline after the fix: **64 passed / 2 failed / 66 total** on `tests/stdlib`.
The 2 pre-existing failures are the TyRef compile cliff in
`subst_apply_env_go` (unrelated to the runner change).

### Rationale

- The launcher intercept is the thinnest layer that solves the problem: the
  wasm binary is unchanged, only the bootstrap shell acquires subprocess
  capability.
- Keeping `src/test_runner.nx` alive preserves the self-host path for when
  nexus migrates to WASI preview3 (nexus-vqmf), where `future<T>` / native
  async may enable in-wasm subprocess-like semantics.
- `xargs -P` parallelism is free and measurable; the thread-count knob
  (`NEXUS_TEST_JOBS`) keeps CI reproducible.

## Consequences

- **Positive**: `nexus test` is functional again without any source changes
  to the compiler or test runner.
- **Positive**: parallel fixture execution is available out of the box; CI
  wall-clock time for the test suite drops proportionally to core count.
- **Positive**: the polyglot boundary is explicit and documented; developers
  know `./nexus test` works, `wasmtime run … nexus.wasm test` does not.
- **Negative**: the test runner is now a two-layer system (shell + wasm).
  Any change to test discovery or output format requires updating `header.sh`,
  not just `src/test_runner.nx`.
- **Negative**: the wasm-direct path prints a diagnostic and exits; any
  tooling that invokes `nexus.wasm` directly for test running silently breaks.
- **Neutral**: `-S threads` (preview1) remains required for the parallel `@`
  runtime.  A future p3 migration (nexus-vqmf) may allow consolidating both
  the subprocess and threading requirements.
