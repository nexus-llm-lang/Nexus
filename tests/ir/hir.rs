use nexus::compiler::passes::hir_build::build_hir;
use nexus::lang::parser;

fn parse_and_build_mir(
    src: &str,
) -> Result<nexus::ir::mir::MirProgram, nexus::compiler::passes::hir_build::HirBuildError> {
    let program = parser::parser().parse(src).unwrap();
    build_hir(&program)
}

#[test]
fn snapshot_hir_basic() {
    let src = "let main = fn () -> unit do return () end";
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
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
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_hir_match_with_constructors() {
    let src = r#"
    type Color = Red | Green | Blue
    let main = fn () -> unit do
        let c = Red
        match c do
          case Red -> return ()
          case Green -> return ()
          case Blue -> return ()
        end
    end
    "#;
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_hir_function_with_generics() {
    let src = r#"
    let id = fn <T>(x: T) -> T do return x end
    let main = fn () -> unit do
        let _ = id(x: 42)
        return ()
    end
    "#;
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_hir_exception_and_try_catch() {
    let src = r#"
    exception Boom(i64)
    let main = fn () -> unit throws { Exn } do
        try
            raise Boom(42)
        catch e ->
            return ()
        end
    end
    "#;
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_hir_record_and_field_access() {
    let src = r#"
    let main = fn () -> unit do
        let r = { x: 1, y: 2 }
        let v = r.x
        return ()
    end
    "#;
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
}

#[test]
fn snapshot_hir_while_loop() {
    let src = r#"
    let main = fn () -> unit do
        let ~i = 0
        while ~i < 5 do
            ~i <- ~i + 1
        end
        return ()
    end
    "#;
    let mir = parse_and_build_mir(src).unwrap();
    insta::assert_debug_snapshot!(mir);
}
