use nexus::compiler::compose;
use nexus::runtime::backtrace;
use std::sync::Arc;
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable, WasiCtxBuilder};

// ---------------------------------------------------------------------------
// Shared Engine cache
// ---------------------------------------------------------------------------

use std::sync::LazyLock;
use std::sync::Mutex;

static SHARED_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_component_model(true);
    Engine::new(&config).expect("failed to create shared engine")
});

/// Cached composed stdlib component bytes.
/// `compose_with_stdlib` encodes the stdlib component every time; caching
/// the composed result per user-WASM is impractical, but we can cache the
/// stdlib component encoding.
static STDLIB_COMPONENT_CACHE: LazyLock<Mutex<Option<Arc<Vec<u8>>>>> =
    LazyLock::new(|| Mutex::new(None));

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

// ---------------------------------------------------------------------------
// Component model execution
// ---------------------------------------------------------------------------

/// WASI state for the component model store.
struct WasiState {
    ctx: wasmtime_wasi::WasiCtx,
    table: ResourceTable,
}

impl wasmtime_wasi::WasiView for WasiState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

/// Execute main() -> () with stdlib via component model composition.
/// The user core WASM is composed with stdlib-component into a single
/// component, then run with wasmtime's component model runtime.
pub fn run_main_with_deps(wasm: &[u8]) -> Result<(), String> {
    let composed = compose::compose_with_stdlib(wasm)
        .map_err(|e| format!("composition failed: {}", e))?;
    run_composed_component(&composed)
}

/// Execute main() -> () with stdlib and custom capability enforcement.
pub fn run_main_with_deps_caps(
    wasm: &[u8],
    _caps: nexus::runtime::ExecutionCapabilities,
) -> Result<(), String> {
    // TODO: enforce capabilities on the component model path.
    // For now, compose and run without enforcement.
    let composed = compose::compose_with_stdlib(wasm)
        .map_err(|e| format!("composition failed: {}", e))?;
    run_composed_component(&composed)
}

/// Run a pre-composed component WASM, providing WASI imports.
fn run_composed_component(component_wasm: &[u8]) -> Result<(), String> {
    let engine = &*SHARED_ENGINE;
    let component = wasmtime::component::Component::from_binary(engine, component_wasm)
        .map_err(|e| format!("failed to load component: {}", e))?;

    let mut linker = wasmtime::component::Linker::<WasiState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
    define_component_nexus_host_stubs(&mut linker)?;
    define_component_runtime_stubs(&mut linker)?;

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    let _ = builder.preopened_dir(".", ".", DirPerms::all(), FilePerms::all());
    let state = WasiState {
        ctx: builder.build(),
        table: ResourceTable::new(),
    };
    let mut store = Store::new(engine, state);

    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|e| format!("instantiation failed: {}", e))?;

    // The composed component exports `main` as a top-level function.
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| format!("failed to get main export: {}", e))?;

    main.call(&mut store, ()).map_err(|e| format!("{:#}", e))
}

/// Define stub (no-op / trapping) implementations for nexus:cli/nexus-host
/// in the component linker. The stdlib always imports this interface (from the
/// net sub-crate), but most tests don't use networking.
fn define_component_nexus_host_stubs(
    linker: &mut wasmtime::component::Linker<WasiState>,
) -> Result<(), String> {
    let mut inst = linker
        .instance("nexus:cli/nexus-host")
        .map_err(|e| format!("failed to create nexus-host instance: {}", e))?;
    inst.func_wrap(
        "host-http-request",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_method, _url, _headers, _body): (String, String, String, String)|
         -> wasmtime::Result<(String,)> { Ok((String::new(),)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-listen",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_addr,): (String,)|
         -> wasmtime::Result<(i64,)> { Ok((-1,)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-accept",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_server_id,): (i64,)|
         -> wasmtime::Result<(String,)> { Ok((String::new(),)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-respond",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_req_id, _status, _headers, _body): (i64, i64, String, String)|
         -> wasmtime::Result<(i32,)> { Ok((0,)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-stop",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_server_id,): (i64,)|
         -> wasmtime::Result<(i32,)> { Ok((0,)) },
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Define stub implementations for nexus:runtime/* imports in the component linker.
fn define_component_runtime_stubs(
    linker: &mut wasmtime::component::Linker<WasiState>,
) -> Result<(), String> {
    // Backtrace stubs — no-op capture, always 0 depth/frames.
    {
        let mut inst = linker
            .instance("nexus:runtime/backtrace")
            .map_err(|e| format!("failed to create backtrace instance: {}", e))?;
        inst.func_wrap(
            "capture-backtrace",
            |_: wasmtime::StoreContextMut<'_, WasiState>, (): ()| -> wasmtime::Result<()> {
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;
        inst.func_wrap(
            "bt-depth",
            |_: wasmtime::StoreContextMut<'_, WasiState>, (): ()| -> wasmtime::Result<(i64,)> {
                Ok((0,))
            },
        )
        .map_err(|e| e.to_string())?;
        inst.func_wrap(
            "bt-frame",
            |_: wasmtime::StoreContextMut<'_, WasiState>,
             (_idx,): (i64,)|
             -> wasmtime::Result<(i64,)> { Ok((0,)) },
        )
        .map_err(|e| e.to_string())?;
    }
    // Lazy evaluation stubs — return 0 (no actual parallelism in test harness).
    {
        let mut inst = linker
            .instance("nexus:runtime/lazy")
            .map_err(|e| format!("failed to create lazy instance: {}", e))?;
        inst.func_wrap(
            "lazy-spawn",
            |_: wasmtime::StoreContextMut<'_, WasiState>,
             (_thunk, _env_size): (i64, i32)|
             -> wasmtime::Result<(i64,)> { Ok((0,)) },
        )
        .map_err(|e| e.to_string())?;
        inst.func_wrap(
            "lazy-join",
            |_: wasmtime::StoreContextMut<'_, WasiState>,
             (_task_id,): (i64,)|
             -> wasmtime::Result<(i64,)> { Ok((0,)) },
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
