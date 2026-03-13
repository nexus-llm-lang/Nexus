//! Runtime host functions for `conc` block parallel execution.
//!
//! Protocol:
//! - `__nx_conc_spawn(func_idx, args_ptr, n_args)`: register a task for parallel execution
//! - `__nx_conc_join()`: execute all pending tasks in parallel threads, block until done
//!
//! Task functions are exported from the WASM module as `__conc_<name>` and receive
//! their captured variables as i64 parameters.
//!
//! Task threads get WASI P1 environments governed by the parent's ExecutionCapabilities,
//! with all dependency modules loaded.

use crate::constants::NEXUS_HOST_HTTP_MODULE;
use crate::runtime::ExecutionCapabilities;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use wasmtime::{Caller, Engine, Linker, Module, Store, Val};
use wasmtime_wasi::WasiCtxBuilder;

pub const CONC_HOST_MODULE: &str = "nexus:runtime/conc";
pub const CONC_SPAWN_FUNC: &str = "__nx_conc_spawn";
pub const CONC_JOIN_FUNC: &str = "__nx_conc_join";
pub const CONC_EXPORT_PREFIX: &str = "__conc_";


struct PendingTask {
    export_name: String,
    args: Vec<i64>,
}

thread_local! {
    static CONC_PENDING: RefCell<Vec<PendingTask>> = const { RefCell::new(Vec::new()) };
    static CONC_MODULE: RefCell<Option<Arc<Module>>> = const { RefCell::new(None) };
    static CONC_ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static CONC_FUNC_MAP: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
    static CONC_DEPS: RefCell<Vec<(String, Arc<Module>)>> = RefCell::new(Vec::new());
    static CONC_CAPS: RefCell<Option<ExecutionCapabilities>> = const { RefCell::new(None) };
}

/// Build the func_idx → export_name mapping by scanning WASM exports.
fn build_conc_export_map(wasm_bytes: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        if let Ok(wasmparser::Payload::ExportSection(reader)) = payload {
            for export in reader.into_iter().flatten() {
                if export.name.starts_with(CONC_EXPORT_PREFIX) {
                    map.insert(export.index, export.name.to_string());
                }
            }
        }
    }
    map
}

/// Check if a module imports from a given module name.
pub fn imports_module(wasm_bytes: &[u8], target: &str) -> bool {
    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        if let Ok(wasmparser::Payload::ImportSection(reader)) = payload {
            for import in reader.into_iter().flatten() {
                if import.module == target {
                    return true;
                }
            }
        }
    }
    false
}

/// Prepare the conc runtime for a module that uses conc blocks.
/// Must be called before instantiating the module.
///
/// `deps` should contain all non-WASI, non-conc dependency modules that the
/// parent linker loaded (e.g., stdlib.wasm). These are re-loaded in each task thread.
pub fn setup_conc_runtime(
    engine: Engine,
    module: Arc<Module>,
    wasm_bytes: &[u8],
    deps: Vec<(String, Arc<Module>)>,
    capabilities: ExecutionCapabilities,
) {
    let map = build_conc_export_map(wasm_bytes);
    CONC_ENGINE.with(|e| *e.borrow_mut() = Some(engine));
    CONC_MODULE.with(|m| *m.borrow_mut() = Some(module));
    CONC_FUNC_MAP.with(|fm| *fm.borrow_mut() = map);
    CONC_DEPS.with(|d| *d.borrow_mut() = deps);
    CONC_CAPS.with(|c| *c.borrow_mut() = Some(capabilities));
    CONC_PENDING.with(|p| p.borrow_mut().clear());
}

/// Returns true if the WASM bytes contain imports from the conc host module.
pub fn needs_conc_runtime(wasm_bytes: &[u8]) -> bool {
    imports_module(wasm_bytes, CONC_HOST_MODULE)
}

/// Add no-op stubs for `nexus:cli/nexus-host` functions to a linker.
/// Required when the stdlib bundle imports nexus-host but the task doesn't use net.
pub fn add_nexus_host_stubs<T: 'static>(linker: &mut Linker<T>) {
    let _ = linker.func_wrap(
        NEXUS_HOST_HTTP_MODULE,
        "host-http-request",
        |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {},
    );
    let _ = linker.func_wrap(
        NEXUS_HOST_HTTP_MODULE,
        "host-http-listen",
        |_: i32, _: i32| -> i64 { -1 },
    );
    let _ = linker.func_wrap(NEXUS_HOST_HTTP_MODULE, "host-http-accept", |_: i64, _: i32| {});
    let _ = linker.func_wrap(
        NEXUS_HOST_HTTP_MODULE,
        "host-http-respond",
        |_: i64, _: i64, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 },
    );
    let _ = linker.func_wrap(NEXUS_HOST_HTTP_MODULE, "host-http-stop", |_: i64| -> i32 { 0 });
}

