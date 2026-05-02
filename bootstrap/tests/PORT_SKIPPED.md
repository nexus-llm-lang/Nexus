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

## unit/driver_*.rs, unit/launcher.rs

- `driver_typecheck.rs`, `driver_repl.rs`, `driver_polyglot.rs`, `launcher.rs` —
  these tests exercise the Nexus CLI subcommands by spawning the `nexus`
  binary as a subprocess via the Rust `std::process::Command` API. While
  Nexus provides a `Proc.exec` wrapper, the underlying WASI environment
  used by `wasmtime run` does not currently support the `spawn` primitive,
  resulting in "operation not supported on this platform" errors.
  End-to-end integration testing of the compiler-as-a-binary must remain
  in the Rust bootstrap or move to a native (non-WASM) test runner.


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

- `snapshot_codegen_error_unsupported_external` — the `insta` snapshot is
  Bucket C (Rust-only), but the underlying assertion is now expressed as a
  standalone fixture: `bootstrap/tests/fixtures/codegen/external_record_return_unsupported.nx`
  + `.expected_error.txt` (substring match for "external return type").
  The snapshot test still pins the exact error string at the Rust layer.

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

## env.rs

Ported as runtime fixtures:

- `env_mock_handler` → `stdlib_env_mock_handler_test.nx`. Exercises
  user-defined handler short-circuit; no PermEnv required.
- `env_get_unset_returns_none` → `stdlib_env_unset_returns_none_test.nx`.
  Uses `system_handler` with `require { PermEnv }`; relies on the
  parent shell not exporting `NX_NEVER_DEFINED_VAR_DVR66` (chosen for
  uniqueness rather than relying on a Rust-driven empty env).

Skipped:

- `env_port_typechecks_with_perm_env`,
  `env_set_typechecks_with_perm_env` — pure `should_typecheck` calls
  with no runtime exercise; not worth a `.nx` mirror given the matching
  inject + Env.set is already exercised structurally by mock_handler
  + system_handler tests.
- `env_get_requires_perm_env` — `should_fail_typecheck` +
  `insta::assert_snapshot!`. Bucket C (typecheck-error snapshot).
- `env_get_set_to_empty_returns_some_empty` — the original verifies
  that an env var deliberately set to "" round-trips as Some("") (not
  None — regression for nexus-9lp4.25). Reproducing this as a runtime
  fixture would require either pre-setting the env in the parent shell
  (fragile against parallel test runs) or routing through `Env.set`,
  which mutates the live process env in standalone wasmtime — but the
  empty-string-vs-unset distinction depends on the host's
  `__nx_has_env` returning true, and there is no portable guarantee
  that `Env.set("X", "")` causes `__nx_has_env` to flip to true on
  every wasi runtime. Deferred to a fixture runner that can set
  explicit envs around the wasm invocation.

`env_get_set_to_value_returns_some_value` is reframed as
`stdlib_env_set_then_get_test.nx`: it does Env.set("X","hello") then
Env.get("X"), validating the same round-trip invariant without relying
on the Rust harness's pre-set env wiring.

## fs.rs

Ported as runtime fixtures, using fixed paths under `/tmp/nx_dvr66_*`
(bootstrap.sh's wasmtime invocation mounts `/tmp` via `--dir` so the
host can read/write there). Tests are not parallel-safe with each
other on the same path prefix; each test uses a distinct subdir name:

- `fs_create_dir_and_exists_work` →
  `stdlib_fs_create_dir_and_exists_test.nx`.
- `fs_append_and_read_roundtrip` →
  `stdlib_fs_append_and_read_roundtrip_test.nx`.
- `fs_remove_file_updates_exists` →
  `stdlib_fs_remove_file_test.nx`.

Skipped:

- `fs_linear_file_requires_close`,
  `fs_linear_file_double_close_is_rejected`,
  `fs_read_requires_fs_coeffect` — all `should_fail_typecheck` +
  `insta::assert_snapshot!`. Bucket C (typecheck-error snapshot).

## json.rs

JSON has direct Nexus analogues for every value kind, so the runtime
suite is the most portable in this batch. Each Rust `#[test]` becomes
one `.nx` fixture under the standard `safe_run` scaffold:

- `json_roundtrip_null_true_false` → `stdlib_json_atom_roundtrip_test.nx`
- `json_number_boundaries` → `stdlib_json_number_boundaries_test.nx`
- `json_serialize_preserves_int_vs_float` →
  `stdlib_json_int_vs_float_serialize_test.nx`
- `json_rejects_leading_zero` →
  `stdlib_json_leading_zero_rejected_test.nx`
- `json_string_named_escapes_roundtrip` →
  `stdlib_json_string_named_escapes_test.nx`
- `json_unicode_basic_bmp_escape` →
  `stdlib_json_unicode_bmp_escape_test.nx`
