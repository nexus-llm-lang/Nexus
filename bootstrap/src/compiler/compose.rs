//! Component composition — encodes user + stdlib core modules as components
//! and composes them into a single runnable component via `wasm-compose`.
//!
//! Replaces the legacy `wasm-merge` bundling pipeline.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use wasm_compose::composer::ComponentComposer;
use wasm_compose::config::{Config as ComposeConfig, Dependency as ComposeDependency};
use wasmparser::Payload;
use wit_component::{embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::Resolve;

use crate::constants::{ENTRYPOINT, NEXUS_CAPABILITIES_SECTION, WASI_SNAPSHOT_MODULE};
use crate::lang::stdlib::is_package_wit_module;
use crate::runtime;

/// The stdlib component core module (built with `--features component`).
const STDLIB_COMPONENT_WASM: &[u8] = include_bytes!("../../../nxlib/stdlib/stdlib-component.wasm");

/// The nexus-host bridge core module (HTTP bridge component).
const NEXUS_HOST_BRIDGE_WASM: &[u8] =
    include_bytes!("../../../nxlib/stdlib/nexus-host-bridge.wasm");

/// Full WIT source for stdlib interfaces.
const STDLIB_WIT: &str = include_str!("../../src/lib/stdlib_bundle/wit/world.wit");

/// WIT source for the nexus:cli package (imported by stdlib for net sub-crate).
const NEXUS_CLI_WIT: &str = include_str!("../../src/lib/stdlib_bundle/wit/deps/nexus-cli.wit");

/// WIT source for the nexus:runtime package (backtrace, lazy, conc).
/// WIT source for the wasi:cli package (run interface).
const WASI_CLI_WIT: &str =
    "package wasi:cli@0.2.6;\n\ninterface run {\n  run: func() -> result;\n}\n";

const NEXUS_RUNTIME_WIT: &str = "\
package nexus:runtime;\n\
\n\
interface backtrace {\n\
    capture-backtrace: func();\n\
    bt-depth: func() -> s64;\n\
    bt-frame: func(idx: s64) -> string;\n\
}\n\
\n\
interface lazy {\n\
    lazy-spawn: func(thunk: s64, env-size: s32) -> s64;\n\
    lazy-join: func(task-id: s64) -> s64;\n\
}\n\
\n\
interface math {\n\
    f64-sqrt: func(val: f64) -> f64;\n\
    f64-floor: func(val: f64) -> f64;\n\
    f64-ceil: func(val: f64) -> f64;\n\
    f64-abs: func(val: f64) -> f64;\n\
}\n\
\n\
interface memory {\n\
    mem-load-i32: func(addr: s64) -> s64;\n\
    mem-store-i32: func(addr: s64, val: s64);\n\
    mem-load-u8: func(addr: s64) -> s64;\n\
    mem-store-u8: func(addr: s64, val: s64);\n\
    mem-load-i64: func(addr: s64) -> s64;\n\
    mem-store-i64: func(addr: s64, val: s64);\n\
    mem-size: func() -> s64;\n\
    mem-grow: func(delta: s64) -> s64;\n\
}\n\
";

/// Compose a user core WASM module with the stdlib component.
///
/// Steps:
/// 1. Detect which stdlib interfaces the user module imports
/// 2. Build a WIT world declaring those imports + `main` export
/// 3. Embed component metadata and encode both as components
/// 4. Compose them into a single component via wasm-compose
///
/// Returns the composed component WASM bytes.
/// Compose with stdlib only (nexus-host left as unresolved import).
/// Used by test harness which provides host stubs at runtime.
pub fn compose_with_stdlib(user_core_wasm: &[u8]) -> Result<Vec<u8>, String> {
    compose_with_stdlib_impl(user_core_wasm, false)
}

/// Compose with stdlib + nexus-host bridge (fully self-contained).
/// Used by `nexus build` for standalone component output.
pub fn compose_with_stdlib_and_host(user_core_wasm: &[u8]) -> Result<Vec<u8>, String> {
    compose_with_stdlib_impl(user_core_wasm, true)
}

fn compose_with_stdlib_impl(user_core_wasm: &[u8], include_host: bool) -> Result<Vec<u8>, String> {
    // Fix misplaced string imports from older nxc compiler output.
    // The nxc diamond-cache bug puts string-* functions under wrong modules.
    let user_core_wasm = &normalize_string_imports(user_core_wasm);
    // Fix cabi_realloc type assignment (old nxc assigns ()→() instead of (i32,i32,i32,i32)→i32).
    let user_core_wasm = &fix_cabi_realloc_type(user_core_wasm);
    // Strip "wasi:cli/run@*" export from nxc-compiled core WASM.
    // The nxc codegen exports it, but the WASI command adapter should provide it.
    // Having both causes the ComponentEncoder to wire the entry point incorrectly.
    let user_core_wasm = &fix_nxc_wasi_run_export(user_core_wasm);

    let caps = runtime::parse_nexus_capabilities(user_core_wasm);

    // Detect which package interfaces and runtime modules the user imports.
    let import_modules = core_import_modules(user_core_wasm)?;
    let nexus_imports: Vec<&str> = import_modules
        .iter()
        .filter(|m| {
            (is_package_wit_module(m) || m.starts_with("nexus:runtime/"))
                && *m != "nexus:runtime/arena" // intrinsic-only, no runtime support needed
        })
        .map(|s| s.as_str())
        .collect();
    let has_stdlib = nexus_imports.iter().any(|m| is_package_wit_module(m));

    if nexus_imports.is_empty() {
        // No nexus imports — just encode as component directly.
        return encode_standalone_component(user_core_wasm, &caps);
    }

    // Build WIT world for the user module.
    let app_wit = build_app_wit(&nexus_imports);

    // Debug: save core WASM on failure
    if std::env::var_os("NEXUS_DEBUG_COMPOSE").is_some() {
        let _ = std::fs::write("/tmp/nexus_debug_core.wasm", user_core_wasm);
        let _ = std::fs::write("/tmp/nexus_debug_app.wit", &app_wit);
    }

    // Encode user core WASM as component.
    let user_component = encode_user_component(user_core_wasm, &app_wit, include_host)
        .map_err(|e| format!("{}\n  generated WIT:\n{}", e, app_wit))?;

    if !has_stdlib {
        // Only runtime imports, no stdlib — return user component as-is.
        let mut result = user_component;
        if !caps.is_empty() {
            append_custom_section(&mut result, &caps);
        }
        return Ok(result);
    }

    // Encode stdlib core WASM as component.
    let stdlib_component = encode_stdlib_component()?;

    let composed = if include_host {
        let nexus_host_component = encode_nexus_host_component()?;
        compose_all(&user_component, &stdlib_component, &nexus_host_component)?
    } else {
        compose_components(&user_component, &stdlib_component)?
    };

    // Re-append capabilities section.
    let mut result = composed;
    if !caps.is_empty() {
        append_custom_section(&mut result, &caps);
    }

    Ok(result)
}

/// Extract unique import module names from a core WASM binary.
fn core_import_modules(wasm: &[u8]) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| format!("failed to parse wasm: {}", e))?;
        if let Payload::ImportSection(section) = payload {
            for import in section {
                let import =
                    import.map_err(|e| format!("failed to parse import section: {}", e))?;
                out.insert(import.module.to_string());
            }
        }
    }
    Ok(out)
}

