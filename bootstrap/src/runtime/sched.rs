//! Runtime host functions for the cooperative scheduler (Phase 3 of nexus-5ldt).
//!
//! Provides a FIFO run-queue of continuation table indices plus a
//! `__nx_sched_resume_slot` setter so handler arm functions written in
//! Nexus can drive round-robin scheduling without direct WASM global access.
//!
//! Host module: `nexus:runtime/sched`
//!
//! Functions:
//!   sched-enqueue(k: i64)              — push k to run queue
//!   sched-dequeue() -> i64             — pop front, or -1 if empty
//!   sched-set-next(k: i64)             — write __nx_sched_resume_slot
//!   sched-queue-size() -> i64          — current queue length

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use wasmtime::{Caller, Error, Linker, Val};

pub const SCHED_HOST_MODULE: &str = "nexus:runtime/sched";

pub fn needs_sched_runtime(wasm_bytes: &[u8]) -> bool {
    use wasmparser::{Parser, Payload};
    for payload in Parser::new(0).parse_all(wasm_bytes) {
        if let Ok(Payload::ImportSection(section)) = payload {
            for import in section {
                if let Ok(import) = import {
                    if import.module == SCHED_HOST_MODULE {
                        return true;
                    }
                }
            }
        }
    }
    false
}

struct SharedRuntime {
    queue: Mutex<VecDeque<i64>>,
    pending: Mutex<VecDeque<i64>>,
}

pub struct SchedRuntime(Arc<SharedRuntime>);

impl SchedRuntime {
    pub fn new() -> Self {
        SchedRuntime(Arc::new(SharedRuntime {
            queue: Mutex::new(VecDeque::new()),
            pending: Mutex::new(VecDeque::new()),
        }))
    }

    pub fn register<T: Send + 'static>(&self, linker: &mut Linker<T>) -> Result<(), String> {
        for name in ["sched-enqueue", "__nx_sched_enqueue"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(SCHED_HOST_MODULE, name, move |_: Caller<'_, T>, k: i64| {
                    rt.queue.lock().unwrap().push_back(k);
                })
                .map_err(|e| e.to_string())?;
        }

        for name in ["sched-dequeue", "__nx_sched_dequeue"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    SCHED_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>| -> i64 {
                        rt.queue.lock().unwrap().pop_front().unwrap_or(-1)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        for name in ["sched-set-next", "__nx_sched_set_next"] {
            linker
                .func_wrap(
                    SCHED_HOST_MODULE,
                    name,
                    move |mut caller: Caller<'_, T>, k: i64| -> Result<(), Error> {
                        let global = caller
                            .get_export("__nx_sched_resume_slot")
                            .ok_or_else(|| Error::msg("missing __nx_sched_resume_slot export"))?
                            .into_global()
                            .ok_or_else(|| {
                                Error::msg("__nx_sched_resume_slot is not a global")
                            })?;
                        global.set(&mut caller, Val::I64(k))?;
                        Ok(())
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        for name in ["sched-queue-size", "__nx_sched_queue_size"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    SCHED_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>| -> i64 {
                        rt.queue.lock().unwrap().len() as i64
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        for name in ["sched-push-pending", "__nx_sched_push_pending"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(SCHED_HOST_MODULE, name, move |_: Caller<'_, T>, t: i64| {
                    rt.pending.lock().unwrap().push_back(t);
                })
                .map_err(|e| e.to_string())?;
        }

        for name in ["sched-pop-pending", "__nx_sched_pop_pending"] {
            let rt = Arc::clone(&self.0);
            linker
                .func_wrap(
                    SCHED_HOST_MODULE,
                    name,
                    move |_: Caller<'_, T>| -> i64 {
                        rt.pending.lock().unwrap().pop_front().unwrap_or(0)
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

impl Default for SchedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::{Engine, Module, Store};

    #[test]
    fn enqueue_dequeue_roundtrip() {
        let rt = SchedRuntime::new();
        {
            let mut q = rt.0.queue.lock().unwrap();
            q.push_back(42);
            q.push_back(7);
            assert_eq!(q.pop_front(), Some(42));
            assert_eq!(q.pop_front(), Some(7));
            assert_eq!(q.pop_front(), None);
        }
    }

    #[test]
    fn host_functions_link() {
        let engine = Engine::default();
        let mut linker = Linker::<()>::new(&engine);
        SchedRuntime::new().register(&mut linker).expect("register");

        let wat = r#"
            (module
              (import "nexus:runtime/sched" "sched-enqueue" (func $enq (param i64)))
              (import "nexus:runtime/sched" "sched-dequeue" (func $deq (result i64)))
              (import "nexus:runtime/sched" "sched-queue-size" (func $sz (result i64)))
              (func (export "main")
                (call $enq (i64.const 10))
                (call $enq (i64.const 20))
                ;; size should be 2
                (if (i64.ne (call $sz) (i64.const 2))
                  (then (unreachable)))
                ;; dequeue 10
                (if (i64.ne (call $deq) (i64.const 10))
                  (then (unreachable)))
                ;; dequeue 20
                (if (i64.ne (call $deq) (i64.const 20))
                  (then (unreachable)))
                ;; dequeue empty → -1
                (if (i64.ne (call $deq) (i64.const -1))
                  (then (unreachable)))))
        "#;
        let module = Module::new(&engine, wat).expect("parse");
        let mut store = Store::new(&engine, ());
        let inst = linker.instantiate(&mut store, &module).expect("inst");
        let main = inst
            .get_typed_func::<(), ()>(&mut store, "main")
            .expect("main");
        main.call(&mut store, ()).expect("main should pass");
    }

    #[test]
    fn needs_sched_runtime_detects_import() {
        let wat = r#"
            (module
              (import "nexus:runtime/sched" "sched-enqueue" (func (param i64))))
        "#;
        let bytes = wat::parse_str(wat).unwrap();
        assert!(needs_sched_runtime(&bytes));
    }

    #[test]
    fn needs_sched_runtime_negative() {
        let bytes = wat::parse_str("(module)").unwrap();
        assert!(!needs_sched_runtime(&bytes));
    }
}
