//! Runtime host functions for one-shot linear channels (Phase 1 of nexus-5ldt).
//!
//! Channel semantics in current single-threaded execution:
//! - `oneshot()` allocates a fresh cell and returns a non-zero i64 id.
//! - `send(id, val)` deposits `val`. Trap if the cell is already full.
//! - `recv(id)` removes and returns the deposited value. Trap if empty.
//!   Recv removes the cell entry, so the id becomes invalid afterward.
//!
//! Linearity at the Nexus surface (stdlib `chan.nx`) makes the "consumed
//! exactly once" invariant a compile-time guarantee. The runtime traps are
//! a defence-in-depth check for ABI misuse only — well-typed user code
//! cannot reach them.
//!
//! Phase 3's blocking semantics (recv waits for a sender via the scheduler)
//! will replace `recv-on-empty trap` with a yield to the scheduler. The
//! API surface stays the same.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use wasmtime::{Caller, Error, Linker};

pub const CHAN_HOST_MODULE: &str = "nexus:runtime/chan";

/// Check if a WASM module imports the chan host module.
pub fn needs_chan_runtime(wasm_bytes: &[u8]) -> bool {
    use wasmparser::{Parser, Payload};
    for payload in Parser::new(0).parse_all(wasm_bytes) {
        if let Ok(Payload::ImportSection(section)) = payload {
            for import in section {
                if let Ok(import) = import {
                    if import.module == CHAN_HOST_MODULE {
                        return true;
                    }
                }
            }
        }
    }
    false
}

struct SharedRuntime {
    /// id -> Some(value) once a sender has deposited, None while empty.
    /// Recv removes the entry entirely so the id becomes invalid.
    cells: Mutex<HashMap<i64, Option<i64>>>,
    /// Monotonic id counter. Starts at 1 so 0 is reserved as "invalid".
    next_id: AtomicI64,
}

pub struct ChanRuntime(Arc<SharedRuntime>);

impl ChanRuntime {
    pub fn new() -> Self {
        ChanRuntime(Arc::new(SharedRuntime {
            cells: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
        }))
    }

