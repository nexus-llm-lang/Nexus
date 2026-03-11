//! Runtime host functions for exception backtrace tracking.
//!
//! Protocol:
//! - `__nx_bt_push(name: i64)`: push a packed-string function name onto the call stack
//! - `__nx_bt_pop()`: pop the top frame from the call stack
//! - `__nx_bt_freeze()`: freeze the current stack (called at `raise` time)
//! - `__nx_bt_depth() -> i64`: number of frames in the frozen backtrace
//! - `__nx_bt_frame(idx: i64) -> i64`: get frame name at given index as packed string

use std::cell::RefCell;
use wasmtime::Linker;

use super::conc::imports_module;

pub const BT_HOST_MODULE: &str = "nexus:runtime/backtrace";

thread_local! {
    static BT_STACK: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
    static BT_FROZEN: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

/// Check if a WASM module imports the backtrace host module.
pub fn needs_bt_runtime(wasm_bytes: &[u8]) -> bool {
    imports_module(wasm_bytes, BT_HOST_MODULE)
}

/// Reset backtrace state (call before each execution).
pub fn reset() {
    BT_STACK.with(|s| s.borrow_mut().clear());
    BT_FROZEN.with(|f| f.borrow_mut().clear());
}

/// Add backtrace host functions to a linker.
pub fn add_bt_to_linker<T: 'static>(linker: &mut Linker<T>) -> Result<(), String> {
    linker
        .func_wrap(BT_HOST_MODULE, "__nx_bt_push", |name: i64| {
            BT_STACK.with(|s| s.borrow_mut().push(name));
        })
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(BT_HOST_MODULE, "__nx_bt_pop", || {
            BT_STACK.with(|s| {
                s.borrow_mut().pop();
            });
        })
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(BT_HOST_MODULE, "__nx_bt_freeze", || {
            BT_STACK.with(|s| {
                let stack = s.borrow().clone();
                BT_FROZEN.with(|f| *f.borrow_mut() = stack);
            });
        })
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(BT_HOST_MODULE, "__nx_bt_depth", || -> i64 {
            BT_FROZEN.with(|f| f.borrow().len() as i64)
        })
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(BT_HOST_MODULE, "__nx_bt_frame", |idx: i64| -> i64 {
            BT_FROZEN.with(|f| {
                let frozen = f.borrow();
                // Return frames in reverse order (most recent first)
                let rev_idx = frozen.len().saturating_sub(1 + idx as usize);
                frozen.get(rev_idx).copied().unwrap_or(0)
            })
        })
        .map_err(|e| e.to_string())?;

    Ok(())
}
