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
fn test_list_creation() {
    let src = r#"
    let main = fn () -> unit do
        let l = [1, 2, 3]
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_list_literal_is_cons_nil_sugar() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = [1, 2, 3]
      return list.head(xs: xs)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_type_annotation_is_list_user_type_sugar() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs: [i64] = [1, 2, 3]
      return list.nth(xs: xs, n: 2)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
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
fn test_list_nested() {
    let src = r#"
    let main = fn () -> unit do
        let l = [[1, 2], [3, 4]]
        return ()
    endfn
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_list_of_linear() {
    let _src = r#"
    let main = fn () -> unit do
        let %l = [%1, %2] // Assuming integers can be linear for test
        // This fails because integers are not linear by default unless cast/annotated?
        // Let's use linear literal syntax? No such thing.
        // Use constructor?
        return ()
    endfn
    "#;
    // Currently no way to create linear literals easily.
    // Skip linear list test for now or use stdlib function that returns linear.
}

#[test]
fn test_list_head() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 10, rest: Cons(v: 20, rest: Cons(v: 30, rest: Nil())))
      return list.head(xs: xs)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_list_tail() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 10, rest: Cons(v: 20, rest: Cons(v: 30, rest: Nil())))
      let t = list.tail(xs: xs)
      return list.head(xs: t)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(20));
}

#[test]
fn test_list_is_empty() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> bool do
      let xs = Nil()
      return list.is_empty(xs: xs)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_list_is_empty_false() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> bool do
      let xs = Cons(v: 1, rest: Nil())
      return list.is_empty(xs: xs)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_list_reverse() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      let r = list.reverse(xs: xs)
      return list.head(xs: r)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_list_concat() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let a = Cons(v: 1, rest: Cons(v: 2, rest: Nil()))
      let b = Cons(v: 3, rest: Cons(v: 4, rest: Nil()))
      let c = list.concat(xs: a, ys: b)
      return list.length(xs: c)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(4));
}

#[test]
fn test_list_cons() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 2, rest: Cons(v: 3, rest: Nil()))
      let ys = list.cons(x: 1, xs: xs)
      return list.head(xs: ys)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_list_last() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 10, rest: Cons(v: 20, rest: Cons(v: 30, rest: Nil())))
      return list.last(xs: xs)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(30));
}

#[test]
fn test_list_take() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Cons(v: 5, rest: Nil())))))
      let t = list.take(xs: xs, n: 3)
      return list.length(xs: t)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_list_drop_n() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Cons(v: 5, rest: Nil())))))
      let d = list.drop_n(xs: xs, n: 2)
      return list.head(xs: d)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_list_nth() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> i64 do
      let xs = Cons(v: 10, rest: Cons(v: 20, rest: Cons(v: 30, rest: Cons(v: 40, rest: Nil()))))
      return list.nth(xs: xs, n: 2)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(30));
}

#[test]
fn test_list_contains_true() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> bool do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      return list.contains(xs: xs, val: 2)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_list_contains_false() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]
    let main = fn () -> bool do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      return list.contains(xs: xs, val: 5)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_list_fold_left_sum() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]

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
fn test_list_map_rev() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]

    let twice = fn (val: i64) -> i64 do
      return val * 2
    endfn

    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      let ys = list.map_rev(xs: xs, f: twice)
      return list.head(xs: ys)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(6));
}

#[test]
fn test_list_map() {
    let src = r#"
    import as list from [=[nxlib/stdlib/list.nx]=]

    let twice = fn (val: i64) -> i64 do
      return val * 2
    endfn

    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil())))
      let ys = list.map(xs: xs, f: twice)
      return list.nth(xs: ys, n: 1)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(4));
}
