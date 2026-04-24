//! Runtime host functions for @ lazy thunk evaluation.
//!
//! Sequential reference implementation. `__nx_lazy_spawn` invokes the thunk
//! immediately on the calling thread via the exported
//! `__indirect_function_table` and returns the forced result as the "task id".
//! `__nx_lazy_join` is identity — the id IS the value, since sequential
//! spawn already has it.
//!
//! The API shape matches a future parallel implementation: spawn dispatches
//! work, join retrieves the result. A threaded rewrite would replace the
//! in-line thunk call with `std::thread::spawn`, wire wasmtime's
//! `SharedMemory` across threads, and have `join` wait on a channel. Callers
//! need no changes.
//!
//! Thunk calling convention (per nxc codegen):
//! - A thunk `@expr` is a 0-arg closure lifted to `__closure_N(env: i64) -> i64`.
//! - The closure pointer `env` points to memory layout `{ table_idx: i64, captures... }`.
//! - To invoke: read `table_idx` from `mem[env..env+8]`, `call_indirect` via
//!   `__indirect_function_table[table_idx]` with `env` as the single i64 arg.
//!
//! Protocol with stdlib/lazy.nx (if imports restored):
//! - `__nx_lazy_spawn(thunk: i64, num_captures: i32) -> i64`: returns the forced value.
//! - `__nx_lazy_join(task_id: i64) -> i64`: returns `task_id` (identity).

use wasmtime::{Caller, Error, Linker, Ref, TypedFunc};

pub const LAZY_HOST_MODULE: &str = "nexus:runtime/lazy";

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

/// Invoke a thunk closure and return its forced result.
///
/// Reads `table_idx` from the closure header (first 8 bytes at `thunk_ptr`),
/// looks up the function in `__indirect_function_table`, and calls it with
/// `thunk_ptr` as the sole argument (the `__env` param of the lifted closure).
fn invoke_thunk<T>(caller: &mut Caller<'_, T>, thunk_ptr: i64) -> Result<i64, Error> {
    // Read the 8-byte table_idx header from the closure pointer.
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| Error::msg("lazy_spawn: no `memory` export on the instance"))?;
    let mut header = [0u8; 8];
    memory
        .read(&mut *caller, thunk_ptr as usize, &mut header)
        .map_err(|e| Error::msg(format!("lazy_spawn: failed to read thunk header: {e}")))?;
    let table_idx = i64::from_le_bytes(header);

    // Look up the function in the indirect table.
    let table = caller
        .get_export("__indirect_function_table")
        .and_then(|e| e.into_table())
        .ok_or_else(|| {
            Error::msg("lazy_spawn: no `__indirect_function_table` export on the instance")
        })?;
    let func = match table.get(&mut *caller, table_idx as u64) {
        Some(Ref::Func(Some(f))) => f,
        Some(Ref::Func(None)) => {
            return Err(Error::msg(format!(
                "lazy_spawn: table entry {table_idx} is null"
            )));
        }
        Some(_) => {
            return Err(Error::msg(format!(
                "lazy_spawn: table entry {table_idx} is not a func"
            )));
        }
        None => {
            return Err(Error::msg(format!(
                "lazy_spawn: table index {table_idx} out of bounds"
            )));
        }
    };

    // Thunk signature: (env: i64) -> i64
    let typed: TypedFunc<i64, i64> = func
        .typed(&*caller)
        .map_err(|e| Error::msg(format!("lazy_spawn: thunk type mismatch at table[{table_idx}]: {e}")))?;
    typed
        .call(&mut *caller, thunk_ptr)
        .map_err(|e| Error::msg(format!("lazy_spawn: thunk trapped: {e}")))
}

/// Register lazy-spawn and lazy-join with the linker, under both the
/// WIT-canonical kebab-case names (what the compiler emits for imports
/// from `nexus:runtime/lazy`) and the `__nx_`-prefixed underscored names
/// (back-compat if future code declares the raw module name).
pub fn add_lazy_to_linker<T: Send + 'static>(linker: &mut Linker<T>) -> Result<(), String> {
    for name in ["lazy-spawn", "__nx_lazy_spawn"] {
        linker
            .func_wrap(
                LAZY_HOST_MODULE,
                name,
                |mut caller: Caller<'_, T>,
                 thunk_ptr: i64,
                 _num_captures: i32|
                 -> Result<i64, Error> { invoke_thunk(&mut caller, thunk_ptr) },
            )
            .map_err(|e| e.to_string())?;
    }

    for name in ["lazy-join", "__nx_lazy_join"] {
        linker
            .func_wrap(
                LAZY_HOST_MODULE,
                name,
                |_caller: Caller<'_, T>, task_id: i64| -> i64 { task_id },
            )
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Linker, Module, Store};

    /// End-to-end test: hand-crafted core WASM that imports __nx_lazy_spawn,
    /// places a thunk-shaped closure at mem[16] (header table_idx=0 → function 0),
    /// calls __nx_lazy_spawn(16, 0), and re-exports the result.
    ///
    /// Thunk function at table[0] ignores its env param and returns 42.
    /// After spawn, main returns the forced value; we assert it equals 42.
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
                ;; write table_idx (i64 = 0) into mem[16..24] as the thunk header
                i32.const 16
                i64.const 0
                i64.store
                ;; call __nx_lazy_spawn(thunk_ptr = 16, num_captures = 0)
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
        assert_eq!(result, 42, "thunk must have been forced to 42 by host spawn");
    }

    /// __nx_lazy_join is identity — the task_id IS the forced value under
    /// sequential semantics. Verify it returns the input unchanged.
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
        assert_eq!(main.call(&mut store, 12345).expect("main trap"), 12345);
        assert_eq!(main.call(&mut store, -7).expect("main trap"), -7);
    }
}
