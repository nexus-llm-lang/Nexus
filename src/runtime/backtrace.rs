//! Runtime host functions for exception backtrace via wasmtime stack walk.
//!
//! Protocol:
//! - `__nx_capture_backtrace()`: walk the WASM call stack and store frame names
//!   (called just before `throw` at raise sites)
//! - `__nx_bt_depth() -> i64`: number of frames in the captured backtrace
//! - `__nx_bt_frame(idx: i64) -> i64`: get frame name at given index as packed string
//!   (writes the string into WASM memory via the allocator on demand)

use std::cell::RefCell;
use wasmtime::{Caller, Linker, Memory, MemoryType, WasmBacktrace};

pub const BT_HOST_MODULE: &str = "nexus:runtime/backtrace";

thread_local! {
    /// Captured frame names (Rust strings) from the most recent stack walk.
    /// Most-recent frame first (reversed from wasmtime's bottom-up order).
    static BT_FRAMES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Check if a WASM module imports the backtrace host module.
pub fn needs_bt_runtime(wasm_bytes: &[u8]) -> bool {
    use wasmparser::{Parser, Payload};
    for payload in Parser::new(0).parse_all(wasm_bytes) {
        if let Ok(Payload::ImportSection(section)) = payload {
            for import in section {
                if let Ok(import) = import {
                    if import.module == BT_HOST_MODULE {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Reset backtrace state (call before each execution).
pub fn reset() {
    BT_FRAMES.with(|f| f.borrow_mut().clear());
}

/// Write a string into WASM memory via the `allocate` export, returning packed i64 (offset << 32 | len).
/// Returns 0 on failure (no memory or no allocator).
fn write_string_to_wasm<T>(caller: &mut Caller<'_, T>, s: &str) -> i64 {
    let bytes = s.as_bytes();
    let len = bytes.len() as i32;
    if len == 0 {
        return 0;
    }

    // Get the allocate function from WASM exports
    let alloc = match caller.get_export("allocate") {
        Some(ext) => match ext.into_func() {
            Some(f) => f,
            None => return 0,
        },
        None => return 0,
    };

    // Allocate space in WASM memory
    let mut results = [wasmtime::Val::I32(0)];
    if alloc
        .call(&mut *caller, &[wasmtime::Val::I32(len)], &mut results)
        .is_err()
    {
        return 0;
    }
    let ptr = results[0].unwrap_i32();
    if ptr == 0 {
        return 0;
    }

    // Write the string bytes
    let memory = match caller.get_export("memory") {
        Some(ext) => match ext.into_memory() {
            Some(m) => m,
            None => return 0,
        },
        None => return 0,
    };
    if memory.write(&mut *caller, ptr as usize, bytes).is_err() {
        return 0;
    }

    // Pack as i64: (offset << 32) | len
    (((ptr as u64) << 32) | (len as u64)) as i64
}

/// Add backtrace host functions to a linker.
///
/// Also defines a shared `memory` export in the backtrace module namespace,
/// because stdlib modules that import `__nx_bt_frame` (which returns string)
/// get compiled with `MemoryMode::Imported { module: "nexus:runtime/backtrace" }`.
pub fn add_bt_to_linker<T: Send + 'static>(
    linker: &mut Linker<T>,
    store: &mut wasmtime::Store<T>,
) -> Result<(), String> {
    // Provide a shared memory for modules that import memory from this namespace.
    // The actual string data is written into the caller's memory (via allocate export),
    // but the WASM module import still needs a memory definition to satisfy the linker.
    let memory = Memory::new(&mut *store, MemoryType::new(1, None))
        .map_err(|e| format!("Failed to create backtrace memory: {}", e))?;
    linker
        .define(&mut *store, BT_HOST_MODULE, "memory", memory)
        .map_err(|e| format!("Failed to define backtrace memory: {}", e))?;

    // Capture backtrace via wasmtime stack walk — called before throw
    linker
        .func_wrap(
            BT_HOST_MODULE,
            "__nx_capture_backtrace",
            |caller: Caller<'_, T>| {
                let bt = WasmBacktrace::capture(&caller);
                let frames: Vec<String> = bt
                    .frames()
                    .iter()
                    .filter_map(|frame| {
                        frame.func_name().map(|name| {
                            // Strip the module prefix from names like "__import_0_.func_name"
                            if let Some(idx) = name.find("_.") {
                                name[idx + 2..].to_string()
                            } else {
                                name.to_string()
                            }
                        })
                    })
                    // Skip the first frame — it's __nx_capture_backtrace itself
                    // (already filtered out because it's a host function, not a WASM frame)
                    .collect();
                BT_FRAMES.with(|f| *f.borrow_mut() = frames);
            },
        )
        .map_err(|e| e.to_string())?;

    // Return number of captured frames
    linker
        .func_wrap(BT_HOST_MODULE, "__nx_bt_depth", || -> i64 {
            BT_FRAMES.with(|f| f.borrow().len() as i64)
        })
        .map_err(|e| e.to_string())?;

    // Return a frame name as a packed string, written into WASM memory on demand
    linker
        .func_wrap(
            BT_HOST_MODULE,
            "__nx_bt_frame",
            |mut caller: Caller<'_, T>, idx: i64| -> i64 {
                let name = BT_FRAMES.with(|f| {
                    let frames = f.borrow();
                    frames.get(idx as usize).cloned()
                });
                match name {
                    Some(s) => write_string_to_wasm(&mut caller, &s),
                    None => 0,
                }
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