/// Extract import names from a component WASM binary.
fn component_import_names(wasm: &[u8]) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| format!("failed to parse component: {}", e))?;
        if let Payload::ComponentImportSection(section) = payload {
            for import in section {
                let import =
                    import.map_err(|e| format!("failed to parse component import: {}", e))?;
                out.insert(import.name.0.to_string());
            }
        }
    }
    Ok(out)
}

/// Build a WIT world source for the user app, importing the given package interfaces.
fn build_app_wit(all_imports: &[&str]) -> String {
    let mut wit = String::new();
    wit.push_str("package nexus:app;\n\n");
    wit.push_str("world app {\n");
    let mut seen = std::collections::HashSet::new();
    for iface in all_imports {
        // Include package WIT-module imports and nexus:runtime/* imports.
        if !is_package_wit_module(iface) && !iface.starts_with("nexus:runtime/") {
            continue;
        }
        if seen.insert(iface) {
            wit.push_str(&format!("    import {};\n", iface));
        }
    }
    wit.push_str(&format!("    export {}: func();\n", ENTRYPOINT));
    wit.push_str("}\n");
    wit
}

/// Encode the user's core WASM as a component.
fn encode_user_component(
    core_wasm: &[u8],
    app_wit: &str,
    command: bool,
) -> Result<Vec<u8>, String> {
    let mut resolve = Resolve::default();
    // Push dependency packages first so stdlib/app WIT can reference them.
    let _cli_pkg = resolve
        .push_str("nexus-cli.wit", NEXUS_CLI_WIT)
        .map_err(|e| format!("failed to parse nexus-cli WIT: {}", e))?;
    let _runtime_pkg = resolve
        .push_str("nexus-runtime.wit", NEXUS_RUNTIME_WIT)
        .map_err(|e| format!("failed to parse nexus-runtime WIT: {}", e))?;
    let _stdlib_pkg = resolve
        .push_str("stdlib.wit", STDLIB_WIT)
        .map_err(|e| format!("failed to parse stdlib WIT: {}", e))?;
    let wasi_cli_pkg = resolve
        .push_str("wasi-cli.wit", WASI_CLI_WIT)
        .map_err(|e| format!("failed to parse wasi-cli WIT: {}", e))?;
    let app_pkg = resolve
        .push_str("app.wit", app_wit)
        .map_err(|e| format!("failed to parse app WIT: {}", e))?;
    let world = resolve
        .select_world(&[app_pkg, wasi_cli_pkg], Some("nexus:app/app"))
        .map_err(|e| format!("failed to resolve app world: {}", e))?;

    let mut embedded = core_wasm.to_vec();
    embed_component_metadata(&mut embedded, &resolve, world, StringEncoding::UTF8)
        .map_err(|e| format!("failed to embed component metadata: {}", e))?;

    let mut encoder = ComponentEncoder::default()
        .module(&embedded)
        .map_err(|e| format!("failed to init component encoder: {}", e))?
        .adapter(
            WASI_SNAPSHOT_MODULE,
            if command {
                wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_COMMAND_ADAPTER
            } else {
                wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER
            },
        )
        .map_err(|e| format!("failed to add WASI adapter: {}", e))?
        .validate(true);

    encoder
        .encode()
        .map_err(|e| format!("failed to encode user component: {:#}", e))
}

