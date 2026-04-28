use nexus::compiler::codegen::{compile_program_to_wasm, compile_program_to_wasm_threaded};
use nexus::lang::parser;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonically-increasing counter combined with PID + ThreadId to produce a
/// unique wasm output path per `compile_fixture_via_nxc*` invocation. Without
/// this, all parallel test threads share a single
/// `nxc_test_<pid>.wasm` path and race-overwrite each other's compiled
/// fixture — manifesting as `sched_*` tests reading whatever wasm a sibling
/// test thread last wrote.
static NXC_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn nxc_output_path(prefix: &str) -> std::path::PathBuf {
    let seq = NXC_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tid = format!("{:?}", std::thread::current().id());
    // ThreadId Debug fmt is e.g. "ThreadId(7)"; strip the wrapper to keep the
    // filename short and POSIX-portable.
    let tid_digits: String = tid.chars().filter(|c| c.is_ascii_digit()).collect();
    std::env::temp_dir().join(format!(
        "{prefix}_{}_{}_{}.wasm",
        std::process::id(),
        tid_digits,
        seq
    ))
}

/// Parse + compile to core WASM bytes. Panics on failure.
pub fn compile(src: &str) -> Vec<u8> {
    super::ensure_repo_root();
    let program = parser::parser()
        .parse(src)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    compile_program_to_wasm(&program).unwrap_or_else(|e| panic!("compile failed: {}", e))
}

/// Parse + compile via the threaded codegen path (host-imported shared memory).
/// Used by `exec_threaded` to drive the LazyRuntime::with_shared_memory mode.
/// Returns `(wasm_bytes, heap_base)` — the harness uses heap_base to seed the
/// runtime's atomic bump allocator so worker thunks share the caller's heap.
pub fn compile_threaded(src: &str) -> (Vec<u8>, i32) {
    super::ensure_repo_root();
    let program = parser::parser()
        .parse(src)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    compile_program_to_wasm_threaded(&program)
        .unwrap_or_else(|e| panic!("threaded compile failed: {}", e))
}

/// Parse + compile to core WASM bytes. Returns error on failure.
pub fn try_compile(src: &str) -> Result<Vec<u8>, String> {
    super::ensure_repo_root();
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    compile_program_to_wasm(&program).map_err(|e| e.to_string())
}

/// Parse + compile, returning the codegen error message. Panics if compilation succeeds.
pub fn get_codegen_error(src: &str) -> String {
    super::ensure_repo_root();
    let program = parser::parser().parse(src).unwrap();
    let err = compile_program_to_wasm(&program).unwrap_err();
    err.to_string()
}

/// Compile a fixture file via the self-hosted compiler (nexus.wasm).
/// Returns the output WASM bytes. Panics if compilation fails.
pub fn compile_fixture_via_nxc(fixture_relpath: &str) -> Vec<u8> {
    super::ensure_repo_root();
    let cwd = std::env::current_dir().expect("cwd");
    let nexus_wasm = cwd.join("nexus.wasm");
    assert!(
        nexus_wasm.exists(),
        "nexus.wasm not found — run bootstrap.sh first"
    );
    let output_pathbuf = nxc_output_path("nxc_test");
    let output_path = output_pathbuf.to_str().unwrap().to_string();
    let cwd_str = cwd.to_str().unwrap();
    let result = Command::new("wasmtime")
        .args([
            "run",
            "-W",
            "tail-call=y,exceptions=y,function-references=y,stack-switching=y,max-memory-size=8589934592",
            &format!("--dir={cwd_str}::."),
            &format!("--dir={}", std::env::temp_dir().display()),
        ])
        .arg(nexus_wasm.to_str().unwrap())
        .arg(fixture_relpath)
        .arg(&output_path)
        .output()
        .expect("failed to invoke wasmtime");
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let _ = std::fs::remove_file(&output_path);
        panic!(
            "nxc compilation failed (exit {}):\nstderr: {stderr}\nstdout: {stdout}",
            result.status
        );
    }
    let bytes = std::fs::read(&output_path).expect("failed to read nxc output wasm");
    let _ = std::fs::remove_file(&output_path);
    bytes
}

/// Compile a fixture file via the self-hosted compiler and expect failure.
/// Returns the stderr output for assertion.
pub fn compile_fixture_via_nxc_should_fail(fixture_relpath: &str) -> String {
    super::ensure_repo_root();
    let cwd = std::env::current_dir().expect("cwd");
    let nexus_wasm = cwd.join("nexus.wasm");
    assert!(
        nexus_wasm.exists(),
        "nexus.wasm not found — run bootstrap.sh first"
    );
    let output_pathbuf = nxc_output_path("nxc_test_fail");
    let output_path = output_pathbuf.to_str().unwrap().to_string();
    let cwd_str = cwd.to_str().unwrap();
    let result = Command::new("wasmtime")
        .args([
            "run",
            "-W",
            "tail-call=y,exceptions=y,function-references=y,stack-switching=y,max-memory-size=8589934592",
            &format!("--dir={cwd_str}::."),
            &format!("--dir={}", std::env::temp_dir().display()),
        ])
        .arg(nexus_wasm.to_str().unwrap())
        .arg(fixture_relpath)
        .arg(&output_path)
        .output()
        .expect("failed to invoke wasmtime");
    let _ = std::fs::remove_file(&output_path);
    assert!(
        !result.status.success(),
        "expected nxc compilation to fail but it succeeded"
    );
    String::from_utf8_lossy(&result.stderr).to_string()
}
