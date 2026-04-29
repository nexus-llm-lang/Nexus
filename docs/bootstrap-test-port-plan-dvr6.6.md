# bootstrap/tests/ → .nx port plan (nexus-dvr6.6)

Audit + pilot + sub-issue plan for porting the Rust integration test surface
(`bootstrap/tests/`) to the self-hosted test framework that landed in dvr6.4
(`nxlib/stdlib/test/`) and dvr6.5 (`src/test_runner.nx` + `tests/`).

This is a multi-issue epic; this document is the contract that downstream
sub-issues (and the orchestrator) work against.

## 1. Audit findings

### 1.1 File inventory (90 .rs files, 110 .nx fixtures)

| Category | .rs count | .rs LOC | Notes |
| --- | --- | --- | --- |
| capabilities | 2 (1+mod) | 99 | WASI capability matrix; needs runtime caps API |
| codegen | 4 (3+mod) | 968 | Bundler + structural wasm checks; **bootstrap-only** |
| harness | 6 | (infrastructure) | shared fixture/exec helpers; not user-facing tests |
| ir | 5 (4+mod) | 879 | HIR/MIR/LIR introspection; **bootstrap-only** until self-host exposes IR APIs |
| nxc | 16 (15+mod) | 1075 | Self-hosted-compiler smoke tests; **wraps fixtures already in .nx** |
| parse | 1 | 1055 | Parser unit tests; **bootstrap-only** until self-host exposes parser AST API |
| runtime | 9 (8+mod) | 3219 | End-to-end exec checks; **directly portable** |
| stdlib | 26 (25+mod) | ~4500 | Library happy-path + edge cases; **directly portable** |
| typecheck | 15 (14+mod) | ~6000 | Inference / errors / linearity; **bootstrap-only** until self-host exposes typecheck error API |
| unit | 5 (4+mod) | 564 | Driver REPL/launcher/string-heap; **bootstrap-only** (Rust unit tests) |
| integration.rs | 1 | 11 | mod aggregator |
| **total** | **90** | **~18.7k** | |

The "75 files" target in the issue body refers to user-test files (excluding
`mod.rs` glue and `harness/*` infrastructure): 90 - 6 (harness) - 9 (mod) =
**75**.

### 1.2 .nx fixture inventory (110 files in bootstrap/tests/fixtures/)

`bootstrap/tests/fixtures/` is split:
- 41 `.nx` files at the top level — used by both Rust harness and the
  self-hosted compiler smoke (`nxc/*.rs` calls `compile_fixture_via_nxc`).
- 69 `.nx` files under `nxc/` — driven by `nxc/*.rs`'s
  `exec_nxc_core` / `exec_nxc_core_capture_stdout`.

These are **already .nx programs**. The port for the `nxc/*.rs` category is
not "rewrite" but "wire the existing .nx fixtures into the new
`tests/.../*_test.nx` discovery".

### 1.3 Portability classification (per category)

| Bucket | Categories | Rationale |
| --- | --- | --- |
| **A. Directly portable (exec only)** | runtime/, stdlib/ (most) | Each `#[test]` is `exec(r#"...nx..."#)` — convert source to a `tests/<area>/*_test.nx` file using `std:test/assert`. |
| **B. Portable via fixture re-use** | nxc/ (16) | Fixtures already exist; need wrappers + the new runner to compile+run them. Some fixtures rely on captured-stdout assertions that the current runner does not surface — see Gap R-1 below. |
| **C. Portable when self-host exposes typecheck-error API** | typecheck/ | Bootstrap calls `should_fail_typecheck` / `should_typecheck` (Rust harness). Self-hosted needs equivalent: `nexus typecheck --emit-json` or a stdlib API to drive the typechecker library and inspect diagnostics. |
| **D. Portable when self-host exposes parser/IR API** | parse/, ir/, codegen/, capabilities/ | These poke compiler internals. Until the self-host exposes parser/HIR/MIR/LIR/codegen artifacts as introspectable values, these stay as Rust tests. |
| **E. Bootstrap-only** | unit/, harness/ | Rust unit tests for Rust code (REPL state machine, launcher path resolution, string-heap impl). They will be deleted with the Rust crate, not ported. |

Net: **runtime (8) + stdlib (~25) + nxc (16) = ~49 files** are portable in the
short-to-medium term. The remaining 26 user-test files block on
introspection-API work that is outside dvr6's "decommission" scope.

### 1.4 Runner gap inventory (blocks downstream porting)

