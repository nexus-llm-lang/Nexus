use nexus::lang::parser;
use nexus::compiler::codegen::compile_program_to_wasm;

fn get_codegen_error(src: &str) -> String {
    let program = parser::parser().parse(src).unwrap();
    let err = compile_program_to_wasm(&program).unwrap_err();
    err.to_string()
}

#[test]
fn snapshot_codegen_error_unsupported_external() {
    let src = r#"
    import external fake.wasm
    external bad = "bad" : (val: i64) -> { x: i64 }
    let main = fn () -> unit do
        let x = bad(val: 42)
        return ()
    end
    "#;
    let err = get_codegen_error(src);
    insta::assert_snapshot!(err);
}
