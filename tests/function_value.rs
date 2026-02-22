use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;

fn run_src(src: &str) -> Result<Value, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    let mut checker = TypeChecker::new();
    checker
        .check_program(&program)
        .map_err(|e| format!("type error: {}", e.message))?;
    let mut interpreter = Interpreter::new(program);
    interpreter.run_function("__test_main", vec![])
}

#[test]
fn test_named_function_value_runtime() {
    let src = r#"
    let id = fn (x: i64) -> i64 do
        return x
    endfn

    let __test_main = fn () -> i64 do
        let f = id
        return f(x: 7)
    endfn
    "#;
    let result = run_src(src).unwrap();
    match result {
        Value::Int(n) => assert_eq!(n, 7),
        other => panic!("Expected int, got {:?}", other),
    }
}

#[test]
fn test_inline_lambda_literal_runtime() {
    let src = r#"
    let __test_main = fn () -> i64 do
        let inc = fn (x: i64) -> i64 do
            return x + 1
        endfn
        return inc(x: 9)
    endfn
    "#;
    let result = run_src(src).unwrap();
    match result {
        Value::Int(n) => assert_eq!(n, 10),
        other => panic!("Expected int, got {:?}", other),
    }
}

#[test]
fn test_lambda_captures_outer_variable() {
    let src = r#"
    let __test_main = fn () -> i64 do
        let base = 10
        let add_base = fn (x: i64) -> i64 do
            return x + base
        endfn
        return add_base(x: 3)
    endfn
    "#;
    let result = run_src(src).unwrap();
    match result {
        Value::Int(n) => assert_eq!(n, 13),
        other => panic!("Expected int, got {:?}", other),
    }
}

// test_direct_call_labeled_args_out_of_order_runtime: covered by prop_n_ary_labeled_call_order_invariant_runtime
// test_function_value_call_labeled_args_out_of_order_runtime: covered by prop_n_ary_labeled_call_order_invariant_via_function_value_runtime

#[test]
fn test_recursive_lambda_with_annotation_runtime() {
    let src = r#"
    let __test_main = fn () -> i64 do
        let fact: (n: i64) -> i64 = fn (n: i64) -> i64 do
            if n == 0 then
                return 1
            else
                let n1 = n - 1
                let rec = fact(n: n1)
                return n * rec
            endif
        endfn
        return fact(n: 5)
    endfn
    "#;
    let result = run_src(src).unwrap();
    match result {
        Value::Int(n) => assert_eq!(n, 120),
        other => panic!("Expected int, got {:?}", other),
    }
}
