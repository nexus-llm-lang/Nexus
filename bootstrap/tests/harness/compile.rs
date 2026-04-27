use nexus::compiler::codegen::{compile_program_to_wasm, compile_program_to_wasm_threaded};
use nexus::lang::parser;
use std::process::Command;

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
    let output_path = std::env::temp_dir()
        .join(format!("nxc_test_{}.wasm", std::process::id()))
        .to_str()
        .unwrap()
        .to_string();
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
        panic!(
            "nxc compilation failed (exit {}):\nstderr: {stderr}\nstdout: {stdout}",
            result.status
        );
    }
    std::fs::read(&output_path).expect("failed to read nxc output wasm")
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
    let output_path = std::env::temp_dir()
        .join(format!("nxc_test_fail_{}.wasm", std::process::id()))
        .to_str()
        .unwrap()
        .to_string();
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
    assert!(
        !result.status.success(),
        "expected nxc compilation to fail but it succeeded"
    );
    String::from_utf8_lossy(&result.stderr).to_string()
}
