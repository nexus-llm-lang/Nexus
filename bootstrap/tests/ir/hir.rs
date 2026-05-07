use nexus::compiler::passes::hir_build::build_hir;
use nexus::lang::parser;

fn parse_and_build_mir(
    src: &str,
) -> Result<nexus::ir::mir::MirProgram, nexus::compiler::passes::hir_build::HirBuildError> {
    crate::harness::ensure_repo_root();
    let program = parser::parser().parse(src).unwrap();
    build_hir(&program)
}

#[test]
fn top_level_let_with_call_initializer_is_rejected() {
    // Regression for nexus-lfup: module-level `let x = some_call()` used to
    // be silently dropped by hir_build, producing a cryptic "Unresolved type
    // in LIR lowering" error at reference sites. It must now surface a clear
    // UnsupportedTopLevelLet error naming the offending binding.
    use nexus::compiler::passes::hir_build::HirBuildError;
    let src = r#"
    external __nx_clock_now = "__nx_clock_now" : () -> i64
    let start_time = __nx_clock_now()
    let main = fn () -> unit do
        let _v = start_time
    end
    "#;
    let err = parse_and_build_mir(src).expect_err("expected UnsupportedTopLevelLet");
    match err {
        HirBuildError::UnsupportedTopLevelLet { name, .. } => {
            assert_eq!(name, "start_time");
        }
        other => panic!("unexpected error variant: {:?}", other),
    }
}