- `json_unicode_surrogate_pair_above_bmp` →
  `stdlib_json_unicode_surrogate_pair_test.nx`
- `json_string_rejects_lone_surrogate` →
  `stdlib_json_lone_surrogate_rejected_test.nx`
- `json_whitespace_around_tokens` → `stdlib_json_whitespace_test.nx`
- `json_array_nested_roundtrip` →
  `stdlib_json_array_nested_roundtrip_test.nx`
- `json_object_lookup_and_roundtrip` → `stdlib_json_object_lookup_test.nx`
- `json_lsp_initialize_request_roundtrip` →
  `stdlib_json_lsp_initialize_request_test.nx`
- `json_lsp_publish_diagnostics_roundtrip` →
  `stdlib_json_lsp_publish_diagnostics_test.nx`
- `json_lsp_did_change_roundtrip` → `stdlib_json_lsp_did_change_test.nx`
- `json_trailing_comma_rejected` →
  `stdlib_json_trailing_comma_rejected_test.nx`
- `json_trailing_data_rejected` →
  `stdlib_json_trailing_data_rejected_test.nx`
- `json_serialize_rejects_nan` →
  `stdlib_json_serialize_rejects_nan_test.nx`

No json.rs tests skipped.

## typecheck/diagnostics.rs

Entire file (2 tests) is **Bucket C — typecheck-warning observation**.
Both tests use `typecheck_warnings()` (the harness wrapper around
`TypeChecker::take_warnings()`) and inspect raw warning-string content:

- `test_linear_primitive_emits_unnecessary_warning` — asserts a warning
  string containing the literal "unnecessary".
- `test_linear_record_does_not_emit_unnecessary_warning` — asserts the
  *absence* of such a warning.

Nexus has no surface for "did the typechecker emit a warning for this
program" — `nexus build` only reports errors, not warnings, on stdout
in a structured form. Both tests stay deferred until
`nexus typecheck --emit-json` (Bucket C) ships and exposes warnings
through a port-stable interface. Tracked in dvr6.1 ADR.

## typecheck/inference.rs

Ported as positive `*_test.nx` fixtures under `tests/typecheck/`:

- `test_basic_poly`, `test_nested_calls`, `test_two_generics`,
  `test_let_poly_binding`, `test_complex_poly_logic` →
  `typecheck_inference_basic_poly_test.nx`.
- `test_record_access` → `typecheck_inference_record_access_test.nx`.
- `test_poly_variants` → `typecheck_inference_poly_variants_test.nx`.
- `test_int_literal_defaults_to_i64`,
  `test_int_literal_annotation_can_select_i32` →
  `typecheck_inference_int_literal_default_test.nx` (the i32 path is
  observed via i32-equality → bool, since there is no `as` cast and
  no `assert_eq_i32` helper).
- `test_float_literal_annotation_can_select_f32`,
  `test_float_arithmetic`, `test_float_compare`,
  `test_float_literal_type`, `test_f32_and_f64_keywords` →
  `typecheck_inference_float_literal_test.nx`.
- `test_named_function_can_be_used_as_value`,
  `test_inline_lambda_literal_typechecks` →
  `typecheck_inference_named_function_as_value_test.nx`.
- `test_binary_op_in_call_arg`, `test_string_concat_in_call_arg` →
  `typecheck_inference_call_arg_exprs_test.nx`.
- `test_anonymous_record`, `test_record_unification` →
  `typecheck_inference_record_unification_test.nx` (the original
  `test_anonymous_record` used Console.print; here we drop the
  side-effect channel and observe field-access typing directly).
- `test_ref_creation_and_type`, `test_cannot_return_ref`,
  `test_ref_assignment`, `test_ref_read` →
  `typecheck_inference_ref_basic_test.nx`.
- `test_match_expr_same_type_cases_typechecks`,
  `test_match_expr_with_return_cases_diverge` →
  `typecheck_inference_match_expr_test.nx`.
- `test_main_with_args_typechecks` →
  `typecheck_inference_main_with_args_test.nx`.
- `test_let_destructure_record`, `test_let_destructure_nested_record` →
  `typecheck_inference_let_destructure_test.nx`.
- `test_no_return_unit_is_ok`, `test_implicit_unit_return_with_side_effect`,
  `test_return_in_if_branch_counts_as_return`,
  `test_return_in_match_counts_as_return` →
  `typecheck_inference_implicit_unit_return_test.nx`.
- `if_else_expr_infers_i64`, `if_else_expr_infers_string`,
  `if_else_expr_as_return_value`, `if_else_expr_nested` →
  `typecheck_inference_if_else_expr_test.nx`.
- `prop_typecheck_nested_arithmetic` (proptest) →
  `typecheck_inference_nested_arithmetic_test.nx` (3 representative
  shapes: positive, mixed-sign, with-zero).

