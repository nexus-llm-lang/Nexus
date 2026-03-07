use crate::common::source::run;
use crate::common::wasm_runner::*;
use nexus::compiler::codegen::compile_program_to_wasm_with_metrics;
use nexus::interpreter::Value;
use std::fs;

// -- Interpreter-based value tests (verify computation correctness) --

#[test]
fn codegen_i64_function_call_works() {
    let src = r#"
let add = fn (x: i64, y: i64) -> i64 do
    return x + y
end

let main = fn () -> i64 do
    return add(x: 40, y: 2)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_i32_arithmetic_works() {
    let src = r#"
let inc = fn (x: i32) -> i32 do
    return x + 1
end

let main = fn () -> i32 do
    let x: i32 = 41
    return inc(x: x)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_bool_return_is_i32_flag() {
    let src = r#"
let main = fn () -> bool do
    return 10 < 11
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn codegen_module_alias_call_compiles() {
    let src = r#"
import as math from examples/math.nx

let main = fn () -> i64 do
    return math.add(a: 19, b: 23)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_try_catch_handles_raised_exception() {
    let src = r#"
exception Boom(i64)

let main = fn () -> i64 effect { Exn } do
    try
      let err = Boom(42)
      raise err
      return 1
    catch e ->
      return 7
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(7));
}

#[test]
fn codegen_nested_try_catch_reraise_propagates_to_outer_catch() {
    let src = r#"
exception Boom(i64)

let main = fn () -> i64 effect { Exn } do
    try
      try
        raise Boom(1)
        return -1
      catch e ->
        raise e
        return -2
      end
      return -3
    catch outer ->
      return 9
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(9));
}

#[test]
fn codegen_match_literal_statement_returns_correct_arm() {
    let src = r#"
let main = fn () -> i64 do
    let x = 2
    match x do
      case 1 -> return 10
      case 2 -> return 20
      case _ -> return 30
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(20));
}

#[test]
fn codegen_try_catch_match_constructor_wildcard_case() {
    let src = r#"
exception Boom(i64)

let main = fn () -> i64 effect { Exn } do
    try
      raise Boom(42)
      return -1
    catch e ->
      match e do
        case Boom(_) -> return 1
        case _ -> return 2
      end
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn codegen_try_catch_match_constructor_binds_payload() {
    let src = r#"
exception Boom(i64)

let main = fn () -> i64 effect { Exn } do
    try
      raise Boom(42)
      return -1
    catch e ->
      match e do
        case Boom(code) -> return code
        case _ -> return -2
      end
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_match_record_pattern_binds_fields() {
    let src = r#"
let main = fn () -> i64 do
    let r = { y: 2, x: 40 }
    match r do
      case { x: a, y: b } -> return a + b
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_match_variable_pattern_can_return_target_value() {
    let src = r#"
let main = fn () -> i64 do
    let x = 42
    match x do
      case v -> return v
    end
    return 0
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_match_literal_then_variable_fallback() {
    let src = r#"
let main = fn () -> i64 do
    let x = 7
    match x do
      case 0 -> return 0
      case other -> return other
    end
    return -1
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(7));
}

#[test]
fn codegen_record_field_access() {
    let src = r#"
let main = fn () -> i64 do
    let r = { y: 2, x: 40 }
    let v = r.x
    return v
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(40));
}

#[test]
fn codegen_record_field_access_multiple() {
    let src = r#"
let main = fn () -> i64 do
    let r = { a: 10, b: 32 }
    let x = r.a
    let y = r.b
    return x + y
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_record_field_access_then_arithmetic() {
    let src = r#"
let main = fn () -> i64 do
    let r = { x: 20, y: 22 }
    let a = r.x
    let b = r.y
    return a + b
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_negate_function() {
    let src = r#"
import { negate } from stdlib/core.nx

let main = fn () -> i64 do
    let t = negate(val: true)
    let f = negate(val: false)
    if t then return 1 else
    if f then return 42 else return 0 end
    end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn codegen_string_return_is_supported() {
    let src = r#"
let main = fn () -> string do
    return "hello"
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("hello".to_string()));
}

#[test]
fn codegen_string_concat_operator_is_supported() {
    let src = r#"
let main = fn () -> string do
    let msg = "foo" ++ "bar"
    return msg
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("foobar".to_string()));
}

// -- WASM compilation + execution tests (main -> unit only) --

#[test]
fn codegen_exports_wasi_cli_run_wrapper() {
    let src = r#"
let main = fn () -> unit do
    return ()
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let run = run_wasi_cli_run(&wasm).expect("wasi:cli/run wrapper should run");
    assert_eq!(run, 0);
}

#[test]
fn codegen_fixture_fib_works_in_wasm() {
    let src = fs::read_to_string("examples/fib.nx").expect("fixture should exist");
    let wasm = compile_src(&src).expect("fib fixture should compile");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_raise_compiles_and_traps() {
    let src = r#"
exception Boom(i64)

let main = fn () -> unit effect { Exn } do
    let err = Boom(42)
    raise err
    return ()
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let _err = run_main_unit_traps(&wasm).expect_err("main should trap");
}

#[test]
fn codegen_fixture_network_access_compiles() {
    let src = fs::read_to_string("examples/network_access.nx").expect("fixture should exist");
    let wasm = compile_src(&src).expect("network_access fixture should compile");
    assert!(!wasm.is_empty(), "compiled wasm should not be empty");
}

#[test]
fn codegen_print_works_via_external_stdio_module() {
    let src = r#"
import external stdlib/stdlib.wasm
external __nx_print = "__nx_print" : (val: string) -> unit

let main = fn () -> unit do
    __nx_print(val: "hello wasm")
    return ()
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_print_after_from_i64_works_via_single_string_abi_module() {
    let src = r#"
import external stdlib/stdlib.wasm
external __nx_print = "__nx_print" : (val: string) -> unit

let main = fn () -> unit do
    let s = from_i64(val: 42)
    __nx_print(val: s)
    return ()
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_handler_reachability_resolves_port_call() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.print(val: "hello")
    end
    return ()
end
"#;
    let wasm = compile_src(src).expect("handler port call should compile");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_exn_constructor_lowering() {
    let src = r#"
let main = fn () -> unit effect { Exn } do
    raise RuntimeError(val: "test error")
    return ()
end
"#;
    let wasm = compile_src(src).expect("Exn constructor should compile");
    let _err = run_main_unit_traps(&wasm).expect_err("main should trap");
}

#[test]
fn codegen_fixture_di_port_compiles() {
    let src = fs::read_to_string("examples/di_port.nx").expect("fixture should exist");
    let wasm = compile_src(&src).expect("di_port fixture should compile");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_fixture_module_test_compiles() {
    let src = fs::read_to_string("examples/module_test.nx").expect("fixture should exist");
    let wasm = compile_src(&src).expect("module_test fixture should compile");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_conc_exports_tasks_and_imports_runtime() {
    let src = r#"
let main = fn () -> unit do
    let x = 1
    conc do
        task t1 do
            let a = x + 1
            return ()
        end
        task t2 do
            let b = x + 2
            return ()
        end
    end
    return ()
end
"#;
    let wasm = compile_src(src).expect("conc block should compile");
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_spawn_import = false;
    let mut has_join_import = false;
    let mut exports = std::collections::HashSet::new();

    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(reader) => {
                for import in reader.into_iter().flatten() {
                    match import.name {
                        "__nx_conc_spawn" => has_spawn_import = true,
                        "__nx_conc_join" => has_join_import = true,
                        _ => {}
                    }
                }
            }
            wasmparser::Payload::ExportSection(reader) => {
                for export in reader.into_iter().flatten() {
                    exports.insert(export.name.to_string());
                }
            }
            _ => {}
        }
    }

    assert!(has_spawn_import, "should import __nx_conc_spawn");
    assert!(has_join_import, "should import __nx_conc_join");
    assert!(exports.contains("__conc_t1"), "should export __conc_t1");
    assert!(exports.contains("__conc_t2"), "should export __conc_t2");
}

#[test]
fn codegen_conc_block_compiles_task_functions() {
    let src = r#"
let main = fn () -> unit do
    let x = 1
    conc do
        task t1 do
            let a = x + 1
            return ()
        end
        task t2 do
            let b = x + 2
            return ()
        end
    end
    return ()
end
"#;
    let wasm = compile_src(src).expect("conc block should compile to WASM");
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    // Count internal functions via wasmparser: main + 2 tasks + wasi_cli_run wrapper = 4
    let mut func_count = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(_) = payload.unwrap() {
            func_count += 1;
        }
    }
    assert_eq!(
        func_count, 4,
        "expected 4 code entries (main + 2 tasks + wasi:cli/run wrapper), got {}",
        func_count
    );
}

#[test]
fn codegen_conc_block_executes_tasks_in_parallel() {
    let src = r#"
let main = fn () -> unit do
    let x = 1
    conc do
        task t1 do
            let a = x + 1
            return ()
        end
        task t2 do
            let b = x + 2
            return ()
        end
    end
    return ()
end
"#;
    let wasm = compile_src(src).expect("conc block should compile");
    run_wasi_cli_run(&wasm).expect("conc block should execute via wasi:cli/run");
}

#[test]
fn codegen_conc_fs_writes_in_parallel() {
    let src = r#"
import as fs from stdlib/fs.nx

let main = fn () -> unit require { PermFs } do
    conc do
        task write_a do
            fs.write_string(path: "nexus_conc_test_a.txt", content: "hello")
            return ()
        end
        task write_b do
            fs.write_string(path: "nexus_conc_test_b.txt", content: "world")
            return ()
        end
    end
    return ()
end
"#;
    let wasm = compile_src(src).expect("conc block should compile");
    run_main_unit_with_wasi(&wasm).expect("conc fs should execute");

    // Verify files were actually written by conc tasks
    let a = fs::read_to_string("nexus_conc_test_a.txt").expect("file a should exist");
    let b = fs::read_to_string("nexus_conc_test_b.txt").expect("file b should exist");
    assert_eq!(a, "hello");
    assert_eq!(b, "world");
    let _ = fs::remove_file("nexus_conc_test_a.txt");
    let _ = fs::remove_file("nexus_conc_test_b.txt");
}

#[test]
fn codegen_main_non_unit_return_is_rejected() {
    let src = r#"
let main = fn () -> i64 do
    return 42
end
"#;
    let err = compile_src(src).unwrap_err();
    assert!(err.contains("main must return unit"), "got: {}", err);
}

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_codegen_arithmetic_associativity(a in -100i64..100, b in -100i64..100, c in -100i64..100) {
        let src = format!("
let main = fn () -> i64 do
    return ({} + {}) + {}
end
", a, b, c);
        assert_eq!(
            crate::common::source::run(&src).unwrap(),
            Value::Int((a + b) + c)
        );
    }

    #[test]
    fn prop_codegen_simple_if(a in 0i64..10) {
        let src = format!("
let main = fn () -> i64 do
    if {} > 5 then
        return 1
    else
        return 2
    end
    return 0
end
", a);
        let expected = if a > 5 { 1 } else { 2 };
        assert_eq!(
            crate::common::source::run(&src).unwrap(),
            Value::Int(expected)
        );
    }
}

#[test]
fn compile_metrics_reports_all_pass_durations() {
    let src = r#"
let main = fn () -> unit do
    return ()
end
"#;
    let program = nexus::lang::parser::parser().parse(src).unwrap();
    let (wasm, metrics) = compile_program_to_wasm_with_metrics(&program).unwrap();
    assert!(!wasm.is_empty());
    assert!(!metrics.hir_build.is_zero());
    assert!(!metrics.mir_lower.is_zero());
    assert!(!metrics.lir_lower.is_zero());
    assert!(!metrics.codegen.is_zero());
}

/// Regression: bundle_core_wasm must resolve stdlib memory import via wasm-merge.
/// (nexus-rx0: --skip-export-conflicts caused memory import to remain unresolved)
#[test]
fn bundle_core_wasm_resolves_stdlib_imports() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "hello")
  end
  return ()
end
"#;
    let wasm = compile_src(src).expect("compile stdlib program");
    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should resolve stdlib imports");
    let merged_imports =
        nexus::compiler::bundler::module_import_names(&merged).expect("parse merged imports");
    assert!(
        !merged_imports.iter().any(|m| m.contains("stdlib")),
        "stdlib imports should be resolved after bundling, got: {:?}",
        merged_imports
    );
}

/// Regression: conc programs with stdlib imports must also bundle successfully.
/// nexus:runtime/conc is host-provided and must be skipped by the bundler.
#[test]
fn bundle_core_wasm_resolves_conc_plus_stdlib() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

let work = fn () -> i64 do
  return 42
end

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    conc do
      task t1 do
        let _ = work()
      end
    end
    Console.println(val: "done")
  end
  return ()
end
"#;
    let wasm = compile_src(src).expect("compile conc+stdlib program");
    let imports = nexus::compiler::bundler::module_import_names(&wasm).expect("parse imports");
    assert!(imports.contains("nexus:runtime/conc"));
    assert!(imports.contains("nxlib/stdlib/stdlib.wasm"));

    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should succeed for conc+stdlib programs");
    let merged_imports =
        nexus::compiler::bundler::module_import_names(&merged).expect("parse merged imports");
    assert!(
        !merged_imports.iter().any(|m| m.contains("stdlib")),
        "stdlib should be resolved, got: {:?}",
        merged_imports
    );
    assert!(
        merged_imports.contains("nexus:runtime/conc"),
        "nexus:runtime/conc should remain (host-provided), got: {:?}",
        merged_imports
    );
}
