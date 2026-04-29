use crate::harness::compile::{compile_fixture_via_nxc, compile_fixture_via_nxc_should_fail};
use crate::harness::{
    exec, exec_nxc_core, exec_nxc_core_capture_stderr_expecting_exit,
    exec_nxc_core_capture_stdout, exec_should_trap, exec_threaded,
    exec_with_stdlib, exec_with_stdlib_core, exec_with_stdlib_core_should_trap, read_fixture,
};

#[test]
fn codegen_minimal_wasm_output() {
    exec_with_stdlib(&read_fixture("nxc/test_codegen_minimal.nx"));
}

#[test]
fn codegen_validate_wasm_output() {
    exec_with_stdlib(&read_fixture("nxc/test_codegen_validate.nx"));

    let files = [
        "nxc_test_empty.wasm",
        "nxc_test_i64.wasm",
        "nxc_test_call.wasm",
        "nxc_test_arith.wasm",
        "nxc_test_if.wasm",
        "nxc_test_f64.wasm",
    ];
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {}", path, e));
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .unwrap_or_else(|e| {
                panic!("{} failed validation: {}", path, e);
            });
    }

    let engine = {
        let mut config = wasmtime::Config::new();
        config.wasm_tail_call(true);
        config.wasm_exceptions(true);
        config.wasm_function_references(true);
        config.wasm_stack_switching(true);
        wasmtime::Engine::new(&config).unwrap()
    };

    // empty main
    {
        let bytes = std::fs::read("nxc_test_empty.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .unwrap();
        main.call(&mut store, ()).unwrap();
    }

    // i64 return: nxc main always returns void (unit)
    {
        let bytes = std::fs::read("nxc_test_i64.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .unwrap();
        main.call(&mut store, ()).unwrap();
    }

    // function call: helper returns 7, main calls helper (void return)
    {
        let bytes = std::fs::read("nxc_test_call.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .unwrap();
        main.call(&mut store, ()).unwrap();
    }

    // arithmetic: 3 + 4 = 7 (void return)
    {
        let bytes = std::fs::read("nxc_test_arith.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .unwrap();
        main.call(&mut store, ()).unwrap();
    }

    // if statement
    {
        let bytes = std::fs::read("nxc_test_if.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .unwrap();
        main.call(&mut store, ()).unwrap();
    }

    // f64 literal: void return
    {
        let bytes = std::fs::read("nxc_test_f64.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .unwrap();
        main.call(&mut store, ()).unwrap();
    }

    for path in &files {
        let _ = std::fs::remove_file(path);
    }
}

/// Regression test for nexus-928: exception constructor fields must use sorted
/// (alphabetical) heap indices in pattern matching, not positional indices.
/// When exception defs were missing from enum_defs, fields like "phase" and
/// "message" got swapped because positional order != alphabetical order.
#[test]
fn exn_field_order_regression() {
    exec_with_stdlib(&read_fixture("nxc/test_exn_field_order.nx"));

    let path = "nxc_test_exn_field_order.wasm";
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {}", path, e));
    wasmparser::Validator::new()
        .validate_all(&bytes)
        .unwrap_or_else(|e| panic!("{} failed validation: {}", path, e));

    let engine = {
        let mut config = wasmtime::Config::new();
        config.wasm_tail_call(true);
        config.wasm_exceptions(true);
        config.wasm_function_references(true);
        config.wasm_stack_switching(true);
        wasmtime::Engine::new(&config).unwrap()
    };

    let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
    let mut store = wasmtime::Store::new(&engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .unwrap();
    // nxc main returns void; the test fixture validates field ordering internally
    // via print or assert before returning
    main.call(&mut store, ()).unwrap();

    let _ = std::fs::remove_file(path);
}

#[test]
fn bytebuffer_minimal() {
    exec_with_stdlib(&read_fixture("nxc/test_bytebuffer_minimal.nx"));
}

#[test]
fn lazy_thunk_syntax() {
    exec_with_stdlib(&read_fixture("nxc/test_lazy.nx"));
}

#[test]
fn lazy_stdlib_combinators() {
    exec_with_stdlib(&read_fixture("nxc/test_lazy_stdlib.nx"));
}

#[test]
fn lazy_host_force_core() {
    // Acceptance test for nexus-ug96: `host_force` invokes the real
    // `LazyRuntime` via core-wasm + WASI preview1, instead of going through
    // the component-model lazy stub (which returns (0,) and would make the
    // fixture's trap-on-mismatch fire). The fixture has no stdlib imports,
    // so `exec_with_stdlib_core` skips bundling and runs the core wasm
    // directly with `LazyRuntime::register` satisfying the lazy host
    // imports.
    exec_with_stdlib_core(&read_fixture("nxc/test_lazy_host_force_core.nx"));
}

#[test]
fn lazy_parallel_consecutive_forces_via_stdlib_path() {
    // Acceptance test for nexus-ug96: routes the LIR parallelize pass
    // fixture through `exec_with_stdlib_core` (the core-wasm + WASI
    // preview1 replacement for `exec_with_stdlib`). Confirms that a
    // *parallelized forced value* — `let v1 = @a; let v2 = @b` rewritten
    // by the pass into spawn+spawn+join+join — round-trips correctly
    // through the harness path that previously stubbed lazy to (0,).
    exec_with_stdlib_core(&read_fixture("nxc/test_lazy_parallel.nx"));
}

#[test]
#[ignore = "blocked on nexus-ug96 — fixture's trap-on-mismatch correctly fires \
            because the component-model lazy stub returns (0,) instead of \
            invoking the thunk; un-ignore when the stub or the test path is fixed."]
fn lazy_host_force() {
    exec_with_stdlib(&read_fixture("nxc/test_lazy_host_force.nx"));
}

#[test]
fn chan_recv_before_send_traps() {
    // Phase 1 of nexus-lim: in single-threaded execution, `recv` on an
    // empty cell traps (Phase 3 will replace this with a yield to the
    // scheduler). The fixture calls `recv` before `send` and the trap
    // message must mention "empty" so the failure mode is identifiable.
    let msg =
        exec_with_stdlib_core_should_trap(&read_fixture("nxc/test_chan_recv_before_send_traps.nx"));
    assert!(
        msg.contains("empty"),
        "expected runtime trap to mention 'empty', got: {msg}"
    );
}

#[test]
fn chan_oneshot_roundtrip() {
    // Acceptance test for nexus-lim (Phase 1 of the concurrency epic):
    // one-shot linear channels via `nexus:runtime/chan` host functions.
    // The fixture allocates a Pair(tx, rx), sends 42, recvs it back, and
    // traps via i64.div_s when the value differs. Routed through
    // `exec_with_stdlib_core` because the chan runtime imports are
    // satisfied by `ChanRuntime::register` (test harness only — same
    // wiring scope as the lazy runtime).
    exec_with_stdlib_core(&read_fixture("nxc/test_chan_oneshot.nx"));
}

#[test]
fn lazy_runtime_raw() {
    // Exercises __nx_lazy_spawn + __nx_lazy_join end-to-end on nxc-compiled
    // core WASM (no stdlib / component composition). The fixture self-asserts
    // by trapping (i64.div_s by zero) when the forced value differs from 42 —
    // prevents a regression where spawn's encoded task_id leaks back as the
    // "value".
    exec(&read_fixture("nxc/test_lazy_runtime_raw.nx"));
}

#[test]
fn lazy_parallel_consecutive_forces() {
    // Exercises the lir_opt parallelize_consecutive_forces pass via natural
    // Nexus syntax: `let v1 = @a; let v2 = @b` produces two adjacent
    // CallIndirect ops which the pass rewrites to spawn+spawn+join+join.
    // Run via the core-wasm exec path so worker threads actually run; the
    // fixture traps on a wrong sum.
    exec(&read_fixture("nxc/test_lazy_parallel.nx"));
}

#[test]
fn lazy_threaded_capture_bearing_forces() {
    // End-to-end test for nexus-tb6p slice 2: a fixture with capture-bearing
    // thunks (`let @z = y`, `let @w = y + 11`) compiled via the threaded
    // codegen path (host-imported shared memory) and run with
    // LazyRuntime::with_shared_memory. The parallel pass emits
    // spawn(num_captures=1)+spawn(num_captures=1)+join+join; without the
    // shared-memory mode those would take the inline fallback at runtime,
    // never threading capture-bearing thunks.
    exec_threaded(&read_fixture("nxc/test_lazy_threaded_captures.nx"));
}

#[test]
fn trap_probe_div_by_zero_actually_traps() {
    // Critical sanity check: confirms that `let _ = 1 / 0` (with the divisor
    // computed via a runtime if-expression) actually traps. If this regresses
    // to a silent pass, the LIR optimizer is dead-code-eliminating the trap
    // and every fixture relying on i64.div_s as an assertion mechanism becomes
    // a no-op (silently passes regardless of value correctness).
    let trap_msg = exec_should_trap(&read_fixture("nxc/test_trap_probe.nx"));
    // Wasmtime may not surface the specific trap kind in the message —
    // just confirm a wasm trap actually fired (i.e., the wasm backtrace).
    assert!(
        trap_msg.contains("wasm") || trap_msg.contains("trap") || trap_msg.contains("divide"),
        "expected a wasm trap, got: {trap_msg}"
    );
}

#[test]
fn lazy_threaded_atomic_alloc() {
    // End-to-end test for nexus-tb6p slice 3 (nexus-hqjy): the threaded
    // codegen path now imports `nexus:runtime/lazy::alloc` and routes all
    // heap allocations through it. The runtime maintains a host-side
    // AtomicI32 bump pointer shared across caller and every worker, so
    // concurrent allocations don't race on per-instance heap-pointer
    // globals. This fixture's main + thunk bodies all allocate (closure
    // records + Pair constructors) and the assertion verifies the result
    // is consistent under the atomic allocator.
    exec_threaded(&read_fixture("nxc/test_lazy_threaded_alloc.nx"));
}

#[test]
fn lazy_threaded_heap_reset_reclaims_worker_allocations() {
    // nexus-unf1: arena.heap_reset must rewind the host-side AtomicI32 bump
    // pointer in shared-memory mode, not silently no-op on the (unused) G0
    // global. Fixture allocates inside main + inside two threaded thunks,
    // joins both, calls heap_reset(mark0), then takes a fresh mark and
    // traps via i64.div_s if the second mark != mark0 — proving the host
    // pointer was actually wound back instead of leaking the worker
    // allocations.
    exec_threaded(&read_fixture("nxc/test_lazy_threaded_heap_reset.nx"));
}

#[test]
fn exception_group_catch() {
    exec_with_stdlib(&read_fixture("nxc/test_exception_group.nx"));
}

#[test]
fn handler_with_kont_resume() {
    exec_nxc_core("bootstrap/tests/fixtures/nxc/test_handler_with_kont_resume.nx");
}

#[test]
fn handler_kont_forget_is_linearity_error() {
    let err = compile_fixture_via_nxc_should_fail(
        "bootstrap/tests/fixtures/nxc/test_handler_kont_forget.nx",
    );
    assert!(
        err.contains("E2005"),
        "expected E2005 linearity error, got: {err}"
    );
}

#[test]
fn handler_kont_double_force_is_linearity_error() {
    let err = compile_fixture_via_nxc_should_fail(
        "bootstrap/tests/fixtures/nxc/test_handler_kont_double_force.nx",
    );
    assert!(
        err.contains("E2006"),
        "expected E2006 linearity error, got: {err}"
    );
}

#[test]
fn handler_kont_type_mismatch() {
    let err = compile_fixture_via_nxc_should_fail(
        "bootstrap/tests/fixtures/nxc/test_handler_kont_type_mismatch.nx",
    );
    assert!(
        err.contains("E2001"),
        "expected E2001 type mismatch, got: {err}"
    );
}

#[test]
fn sched_yield_roundtrip() {
    exec_nxc_core("bootstrap/tests/fixtures/nxc/test_sched_yield.nx");
}

/// Issue nexus-urf5: the self-host narrowed `validate_main` rejects rows
/// whose entries are not named exception (group) types (E2007
/// `MainThrowsTooBroad`). The wrap pass leaves unwrappable rows intact so
/// the typecheck rejection surfaces.
#[test]
fn main_throws_unwrappable_rejected_by_self_host() {
    let err = compile_fixture_via_nxc_should_fail(
        "bootstrap/tests/fixtures/nxc/test_main_throws_exn_rejected.nx",
    );
    assert!(
        err.contains("E2007"),
        "expected E2007 (MainThrowsTooBroad), got: {err}"
    );
}

/// Issue nexus-urf5: the self-host HIR `wrap_main_if_needed` pass synthesises
/// a top-level catch-all when `main` declares non-empty throws, so an
/// uncaught raise prints the variant tag + backtrace to stderr and exits
/// non-zero rather than aborting the wasm with an uncaught throw.
///
/// The fixture's `main` declares `throws { BoomGroup }` and raises
/// `Boom(code: 42)`. After the wrap pass, the synthesised wrapper:
///   - eprints `Boom(...)\n` (the variant tag)
///   - iterates `__nx_bt_depth()` entries calling `__nx_bt_frame` for each
///   - calls `__nx_exit(1)` (surfaces as WASI `proc_exit(1)`)
///
/// Self-host codegen now emits `call __nx_main_wrap_bt_capture` immediately
/// before each `op_throw` (issue nexus-55x0). The capture is gated
/// per-function on `EffectMap.can_throw`, so functions that cannot
/// transitively throw skip the call. With capture wired up, the runtime's
/// `BT_FRAMES` is populated and the wrapper's iteration loop emits at
/// least one `  at` line (the throwing user function frame).
#[test]
fn main_throws_wrap_emits_variant_and_exits() {
    let (stderr, exit_code) = exec_nxc_core_capture_stderr_expecting_exit(
        "bootstrap/tests/fixtures/nxc/test_main_throws_wrap.nx",
    );
    assert_eq!(
        exit_code, 1,
        "expected non-zero exit from synthesised wrapper, full stderr:\n{}",
        stderr
    );
    assert!(
        stderr.contains("Boom"),
        "expected stderr to mention the variant tag `Boom`, got:\n{}",
        stderr
    );
    // After nexus-55x0 the self-host emits `__nx_capture_backtrace` at raise
    // sites, so the wrapper's loop produces at least one `  at` line.
    let frame_lines = stderr.lines().filter(|l| l.starts_with("  at ")).count();
    assert!(
        frame_lines >= 1,
        "expected at least one `  at` frame line after capture-backtrace \
         landed (issue nexus-55x0); got {}: {}",
        frame_lines, stderr
    );
}

#[test]
fn sched_producer_consumer() {
    // Acceptance for nexus-bes5 #5b: producer-consumer using Phase 1 channels
    // + scheduler. Producer yields once then `send`s; consumer yields once
    // then `recv`s and `Console.println`s. Expected stdout: a single line "42".
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_sched_producer_consumer.nx",
    );
    let trimmed = out.trim();
    assert_eq!(
        trimmed, "42",
        "expected single line `42`, got — full stdout:\n{}",
        out
    );
}

/// Regression test for nexus-w9ne: bare `if FLOAT_LIT >. FLOAT_LIT then ... end`
/// at main entry. The self-host LIR `infer_binary_type` returned `lhs_type`
/// (TyF64) for OpFGt instead of TyBool, so the temp holding the comparison
/// result was allocated as f64 and the `local.set` of the i32 produced by
/// `f64.gt` into an f64 local failed wasmtime validation with a 'type
/// mismatch' at the function's entry block. Running the fixture exercises
/// every f64 relational operator (`>.`, `<.`, `>=.`, `<=.`, `==.`, `!=.`).
#[test]
fn nxc_f64_compare_in_main_entry_if() {
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_codegen_f64_cmp_if.nx",
    );
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    assert_eq!(
        lines,
        ["gt", "lt", "ge", "le", "eq", "ne"],
        "unexpected stdout — {:?}",
        out
    );
}

/// Regression test for nexus-gyj6: `compile_fixture_via_nxc` previously
/// derived its output path from `std::process::id()` alone, so all parallel
/// test threads inside the same `cargo test` process shared the *same*
/// `/tmp/nxc_test_<pid>.wasm` and overwrote each other's compiled fixture.
/// This test compiles three distinct fixtures concurrently from N worker
/// threads — if path-uniquing per call is broken, at least one thread reads
/// back wasm bytes a sibling wrote (different stdout when executed) and the
/// `assert_eq!` fails.
#[test]
fn compile_fixture_via_nxc_is_thread_safe() {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::thread;

    // Pre-resolve fixture bytes serially once so we have a baseline to
    // compare each thread's parallel compile against. Parallel runs that
    // disagree with the serial baseline = path collision.
    let fixtures = [
        "bootstrap/tests/fixtures/nxc/test_sched_yield.nx",
        "bootstrap/tests/fixtures/nxc/test_sched_producer_consumer.nx",
        "bootstrap/tests/fixtures/nxc/test_sched_multi_fiber.nx",
    ];
    // Force chdir-to-repo-root via an upfront serial compile (also seeds
    // the `Once` inside `ensure_repo_root` before threads race on it).
    let baselines: HashMap<&str, Vec<u8>> = fixtures
        .iter()
        .map(|f| (*f, compile_fixture_via_nxc(f)))
        .collect();
    let baselines = Arc::new(baselines);

    // 4 rounds × 3 fixtures = 12 concurrent compiles. Empirically enough
    // to expose the shared-path race on a 10-core M-series machine when
    // the bug is reintroduced; the correct fix passes deterministically.
    const ROUNDS: usize = 4;
    let mut handles = Vec::new();
    for round in 0..ROUNDS {
        for (i, fixture) in fixtures.iter().enumerate() {
            let baselines = Arc::clone(&baselines);
            let fixture = *fixture;
            handles.push(thread::spawn(move || {
                let bytes = compile_fixture_via_nxc(fixture);
                let expected = baselines.get(fixture).unwrap();
                assert_eq!(
                    bytes.len(),
                    expected.len(),
                    "round={round} idx={i} fixture={fixture}: \
                     parallel compile produced wasm of length {} but serial \
                     baseline was {} — likely path-collision overwrite by \
                     another thread",
                    bytes.len(),
                    expected.len()
                );
                assert_eq!(
                    bytes, *expected,
                    "round={round} idx={i} fixture={fixture}: parallel \
                     compile produced different bytes than serial baseline"
                );
            }));
        }
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

#[test]
fn sched_multi_fiber_spawn() {
    // Acceptance for nexus-bes5 #5a: spawn 3 fibers each yielding 2 times,
    // observe round-robin interleaving via Console.println. Empty stdout =
    // fibers never ran (the queue-based scheduler silently dropped them).
    let out = exec_nxc_core_capture_stdout("bootstrap/tests/fixtures/nxc/test_sched_multi_fiber.nx");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.len(),
        9,
        "expected 9 lines (3 fibers x 3 prints), got {} — full stdout:\n{}",
        lines.len(),
        out
    );
    // Each fiber must print all 3 of its phases.
    for fiber in &["f1", "f2", "f3"] {
        for phase in &["a", "b", "c"] {
            let needle = format!("{}.{}", fiber, phase);
            assert!(
                lines.iter().any(|l| l.trim() == needle),
                "missing line {:?} in stdout — full:\n{}",
                needle,
                out
            );
        }
    }
    // Round-robin interleaving: the first occurrence of f1.a, f2.a, f3.a
    // must precede the first occurrence of any *.b. (Cooperative single-step
    // scheduling implies all 3 fibers print phase `a` before any prints `b`.)
    let pos = |needle: &str| lines.iter().position(|l| l.trim() == needle).unwrap();
    let first_b = [pos("f1.b"), pos("f2.b"), pos("f3.b")]
        .iter()
        .copied()
        .min()
        .unwrap();
    for fiber in &["f1", "f2", "f3"] {
        let a = pos(&format!("{}.a", fiber));
        assert!(
            a < first_b,
            "fiber {}.a (line {}) ran after the first .b (line {}) — scheduler isn't round-robin yielding\nfull:\n{}",
            fiber, a, first_b, out
        );
    }
}

/// nexus-dvr6.9.1: safe_* mem wrappers must raise the catchable
/// MemoryOutOfBounds exception (op, addr, access_size) instead of trapping
/// or surfacing a generic RuntimeError. Verifies the partial-function
/// exception contract for the runtime/memory FFI surface.
#[test]
fn runtime_mem_safe_oob_raises_specific_exception() {
    let out =
        exec_nxc_core_capture_stdout("bootstrap/tests/fixtures/nxc/test_runtime_mem_safe_oob_exn.nx");
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    // Caller catches MemoryOutOfBounds → prints op name, addr, access_size for
    // the load case, and op + access_size for the store case.
    assert!(
        lines.iter().any(|l| *l == "load: safe_load_i32"),
        "missing load exception line — full stdout:\n{}",
        out
    );
    assert!(
        lines.iter().any(|l| *l == "store: safe_store_i64"),
        "missing store exception line — full stdout:\n{}",
        out
    );
    assert!(
        lines.iter().any(|l| *l == "4"),
        "missing access_size=4 line for load i32 — full stdout:\n{}",
        out
    );
    assert!(
        lines.iter().any(|l| *l == "8"),
        "missing access_size=8 line for store i64 — full stdout:\n{}",
        out
    );
    assert!(
        !out.contains("BUG: "),
        "safe_* wrapper returned without raising — full stdout:\n{}",
        out
    );
}

/// nexus-dvr6.9.1: generalized intrinsic dispatch — codegen.nx now inline-emits
/// `nexus:runtime/memory` (8 ops) and `nexus:runtime/math` (4 f64 unary ops)
/// in addition to the existing arena externals. Acceptance:
///   - mem-load-i64 + mem-store-i64 round-trip with explicit address
///   - mem-size = N → mem-grow(1) → mem-size = N+1
///   - f64-sqrt(4.0) = 2.0, f64-floor(2.7) = 2.0, f64-ceil(2.3) = 3.0,
///     f64-abs(-3.5) = 3.5, f64-abs(-30.0) = 30.0
///   - i32 / u8 round-trips and mem-grow return path also exercised
/// Stdout-interleave assertion (assert_stdout-style) catches both value
/// drift and any silently-dropped intrinsic dispatch (a missing print
/// fails the line-count check, not a quiet PASS).
#[test]
fn runtime_mem_math_intrinsics_dispatch() {
    let out =
        exec_nxc_core_capture_stdout("bootstrap/tests/fixtures/nxc/test_runtime_mem_math_intrinsics.nx");
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    let expected = [
        "305419896", // (1) mem-load-i32 round-trip
        "171",       // (2) mem-load-u8 round-trip
        "9223372036854775000", // (3) mem-load-i64 round-trip
        "1",         // (4) mem-size delta after grow(1)
        "2",         // f64-sqrt(4.0)
        "2",         // f64-floor(2.7)
        "3",         // f64-ceil(2.3)
        "3",         // f64-abs(-3.5) → 3 after truncating to i64
        "30",        // f64-abs(-30.0)
    ];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines[i], *want,
            "line {} mismatch: want {:?}, got {:?} — full stdout:\n{}",
            i, want, lines[i], out
        );
    }
}

/// nexus-dvr6.9.4 acceptance: math.nx integer ops (abs/max/min/mod_i64) compile
/// and execute end-to-end without going through the host `__nx_*_i64` FFI
/// (they are pure-Nexus implementations using only language primitives), and
/// `mod_i64` raises a specific RuntimeError on division by zero rather than
/// silently returning `i64::MIN` like the prior Rust impl. The fixture also
/// exercises sqrt/floor/ceil/abs_float so the runtime_math intrinsic dispatch
/// (already covered by `runtime_mem_math_intrinsics_dispatch`) keeps working
/// when called via the higher-level math.nx wrappers.
#[test]
fn math_pure_nexus_integer_ops_acceptance() {
    exec_nxc_core("bootstrap/tests/fixtures/nxc/test_math_pure_nexus_integer_ops.nx");
}

/// nexus-thby acceptance: 128-bit SIMD primitives (`i32x4` / `i64x2` /
/// `f32x4` / `f64x2` add+mul) and the autovectorized `i32x4_add_array` /
/// `f32x4_add_array` loop intrinsics dispatch through codegen.nx as inline
/// 0xFD-prefixed wasm and produce correct lane-wise results.
///
/// Two-pronged check (mirrors the runtime_mem_math intrinsic test):
///   1. Stdout-interleave: every printed line is one lane / one
///      autovectorize-equality count, so a missing or mis-encoded SIMD op
///      surfaces as a value or line-count mismatch.
///   2. Bytecode scan: the compiled wasm bytes must contain the SIMD-prefix
///      sequence (0xFD) followed by at least one of the lane sub-opcodes
///      we expect (e.g. `i32x4.add` = sub 0xAE → encoded as `FD AE 01`).
///      The scan rejects a regression where dispatch silently falls back
///      to a function call, which would still pass the stdout check but
///      produce zero SIMD instructions in the output module.
fn count_simd_op_uses(wasm: &[u8], sub_op: u32) -> usize {
    // SIMD encoding: 0xFD prefix byte followed by ULEB128 sub-opcode. For
    // sub-ops < 128 the ULEB is one byte; for >= 128 it is two bytes
    // (low7 with continuation bit set, then high7).
    let mut count = 0;
    let mut i = 0;
    while i + 1 < wasm.len() {
        if wasm[i] == 0xFD {
            // Decode the next ULEB128 (max 5 bytes for u32).
            let mut val: u32 = 0;
            let mut shift = 0;
            let mut j = i + 1;
            while j < wasm.len() {
                let b = wasm[j];
                val |= u32::from(b & 0x7F) << shift;
                j += 1;
                if b & 0x80 == 0 {
                    break;
                }
                shift += 7;
                if shift >= 32 {
                    break;
                }
            }
            if val == sub_op {
                count += 1;
            }
        }
        i += 1;
    }
    count
}

#[test]
fn simd_autovectorize_acceptance() {
    let fixture = "bootstrap/tests/fixtures/nxc/test_simd_autovectorize.nx";

    // Bytecode-side check first — proves the dispatch table emits SIMD ops,
    // not just that the program ran. Without this, a regression that elided
    // every `0xFD` from emit_simd_* would still pass the stdout check
    // (because a function-call fallback returning the right value would
    // print the same lines).
    let wasm = compile_fixture_via_nxc(fixture);
    // i32x4.add (sub 0xAE = 174) — used by both the single-quad ops and the
    // i32x4_add_array intrinsic, so must appear at least twice.
    let n_i32x4_add = count_simd_op_uses(&wasm, 0xAE);
    assert!(
        n_i32x4_add >= 2,
        "expected at least 2 i32x4.add (0xAE) uses, found {n_i32x4_add} — \
         dispatch may have fallen back to function calls"
    );
    // f32x4.add (sub 0xE4 = 228) — used by the f32x4_add_array intrinsic,
    // so must appear at least once.
    let n_f32x4_add = count_simd_op_uses(&wasm, 0xE4);
    assert!(
        n_f32x4_add >= 1,
        "expected at least 1 f32x4.add (0xE4) use, found {n_f32x4_add}"
    );
    // i64x2.add (sub 0xCE = 206) — used by the single-pair op.
    let n_i64x2_add = count_simd_op_uses(&wasm, 0xCE);
    assert!(
        n_i64x2_add >= 1,
        "expected at least 1 i64x2.add (0xCE) use, found {n_i64x2_add}"
    );

    // Runtime-side: every line is the answer for one lane / equality count.
    // A missing or mis-encoded SIMD op surfaces here as a value or count
    // mismatch (silently-dropped op fails the line-count guard).
    let out = exec_nxc_core_capture_stdout(fixture);
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let expected = [
        // (1) i32x4_add: lanes [10,20,30,40] + [1,2,3,4]
        "11", "22", "33", "44",
        // (2) i32x4_mul: same operands → [10,40,90,160]
        "10", "40", "90", "160",
        // (3) i64x2_add: [1000,2000] + [7,11]
        "1007", "2011",
        // (4) i64x2_mul: same operands
        "7000", "22000",
        // (5) i32x4_add_array vs scalar_i32_add_array agreement (64 bytes)
        "64",
        // (6) f32x4_add_array determinism (same input → same 64-byte output)
        "64",
    ];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines[i], *want,
            "line {i} mismatch: want {want:?}, got {:?} — full stdout:\n{out}",
            lines[i]
        );
    }
}

/// nexus-iesh: extended lane-op coverage for the autovectorized SIMD
/// array intrinsics (`mul`/`sub` for all four lane shapes plus `div` for
/// the two float shapes). Same two-pronged check as the autovectorize
/// MVP test:
///   1. Bytecode scan: each new sub-opcode (i32x4.sub = 0xB1, i64x2.sub
///      = 0xD1, f32x4.sub/div = 0xE5/0xE7, f64x2.sub/div = 0xF1/0xF3)
///      must appear at least once in the compiled wasm. Catches a
///      regression where dispatch silently falls back to a function
///      call — the runtime check would still pass for the `f*` ops
///      (determinism gate) but no 0xFD-prefixed instruction would be
///      emitted.
///   2. Runtime: each printed line is a 64-byte agreement count between
///      the SIMD path and either a scalar reference (i32/i64) or a
///      second SIMD pass on the same input (f32/f64 — no scalar f-arith
///      reference at this layer). 64 means perfect byte-for-byte match.
///
/// Per-lane integer division does not exist in the WebAssembly SIMD
/// proposal, so `i32x4_div_array` / `i64x2_div_array` are intentionally
/// absent from both the stdlib surface and this test.
#[test]
fn simd_lane_op_coverage_acceptance() {
    let fixture = "bootstrap/tests/fixtures/nxc/test_simd_lane_op_coverage.nx";

    // Bytecode-side: each new sub-opcode must be present in the compiled
    // module. Existing mul opcodes (0xB5, 0xD5, 0xE6, 0xF2) are also
    // re-checked here because the new `_mul_array` autovectorize variants
    // are the first call sites for f64x2.mul / i64x2.mul outside of the
    // single-quad MVP path.
    let wasm = compile_fixture_via_nxc(fixture);

    // (i32x4.mul = sub 0xB5) — used by i32x4_mul_array.
    let n_i32x4_mul = count_simd_op_uses(&wasm, 0xB5);
    assert!(
        n_i32x4_mul >= 1,
        "expected >= 1 i32x4.mul (0xB5) use, found {n_i32x4_mul}"
    );
    // (i64x2.mul = sub 0xD5) — used by i64x2_mul_array.
    let n_i64x2_mul = count_simd_op_uses(&wasm, 0xD5);
    assert!(
        n_i64x2_mul >= 1,
        "expected >= 1 i64x2.mul (0xD5) use, found {n_i64x2_mul}"
    );
    // (f32x4.mul = sub 0xE6) — used by f32x4_mul_array.
    let n_f32x4_mul = count_simd_op_uses(&wasm, 0xE6);
    assert!(
        n_f32x4_mul >= 1,
        "expected >= 1 f32x4.mul (0xE6) use, found {n_f32x4_mul}"
    );
    // (f64x2.mul = sub 0xF2) — used by f64x2_mul_array.
    let n_f64x2_mul = count_simd_op_uses(&wasm, 0xF2);
    assert!(
        n_f64x2_mul >= 1,
        "expected >= 1 f64x2.mul (0xF2) use, found {n_f64x2_mul}"
    );
    // (i32x4.sub = sub 0xB1) — new in iesh.
    let n_i32x4_sub = count_simd_op_uses(&wasm, 0xB1);
    assert!(
        n_i32x4_sub >= 1,
        "expected >= 1 i32x4.sub (0xB1) use, found {n_i32x4_sub} \
         — dispatch may have fallen back to a function call"
    );
    // (i64x2.sub = sub 0xD1) — new in iesh.
    let n_i64x2_sub = count_simd_op_uses(&wasm, 0xD1);
    assert!(
        n_i64x2_sub >= 1,
        "expected >= 1 i64x2.sub (0xD1) use, found {n_i64x2_sub}"
    );
    // (f32x4.sub = sub 0xE5) — new in iesh.
    let n_f32x4_sub = count_simd_op_uses(&wasm, 0xE5);
    assert!(
        n_f32x4_sub >= 1,
        "expected >= 1 f32x4.sub (0xE5) use, found {n_f32x4_sub}"
    );
    // (f32x4.div = sub 0xE7) — new in iesh.
    let n_f32x4_div = count_simd_op_uses(&wasm, 0xE7);
    assert!(
        n_f32x4_div >= 1,
        "expected >= 1 f32x4.div (0xE7) use, found {n_f32x4_div}"
    );
    // (f64x2.sub = sub 0xF1) — new in iesh.
    let n_f64x2_sub = count_simd_op_uses(&wasm, 0xF1);
    assert!(
        n_f64x2_sub >= 1,
        "expected >= 1 f64x2.sub (0xF1) use, found {n_f64x2_sub}"
    );
    // (f64x2.div = sub 0xF3) — new in iesh.
    let n_f64x2_div = count_simd_op_uses(&wasm, 0xF3);
    assert!(
        n_f64x2_div >= 1,
        "expected >= 1 f64x2.div (0xF3) use, found {n_f64x2_div}"
    );

    // Runtime: 10 lines, each an agreement count of 64 bytes — one per
    // array intrinsic exercised in the fixture. Order matches the fixture
    // (i32 mul, i32 sub, i64 mul, i64 sub, f32 mul/sub/div, f64 mul/sub/div).
    let out = exec_nxc_core_capture_stdout(fixture);
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let expected = ["64"; 10];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines[i], *want,
            "line {i} mismatch: want {want:?}, got {:?} — full stdout:\n{out}",
            lines[i]
        );
    }
}

