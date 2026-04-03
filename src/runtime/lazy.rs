//! Runtime host functions for DAG-parallel lazy evaluation.
//!
//! Protocol:
//! - `__nx_lazy_spawn(thunk_ptr: i64, num_captures: i32) -> i64`: spawn thunk evaluation
//!   on a separate thread. Reads the closure data (funcref table index + captures) from
//!   WASM memory, creates a new module instance in a dedicated thread, and evaluates the
//!   thunk there. Returns a task ID.
//! - `__nx_lazy_join(task_id: i64) -> i64`: wait for a spawned thunk to complete, return
//!   the result value.

use std::cell::RefCell;
use std::sync::Arc;
use std::thread;

use wasmtime::{Caller, Engine, Func, Linker, Module, Store, Val};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::WasiCtxBuilder;

use super::backtrace;
use super::net_host;
use super::net_host::imports_module;

pub const LAZY_HOST_MODULE: &str = "nexus:runtime/lazy";

/// Pre-compiled module state shared across spawn threads.
struct LazySetup {
    engine: Engine,
    module: Module,
    dep_modules: Vec<(String, Module)>,
}

// SAFETY: Module and Engine are Send+Sync. The Arc provides shared ownership.
unsafe impl Send for LazySetup {}
unsafe impl Sync for LazySetup {}

thread_local! {
    static LAZY_SETUP: RefCell<Option<Arc<LazySetup>>> = const { RefCell::new(None) };
    static TASKS: RefCell<Vec<Option<thread::JoinHandle<i64>>>> = const { RefCell::new(Vec::new()) };
}

/// Initialize the lazy runtime with the engine, pre-compiled main module, and dependencies.
/// Must be called before any `__nx_lazy_spawn` host calls.
pub fn setup_lazy_runtime(engine: Engine, module: Module, dep_modules: Vec<(String, Module)>) {
    let setup = Arc::new(LazySetup {
        engine,
        module,
        dep_modules,
    });
    LAZY_SETUP.with(|s| *s.borrow_mut() = Some(setup));
    TASKS.with(|t| t.borrow_mut().clear());
}

/// Check if a WASM module imports the lazy host module.
pub fn needs_lazy_runtime(wasm_bytes: &[u8]) -> bool {
    imports_module(wasm_bytes, LAZY_HOST_MODULE)
}

/// Reset lazy runtime state (call before each execution).
pub fn reset() {
    TASKS.with(|t| t.borrow_mut().clear());
}

