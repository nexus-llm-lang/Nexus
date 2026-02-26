use chumsky::Parser;
use nexus::compiler::anf::AnfExpr;
use nexus::compiler::lower::lower_to_typed_anf;
use nexus::lang::parser;

fn lower(src: &str) -> Result<nexus::compiler::anf::AnfProgram, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    lower_to_typed_anf(&program).map_err(|e| e.to_string())
}

#[test]
fn lowers_only_functions_reachable_from_main() {
    let src = r#"
let callee = fn (x: i64) -> i64 do
    return x + 1
endfn

let dead = fn () -> i64 do
    return 0
endfn

let main = fn () -> i64 do
    let v = callee(x: 41)
    return v
endfn
"#;

    let anf = lower(src).expect("ANF lowering should succeed");
    let names: Vec<String> = anf.functions.iter().map(|f| f.name.clone()).collect();
    assert_eq!(names, vec!["callee".to_string(), "main".to_string()]);
}

#[test]
fn reachable_generic_function_is_rejected_for_wasm_mvp() {
    let src = r#"
let id = fn <T>(x: T) -> T do
    return x
endfn

let main = fn () -> i64 do
    return id(x: 1)
endfn
"#;

    let err = lower(src).expect_err("reachable generic should be rejected");
    assert!(err.contains("reachable generic function 'id'"));
}

#[test]
fn unreachable_generic_function_is_ignored_for_now() {
    let src = r#"
let id = fn <T>(x: T) -> T do
    return x
endfn

let main = fn () -> i64 do
    return 1
endfn
"#;

    let anf = lower(src).expect("unreachable generic should not block lowering");
    let names: Vec<String> = anf.functions.iter().map(|f| f.name.clone()).collect();
    assert_eq!(names, vec!["main".to_string()]);
}

#[test]
fn explicit_i32_annotation_drives_binary_result_type() {
    let src = r#"
let main = fn () -> i32 do
    let x: i32 = 1
    return x + 2
endfn
"#;

    let anf = lower(src).expect("ANF lowering should succeed");
    let main_fn = anf
        .functions
        .iter()
        .find(|f| f.name == "main")
        .expect("main function should exist");

    let has_i32_binary = main_fn.body.iter().any(|stmt| {
        if let nexus::compiler::anf::AnfStmt::Let { expr, .. } = stmt {
            matches!(
                expr,
                AnfExpr::Binary {
                    typ: nexus::lang::ast::Type::I32,
                    ..
                }
            )
        } else {
            false
        }
    });
    assert!(has_i32_binary, "expected an i32-typed binary ANF node");
}

#[test]
fn lowering_resolves_print_from_stdlib_signatures() {
    let src = r#"
let main = fn () -> unit effect { Console } do
    print(val: [=[hello]=])
    return ()
endfn
"#;

    let _anf = lower(src).expect("ANF lowering should resolve print");
}

#[test]
fn lowering_rejects_external_without_import_external() {
    let src = r#"
external foo = [=[foo]=] : (x: i64) -> i64

let main = fn () -> i64 do
    return foo(x: 1)
endfn
"#;

    let err = lower(src).expect_err("missing import external should be rejected");
    assert!(err.contains("requires a preceding 'import external"));
}

#[test]
fn lowering_rejects_missing_external_module_file() {
    let src = r#"
import external /tmp/__no_such_module__.wasm
external foo = [=[foo]=] : (x: i64) -> i64

let main = fn () -> i64 do
    return foo(x: 1)
endfn
"#;

    let err = lower(src).expect_err("missing external module file should be rejected");
    assert!(err.contains("external module '/tmp/__no_such_module__.wasm' not found"));
}