/// Encode the stdlib core WASM as a component.
fn encode_stdlib_component() -> Result<Vec<u8>, String> {
    // stdlib-component.wasm already has wit-bindgen metadata embedded.
    let mut encoder = ComponentEncoder::default()
        .module(STDLIB_COMPONENT_WASM)
        .map_err(|e| format!("failed to init stdlib component encoder: {}", e))?
        .adapter(
            WASI_SNAPSHOT_MODULE,
            wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .map_err(|e| format!("failed to add WASI adapter to stdlib: {}", e))?
        .validate(true);

    encoder
        .encode()
        .map_err(|e| format!("failed to encode stdlib component: {}", e))
}

/// Encode the nexus-host bridge core WASM as a component.
fn encode_nexus_host_component() -> Result<Vec<u8>, String> {
    let mut encoder = ComponentEncoder::default()
        .module(NEXUS_HOST_BRIDGE_WASM)
        .map_err(|e| format!("failed to init nexus-host component encoder: {}", e))?
        .adapter(
            WASI_SNAPSHOT_MODULE,
            wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .map_err(|e| format!("failed to add WASI adapter to nexus-host: {}", e))?
        .validate(true);

    encoder
        .encode()
        .map_err(|e| format!("failed to encode nexus-host component: {}", e))
}

/// Monotonically-increasing counter combined with PID + ThreadId to produce a
/// unique staging directory per `compose_*` invocation. Without this, two
/// threads inside the same process that mint the same `SystemTime` nanos value
/// (well within possibility on M-series machines under `--test-threads=N>1`)
/// race-overwrite each other's `user.wasm` / `stdlib.wasm` staging files and
/// the loser reads back a sibling's bytes. Same idiom as
/// `bootstrap/tests/harness/compile.rs::nxc_output_path` (nexus-gyj6).
static COMPOSE_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn compose_staging_dir(prefix: &str) -> PathBuf {
    let seq = COMPOSE_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tid = format!("{:?}", std::thread::current().id());
    // ThreadId Debug fmt is e.g. "ThreadId(7)"; strip the wrapper to keep the
    // directory name short and POSIX-portable.
    let tid_digits: String = tid.chars().filter(|c| c.is_ascii_digit()).collect();
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        tid_digits,
        seq
    ))
}

