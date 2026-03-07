use nexus::compiler::codegen::compile_program_to_wasm;
use nexus::lang::parser;
use nexus::lang::stdlib::resolve_import_path;
use nexus::runtime::conc;
use std::path::PathBuf;
use std::sync::Arc;
use wasmtime::{Engine, Instance, Linker, Module, Store};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

pub fn compile_src(src: &str) -> Result<Vec<u8>, String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("parse error: {:?}", e))?;
    compile_program_to_wasm(&program).map_err(|e| e.to_string())
}

pub fn run_wasi_cli_run(wasm: &[u8]) -> Result<i32, String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;

    if conc::needs_conc_runtime(wasm) {
        let module = Arc::new(module);
        conc::setup_conc_runtime(
            engine.clone(),
            module.clone(),
            wasm,
            vec![],
            nexus::runtime::ExecutionCapabilities::permissive_legacy(),
        );
        let mut linker = Linker::new(&engine);
        conc::add_conc_to_linker(&mut linker).map_err(|e| e.to_string())?;
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &*module)
            .map_err(|e| e.to_string())?;
        let run = instance
            .get_typed_func::<(), i32>(&mut store, "wasi:cli/run@0.2.6#run")
            .map_err(|e| e.to_string())?;
        return run.call(&mut store, ()).map_err(|e| e.to_string());
    }

    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).map_err(|e| e.to_string())?;
    let run = instance
        .get_typed_func::<(), i32>(&mut store, "wasi:cli/run@0.2.6#run")
        .map_err(|e| e.to_string())?;
    run.call(&mut store, ()).map_err(|e| e.to_string())
}

pub fn run_main_unit_traps(wasm: &[u8]) -> Result<(), String> {
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

/// Provide stub (trap) implementations for `nexus:cli/nexus-host` so that
/// the stdlib bundle can be instantiated even when the test doesn't use net.
pub fn define_nexus_host_stubs(
    linker: &mut Linker<wasmtime_wasi::p1::WasiP1Ctx>,
) -> Result<(), String> {
    const MOD: &str = "nexus:cli/nexus-host";
    linker
        .func_wrap(
            MOD,
            "host-http-request",
            |_: wasmtime::Caller<'_, _>,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32,
             _: i32| {},
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-listen",
            |_: wasmtime::Caller<'_, _>, _: i32, _: i32| -> i64 { -1 },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-accept",
            |_: wasmtime::Caller<'_, _>, _: i64, _: i32| {},
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-respond",
            |_: wasmtime::Caller<'_, _>, _: i64, _: i64, _: i32, _: i32, _: i32, _: i32| -> i32 {
                0
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            MOD,
            "host-http-stop",
            |_: wasmtime::Caller<'_, _>, _: i64| -> i32 { 0 },
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn run_main_unit_with_wasi(wasm: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, wasm).map_err(|e| e.to_string())?;
    let has_conc = conc::needs_conc_runtime(wasm);

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;
    define_nexus_host_stubs(&mut linker)?;
    if has_conc {
        conc::add_conc_to_linker(&mut linker)?;
    }

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

    let mut deps = Vec::new();
    for module_name in imported_modules {
        if module_name == "wasi_snapshot_preview1"
            || module_name == conc::CONC_HOST_MODULE
            || module_name == "nexus:cli/nexus-host"
        {
            continue;
        }
        let resolved = resolve_import_path(&module_name);
        let path = PathBuf::from(&resolved);
        let dep = Arc::new(Module::from_file(&engine, &path).map_err(|e| e.to_string())?);
        linker
            .module(&mut store, &module_name, &*dep)
            .map_err(|e| e.to_string())?;
        deps.push((module_name.clone(), dep));
    }

    if has_conc {
        conc::setup_conc_runtime(
            engine.clone(),
            Arc::new(module.clone()),
            wasm,
            deps,
            nexus::runtime::ExecutionCapabilities::permissive_legacy(),
        );
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| e.to_string())?;
    main.call(&mut store, ()).map_err(|e| e.to_string())
}
