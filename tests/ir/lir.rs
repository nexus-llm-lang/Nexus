use nexus::compiler::passes::hir_build::build_hir;
use nexus::compiler::passes::lir_lower::lower_mir_to_lir;
use nexus::compiler::passes::mir_lower::lower_hir_to_mir;
use nexus::lang::parser;

fn build_lir(src: &str) -> nexus::ir::lir::LirProgram {
    let program = parser::parser().parse(src).unwrap();
    let hir = build_hir(&program).unwrap();
    let mir = lower_hir_to_mir(&hir).unwrap();
    lower_mir_to_lir(&mir, &hir.enum_defs).unwrap()
}

#[test]
fn snapshot_lir_basic() {
    let src = "let main = fn () -> unit do let x = 42 return () end";
    let lir = build_lir(src);
    insta::assert_debug_snapshot!(lir);
}

#[test]
fn snapshot_lir_with_exception() {
    let src = r#"
    exception Boom(i64)
    let main = fn () -> unit do
        try
            raise Boom(42)
        catch e ->
            return ()
        end
    end
    "#;
    let lir = build_lir(src);
    insta::assert_debug_snapshot!(lir);
}


#[test]
fn snapshot_lir_top_level_constant_inlining() {
    let src = r#"
let MY_CONST = 42
let main = fn () -> unit do
  if MY_CONST != 42 then raise RuntimeError(val: "wrong") end
  return ()
end
"#;
    let lir = build_lir(src);
    let main_fn = lir.functions.iter().find(|f| f.name == "main").unwrap();
    // MY_CONST should be inlined as Int(42), not a Constructor
    let body_str = format!("{:?}", main_fn.body);
    assert!(
        !body_str.contains("MY_CONST"),
        "MY_CONST should be inlined, not referenced as constructor"
    );
}