/// Regression for nexus-pt8g: arity-N (N >= 1) top-level fn references
/// stored as record fields (handler-vtable shape) used to trip
/// `lookup_closure_type_idx_pairs` with E3001 because
/// `collect_closure_arities` walks only `__closure_*` definitions and
/// never registered the call_indirect type for the field-load → call
/// shape. Post-fix, the MIR pass lifts each value-position reference
/// to a known top-level fn into a `__closure_wrap_<target>` thunk
/// (mirroring bootstrap/src/compiler/passes/lir_lower.rs::closure_convert),
/// so the closure machinery (closure_table population, arity-N type
/// dedup) carries the wrapper and the call_indirect resolves.
///
/// `exec_nxc_core_capture_stdout` exercises the nxc codegen path so the
/// regression covers compilation **and** runtime semantics
/// (`add=7 mul=12` proves the wrapper forwards the call to the right
/// target, not just that the wasm builds).
#[test]
fn funcref_arity2_handler_vtable_via_nxc() {
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_funcref_arity2_handler_vtable.nx",
    );
    assert!(
        out.contains("add=7 mul=12"),
        "expected 'add=7 mul=12' (wrapper forwards through Vt2 vtable) but got: {out}"
    );
}

/// nexus-dvr6.9.2 acceptance: pure-Nexus `nxlib/stdlib/wasm_alloc.nx` bump
/// allocator compiles + runs end-to-end via the self-hosted compiler.
/// The fixture covers all four acceptance assertions in one stdout-interleave
/// check so a silently-dropped intrinsic dispatch fails the line count, not
/// a quiet PASS:
///   1. allocate alignment (two consecutive 24-byte allocations are 32 bytes apart)
///   2. memory.grow trigger when the request would exceed the state-cell address
///   3. mark + reset round-trip leaves the bump pointer at the saved mark
///   4. 1000-iteration alloc + reset workload keeps the bump pointer bounded
///      (RSS-bounded — the same shape as the gv2u arena workloads, but
///      driven through the new pure-Nexus allocator instead of routed-stdlib)
///   5. store_string_result + read_string round-trip recovers the original
///      string via raw byte access through `runtime_mem` load_u8 / store_u8.
/// Path X (pure bump, no per-allocation tracking) per the issue body's
/// "deallocate can be no-op for bump alloc" — full audit in the commit
/// body and in `bd note nexus-dvr6.9.2`.
#[test]
fn wasm_alloc_pure_nexus_acceptance() {
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_wasm_alloc_pure_nexus.nx",
    );
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    let expected = [
        "ok-align",
        "ok-grow",
        "ok-reset",
        "ok-bounded",
        "ok-string",
    ];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines[i], *want,
            "line {} mismatch: want {:?}, got {:?} — full stdout:\n{}",
            i, want, lines[i], out
        );
    }
}

