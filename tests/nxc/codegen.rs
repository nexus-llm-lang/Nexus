use crate::harness::{exec_with_stdlib, read_fixture};

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
fn exception_group_catch() {
    exec_with_stdlib(&read_fixture("nxc/test_exception_group.nx"));
}