/// Add conc host functions (`__nx_conc_spawn`, `__nx_conc_join`) to a linker.
/// Works with any store data type.
pub fn add_conc_to_linker<T: 'static>(linker: &mut Linker<T>) -> Result<(), String> {
    linker
        .func_wrap(
            CONC_HOST_MODULE,
            CONC_SPAWN_FUNC,
            |mut caller: Caller<'_, T>,
             func_idx: i32,
             args_ptr: i32,
             n_args: i32|
             -> Result<(), wasmtime::Error> {
                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| wasmtime::Error::msg("conc: module must export memory"))?;

                let mut args = Vec::with_capacity(n_args as usize);
                for i in 0..n_args {
                    let offset = args_ptr as usize + (i as usize) * 8;
                    let mut buf = [0u8; 8];
                    memory.read(&caller, offset, &mut buf).map_err(|e| {
                        wasmtime::Error::msg(format!(
                            "conc: failed to read task arg from memory: {}",
                            e
                        ))
                    })?;
                    args.push(i64::from_le_bytes(buf));
                }

                let export_name = CONC_FUNC_MAP.with(|map| {
                    map.borrow()
                        .get(&(func_idx as u32))
                        .cloned()
                        .ok_or_else(|| {
                            wasmtime::Error::msg(format!("conc: unknown func_idx {}", func_idx))
                        })
                })?;

                CONC_PENDING.with(|pending| {
                    pending.borrow_mut().push(PendingTask { export_name, args });
                });
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            CONC_HOST_MODULE,
            CONC_JOIN_FUNC,
            |_caller: Caller<'_, T>| -> Result<(), wasmtime::Error> {
                let tasks: Vec<PendingTask> =
                    CONC_PENDING.with(|p| p.borrow_mut().drain(..).collect());
                if tasks.is_empty() {
                    return Ok(());
                }
                let module = CONC_MODULE.with(|m| m.borrow().clone()).ok_or_else(|| {
                    wasmtime::Error::msg("conc: module not set (call setup_conc_runtime first)")
                })?;
                let engine = CONC_ENGINE.with(|e| e.borrow().clone()).ok_or_else(|| {
                    wasmtime::Error::msg("conc: engine not set (call setup_conc_runtime first)")
                })?;
                let deps: Vec<(String, Arc<Module>)> = CONC_DEPS.with(|d| d.borrow().clone());
                let caps: ExecutionCapabilities =
                    CONC_CAPS.with(|c| c.borrow().clone()).ok_or_else(|| {
                        wasmtime::Error::msg(
                            "conc: capabilities not set (call setup_conc_runtime first)",
                        )
                    })?;

                std::thread::scope(|s| {
                    let handles: Vec<_> = tasks
                        .into_iter()
                        .map(|task| {
                            let module = module.clone();
                            let engine = engine.clone();
                            let deps = deps.clone();
                            let caps = caps.clone();
                            s.spawn(move || run_task_thread(&engine, &module, &deps, &task, &caps))
                        })
                        .collect();

                    for handle in handles {
                        match handle.join() {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                return Err(wasmtime::Error::msg(format!(
                                    "conc: task failed: {}",
                                    e
                                )));
                            }
                            Err(_) => {
                                return Err(wasmtime::Error::msg("conc: task thread panicked"));
                            }
                        }
                    }
                    Ok(())
                })
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Execute a single task in its own WASI-enabled wasmtime instance.
fn run_task_thread(
    engine: &Engine,
    module: &Module,
    deps: &[(String, Arc<Module>)],
    task: &PendingTask,
    capabilities: &ExecutionCapabilities,
) -> Result<(), wasmtime::Error> {
    let has_deps = !deps.is_empty();

    if has_deps {
        // Full WASI environment with dependency modules
        let mut linker = Linker::<wasmtime_wasi::p1::WasiP1Ctx>::new(engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx)?;
        capabilities
            .enforce_denied_wasi_functions(&mut linker)
            .map_err(wasmtime::Error::msg)?;
        // Conc stubs (tasks don't spawn nested conc blocks)
        linker.func_wrap(
            CONC_HOST_MODULE,
            CONC_SPAWN_FUNC,
            |_: i32, _: i32, _: i32| {},
        )?;
        linker.func_wrap(CONC_HOST_MODULE, CONC_JOIN_FUNC, || {})?;
        // nexus-host stubs (needed by stdlib bundle even if task doesn't use net)
        add_nexus_host_stubs(&mut linker);
        // Backtrace stubs (tasks share thread-local bt state)
        super::backtrace::add_bt_to_linker(&mut linker)
            .map_err(wasmtime::Error::msg)?;

        let mut builder = WasiCtxBuilder::new();
        capabilities
            .apply_to_wasi_builder(&mut builder)
            .map_err(wasmtime::Error::msg)?;
        let mut store = Store::new(engine, builder.build_p1());

        for (name, dep) in deps {
            linker.module(&mut store, name, dep).map_err(|e| {
                wasmtime::Error::msg(format!("conc: failed to load dep '{}': {}", name, e))
            })?;
        }

        let instance = linker.instantiate(&mut store, module)?;
        call_task_func(&mut store, &instance, task)
    } else {
        // Lightweight: no deps, no WASI
        let mut linker = Linker::<()>::new(engine);
        linker.func_wrap(
            CONC_HOST_MODULE,
            CONC_SPAWN_FUNC,
            |_: i32, _: i32, _: i32| {},
        )?;
        linker.func_wrap(CONC_HOST_MODULE, CONC_JOIN_FUNC, || {})?;
        super::backtrace::add_bt_to_linker(&mut linker)
            .map_err(wasmtime::Error::msg)?;

        let mut store = Store::new(engine, ());
        let instance = linker.instantiate(&mut store, module)?;
        call_task_func(&mut store, &instance, task)
    }
}

fn call_task_func<T>(
    store: &mut Store<T>,
    instance: &wasmtime::Instance,
    task: &PendingTask,
) -> Result<(), wasmtime::Error> {
    let func = instance
        .get_func(&mut *store, &task.export_name)
        .ok_or_else(|| {
            wasmtime::Error::msg(format!(
                "conc: task export '{}' not found",
                task.export_name
            ))
        })?;
    let args: Vec<Val> = task.args.iter().map(|&v| Val::I64(v)).collect();
    let mut results = [];
    func.call(store, &args, &mut results)?;
    Ok(())
}
