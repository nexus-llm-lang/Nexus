# Skipped ports — bootstrap/tests/runtime → tests/runtime

Tests not ported to `.nx` because they verify implementation details specific
to the Rust integration harness (parser-only `compile`, raw WASM byte
inspection, capability-bearing `main`, etc.) rather than Nexus-level
semantics. Tracked for the dvr6.1 ADR.

## strings.rs

- `codegen_utf8_in_data_segment` — inspects raw WASM bytes for UTF-8
  encoding (`F0 9F 91 BD`) and rejects double-encoding. No Nexus-language
  surface for this assertion; only the Rust `compile()` harness exposes the
  byte vector.
- `codegen_utf8_survives_bundling` — same as above plus a call to
  `nexus::compiler::compose::compose_with_stdlib`, a Rust-API-only path.

## functions.rs

- `codegen_fixture_fib_works_in_wasm`,
  `codegen_fixture_di_cap_compiles`,
  `codegen_fixture_module_test_compiles`,
  `codegen_fixture_network_access_compiles` — read fixture text from
  `examples/*.nx` and run via `exec_with_stdlib`. The test is a thin wrapper
  around running the example; runtime exercise belongs in
  `examples/`-targeted runners, not the unit-test corpus.
- `codegen_print_works_via_external_stdio_module`,
  `codegen_print_after_from_i64_works_via_single_string_abi_module`,
  `codegen_handler_reachability_resolves_port_call` — `main` requires
  `PermConsole` and runs `inject stdio.system_handler do … end`. The
  `safe_run`/divide-by-zero pattern used for `tests/runtime/*_test.nx`
  cannot wrap a capability-bearing `main` cleanly — the test harness's
  bundled stdio capability is what makes these compile under `cargo test`.
- `codegen_main_with_args_runs_with_stdlib` — verifies that
  `main(args: [string])` compiles. Signature-only test with no behavior.

## chars.rs

- `char_unicode` — original asserts `'\u{1F600}'` (emoji codepoint 0x1F600)
  parses. Nexus chars are ASCII-only (0x00-0x7F), so the multibyte case is
  not portable. The hex-escape syntax itself (`'\u{NN}'`) is exercised by
  `tests/runtime/chars_unicode_escape_test.nx` against ASCII codepoints.

## concurrency.rs

- `handler_with_kont_clause_parses` — calls
  `nexus::lang::parser::parser().parse(src)` directly, asserting that the
  bootstrap parser accepts the `with @k` continuation-binder syntax used in
  `nxlib/stdlib/sched.nx`. No Nexus-language surface for "the bootstrap
  parser parsed this" — the corresponding self-hosted check lives in
  `nxc::codegen::handler_with_kont_resume`.
