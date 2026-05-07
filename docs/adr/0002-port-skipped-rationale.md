# ADR 0002 — Rationale for un-ported `bootstrap/tests` cases

- Status: Accepted (2026-05-08)
- Scope: any contributor evaluating whether a Rust-side test under
  `bootstrap/tests/**` should also exist as a `.nx` fixture under
  `tests/**`.
- Owners: compiler maintainers.
- Related: epic `nexus-dvr6.6` (port catalog).
- Detailed catalog: `bootstrap/tests/PORT_SKIPPED.md` (per-test entries
  grouped by source file). This ADR is the categorical rollup; the
  catalog stays as the line-itemized companion.

## Context

The dvr6 epic ported the bootstrap (Rust) integration suite to
language-level `.nx` fixtures runnable through `nexus build` +
standalone `wasmtime`. A residual set of cases did not port. The
catalog lists 179 entries; the question that recurs in review is
"why not this one?" Without a categorical record, every reviewer
re-derives the answer from per-file prose.

This ADR fixes the categories so reviewers can match a residual test
to a bucket and either accept the skip or reopen the port. The
catalog remains the source of truth for which test belongs to which
bucket; this document only names the buckets and their policy.

## Decision

A bootstrap test stays un-ported iff it falls into one of the buckets
below. Buckets A–J are exhaustive over the catalog as of dvr6.6.
Adding a new skipped case requires (1) a catalog entry pointing at
the matching bucket, or (2) a new bucket appended here with the same
shape.

### Bucket A — Raw WASM byte inspection

Tests that drive `wasmparser::Parser` (or equivalent) over the
compiled module bytes to assert section / opcode / import shape:
`structure.rs` (all 13 cases), `strings.rs::codegen_utf8_*`,
`bundler.rs::compose_*`, the TCO `ReturnCall` family,
`notrace_elides_*`. The "did codegen emit X" contract is internal —
Nexus itself has no surface for "the produced WASM has section Y." A
language-level analogue would have to assert observable behavior
(stack-overflow regression, performance, correct utf-8 roundtrip)
and several already exist alongside the byte-grep originals.

### Bucket B — Capability-bearing `main`

The `tests/runtime/*_test.nx` `safe_run` scaffold wraps `body()` in
divide-by-zero so an uncaught raise flips an `ok=0` cell. It cannot
host a `main` that declares `require { PermConsole, PermFs, ... }`
because those caps make the wrapper itself ill-typed. Tests that
must emit to stdout, mount a temp dir, or chain a `Console.println`
status line stay Rust-side until a fixture runner that provisions
caps + stdout capture lands. Examples: `runtime/functions.rs`
console family, `arena_cross_crate_fs_*`, `lazy_thunk_syntax`,
`test_backtrace_captures_call_stack`.

### Bucket C — Typecheck-error snapshots / warnings

`should_fail_typecheck` + `insta::assert_snapshot!(err)` locks the
exact diagnostic *text*. The standalone `nexus build` exposes "compile
failed" via exit code and unstructured stderr only. The whole
`typecheck/error_snapshots.rs` file (25 cases), the snapshot-bearing
arms of `inference.rs`, `array.rs`, `linear.rs`, `effects.rs`,
`exhaustiveness.rs`, `ffi.rs`, `fs.rs`, `env.rs`, `clock.rs`, `string.rs`,
and the warning-observation pair in `typecheck/diagnostics.rs` all
sit here. Unblocked by `nexus typecheck --emit-json` (Bucket C is the
canonical name in the catalog). The error *contracts* (mismatch
shapes, undefined ident, arity, missing perm, etc.) are already
covered by deterministic positive/negative `.nx` fixture pairs in
batches 7–10; the snapshot tests pin the strings, which adds no
behavioral coverage.

### Bucket D — Programmatic AST construction

Tests that build a `Program` AST in Rust (`Stmt::Let { sigil:
Linear, ... }`, `EnumDef`, `MatchCase`, ...) and call
`TypeChecker::check_program` directly. The semantics under test is
"does this AST node shape pass the checker?" — there is no surface
syntax round-trip. Examples:
`test_enum_constructor_with_linear_arg_*` in `linear.rs`,
`test_enum_exhaustive` / `test_enum_non_exhaustive` and the
proptest sweeps in `exhaustiveness.rs`. Skipped permanently; no
emit-json could lift them because they bypass the parser by
construction.

### Bucket E — Rust-only runtime APIs

