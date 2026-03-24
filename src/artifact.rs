//! Component encoding, WASM composition, and artifact management.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};
use wasm_compose::{
    composer::ComponentComposer,
    config::{Config as ComposeConfig, Dependency as ComposeDependency},
};
use wasm_encoder as wenc;
use wasmtime::{Engine, Module};
use wit_component::{embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::Resolve;

use crate::cli::{ExplainCapabilities, ExplainCapabilitiesFormat};
use nexus::compiler::bundler::WASM_MERGE_MAIN_NAME;
use nexus::constants::{
    Permission, ENTRYPOINT, NEXUS_CAPABILITIES_SECTION, NEXUS_HOST_HTTP_MODULE,
    WASI_SNAPSHOT_MODULE,
};
use nexus::runtime;

const NEXUS_HOST_BRIDGE_WASM: &[u8] = include_bytes!("../nxlib/stdlib/nexus-host-bridge.wasm");

#[cfg(test)]
pub fn is_component_wasm(wasm: &[u8]) -> bool {
    wasmparser::Parser::is_component(wasm)
}

pub fn encode_core_wasm_as_component(
    core_wasm: &[u8],
    needs_nexus_host: bool,
) -> Result<Vec<u8>, String> {
    validate_main_export(core_wasm)?;

    // Extract nexus:capabilities custom section before component encoding
    // (ComponentEncoder does not preserve custom sections from the core module).
    let caps = runtime::parse_nexus_capabilities(core_wasm);

    let wit_source = if needs_nexus_host {
        "package nexus:cli;\n\ninterface nexus-host {\n  host-http-request: func(method: string, url: string, headers: string, body: string) -> string;\n  host-http-listen: func(addr: string) -> s64;\n  host-http-accept: func(server-id: s64) -> string;\n  host-http-respond: func(req-id: s64, status: s64, headers: string, body: string) -> s32;\n  host-http-stop: func(server-id: s64) -> s32;\n}\n\nworld app {\n  import nexus-host;\n  export main: func();\n  export wasi:cli/run@0.2.6;\n}\n".to_string()
    } else {
        "package nexus:cli;\n\nworld app {\n  export main: func();\n  export wasi:cli/run@0.2.6;\n}\n".to_string()
    };
    let wasi_cli_run_wit_source =
        "package wasi:cli@0.2.6;\n\ninterface run {\n  run: func() -> result;\n}\n";

    let mut resolve = Resolve::default();
    let wasi_cli_package_id = resolve
        .push_str("wasi_cli_run.wit", wasi_cli_run_wit_source)
        .map_err(|e| format!("failed to parse wasi:cli/run WIT package: {}", e))?;
    let app_package_id = resolve
        .push_str("app.wit", &wit_source)
        .map_err(|e| format!("failed to parse app WIT world: {}", e))?;
    let world = resolve
        .select_world(
            &[app_package_id, wasi_cli_package_id],
            Some("nexus:cli/app"),
        )
        .map_err(|e| format!("failed to resolve WIT world 'app': {}", e))?;

    let mut embedded = core_wasm.to_vec();
    embed_component_metadata(&mut embedded, &resolve, world, StringEncoding::UTF8)
        .map_err(|e| format!("failed to embed component metadata: {}", e))?;

    let mut encoder = ComponentEncoder::default()
        .module(&embedded)
        .map_err(|e| format!("failed to initialize component encoder: {}", e))?
        .adapter(
            WASI_SNAPSHOT_MODULE,
            wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .map_err(|e| format!("failed to add preview1 adapter: {}", e))?
        .validate(true);
    let mut component_wasm = encoder
        .encode()
        .map_err(|e| format!("failed to encode component wasm: {}", e))?;

    if needs_nexus_host {
        let adapter_component_wasm = build_nexus_host_adapter_component()?;
        component_wasm =
            compose_component_with_nexus_host_adapter(&component_wasm, &adapter_component_wasm)?;
    }

    // Re-append nexus:capabilities custom section to the component binary.
    if !caps.is_empty() {
        append_custom_section(&mut component_wasm, &caps);
    }

    Ok(component_wasm)
}

fn validate_main_export(core_wasm: &[u8]) -> Result<(), String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, core_wasm)
        .map_err(|e| format!("failed to inspect core wasm module: {}", e))?;

    let main_export = module
        .exports()
        .find(|export| export.name() == ENTRYPOINT)
        .ok_or_else(|| "core wasm module has no exported function 'main'".to_string())?;

    let func = match main_export.ty() {
        wasmtime::ExternType::Func(func) => func,
        _ => {
            return Err("core wasm export 'main' is not a function".to_string());
        }
    };

    if func.params().len() != 0 {
        return Err("'main' must have no parameters".to_string());
    }

    if func.results().next().is_some() {
        return Err("'main' must return unit (no return values)".to_string());
    }

    Ok(())
}

