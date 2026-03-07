use crate::common::source::{check_and_run, check_raw, prepare_test_source, run};
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser;
use nexus::lang::stdlib::list_stdlib_nx_paths;
use nexus::lang::typecheck::TypeChecker;
use std::fs;

#[test]
fn test_module_import() {
    let res = check_and_run("examples/module_test.nx");
    assert!(res.is_ok(), "Module test failed: {:?}", res.err());
}

#[test]
fn test_module_default_alias() {
    let src = r#"
    import from examples/math.nx
    let main = fn () -> i64 do
      return math.add(a: 5, b: 5)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_module_selective_import() {
    let src = r#"
    import { add } from examples/math.nx
    let main = fn () -> i64 do
      return add(a: 1, b: 2)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_import_external_syntax() {
    let src = r#"
    import external math.wasm
    pub external add = "add" : (a: i64, b: i64) -> i64
    let main = fn () -> unit do return () end
    "#;
    check_raw(src).unwrap();
}

#[test]
fn test_pub_import_syntax_is_rejected() {
    let src = r#"
    pub import from examples/math.nx
    let main = fn () -> i64 do
      return 0
    end
    "#;
    let parser = parser::parser();
    assert!(parser.parse(&src).is_err());
}

#[test]
fn test_stdlib_list_module_length() {
    let src = r#"
    import as list from stdlib/list.nx

    let main = fn () -> i64 do
      let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Cons(v: 4, rest: Nil()))))
      return list.length(xs: xs)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(4));
}

