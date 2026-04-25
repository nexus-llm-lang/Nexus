//! Runtime host functions for @ lazy thunk evaluation.
//!
//! This implements `__nx_lazy_spawn` / `__nx_lazy_join` / `__nx_alloc` via the
//! per-thread-Store pattern canonical in wasmtime (see
//! wasmtime/examples/threads.rs): `Engine` and `Module` are Arc-backed, cheap
//! to clone; each worker thread instantiates its own `Store`. Spawn creates a
//! fresh worker, returns a task id; join waits for the worker and returns the
//! thunk's forced value.
//!
//! Two modes:
//! - **Legacy** (`LazyRuntime::new`): per-instance memories. Zero-capture
//!   thunks thread (env=0); capture-bearing thunks fall back to inline on the
//!   caller thread because the worker's Store-local memory can't see the
//!   caller's captures.
//! - **Shared-memory** (`LazyRuntime::with_shared_memory`): caller and every
//!   worker import a host-provided `wasmtime::SharedMemory` under
//!   `("env", "memory")`. All thunks thread regardless of `num_captures`;
//!   workers read captures through the same view the caller wrote. A host-side
//!   atomic bump pointer (`__nx_alloc`) replaces the per-instance heap-pointer
//!   global so concurrent allocations from worker thunks don't race.
//!
//! Thunk calling convention (from nxc codegen):
//! - A thunk `@expr` is a 0-arg closure lifted to `__closure_N(env: i64) -> i64`.
//! - The closure pointer `env` points to `{ table_idx: i64, captures... }`.
//! - Invoke via `__indirect_function_table[table_idx](env)`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use wasmtime::{Caller, Engine, Error, Linker, Module, Ref, SharedMemory, Store, TypedFunc};

pub const LAZY_HOST_MODULE: &str = "nexus:runtime/lazy";

/// Module name and field used to import a host-provided shared linear memory
/// when the lazy runtime operates in shared-memory mode.
pub const SHARED_MEMORY_MODULE: &str = "env";
pub const SHARED_MEMORY_FIELD: &str = "memory";

/// Host-side bump allocator for shared-memory mode. Maintained as an AtomicI32
/// so all worker threads atomically advance the same heap pointer. Initialised
/// to the program's `heap_base` (passed at LazyRuntime construction). Returns
/// the OLD value (the address where the caller may write `size` bytes) and
/// atomically advances by `size`. Sizes should be 8-aligned at the call site.
pub const ALLOC_NAME: &str = "alloc";

