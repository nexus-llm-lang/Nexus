use nexus::lang::stdlib::resolve_import_to_file;
use nexus::runtime::backtrace;
use std::path::PathBuf;
use std::sync::Arc;
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

// ---------------------------------------------------------------------------
// Shared Engine & stdlib Module cache
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

static SHARED_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    Engine::new(&config).expect("failed to create shared engine")
});

static DEP_MODULE_CACHE: LazyLock<Mutex<HashMap<String, Arc<Module>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached_dep_module(module_name: &str) -> Arc<Module> {
    let mut cache = DEP_MODULE_CACHE.lock().unwrap();
    if let Some(m) = cache.get(module_name) {
        return Arc::clone(m);
    }
    let resolved = resolve_import_to_file(module_name);
    // Cache by resolved file path — multiple WIT module names
    // (e.g. nexus:stdlib/math, nexus:stdlib/stdio) map to the same stdlib.wasm.
    if let Some(m) = cache.get(&resolved) {
        let m = Arc::clone(m);
        cache.insert(module_name.to_string(), m.clone());
        return m;
    }
    let path = PathBuf::from(&resolved);
    let module = Arc::new(
        Module::from_file(&*SHARED_ENGINE, &path)
            .unwrap_or_else(|e| panic!("failed to load dep module {}: {}", module_name, e)),
    );
    cache.insert(resolved, Arc::clone(&module));
    cache.insert(module_name.to_string(), Arc::clone(&module));
    module
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compile and execute main(). Panics if main traps or compilation fails.
pub fn exec(src: &str) {
    let wasm = super::compile::compile(src);
    run_main(&wasm).unwrap_or_else(|e| panic!("execution failed: {}", e));
}

/// Compile and execute main() with full WASI + stdlib.
pub fn exec_with_stdlib(src: &str) {
    let wasm = super::compile::compile(src);
    run_main_with_deps(&wasm).unwrap_or_else(|e| panic!("execution failed: {}", e));
}

/// Compile and execute, expecting a trap. Returns the trap message.
pub fn exec_should_trap(src: &str) -> String {
    let wasm = super::compile::compile(src);
    match run_main(&wasm) {
        Ok(()) => panic!("expected trap but main returned successfully"),
        Err(msg) => msg,
    }
}

/// Execute main() -> () on raw WASM bytes (no WASI, no stdlib).
pub fn run_main(wasm: &[u8]) -> Result<(), String> {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let has_bt = backtrace::needs_bt_runtime(wasm);

    if has_bt {
        let mut linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        backtrace::reset();
        backtrace::add_bt_to_linker(&mut linker, &mut store)?;
        let instance = linker
            .instantiate(&mut store, &module)
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

/// Compile and execute main() with stdlib, expecting a trap. Returns the trap message.
pub fn exec_with_stdlib_should_trap(src: &str) -> String {
    let wasm = super::compile::compile(src);
    match run_main_with_deps(&wasm) {
        Ok(()) => panic!("expected trap but main returned successfully"),
        Err(msg) => msg,
    }
}

/// Compile and execute main() with stdlib and specific capabilities.
/// Panics if main traps or compilation fails.
pub fn exec_with_stdlib_caps(src: &str, caps: nexus::runtime::ExecutionCapabilities) {
    let wasm = super::compile::compile(src);
    run_main_with_deps_caps(&wasm, caps).unwrap_or_else(|e| panic!("execution failed: {}", e));
}

/// Compile and execute main() with stdlib and specific capabilities, expecting a trap.
pub fn exec_with_stdlib_caps_should_trap(
    src: &str,
    caps: nexus::runtime::ExecutionCapabilities,
) -> String {
    let wasm = super::compile::compile(src);
    match run_main_with_deps_caps(&wasm, caps) {
        Ok(()) => panic!("expected trap but main returned successfully"),
        Err(msg) => msg,
    }
}

/// Execute main() -> () with WASI P1, stdlib dependency resolution, and backtrace runtime.
/// Uses cached Engine and Module instances for stdlib dependencies.
pub fn run_main_with_deps(wasm: &[u8]) -> Result<(), String> {
    let engine = &*SHARED_ENGINE;
    let module = Module::from_binary(engine, wasm).map_err(|e| format!("{:#}", e))?;

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;
    define_nexus_host_stubs(&mut linker)?;

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    let _ = builder.preopened_dir(".", "/", DirPerms::all(), FilePerms::all());
    let wasi = builder.build_p1();
    let mut store = Store::new(engine, wasi);

    backtrace::reset();
    backtrace::add_bt_to_linker(&mut linker, &mut store)?;

    link_dep_modules(&module, &mut linker, &mut store)?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

/// Execute main() -> () with WASI P1, stdlib, and custom capability enforcement.
pub fn run_main_with_deps_caps(
    wasm: &[u8],
    caps: nexus::runtime::ExecutionCapabilities,
) -> Result<(), String> {
    let engine = &*SHARED_ENGINE;
    let module = Module::from_binary(engine, wasm).map_err(|e| format!("{:#}", e))?;

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;
    caps.enforce_denied_wasi_functions(&mut linker)?;
    define_nexus_host_stubs(&mut linker)?;

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    let _ = builder.preopened_dir(".", "/", DirPerms::all(), FilePerms::all());
    let wasi = builder.build_p1();
    let mut store = Store::new(engine, wasi);

    backtrace::reset();
    backtrace::add_bt_to_linker(&mut linker, &mut store)?;

    link_dep_modules(&module, &mut linker, &mut store)?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

/// Link dependency modules, deduplicating instances for modules that resolve
/// to the same file (e.g. `nexus:stdlib/math` and `nexus:stdlib/stdio` both
/// resolve to `nxlib/stdlib/stdlib.wasm`).
fn link_dep_modules(
    module: &Module,
    linker: &mut Linker<wasmtime_wasi::p1::WasiP1Ctx>,
    store: &mut Store<wasmtime_wasi::p1::WasiP1Ctx>,
) -> Result<(), String> {
    let mut imported_modules = module
        .imports()
        .map(|i| i.module().to_string())
        .collect::<Vec<_>>();
    imported_modules.sort();
    imported_modules.dedup();

    // Track which file paths have been instantiated to share a single instance
    // across multiple WIT module names that resolve to the same file.
    let mut instantiated_files: HashMap<String, String> = HashMap::new();

    for module_name in imported_modules {
        if module_name == "wasi_snapshot_preview1"
            || module_name == backtrace::BT_HOST_MODULE
            || module_name == "nexus:cli/nexus-host"
        {
            continue;
        }
        let resolved = resolve_import_to_file(&module_name);
        if let Some(first_name) = instantiated_files.get(&resolved) {
            // Already instantiated under a different name — alias exports
            // from the first instance under this module name.
            let first_name = first_name.clone();
            let dep = cached_dep_module(&module_name);
            for export in dep.exports() {
                if let Some(def) = linker.get(&mut *store, &first_name, export.name()) {
                    linker
                        .define(&mut *store, &module_name, export.name(), def)
                        .map_err(|e| format!("alias define error: {}", e))?;
                }
            }
            continue;
        }
        let dep = cached_dep_module(&module_name);
        linker
            .module(&mut *store, &module_name, &*dep)
            .map_err(|e| e.to_string())?;
        instantiated_files.insert(resolved, module_name.clone());
    }
    Ok(())
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
