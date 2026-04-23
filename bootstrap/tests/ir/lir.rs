use nexus::compiler::passes::hir_build::build_hir;
use nexus::compiler::passes::lir_lower::lower_mir_to_lir;
use nexus::lang::parser;

fn build_lir(src: &str) -> nexus::ir::lir::LirProgram {
    crate::harness::ensure_repo_root();
    let program = parser::parser().parse(src).unwrap();
    let mir = build_hir(&program).unwrap();
    lower_mir_to_lir(&mir, &mir.enum_defs).unwrap()
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
    let body_str = format!("{:?}", main_fn.body);
    assert!(
        !body_str.contains("MY_CONST"),
        "MY_CONST should be inlined, not referenced as constructor"
    );
}

#[test]
fn snapshot_lir_record_field_access() {
    let src = r#"
    let main = fn () -> unit do
        let r = { x: 1, y: 2 }
        let v = r.x
        return ()
    end
    "#;
    let lir = build_lir(src);
    insta::assert_debug_snapshot!(lir);
}

#[test]
fn snapshot_lir_match_expression() {
    let src = r#"
    let f = fn (x: i64) -> i64 do
        let result = match x do
          case 0 -> 10
          case _ -> 20
        end
        return result
    end
    let main = fn () -> unit do
        let _ = f(x: 1)
        return ()
    end
    "#;
    let lir = build_lir(src);
    insta::assert_debug_snapshot!(lir);
}

#[test]
fn snapshot_lir_while_loop() {
    let src = r#"
    let main = fn () -> unit do
        let ~i = 0
        while ~i < 5 do
            ~i <- ~i + 1
        end
        return ()
    end
    "#;
    let lir = build_lir(src);
    insta::assert_debug_snapshot!(lir);
}

#[test]
fn snapshot_lir_tail_call() {
    let src = r#"
    let loop_n = fn (n: i64) -> i64 do
        if n <= 0 then return 0 end
        return loop_n(n: n - 1)
    end
    let main = fn () -> unit do
        let _ = loop_n(n: 10)
        return ()
    end
    "#;
    let lir = build_lir(src);
    insta::assert_debug_snapshot!(lir);
}

#[test]
fn mutual_recursion_produces_tail_call_in_lir() {
    let src = r#"
    let is_even = fn (n: i64) -> bool do
        if n == 0 then return true end
        return is_odd(n: n - 1)
    end
    let is_odd = fn (n: i64) -> bool do
        if n == 0 then return false end
        return is_even(n: n - 1)
    end
    let main = fn () -> unit do
        let _ = is_even(n: 10)
        return ()
    end
    "#;
    let lir = build_lir(src);
    let is_even_fn = lir.functions.iter().find(|f| f.name.as_str().ends_with("#is_even")).unwrap();
    let is_odd_fn = lir.functions.iter().find(|f| f.name.as_str().ends_with("#is_odd")).unwrap();
    let even_body = format!("{:?}", is_even_fn.body);
    let odd_body = format!("{:?}", is_odd_fn.body);
    assert!(
        even_body.contains("TailCall"),
        "is_even's call to is_odd should be TailCall, got: {even_body}"
    );
    assert!(
        odd_body.contains("TailCall"),
        "is_odd's call to is_even should be TailCall, got: {odd_body}"
    );
}

#[test]
fn tail_call_in_if_else_branches() {
    let src = r#"
    let count = fn (n: i64) -> i64 do
        if n <= 0 then
            return 0
        else
            return count(n: n - 1)
        end
    end
    let main = fn () -> unit do
        let _ = count(n: 10)
        return ()
    end
    "#;
    let lir = build_lir(src);
    let count_fn = lir.functions.iter().find(|f| f.name.as_str().ends_with("#count")).unwrap();
    let body_str = format!("{:?}", count_fn.body);
    assert!(
        body_str.contains("TailCall"),
        "if-else branch return should produce TailCall, not Call"
    );
}
