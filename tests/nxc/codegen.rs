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

    let engine = wasmtime::Engine::default();

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

    // i64 return: main() should return 42
    {
        let bytes = std::fs::read("nxc_test_i64.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .unwrap();
        let result = main.call(&mut store, ()).unwrap();
        assert_eq!(result, 42, "i64 return: expected 42, got {}", result);
    }

    // function call: helper returns 7, main calls it
    {
        let bytes = std::fs::read("nxc_test_call.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .unwrap();
        let result = main.call(&mut store, ()).unwrap();
        assert_eq!(result, 7, "function call: expected 7, got {}", result);
    }

    // arithmetic: 3 + 4 = 7
    {
        let bytes = std::fs::read("nxc_test_arith.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .unwrap();
        let result = main.call(&mut store, ()).unwrap();
        assert_eq!(result, 7, "arithmetic: expected 7, got {}", result);
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

    // f64 literal: main() should return 3.14
    {
        let bytes = std::fs::read("nxc_test_f64.wasm").unwrap();
        let module = wasmtime::Module::from_binary(&engine, &bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let main = instance
            .get_typed_func::<(), f64>(&mut store, "main")
            .unwrap();
        let result = main.call(&mut store, ()).unwrap();
        assert!(
            (result - 3.14).abs() < 1e-10,
            "f64 literal: expected 3.14, got {}",
            result
        );
    }

    for path in &files {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn bytebuffer_minimal() {
    exec_with_stdlib(&read_fixture("nxc/test_bytebuffer_minimal.nx"));
}