/// Add lazy runtime host functions to a linker.
pub fn add_lazy_to_linker(linker: &mut Linker<WasiP1Ctx>) -> Result<(), String> {
    // __nx_lazy_spawn(thunk_ptr: i64, num_captures: i32) -> i64
    //
    // Reads the closure at thunk_ptr:
    //   [table_idx_i64, cap0_i64, cap1_i64, ...]
    // Total words = 1 + num_captures (standard closure layout).
    linker
        .func_wrap(
            LAZY_HOST_MODULE,
            "__nx_lazy_spawn",
            |mut caller: Caller<'_, WasiP1Ctx>, thunk_ptr: i64, num_captures: i32| -> i64 {
                let setup = LAZY_SETUP
                    .with(|s| s.borrow().clone())
                    .expect("lazy runtime not initialized — call setup_lazy_runtime first");

                let memory = caller
                    .get_export("memory")
                    .and_then(|e| e.into_memory())
                    .expect("lazy_spawn: module must export memory");

                let ptr = thunk_ptr as u32 as usize;
                let total_words = 1 + num_captures as usize;

                // Read the entire closure: [table_idx, cap0, cap1, ...]
                let mut closure_data = Vec::with_capacity(total_words);
                for i in 0..total_words {
                    let offset = ptr + i * 8;
                    let mut buf = [0u8; 8];
                    memory
                        .read(&caller, offset, &mut buf)
                        .expect("lazy_spawn: failed to read closure data");
                    closure_data.push(i64::from_le_bytes(buf));
                }

                let handle = thread::spawn(move || {
                    eval_thunk_in_new_instance(&setup, &closure_data)
                });

                TASKS.with(|t| {
                    let mut tasks = t.borrow_mut();
                    let id = tasks.len();
                    tasks.push(Some(handle));
                    id as i64
                })
            },
        )
        .map_err(|e| e.to_string())?;

    // __nx_lazy_join(task_id: i64) -> i64
    linker
        .func_wrap(
            LAZY_HOST_MODULE,
            "__nx_lazy_join",
            |_caller: Caller<'_, WasiP1Ctx>, task_id: i64| -> i64 {
                TASKS.with(|t| {
                    let mut tasks = t.borrow_mut();
                    let idx = task_id as usize;
                    let handle = tasks[idx]
                        .take()
                        .unwrap_or_else(|| panic!("lazy task {} already joined or invalid", idx));
                    match handle.join() {
                        Ok(result) => result,
                        Err(e) => {
                            eprintln!("lazy task {} panicked: {:?}", idx, e);
                            0
                        }
                    }
                })
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Evaluate a thunk closure in a fresh module instance (runs in a spawned thread).
///
/// `closure_data` layout: [table_idx_i64, cap0_i64, cap1_i64, ...]
fn eval_thunk_in_new_instance(setup: &LazySetup, closure_data: &[i64]) -> i64 {
    let mut linker = Linker::<WasiP1Ctx>::new(&setup.engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx)
        .expect("lazy thread: failed to add WASI");

    // Add net host stubs (stdlib may import from it)
    net_host::add_net_host_to_linker(&mut linker)
        .unwrap_or_else(|e| panic!("lazy thread: net host: {}", e));

    let mut builder = WasiCtxBuilder::new();
    let mut store = Store::new(&setup.engine, builder.build_p1());

    // Add backtrace stubs
    backtrace::add_bt_to_linker(&mut linker, &mut store)
        .unwrap_or_else(|e| panic!("lazy thread: backtrace: {}", e));

    // Add stub lazy host functions (nested parallelism falls back to sequential)
    linker.allow_shadowing(true);
    linker
        .func_wrap(
            LAZY_HOST_MODULE,
            "__nx_lazy_spawn",
            |_: i64, _: i32| -> i64 { panic!("nested lazy spawn not supported") },
        )
        .ok();
    linker
        .func_wrap(
            LAZY_HOST_MODULE,
            "__nx_lazy_join",
            |_: i64| -> i64 { panic!("nested lazy join not supported") },
        )
        .ok();
    linker.allow_shadowing(false);

    // Link dependency modules (stdlib, etc.)
    for (name, dep_module) in &setup.dep_modules {
        linker
            .module(&mut store, name, dep_module)
            .unwrap_or_else(|e| panic!("lazy thread: failed to link dep '{}': {}", name, e));
    }

    let instance = linker
        .instantiate(&mut store, &setup.module)
        .expect("lazy thread: failed to instantiate module");

    // Build the closure object in the new instance's memory.
    // Layout: [table_idx_i64, cap0_i64, cap1_i64, ...]
    let n_words = closure_data.len();
    let alloc_fn = instance
        .get_typed_func::<i32, i32>(&mut store, "allocate")
        .expect("lazy thread: module must export allocate");
    let closure_ptr = alloc_fn
        .call(&mut store, (n_words * 8) as i32)
        .expect("lazy thread: allocate failed");

    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("lazy thread: module must export memory");

    // Write closure data into the new instance's memory
    for (i, &word) in closure_data.iter().enumerate() {
        memory
            .write(
                &mut store,
                closure_ptr as usize + i * 8,
                &word.to_le_bytes(),
            )
            .expect("lazy thread: write closure data");
    }

    // Look up the function from the funcref table and call it
    let table = instance
        .get_table(&mut store, "__indirect_function_table")
        .expect("lazy thread: no funcref table");
    let table_idx = closure_data[0] as u32;
    let func_ref = table
        .get(&mut store, table_idx as u64)
        .expect("lazy thread: table_idx out of bounds");
    let func: Func = func_ref
        .unwrap_func()
        .copied()
        .expect("lazy thread: null funcref at table_idx");

    // Call the thunk: (__env: i64) -> i64
    let env_val = Val::I64(closure_ptr as i64);
    let mut results = [Val::I64(0)];
    func.call(&mut store, &[env_val], &mut results)
        .expect("lazy thread: thunk execution failed");

    results[0].unwrap_i64()
}