| ID | Gap | Location | Impact |
| --- | --- | --- | --- |
| R-1 | `tests/.../*_test.nx` runner does not capture stdout for assertion | `src/test_runner.nx:222` (`Proc.exec` returns combined `stderr` only used for failure tail) | Tests that need to verify "what was printed" (scheduler interleave, REPL output) fail silently — equivalent to `feedback_verify_actual_output.md` warning. Sub-issue blocker for nxc/ category. |
| R-2 | `wasmtime_flags` missing `component-model=y` and `-S http,inherit-network` | `src/test_runner.nx:238` | Bootstrap-emitted components fail to instantiate (`no exported instance named wasi:cli/run` or `wasi:http/types` missing). Affects every fixture that goes through `nexus build` + bundle path. |
| R-3 | `Proc.exec` is blocking → `--jobs N` is a stub | `src/test_runner.nx:24-25` | Sequential runs are 10-20× slower than the Rust `cargo test` parallel runner. Acceptable for MVP; sub-issue file under dvr6.x. |
| R-4 | f64 arithmetic via `std:math::abs_float` lowers to `nexus:runtime/math::f64-abs` host import that wasmtime CLI does not stub | bootstrap codegen (no host-stub binding); self-host nxc inlines the op | Until bootstrap is decommissioned (or dvr6.6's runner switches to nxc by default), tests must inline `abs_f64` instead of importing. Pilot tests follow this convention. |
| R-5 | Bundler skips stdlib link when zero `nexus:std/*` imports remain after dead-code-elim, producing a "thin" component without `wasi:cli/run` | bundler logic; surfaces when only `assert_true` (no string-conversion) is used | Workaround: ensure each `*_test.nx` uses at least one assert variant that calls `from_i64`/`from_bool` (transitively pulls `std:str`). Pilot tests follow this convention. |

R-1 / R-2 are runner bugs — they should be fixed before scheduling the nxc/
fixture-wrap sub-issue (otherwise that sub-issue ports tests that silently
pass-on-empty).

## 2. Pilot port (Round 1 deliverable)

**Scope**: 12 fixtures from `runtime/arithmetic.rs` (7) + `runtime/records.rs` (5).

**Files added** (all under `tests/runtime/`):
- `arithmetic_i64_function_call_test.nx`
- `arithmetic_i64_inc_test.nx`
- `arithmetic_negate_test.nx`
- `arithmetic_f64_literal_arith_test.nx`
- `arithmetic_prefix_neg_i64_test.nx`
- `arithmetic_prefix_fneg_f64_test.nx`
- `arithmetic_prefix_not_bool_test.nx`
- `records_field_access_test.nx`
- `records_field_access_multiple_test.nx`
- `records_field_then_arith_test.nx`
- `records_let_destructure_test.nx`
- `records_let_destructure_multi_test.nx`

**Validation** (with bootstrap nexus + standalone wasmtime — runner gaps R-1/R-2
mean we cannot use `nexus test` end-to-end yet):

```
build:  12 / 12 PASS
run:    12 / 12 PASS (component-model=y -S http,inherit-network --dir=.)
```

**Skipped from this pilot**:
- `arithmetic.rs::prop_codegen_arithmetic_associativity` (proptest macro;
  property-based testing is covered by `nxlib/stdlib/test/property.nx` but
  needs a separate sub-issue — the harness must seed RNG and report failing
  cases).

**Conventions established by pilot** (downstream sub-issues should follow):

1. **One Rust `#[test]` → one `*_test.nx` file**. Filename pattern
   `<original-fn-name>_test.nx`, kebab- or snake-cased to match the original
   test fn.
2. **Standard scaffold** — every fixture wraps its asserting body in a
   `safe_run` and traps via `1/ok` so the runner reads the exit code:
   ```nx
   let body = fn () -> unit throws { Exn } do … end
   let safe_run = fn () -> i64 do
     try body(); return 1 catch _ -> return 0 end
   end
   let main = fn () -> unit do
     let ok = safe_run()
     let _ = 1 / ok
     return ()
   end
   ```
3. **Use `assert_eq_*` over `assert_true`** wherever feasible — the former
   keeps `std:str::from_*` live so the bundler links stdlib (workaround for
   R-5).
4. **No `std:math::abs_float`** until R-4 is fixed — inline the `if val<.0.0
   then 0.0-.val end` if needed.
5. **Header comment cites the source** — `Port of bootstrap/tests/<path>::<fn>`
   so a reverse-lookup is one `rg` away during decommission cleanup.

## 3. Sub-issue proposal (orchestrator-friendly)

Sub-issues should be filed under `nexus-dvr6.6.<n>` (or freshly minted bd ids
of the orchestrator's choosing). Acceptance criteria for each: pilot
conventions §2 applied + per-file `nexus build` + standalone wasmtime PASS.

| Sub-id | Title | Scope (.rs files / fixture count) | Blockers |
| --- | --- | --- | --- |
| dvr6.6.A | runtime/* tail port | concurrency.rs, control_flow.rs, chars.rs, functions.rs, strings.rs, wasm_eh.rs (66 + 11 + 8 + 24 + 9 + ~10 = ~128 fixtures across 6 files) | R-1 for any test that asserts on captured stdout (control_flow has a few) |
| dvr6.6.B | stdlib/list + stdlib/option + stdlib/result port | list.rs, option.rs, result.rs (~30 fixtures) | none (pure-language, no host caps) |
| dvr6.6.C | stdlib/string + stdlib/char + stdlib/json port | string.rs, char.rs, from_char_code_utf8.rs, json.rs (~40 fixtures) | none |
| dvr6.6.D | stdlib/array + stdlib/collections + stdlib/math port | array.rs, collections.rs, math.rs (~35 fixtures) | R-4 for math (mostly passes via inlined helpers; some sqrt/floor/ceil tests will need stubs) |
| dvr6.6.E | stdlib/io + stdlib/fs + stdlib/clock + stdlib/env + stdlib/proc + stdlib/rand + stdlib/net port | stdio.rs, fs.rs, clock.rs, env.rs, proc.rs, rand.rs, net.rs (~30 fixtures) | R-2 (capabilities flags), per-test caps audit; net.rs needs wasi:http stubs at the runner level |
| dvr6.6.F | stdlib/test_lib + stdlib/lazy + stdlib/arena + stdlib/exn + stdlib/jsonrpc + stdlib/lsp_* port | remaining stdlib/*.rs (~20 fixtures) | none |
| dvr6.6.G | nxc/ fixture wrappers | 16 .rs files; the 69 .nx fixtures already exist under bootstrap/tests/fixtures/nxc/ — relocate them to tests/nxc/ + add scaffolding | R-1 (several fixtures rely on captured stdout, e.g. test_chan_oneshot) |
| dvr6.6.H | runner R-1 fix | `run_one_test` must capture stdout into `TestOutcome` and expose it for assertion-style fixtures | self-contained |
| dvr6.6.I | runner R-2 fix | `wasmtime_flags`: add `component-model=y` and `-S http,inherit-network` (guarded by capability detection) | self-contained |
| dvr6.6.J | runner R-3 (parallel `--jobs`) | Requires non-blocking exec or thread pool | wasi runtime work |
| dvr6.6.K | typecheck/* port (Bucket C) | depends on self-host exposing `nexus typecheck --emit-json` or equivalent | new self-host CLI surface |
| dvr6.6.L | parse/ + ir/ + codegen/ + capabilities/ port (Bucket D) | depends on self-host introspection APIs | epic-scale; defer until decommission timeline finalised |

**Recommended order** for the orchestrator: H/I (unblock runner) → A/B/C/D/F
(in parallel; pure-portable buckets) → E (wasi-cap audit) → G (nxc wrappers
once R-1 is fixed) → J (parallelism polish) → K/L (defer pending API work).

**Test count reality check** — `rg -c '^#\[test\]'` across all .rs files
yields **704 individual `#[test]` functions** (i.e., 704 `*_test.nx` files
the port produces, since each Rust test becomes one .nx fixture under the
1-test-per-file convention). The "75 files" figure in the issue body counted
.rs files; the actual fixture count is an order of magnitude larger:

| Bucket | .rs files | #[test] count | est. .nx fixtures |
| --- | --- | --- | --- |
| runtime/ | 8 | 134 | 134 |
| stdlib/ | 25 | 169 | 169 |
| typecheck/ | 14 | 199 | 199 (after Bucket-C unblocks) |
| nxc/ | 15 | ~30 | (re-uses 69 existing fixtures) |
| codegen/ + ir/ + parse/ | 9 | 130+ | (Bucket D — defer) |
| capabilities/ + unit/ | 6 | ~40 | (Bucket E — won't port) |

At the pilot's 12-fixture-per-round cadence, **Buckets A+B (runtime + stdlib)
alone are ~25 worker-rounds**, plus runner R-1/R-2 fix rounds and the
typecheck-API epic. The full epic is closer to **40-50 sub-issues**, not 11.
The proposal above (12 sub-issues) is a coarse first cut — orchestrator
should split A/B/C/D further if individual rounds blow past the
~10-fixture-per-round budget.

## 4. Out of scope for this round

- Removing any Rust test (`bootstrap/tests/runtime/arithmetic.rs` stays — it
  remains the canonical run until the corresponding self-hosted port has
  shipped under all sub-issues above).
- Wiring `bootstrap.sh` to invoke `nexus test` instead of `cargo test`
  (premature until A-G are merged).
- Property-based testing port (proptest harness) — separate sub-issue under
  the test-framework epic (dvr6.4 follow-up).
- Snapshot testing port — `insta::assert_snapshot!` users (typecheck/
  errors, parse/ errors) need either a snapshot stdlib API equivalent (already
  partial under `nxlib/stdlib/test/snapshot.nx`) or a pivot to
  exact-string-match assertions.
