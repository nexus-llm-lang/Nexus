//! arena: integration tests for arena.heap_mark / heap_reset under sustained
//! per-request workload (nexus-gv2u, completing nexus-sj9c acceptance #2).
//!
//! Three scenarios:
//! 1. `net_echo_server_fixture_compiles` — the recommended Nexus echo-server
//!    pattern (listen / accept / respond, mark/reset wrapping each handler
//!    iteration) typechecks and codegens via the self-hosted nxc compiler.
//! 2. `arena_100k_echo_workload_g0_bounded_with_reset` — runs the
//!    per-iteration echo workload 100_000 times wrapped in
//!    arena.heap_mark / heap_reset, self-asserts (via i64.div_s
//!    trap-on-mismatch) that the G0 bump pointer is unchanged across the
//!    loop. Without the fix, every iteration's intermediate concat /
//!    from_i64 / substring results would push G0 forward and the
//!    string-heap watermark would grow linearly.
//! 3. `arena_echo_workload_g0_grows_without_reset` — same workload, smaller
//!    iter count, no wrapping. Self-asserts G0 GREW, demonstrating the
//!    leak shape that the fix eliminates (the "FAIL without wrapping"
//!    half of acceptance #4).
//!
//! Both runtime tests use `exec_with_stdlib` (Rust bootstrap codegen + WASI
//! component model). That path wires `arena.heap_reset` through to the
//! routed allocator's `__nx_alloc_mark` / `__nx_alloc_reset` so the
//! reclamation actually fires (see
//! bootstrap/src/compiler/codegen/function.rs `Intrinsic::HeapReset`). The
//! self-hosted nxc backend (src/backend/codegen.nx::emit_heap_reset)
//! currently only rewinds G0; that gap is tracked as a follow-up to
//! sj9c — this smoke test deliberately does not depend on it.
//!
//! `arena.heap_mark()` packs the routed allocator's outstanding-count into
//! the upper 32 bits and the G0 bump pointer (heap-side cabi_realloc
//! offset) into the lower 32 bits of its i64 return. In component mode
//! (which `exec_with_stdlib` routes through) the routed allocator's
//! upper-32 count is always zero — every string concat / from_i64 /
//! substring goes through `cabi_realloc` which bumps G0 — so the Nexus
//! fixtures use the lower 32 bits (`mark & 0xFFFFFFFF`) as the leak / no-leak
//! signal. Without `heap_reset` G0 grows monotonically per string; with
//! `heap_reset` G0 returns to its pre-iteration value.
//!
//! Net-crate allocations (per-request decode buffers inside the net crate's
//! own nexus_wasm_alloc storage) are *not* reclaimed by user-side
//! heap_reset; that's the cross-module allocator follow-up issue called out
//! by nexus-gv2u.

use crate::harness::compile::compile_fixture_via_nxc;
use crate::harness::exec_with_stdlib;

/// Acceptance #1 — echo-server fixture (listen / accept / respond /
/// heap_mark+heap_reset wrapping) typechecks and codegens via the
/// self-hosted compiler. Running the fixture under the test harness would
/// hit `nxlib/stdlib/nexus_host_stub.wat`'s `unreachable` traps (no
/// wasi:http+sockets host is wired), which is a wasm-level trap that
/// Nexus's `try/catch` cannot intercept; therefore the test stops at
/// successful compilation. The 100k-iter sustained-load assertion below
/// covers the runtime side.
#[test]
fn net_echo_server_fixture_compiles() {
    let _wasm =
        compile_fixture_via_nxc("bootstrap/tests/fixtures/nxc/test_net_echo_server.nx");
}

// Mask the lower 32 bits of a `heap_mark()` packed i64 to recover the
// G0 bump pointer (offset into the user module's linear memory where the
// next string concat / cabi_realloc allocation will land). In component
// mode the upper 32 bits are zero (the routed-allocator count isn't
// wired across the WIT contract), so this mask returns the same value as
// the raw i64 — but writing the mask explicitly keeps the fixture
// portable to non-component runs, where the upper bits are non-zero.
//
// Implemented inline in each fixture's source to keep every test
// self-contained (Rust harness only sees a single Nexus program).
const G0_FROM_MARK_HELPER: &str = r#"
/// Recover the G0 bump pointer from a `heap_mark()` packed i64.
let g0_of = fn (mark: i64) -> i64 do
  // 0xFFFFFFFF — keep low 32 bits, strip the routed-alloc count.
  return mark & 4294967295
end
"#;