#[test]
fn test_stdlib_array_module_length() {
    let src = r#"
    import as array from stdlib/array.nx

    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = &%arr
      let n = array.length(arr: arr_ref)
      match %arr do case _ -> () end
      return n
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_stdlib_result_module_helpers() {
    let src = &crate::common::fixtures::read_test_fixture("test_stdlib_result_module_helpers.nx");
    assert_eq!(run(src).unwrap(), Value::Int(52));
}

#[test]
fn test_stdlib_module_not_auto_exported() {
    let src = r#"
    let main = fn () -> i64 do
      let xs = [1, 2, 3]
      return list.length(xs: xs)
    end
    "#;
    let src = prepare_test_source(src);
    let parser = parser::parser();
    let program = parser.parse(&src).unwrap();
    let mut checker = TypeChecker::new();
    assert!(checker.check_program(&program).is_err());
}

#[test]
fn test_stdlib_public_names_are_not_native_functions_and_drop_is_statement() {
    let parser = parser::parser();
    let program = parser.parse("").unwrap();
    let interpreter = Interpreter::new(program);

    // from_i64 has a native fallback for interpreter-only mode (no Wasm FFI).
    assert!(interpreter.native_functions.contains_key("from_i64"));
    assert!(!interpreter.native_functions.contains_key("from_float"));
    assert!(!interpreter.native_functions.contains_key("from_bool"));
    assert!(!interpreter.native_functions.contains_key("drop"));
    assert!(!interpreter.native_functions.contains_key("drop_i64"));
    assert!(!interpreter.native_functions.contains_key("drop_array"));
    assert!(!interpreter.native_functions.contains_key("list_length"));
    assert!(!interpreter.native_functions.contains_key("array_length"));

    assert!(!interpreter.native_functions.contains_key("__nx_drop_i64"));
    assert!(!interpreter
        .native_functions
        .contains_key("__nx_list_length"));
    assert!(!interpreter
        .native_functions
        .contains_key("__nx_string_length"));
    assert!(!interpreter.native_functions.contains_key("__nx_abs_i64"));
    assert!(!interpreter.native_functions.contains_key("__nx_drop_array"));
    assert!(!interpreter
        .native_functions
        .contains_key("__nx_array_length"));
    assert!(!interpreter
        .external_functions
        .contains_key("__nx_drop_array"));
    assert!(interpreter.external_functions.contains_key("length"));
    assert!(interpreter.external_functions.contains_key("abs"));
    assert!(interpreter.external_functions.contains_key("length"));
    assert!(interpreter.external_functions.contains_key("from_i64"));
    assert!(interpreter.external_functions.contains_key("from_float"));
    assert!(interpreter.external_functions.contains_key("from_bool"));
    assert!(!interpreter.external_functions.contains_key("__nx_drop_i64"));
}

#[test]
fn test_stdio_defines_console_port_and_system_handler() {
    let src = fs::read_to_string("nxlib/stdlib/stdio.nx").unwrap();
    let parser = parser::parser();
    let program = parser.parse(&src).unwrap();

    let has_console_port = program
        .definitions
        .iter()
        .any(|d| matches!(&d.node, nexus::lang::ast::TopLevel::Port(p) if p.name == "Console"));
    assert!(
        has_console_port,
        "Console port should be defined in stdio.nx"
    );

    let let_names: Vec<String> = program
        .definitions
        .iter()
        .filter_map(|d| match &d.node {
            nexus::lang::ast::TopLevel::Let(gl) => Some(gl.name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        let_names.contains(&"system_handler".to_string()),
        "system_handler should be defined in stdio.nx"
    );
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
    import { length } from stdlib/array.nx
    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = &%arr
      let n = length(arr: arr_ref)
      match %arr do case _ -> () end
      return n
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_exception_constructor_raise_and_catch() {
    let src = &crate::common::fixtures::read_test_fixture(
        "test_exception_constructor_raise_and_catch.nx",
    );
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_exception_constructor_with_labels_raise_and_catch() {
    let src = &crate::common::fixtures::read_test_fixture(
        "test_exception_constructor_with_labels_raise_and_catch.nx",
    );
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_try_catch_can_catch_runtime_error_as_exception() {
    let src = &crate::common::fixtures::read_test_fixture(
        "test_try_catch_can_catch_runtime_error_as_exception.nx",
    );
    let res = run(src);
    assert!(
        res.is_ok(),
        "runtime error should be reified as Exn and caught"
    );
}

#[test]
fn all_examples_parse() {
    for entry in fs::read_dir("examples").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(false, |e| e == "nx") {
            let src = fs::read_to_string(&path).unwrap();
            parser::parser()
                .parse(&src)
                .unwrap_or_else(|e| panic!("{}: parse error: {:?}", path.display(), e));
        }
    }
}

#[test]
fn all_examples_typecheck() {
    for entry in fs::read_dir("examples").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(false, |e| e == "nx") {
            let src = fs::read_to_string(&path).unwrap();
            let program = parser::parser()
                .parse(&src)
                .unwrap_or_else(|e| panic!("{}: parse error: {:?}", path.display(), e));
            let mut checker = TypeChecker::new();
            checker
                .check_program(&program)
                .unwrap_or_else(|e| panic!("{}: typecheck error: {}", path.display(), e.message));
        }
    }
}

// ── String operations ────────────────────────────────────

#[test]
fn test_stdlib_string_length() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> i64 do
      return string.length(s: "hello")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_stdlib_string_contains() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> bool do
      return string.contains(s: "hello world", sub: "world")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_stdlib_string_contains_false() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> bool do
      return string.contains(s: "hello", sub: "xyz")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_stdlib_string_substring() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> string do
      return string.substring(s: "hello world", start: 6, len: 5)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::String("world".to_string()));
}

#[test]
fn test_stdlib_string_index_of() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> i64 do
      return string.index_of(s: "abcdef", sub: "cd")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn test_stdlib_string_index_of_not_found() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> i64 do
      return string.index_of(s: "abc", sub: "xyz")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(-1));
}

#[test]
fn test_stdlib_string_starts_with() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> bool do
      return string.starts_with(s: "hello", prefix: "hel")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_stdlib_string_ends_with() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> bool do
      return string.ends_with(s: "hello", suffix: "llo")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_stdlib_string_trim() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> string do
      return string.trim(s: "  hi  ")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::String("hi".to_string()));
}

#[test]
fn test_stdlib_string_to_upper() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> string do
      return string.to_upper(s: "hello")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::String("HELLO".to_string()));
}

#[test]
fn test_stdlib_string_to_lower() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> string do
      return string.to_lower(s: "HELLO")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::String("hello".to_string()));
}

#[test]
fn test_stdlib_string_replace() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> string do
      return string.replace(s: "hello world", from_str: "world", to_str: "nexus")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::String("hello nexus".to_string()));
}

#[test]
fn test_stdlib_string_split() {
    let src = r#"
    import as string from stdlib/string.nx
    import as list from stdlib/list.nx
    let main = fn () -> i64 do
      let parts = string.split(s: "a,b,c", sep: ",")
      return list.length(xs: parts)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_stdlib_string_char_at() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> string do
      return string.char_at(s: "hello", idx: 1)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::String("e".to_string()));
}