/// Compose user + stdlib + nexus-host in one step.
/// All components are placed in a temp dir and the composer auto-discovers them.
fn compose_all(
    user_component: &[u8],
    stdlib_component: &[u8],
    nexus_host_component: &[u8],
) -> Result<Vec<u8>, String> {
    let temp_dir = compose_staging_dir("nexus-compose-all");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp compose dir: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let user_path = temp_dir.join("user.wasm");
        fs::write(&user_path, user_component)
            .map_err(|e| format!("failed to write user component: {}", e))?;
        fs::write(temp_dir.join("stdlib.wasm"), stdlib_component)
            .map_err(|e| format!("failed to write stdlib component: {}", e))?;
        fs::write(temp_dir.join("nexus-host.wasm"), nexus_host_component)
            .map_err(|e| format!("failed to write nexus-host component: {}", e))?;

        // Register explicit deps for user's stdlib imports, plus search path for transitive deps.
        let stdlib_file = PathBuf::from("stdlib.wasm");
        let mut config = ComposeConfig {
            dir: temp_dir.clone(),
            search_paths: vec![temp_dir.clone()],
            disallow_imports: false,
            ..Default::default()
        };

        let nexus_host_file = PathBuf::from("nexus-host.wasm");
        let user_imports = component_import_names(user_component)?;
        for import_name in &user_imports {
            if is_package_wit_module(import_name) {
                config.dependencies.insert(
                    import_name.clone(),
                    ComposeDependency {
                        path: stdlib_file.clone(),
                    },
                );
            }
        }
        // Satisfy stdlib's transitive nexus:cli/nexus-host import.
        config.dependencies.insert(
            "nexus:cli/nexus-host".to_string(),
            ComposeDependency {
                path: nexus_host_file,
            },
        );

        ComponentComposer::new(&user_path, &config)
            .compose()
            .map_err(|e| format!("component composition failed: {e:#}"))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Compose user component + stdlib component via wasm-compose.
fn compose_components(user_component: &[u8], stdlib_component: &[u8]) -> Result<Vec<u8>, String> {
    let temp_dir = compose_staging_dir("nexus-compose");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp compose dir: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let user_path = temp_dir.join("user.wasm");
        let stdlib_path = temp_dir.join("stdlib.wasm");

        fs::write(&user_path, user_component)
            .map_err(|e| format!("failed to write user component: {}", e))?;
        fs::write(&stdlib_path, stdlib_component)
            .map_err(|e| format!("failed to write stdlib component: {}", e))?;

        // Register the stdlib component as the provider for all nexus:std/*
        // imports. Each interface name (e.g. "nexus:std/math") must be a
        // separate dependency entry pointing to the same stdlib component file.
        let stdlib_file = PathBuf::from("stdlib.wasm");
        let mut config = ComposeConfig {
            dir: temp_dir.clone(),
            disallow_imports: false,
            ..Default::default()
        };

        // Detect which stdlib interfaces the user component imports.
        // Note: user_component is component WASM; component imports have
        // the same module names as core imports for our encoding.
        let user_imports = component_import_names(user_component)?;
        for import_name in &user_imports {
            if is_package_wit_module(import_name) {
                config.dependencies.insert(
                    import_name.clone(),
                    ComposeDependency {
                        path: stdlib_file.clone(),
                    },
                );
            }
        }

        ComponentComposer::new(&user_path, &config)
            .compose()
            .map_err(|e| format!("component composition failed: {e:#}"))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Encode a standalone user component (no stdlib dependencies).
fn encode_standalone_component(core_wasm: &[u8], caps: &[String]) -> Result<Vec<u8>, String> {
    let wit_source = "package nexus:app;\n\nworld app {\n  export main: func();\n}\n".to_string();

    // Use the same approach as the old artifact.rs: push wasi:cli first, then app.
    let mut resolve = Resolve::default();
    let wasi_cli_pkg = resolve
        .push_str("wasi_cli_run.wit", WASI_CLI_WIT)
        .map_err(|e| format!("failed to parse wasi-cli WIT: {}", e))?;
    let app_pkg = resolve
        .push_str("app.wit", &wit_source)
        .map_err(|e| format!("failed to parse app WIT: {}", e))?;
    let world = resolve
        .select_world(&[app_pkg, wasi_cli_pkg], Some("nexus:app/app"))
        .map_err(|e| format!("failed to resolve app world: {}", e))?;

    let mut embedded = core_wasm.to_vec();
    embed_component_metadata(&mut embedded, &resolve, world, StringEncoding::UTF8)
        .map_err(|e| format!("failed to embed component metadata: {}", e))?;

    let mut encoder = ComponentEncoder::default()
        .module(&embedded)
        .map_err(|e| format!("failed to init component encoder: {}", e))?
        .adapter(
            WASI_SNAPSHOT_MODULE,
            wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .map_err(|e| format!("failed to add WASI adapter: {}", e))?
        .validate(true);

    let mut result = encoder
        .encode()
        .map_err(|e| format!("failed to encode standalone component: {:#}", e))?;

    if !caps.is_empty() {
        append_custom_section(&mut result, caps);
    }

    Ok(result)
}

fn append_custom_section(wasm: &mut Vec<u8>, caps: &[String]) {
    use std::borrow::Cow;
    use wasm_encoder as wenc;
    let payload = caps.join("\n");
    let section = wenc::CustomSection {
        name: Cow::Borrowed(NEXUS_CAPABILITIES_SECTION),
        data: Cow::Borrowed(payload.as_bytes()),
    };
    let mut comp = wenc::Component::new();
    comp.section(&section);
    let encoded = comp.finish();
    wasm.extend_from_slice(&encoded[8..]);
}

/// Strip "wasi:cli/run@*" export and fix "_start" to point to the wasi_run wrapper.
///
/// The nxc codegen exports `wasi:cli/run@0.2.6#run` from the core module, but
/// in the component model the WASI command adapter provides this interface.
/// Having both confuses the ComponentEncoder. Additionally, nxc maps `_start`
/// to `$main` directly instead of to the wasi_run wrapper that calls argv+main.
/// We fix `_start` to point to the wasi_run wrapper (the last function, which
/// just `call $main`).
fn fix_nxc_wasi_run_export(wasm: &[u8]) -> Vec<u8> {
    use wasm_encoder::{ExportKind, ExportSection, Module, RawSection};

    let has_wasi_run = wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| p.ok())
        .any(|p| {
            if let Payload::ExportSection(section) = p {
                section
                    .into_iter()
                    .any(|e| e.map_or(false, |e| e.name.contains("wasi:cli/run")))
            } else {
                false
            }
        });
    if !has_wasi_run {
        return wasm.to_vec();
    }

    // Find the wasi_run wrapper function index (it just calls $main).
    // It's the function exported as "wasi:cli/run@0.2.6#run".
    let mut wasi_run_idx: Option<u32> = None;
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let Ok(payload) = payload else { continue };
        if let Payload::ExportSection(section) = payload {
            for export in section {
                let Ok(export) = export else { continue };
                if export.name.starts_with("wasi:cli/run") {
                    wasi_run_idx = Some(export.index);
                }
            }
        }
    }

    let parser = wasmparser::Parser::new(0);
    let mut module = Module::new();
    for payload in parser.parse_all(wasm) {
        let payload = match payload {
            Ok(p) => p,
            Err(_) => return wasm.to_vec(),
        };
        match &payload {
            Payload::ExportSection(section) => {
                let mut exports = ExportSection::new();
                for export in section.clone() {
                    let Ok(export) = export else {
                        return wasm.to_vec();
                    };
                    // Skip all wasi:cli/run exports
                    if export.name.contains("wasi:cli/run") {
                        continue;
                    }
                    let kind = match export.kind {
                        wasmparser::ExternalKind::Func => ExportKind::Func,
                        wasmparser::ExternalKind::Table => ExportKind::Table,
                        wasmparser::ExternalKind::Memory => ExportKind::Memory,
                        wasmparser::ExternalKind::Global => ExportKind::Global,
                        wasmparser::ExternalKind::Tag => ExportKind::Tag,
                    };
                    // Fix _start to point to the wasi_run wrapper
                    let index = if export.name == "_start" {
                        wasi_run_idx.unwrap_or(export.index)
                    } else {
                        export.index
                    };
                    exports.export(export.name, kind, index);
                }
                module.section(&exports);
                continue;
            }
            _ => {}
        }
        if let Some((id, range)) = payload.as_section() {
            module.section(&RawSection {
                id,
                data: &wasm[range],
            });
        }
    }
    module.finish()
}

/// Fix cabi_realloc's type assignment in the function section.
/// Old nxc codegen assigns it the wasi_run type `()→()` instead of
/// `(i32,i32,i32,i32)→i32`. Find the correct type index and rewrite.
fn fix_cabi_realloc_type(wasm: &[u8]) -> Vec<u8> {
    use wasm_encoder::{FunctionSection, Module, RawSection};

    // Find the cabi_realloc type index: (i32, i32, i32, i32) -> i32
    let mut cabi_type_idx: Option<u32> = None;
    let mut type_idx: u32 = 0;
    let mut n_local_funcs: u32 = 0;
    let mut has_cabi_export = false;

    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let Ok(payload) = payload else { continue };
        match payload {
            Payload::TypeSection(section) => {
                for rec_group in section {
                    let Ok(rec_group) = rec_group else { continue };
                    for sub_type in rec_group.into_types() {
                        if let wasmparser::CompositeInnerType::Func(f) =
                            &sub_type.composite_type.inner
                        {
                            let params: Vec<_> = f.params().to_vec();
                            let results: Vec<_> = f.results().to_vec();
                            if params == [wasmparser::ValType::I32; 4]
                                && results == [wasmparser::ValType::I32]
                            {
                                cabi_type_idx = Some(type_idx);
                            }
                        }
                        type_idx += 1;
                    }
                }
            }
            Payload::FunctionSection(section) => {
                n_local_funcs = section.count();
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let Ok(export) = export else { continue };
                    if export.name == "cabi_realloc" {
                        has_cabi_export = true;
                    }
                }
            }
            _ => {}
        }
    }

    let cabi_type_idx = match cabi_type_idx {
        Some(idx) if has_cabi_export => idx,
        _ => return wasm.to_vec(),
    };
    // cabi_realloc is the second-to-last local function.
    let cabi_local_idx = n_local_funcs - 2;

    let parser = wasmparser::Parser::new(0);
    let mut module = Module::new();
    for payload in parser.parse_all(wasm) {
        let payload = match payload {
            Ok(p) => p,
            Err(_) => return wasm.to_vec(),
        };
        match &payload {
            Payload::FunctionSection(section) => {
                let mut funcs = FunctionSection::new();
                let mut idx: u32 = 0;
                for func_type in section.clone() {
                    let Ok(func_type) = func_type else {
                        return wasm.to_vec();
                    };
                    if idx == cabi_local_idx {
                        funcs.function(cabi_type_idx);
                    } else {
                        funcs.function(func_type);
                    }
                    idx += 1;
                }
                module.section(&funcs);
                continue;
            }
            _ => {}
        }
        if let Some((id, range)) = payload.as_section() {
            module.section(&RawSection {
                id,
                data: &wasm[range],
            });
        }
    }
    module.finish()
}

