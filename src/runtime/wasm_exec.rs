use crate::constants::*;
use crate::runtime::backtrace;
use crate::runtime::conc;
use crate::runtime::net_host;
use crate::runtime::ExecutionCapabilities;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use wasmtime::{
    component::{Component, Linker as ComponentLinker, ResourceTable},
    Engine, Linker, Module, Store,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

fn is_component_wasm(wasm: &[u8]) -> bool {
    wasmparser::Parser::is_component(wasm)
}

struct ComponentStoreData {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
}

impl WasiView for ComponentStoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for ComponentStoreData {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

/// Executes wasm bytes and dispatches to core or component runtime automatically.
pub fn run_wasm_bytes(
    wasm: &[u8],
    module_dir: Option<&Path>,
    capabilities: &ExecutionCapabilities,
    guest_args: &[String],
) -> ExitCode {
    // Validate declared capabilities before running
    if let Err(msg) = capabilities.validate_wasm_capabilities(wasm) {
        eprintln!("Capability error: {}", msg);
        return ExitCode::from(1);
    }
    if is_component_wasm(wasm) {
        return run_component_wasm_bytes(wasm, capabilities, guest_args);
    }
    run_core_wasm_bytes(wasm, module_dir, capabilities, guest_args)
}

fn run_component_wasm_bytes(
    wasm: &[u8],
    capabilities: &ExecutionCapabilities,
    guest_args: &[String],
) -> ExitCode {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    let engine = match Engine::new(&config) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!(
                "Failed to initialize Wasmtime engine for component model: {}",
                e
            );
            return ExitCode::from(1);
        }
    };
    let component = match Component::from_binary(&engine, wasm) {
        Ok(component) => component,
        Err(e) => {
            eprintln!("Failed to load wasm component: {}", e);
            return ExitCode::from(1);
        }
    };

    let mut linker = ComponentLinker::<ComponentStoreData>::new(&engine);
    if let Err(e) = wasmtime_wasi::p2::add_to_linker_sync(&mut linker) {
        eprintln!("Failed to add WASI preview2 to component linker: {}", e);
        return ExitCode::from(1);
    }
    if let Err(e) = wasmtime_wasi_http::add_only_http_to_linker_sync(&mut linker) {
        eprintln!("Failed to add WASI HTTP to component linker: {}", e);
        return ExitCode::from(1);
    }

    let mut builder = WasiCtxBuilder::new();
    if !guest_args.is_empty() {
        let mut all_args = vec!["nexus".to_string()];
        all_args.extend(guest_args.iter().cloned());
        builder.args(&all_args);
    }
    if let Err(msg) = capabilities.apply_to_wasi_builder(&mut builder) {
        eprintln!("Failed to apply capability policy: {}", msg);
        return ExitCode::from(1);
    }
    let store_data = ComponentStoreData {
        table: ResourceTable::new(),
        wasi: builder.build(),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, store_data);

    let instance = match linker.instantiate(&mut store, &component) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            return ExitCode::from(1);
        }
    };

    let main = match instance.get_typed_func::<(), ()>(&mut store, ENTRYPOINT) {
        Ok(main) => main,
        Err(e) => {
            eprintln!(
                "Runtime Error: could not find exported '{}' with signature () -> (): {}",
                ENTRYPOINT, e
            );
            return ExitCode::from(1);
        }
    };
    match main.call(&mut store, ()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            ExitCode::from(1)
        }
    }
}

fn run_core_wasm_bytes(
    wasm: &[u8],
    module_dir: Option<&Path>,
    capabilities: &ExecutionCapabilities,
    guest_args: &[String],
) -> ExitCode {
    // Spawn a thread with a large stack so deeply-recursive WASM programs
    // (e.g. the self-hosting compiler) don't overflow the default 8 MiB stack.
    let wasm = wasm.to_vec();
    let module_dir = module_dir.map(|p| p.to_path_buf());
    let capabilities = capabilities.clone();
    let guest_args = guest_args.to_vec();
    let handle = std::thread::Builder::new()
        .name("wasm-exec".into())
        .stack_size(64 * 1024 * 1024) // 64 MiB native stack
        .spawn(move || {
            run_core_wasm_bytes_inner(&wasm, module_dir.as_deref(), &capabilities, &guest_args)
        })
        .expect("failed to spawn wasm-exec thread");
    handle.join().unwrap_or(ExitCode::from(1))
}

