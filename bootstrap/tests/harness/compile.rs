use nexus::compiler::codegen::{compile_program_to_wasm, compile_program_to_wasm_threaded};
use nexus::lang::parser;

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
pub fn compile_threaded(src: &str) -> Vec<u8> {
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
