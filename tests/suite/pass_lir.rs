use nexus::lang::parser;
use nexus::compiler::passes::hir_build::build_hir;
use nexus::compiler::passes::mir_lower::lower_hir_to_mir;
use nexus::compiler::passes::lir_lower::lower_mir_to_lir;

fn build_lir(src: &str) -> nexus::ir::lir::LirProgram {
    let program = parser::parser().parse(src).unwrap();
    let hir = build_hir(&program).unwrap();
    let mir = lower_hir_to_mir(&hir).unwrap();
    lower_mir_to_lir(&mir).unwrap()
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
