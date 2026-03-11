//! WASM compilation + execution helpers.

use nexus::compiler::codegen::compile_program_to_wasm;
use nexus::lang::parser;
use nexus::lang::stdlib::resolve_import_path;
use nexus::runtime::backtrace;
use nexus::runtime::conc;
use std::path::PathBuf;
use std::sync::Arc;
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

/// Parse + compile to core WASM bytes. Panics on failure.
pub fn compile(src: &str) -> Vec<u8> {
    let program = parser::parser()
        .parse(src)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    compile_program_to_wasm(&program).unwrap_or_else(|e| panic!("compile failed: {}", e))
}

/// Parse + compile to core WASM bytes. Returns error on failure.
pub fn try_compile(src: &str) -> Result<Vec<u8>, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    compile_program_to_wasm(&program).map_err(|e| e.to_string())
}

/// Compile and execute main(). Panics if main traps or compilation fails.
pub fn exec(src: &str) {
    let wasm = compile(src);
    run_main(&wasm).unwrap_or_else(|e| panic!("execution failed: {}", e));
}

/// Compile and execute main() with full WASI + stdlib.
/// Use this when the program imports stdlib modules.
pub fn exec_with_stdlib(src: &str) {
    let wasm = compile(src);
    run_main_with_deps(&wasm).unwrap_or_else(|e| panic!("execution failed: {}", e));
}

/// Compile and execute, expecting a trap. Returns the trap message.
pub fn exec_should_trap(src: &str) -> String {
    let wasm = compile(src);
    match run_main(&wasm) {
        Ok(()) => panic!("expected trap but main returned successfully"),
        Err(msg) => msg,
    }
}

/// Execute main() -> () on raw WASM bytes (no WASI, no stdlib).
fn run_main(wasm: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let has_conc = conc::needs_conc_runtime(wasm);
    let has_bt = backtrace::needs_bt_runtime(wasm);

    if has_conc || has_bt {
        let module = if has_conc {
            Arc::new(module)
        } else {
            Arc::new(module)
        };
        if has_conc {
            conc::setup_conc_runtime(
                engine.clone(),
                module.clone(),
                wasm,
                vec![],
                nexus::runtime::ExecutionCapabilities::permissive_legacy(),
            );
        }
        let mut linker = Linker::new(&engine);
        if has_conc {
            conc::add_conc_to_linker(&mut linker).map_err(|e| e.to_string())?;
        }
        if has_bt {
            backtrace::reset();
            backtrace::add_bt_to_linker(&mut linker)?;
        }
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &*module)
            .map_err(|e| e.to_string())?;
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .map_err(|e| e.to_string())?;
        return main.call(&mut store, ()).map_err(|e| e.to_string());
    }

    let mut store = Store::new(&engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

/// Execute main() -> () with WASI P1, stdlib dependency resolution, and conc runtime.
fn run_main_with_deps(wasm: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| format!("{:#}", e))?;
    let has_conc = conc::needs_conc_runtime(wasm);

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;
    define_nexus_host_stubs(&mut linker)?;
    if has_conc {
        conc::add_conc_to_linker(&mut linker)?;
    }
    // Always add backtrace to linker — stdlib modules may import it
    backtrace::reset();
    backtrace::add_bt_to_linker(&mut linker)?;

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    let _ = builder.preopened_dir(".", "/", DirPerms::all(), FilePerms::all());
    let wasi = builder.build_p1();
    let mut store = Store::new(&engine, wasi);

    let mut imported_modules = module
        .imports()
        .map(|i| i.module().to_string())
        .collect::<Vec<_>>();
    imported_modules.sort();
    imported_modules.dedup();

    let mut deps = Vec::new();
    for module_name in imported_modules {
        if module_name == "wasi_snapshot_preview1"
            || module_name == conc::CONC_HOST_MODULE
            || module_name == backtrace::BT_HOST_MODULE
            || module_name == "nexus:cli/nexus-host"
        {
            continue;
        }
        let resolved = resolve_import_path(&module_name);
        let path = PathBuf::from(&resolved);
        let dep = Arc::new(Module::from_file(&engine, &path).map_err(|e| e.to_string())?);
        linker
            .module(&mut store, &module_name, &*dep)
            .map_err(|e| e.to_string())?;
        deps.push((module_name.clone(), dep));
    }

    if has_conc {
        conc::setup_conc_runtime(
            engine.clone(),
            Arc::new(module.clone()),
            wasm,
            deps,
            nexus::runtime::ExecutionCapabilities::permissive_legacy(),
        );
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

/// Stub implementations for nexus-host functions (trapping).
fn define_nexus_host_stubs(
    linker: &mut Linker<wasmtime_wasi::p1::WasiP1Ctx>,
) -> Result<(), String> {
    const MOD: &str = "nexus:cli/nexus-host";
    linker
        .func_wrap(
            MOD,
            "host-http-request",
            |_: wasmtime::Caller<'_, _>,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32| {},
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-listen",
            |_: wasmtime::Caller<'_, _>, _: i32, _: i32| -> i64 { -1 },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-accept",
            |_: wasmtime::Caller<'_, _>, _: i64, _: i32| {},
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-respond",
            |_: wasmtime::Caller<'_, _>, _: i64, _: i64, _: i32, _: i32, _: i32, _: i32| -> i32 {
                0
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-stop",
            |_: wasmtime::Caller<'_, _>, _: i64| -> i32 { 0 },
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read a test fixture from `tests/fixtures/`.
pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}", name))
        .unwrap_or_else(|e| panic!("fixture tests/fixtures/{} should exist: {}", name, e))
}

/// RAII guard for temporary directories — cleans up on drop.
pub struct TempDir {
    path: String,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        Self {
            path: format!("/tmp/nexus_test_{}_{}", label, nanos),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