Negative `should_fail_typecheck` tests **without** `insta::assert_snapshot!`
are ported as standalone fixtures under
`bootstrap/tests/fixtures/typecheck/<name>.nx` plus
`<name>.expected_error.txt`:

- `test_arg_mismatch` → `inference_arg_mismatch.nx`.
- `test_int_literal_is_not_i32_without_annotation` →
  `inference_int_literal_not_i32.nx`.
- `test_float_int_mismatch` → `inference_float_int_mismatch.nx`.
- `test_record_fail` → `inference_record_missing_field.nx`.
- `if_else_expr_type_mismatch_fails` (and
  `prop_typecheck_if_else_branches_must_match`) →
  `inference_if_else_branch_mismatch.nx`.

Tests reading existing fixtures already in `bootstrap/tests/fixtures/`
need no additional port (the fixture is already its own
self-typechecking program):

- `test_type_sum_definition_with_labeled_variant_fields`,
- `test_recursive_lambda_with_annotation_typechecks`,
- `test_gravity_rule_immutable_holds_value`,
- `test_ref_generic`.

Skipped (Bucket C — typecheck-error snapshot via `insta::assert_snapshot!`):

- `test_lambda_cannot_capture_ref`,
- `test_linear_capturing_lambda_cannot_be_called_twice`,
- `test_constructor_arity_error_is_llm_friendly`,
- `test_constructor_pattern_arity_error_is_llm_friendly`,
- `test_function_arity_mismatch_shows_expected`,
- `test_function_arity_mismatch_too_many_args`,
- `test_match_expr_type_mismatch_fails`,
- `test_main_with_wrong_arg_type_fails`,
- `test_main_with_too_many_args_fails`,
- `test_let_destructure_non_exhaustive_fails`,
- `test_no_return_non_unit_is_type_error`.

These all assert the *exact* typecheck error message via insta
snapshots. The error-string contract is a Rust-API observation — Nexus
itself only exposes "compile failed with error" via `nexus build`'s
exit code and stderr. Until `nexus typecheck --emit-json` lands, these
remain Bucket C.

## typecheck/linear.rs

Ported as positive `*_test.nx` fixtures:

- `test_linear_basic_pass`,
  `test_linear_param_accepts_plain_value_via_weakening`,
  `test_linear_primitive_auto_drop_pass`,
  `test_linear_primitive_wildcard_pass`,
  `test_linear_primitive_match_wildcard_pass` →
  `typecheck_linear_basic_pass_test.nx`.
- `test_linear_borrow_basic` → `typecheck_linear_borrow_basic_test.nx`
  (the original used `Console.print`; here we observe the borrow via
  a pure `&i64 -> i64` accumulator, dropping the stdio channel).
- `test_generic_drop_accepts_non_linear_primitives` →
  `typecheck_linear_generic_drop_primitives_test.nx`.
- `test_adt_with_linear_arg_consumed_once_passes` (combined with
  the inference-side `test_linear_capture_makes_lambda_linear_and_single_use`)
  → `typecheck_linear_capture_lambda_test.nx`.
- `test_lazy_binding_and_force`, `test_lazy_type_annotation`,
  `test_lazy_force_on_non_lazy_via_parens`,
  `test_lazy_pass_thunk_by_bare_name` →
  `typecheck_linear_lazy_basic_test.nx`.
- `test_linear_deeply_nested_else_if_value_branches`,
  `test_linear_deeply_nested_else_if_all_return`,
  `test_linear_deeply_nested_else_if_mixed_return_and_value` →
  `typecheck_linear_deeply_nested_else_if_test.nx`.
- `prop_linear_primitive_drops` (proptest) →
  `typecheck_linear_primitive_drops_test.nx` (one representative
  case; the original used `let %a = {}` (empty record). Here we
  use `let %a = 42` because empty-record `{}` is unusual surface
  syntax and the test name calls out *primitives*).

Negative `should_fail_typecheck` (no insta) ported as fixtures:

- `test_generic_drop_user_defined_linear_consumes_once` →
  `linear_user_defined_unconsumed.nx`.
- `test_adt_with_linear_arg_is_promoted_to_linear` →
  `linear_adt_promoted_unconsumed.nx`.
- `test_lazy_unused_is_error` → `linear_lazy_unused.nx`.
- `test_lazy_primitive_unused_is_error` → `linear_lazy_primitive_unused.nx`.
- `test_lazy_double_force_is_error` → `linear_lazy_double_force.nx`.
- `test_lazy_capture_linearizes_closure` →
  `linear_lazy_capture_closure_double_call.nx`.
- `prop_linear_shadowing_requires_consumption` →
  `linear_shadowed_unconsumed.nx` (one representative case; we use
  `{ id: N }` payloads because empty-record `{}` is unusual surface).

