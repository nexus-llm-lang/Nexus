use chumsky::Parser;
use nexus::compiler::codegen::compile_program_to_wasm;
use nexus::lang::parser;
use std::fs;
use std::path::PathBuf;
use wasmtime::{Engine, Instance, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

fn compile_src(src: &str) -> Result<Vec<u8>, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    compile_program_to_wasm(&program).map_err(|e| e.to_string())
}

fn run_main_i64(wasm: &[u8]) -> Result<i64, String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), i64>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

fn run_main_i32(wasm: &[u8]) -> Result<i32, String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), i32>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

fn run_wasi_cli_run(wasm: &[u8]) -> Result<i32, String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let run = instance
        .get_typed_func::<(), i32>(&mut store, "wasi:cli/run@0.2.6#run")
        .map_err(|e| e.to_string())?;
    run.call(&mut store, ()).map_err(|e| e.to_string())
}

fn run_main_unit_traps(wasm: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ())
        .map_err(|e| e.to_string())
        .and_then(|_| Err("expected trap but main returned successfully".to_string()))
}

fn run_main_unit_with_wasi(wasm: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    let _ = builder.preopened_dir(".", "/", DirPerms::all(), FilePerms::all());
    let wasi = builder.build_p1();
    let mut store = Store::new(&engine, wasi);

    let mut imported_modules = module
        .imports()
        .map(|i| i.module().to_string())
        .collect::<Vec<_>>();
    imported_modules.sort();
    imported_modules.dedup();
    for module_name in imported_modules {
        if module_name == "wasi_snapshot_preview1" {
            continue;
        }
        let path = PathBuf::from(&module_name);
        let dep = Module::from_file(&engine, &path).map_err(|e| e.to_string())?;
        linker
            .module(&mut store, &module_name, &dep)
            .map_err(|e| e.to_string())?;
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}

#[test]
fn codegen_i64_function_call_works() {
    let src = r#"
let add = fn (x: i64, y: i64) -> i64 do
    return x + y
endfn

let main = fn () -> i64 do
    return add(x: 40, y: 2)
endfn
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
endfn
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
endfn

let main = fn () -> i32 do
    let x: i32 = 41
    return inc(x: x)
endfn
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
endfn
"#;
    let wasm = compile_src(src).expect("compile should succeed");
    let result = run_main_i32(&wasm).expect("wasm main should run");
    assert_eq!(result, 1);
}

#[test]
fn codegen_module_alias_call_compiles() {
    let src = r#"
import as math from [=[examples/math.nx]=]

let main = fn () -> i64 do
    return math.add(a: 19, b: 23)
endfn
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
endfn
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
endfn
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
endfn
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
    endtry
    return 0
endfn
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
      endtry
      return -3
    catch outer ->
      return 9
    endtry
    return 0
endfn
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
    endmatch
    return 0
endfn
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
      endmatch
    endtry
    return 0
endfn
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
      endmatch
    endtry
    return 0
endfn
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
    endmatch
    return 0
endfn
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
    endmatch
    return 0
endfn
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
    endmatch
    return -1
endfn
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
let main = fn () -> unit effect { IO } do
    perform print(val: [=[hello wasm]=])
    return ()
endfn
"#;

    let wasm = compile_src(src).expect("compile should succeed");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}

#[test]
fn codegen_print_after_i64_to_string_works_via_single_string_abi_module() {
    let src = r#"
let main = fn () -> unit effect { IO } do
    let s = i64_to_string(val: 42)
    perform print(val: s)
    return ()
endfn
"#;

    let wasm = compile_src(src).expect("compile should succeed");
    run_main_unit_with_wasi(&wasm).expect("wasm main should run");
}