/// nexus-dvr6.9.3 acceptance: pure-Nexus `nxlib/stdlib/string.nx` covers the
/// 30+ ops formerly served by `bootstrap/src/lib/string/src/lib.rs` —
/// length, contains, substring, index_of, starts_with, ends_with,
/// replace, repeat, pad_left, pad_right, split, join, trim, to_upper,
/// to_lower, from_i64, to_i64, is_valid_i64, from_bool, to_f64,
/// is_valid_f64, char_at_strict — across ASCII / 2-byte / 3-byte /
/// 4-byte UTF-8 boundaries plus parity-vs-Rust on representative
/// inputs. The fixture also asserts the partial-functions raise
/// specific Exn variants (InvalidIndex on OOB char_at, RuntimeError on
/// to_i64 of non-digits) per memory `feedback_partial_functions_raise`.
///
/// Full stdout interleave is asserted (line count + content), matching
/// the wasm_alloc_pure_nexus_acceptance idiom — a silently-dropped
/// intrinsic dispatch fails the line count, not a quiet PASS.
#[test]
fn string_pure_nexus_acceptance() {
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_string_pure_nexus.nx",
    );
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    let expected = [
        // case_length
        "ok len-ascii",
        "ok len-latin1",
        "ok len-cjk",
        "ok len-emoji",
        "ok len-mixed",
        // case_predicates
        "ok contains-yes",
        "ok contains-no",
        "ok starts-yes",
        "ok starts-no",
        "ok ends-yes",
        "ok ends-no",
        "ok indexof-found",
        "ok indexof-missing",
        // case_substring
        "ok substr-ascii",
        "ok substr-cjk",
        "ok substr-emoji",
        // case_transform
        "ok upper-ascii",
        "ok lower-ascii",
        "ok upper-cjk-preserves",
        "ok trim-ws",
        "ok trim-mixed-ws",
        "ok replace-multi",
        "ok repeat-3",
        "ok repeat-0",
        "ok pad-left",
        "ok pad-right",
        // case_split_join
        "ok split-3",
        "ok split-no-match",
        "ok join-3",
        "ok join-empty",
        // case_conversions
        "ok i64-zero",
        "ok i64-pos",
        "ok i64-neg",
        "ok i64-big",
        "ok to_i64-pos",
        "ok to_i64-neg",
        "ok to_i64-trim",
        "ok valid-i64-yes",
        "ok valid-i64-no",
        "ok bool-true",
        "ok bool-false",
        "ok to_f64-frac",
        "ok to_f64-exp",
        "ok to_f64-neg",
        "ok valid-f64-yes",
        "ok valid-f64-no",
        // case_parity_vs_rust (10+ cases comparing pure-Nexus vs Rust FFI)
        "ok parity-length",
        "ok parity-contains",
        "ok parity-index_of",
        "ok parity-substring",
        "ok parity-starts_with",
        "ok parity-ends_with",
        "ok parity-to_upper",
        "ok parity-to_lower",
        "ok parity-replace",
        "ok parity-from_i64-pos",
        "ok parity-from_i64-neg",
        "ok parity-from_bool",
        // partial-function raises
        "ok char_at_strict-oob",
        "ok to_i64-raises",
    ];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines[i], *want,
            "line {} mismatch: want {:?}, got {:?} — full stdout:\n{}",
            i, want, lines[i], out
        );
    }
}