/// Check if a WASM module imports the lazy host module.
pub fn needs_lazy_runtime(wasm_bytes: &[u8]) -> bool {
    use wasmparser::{Parser, Payload};
    for payload in Parser::new(0).parse_all(wasm_bytes) {
        if let Ok(Payload::ImportSection(section)) = payload {
            for import in section {
                if let Ok(import) = import {
                    if import.module == LAZY_HOST_MODULE {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// A pending or already-resolved spawn. Inline-path results are stored as
/// `Inline(value)` so join always consults the tasks map; sign-discriminating
/// the i64 channel is unsound (any nexus value, including negative i64s, may
/// be a legitimate forced thunk result).
enum Task {
    Inline(i64),
    Threaded(JoinHandle<Result<i64, String>>),
}

/// Per-process state shared across the linker closures.
struct SharedRuntime {
    engine: Engine,
    module: Module,
    /// When `Some`, workers import this memory under
    /// `SHARED_MEMORY_MODULE.SHARED_MEMORY_FIELD` and the spawn path always
    /// threads (capture-bearing thunks read their captures from this
    /// memory just like the caller). When `None`, capture-bearing thunks
    /// fall back to inline on the caller thread (the legacy behaviour).
    shared_memory: Option<SharedMemory>,
    /// Atomic bump-allocator pointer for shared-memory mode. Initialised
    /// at construction to the program's `heap_base`. All threads (caller
    /// and every worker) `fetch_add` on this counter, so concurrent
    /// allocations from worker thunks never collide. Unused in legacy
    /// (non-shared) mode.
    alloc_ptr: AtomicI32,
    tasks: Mutex<HashMap<i64, Task>>,
    next_task_id: AtomicI64,
}

/// Publicly constructed with `LazyRuntime::new(engine, module)` and then
/// installed into a `Linker` via `register(&mut linker)`. Worker threads
/// re-instantiate the same `Module` with the same `Engine`, so
/// `__indirect_function_table` indices are identical across threads
/// (identical elem segment from the same compiled module).
pub struct LazyRuntime(Arc<SharedRuntime>);

impl LazyRuntime {
    pub fn new(engine: Engine, module: Module) -> Self {
        LazyRuntime(Arc::new(SharedRuntime {
            engine,
            module,
            shared_memory: None,
            alloc_ptr: AtomicI32::new(0),
            tasks: Mutex::new(HashMap::new()),
            next_task_id: AtomicI64::new(1),
        }))
    }

    /// Construct a runtime that runs all thunks on worker threads, including
    /// capture-bearing ones, by sharing a single linear memory between the
    /// caller and every worker. The provided `SharedMemory` must already be
    /// declared shared in the module (via `(import "env" "memory" (memory N M shared))`)
    /// and the host's `wasmtime::Config` must have `wasm_threads(true)`.
    ///
    /// `heap_base` is the program's static heap base (the address right after
    /// the data section + nullary-constructor slots). It seeds the host-side
    /// atomic bump pointer that the threaded `__nx_alloc` host function
    /// advances on every allocation, so concurrent allocations from worker
    /// thunks all draw from a single coherent heap.
    pub fn with_shared_memory(
        engine: Engine,
        module: Module,
        shared_memory: SharedMemory,
        heap_base: i32,
    ) -> Self {
        LazyRuntime(Arc::new(SharedRuntime {
            engine,
            module,
            shared_memory: Some(shared_memory),
            alloc_ptr: AtomicI32::new(heap_base),
            tasks: Mutex::new(HashMap::new()),
            next_task_id: AtomicI64::new(1),
        }))
    }

    /// Register `lazy-spawn` / `lazy-join` (and the `__nx_`-prefixed
    /// aliases for forward compat) under `LAZY_HOST_MODULE`. Generic
    /// over the Store state type so this composes with any Linker.
    ///
    /// Note: when the runtime is in shared-memory mode, the *caller* is
    /// responsible for binding the shared memory itself via
    /// `linker.define(&mut store, SHARED_MEMORY_MODULE, SHARED_MEMORY_FIELD, mem)`
    /// before instantiating — `Linker::define` requires a Store context that
    /// `register` doesn't have. Workers do it themselves inside `spawn_impl`.
    pub fn register<T: Send + 'static>(&self, linker: &mut Linker<T>) -> Result<(), String> {
        for name in ["lazy-spawn", "__nx_lazy_spawn"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    LAZY_HOST_MODULE,
                    name,
                    move |mut caller: Caller<'_, T>,
                          thunk_ptr: i64,
                          num_captures: i32|
                          -> Result<i64, Error> {
                        spawn_impl(&mut caller, &rt, thunk_ptr, num_captures)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        for name in ["lazy-join", "__nx_lazy_join"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    LAZY_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>, task_id: i64| -> Result<i64, Error> {
                        join_impl(&rt, task_id)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        // Atomic bump allocator. Registered unconditionally so a module
        // imports `(import "nexus:runtime/lazy" "alloc" ...)` resolves
        // whether or not it actually uses shared-memory threading; the
        // legacy mode just doesn't import it.
        for name in [ALLOC_NAME, "__nx_alloc"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    LAZY_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>, size: i32| -> i32 {
                        // Round size up to 8 bytes so subsequent allocations stay
                        // 8-aligned. Caller-supplied `size` may be unaligned for
                        // small fields (e.g., string-byte allocators emit size=1).
                        let size_aligned = (size + 7) & !7;
                        rt.alloc_ptr.fetch_add(size_aligned, Ordering::SeqCst)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    /// Returns the host-side current heap pointer. Test-only — used to verify
    /// after-the-fact that workers actually drew allocations from the shared
    /// pool (i.e., the pointer advanced past the initial heap_base).
    #[cfg(test)]
    pub fn current_alloc_ptr(&self) -> i32 {
        self.0.alloc_ptr.load(Ordering::SeqCst)
    }
}

/// Inline thunk invocation on the caller's thread. Used as the fallback
/// path for thunks that have captures, and as the implementation body
/// for zero-capture thunks on the worker.
fn invoke_thunk_on_store<S: wasmtime::AsContextMut>(
    mut store: S,
    table: &wasmtime::Table,
    table_idx: i64,
    env: i64,
) -> Result<i64, Error> {
    let func_ref = table.get(&mut store, table_idx as u64).ok_or_else(|| {
        Error::msg(format!("lazy: table index {table_idx} out of bounds"))
    })?;
    let func = match func_ref {
        Ref::Func(Some(f)) => f,
        Ref::Func(None) => {
            return Err(Error::msg(format!("lazy: table entry {table_idx} is null")));
        }
        _ => {
            return Err(Error::msg(format!(
                "lazy: table entry {table_idx} is not a func"
            )));
        }
    };
    let typed: TypedFunc<i64, i64> = func
        .typed(&store)
        .map_err(|e| Error::msg(format!("lazy: thunk signature mismatch: {e}")))?;
    typed
        .call(&mut store, env)
        .map_err(|e| Error::msg(format!("lazy: thunk trapped: {e}")))
}

fn spawn_impl<T>(
    caller: &mut Caller<'_, T>,
    rt: &Arc<SharedRuntime>,
    thunk_ptr: i64,
    num_captures: i32,
) -> Result<i64, Error> {
    // Read the 8-byte table_idx header from the closure pointer. Both paths
    // (threaded + inline) need this value; reading it here avoids duplicating
    // the memory access.
    let table_idx = if let Some(shared_mem) = rt.shared_memory.as_ref() {
        // Shared-memory mode: read directly from the host-held SharedMemory.
        // The caller's `memory` export is the same shared memory; this avoids
        // a Caller-mediated read that would borrow the Store mutably.
        let data = shared_mem.data();
        if (thunk_ptr as usize) + 8 > data.len() {
            return Err(Error::msg(format!(
                "lazy-spawn: thunk_ptr {thunk_ptr} out of bounds for shared memory of size {}",
                data.len()
            )));
        }
        let mut header = [0u8; 8];
        for (i, slot) in header.iter_mut().enumerate() {
            // SAFETY: SharedMemory cells permit unsynchronised reads from the
            // host; the value we are reading was written by the user wasm
            // before it called lazy-spawn (call serializes the write).
            *slot = unsafe { *data[thunk_ptr as usize + i].get() };
        }
        i64::from_le_bytes(header)
    } else {
        let memory = caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| Error::msg("lazy-spawn: no `memory` export"))?;
        let mut header = [0u8; 8];
        memory
            .read(&mut *caller, thunk_ptr as usize, &mut header)
            .map_err(|e| Error::msg(format!("lazy-spawn: read thunk header: {e}")))?;
        i64::from_le_bytes(header)
    };

    // In shared-memory mode every thunk threads — captures live in the same
    // SharedMemory the worker imports, so the worker reads them through its
    // own Store with identical addresses.
    let force_thread = rt.shared_memory.is_some();
    if !force_thread && num_captures > 0 {
        // Captures live in caller's Store-local memory. Without SharedMemory
        // (module must declare `(memory ... shared)`; codegen doesn't yet),
        // the worker cannot read them. Execute inline on the caller and stash
        // the value under a fresh task id; join looks it up. Storing in the
        // map (vs returning the raw value) keeps the i64 channel from
        // overlapping inline-result and threaded-id spaces — any negative
        // forced value would otherwise be misread as a task id by join_impl.
        let table = caller
            .get_export("__indirect_function_table")
            .and_then(|e| e.into_table())
            .ok_or_else(|| Error::msg("lazy-spawn: no `__indirect_function_table` export"))?;
        let value = invoke_thunk_on_store(&mut *caller, &table, table_idx, thunk_ptr)?;
        let task_id = rt.next_task_id.fetch_add(1, Ordering::SeqCst);
        rt.tasks.lock().unwrap().insert(task_id, Task::Inline(value));
        return Ok(-(task_id + 1));
    }

    // Threaded path. Worker gets its own Store+Instance from the same Module.
    // env passed to the thunk:
    //   - shared-memory mode: thunk_ptr (the closure record itself, captures
    //     readable via shared memory).
    //   - legacy zero-capture: 0 (thunk body doesn't read env).
    let env_for_worker: i64 = if force_thread { thunk_ptr } else { 0 };
    let task_id = rt.next_task_id.fetch_add(1, Ordering::SeqCst);
    let engine = rt.engine.clone();
    let module = rt.module.clone();
    let rt_for_worker = Arc::clone(rt);

    let handle = thread::spawn(move || -> Result<i64, String> {
        let mut linker = Linker::<()>::new(&engine);
        // Worker's Linker must satisfy the same lazy imports (modules
        // declare lazy-spawn/lazy-join even if the thunk body never calls
        // them — instantiation validates imports). Sharing the runtime
        // across threads also supports nested spawn from within a thunk.
        let worker_rt = LazyRuntime(rt_for_worker);
        worker_rt
            .register(&mut linker)
            .map_err(|e| format!("lazy worker: register: {e}"))?;
        let mut store = Store::new(&engine, ());
        if let Some(mem) = worker_rt.0.shared_memory.as_ref() {
            // Shared-memory mode: bind the shared memory under the agreed
            // import name so the worker's Module instance gets the same view
            // the caller has.
            linker
                .define(
                    &mut store,
                    SHARED_MEMORY_MODULE,
                    SHARED_MEMORY_FIELD,
                    mem.clone(),
                )
                .map_err(|e| format!("lazy worker: define shared memory: {e}"))?;
        }
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("lazy worker: instantiate: {e}"))?;
        let table = instance
            .get_table(&mut store, "__indirect_function_table")
            .ok_or_else(|| "lazy worker: no `__indirect_function_table` export".to_string())?;
        invoke_thunk_on_store(&mut store, &table, table_idx, env_for_worker)
            .map_err(|e| e.to_string())
    });

    rt.tasks.lock().unwrap().insert(task_id, Task::Threaded(handle));
    // Encoding: returned id = -(task_id + 1), so task_id 1 → -2, 2 → -3, ...
    // (Avoid -0 / 0 collision.) Both inline and threaded paths share this
    // encoding; join_impl always consults the tasks map.
    Ok(-(task_id + 1))
}

fn join_impl(rt: &Arc<SharedRuntime>, tid: i64) -> Result<i64, Error> {
    // Decode the encoded task id. Both inline-fallback and threaded spawns
    // return `-(task_id + 1)`; any positive `tid` here is an ABI violation
    // (the user wasm or a back-compat wrapper passed a raw value).
    if tid >= 0 {
        return Err(Error::msg(format!(
            "lazy-join: malformed task handle {tid} (must be negative encoded id)"
        )));
    }
    let task_id = -tid - 1;
    let task = rt
        .tasks
        .lock()
        .unwrap()
        .remove(&task_id)
        .ok_or_else(|| Error::msg(format!("lazy-join: unknown task id {task_id}")))?;
    match task {
        Task::Inline(v) => Ok(v),
        Task::Threaded(handle) => handle
            .join()
            .map_err(|_| Error::msg("lazy-join: worker panicked"))?
            .map_err(Error::msg),
    }
}

// Back-compat wrapper for call sites that don't want to construct a
// LazyRuntime explicitly — falls back to inline on every call (no threading).
pub fn add_lazy_to_linker<T: Send + 'static>(linker: &mut Linker<T>) -> Result<(), String> {
    for name in ["lazy-spawn", "__nx_lazy_spawn"] {
        linker
            .func_wrap(
                LAZY_HOST_MODULE,
                name,
                |mut caller: Caller<'_, T>,
                 thunk_ptr: i64,
                 _num_captures: i32|
                 -> Result<i64, Error> {
                    let memory = caller
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .ok_or_else(|| Error::msg("lazy-spawn: no `memory` export"))?;
                    let mut header = [0u8; 8];
                    memory
                        .read(&mut caller, thunk_ptr as usize, &mut header)
                        .map_err(|e| {
                            Error::msg(format!("lazy-spawn: read thunk header: {e}"))
                        })?;
                    let table_idx = i64::from_le_bytes(header);
                    let table = caller
                        .get_export("__indirect_function_table")
                        .and_then(|e| e.into_table())
                        .ok_or_else(|| {
                            Error::msg("lazy-spawn: no `__indirect_function_table` export")
                        })?;
                    invoke_thunk_on_store(&mut caller, &table, table_idx, thunk_ptr)
                },
            )
            .map_err(|e| e.to_string())?;
    }

    for name in ["lazy-join", "__nx_lazy_join"] {
        linker
            .func_wrap(
                LAZY_HOST_MODULE,
                name,
                |_: Caller<'_, T>, task_id: i64| -> i64 { task_id },
            )
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-crafted WAT with an indirect-table-stored thunk; the host calls
    /// the inline fallback (no LazyRuntime) and verifies the value returns.
    #[test]
    fn spawn_forces_thunk_via_indirect_table() {
        let wat = r#"
            (module
              (import "nexus:runtime/lazy" "__nx_lazy_spawn"
                (func $lazy_spawn (param i64 i32) (result i64)))
              (memory (export "memory") 1)
              (table (export "__indirect_function_table") 1 funcref)
              (func $thunk (param i64) (result i64)
                i64.const 42)
              (elem (i32.const 0) $thunk)
              (func (export "main") (result i64)
                i32.const 16
                i64.const 0
                i64.store
                i64.const 16
                i32.const 0
                call $lazy_spawn))
        "#;
        let bytes = wat::parse_str(wat).expect("WAT parse");

        let engine = Engine::default();
        let module = Module::from_binary(&engine, &bytes).expect("module load");
        let mut linker = Linker::<()>::new(&engine);
        add_lazy_to_linker(&mut linker).expect("linker");
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .expect("main export");
        let result = main.call(&mut store, ()).expect("main trap");
        assert_eq!(result, 42);
    }

    #[test]
    fn join_is_identity() {
        let wat = r#"
            (module
              (import "nexus:runtime/lazy" "__nx_lazy_join"
                (func $lazy_join (param i64) (result i64)))
              (func (export "main") (param i64) (result i64)
                local.get 0
                call $lazy_join))
        "#;
        let bytes = wat::parse_str(wat).expect("WAT parse");

        let engine = Engine::default();
        let module = Module::from_binary(&engine, &bytes).expect("module load");
        let mut linker = Linker::<()>::new(&engine);
        add_lazy_to_linker(&mut linker).expect("linker");
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let main = instance
            .get_typed_func::<i64, i64>(&mut store, "main")
            .expect("main export");
        assert_eq!(main.call(&mut store, 12345).expect("trap"), 12345);
    }

    /// Real threading: `LazyRuntime::new` + `register`. spawn returns a
    /// negative task_id; join decodes it, waits on the worker thread, and
    /// returns the forced value.
    #[test]
    fn threaded_spawn_and_join_zero_capture_thunk() {
        let wat = r#"
            (module
              (import "nexus:runtime/lazy" "__nx_lazy_spawn"
                (func $lazy_spawn (param i64 i32) (result i64)))
              (import "nexus:runtime/lazy" "__nx_lazy_join"
                (func $lazy_join (param i64) (result i64)))
              (memory (export "memory") 1)
              (table (export "__indirect_function_table") 1 funcref)
              (func $thunk (param i64) (result i64)
                i64.const 77)
              (elem (i32.const 0) $thunk)
              (func (export "main") (result i64)
                ;; write table_idx=0 header at mem[16]
                i32.const 16
                i64.const 0
                i64.store
                ;; spawn with num_captures=0
                i64.const 16
                i32.const 0
                call $lazy_spawn
                ;; join the task id
                call $lazy_join))
        "#;
        let bytes = wat::parse_str(wat).expect("WAT parse");

        let engine = Engine::default();
        let module = Module::from_binary(&engine, &bytes).expect("module load");
        let runtime = LazyRuntime::new(engine.clone(), module.clone());
        let mut linker = Linker::<()>::new(&engine);
        runtime.register(&mut linker).expect("register");
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .expect("main export");
        let result = main.call(&mut store, ()).expect("main trap");
        assert_eq!(result, 77, "worker thread must force the thunk to 77");
    }

    /// Direct unit test of `__nx_alloc`: invoking it with a sequence of sizes
    /// should atomically advance the host bump pointer and return the OLD
    /// values. Verifies the contract codegen relies on (allocator returns the
    /// allocation address; subsequent calls don't reuse the same address).
    #[test]
    fn nx_alloc_atomically_advances_heap_pointer() {
        let wat = r#"
            (module
              (import "nexus:runtime/lazy" "alloc"
                (func $alloc (param i32) (result i32)))
              (memory (export "memory") 1)
              (func (export "alloc1") (param i32) (result i32)
                local.get 0
                call $alloc))
        "#;
        let bytes = wat::parse_str(wat).expect("WAT parse");

        let mut config = wasmtime::Config::new();
        config.wasm_threads(true);
        config.shared_memory(true);
        let engine = Engine::new(&config).expect("engine");
        let module = Module::from_binary(&engine, &bytes).expect("module load");

        let mem_type = wasmtime::MemoryType::shared(1, 65536);
        let shared_mem = SharedMemory::new(&engine, mem_type).expect("shared memory");

        let runtime = LazyRuntime::with_shared_memory(
            engine.clone(),
            module.clone(),
            shared_mem,
            128, // initial heap_base
        );
        let mut linker = Linker::<()>::new(&engine);
        runtime.register(&mut linker).expect("register");
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let alloc1 = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc1")
            .expect("alloc1 export");

        // First alloc(16): returns 128 (initial heap_base), advances to 144.
        assert_eq!(alloc1.call(&mut store, 16).expect("call"), 128);
        assert_eq!(runtime.current_alloc_ptr(), 144);
        // Second alloc(8): returns 144, advances to 152.
        assert_eq!(alloc1.call(&mut store, 8).expect("call"), 144);
        assert_eq!(runtime.current_alloc_ptr(), 152);
        // Unaligned size (5) is rounded up to 8 — returned address still 8-aligned.
        assert_eq!(alloc1.call(&mut store, 5).expect("call"), 152);
        assert_eq!(
            runtime.current_alloc_ptr(),
            160,
            "size 5 must round up to 8 — keeps subsequent allocations 8-aligned"
        );
    }

    /// Shared-memory threaded capture-bearing thunk: caller writes a capture
    /// value into a host-provided `SharedMemory` at the closure's `env+8`
    /// offset; spawn dispatches to a worker thread that reads the same shared
    /// memory at the same offset and adds 100. Proves the runtime substrate
    /// works end-to-end; codegen + atomic allocator are independent follow-ups
    /// inside nexus-tb6p.
    #[test]
    fn shared_memory_threaded_capture_bearing_thunk() {
        // Module imports a shared memory under "env"."memory", lays out a
        // closure record at byte 16, writes capture[0]=37 at byte 24, then
        // calls __nx_lazy_spawn with num_captures=1. The thunk reads env+8
        // (the capture value) and returns it + 100. spawn should THREAD it
        // (not fall back to inline) because the runtime is in shared-memory
        // mode; the worker reads the same shared memory the caller wrote.
        let wat = r#"
            (module
              (import "env" "memory" (memory $mem 1 65536 shared))
              (import "nexus:runtime/lazy" "__nx_lazy_spawn"
                (func $lazy_spawn (param i64 i32) (result i64)))
              (import "nexus:runtime/lazy" "__nx_lazy_join"
                (func $lazy_join (param i64) (result i64)))
              (table (export "__indirect_function_table") 1 funcref)
              (func $thunk (param $env i64) (result i64)
                ;; capture[0] sits at env+8
                local.get $env
                i32.wrap_i64
                i32.const 8
                i32.add
                i64.load
                i64.const 100
                i64.add)
              (elem (i32.const 0) $thunk)
              (func (export "main") (result i64)
                ;; closure record at mem[16]: table_idx=0
                i32.const 16
                i64.const 0
                i64.store
                ;; capture[0] at mem[24]: value 37
                i32.const 24
                i64.const 37
                i64.store
                ;; spawn with num_captures=1; in shared-memory mode the runtime
                ;; threads regardless and passes thunk_ptr as env.
                i64.const 16
                i32.const 1
                call $lazy_spawn
                call $lazy_join))
        "#;
        let bytes = wat::parse_str(wat).expect("WAT parse");

        let mut config = wasmtime::Config::new();
        config.wasm_threads(true);
        config.shared_memory(true);
        let engine = Engine::new(&config).expect("engine");
        let module = Module::from_binary(&engine, &bytes).expect("module load");

        let mem_type = wasmtime::MemoryType::shared(1, 65536);
        let shared_mem = SharedMemory::new(&engine, mem_type).expect("shared memory");

        let runtime = LazyRuntime::with_shared_memory(
            engine.clone(),
            module.clone(),
            shared_mem.clone(),
            64, // heap_base placeholder for this hand-crafted WAT (no allocs)
        );
        let mut linker = Linker::<()>::new(&engine);
        runtime.register(&mut linker).expect("register");
        let mut store = Store::new(&engine, ());
        linker
            .define(
                &mut store,
                SHARED_MEMORY_MODULE,
                SHARED_MEMORY_FIELD,
                shared_mem.clone(),
            )
            .expect("define shared memory");
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .expect("main export");
        let result = main.call(&mut store, ()).expect("main trap");
        assert_eq!(
            result, 137,
            "shared-memory worker must read capture (37) and add 100 → 137"
        );
    }

    /// Regression for the sign-overlap bug: an inline-fallback spawn whose
    /// thunk forces to a negative i64 must round-trip through join unchanged.
    /// Prior to the Inline/Threaded split, join_impl misread negative values
    /// as encoded task ids and either errored or returned the wrong worker's
    /// result.
    #[test]
    fn inline_spawn_preserves_negative_thunk_value() {
        let wat = r#"
            (module
              (import "nexus:runtime/lazy" "__nx_lazy_spawn"
                (func $lazy_spawn (param i64 i32) (result i64)))
              (import "nexus:runtime/lazy" "__nx_lazy_join"
                (func $lazy_join (param i64) (result i64)))
              (memory (export "memory") 1)
              (table (export "__indirect_function_table") 1 funcref)
              (func $thunk (param i64) (result i64)
                i64.const -5)
              (elem (i32.const 0) $thunk)
              (func (export "main") (result i64)
                ;; closure record at mem[16]: table_idx=0
                i32.const 16
                i64.const 0
                i64.store
                ;; spawn with num_captures=1 → inline-fallback path
                i64.const 16
                i32.const 1
                call $lazy_spawn
                call $lazy_join))
        "#;
        let bytes = wat::parse_str(wat).expect("WAT parse");

        let engine = Engine::default();
        let module = Module::from_binary(&engine, &bytes).expect("module load");
        let runtime = LazyRuntime::new(engine.clone(), module.clone());
        let mut linker = Linker::<()>::new(&engine);
        runtime.register(&mut linker).expect("register");
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let main = instance
            .get_typed_func::<(), i64>(&mut store, "main")
            .expect("main export");
        let result = main.call(&mut store, ()).expect("main trap");
        assert_eq!(result, -5, "inline-fallback must preserve negative i64 thunk results");
    }
}
