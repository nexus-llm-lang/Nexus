//! Regression test for the cfy4 4GB linear-memory cliff (nexus-xcp6).
//!
//! Background:
//! cfy4 surfaced the self-hosted compiler hitting the 32-bit wasm 4GB linear
//! memory cap when compiling moderately large programs. The xcp6 audit traced
//! it to multiple O(n²) hot paths in stdlib (HashMap.keys/values via .nth(),
//! StringMap.keys parsing, JSON serializer string concat in loop, several
//! string.nx accumulator helpers, etc.). Sub-issues A (HashMap.keys/values),
//! F (string _go helpers), and I (json entries() accessor) have landed; D
//! (JSON serializer), B+C+E (stringmap/set/json parser concat-in-loop), and
//! G+H (proc.exec / network.encode_headers join_args) are still pending.
//!
//! This test guards against regression by:
//!   1. Generating a synthetic Nexus source with N trivial top-level
//!      `let f<i>` definitions (forces the compiler's symbol-table/HashMap
//!      paths to walk N entries during typecheck + codegen).
//!   2. Compiling it with the self-hosted compiler (nexus.wasm) under a
//!      tight wasmtime `max-memory-size` cap with `trap-on-grow-failure`.
//!   3. Asserting the compile succeeds — i.e. peak linear memory stayed
//!      under the cap, which is only possible if memory growth is linear
//!      (or better) in N.
//!
//! Sizing: at the time of writing (post-A/F/I) N=2000 functions compile in
//! ~3s with peak memory well under 256 MiB. A future O(N²) regression in any
//! audited path would balloon allocation by ~16x (2000² vs 500²) and trip
//! the cap. The test thus deterministically catches a quadratic regression
//! without any flaky timing assertions.
//!
//! If this test starts failing on a clean tree, run with N halved to
//! disambiguate "real cliff regression" vs "test budget too tight".

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_paths(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let seq = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let src = std::env::temp_dir().join(format!("cliff_{label}_{pid}_{seq}.nx"));
    let out = std::env::temp_dir().join(format!("cliff_{label}_{pid}_{seq}.wasm"));
    (src, out)
}

/// Build a synthetic Nexus source: empty `main` plus `n` trivial top-level
/// `let f<i> = fn () -> i64 do return <i> end` definitions. This forces the
/// front-end to populate a symbol table of N entries and the back-end to
/// codegen N function bodies — exercising the HashMap / StringMap / lookup
/// paths audited in xcp6 without dragging in stdlib imports.
fn synth_source(n: usize) -> String {
    let mut s = String::with_capacity(n * 48 + 64);
    s.push_str("let main = fn () -> unit do\n  return ()\nend\n\n");
    for i in 0..n {
        s.push_str(&format!(
            "let f{i} = fn () -> i64 do return {i} end\n"
        ));
    }
    s
}

/// Run `wasmtime run nexus.wasm build <src> <out>` with a tight memory cap.
/// `max_memory_bytes` caps the linear memory wasmtime grants the compiler;
/// `trap-on-grow-failure=y` turns OOM into a hard trap (visible exit code).
fn run_self_hosted_compile(
    src_path: &std::path::Path,
    out_path: &std::path::Path,
    max_memory_bytes: u64,
) -> std::process::Output {
    crate::harness::ensure_repo_root();
    let cwd = std::env::current_dir().expect("cwd");
    let nexus_wasm = cwd.join("nexus.wasm");
    assert!(
        nexus_wasm.exists(),
        "nexus.wasm not found at {:?} — run bootstrap.sh first",
        nexus_wasm
    );
    let cwd_str = cwd.to_str().unwrap();
    let w_flags = format!(
        "tail-call=y,exceptions=y,function-references=y,stack-switching=y,\
         max-memory-size={max_memory_bytes},trap-on-grow-failure=y"
    );
    Command::new("wasmtime")
        .args([
            "run",
            "-W",
            &w_flags,
            &format!("--dir={cwd_str}::."),
            &format!("--dir={}", std::env::temp_dir().display()),
        ])
        .arg(&nexus_wasm)
        .arg("build")
        .arg(src_path)
        .arg(out_path)
        .output()
        .expect("failed to invoke wasmtime")
}

/// Compile a synthetic N-function source with the self-hosted compiler under
/// a tight 256 MiB linear-memory cap. Asserts compilation succeeds — any
/// O(N²) regression in the audited stdlib paths (xcp6 A-I) would balloon
/// allocation past the cap and trap.
///
/// N=2000 was chosen empirically: post-xcp6-A/F/I peak memory at this size
/// is well under 256 MiB and compile time is ~3s on a modern laptop. A
/// quadratic regression would push allocation roughly 16x relative to N=500,
/// which exceeds the cap.
#[test]
fn cliff_scaling_2000_functions_under_256mb() {
    const N: usize = 2000;
    const CAP_BYTES: u64 = 256 * 1024 * 1024;

    let (src_path, out_path) = unique_paths("scaling");
    std::fs::write(&src_path, synth_source(N))
        .expect("failed to write synthetic source");

    let result = run_self_hosted_compile(&src_path, &out_path, CAP_BYTES);

    let _ = std::fs::remove_file(&src_path);
    let _ = std::fs::remove_file(&out_path);

    if !result.status.success() {
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        panic!(
            "cliff regression: self-hosted compile of N={N} functions failed under \
             {CAP_MB} MiB linear-memory cap (exit {status}). \
             A failure here typically means an O(n²) hot path was reintroduced — \
             review nexus-xcp6 findings.\nstderr: {stderr}\nstdout: {stdout}",
            CAP_MB = CAP_BYTES / (1024 * 1024),
            status = result.status,
        );
    }
}