/// nexus-dvr6.9.5 acceptance: pure-Nexus collection rewrites
/// (`hashmap_nx`, `set_nx`, `stringmap_nx`, `bytebuffer_nx`) backed by
/// the open-addressed `collection_table` over `wasm_alloc` +
/// `runtime_mem`.  Ten cases drive insert/get/remove, growth across
/// the load-factor threshold, set algebra, partial-function exception
/// discipline (KeyNotFound / IndexOutOfBounds — never RuntimeError),
/// and ByteBuffer push/grow/get round-trips.  Stdout is checked
/// line-by-line to catch silently-dropped `nexus:runtime/memory`
/// dispatch (same failure mode dvr6.9.2 protects against).
#[test]
fn collection_nx_parity_acceptance() {
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_collection_nx_parity.nx",
    );
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    let expected = [
        "ok-hashmap-basic",
        "ok-hashmap-grow",
        "ok-hashmap-unchecked",
        "ok-set-basic",
        "ok-set-algebra",
        "ok-stringmap-basic",
        "ok-stringmap-grow",
        "ok-bytebuffer-basic",
        "ok-bytebuffer-grow",
        "ok-bytebuffer-unchecked",
    ];
    assert_eq!(
        lines.len(),
        expected.len(),
        "expected {} lines, got {} — full stdout:\n{}",
        expected.len(),
        lines.len(),
        out
    );
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            lines[i], *want,
            "line {} mismatch: want {:?}, got {:?} — full stdout:\n{}",
            i, want, lines[i], out
        );
    }
}