// ── Math operations ──────────────────────────────────────

#[test]
fn test_stdlib_abs() {
    let src = r#"
    import { abs } from stdlib/math.nx
    let main = fn () -> i64 do
      let x = 0 - 42
      return abs(val: x)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_stdlib_max() {
    let src = r#"
    import { max } from stdlib/math.nx
    let main = fn () -> i64 do
      return max(a: 10, b: 20)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(20));
}

#[test]
fn test_stdlib_min() {
    let src = r#"
    import { min } from stdlib/math.nx
    let main = fn () -> i64 do
      return min(a: 10, b: 20)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_stdlib_mod_i64() {
    let src = r#"
    import { mod_i64 } from stdlib/math.nx
    let main = fn () -> i64 do
      return mod_i64(a: 10, b: 3)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_stdlib_sqrt() {
    let src = r#"
    import { sqrt } from stdlib/math.nx
    let main = fn () -> float do
      return sqrt(val: 9.0)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Float(3.0));
}

#[test]
fn test_stdlib_floor() {
    let src = r#"
    import { floor } from stdlib/math.nx
    let main = fn () -> float do
      return floor(val: 3.7)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Float(3.0));
}

#[test]
fn test_stdlib_ceil() {
    let src = r#"
    import { ceil } from stdlib/math.nx
    let main = fn () -> float do
      return ceil(val: 3.2)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Float(4.0));
}

#[test]
fn test_stdlib_pow() {
    let src = r#"
    import { pow } from stdlib/math.nx
    let main = fn () -> float do
      return pow(base: 2.0, exp: 10.0)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Float(1024.0));
}

#[test]
fn test_stdlib_abs_float() {
    let src = r#"
    import { abs_float } from stdlib/math.nx
    let main = fn () -> float do
      let x = 0.0 -. 5.5
      return abs_float(val: x)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Float(5.5));
}

// ── Conversion operations ────────────────────────────────

#[test]
fn test_stdlib_i64_to_float() {
    let src = r#"
    import { i64_to_float } from stdlib/math.nx
    let main = fn () -> float do
      return i64_to_float(val: 42)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Float(42.0));
}

#[test]
fn test_stdlib_float_to_i64() {
    let src = r#"
    import { float_to_i64 } from stdlib/math.nx
    let main = fn () -> i64 do
      return float_to_i64(val: 3.9)
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_stdlib_short_import_path() {
    // Users can write `stdlib/X` instead of `nxlib/stdlib/X`
    let src = r#"
    import { length } from stdlib/string.nx
    let main = fn () -> i64 do
      return length(s: "hello")
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_stdlib_string_parse_i64() {
    let src = r#"
    import as string from stdlib/string.nx
    let main = fn () -> i64 do
      match string.parse_i64(s: "123") do
        case Some(val: v) -> return v
        case None() -> return 0 - 1
      end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(123));
}

// ── stdio ────────────────────────────────────────────────

#[test]
fn test_stdio_system_handler_defined() {
    let src = fs::read_to_string("nxlib/stdlib/stdio.nx").unwrap();
    let p = parser::parser();
    let program = p.parse(&src).unwrap();

    let let_names: Vec<String> = program
        .definitions
        .iter()
        .filter_map(|d| match &d.node {
            nexus::lang::ast::TopLevel::Let(gl) => Some(gl.name.clone()),
            _ => None,
        })
        .collect();

    assert!(
        let_names.contains(&"system_handler".to_string()),
        "system_handler should be defined in stdio.nx"
    );
}

// ── Result utilities ─────────────────────────────────────

#[test]
fn test_result_map_ok() {
    let src = &crate::common::fixtures::read_test_fixture("test_result_map_ok.nx");
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_result_map_err_case() {
    let src = &crate::common::fixtures::read_test_fixture("test_result_map_err_case.nx");
    assert_eq!(run(src).unwrap(), Value::Int(99));
}

#[test]
fn test_result_and_then_ok() {
    let src = &crate::common::fixtures::read_test_fixture("test_result_and_then_ok.nx");
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_result_map_err_fn() {
    let src = &crate::common::fixtures::read_test_fixture("test_result_map_err_fn.nx");
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
