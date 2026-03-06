use crate::common::source::{check, run};
use nexus::interpreter::Value;
use nexus::lang::ast::{Expr, Type};
use nexus::lang::parser;

#[test]
fn test_list_type_is_builtin() {
    // Parser should produce Type::List, not Type::UserDefined("List", ...)
    let src = r#"
    let main = fn () -> unit do
        let xs: [i64] = [1, 2, 3]
        return ()
    end
    "#;
    let program = parser::parser().parse(src).unwrap();
    // Find the let binding and check that its type annotation is Type::List
    let found_list_type = program.definitions.iter().any(|def| {
        if let nexus::lang::ast::TopLevel::Let(gl) = &def.node {
            // Check the function body for let xs: [i64]
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
    // Parser should produce Expr::List, not nested Expr::Constructor("Cons"/"Nil")
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
    // List constructors and pattern matching should work without importing stdlib
    let src = r#"
    let main = fn () -> i64 do
        let xs = [10, 20, 30]
        match xs do
            case Nil() -> return 0
            case Cons(v: h, rest: _) -> return h
        end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_list_constructor_returns_list_type() {
    // Cons/Nil constructors should be assignable to [T] typed bindings
    let src = r#"
    let main = fn () -> i64 do
        let xs: [i64] = Cons(v: 1, rest: Nil())
        match xs do
            case Cons(v: h, rest: _) -> return h
            case Nil() -> return 0
        end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_stdlib_functions_with_builtin_type() {
    // Stdlib functions should work with built-in [T] after removing pub type List<T> from stdlib
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
        let xs = [1, 2, 3, 4, 5]
        let len = list.length(xs: xs)
        let rev = list.reverse(xs: xs)
        let h = list.head(xs: rev)
        return len * 10 + h
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(55));
}

#[test]
fn test_list_type_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let l = [1, true]
        return ()
    end
    "#;
    assert!(check(src).is_err(), "Should fail: mixed types in list");
}

#[test]
fn test_list_literal_and_head_tail() {
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let xs = [1, 2, 3]
      let h = list.head(xs: xs)
      let t = list.tail(xs: xs)
      let h2 = list.head(xs: t)
      return h * 10 + h2
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(12));
}

#[test]
fn test_list_type_annotation_sugar() {
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let xs: [i64] = [1, 2, 3]
      return list.nth(xs: xs, n: 2)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_list_is_empty() {
    let src = &crate::common::fixtures::read_test_fixture("test_list_is_empty.nx");
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_reverse_concat_length() {
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let a = Cons(v: 1, rest: Cons(v: 2, rest: Nil()))
      let b = Cons(v: 3, rest: Cons(v: 4, rest: Nil()))
      let c = list.concat(xs: a, ys: b)
      let r = list.reverse(xs: c)
      let h = list.head(xs: r)
      let len = list.length(xs: c)
      return h * 10 + len
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(44));
}

#[test]
fn test_list_cons_last() {
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 2, rest: Cons(v: 3, rest: Nil()))
      let ys = list.cons(x: 1, xs: xs)
      let h = list.head(xs: ys)
      let l = list.last(xs: ys)
      return h * 10 + l
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(13));
}

#[test]
fn test_list_take_and_drop() {
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Cons(v: 5, rest: Nil())))))
      let t = list.take(xs: xs, n: 3)
      let d = list.drop_n(xs: xs, n: 2)
      let th = list.head(xs: d)
      let tl = list.length(xs: t)
      return th * 10 + tl
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(33));
}

#[test]
fn test_list_nth() {
    let src = r#"
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 10, rest: Cons(v: 20, rest: Cons(v: 30, rest: Cons(v: 40, rest: Nil()))))
      return list.nth(xs: xs, n: 2)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(30));
}

#[test]
fn test_list_contains() {
    let src = &crate::common::fixtures::read_test_fixture("test_list_contains.nx");
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_fold_left_sum() {
    let src = r#"
    import as list from stdlib/list.nx

    let add = fn (acc: i64, val: i64) -> i64 do
      return acc + val
    end

    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Nil()))))
      return list.fold_left(xs: xs, init: 0, f: add)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_list_map_and_map_rev() {
    let src = &crate::common::fixtures::read_test_fixture("test_list_map_and_map_rev.nx");
    assert_eq!(run(src).unwrap(), Value::Int(46));
}