Skipped (Bucket C — programmatic AST construction):

- `test_enum_constructor_with_linear_arg_requires_consumption`,
- `test_enum_constructor_with_linear_arg_can_be_consumed_once`.

Both build a `Program` AST in Rust via `Stmt::Let { sigil: Linear, .. }`
and call `TypeChecker::check_program` directly — no Nexus surface
syntax at all. Skipped permanently (not even `--emit-json` would
help; the semantics are *whether the AST node shape passes
typecheck*, which is implementation-detail-by-construction).

## stdlib/string.rs (batch 9)

- `console_read_line_requires_perm_console` — uses
  `should_fail_typecheck` with `insta::assert_snapshot!`. Snapshot
  tests are Rust-harness-bound; capability-denial diagnostics are
  exercised by `tests/typecheck/*` via positive `should_typecheck`
  rather than snapshot diff.
- `console_read_line_typechecks_with_perm_console` — typecheck-only
  positive that does not exec. Already covered by other Console
  tests that exec under PermConsole; standalone .nx port adds no
  signal beyond "does it compile", which is implicit in any port.
- `test_backtrace_captures_call_stack` — first-frame contract
  ("expected 'main'") is brittle under the safe_run scaffold (the
  raise lives inside `body`, not `main`, so the top frame becomes
  `body`). Restructuring without safe_run would require a
  PermConsole-bearing `main` (the original `Console.println` on
  failure path), which the .nx scaffold can't host.
