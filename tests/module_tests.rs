use chumsky::Parser;
use nexus::interpreter::Interpreter;
use nexus::lang::stdlib::list_stdlib_nx_paths;
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
    import from tests/fixtures/math.nx
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
    import { add } from tests/fixtures/math.nx
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
    import external math.wasm
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
    import as list from nxlib/stdlib/list.nx

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
    import as array from nxlib/stdlib/array.nx

    fn main() -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = borrow %arr
      let n = array.length(arr: arr_ref)
      drop %arr
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

#[test]
fn test_stdlib_public_names_are_not_native_functions_and_drop_is_statement() {
    let parser = parser::parser();
    let program = parser.parse("").unwrap();
    let interpreter = Interpreter::new(program);

    assert!(!interpreter.native_functions.contains_key("i64_to_string"));
    assert!(!interpreter.native_functions.contains_key("float_to_string"));
    assert!(!interpreter.native_functions.contains_key("bool_to_string"));
    assert!(!interpreter.native_functions.contains_key("drop"));
    assert!(!interpreter.native_functions.contains_key("drop_i64"));
    assert!(!interpreter.native_functions.contains_key("drop_array"));
    assert!(!interpreter.native_functions.contains_key("list_length"));
    assert!(!interpreter.native_functions.contains_key("array_length"));

    assert!(!interpreter
        .native_functions
        .contains_key("__nx_i64_to_string"));
    assert!(!interpreter
        .native_functions
        .contains_key("__nx_float_to_string"));
    assert!(!interpreter
        .native_functions
        .contains_key("__nx_bool_to_string"));
    assert!(!interpreter.native_functions.contains_key("__nx_drop_i64"));
    assert!(interpreter
        .native_functions
        .contains_key("__nx_list_length"));
    assert!(!interpreter.native_functions.contains_key("__nx_drop_array"));
    assert!(!interpreter
        .native_functions
        .contains_key("__nx_array_length"));
    assert!(!interpreter
        .external_functions
        .contains_key("__nx_drop_array"));
    assert!(interpreter
        .external_functions
        .contains_key("__nx_array_length"));
    assert!(interpreter
        .external_functions
        .contains_key("__nx_i64_to_string"));
    assert!(interpreter
        .external_functions
        .contains_key("__nx_float_to_string"));
    assert!(interpreter
        .external_functions
        .contains_key("__nx_bool_to_string"));
    assert!(!interpreter.external_functions.contains_key("__nx_drop_i64"));
}

#[test]
fn test_stdio_defines_print_only() {
    let src = fs::read_to_string("nxlib/stdlib/stdio.nx").unwrap();
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();

    let defined_names: Vec<String> = program
        .definitions
        .iter()
        .filter_map(|d| match &d.node {
            nexus::ast::TopLevel::Function(f) => Some(f.name.clone()),
            nexus::ast::TopLevel::ExternalFn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(defined_names, vec!["print".to_string()]);
}

#[test]
fn test_typechecker_does_not_register_drop_function() {
    let checker = TypeChecker::new();
    assert!(checker.env.get("drop").is_none());
}

#[test]
fn test_stdlib_loader_uses_nx_only() {
    let paths = list_stdlib_nx_paths().expect("failed to list stdlib paths");
    assert!(!paths.is_empty(), "stdlib .nx files should exist");
    for p in paths {
        assert_eq!(p.extension().and_then(|s| s.to_str()), Some("nx"));
    }
}

#[test]
fn test_stdlib_global_array_length() {
    let src = r#"
    fn main() -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = borrow %arr
      let n = array_length(arr: arr_ref)
      drop %arr
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
fn test_exception_constructor_raise_and_catch() {
    let src = r#"
    exception Boom(i64)

    fn main() -> i64 do
      try
        let err = Boom(42)
        raise err
      catch e ->
        match e do
          case Boom(code) -> return code
          case RuntimeError(_) -> return -1
          case InvalidIndex(_) -> return -2
        endmatch
      endtry
      return 0
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
    let mut interpreter = Interpreter::new(program);
    let res = interpreter.run_function("main", vec![]).unwrap();
    match res {
        nexus::interpreter::Value::Int(42) => (),
        _ => panic!("Expected 42, got {:?}", res),
    }
}

#[test]
fn test_try_catch_can_catch_runtime_error_as_exception() {
    let src = r#"
    fn main() -> unit do
      try
        let _ = [| 1 |][10]
      catch e ->
        match e do
          case RuntimeError(_) -> return ()
          case InvalidIndex(_) -> return ()
        endmatch
      endtry
      return ()
    endfn
    "#;
    let parser = parser::parser();
    let program = parser.parse(src).unwrap();
    let mut checker = TypeChecker::new();
    checker.check_program(&program).unwrap();
    let mut interpreter = Interpreter::new(program);
    let res = interpreter.run_function("main", vec![]);
    assert!(
        res.is_ok(),
        "runtime error should be reified as Exn and caught"
    );
}