    /// Register `chan-oneshot` / `chan-send` / `chan-recv` (and
    /// `__nx_`-prefixed aliases for the way generic externals are emitted
    /// from the Nexus side) under `CHAN_HOST_MODULE`.
    pub fn register<T: Send + 'static>(&self, linker: &mut Linker<T>) -> Result<(), String> {
        for name in ["chan-oneshot", "__nx_chan_oneshot"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    CHAN_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>| -> Result<i64, Error> {
                        let id = rt.next_id.fetch_add(1, Ordering::SeqCst);
                        rt.cells.lock().unwrap().insert(id, None);
                        Ok(id)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        for name in ["chan-send", "__nx_chan_send"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    CHAN_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>, id: i64, val: i64| -> Result<(), Error> {
                        let mut cells = rt.cells.lock().unwrap();
                        let slot = cells.get_mut(&id).ok_or_else(|| {
                            Error::msg(format!("chan-send: unknown channel id {id}"))
                        })?;
                        if slot.is_some() {
                            return Err(Error::msg(format!(
                                "chan-send: channel {id} already filled (one-shot violation)"
                            )));
                        }
                        *slot = Some(val);
                        Ok(())
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        for name in ["chan-recv", "__nx_chan_recv"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    CHAN_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>, id: i64| -> Result<i64, Error> {
                        let mut cells = rt.cells.lock().unwrap();
                        let slot = cells.get(&id).ok_or_else(|| {
                            Error::msg(format!("chan-recv: unknown channel id {id}"))
                        })?;
                        let value = slot.ok_or_else(|| {
                            Error::msg(format!(
                                "chan-recv: channel {id} empty (sender has not run)"
                            ))
                        })?;
                        cells.remove(&id);
                        Ok(value)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

impl Default for ChanRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Module, Store};

    fn build_test_module() -> (Engine, Module) {
        // Minimal core wasm that imports the three chan functions and exports
        // a `main` that sends 99 and reads it back, trapping on mismatch via
        // i64.div_s by zero.
        let wat = r#"
            (module
              (import "nexus:runtime/chan" "chan-oneshot" (func $oneshot (result i64)))
              (import "nexus:runtime/chan" "chan-send"
                (func $send (param i64 i64)))
              (import "nexus:runtime/chan" "chan-recv"
                (func $recv (param i64) (result i64)))
              (func (export "main")
                (local $id i64)
                (local $v i64)
                (local.set $id (call $oneshot))
                (call $send (local.get $id) (i64.const 99))
                (local.set $v (call $recv (local.get $id)))
                ;; trap if v != 99: divide 1 by (v == 99 ? 1 : 0)
                (drop (i64.div_s
                  (i64.const 1)
                  (select
                    (i64.const 1)
                    (i64.const 0)
                    (i64.eq (local.get $v) (i64.const 99))))))
            )
        "#;
        let engine = Engine::default();
        let module = Module::new(&engine, wat).expect("wat");
        (engine, module)
    }

    #[test]
    fn oneshot_send_recv_roundtrip() {
        let (engine, module) = build_test_module();
        let mut linker = Linker::<()>::new(&engine);
        ChanRuntime::new().register(&mut linker).expect("register");
        let mut store = Store::new(&engine, ());
        let inst = linker.instantiate(&mut store, &module).expect("inst");
        let main = inst
            .get_typed_func::<(), ()>(&mut store, "main")
            .expect("main");
        main.call(&mut store, ()).expect("main run");
    }

    #[test]
    fn double_send_traps() {
        let wat = r#"
            (module
              (import "nexus:runtime/chan" "chan-oneshot" (func $oneshot (result i64)))
              (import "nexus:runtime/chan" "chan-send"
                (func $send (param i64 i64)))
              (func (export "main")
                (local $id i64)
                (local.set $id (call $oneshot))
                (call $send (local.get $id) (i64.const 1))
                (call $send (local.get $id) (i64.const 2))))
        "#;
        let engine = Engine::default();
        let module = Module::new(&engine, wat).unwrap();
        let mut linker = Linker::<()>::new(&engine);
        ChanRuntime::new().register(&mut linker).unwrap();
        let mut store = Store::new(&engine, ());
        let inst = linker.instantiate(&mut store, &module).unwrap();
        let main = inst.get_typed_func::<(), ()>(&mut store, "main").unwrap();
        let err = main.call(&mut store, ()).expect_err("must trap");
        assert!(
            format!("{err:#}").contains("already filled"),
            "wrong error: {err:#}"
        );
    }

    #[test]
    fn recv_before_send_traps() {
        let wat = r#"
            (module
              (import "nexus:runtime/chan" "chan-oneshot" (func $oneshot (result i64)))
              (import "nexus:runtime/chan" "chan-recv"
                (func $recv (param i64) (result i64)))
              (func (export "main")
                (local $id i64)
                (local.set $id (call $oneshot))
                (drop (call $recv (local.get $id)))))
        "#;
        let engine = Engine::default();
        let module = Module::new(&engine, wat).unwrap();
        let mut linker = Linker::<()>::new(&engine);
        ChanRuntime::new().register(&mut linker).unwrap();
        let mut store = Store::new(&engine, ());
        let inst = linker.instantiate(&mut store, &module).unwrap();
        let main = inst.get_typed_func::<(), ()>(&mut store, "main").unwrap();
        let err = main.call(&mut store, ()).expect_err("must trap");
        assert!(format!("{err:#}").contains("empty"), "wrong error: {err:#}");
    }

    #[test]
    fn double_recv_traps() {
        let wat = r#"
            (module
              (import "nexus:runtime/chan" "chan-oneshot" (func $oneshot (result i64)))
              (import "nexus:runtime/chan" "chan-send"
                (func $send (param i64 i64)))
              (import "nexus:runtime/chan" "chan-recv"
                (func $recv (param i64) (result i64)))
              (func (export "main")
                (local $id i64)
                (local.set $id (call $oneshot))
                (call $send (local.get $id) (i64.const 7))
                (drop (call $recv (local.get $id)))
                (drop (call $recv (local.get $id)))))
        "#;
        let engine = Engine::default();
        let module = Module::new(&engine, wat).unwrap();
        let mut linker = Linker::<()>::new(&engine);
        ChanRuntime::new().register(&mut linker).unwrap();
        let mut store = Store::new(&engine, ());
        let inst = linker.instantiate(&mut store, &module).unwrap();
        let main = inst.get_typed_func::<(), ()>(&mut store, "main").unwrap();
        let err = main.call(&mut store, ()).expect_err("must trap");
        assert!(
            format!("{err:#}").contains("unknown channel id"),
            "wrong error: {err:#}"
        );
    }

    #[test]
    fn needs_chan_runtime_detects_import() {
        let wat = r#"
            (module
              (import "nexus:runtime/chan" "chan-oneshot" (func (result i64))))
        "#;
        let bytes = wat::parse_str(wat).unwrap();
        assert!(needs_chan_runtime(&bytes));
    }

    #[test]
    fn needs_chan_runtime_negative() {
        let wat = r#"(module)"#;
        let bytes = wat::parse_str(wat).unwrap();
        assert!(!needs_chan_runtime(&bytes));
    }

    /// Smoke test for nexus-hs25 (Phase 2a substrate): the engine config used
    /// by the test harness validates a module that declares stack-switching
    /// types (`cont`) + a tag and emits a `suspend` instruction. Proves the
    /// wasmtime `function-references` + `stack-switching` flags are wired
    /// through. We don't yet exercise resume — Phase 2c (codegen) will add an
    /// end-to-end shift/reset round-trip when handler-arm `with @k` lowers.
    #[test]
    fn stack_switching_engine_validates_cont_and_suspend() {
        let wat = r#"
            (module
              (type $iv (func (param i64)))
              (type $cont_iv (cont $iv))
              (tag $t (param i64))

              (func (export "may_suspend") (param i64)
                (suspend $t (i64.const 7))))
        "#;
        let bytes = wat::parse_str(wat).expect("wat compiles");
        let mut config = wasmtime::Config::new();
        config.wasm_tail_call(true);
        config.wasm_exceptions(true);
        config.wasm_function_references(true);
        config.wasm_stack_switching(true);
        let engine = wasmtime::Engine::new(&config).expect("engine");
        wasmtime::Module::from_binary(&engine, &bytes)
            .expect("module with cont type + suspend instruction validates");
    }
}