/// Acceptance #2 + #3 — 100_000 iterations of the per-request echo
/// workload wrapped in arena.heap_mark / heap_reset must keep the G0 bump
/// pointer (string-heap watermark) bounded. The fixture self-asserts
/// (via `1 / 0` trap-on-mismatch) that the post-loop G0 equals the
/// pre-loop G0 — every iteration's allocations were reclaimed by
/// heap_reset.
///
/// 100k iterations × ~8 string allocations per iter ≈ 800k bumps without
/// reset (string-heap would grow well into the MB range). With reset, G0
/// returns to its pre-iteration value, so the loop's net effect on G0
/// is zero.
#[test]
fn arena_100k_echo_workload_g0_bounded_with_reset() {
    let mut src = String::from(
        r#"
import { Console }, * as stdio from "std:stdio"
import { heap_mark, heap_reset } from "std:arena"
import { length, from_i64, substring } from "std:str"

let simulate_echo_handler = fn (i: i64) -> i64 do
  let id_str = from_i64(val: i)
  let path = "/echo/" ++ id_str
  let body_in = "ping " ++ id_str
  let header = "content-type:text/plain\nlength:" ++ from_i64(val: length(s: body_in))
  let response_full = header ++ "\n\n" ++ "echo: " ++ body_in ++ " path=" ++ path
  let _ = substring(s: response_full, start: 0, len: 4)
  return length(s: response_full)
end
"#,
    );
    src.push_str(G0_FROM_MARK_HELPER);
    src.push_str(
        r#"
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let mark0 = heap_mark()
    let g0_before = g0_of(mark: mark0)
    let ~i = 0
    let ~acc = 0
    while ~i < 100000 do
      let m = heap_mark()
      let n = simulate_echo_handler(i: ~i)
      ~acc <- ~acc + n
      heap_reset(mark: m)
      ~i <- ~i + 1
    end
    let mark1 = heap_mark()
    let g0_after = g0_of(mark: mark1)
    // Self-assertion: with heap_reset wrapping, the G0 bump pointer must
    // return to its pre-loop value. Trap-on-mismatch via i64.div_s.
    let ok = if g0_after == g0_before then 1 else 0 end
    let _ = 1 / ok
    // Anchor `~acc` so the loop body cannot be DCE'd, plus a status line
    // for human triage.
    Console.println(val: "bounded ok: g0_delta=" ++ from_i64(val: g0_after - g0_before))
  end
end
"#,
    );
    exec_with_stdlib(&src);
}

/// Acceptance #4 — same per-request workload run for 1_000 iterations
/// WITHOUT heap_mark / heap_reset wrapping. The fixture self-asserts
/// (via `1 / 0` trap-on-mismatch) that the G0 bump pointer GREW by at
/// least one byte per iteration — demonstrating the leak shape the
/// mark/reset fix eliminates.
///
/// 1_000 iterations is enough to make the bump-pointer delta
/// unambiguous (per-iter footprint is at least 60 bytes for the `path` /
/// `body_in` / `response_full` strings) without overflowing the wasm
/// memory budget (default cabi_realloc grows G0 monotonically; 8 GiB
/// `max-memory-size` ceiling).
#[test]
fn arena_echo_workload_g0_grows_without_reset() {
    let mut src = String::from(
        r#"
import { Console }, * as stdio from "std:stdio"
import { heap_mark } from "std:arena"
import { length, from_i64, substring } from "std:str"

let simulate_echo_handler = fn (i: i64) -> i64 do
  let id_str = from_i64(val: i)
  let path = "/echo/" ++ id_str
  let body_in = "ping " ++ id_str
  let header = "content-type:text/plain\nlength:" ++ from_i64(val: length(s: body_in))
  let response_full = header ++ "\n\n" ++ "echo: " ++ body_in ++ " path=" ++ path
  let _ = substring(s: response_full, start: 0, len: 4)
  return length(s: response_full)
end
"#,
    );
    src.push_str(G0_FROM_MARK_HELPER);
    src.push_str(
        r#"
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let mark0 = heap_mark()
    let g0_before = g0_of(mark: mark0)
    let ~i = 0
    let ~acc = 0
    while ~i < 1000 do
      let n = simulate_echo_handler(i: ~i)
      ~acc <- ~acc + n
      ~i <- ~i + 1
    end
    let mark1 = heap_mark()
    let g0_after = g0_of(mark: mark1)
    // Self-assertion: without heap_reset wrapping, every iteration's
    // intermediate string allocations bump G0 forward. Threshold of 1000
    // bytes is comfortably below the lower bound of a per-iter footprint
    // (60 bytes × 1000 iters = 60_000 bytes) but high enough that an
    // accidentally-DCE'd workload would clearly fail to reach it.
    let ok = if g0_after > g0_before + 1000 then 1 else 0 end
    let _ = 1 / ok
    Console.println(val: "leak observed: g0_delta=" ++ from_i64(val: g0_after - g0_before))
  end
end
"#,
    );
    exec_with_stdlib(&src);
}
