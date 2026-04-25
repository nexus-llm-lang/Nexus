use crate::harness::{
    exec, exec_should_trap, exec_threaded, exec_with_stdlib, exec_with_stdlib_core, read_fixture,
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
        trap_msg.contains("wasm")
            || trap_msg.contains("trap")
            || trap_msg.contains("divide"),
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
fn exception_group_catch() {
    exec_with_stdlib(&read_fixture("nxc/test_exception_group.nx"));
}