/// Returns the correct WIT module for a function name, or None if it belongs
/// to the module it's already in.
fn correct_module_for_import(module: &str, name: &str) -> Option<&'static str> {
    if module == "nexus:std/str" {
        return None; // already correct
    }
    if name.starts_with("string-") || name.starts_with("char-") {
        return Some("nexus:std/str");
    }
    None
}

/// Rewrite core WASM imports to move misplaced string functions to
/// `nexus:std/str`. Deduplicates by (module, name) pair.
/// Needed for nxc compiler output where diamond-cached imports cause
/// string functions to be registered under wrong stdlib modules.
fn normalize_string_imports(wasm: &[u8]) -> Vec<u8> {
    use wasm_encoder::{EntityType, ImportSection, Module, RawSection};

    // Quick check: any nexus:std/* import with a string-* name not in string?
    let needs_fix = wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| p.ok())
        .any(|p| {
            if let Payload::ImportSection(section) = p {
                section.into_iter().any(|i| {
                    i.map_or(false, |i| {
                        correct_module_for_import(i.module, i.name).is_some()
                    })
                })
            } else {
                false
            }
        });
    if !needs_fix {
        return wasm.to_vec();
    }

    // Remap module names without deduplicating (preserves function indices).
    let parser = wasmparser::Parser::new(0);
    let mut module = Module::new();

    for payload in parser.parse_all(wasm) {
        let payload = match payload {
            Ok(p) => p,
            Err(_) => return wasm.to_vec(),
        };
        match &payload {
            Payload::ImportSection(section) => {
                let mut imports = ImportSection::new();
                for import in section.clone() {
                    let import = match import {
                        Ok(i) => i,
                        Err(_) => return wasm.to_vec(),
                    };
                    let effective_module = correct_module_for_import(import.module, import.name)
                        .unwrap_or(import.module);
                    let entity = match import.ty {
                        wasmparser::TypeRef::Func(idx) => EntityType::Function(idx),
                        wasmparser::TypeRef::Memory(m) => {
                            EntityType::Memory(wasm_encoder::MemoryType {
                                minimum: m.initial,
                                maximum: m.maximum,
                                memory64: m.memory64,
                                shared: m.shared,
                                page_size_log2: m.page_size_log2,
                            })
                        }
                        wasmparser::TypeRef::Global(g) => {
                            let val_type = match g.content_type {
                                wasmparser::ValType::I32 => wasm_encoder::ValType::I32,
                                wasmparser::ValType::I64 => wasm_encoder::ValType::I64,
                                wasmparser::ValType::F32 => wasm_encoder::ValType::F32,
                                wasmparser::ValType::F64 => wasm_encoder::ValType::F64,
                                _ => return wasm.to_vec(),
                            };
                            EntityType::Global(wasm_encoder::GlobalType {
                                val_type,
                                mutable: g.mutable,
                                shared: g.shared,
                            })
                        }
                        _ => return wasm.to_vec(),
                    };
                    imports.import(effective_module, import.name, entity);
                }
                module.section(&imports);
                continue;
            }
            _ => {}
        }
        if let Some((id, range)) = payload.as_section() {
            module.section(&RawSection {
                id,
                data: &wasm[range],
            });
        }
    }
    module.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_imports(wasm: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for payload in wasmparser::Parser::new(0).parse_all(wasm).flatten() {
            if let Payload::ImportSection(reader) = payload {
                for imp in reader.into_iter().flatten() {
                    out.push((imp.module.to_string(), imp.name.to_string()));
                }
            }
        }
        out
    }

    /// Acceptance for nexus-upzz.10: the bridge guest must drive the
    /// FutureTrailers resource through `subscribe().block()` then `get()`
    /// before dropping it, so wasi-http hosts that defer trailer delivery
    /// can release any deferred bookkeeping. Pin the resulting wasi-http
    /// import shape against the compiled bridge module so a regression
    /// (dropping the future without polling) is caught at unit-test time.
    #[test]
    fn bridge_polls_future_trailers_before_drop() {
        let imports = collect_imports(NEXUS_HOST_BRIDGE_WASM);
        let trailers_imports: Vec<&(String, String)> = imports
            .iter()
            .filter(|(_, n)| n.contains("future-trailers"))
            .collect();
        let names: Vec<&str> = trailers_imports.iter().map(|(_, n)| n.as_str()).collect();

        assert!(
            names.iter().any(|n| n.contains("future-trailers.subscribe")),
            "bridge must subscribe() the FutureTrailers (got {:?})",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("future-trailers.get")),
            "bridge must get() the FutureTrailers after block (got {:?})",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("[resource-drop]future-trailers")),
            "bridge must drop the FutureTrailers (got {:?})",
            names
        );
    }
}
