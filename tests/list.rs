use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn test_list_type_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let l = [1, true]
        return ()
    endfn
    "#;
    assert!(check(src).is_err(), "Should fail: mixed types in list");
}

#[test]
fn test_list_literal_and_head_tail() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let xs = [1, 2, 3]
      let h = list.head(xs: xs)
      let t = list.tail(xs: xs)
      let h2 = list.head(xs: t)
      return h * 10 + h2
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(12));
}

#[test]
fn test_list_type_annotation_sugar() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let xs: [i64] = [1, 2, 3]
      return list.nth(xs: xs, n: 2)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_list_is_empty() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let empty = Nil()
      let nonempty = Cons(v: 1, rest: Nil())
      let a = list.is_empty(xs: empty)
      let b = list.is_empty(xs: nonempty)
      if a then
        if b then return 0 else return 1 endif
      else
        return 0
      endif
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_reverse_concat_length() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let a = Cons(v: 1, rest: Cons(v: 2, rest: Nil()))
      let b = Cons(v: 3, rest: Cons(v: 4, rest: Nil()))
      let c = list.concat(xs: a, ys: b)
      let r = list.reverse(xs: c)
      let h = list.head(xs: r)
      let len = list.length(xs: c)
      return h * 10 + len
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(44));
}

#[test]
fn test_list_cons_last() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 2, rest: Cons(v: 3, rest: Nil()))
      let ys = list.cons(x: 1, xs: xs)
      let h = list.head(xs: ys)
      let l = list.last(xs: ys)
      return h * 10 + l
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(13));
}

#[test]
fn test_list_take_and_drop() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Cons(v: 5, rest: Nil())))))
      let t = list.take(xs: xs, n: 3)
      let d = list.drop_n(xs: xs, n: 2)
      let th = list.head(xs: d)
      let tl = list.length(xs: t)
      return th * 10 + tl
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(33));
}

#[test]
fn test_list_nth() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 10, rest: Cons(v: 20, rest: Cons(v: 30, rest: Cons(v: 40, rest: Nil()))))
      return list.nth(xs: xs, n: 2)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(30));
}

#[test]
fn test_list_contains() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx
    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      let a = list.contains(xs: xs, val: 2)
      let b = list.contains(xs: xs, val: 5)
      if a then
        if b then return 0 else return 1 endif
      else
        return 0
      endif
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_fold_left_sum() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx

    let add = fn (acc: i64, val: i64) -> i64 do
      return acc + val
    endfn

    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Nil()))))
      return list.fold_left(xs: xs, init: 0, f: add)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_list_map_and_map_rev() {
    let src = r#"
    import as list from nxlib/stdlib/list.nx

    let twice = fn (val: i64) -> i64 do
      return val * 2
    endfn

    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      let mapped = list.map(xs: xs, f: twice)
      let rev_mapped = list.map_rev(xs: xs, f: twice)
      let a = list.nth(xs: mapped, n: 1)
      let b = list.head(xs: rev_mapped)
      return a * 10 + b
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(46));
}
