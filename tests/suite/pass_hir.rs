use nexus::lang::parser;
use nexus::compiler::passes::hir_build::build_hir;

fn parse_and_build_hir(src: &str) -> Result<nexus::ir::hir::HirProgram, nexus::compiler::passes::hir_build::HirBuildError> {
    let program = parser::parser().parse(src).unwrap();
    build_hir(&program)
}

#[test]
fn snapshot_hir_basic() {
    let src = "let main = fn () -> unit do return () end";
    let hir = parse_and_build_hir(src).unwrap();
    insta::assert_debug_snapshot!(hir);
}

#[test]
fn snapshot_hir_with_handler() {
    let src = r#"
    port Console do fn println(s: string) -> unit end
    let my_handler = handler Console do
        fn println(s: string) -> unit do return () end
    end
    let main = fn () -> unit do
        inject my_handler do
            Console.println(val: "hello")
        end
    end
    "#;
    let hir = parse_and_build_hir(src).unwrap();
    insta::assert_debug_snapshot!(hir);
}