fn build_nexus_host_adapter_component() -> Result<Vec<u8>, String> {
    let mut encoder = ComponentEncoder::default()
        .module(NEXUS_HOST_BRIDGE_WASM)
        .map_err(|e| format!("failed to load host adapter core module: {}", e))?
        .adapter(
            WASI_SNAPSHOT_MODULE,
            wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .map_err(|e| {
            format!(
                "failed to add preview1 adapter to host adapter module: {}",
                e
            )
        })?
        .validate(true);

    encoder
        .encode()
        .map_err(|e| format!("failed to encode host adapter component: {}", e))
}

fn compose_component_with_nexus_host_adapter(
    app_component_wasm: &[u8],
    adapter_component_wasm: &[u8],
) -> Result<Vec<u8>, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-compose-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp compose directory: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let app_component_path = temp_dir.join("app-component.wasm");
        fs::write(&app_component_path, app_component_wasm)
            .map_err(|e| format!("failed to write temporary app component wasm: {}", e))?;

        let adapter_component_file = PathBuf::from("nexus-host-adapter-component.wasm");
        let adapter_component_path = temp_dir.join(&adapter_component_file);
        fs::write(&adapter_component_path, adapter_component_wasm).map_err(|e| {
            format!(
                "failed to write temporary host adapter component wasm: {}",
                e
            )
        })?;

        let mut config = ComposeConfig {
            dir: temp_dir.clone(),
            disallow_imports: false,
            ..Default::default()
        };
        config.dependencies.insert(
            NEXUS_HOST_HTTP_MODULE.to_string(),
            ComposeDependency {
                path: adapter_component_file,
            },
        );

        ComponentComposer::new(&app_component_path, &config)
            .compose()
            .map_err(|e| format!("failed to compose component with host adapter: {e:#}"))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Appends the `nexus:capabilities` custom section to a WASM component binary.
/// Uses raw LEB128 encoding to add a section 0 (custom) at the end.
fn append_custom_section(wasm: &mut Vec<u8>, caps: &[String]) {
    use std::borrow::Cow;
    let payload = caps.join("\n");
    let section = wenc::CustomSection {
        name: Cow::Borrowed(NEXUS_CAPABILITIES_SECTION),
        data: Cow::Borrowed(payload.as_bytes()),
    };
    // Component custom sections use section id 0, same as core modules.
    // wasm_encoder::CustomSection implements ComponentSection, so we can
    // append it directly to a component.
    let mut comp = wenc::Component::new();
    comp.section(&section);
    let encoded = comp.finish();
    // The component preamble is 8 bytes (magic + version). Skip it and
    // append only the section bytes.
    wasm.extend_from_slice(&encoded[8..]);
}

/// Builds a tiny wasm module with no-op stubs for the 3 backtrace host
/// functions (`__nx_bt_push`, `__nx_bt_pop`, `__nx_bt_freeze`).  Used to
/// satisfy `nexus:runtime/backtrace` imports so the component encoder sees
/// no unresolved host imports.
fn build_backtrace_stub_module() -> Vec<u8> {
    use wenc::*;
    let mut module = wenc::Module::new();

    // Type section:
    //   0: (i64) -> ()   __nx_bt_push
    //   1: () -> ()       __nx_bt_pop, __nx_bt_freeze
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I64], vec![]);
    types.ty().function(vec![], vec![]);
    module.section(&types);

    // Function section
    let mut functions = FunctionSection::new();
    functions.function(0); // __nx_bt_push
    functions.function(1); // __nx_bt_pop
    functions.function(1); // __nx_bt_freeze
    module.section(&functions);

    // Export section
    let mut exports = ExportSection::new();
    exports.export("__nx_bt_push", ExportKind::Func, 0);
    exports.export("__nx_bt_pop", ExportKind::Func, 1);
    exports.export("__nx_bt_freeze", ExportKind::Func, 2);
    module.section(&exports);

    // Code section: all functions are no-ops
    let mut codes = CodeSection::new();
    for _ in 0..3 {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::End);
        codes.function(&f);
    }
    module.section(&codes);

    module.finish()
}

