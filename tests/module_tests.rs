use chumsky::Parser;
use nexus::interpreter::Interpreter;
use nexus::parser;
use nexus::typecheck::TypeChecker;
use std::fs;

fn check_and_run(src_path: &str) -> Result<(), String> {
    let src = fs::read_to_string(src_path).map_err(|e| e.to_string())?;
    let parser = parser::parser();
    let program = parser.parse(src).map_err(|e| format!("{:?}", e))?;

    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)?;

    let mut interpreter = Interpreter::new(program);
    interpreter
        .run_function("main", vec![])
        .map(|_| ())
        .map_err(|e| e)
}

#[test]
fn test_module_import() {
    let res = check_and_run("tests/fixtures/module_test.nx");
    assert!(res.is_ok(), "Module test failed: {:?}", res.err());
}

#[test]
fn test_module_default_alias() {
    let src = r#"
    import from "tests/fixtures/math.nx"
    fn main() -> i64 do
      return math.add(a: 5, b: 5)
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
    let mut interpreter = Interpreter::new(program);
    let res = interpreter.run_function("main", vec![]).unwrap();
    match res {
        nexus::interpreter::Value::Int(10) => (),
        _ => panic!("Expected 10, got {:?}", res),
    }
}

#[test]
fn test_module_selective_import() {
    let src = r#"
    import { add } from "tests/fixtures/math.nx"
    fn main() -> i64 do
      return add(a: 1, b: 2)
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
    let mut interpreter = Interpreter::new(program);
    let res = interpreter.run_function("main", vec![]).unwrap();
    match res {
        nexus::interpreter::Value::Int(3) => (),
        _ => panic!("Expected 3, got {:?}", res),
    }
}

#[test]
fn test_import_external_syntax() {
    let src = r#"
    import external "math.wasm"
    pub external add : (a: i64, b: i64) -> i64 = "add"
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
}

#[test]
fn test_stdlib_list_module_length() {
    let src = r#"
    import as list from "nxlib/stdlib/list.nx"

    fn main() -> i64 do
      let xs = [1, 2, 3, 4]
      return list.length(xs: xs)
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
    let mut interpreter = Interpreter::new(program);
    let res = interpreter.run_function("main", vec![]).unwrap();
    match res {
        nexus::interpreter::Value::Int(4) => (),
        _ => panic!("Expected 4, got {:?}", res),
    }
}

#[test]
fn test_stdlib_array_module_length() {
    let src = r#"
    import as array from "nxlib/stdlib/array.nx"

    fn main() -> i64 do
      let %arr = [| 10, 20, 30 |]
      let n = array.length(arr: borrow %arr)
      drop_array(arr: %arr)
      return n
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
    let mut interpreter = Interpreter::new(program);
    let res = interpreter.run_function("main", vec![]).unwrap();
    match res {
        nexus::interpreter::Value::Int(3) => (),
        _ => panic!("Expected 3, got {:?}", res),
    }
}

#[test]
fn test_stdlib_module_not_auto_exported() {
    let src = r#"
    fn main() -> i64 do
      let xs = [1, 2, 3]
      return list.length(xs: xs)
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_err());
}