- `console_getchar_with_mock_handler`,
  `console_read_line_with_mock_handler` — the current `Console`
  port has additional methods (`eprint`, `eprintln`, `read_bytes`)
  beyond what the original Rust mock provided. The strict missing-
  method check forces a full implementation. `read_bytes` returns
  `%ByteBuffer`, whose constructor is opaque outside the
  `std:bytebuffer` module — and calling `bytebuffer.empty()` from a
  handler clause body fails the cap-row unification ("requires
  {}"), since handler clauses do not propagate the surrounding
  `require` context. No clean way to write a stub handler in
  surface .nx; the original Rust harness predates the strict check.

## stdlib/rand.rs (batch 9)

The original `rand_determinism_pcg_step_byte_equal_across_runs`
uses `exec_nxc_core_capture_stdout` to compare stdout line-by-line
against an offline reference; the fixture
`bootstrap/tests/fixtures/nxc/test_rand_determinism.nx` declares
`main` with `require { PermConsole, PermRandom }` and calls
`Console.println(from_i64(...))` for each step.

Ported as `tests/stdlib/stdlib_rand_determinism_pcg_test.nx` with
the same reference values, but checked inline via direct
comparison against the Random.next_i64() return values (no
PermConsole, no stdout capture). Same contract — divergence on any
of the eleven outputs (10 from seed=1 + 1 from seed=2) catches
PCG-step regressions and state-cell layout corruption.

## typecheck/linearity.rs (batch 10)

All four positives ported; all three negatives deposited as fixtures.

- `test_try_catch_arm_consumes_pre_try_linear_pass` →
  `typecheck_linearity_try_catch_pre_try_consume_test.nx`.
- `test_linear_consumed_before_throwable_call_passes`,
  `test_linear_across_pure_call_passes`,
  `test_linear_across_throwable_call_inside_try_with_catch_consume_passes`
  → `typecheck_linearity_consumed_before_throwable_call_test.nx`.

Negative `should_fail_typecheck` (no insta) ported as fixtures:

- `test_try_catch_arm_starts_from_pre_try_linear_set` →
  `linearity_try_catch_pre_try_unconsumed.nx`.
- `test_linear_across_throwable_call_outside_try_rejects` →
  `linearity_across_throwable_call_outside_try.nx`.
- `test_linear_created_inside_try_across_throwable_call_rejects` →
  `linearity_created_inside_try_across_throwable_call.nx`.

## typecheck/exhaustiveness.rs (batch 10)

Positive `should_typecheck` tests ported as runnable .nx (one file per
logical group). Negative `should_fail_typecheck` tests deposited as
fixture pairs for the future driver harness.

- `test_nested_result_exhaustive` →
  `typecheck_exhaustiveness_nested_result_test.nx`.
- `test_bool_exhaustive` → `typecheck_exhaustiveness_bool_test.nx`.
- `test_wildcard_exhaustive` →
  `typecheck_exhaustiveness_wildcard_test.nx`.
- `test_record_exhaustive` →
  `typecheck_exhaustiveness_record_test.nx`.
- `test_or_pattern_covers_all_constructors_is_exhaustive`,
  `test_or_pattern_alternatives_with_same_binding_typechecks` (combined
  with the implicit positive of `test_enum_exhaustive`) →
  `typecheck_exhaustiveness_enum_or_pattern_test.nx`.

Negative `should_fail_typecheck` ported as fixtures:

- `test_nested_result_non_exhaustive` →
  `exhaustiveness_nested_result_non_exhaustive.nx`.
- `test_bool_non_exhaustive` →
  `exhaustiveness_bool_non_exhaustive.nx`.
- `test_int_non_exhaustive` → `exhaustiveness_int_non_exhaustive.nx`.
- `test_record_non_exhaustive` →
  `exhaustiveness_record_non_exhaustive.nx`.
- `test_or_pattern_missing_constructor_is_non_exhaustive` →
  `exhaustiveness_or_pattern_missing_constructor.nx`.
- `test_or_pattern_alternatives_must_bind_same_variables` →
  `exhaustiveness_or_pattern_diff_bindings.nx`.

Skipped (Bucket C — programmatic AST construction):

- `test_enum_exhaustive`, `test_enum_non_exhaustive` — both build a
  `Program` with `EnumDef`/`MatchCase` AST nodes in Rust via
  `color_program_with_cases` and call `TypeChecker::check_program`
  directly. The positive case is folded into
  `typecheck_exhaustiveness_enum_or_pattern_test.nx` via concrete
  `type Color = Red | Green | Blue` surface syntax; the negative
  `Red`-only case is subsumed by the existing
  `exhaustiveness_or_pattern_missing_constructor.nx` fixture.

Skipped (Bucket C — proptest):

- `prop_enum_any_proper_subset_is_non_exhaustive`,
  `prop_bool_exhaustiveness` — proptest harness drives random subsets
  through the same Rust-AST builder. The deterministic positive cases
  are already covered by the surface-level ports above; the
  proptest's "every proper subset is non-exhaustive" coverage cannot
  be expressed without `--emit-json` for typecheck diagnostics.

## typecheck/effects.rs (batch 10)

Ported representative positives covering effect-row propagation,
selective catch, and exception groups. Negative tests deposited as
fixture pairs.

- `test_throws_propagation`, `test_call_pure_from_impure` →
  `typecheck_effects_throws_propagation_test.nx` (the polymorphic-apply
  form from `test_throws_polymorphism` is dropped: the surface-level
  pure_fn defaults its throws-row to `{}` and instantiating E={} from
  the polymorphic call site produces a `Row mismatch` in the self-host
  typechecker, which the original Rust harness silently accepted).
- `test_main_require_known_perm_is_accepted` →
  `typecheck_effects_main_require_known_perm_test.nx` (runs under
  `--allow-fs`, exercises `inject fs.system_handler` + a no-op
  `Fs.exists` call so the permissioned `main` is observable end-to-end
  rather than typecheck-only).
- `test_selective_catch_constructor_field_binding`,
  `test_selective_catch_multiple_exceptions` →
  `typecheck_effects_selective_catch_test.nx`. The unreachable
  `return -1` after `raise` is moved past `end` to dodge the wasm-
  codegen E2010 ("variable '__t2' has conflicting wasm local types")
  triggered by an i64 return statement immediately after a raise inside
  a try body. Tracked separately from this port.
- `test_exception_group_catch_matches_any_member`,
  `test_exception_group_catch_executes`,
  `test_exception_group_catch_second_member`,
  `test_exception_group_catch_with_wildcard` →
  `typecheck_effects_exception_group_test.nx`.

Skipped (`test_exception_group_throws_allows_member_raise`): wrapping
a `throws { IOError }` callee inside `throws { Exn }` raises a
row-mismatch at the call site in the bootstrap unifier even when the
group expands to Exn — documented in the file's header comment.

Negative `should_fail_typecheck` (no insta) ported as fixtures:

- `test_raise_requires_exn` → `effects_raise_without_exn.nx`.
- `test_main_cannot_declare_exn_throws`,
  `test_main_rejects_nonempty_throws` →
  `effects_main_cannot_declare_exn_throws.nx` (one canonical fixture
  per the bootstrap-side gate; the second test is covered by the
  same diagnostic and the same shape).
- `test_main_require_unknown_port_is_rejected`,
  `test_main_require_port_name_is_rejected` →
  `effects_main_require_unknown_port.nx` (one fixture; both probes
  surface the same "main function requires must be {...}"
  diagnostic).
- `test_throws_mismatch` → `effects_throws_mismatch.nx`.
- `test_throws_polymorphism_mismatch` →
  `effects_throws_polymorphism_mismatch.nx`.
- `test_selective_catch_wrong_field_type_fails` →
  `effects_selective_catch_wrong_field_type.nx`.

Skipped (Bucket C — insta::assert_snapshot):

- `test_main_must_return_unit`,
  `test_main_throws_net_only_is_rejected` — both use
  `insta::assert_snapshot!(err)`. Snapshot tests are Rust-harness-bound;
  the gate itself (main must return unit, main may not declare
  arbitrary throws) is exercised by the existing
  `main`-shape positives in batch-9 stdlib ports.

Skipped (Bucket C — proptest):

- `prop_polymorphic_id_accepts_i64`,
  `prop_polymorphic_id_rejects_return_mismatch`,
  `prop_effectful_call_with_perform_is_ok`,
  `prop_pure_call_without_perform_is_ok`,
  `prop_first_combinator_keeps_first_type`,
  `prop_named_argument_label_mismatch_is_error`,
  `prop_declared_pure_function_cannot_perform_io`,
  `prop_raise_without_exn_throws_is_error`,
  `prop_try_catch_with_io_handler_typechecks`,
  `prop_linear_array_borrow_then_drop_is_ok`,
  `prop_ref_write_then_read_typechecks`,
  `prop_ref_assignment_type_mismatch_is_error`,
  `prop_immutable_assignment_is_error`,
  `prop_linear_value_must_be_consumed_once`,
  `prop_linear_primitive_auto_drop_is_ok`,
  `prop_linear_double_consume_is_error`,
  `prop_linear_cannot_be_stored_in_ref`,
  `prop_linear_borrow_then_consume_is_ok`,
  `prop_linear_param_accepts_plain_value_via_weakening`,
  `prop_linear_branch_mismatch_is_error` — proptest sweeps over
  randomised `n: i64` / `b: bool` / `msg: string` inputs that can't be
  expressed in surface .nx without a Random handler; the deterministic
  contracts are already covered by hand-picked positive/negative
  examples in batch-7/8 inference and linear ports plus the new
  `typecheck_effects_*` files above.

## capabilities/permissions.rs (batch 10)

Bucket C in full. Every test calls `ExecutionCapabilities::deny_all()`
and `caps.validate_program_requires(...)` — capability validation is a
runtime API, not a Nexus-language surface. The .nx companion would have
to be driven from Rust to set deny/allow flags before exec; the
self-host equivalent will need to come through `nexus exec
--allow-net` / `--deny-net` flag wiring, which is currently absent
from the standalone harness.

- `static_capability_check_rejects_missing_net`,
  `static_capability_check_passes_when_net_allowed`,
  `static_capability_check_rejects_multiple_missing`,
  `no_requires_clause_passes_with_deny_all` — all four call
  `validate_program_requires` against an `ExecutionCapabilities` value
  constructed in Rust. No surface syntax. Skipped permanently.

## typecheck/ffi.rs (batch 10)

Bucket C in full. Every positive `should_typecheck` test references a
fictitious `import external "math.wasm"` / `"time.wasm"` /
`"core.wasm"` that has no resolvable WASM linkage target — the test is
implicitly typecheck-only by design. The single negative
(`test_ffi_unintroduced_type_var_errors`) is `insta::assert_snapshot!`.
Real FFI is exercised end-to-end via `import external "std:collections"`
in `tests/stdlib/stdlib_array_basic_test.nx`.

- `test_ffi_declaration`, `test_ffi_effectful`,
  `test_ffi_explicit_type_params`,
  `test_ffi_concrete_types_no_type_params_needed` — typecheck-only
  against fictitious extern WASM modules; no runtime exercise possible
  in standalone .nx form.
- `test_ffi_mismatch` — `should_fail_typecheck` without insta but the
  surface contract (mismatched arg type to extern) is the same as
  `inference_arg_mismatch.nx` already covered by batch 7.
- `test_ffi_unintroduced_type_var_errors` — `insta::assert_snapshot!`,
  Bucket C.

## typecheck/error_snapshots.rs (batch 10)

Bucket C in full (>95%). Every test in this file is structured as
`should_fail_typecheck` + `insta::assert_snapshot!(err)`. The whole
purpose of the file is to lock the diagnostic *text* via insta — by
definition that is Rust-harness-bound and not a Nexus-language
surface. The underlying error-shape contracts (type mismatch on
return / let / arg, undefined identifier, arity, linear-misuse, effect
leak, exhaustiveness, ref/mutability, capability missing, constructor
shape, implicit non-unit return) are all already covered by
deterministic fixture pairs scattered across batches 7/8 and the new
`typecheck_effects_*` / `typecheck_exhaustiveness_*` / `linearity_*`
fixtures above. The 18 snapshot tests provide no signal that the
fixture pairs do not.

- `snapshot_type_mismatch_return`,
  `snapshot_type_mismatch_let_annotation`,
  `snapshot_type_mismatch_function_arg`,
  `snapshot_type_mismatch_if_branches`,
  `snapshot_type_mismatch_match_arms`,
  `snapshot_undefined_variable`,
  `snapshot_undefined_function`,
  `snapshot_undefined_type`,
  `snapshot_too_few_args`,
  `snapshot_too_many_args`,
  `snapshot_wrong_arg_label`,
  `snapshot_linear_unconsumed`,
  `snapshot_linear_double_consume`,
  `snapshot_linear_branch_mismatch`,
  `snapshot_effect_leak_pure_calls_impure`,
  `snapshot_raise_without_throws`,
  `snapshot_match_non_exhaustive_option`,
  `snapshot_match_non_exhaustive_bool`,
  `snapshot_match_non_exhaustive_enum`,
  `snapshot_assign_to_immutable`,
  `snapshot_ref_type_mismatch_on_assign`,
  `snapshot_missing_permission`,
  `snapshot_unknown_constructor`,
  `snapshot_constructor_wrong_field_count`,
  `snapshot_non_unit_function_without_return` — all skipped as
  `insta::assert_snapshot!` Bucket C.

## typecheck/fuzz.rs (batch 10)

Bucket C in full. Every test wraps `parser::parser().parse(src)` +
`TypeChecker::check_program` inside `std::panic::catch_unwind` and
proptest-generates random ASCII / keyword soup / structurally-valid
fragments to assert the typechecker never panics. The contract is
"the Rust process did not abort" — there is no Nexus-language way to
observe a panic short of a deny-all wrap that the standalone
runtime does not provide. proptest generators are also Rust-only.

- `fuzz_random_ascii_no_panic`,
  `fuzz_random_keyword_soup_no_panic`,
  `fuzz_valid_let_binding_no_panic`,
  `fuzz_valid_function_def_no_panic`,
  `fuzz_deeply_nested_if_no_panic`,
  `fuzz_deeply_nested_match_no_panic`,
  `fuzz_many_params_no_panic`,
  `fuzz_many_type_params_no_panic`,
  `fuzz_enum_many_variants_no_panic` — all skipped as proptest +
  catch_unwind Bucket C.

# Skipped ports — bootstrap/tests/ir (batch 11)

## ir/{hir,mir,lir,lir_opt}.rs (30 tests)

Bucket C in full (with one fixturized exception, see below). `ir/*.rs`
either:

- Calls `nexus::ir::*::lower_*` directly and runs `insta::assert_snapshot!`
  on the resulting IR pretty-print (`snapshot_hir_*`, `snapshot_mir_*`,
  `snapshot_lir_*`) — IR shape and snapshot infra are Rust-only.
- Walks the LIR pre/post the optimizer pass to assert that pattern-binding
  references are preserved (`opt_preserves_pattern_binding_refs_*`,
  `opt_preserves_refs_in_*`, `opt_full_compile_*`) — these reach into
  `LirAtom`, `LirExpr`, `LirStmt`, `LirFunction` types directly to traverse
  refs/defs sets. No Nexus-language surface for "every Symbol referenced
  after optimization is also defined."
- Asserts mutual / branch tail-call lowering produces specific
  `LirExpr::TailCall` nodes (`mutual_recursion_produces_tail_call_in_lir`,
  `tail_call_in_if_else_branches`, `snapshot_lir_tail_call`) — IR
  introspection.

Fixturized:

- `ir/hir.rs::top_level_let_with_call_initializer_is_rejected` →
  `bootstrap/tests/fixtures/ir/top_level_let_with_call_initializer_rejected.nx`
  + `.expected_error.txt`. The Rust test pattern-matches
  `HirBuildError::UnsupportedTopLevelLet` directly; the fixture's
  `.expected_error.txt` substring-matches the offending binding name
  (`start_time`) which the error message must contain.

# Skipped ports — bootstrap/tests/nxc (batch 11)

## nxc/*.rs (67 tests)

The nxc/ tests run via the Rust harness in two modes:

1. `exec_with_stdlib(read_fixture("nxc/test_X.nx"))` — runs a fixture
   `.nx` file that exercises the **bootstrap (self-host) compiler in
   Nexus**. The vast majority of these fixtures `import "src/..."`
   modules (typecheck/types, frontend/parser, ir/hir_types, etc.) and
   construct in-language IR values to drive specific compiler passes.
   They are testing the bootstrap compiler the same way `ir/*.rs` tests
   the Rust compiler — Bucket C in spirit, since reproducing them under
   `tests/` would couple `tests/` to the bootstrap's source tree
   organization.
2. `compile_fixture_via_nxc{_should_fail}` — drives the bootstrap
   `nexus.wasm` binary from Rust and observes its stdout/stderr/exit
   code. Pure harness.
3. Some run via `exec_threaded` / `exec_with_stdlib_core` /
   `exec_nxc_core_capture_stdout` — additional Rust-side harnessing for
   shared-memory threading, core-wasm bypass of stdlib composition, and
   stdout capture, none of which has a `tests/`-runner analogue.

Ported (user-language regressions that don't import `src/`):

- `nxc/import_after_use::import_after_use_resolves` →
  `tests/nxc/nxc_import_after_use_resolves_test.nx`. Asserts canonical
  naming resolves a `str_length` reference in a function body even when
  the `import { length as str_length } from "std:str"` declaration
  appears below the use site.
- `nxc/transitive_wrapper::transitive_wrapper_resolves` →
  `tests/nxc/nxc_transitive_wrapper_resolves_test.nx`. Asserts
  Nexus-level wrapper functions (`bytebuffer.copy_range` over
  `__nx_copy_range`) survive reachability filtering and emit with their
  canonical name when called through a qualified import alias.
- `nxc/qualified_imports::qualified_imports_diamond_and_mixed` →
  `tests/nxc/nxc_qualified_imports_diamond_test.nx`. Asserts diamond
  imports of `std:list` / `std:str` resolve to a single registered
  function each (no forwarder stubs) when reachable through both direct
  use and transitive stdlib internal use.

Skipped:

- All fixtures importing `src/...`: `test_typecheck.nx`,
  `test_lambda_capture_linearity.nx`, `test_handler_first_class.nx`,
  `test_match_exhaustiveness.nx`, `test_throws_row_narrowing.nx`,
  `test_call_throw_row_subsumption.nx`, `test_lsp_diagnostics.nx`,
  `test_lsp_publish_diagnostics.nx`, `test_lsp_document_symbols.nx`,
  `test_lsp_multifile_imports.nx`, `test_infer_let_annotation.nx`,
  `test_codegen_minimal.nx`, `test_codegen_validate.nx`, `test_lir.nx`,
  `test_lir_minimal.nx`, `test_mir.nx`, `test_mir_minimal.nx`,
  `test_hir.nx`, `test_ast.nx`, `test_lexer.nx`, `test_parser.nx`,
  `test_parser_minimal.nx`, `test_parser_tokenize_only.nx`,
  `test_handler_arm_span.nx`, `test_rdrname.nx`, `test_resolve.nx`,
  `test_symtab.nx`. Bucket C — these fixtures load bootstrap compiler
  internal modules and exercise them as Nexus libraries; they are the
  self-host compiler's own test corpus and depend on the `src/` layout.
- `compile_fixture_via_nxc{_should_fail}` driven cases: every test in
  `nxc/codegen.rs` that reads back a `nxc_test_*.wasm` artifact and
  validates it via `wasmparser` / runs it via `wasmtime::Module`
  (`codegen_validate_wasm_output`, `exn_field_order_regression`,
  `funcref_arity2_handler_vtable_via_nxc`, `simd_autovectorize_acceptance`,
  `simd_lane_op_coverage_acceptance`, `wasm_alloc_pure_nexus_acceptance`,
  `string_pure_nexus_acceptance`, `collection_nx_parity_acceptance`,
  `runtime_mem_safe_oob_raises_specific_exception`,
  `runtime_mem_math_intrinsics_dispatch`,
  `math_pure_nexus_integer_ops_acceptance`,
  `compile_fixture_via_nxc_is_thread_safe`,
  `main_throws_unwrappable_rejected_by_self_host`,
  `main_throws_wrap_emits_variant_and_exits`,
  `nxc_f64_compare_in_main_entry_if`). The contract these tests verify
  is "the self-host bootstrap binary compiles this fixture and produces
  WASM with property X" — observable only through the Rust harness's
  subprocess + wasmparser inspection path.
- `exec_threaded` cases: `lazy_threaded_atomic_alloc`,
  `lazy_threaded_capture_bearing_forces`,
  `lazy_threaded_heap_reset_reclaims_worker_allocations`. The threaded
  exec harness wires up a host-imported shared memory + per-instance
  `LazyRuntime::with_shared_memory`; the standalone `nexus build`
  runtime does not provide this mode.
- `exec_with_stdlib_core_should_trap` cases: `chan_recv_before_send_traps`
  asserts the trap message contains "empty" — substring match on a trap
  reason is a Rust-side observation; the standalone runtime surfaces
  trap-vs-exit only.
- Capability-bearing-main fixtures (`require { Console }` /
  `require { PermConsole }`): `lazy_thunk_syntax`, `lazy_stdlib_combinators`
  exercise `std:lazy` combinators, `inject_try_catch_compiles` exercises
  `inject ... do try ... catch end end`, `nxc_f64_compare_in_main_entry_if`
  uses `Console.println` to surface 6 arms. The `safe_run` scaffold under
  `tests/` cannot wrap a capability-bearing main cleanly (same constraint
  documented under runtime/functions.rs and runtime/strings.rs in earlier
  batches).
- Snapshot tests: `dump_lir_minimal_wasm` writes WASM bytes to `/tmp` and
  prints byte counts — diagnostic-only, no assertion.

