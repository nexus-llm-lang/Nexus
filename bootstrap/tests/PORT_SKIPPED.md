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
