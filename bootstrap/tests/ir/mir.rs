use nexus::compiler::passes::hir_build::build_hir;
use nexus::lang::parser;

fn build_mir(src: &str) -> nexus::ir::mir::MirProgram {
    crate::harness::ensure_repo_root();
    let program = parser::parser().parse(src).unwrap();
    build_hir(&program).unwrap()
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

#[test]
fn snapshot_mir_function_call() {
    let src = r#"
    let add = fn (a: i64, b: i64) -> i64 do return a + b end
    let main = fn () -> unit do
        let _ = add(a: 1, b: 2)
        return ()
    end
    "#;
    let mir = build_mir(src);
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_mir_match_statement() {
    let src = r#"
    let main = fn () -> unit do
        let x = 42
        match x do
          case 0 -> return ()
          case _ -> return ()
        end
    end
    "#;
    let mir = build_mir(src);
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_mir_port_handler_inject() {
    let src = r#"
    port Logger do fn log(msg: string) -> unit end
    let my_handler = handler Logger do
        fn log(msg: string) -> unit do return () end
    end
    let main = fn () -> unit do
        inject my_handler do
            Logger.log(msg: "test")
        end
        return ()
    end
    "#;
    let mir = build_mir(src);
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_mir_string_concat() {
    let src = r#"
    let main = fn () -> unit do
        let s = "hello" ++ " world"
        return ()
    end
    "#;
    let mir = build_mir(src);
    insta::assert_debug_snapshot!(mir);
}
