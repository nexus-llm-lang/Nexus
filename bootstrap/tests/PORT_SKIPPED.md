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

