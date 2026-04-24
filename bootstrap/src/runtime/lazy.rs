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

/// Register `__nx_lazy_spawn` and `__nx_lazy_join` with the linker.
pub fn add_lazy_to_linker<T: Send + 'static>(linker: &mut Linker<T>) -> Result<(), String> {
    linker
        .func_wrap(
            LAZY_HOST_MODULE,
            "__nx_lazy_spawn",
            |mut caller: Caller<'_, T>,
             thunk_ptr: i64,
             _num_captures: i32|
             -> Result<i64, Error> { invoke_thunk(&mut caller, thunk_ptr) },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            LAZY_HOST_MODULE,
            "__nx_lazy_join",
            |_caller: Caller<'_, T>, task_id: i64| -> i64 { task_id },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
