use nexus::compiler::passes::hir_build::build_hir;
use nexus::compiler::passes::mir_lower::lower_hir_to_mir;
use nexus::lang::parser;

fn build_mir(src: &str) -> nexus::ir::mir::MirProgram {
    let program = parser::parser().parse(src).unwrap();
    let hir = build_hir(&program).unwrap();
    lower_hir_to_mir(&hir).unwrap()
}

#[test]
fn snapshot_mir_basic() {
    let src = "let main = fn () -> unit do let x = 42 return () end";
    let mir = build_mir(src);
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_mir_with_control_flow() {
    let src = "let main = fn () -> unit do if true then return () else return () end end";
    let mir = build_mir(src);
    insta::assert_debug_snapshot!(mir);
}