Tests that call `nexus::compiler::*`, `nexus::runtime::*`,
`nexus::ir::*`, or `compose_with_stdlib` directly and inspect Rust
return values. `unit/string_heap.rs` (refcount / freed-slot reuse),
`bundler.rs::compose_with_stdlib_is_thread_safe`,
`compile_metrics_reports_all_pass_durations`,
`codegen_main_with_args_includes_proc_capability` (parses
`nexus_capabilities` directly), every `ir/{hir,mir,lir,lir_opt}.rs`
case (30 tests). Snapshotted IR pretty-prints belong here too. No
language-level surface exists for "the runtime's internal
string-table shape" or "the LIR optimizer preserved this ref-set."

### Bucket F — Subprocess drivers

`unit/driver_*.rs`, `unit/launcher.rs` spawn the `nexus` binary as
a child via `std::process::Command`, write to its stdin, and assert
on exit / stdout / stderr. Nexus has no `Process` capability that
lets a fixture spawn a sibling and observe output, and the `Proc.exec`
wrapper currently rides on a WASI `spawn` primitive that
`wasmtime run` does not implement ("operation not supported on this
platform"). Permanently Rust-side until either (i) wasmtime ships
`spawn`, or (ii) a native Nexus host gains the capability.

### Bucket G — Proptest / fuzz / catch_unwind

Random-input sweeps that depend on a Rust generator + `panic` /
`catch_unwind` to assert "the typechecker did not abort" or "every
proper subset is non-exhaustive." `typecheck/fuzz.rs` (9 cases), the
`prop_*` family in `inference.rs` / `linear.rs` / `effects.rs` /
`exhaustiveness.rs` (~25 cases). The deterministic positive cases
the proptests randomise around are all covered by the
hand-picked fixture pairs in batches 7/8/10. The "no panic over
random ASCII" contract has no surface analogue without a Random
handler + a process-level deny-all wrapper.

### Bucket H — Engine-level integration

`wasm_eh.rs` builds raw modules with `wasm_encoder` and instantiates
them through `wasmtime::Module::from_binary` to verify the WASM
Exception-Handling proposal is supported by the engine. The
language-level analogues (raise/try/catch payload roundtrip,
cross-function unwind, catch-all wildcard) are already ported to
`tests/runtime/wasm_eh_*_test.nx`. The raw-bytes engine-flag flavor
stays Rust-only. The `exec_threaded` family (shared-memory threading
in `nxc/`) and `clock_denied_at_wasi_level_without_allow_clock`
(programmatic `ExecutionCapabilities`) sit here too: standalone
`wasmtime run` takes caps from CLI flags and does not provide
shared-memory threading.

### Bucket I — Self-host fixtures importing `src/**`

`nxc/test_*.nx` fixtures `import "src/typecheck/types"` and
construct in-language IR values to drive specific compiler passes.
They test the bootstrap compiler the same way `ir/*.rs` tests the
Rust compiler: by reaching into its module tree. Reproducing them
under `tests/` would couple the language-level test corpus to the
bootstrap source layout. 27 cases.

### Bucket J — Engine-blocked (open follow-ups)

Cases that *would* port today but are blocked by a known bug
tracked under another bd id:

- `exn::backtrace_*` — WIT vs host-stub mismatch on
  `nexus:runtime/backtrace::capture-backtrace`. Reinstate post
  `nexus-55x0`.
- `array.rs::*` runtime ports (`get`/`set`/`fold_left`/...) —
  `__array_get` not lowered in standalone codegen (E2007); the
  committed `stdlib_array_basic_test.nx` only exercises `length` /
  `is_empty`. The known `array.length` heap-pointer-shaped return
  (originally filed as nexus-u8w7) is tracked under the re-scoped
  follow-up.
- Trap-message substring matches (`wasm_eh_uncaught_traps`,
  `chan_recv_before_send_traps`) — the standalone runtime surfaces
  trap-vs-exit only.

## Consequences

- Reviewers can match any `bootstrap/tests/**` test that lacks a
  `.nx` mirror to one of A–J. Anything that does not match opens a
  question for the catalog.
- Buckets B, C, F, J are the only ones with a forward path. A, D, E,
  G, H, I are permanent skips by construction.
- `bootstrap/tests/runtime/concurrency.rs:54,78,101` cite
  `bootstrap/tests/PORT_SKIPPED.md` directly; that catalog stays in
  place as the line-item companion. Future port-skips append entries
  there and reference the bucket from this ADR.
- Lifting Bucket C requires `nexus typecheck --emit-json` (or an
  in-language typechecker API surface). Lifting Bucket B requires
  a fixture runner that provisions caps + captures stdout. Both
  are out of scope for dvr6.