/// Merges no-op stubs for the 3 `nexus:runtime/backtrace` host functions into
/// `wasm`, satisfying those imports for component encoding.
pub fn merge_backtrace_stubs(wasm: &[u8], wasm_merge_command: &Path) -> Result<Vec<u8>, String> {
    let stub = build_backtrace_stub_module();
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-bt-stub-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp bt-stub directory: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let main_path = temp_dir.join("main.wasm");
        let stub_path = temp_dir.join("bt_stub.wasm");
        let merged_path = temp_dir.join("merged.wasm");
        fs::write(&main_path, wasm)
            .map_err(|e| format!("failed to write main wasm for bt-stub merge: {}", e))?;
        fs::write(&stub_path, &stub).map_err(|e| format!("failed to write bt-stub wasm: {}", e))?;

        let output = ProcessCommand::new(wasm_merge_command)
            .arg(&main_path)
            .arg(WASM_MERGE_MAIN_NAME)
            .arg(&stub_path)
            .arg(runtime::backtrace::BT_HOST_MODULE)
            .arg("--all-features")
            .arg("--enable-tail-call")
            .arg("--enable-multimemory")
            .arg("-o")
            .arg(&merged_path)
            .arg("--skip-export-conflicts")
            .output()
            .map_err(|e| format!("failed to run wasm-merge for bt-stub: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "wasm-merge bt-stub failed: {} {}",
                output.status,
                stderr.trim()
            ));
        }
        fs::read(&merged_path).map_err(|e| format!("failed to read bt-stub-merged wasm: {}", e))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Builds a tiny wasm module that provides stub (unreachable) implementations
/// of the 5 nexus-host functions.  Used to satisfy imports from the stdlib
/// bundle's net sub-crate when the app doesn't actually use networking.
fn build_nexus_host_stub_module() -> Vec<u8> {
    use wenc::*;
    let mut module = wenc::Module::new();

    // Type section: the 5 host function signatures.
    //   0: (i32,i32,i32,i32,i32,i32,i32,i32,i32)->()  host-http-request
    //   1: (i32,i32)->i64                                host-http-listen
    //   2: (i64,i32)->()                                 host-http-accept
    //   3: (i64,i64,i32,i32,i32,i32)->i32                host-http-respond
    //   4: (i64)->i32                                     host-http-stop
    let mut types = TypeSection::new();
    types.ty().function(
        vec![
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ],
        vec![],
    );
    types
        .ty()
        .function(vec![ValType::I32, ValType::I32], vec![ValType::I64]);
    types
        .ty()
        .function(vec![ValType::I64, ValType::I32], vec![]);
    types.ty().function(
        vec![
            ValType::I64,
            ValType::I64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ],
        vec![ValType::I32],
    );
    types.ty().function(vec![ValType::I64], vec![ValType::I32]);
    module.section(&types);

    // Function section
    let mut functions = FunctionSection::new();
    functions.function(0); // host-http-request
    functions.function(1); // host-http-listen
    functions.function(2); // host-http-accept
    functions.function(3); // host-http-respond
    functions.function(4); // host-http-stop
    module.section(&functions);

    // Export section
    let mut exports = ExportSection::new();
    exports.export("host-http-request", ExportKind::Func, 0);
    exports.export("host-http-listen", ExportKind::Func, 1);
    exports.export("host-http-accept", ExportKind::Func, 2);
    exports.export("host-http-respond", ExportKind::Func, 3);
    exports.export("host-http-stop", ExportKind::Func, 4);
    module.section(&exports);

    // Code section: all functions body = unreachable
    let mut codes = CodeSection::new();
    for _ in 0..5 {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::Unreachable);
        f.instruction(&Instruction::End);
        codes.function(&f);
    }
    module.section(&codes);

    module.finish()
}

