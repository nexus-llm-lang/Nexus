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
