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
//! Cross-crate FFI return values (e.g. `Fs.read_to_string` returning a
//! string produced inside the fs sub-component, or `__nx_http_accept`
//! producing a Request buffer in the net sub-component) are reclaimed by
//! user-side `heap_reset` because the canonical ABI lowers each
//! cross-component string return through the caller's `cabi_realloc`,
//! which bumps user-component G0. The
//! `arena_cross_crate_fs_string_workload_*` pair below verifies this by
//! pinning G0 across 5_000 iterations of `Fs.read_to_string` + `substring`
//! + `++` — every per-iter buffer crosses the fs / string / user component
//! boundaries and lands in user memory, where `heap_reset` reclaims it.

use crate::harness::compile::compile_fixture_via_nxc;
use crate::harness::{exec_with_stdlib, TempDir};

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

/// nexus-l28w acceptance #4 — cross-crate workload (fs read + string ops)
/// wrapped in arena.heap_mark / heap_reset. Each iteration calls into the
/// `fs` sub-crate (`__nx_read_to_string` returning a heap-allocated string)
/// and the `string` sub-crate (`length` / `substring` / `from_i64`),
/// concats the results, then resets. With the fix, G0 returns to the
/// pre-iteration value — every cross-crate FFI return value crossed the
/// component boundary into user-component memory via the canonical-ABI
/// `cabi_realloc` lowering and is reclaimed by the user-side G0 rewind.
///
/// This empirically refutes the original l28w framing that each sub-crate
/// owns an independent `nexus_wasm_alloc::ALLOCATIONS` static unreachable
/// from `arena.heap_reset`: in component-model mode (the `exec_with_stdlib`
/// path) every cross-component string-typed return is copied into the
/// caller's own memory by the canonical ABI, so `heap_reset`'s G0 rewind
/// reclaims it regardless of which sub-component originally produced the
/// bytes.
#[test]
fn arena_cross_crate_fs_string_workload_bounded_with_reset() {
    let tmp = TempDir::new("arena_cross_crate");
    let dir = tmp.path();
    let mut src = String::from(
        r#"
import { Console }, * as stdio from "std:stdio"
import { Fs }, * as fs_mod from "std:fs"
import { heap_mark, heap_reset } from "std:arena"
import { length, from_i64, substring } from "std:str"
"#,
    );
    src.push_str(G0_FROM_MARK_HELPER);
    src.push_str(&format!(
        r#"
let main = fn () -> unit require {{ PermConsole, PermFs }} do
  inject stdio.system_handler do
    inject fs_mod.system_handler do
      try
        // Seed a small file so each iteration reads fresh bytes from fs sub-crate.
        Fs.create_dir_all(path: "{dir}")
        let path = "{dir}/seed.txt"
        Fs.write_string(path: path, content: "cross-crate-fs-payload-1234567890")
        let mark0 = heap_mark()
        let g0_before = g0_of(mark: mark0)
        let ~i = 0
        let ~acc = 0
        while ~i < 5000 do
          let m = heap_mark()
          // fs sub-crate: returns a string allocated by host bridge,
          // lifted into user memory via canonical ABI.
          let body = Fs.read_to_string(path: path)
          // string sub-crate: substring + length operate on lifted bytes.
          let head = substring(s: body, start: 0, len: 4)
          let tagged = "iter=" ++ from_i64(val: ~i) ++ " head=" ++ head
          ~acc <- ~acc + length(s: tagged)
          heap_reset(mark: m)
          ~i <- ~i + 1
        end
        let mark1 = heap_mark()
        let g0_after = g0_of(mark: mark1)
        // Self-assertion: cross-crate FFI returns + intermediate concats are
        // all reclaimed. Trap on mismatch via i64.div_s.
        let ok = if g0_after == g0_before then 1 else 0 end
        let _ = 1 / ok
        Console.println(val: "cross-crate bounded ok: g0_delta=" ++ from_i64(val: g0_after - g0_before))
        return ()
      catch e ->
        raise RuntimeError(val: "fs op failed in cross-crate bounded test")
      end
    end
  end
end
"#
    ));
    exec_with_stdlib(&src);
}

/// nexus-l28w companion to the above — same cross-crate fs+string workload
/// without heap_mark / heap_reset wrapping. G0 must grow monotonically by
/// at least one byte per iteration. Demonstrates that the bounded-G0 result
/// in the wrapped variant comes from the reset, not from accidental
/// allocation elision.
#[test]
fn arena_cross_crate_fs_string_workload_grows_without_reset() {
    let tmp = TempDir::new("arena_cross_crate_no_reset");
    let dir = tmp.path();
    let mut src = String::from(
        r#"
import { Console }, * as stdio from "std:stdio"
import { Fs }, * as fs_mod from "std:fs"
import { heap_mark } from "std:arena"
import { length, from_i64, substring } from "std:str"
"#,
    );
    src.push_str(G0_FROM_MARK_HELPER);
    src.push_str(&format!(
        r#"
let main = fn () -> unit require {{ PermConsole, PermFs }} do
  inject stdio.system_handler do
    inject fs_mod.system_handler do
      try
        Fs.create_dir_all(path: "{dir}")
        let path = "{dir}/seed.txt"
        Fs.write_string(path: path, content: "cross-crate-fs-payload-1234567890")
        let mark0 = heap_mark()
        let g0_before = g0_of(mark: mark0)
        let ~i = 0
        let ~acc = 0
        while ~i < 500 do
          let body = Fs.read_to_string(path: path)
          let head = substring(s: body, start: 0, len: 4)
          let tagged = "iter=" ++ from_i64(val: ~i) ++ " head=" ++ head
          ~acc <- ~acc + length(s: tagged)
          ~i <- ~i + 1
        end
        let mark1 = heap_mark()
        let g0_after = g0_of(mark: mark1)
        let ok = if g0_after > g0_before + 1000 then 1 else 0 end
        let _ = 1 / ok
        Console.println(val: "cross-crate leak observed: g0_delta=" ++ from_i64(val: g0_after - g0_before))
        return ()
      catch e ->
        raise RuntimeError(val: "fs op failed in cross-crate growing test")
      end
    end
  end
end
"#
    ));
    exec_with_stdlib(&src);
}