/// Merges a stub module providing dummy (unreachable) implementations of the
/// 5 `nexus:cli/nexus-host` functions into `wasm`, satisfying those imports.
pub fn merge_nexus_host_stubs(wasm: &[u8], wasm_merge_command: &Path) -> Result<Vec<u8>, String> {
    let stub = build_nexus_host_stub_module();
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-stub-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp stub directory: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let main_path = temp_dir.join("main.wasm");
        let stub_path = temp_dir.join("stub.wasm");
        let merged_path = temp_dir.join("merged.wasm");
        fs::write(&main_path, wasm)
            .map_err(|e| format!("failed to write main wasm for stub merge: {}", e))?;
        fs::write(&stub_path, &stub).map_err(|e| format!("failed to write stub wasm: {}", e))?;

        let output = ProcessCommand::new(wasm_merge_command)
            .arg(&main_path)
            .arg(WASM_MERGE_MAIN_NAME)
            .arg(&stub_path)
            .arg(NEXUS_HOST_HTTP_MODULE)
            .arg("--all-features")
            .arg("--enable-tail-call")
            .arg("--enable-multimemory")
            .arg("-o")
            .arg(&merged_path)
            .arg("--skip-export-conflicts")
            .output()
            .map_err(|e| format!("failed to run wasm-merge for stub: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "wasm-merge stub failed: {} {}",
                output.status,
                stderr.trim()
            ));
        }
        fs::read(&merged_path).map_err(|e| format!("failed to read stub-merged wasm: {}", e))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Maps a capability name to the wasmtime CLI flags required.
pub fn capability_wasmtime_flags(cap: &str) -> Vec<&'static str> {
    match Permission::from_cap_name(cap) {
        Some(Permission::Net) => vec!["--wasi", "http", "--wasi", "inherit-network"],
        Some(Permission::Fs) => vec!["--dir", "."],
        // Console, Random, Clock, Proc are provided by the wasmtime CLI by default.
        // At the API level, PermConsole explicitly maps to WasiCtxBuilder::inherit_stdio(),
        // while Clock and Random are inherent to the default Wasmtime WasiCtx.
        _ => vec![],
    }
}

pub fn print_build_result(
    output_name: &str,
    caps: &[String],
    explain: &ExplainCapabilities,
    format: &ExplainCapabilitiesFormat,
) {
    match format {
        ExplainCapabilitiesFormat::Text => {
            print_build_result_text(output_name, caps, explain);
        }
        ExplainCapabilitiesFormat::Json => {
            print_build_result_json(output_name, caps, explain);
        }
    }
}

fn print_build_result_text(output_name: &str, caps: &[String], explain: &ExplainCapabilities) {
    eprintln!("Built {output_name}");
    match explain {
        ExplainCapabilities::None => {}
        ExplainCapabilities::Yes => {
            if !caps.is_empty() {
                eprintln!("Capabilities: {}", caps.join(", "));
            }
        }
        ExplainCapabilities::Wasmtime => {
            if !caps.is_empty() {
                eprintln!("Capabilities: {}", caps.join(", "));
            }
            let mut flags: Vec<&str> = Vec::new();
            for cap in caps {
                flags.extend(capability_wasmtime_flags(cap));
            }
            flags.dedup();
            let mut cmd_parts = vec!["wasmtime", "run"];
            cmd_parts.extend(&flags);
            cmd_parts.push(output_name);
            eprintln!("Run: {}", cmd_parts.join(" "));
        }
    }
}

fn print_build_result_json(output_name: &str, caps: &[String], explain: &ExplainCapabilities) {
    match explain {
        ExplainCapabilities::None => {
            eprintln!("{{\"file\":\"{output_name}\"}}");
        }
        ExplainCapabilities::Yes => {
            let caps_json: Vec<String> = caps.iter().map(|c| format!("\"{c}\"")).collect();
            eprintln!(
                "{{\"file\":\"{output_name}\",\"capabilities\":[{}]}}",
                caps_json.join(",")
            );
        }
        ExplainCapabilities::Wasmtime => {
            let caps_json: Vec<String> = caps.iter().map(|c| format!("\"{c}\"")).collect();
            let mut flags: Vec<&str> = Vec::new();
            for cap in caps {
                flags.extend(capability_wasmtime_flags(cap));
            }
            flags.dedup();
            let mut cmd_parts = vec!["wasmtime", "run"];
            cmd_parts.extend(&flags);
            cmd_parts.push(output_name);
            let flags_json: Vec<String> = flags.iter().map(|f| format!("\"{f}\"")).collect();
            eprintln!(
                "{{\"file\":\"{output_name}\",\"capabilities\":[{}],\"wasmtime\":{{\"command\":\"{}\",\"flags\":[{}]}}}}",
                caps_json.join(","),
                cmd_parts.join(" "),
                flags_json.join(",")
            );
        }
    }
}
