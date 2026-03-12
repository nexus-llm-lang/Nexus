use nexus::compiler::codegen::compile_program_to_wasm;
use nexus::lang::parser;

/// Parse + compile to core WASM bytes. Panics on failure.
pub fn compile(src: &str) -> Vec<u8> {
    let program = parser::parser()
        .parse(src)
        .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    compile_program_to_wasm(&program).unwrap_or_else(|e| panic!("compile failed: {}", e))
}

/// Parse + compile to core WASM bytes. Returns error on failure.
pub fn try_compile(src: &str) -> Result<Vec<u8>, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    compile_program_to_wasm(&program).map_err(|e| e.to_string())
}

/// Parse + compile, returning the codegen error message. Panics if compilation succeeds.
pub fn get_codegen_error(src: &str) -> String {
    let program = parser::parser().parse(src).unwrap();
    let err = compile_program_to_wasm(&program).unwrap_err();
    err.to_string()
}