fn run_core_wasm_bytes_inner(
    wasm: &[u8],
    module_dir: Option<&Path>,
    capabilities: &ExecutionCapabilities,
    guest_args: &[String],
) -> ExitCode {
    let mut config = wasmtime::Config::new();
    config.max_wasm_stack(64 * 1024 * 1024); // 64 MiB
    config.wasm_tail_call(true);
    config.wasm_exceptions(true);
    config.wasm_backtrace(true);
    let engine = match Engine::new(&config) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("Failed to create engine: {}", e);
            return ExitCode::from(1);
        }
    };
    let module = match Module::from_binary(&engine, wasm) {
        Ok(module) => module,
        Err(e) => {
            eprintln!("Failed to load wasm module: {}", e);
            return ExitCode::from(1);
        }
    };

    let has_conc = conc::needs_conc_runtime(wasm);
    let has_bt = backtrace::needs_bt_runtime(wasm);

    let mut linker = Linker::<wasmtime_wasi::p1::WasiP1Ctx>::new(&engine);
    if let Err(e) = wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx) {
        eprintln!("Failed to add WASI to linker: {}", e);
        return ExitCode::from(1);
    }
    if let Err(msg) = capabilities.enforce_denied_wasi_functions(&mut linker) {
        eprintln!("Failed to enforce WASI capability policy: {}", msg);
        return ExitCode::from(1);
    }
    if has_conc {
        if let Err(e) = conc::add_conc_to_linker(&mut linker) {
            eprintln!("Failed to add conc runtime to linker: {}", e);
            return ExitCode::from(1);
        }
    }
    // Always add net_host — dep modules (e.g. stdlib) may import from it
    // even if the main module doesn't directly.
    if let Err(e) = net_host::add_net_host_to_linker(&mut linker) {
        eprintln!("Failed to add net host to linker: {}", e);
        return ExitCode::from(1);
    }
    let mut builder = WasiCtxBuilder::new();
    if let Err(msg) = capabilities.apply_to_wasi_builder(&mut builder) {
        eprintln!("Failed to apply capability policy: {}", msg);
        return ExitCode::from(1);
    }
    if !guest_args.is_empty() {
        // Prepend program name as argv[0] (Unix convention).
        let mut all_args = vec!["nexus".to_string()];
        all_args.extend(guest_args.iter().cloned());
        builder.args(&all_args);
    }
    let mut store = Store::new(&engine, builder.build_p1());
    if has_bt {
        backtrace::reset();
        if let Err(e) = backtrace::add_bt_to_linker(&mut linker, &mut store) {
            eprintln!("Failed to add backtrace runtime to linker: {}", e);
            return ExitCode::from(1);
        }
    }

    let mut imported_modules = module
        .imports()
        .map(|import| import.module().to_string())
        .collect::<Vec<_>>();
    imported_modules.sort();
    imported_modules.dedup();

    let mut conc_deps = Vec::new();
    for module_name in imported_modules {
        if module_name == WASI_SNAPSHOT_MODULE || module_name == conc::CONC_HOST_MODULE {
            continue;
        }
        if module_name == backtrace::BT_HOST_MODULE {
            continue; // handled by backtrace linker
        }
        if module_name == NEXUS_HOST_HTTP_MODULE {
            continue; // handled by net_host linker
        }
        if is_preview2_wasi_module(&module_name) {
            eprintln!(
                "Runtime Error: preview2/WASI import '{}' cannot run in core-wasm mode",
                module_name
            );
            eprintln!("Hint: use `nexus build` and run the output with `wasmtime run`.");
            return ExitCode::from(1);
        }

        let Some(module_dir) = module_dir else {
            eprintln!(
                "Runtime Error: unresolved import module '{}' (no module dir available)",
                module_name
            );
            return ExitCode::from(1);
        };
        let dep_path = {
            let raw = PathBuf::from(&module_name);
            if raw.is_absolute() {
                raw
            } else {
                module_dir.join(raw)
            }
        };
        let dep = match Module::from_file(&engine, &dep_path) {
            Ok(dep) => dep,
            Err(e) => {
                eprintln!(
                    "Failed to load dependency module '{}' (resolved as '{}'): {}",
                    module_name,
                    dep_path.display(),
                    e
                );
                return ExitCode::from(1);
            }
        };
        if let Err(e) = linker.module(&mut store, &module_name, &dep) {
            eprintln!("Failed to link dependency module '{}': {}", module_name, e);
            return ExitCode::from(1);
        }
        if has_conc {
            conc_deps.push((module_name.clone(), Arc::new(dep)));
        }
    }

    if has_conc {
        conc::setup_conc_runtime(
            engine.clone(),
            Arc::new(module.clone()),
            wasm,
            conc_deps,
            capabilities.clone(),
        );
    }

    let instance = match linker.instantiate(&mut store, &module) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            return ExitCode::from(1);
        }
    };

    let main = match instance.get_typed_func::<(), ()>(&mut store, ENTRYPOINT) {
        Ok(main) => main,
        Err(e) => {
            eprintln!(
                "Runtime Error: could not find exported '{}' with signature () -> (): {}",
                ENTRYPOINT, e
            );
            return ExitCode::from(1);
        }
    };
    let result = match main.call(&mut store, ()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            ExitCode::from(1)
        }
    };
    if has_conc {
        conc::reset_conc_runtime();
    }
    result
}
