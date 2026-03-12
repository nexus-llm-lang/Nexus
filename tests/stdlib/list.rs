use crate::harness::{exec, should_fail_typecheck};
use nexus::lang::ast::{Expr, Type};
use nexus::lang::parser;

#[test]
fn test_list_type_is_builtin() {
    let src = r#"
    let main = fn () -> unit do
        let xs: [i64] = [1, 2, 3]
        return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let found_list_type = program.definitions.iter().any(|def| {
        if let nexus::lang::ast::TopLevel::Let(gl) = &def.node {
            if let Expr::Lambda { body, .. } = &gl.value.node {
                body.iter().any(|stmt| {
                    if let nexus::lang::ast::Stmt::Let { typ: Some(t), .. } = &stmt.node {
                        matches!(t, Type::List(_))
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        } else {
            false
        }
    });
    assert!(
        found_list_type,
        "Parser should produce Type::List for [i64] syntax"
    );
}

#[test]
fn test_list_expr_is_builtin() {
    let src = r#"
    let main = fn () -> unit do
        let xs = [1, 2, 3]
        return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    let found_list_expr = program.definitions.iter().any(|def| {
        if let nexus::lang::ast::TopLevel::Let(gl) = &def.node {
            if let Expr::Lambda { body, .. } = &gl.value.node {
                body.iter().any(|stmt| {
                    if let nexus::lang::ast::Stmt::Let { value, .. } = &stmt.node {
                        matches!(value.node, Expr::List(_))
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        } else {
            false
        }
    });
    assert!(
        found_list_expr,
        "Parser should produce Expr::List for [1,2,3] syntax"
    );
}

#[test]
fn test_list_builtin_no_import() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = [10, 20, 30]
        match xs do
            case Nil() -> raise RuntimeError(val: "expected Cons")
            case Cons(v: h, rest: _) ->
                if h != 10 then raise RuntimeError(val: "expected 10") end
                return ()
        end
    end
    "#,
    );
}

#[test]
fn test_list_constructor_returns_list_type() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs: [i64] = Cons(v: 1, rest: Nil())
        match xs do
            case Cons(v: h, rest: _) ->
                if h != 1 then raise RuntimeError(val: "expected 1") end
                return ()
            case Nil() -> raise RuntimeError(val: "expected Cons")
        end
    end
    "#,
    );
}

#[test]
fn test_list_type_mismatch() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let l = [1, true]
        return ()
    end
    "#,
    );
    assert!(!err.is_empty(), "Should fail: mixed types in list");
}

#[test]
fn partition_type_in_list_module() {
    exec(
        r#"
import { Partition } from stdlib/list.nx

let main = fn () -> unit do
  let p = Partition(matched: Cons(v: 1, rest: Nil()), rest: Nil())
  match p do
    case Partition(matched: m, rest: _) ->
      match m do
        case Cons(v: v, rest: _) ->
            if v != 1 then raise RuntimeError(val: "expected 1") end
            return ()
        case Nil() -> raise RuntimeError(val: "expected Cons")
      end
  end
end
"#,
    );
}
