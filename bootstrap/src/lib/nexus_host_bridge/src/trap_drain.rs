//! Trap-resilience drain helper for the bridge guest's thread_local maps.
//!
//! Linearity proves the well-behaved listen→stop and accept→respond paths
//! never leak; a wasm trap, however, unwinds without dropping `RefCell`-stored
//! entries because Rust thread_local Drop only fires on thread exit. The
//! bridge's `host-bridge-finalize` WIT export wraps `drain` over the SERVERS
//! and CONNS maps so an embedder can reclaim the entries (and the wasi
//! resources they own) without rebuilding the wasmtime Store.
//!
//! Kept as a host-buildable module (no `#[cfg(target_family = "wasm")]`) so
//! the drain semantics — idempotency, count return, contained-on-empty —
//! are unit-testable without wasi stubs.

use std::collections::HashMap;

/// Drain a single bridge map, dropping every entry. Returns the count of
/// dropped entries. Idempotent on empty input.
pub fn drain<T>(map: &mut HashMap<i64, T>) -> usize {
    let n = map.len();
    map.clear();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Acceptance for nexus-upzz.9: a populated map drains all entries and
    /// reports the right count. Models a leaked-after-trap state where
    /// SERVERS / CONNS still hold entries that the well-behaved path would
    /// have removed.
    #[test]
    fn drain_removes_every_entry_and_reports_count() {
        let mut map: HashMap<i64, String> = HashMap::new();
        map.insert(1, "server-1".to_string());
        map.insert(2, "server-2".to_string());
        map.insert(3, "conn-3".to_string());

        let dropped = drain(&mut map);
        assert_eq!(dropped, 3, "every entry must be dropped");
        assert!(map.is_empty(), "drain leaves the map empty");
    }

    #[test]
    fn drain_idempotent_on_empty() {
        let mut map: HashMap<i64, ()> = HashMap::new();
        assert_eq!(drain(&mut map), 0);
        assert_eq!(drain(&mut map), 0);
        assert!(map.is_empty());
    }

    /// Verifies that drain runs Drop on every value (the actual reason this
    /// matters: each ServerEntry / ConnEntry owns wasi resource handles whose
    /// Drop releases the underlying socket).
    #[test]
    fn drain_runs_drop_on_each_value() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Counted {
            counter: Arc<AtomicUsize>,
        }
        impl Drop for Counted {
            fn drop(&mut self) {
                self.counter.fetch_add(1, Ordering::SeqCst);
            }
        }

        let counter = Arc::new(AtomicUsize::new(0));
        let mut map: HashMap<i64, Counted> = HashMap::new();
        for id in 0..5_i64 {
            map.insert(
                id,
                Counted {
                    counter: counter.clone(),
                },
            );
        }
        let dropped = drain(&mut map);
        assert_eq!(dropped, 5);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            5,
            "every value's Drop must have run"
        );
    }

    /// Trap-after-listen fixture: emulates the leak by populating SERVERS-like
    /// map, then drain reclaims it.
    #[test]
    fn trap_after_listen_reclaim_via_drain() {
        let mut servers: HashMap<i64, &str> = HashMap::new();
        // do_listen succeeded, then a trap fired before do_stop reached the map.
        servers.insert(7, "owned-tcp-listener");
        // Embedder calls host-bridge-finalize after catching the trap.
        let dropped = drain(&mut servers);
        assert_eq!(dropped, 1);
        assert!(servers.is_empty());
    }

    /// Trap-after-accept fixture: emulates the leak across accept→respond.
    #[test]
    fn trap_after_accept_reclaim_via_drain() {
        let mut conns: HashMap<i64, &str> = HashMap::new();
        // do_accept moved client_socket + streams into CONNS, but the wasm
        // body trapped before do_respond reached the map.
        conns.insert(11, "owned-input-output-streams-and-client-socket");
        let dropped = drain(&mut conns);
        assert_eq!(dropped, 1);
        assert!(conns.is_empty());
    }
}