- `test_net_effect_enforcement` — calls `should_fail_typecheck` and matches
  via `insta::assert_snapshot!`. Typecheck-error snapshots depend on the
  Rust harness driving the typechecker library; deferred to dvr6.6.K
  (Bucket C — typecheck/* needs `nexus typecheck --emit-json` or stdlib
  typechecker API).
- `test_net_request_method_and_headers_runtime`,
  `test_net_request_https_url_is_accepted`,
  `test_net_request_response_status_and_body_with_request_body` — already
  downgraded to `should_typecheck` in the Rust source ("List types
  ([Header]) and HTTP requests are not yet supported in WASM codegen").
  Pure typecheck-only checks belong in Bucket C alongside
  `test_net_effect_enforcement`.

## wasm_eh.rs

The wasm_eh suite is intrinsically a wasmtime-API integration test: each
case builds raw WASM modules via `wasm_encoder` and calls
`wasmtime::Module::from_binary` / `Instance::new` / typed-func APIs to
verify the WASM Exception-Handling proposal is supported by the engine.
There is no Nexus-language surface for "wasmtime accepted these bytes" —
the language-level analogues (raise/try/catch payload roundtrip,
cross-function unwind, catch-all wildcard) are ported to
`tests/runtime/wasm_eh_*_test.nx` instead.

- `wasm_eh_engine_config_compiles` — purely instantiates a
  `wasmtime::Config { wasm_exceptions, wasm_tail_call,
  wasm_function_references, wasm_stack_switching }`. Engine-flag config is
  Rust-API only.
- `wasm_eh_throw_catch_roundtrip` — replaced by
  `wasm_eh_throw_catch_test.nx` (language-level raise/catch with i64
  payload). The raw-bytes flavor stays Rust-only.
- `wasm_eh_uncaught_traps` — asserts wasmtime's error message contains one
  of `"uncaught" | "exception" | "unhandled" | "wasm trap"`. Trap-message
  shape is wasmtime-internal; the language-level "uncaught raise traps"
  behavior is verified implicitly by every other test's `safe_run`
  divide-by-zero scaffold (an uncaught raise inside `body()` flips `ok=0`
  and `1/ok` traps).
- `wasm_eh_catch_all` — replaced by the `catch _` arm in
  `wasm_eh_throw_catch_test.nx`.
- `wasm_eh_cross_function_unwind` — replaced by
  `wasm_eh_cross_function_unwind_test.nx`.

### Note on backtrace capture (nexus-55x0)

A language-level acceptance test for the post-nexus-55x0 capture-backtrace
path (calling `std:exn::backtrace`) was scoped but not landed: the
bundler currently registers the `nexus:runtime/backtrace` host stub with
`__nx_bt_frame: (i64) -> i64` (returning a string handle), while the WIT
package declared in `bootstrap/src/compiler/compose.rs` types it as
`bt-frame: func(idx: s64) -> string`. wasmtime rejects the resulting
component with `instance export 'capture-backtrace' has the wrong type`,
so end-to-end through `nexus build` + standalone `wasmtime`
(component-model on) cannot observe the captured frames. The unit-level
gate is still covered by `tests/runtime/backtrace_elision_test.nx`
(already in main), which exercises the codegen decision without invoking
the host import.

# Skipped ports — bootstrap/tests/codegen → tests/codegen

Codegen tests are highly Rust-implementation-detail oriented: most inspect
raw WASM bytes via `wasmparser`, call internal Rust APIs, or hold `insta`
snapshots. Only the small subset that asserts Nexus-observable behavior
(raise compiles + traps, main signature rejection) is portable.

## bundler.rs

- `compose_with_stdlib_resolves_imports` — calls
  `nexus::compiler::compose::compose_with_stdlib` directly (Rust API)
  and inspects the resulting bytes via `wasmparser::Parser::is_component`.
  No Nexus-language surface for "the composer produced a component."
- `compose_with_stdlib_is_thread_safe` — concurrency regression for the
  staging temp_dir race; spawns Rust threads, calls `compose_with_stdlib`
  per source, and byte-compares results across threads. Multi-threaded
  Rust harness with no Nexus-level analogue.

## errors.rs

- `snapshot_codegen_error_unsupported_external` — uses
  `insta::assert_snapshot!` to capture the codegen error string verbatim.
  Snapshot infra is Rust-only; the underlying assertion (external imports
  reject `wasm` extension) is better expressed as a future
  `bootstrap/tests/fixtures/codegen/` negative fixture if needed.

## structure.rs

All thirteen tests in `structure.rs` skipped — every one walks the WASM
binary via `wasmparser::Parser::new(0).parse_all(&wasm)` to assert on
specific opcodes / sections / imports / globals, which is not observable
from inside Nexus:

- `codegen_exports_wasi_cli_run_wrapper` — checks `_start` is in the
  export section.
- `compile_metrics_reports_all_pass_durations` — calls
  `compile_program_to_wasm_with_metrics` and asserts each pass duration
  is non-zero. Pure Rust API.
- `codegen_tail_call_emits_return_call_instruction`,
  `codegen_tail_call_in_if_branch_emits_loop_br`,
  `codegen_non_self_tail_call_emits_return_call`,
  `codegen_non_tail_call_does_not_emit_return_call`,
  `codegen_match_arm_tail_call_emits_tco`,
  `codegen_match_arm_mutual_tail_call_emits_return_call`,
  `codegen_return_if_with_tail_calls_emits_return_call`,
  `codegen_return_match_with_tail_calls_emits_return_call` — TCO opcode
  inspection (`Operator::Loop`, `Br`, `ReturnCall`). The optimization is
  internal; programs run correctly with or without TCO at the Nexus level
  (modulo stack-overflow regressions, which would need a deep-recursion
  fixture if regressions emerge).
- `codegen_main_with_args_desugars_to_zero_param_wasm` — checks WASM
  export kind for `main`.
- `codegen_main_with_args_includes_proc_capability` — uses
  `nexus::runtime::parse_nexus_capabilities` (Rust API).
- `stdlib_wasm_modules_are_wasi_only_or_self_contained` — walks files
  under `nxlib/stdlib/*.wasm` and inspects each import section. Build-
  artifact inspection, not Nexus semantics.
- `codegen_deduplicates_externals_by_wasm_identity` — constructs an
  `LirProgram` directly (in-memory IR), calls `compile_lir_to_wasm`, and
  counts WASM imports. Pure compiler-internal regression that has no
  Nexus surface (you cannot author two externals pointing at the same
  WASM symbol from `.nx` in a way that survives the parser).
- `codegen_dwarf_sections_emitted` — asserts custom WASM sections
  `.debug_abbrev`, `.debug_info`, `.debug_line` exist. Toolchain-detail.
- `diag_bump_mode_g0_g2_initial_values` — heap-collision diagnostic
  reading raw WASM globals init values.
- `notrace_elides_capture_backtrace_import`,
  `backtrace_usage_keeps_capture_import` — inspect the WASM import
  section for `nexus:runtime/backtrace::capture-backtrace`. Tree-shake
  observability test, no Nexus surface.

## errors.rs (ported)

The remaining errors.rs tests are ported (or fixturized):

- `codegen_main_non_unit_return_is_rejected` →
  `bootstrap/tests/fixtures/codegen/main_non_unit_return.nx` +
  `.expected_error.txt`. Standalone fixture for a future runner.
- `codegen_raise_compiles_and_traps` →
  `tests/codegen/codegen_errors_raise_compiles_test.nx`.
- `codegen_exn_constructor_lowering` →
  `tests/codegen/codegen_errors_runtime_error_test.nx`.

# Skipped ports — bootstrap/tests/stdlib (batch 5)

Batch 5 covers the simpler stdlib subdirectories: `arena`, `array`, `char`,
`clock`, `exn`. Two of those (`array`, `exn`) lean almost entirely on the
Rust harness — see notes per file below — while the others ported as
runtime fixtures under `tests/stdlib/`.

## arena.rs

Ported (runtime fixtures under `tests/stdlib/`):

- `arena_100k_echo_workload_g0_bounded_with_reset` →
  `stdlib_arena_heap_reset_bounded_test.nx`. Drops the
  `Console.println` status line so `main` is `() -> unit` (the safe_run
  scaffold cannot wrap a capability-bearing main); the trap-on-mismatch
  invariant is preserved through `assert_eq_i64` on the masked G0.
- `arena_echo_workload_g0_grows_without_reset` →
  `stdlib_arena_heap_grows_without_reset_test.nx`. Same shape; the
  `g0_after > g0_before + 1000` invariant is the leak signal.

Skipped:

- `net_echo_server_fixture_compiles` — `compile_fixture_via_nxc` is a
  Rust-only harness API (drives the self-hosted nxc compiler entry point).
  No Nexus-language surface for "this fixture compiles via the
  self-hosted nxc."
- `arena_cross_crate_fs_string_workload_bounded_with_reset`,
  `arena_cross_crate_fs_string_workload_grows_without_reset` — both
  `main`s declare `require { PermConsole, PermFs }` and use a Rust-side
  `TempDir` to seed the input file. The PermFs/PermConsole capability
  combo can't be expressed without a capability-bearing main, and the
  TempDir lifetime / path interpolation is harness-side. Bucket: cap+fs
  fixtures, deferred to a follow-up that introduces a fixture runner
  capable of provisioning a temp dir.
- `arena_100k_echo_workload_alloc_lifo_reset_via_nxc` — uses
  `exec_nxc_core_capture_stdout` to read `delta_pages=...` from the
  fixture's stdout (the Rust harness drives the self-hosted nxc + an
  in-process executor). No Nexus-level surface for "stdout of the
  fixture under the self-hosted compiler."

## array.rs

All five tests skipped — typecheck-only `should_typecheck` /
`should_fail_typecheck` cases:

- `test_array_type_mismatch`,
  `test_array_indexing_non_array`,
  `test_array_assignment_mismatch`,
  `test_array_consume_nonlinear_consumer_is_rejected` — every one is
  `should_fail_typecheck` + `insta::assert_snapshot!`. Snapshot infra is
  Rust-only (Bucket C — typecheck/* needs `nexus typecheck --emit-json`
  or stdlib typechecker API).
- `test_array_consume_with_proper_consumer_passes` — pure
  `should_typecheck` call, no runtime exercise. Substituted with a real
  runtime exerciser in `stdlib_array_basic_test.nx` (length, is_empty)
  rather than ported as a typecheck-only check.

Note on the `tests/stdlib/stdlib_array_basic_test.nx` runtime gap: any
call that goes through `arr[idx]` (which lowers to a `__array_get`
pseudo-call in `bootstrap/src/compiler/passes/lir_lower.rs`) currently
ICEs in standalone codegen with `internal compiler error: call target
'__array_get' not found in lowered symbols [E2007]`. That blocks runtime
porting of `array.get` / `array.set` / `array.fold_left` / `array.any` /
`array.all` / `array.find_index` / `array.map_in_place` / `array.consume`.
Tracked separately; the basic test only exercises `array.length`
(resolved through the `__nx_array_length` external) and `array.is_empty`.

Additional follow-up — `array.length` returns a heap-pointer-shaped
garbage value in standalone codegen (see `tests/runtime/u8w7_repro_test.nx`
for the bisected diagnosis). The host implementation
(`bootstrap/src/lib/core/src/core.rs::__nx_array_length`) is an identity
function on its second arg (`len: i32`); the codegen for `TyArray`
external args (`src/backend/codegen.nx::emit_external_arg`, ~line 821)
unpacks the array's i64 atom as a packed `(ptr<<32 | len)` value the
same way it does for strings. Strings are stored that way on the wire,
but Nexus arrays are heap-allocated records (the i64 atom is just the
heap pointer with the upper 32 bits zero), so `len` ends up being the
heap-pointer's lower 32 bits. The committed array test "passes" only
because `1 / (n - 2)` for n in the 60000-range yields 0 via integer
truncation rather than trapping. Originally filed as
"`assert_eq_i64` traps inside `try`" (nexus-u8w7) — that diagnosis was
a misattribution; assert_eq_i64 raises `AssertionFailed` correctly
because n != expected. Tracked under the re-scoped follow-up filing.

## char.rs

Ported as a runtime fixture:

- `char_classification` →
  `stdlib_char_classification_test.nx`. Inlines
  `bootstrap/tests/fixtures/test_char_classification.nx` into the
  `safe_run` scaffold; uses `std:test/assert` directly because the body
  is exception-free apart from the assertion path.

## clock.rs

Ported as a runtime fixture:

- `clock_now_returns_positive_value` + `clock_sleep_does_not_crash` →
  `stdlib_clock_basic_test.nx` (combined into one fixture; both
  exercised under `inject clk.system_handler`). `main` carries
  `require { PermClock }`; the safe_run scaffold passes the requirement
  through unchanged.

Skipped:

- `clock_denied_at_wasi_level_without_allow_clock` — uses
  `exec_with_stdlib_caps_should_trap` with a hand-built
  `ExecutionCapabilities` that denies `allow_clock`, then asserts the
  resulting wasmtime trap message contains "denied". The capability
  matrix is a Rust-runtime configuration; standalone wasmtime takes
  capabilities from CLI flags, not from a programmatic
  `ExecutionCapabilities`.
- `clock_requires_perm_clock` — `should_fail_typecheck` +
  `insta::assert_snapshot!`. Bucket C (typecheck-error snapshot).

## exn.rs

All tests skipped:

- `backtrace_depth_nonzero_on_raise`,
  `backtrace_cross_function_has_frames` — both call
  `std:exn::backtrace(exn: e)`, which depends on the
  `nexus:runtime/backtrace` host import. Per the existing note in this
  file ("Note on backtrace capture (nexus-55x0)"), the bundler's host
  stub registers `__nx_bt_frame: (i64) -> i64` while the WIT package
  declares it as `bt-frame: func(idx: s64) -> string`; standalone
  `wasmtime run --component-model` rejects the resulting component with
  `instance export 'capture-backtrace' has the wrong type`. End-to-end
  observation of `backtrace(exn:)` is therefore blocked at the standalone
  runner. Reinstate once that WIT-vs-stub mismatch is fixed (epic
  nexus-55x0 follow-up).
