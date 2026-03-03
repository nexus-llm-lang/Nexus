mod common;

use common::wasm_runner::*;
use std::fs;

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
    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 42);
}

#[test]
fn codegen_exports_wasi_cli_run_wrapper() {
    let src = r#"
let main = fn () -> i64 do
    return 42
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let main = run_main_i64(&wasm).expect("wasm main should run");
    let run = run_wasi_cli_run(&wasm).expect("wasi:cli/run wrapper should run");
    assert_eq!(main, 42);
    assert_eq!(run, 0);
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
    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i32(&wasm).expect("wasm main should run");
    assert_eq!(result, 42);
}

#[test]
fn codegen_bool_return_is_i32_flag() {
    let src = r#"
let main = fn () -> bool do
    return 10 < 11
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i32(&wasm).expect("wasm main should run");
    assert_eq!(result, 1);
}

#[test]
fn codegen_module_alias_call_compiles() {
    let src = r#"
import as math from examples/math.nx

let main = fn () -> i64 do
    return math.add(a: 19, b: 23)
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 42);
}

#[test]
fn codegen_string_return_is_supported() {
    let src = r#"
let main = fn () -> string do
    return [=[hello]=]
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let packed = run_main_i64(&wasm).expect("wasm main should run");
    let len = (packed as u64 & 0xffff_ffff) as u32;
    assert_eq!(len, 5);
}

#[test]
fn codegen_string_concat_operator_is_supported() {
    let src = r#"
let main = fn () -> string do
    let msg = [=[foo]=] ++ [=[bar]=]
    return msg
end
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let packed = run_main_i64(&wasm).expect("wasm main should run");
    let len = (packed as u64 & 0xffff_ffff) as u32;
    assert_eq!(len, 6);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 7);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 9);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 20);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 1);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 42);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 42);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 42);
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

    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i64(&wasm).expect("wasm main should run");
    assert_eq!(result, 7);
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
import external nxlib/stdlib/stdio.wasm
external __nx_print = [=[__nx_print]=] : (val: string) -> unit

let main = fn () -> unit do
    __nx_print(val: [=[hello wasm]=])
    return ()
end
"#;

    let wasm = compile_src(src).expect("compile should succeed");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_print_after_from_i64_works_via_single_string_abi_module() {
    let src = r#"
import external nxlib/stdlib/stdio.wasm
external __nx_print = [=[__nx_print]=] : (val: string) -> unit

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
fn codegen_record_field_access() {
    let src = r#"
let main = fn () -> i64 do
    let r = { y: 2, x: 40 }
    let v = r.x
    return v
end
"#;
    let wasm = compile_src(src).expect("compile");
    assert_eq!(run_main_i64(&wasm).unwrap(), 40);
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
    let wasm = compile_src(src).expect("compile");
    assert_eq!(run_main_i64(&wasm).unwrap(), 42);
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
    let wasm = compile_src(src).expect("compile");
    assert_eq!(run_main_i64(&wasm).unwrap(), 42);
}

#[test]
fn codegen_negate_function() {
    let src = r#"
import { negate } from nxlib/stdlib/core.nx
let main = fn () -> i64 do
    let t = negate(val: true)
    let f = negate(val: false)
    if t then return 1 else
    if f then return 42 else return 0 end
    end
end
"#;
    let wasm = compile_src(src).expect("compile");
    assert_eq!(run_main_i64(&wasm).unwrap(), 42);
}

#[test]
fn codegen_handler_reachability_resolves_port_call() {
    let src = r#"
import { Console }, * as stdio from nxlib/stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.print(val: [=[hello]=])
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
    raise RuntimeError(val: [=[test error]=])
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
