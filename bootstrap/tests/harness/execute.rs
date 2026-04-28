use nexus::compiler::bundler;
use nexus::compiler::compose;
use nexus::runtime::backtrace;
use nexus::runtime::chan;
use nexus::runtime::lazy;
use nexus::runtime::sched;
use nexus::runtime::ExecutionCapabilities;
use std::sync::Arc;
use std::time::Duration;
use wasmtime::{Engine, Linker, MemoryType, Module, SharedMemory, Store, WasmBacktrace};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable, WasiCtxBuilder};

// ---------------------------------------------------------------------------
// Shared Engine cache
// ---------------------------------------------------------------------------

use std::cell::RefCell;
use std::sync::LazyLock;
use std::sync::Mutex;

thread_local! {
    static COMPONENT_BT_FRAMES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

static SHARED_ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_function_references(true);
    config.wasm_stack_switching(true);
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

/// Compile and execute via the threaded core-wasm path: program is compiled
/// with `compile_program_to_wasm_threaded` (which emits
/// `(import "env" "memory" (memory N M shared))`), and the harness wires up a
/// host-owned `wasmtime::SharedMemory` plus a `LazyRuntime::with_shared_memory`
/// so that any capture-bearing thunks the LIR pass parallelizes actually
/// dispatch to worker threads instead of taking the inline fallback. Test-only
/// path for nexus-tb6p slice 2.
pub fn exec_threaded(src: &str) {
    let (wasm, heap_base) = super::compile::compile_threaded(src);
    run_main_threaded(&wasm, heap_base)
        .unwrap_or_else(|e| panic!("threaded execution failed: {}", e));
}

fn run_main_threaded(wasm: &[u8], heap_base: i32) -> Result<(), String> {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_function_references(true);
    config.wasm_stack_switching(true);
    config.wasm_threads(true);
    config.shared_memory(true);
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;

    // Build a SharedMemory with the same shape the codegen emits in
    // `compile_lir_to_wasm_threaded`. The host side keeps an Arc-clone so it
    // survives across the caller's Store and every worker's Store.
    let mem_type = MemoryType::shared(1, 65536);
    let shared_mem = SharedMemory::new(&engine, mem_type).map_err(|e| e.to_string())?;

    let runtime = lazy::LazyRuntime::with_shared_memory(
        engine.clone(),
        module.clone(),
        shared_mem.clone(),
        heap_base,
    );
    let mut linker = Linker::new(&engine);
    runtime.register(&mut linker)?;
    let mut store = Store::new(&engine, ());
    linker
        .define(
            &mut store,
            lazy::SHARED_MEMORY_MODULE,
            lazy::SHARED_MEMORY_FIELD,
            shared_mem.clone(),
        )
        .map_err(|e| format!("define shared memory: {e}"))?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

/// Compile and execute via the core-wasm + WASI preview1 path.
///
/// Routes the program through `wasmtime_wasi::p1::add_to_linker_sync` instead
/// of the component model, so `nexus:runtime/lazy` resolves to the real
/// `LazyRuntime` (which can read user memory and call indirectly through
/// `__indirect_function_table`) rather than the component-model stub that
/// returns `(0,)`. Lets fixtures that depend on actual thunk invocation —
/// e.g. the LIR `parallelize_consecutive_forces` pass output, or
/// `lazy.host_force` calls — assert real forced values end-to-end.
///
/// Skips stdlib bundling when the program has no `nexus:std/*` imports
/// (the binaryen `wasm-merge` path is unfit for stdlib-using programs
/// because it leaves stdlib's data section in a separate memory from the
/// caller's; covered in the implementation comment).
pub fn exec_with_stdlib_core(src: &str) {
    let wasm = super::compile::compile(src);
    run_main_with_deps_core(&wasm).unwrap_or_else(|e| panic!("core execution failed: {}", e));
}

/// Compile a fixture file via the self-hosted compiler (nexus.wasm) and
/// execute via the core-wasm WASI path. Use this for features that only the
/// self-hosted compiler supports (e.g. `with @k` handler arms).
pub fn exec_nxc_core(fixture_relpath: &str) {
    let wasm = super::compile::compile_fixture_via_nxc(fixture_relpath);
    run_main_with_deps_core(&wasm).unwrap_or_else(|e| panic!("nxc core execution failed: {}", e));
}

/// Compile + run via the self-hosted compiler core path AND capture stdout
/// into a String. Use this when the test must verify *what* the program
/// printed, not merely that it didn't trap. The default `exec_nxc_core`
/// inherits stdio, so a wasm that exits cleanly with empty stdout passes
/// silently — that's exactly the "4th virtual close" failure mode the
/// scheduler tests need to avoid (a multi-fiber fixture with empty stdout
/// is *not* "working", it's "fibers never ran").
pub fn exec_nxc_core_capture_stdout(fixture_relpath: &str) -> String {
    let wasm = super::compile::compile_fixture_via_nxc(fixture_relpath);
    run_main_with_deps_core_capture(&wasm)
        .unwrap_or_else(|e| panic!("nxc core execution failed: {}", e))
}

fn run_main_with_deps_core_capture(wasm: &[u8]) -> Result<String, String> {
    let imports = bundler::module_import_names(wasm)?;
    let needs_stdlib_bundle = imports
        .iter()
        .any(|m| nexus::lang::stdlib::is_package_wit_module(m));
    let bundled = if needs_stdlib_bundle {
        let cfg = bundler::BundleConfig::default();
        let merged = bundler::bundle_core_wasm(wasm, &cfg)?;
        bundler::merge_remaining_stubs(&merged, &cfg.wasm_merge_command)?
    } else {
        wasm.to_vec()
    };

    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_function_references(true);
    config.wasm_stack_switching(true);
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let module = Module::from_binary(&engine, &bundled).map_err(|e| e.to_string())?;

    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut WasiP1Ctx| s)
        .map_err(|e| format!("p1 linker: {e}"))?;

    if lazy::needs_lazy_runtime(&bundled) {
        let runtime = lazy::LazyRuntime::new(engine.clone(), module.clone());
        runtime.register(&mut linker)?;
    }
    let chan_runtime = if chan::needs_chan_runtime(&bundled) {
        let rt = chan::ChanRuntime::new();
        rt.register(&mut linker)?;
        Some(rt)
    } else {
        None
    };
    if sched::needs_sched_runtime(&bundled) {
        sched::SchedRuntime::new().register(&mut linker)?;
    }

    // 64KB stdout buffer — overkill for fixtures but cheap. Returned-as-String
    // means assertions can compare against a golden output line-by-line.
    let stdout_pipe = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(64 * 1024);

    let mut builder = WasiCtxBuilder::new();
    builder.stdout(stdout_pipe.clone());
    builder.inherit_stderr();
    let _ = builder.preopened_dir(".", ".", DirPerms::all(), FilePerms::all());
    let p1_ctx = builder.build_p1();
    let mut store = Store::new(&engine, p1_ctx);

    if backtrace::needs_bt_runtime(&bundled) {
        backtrace::reset();
        backtrace::add_bt_to_linker(&mut linker, &mut store)?;
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {e:#}"))?;
    let entry = if let Ok(start) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
        start
    } else {
        instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .map_err(|e| format!("get main export: {e}"))?
    };
    let call_result = entry.call(&mut store, ());

    // nexus-ygxg: drain chan cells unconditionally so a trap between
    // oneshot and recv does not leak a cell past instance teardown.
    if let Some(rt) = &chan_runtime {
        rt.teardown();
    }

    call_result.map_err(|e| format!("{e:#}"))?;

    // Drop the store first so any buffered stdout is flushed via the
    // MemoryOutputPipe clone we kept around.
    drop(store);
    let bytes = stdout_pipe.contents();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Same as `exec_with_stdlib_core` but expects a runtime trap. Returns the
/// trap message so the caller can pattern-match on it.
pub fn exec_with_stdlib_core_should_trap(src: &str) -> String {
    let wasm = super::compile::compile(src);
    match run_main_with_deps_core(&wasm) {
        Ok(()) => panic!("expected trap but main returned successfully"),
        Err(msg) => msg,
    }
}

fn run_main_with_deps_core(wasm: &[u8]) -> Result<(), String> {
    // `bundle_core_wasm` (binaryen `wasm-merge`) keeps each module's memory
    // separate (`--enable-multimemory`), so stdlib's `__nx_print` reads bytes
    // from stdlib's memory while the caller wrote them into the user's
    // memory. The Nexus self-hosted `wasm/merge.nx` reconciles this by
    // relocating user data above stdlib's region in a single shared memory;
    // reproducing that here is out of scope. Skip bundling when the user
    // wasm has no stdlib imports — programs that only touch
    // `nexus:runtime/*` (lazy, backtrace) and WASI run cleanly as-is.
    let imports = bundler::module_import_names(wasm)?;
    let needs_stdlib_bundle = imports
        .iter()
        .any(|m| nexus::lang::stdlib::is_package_wit_module(m));
    let bundled = if needs_stdlib_bundle {
        let cfg = bundler::BundleConfig::default();
        let merged = bundler::bundle_core_wasm(wasm, &cfg)?;
        bundler::merge_remaining_stubs(&merged, &cfg.wasm_merge_command)?
    } else {
        wasm.to_vec()
    };

    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_function_references(true);
    config.wasm_stack_switching(true);
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let module = Module::from_binary(&engine, &bundled).map_err(|e| e.to_string())?;

    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut WasiP1Ctx| s)
        .map_err(|e| format!("p1 linker: {e}"))?;

    if lazy::needs_lazy_runtime(&bundled) {
        let runtime = lazy::LazyRuntime::new(engine.clone(), module.clone());
        runtime.register(&mut linker)?;
    }

    let chan_runtime = if chan::needs_chan_runtime(&bundled) {
        let rt = chan::ChanRuntime::new();
        rt.register(&mut linker)?;
        Some(rt)
    } else {
        None
    };

    if sched::needs_sched_runtime(&bundled) {
        sched::SchedRuntime::new().register(&mut linker)?;
    }

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    let _ = builder.preopened_dir(".", ".", DirPerms::all(), FilePerms::all());
    let p1_ctx = builder.build_p1();
    let mut store = Store::new(&engine, p1_ctx);

    if backtrace::needs_bt_runtime(&bundled) {
        backtrace::reset();
        backtrace::add_bt_to_linker(&mut linker, &mut store)?;
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {e:#}"))?;

    // WASI command modules are entered via `_start`; fall back to `main` for
    // hand-crafted modules that only export `main`.
    let entry = if let Ok(start) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
        start
    } else {
        instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .map_err(|e| format!("get main export: {e}"))?
    };
    let call_result = entry.call(&mut store, ()).map_err(|e| format!("{e:#}"));

    // nexus-ygxg: drain chan cells unconditionally so a trap between
    // oneshot and recv does not leak past instance teardown.
    if let Some(rt) = &chan_runtime {
        rt.teardown();
    }

    call_result
}

/// Execute main() -> () on raw WASM bytes (no WASI, no stdlib).
pub fn run_main(wasm: &[u8]) -> Result<(), String> {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_function_references(true);
    config.wasm_stack_switching(true);
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let has_bt = backtrace::needs_bt_runtime(wasm);
    let has_lazy = lazy::needs_lazy_runtime(wasm);
    let has_chan = chan::needs_chan_runtime(wasm);
    let has_sched = sched::needs_sched_runtime(wasm);

    if has_bt || has_lazy || has_chan || has_sched {
        let mut linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        if has_bt {
            backtrace::reset();
            backtrace::add_bt_to_linker(&mut linker, &mut store)?;
        }
        if has_lazy {
            // Real threaded runtime — worker threads get their own
            // Store+Instance of the same Module. Zero-capture thunks run
            // on workers; captures fall back to inline on the caller.
            let runtime = lazy::LazyRuntime::new(engine.clone(), module.clone());
            runtime.register(&mut linker)?;
        }
        let chan_runtime = if has_chan {
            let rt = chan::ChanRuntime::new();
            rt.register(&mut linker)?;
            Some(rt)
        } else {
            None
        };
        if has_sched {
            sched::SchedRuntime::new().register(&mut linker)?;
        }
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| e.to_string())?;
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .map_err(|e| e.to_string())?;
        let call_result = main.call(&mut store, ()).map_err(|e| e.to_string());
        // nexus-ygxg: drain chan cells unconditionally so a trap between
        // oneshot and recv does not leak past instance teardown.
        if let Some(rt) = &chan_runtime {
            rt.teardown();
        }
        return call_result;
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

/// Compile and execute main() with stdlib and the given WASI env vars injected.
/// Allows tests to observe deterministic env state without mutating the host
/// process env (which would race under cargo's parallel test runner).
pub fn exec_with_stdlib_envs(src: &str, envs: &[(&str, &str)]) {
    let wasm = super::compile::compile(src);
    run_main_with_deps_envs(&wasm, envs).unwrap_or_else(|e| panic!("execution failed: {}", e));
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

// ---------------------------------------------------------------------------
// Denying WASI subsystem implementations (Approach B)
// ---------------------------------------------------------------------------

/// Clock implementation that traps on any access.
struct DenyingClock;
impl wasmtime_wasi::HostWallClock for DenyingClock {
    fn resolution(&self) -> Duration {
        panic!("clock access denied by capability enforcement")
    }
    fn now(&self) -> Duration {
        panic!("clock access denied by capability enforcement")
    }
}
impl wasmtime_wasi::HostMonotonicClock for DenyingClock {
    fn resolution(&self) -> u64 {
        panic!("clock access denied by capability enforcement")
    }
    fn now(&self) -> u64 {
        panic!("clock access denied by capability enforcement")
    }
}

/// Build a WasiCtx that traps on denied capabilities.
fn build_wasi_ctx(caps: &ExecutionCapabilities) -> wasmtime_wasi::WasiCtx {
    build_wasi_ctx_with_envs(caps, &[])
}

fn build_wasi_ctx_with_envs(
    caps: &ExecutionCapabilities,
    envs: &[(&str, &str)],
) -> wasmtime_wasi::WasiCtx {
    let mut builder = WasiCtxBuilder::new();
    if caps.allow_console {
        builder.inherit_stdio();
    }
    if caps.allow_fs {
        let _ = builder.preopened_dir(".", ".", DirPerms::all(), FilePerms::all());
    }
    if !caps.allow_clock {
        builder.wall_clock(DenyingClock);
        builder.monotonic_clock(DenyingClock);
    }
    if !caps.allow_random {
        builder.secure_random(DenyingRandom);
        builder.insecure_random(DenyingRandom);
    }
    for (k, v) in envs {
        builder.env(k, v);
    }
    builder.build()
}

/// RNG that traps on any access.
struct DenyingRandom;
impl wasmtime_wasi::RngCore for DenyingRandom {
    fn next_u32(&mut self) -> u32 {
        panic!("random access denied by capability enforcement")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("random access denied by capability enforcement")
    }
    fn fill_bytes(&mut self, _dest: &mut [u8]) {
        panic!("random access denied by capability enforcement")
    }
    fn try_fill_bytes(&mut self, _dest: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("random access denied by capability enforcement")
    }
}

// ---------------------------------------------------------------------------
// Component model execution
// ---------------------------------------------------------------------------

/// Execute main() -> () with stdlib via component model composition.
pub fn run_main_with_deps(wasm: &[u8]) -> Result<(), String> {
    let composed =
        compose::compose_with_stdlib(wasm).map_err(|e| format!("composition failed: {}", e))?;
    run_composed_component(&composed, &ExecutionCapabilities::allow_all())
}

/// Execute main() -> () with stdlib and custom capability enforcement.
pub fn run_main_with_deps_caps(wasm: &[u8], caps: ExecutionCapabilities) -> Result<(), String> {
    let composed =
        compose::compose_with_stdlib(wasm).map_err(|e| format!("composition failed: {}", e))?;
    run_composed_component(&composed, &caps)
}

/// Execute main() with stdlib and a custom WASI env set.
pub fn run_main_with_deps_envs(wasm: &[u8], envs: &[(&str, &str)]) -> Result<(), String> {
    let composed =
        compose::compose_with_stdlib(wasm).map_err(|e| format!("composition failed: {}", e))?;
    run_composed_component_with_envs(&composed, &ExecutionCapabilities::allow_all(), envs)
}

/// Run a pre-composed component WASM, providing WASI imports.
fn run_composed_component(
    component_wasm: &[u8],
    caps: &ExecutionCapabilities,
) -> Result<(), String> {
    run_composed_component_with_envs(component_wasm, caps, &[])
}

fn run_composed_component_with_envs(
    component_wasm: &[u8],
    caps: &ExecutionCapabilities,
    envs: &[(&str, &str)],
) -> Result<(), String> {
    let engine = &*SHARED_ENGINE;
    let component = wasmtime::component::Component::from_binary(engine, component_wasm)
        .map_err(|e| format!("failed to load component: {}", e))?;

    let mut linker = wasmtime::component::Linker::<WasiState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
    define_component_nexus_host_stubs(&mut linker)?;
    define_component_runtime_stubs(&mut linker)?;

    let state = WasiState {
        ctx: build_wasi_ctx_with_envs(caps, envs),
        table: ResourceTable::new(),
    };
    let mut store = Store::new(engine, state);

    // Wrap instantiation + execution in catch_unwind: denying WASI subsystems
    // panic from host functions when denied capabilities are accessed.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(|e| format!("instantiation failed: {:#}", e))?;
        let main = instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .map_err(|e| format!("failed to get main export: {}", e))?;
        main.call(&mut store, ()).map_err(|e| format!("{:#}", e))
    })) {
        Ok(result) => result,
        Err(panic) => {
            let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic in WASM execution".to_string()
            };
            Err(msg)
        }
    }
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
    inst.func_wrap(
        "host-bridge-finalize",
        |_: wasmtime::StoreContextMut<'_, WasiState>, (): ()|
         -> wasmtime::Result<(i64,)> { Ok((0,)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-respond-chunk-start",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_req_id, _status, _headers): (i64, i64, String)|
         -> wasmtime::Result<(i32,)> { Ok((0,)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-respond-chunk-write",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_req_id, _chunk): (i64, String)|
         -> wasmtime::Result<(i32,)> { Ok((0,)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-respond-chunk-finish",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_req_id,): (i64,)|
         -> wasmtime::Result<(i32,)> { Ok((0,)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-request-with-options",
        |_: wasmtime::StoreContextMut<'_, WasiState>,
         (_method, _url, _headers, _body, _timeout): (String, String, String, String, i64)|
         -> wasmtime::Result<(String,)> { Ok((String::new(),)) },
    )
    .map_err(|e| e.to_string())?;
    inst.func_wrap(
        "host-http-cancel-accept",
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
    // Backtrace — real stack capture via WasmBacktrace::capture.
    {
        let mut inst = linker
            .instance("nexus:runtime/backtrace")
            .map_err(|e| format!("failed to create backtrace instance: {}", e))?;
        inst.func_wrap(
            "capture-backtrace",
            |ctx: wasmtime::StoreContextMut<'_, WasiState>, (): ()| -> wasmtime::Result<()> {
                let bt = WasmBacktrace::capture(&ctx);
                let frames: Vec<String> = bt
                    .frames()
                    .iter()
                    .filter_map(|f| {
                        f.func_name().map(|name| {
                            if let Some(idx) = name.find("_.") {
                                name[idx + 2..].to_string()
                            } else {
                                name.to_string()
                            }
                        })
                    })
                    .collect();
                COMPONENT_BT_FRAMES.with(|f| *f.borrow_mut() = frames);
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;
        inst.func_wrap(
            "bt-depth",
            |_: wasmtime::StoreContextMut<'_, WasiState>, (): ()| -> wasmtime::Result<(i64,)> {
                Ok((COMPONENT_BT_FRAMES.with(|f| f.borrow().len() as i64),))
            },
        )
        .map_err(|e| e.to_string())?;
        inst.func_wrap(
            "bt-frame",
            |_: wasmtime::StoreContextMut<'_, WasiState>,
             (idx,): (i64,)|
             -> wasmtime::Result<(String,)> {
                let name = COMPONENT_BT_FRAMES
                    .with(|f| f.borrow().get(idx as usize).cloned())
                    .unwrap_or_default();
                Ok((name,))
            },
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
