//! Component composition — encodes user + stdlib core modules as components
//! and composes them into a single runnable component via `wasm-compose`.
//!
//! Replaces the legacy `wasm-merge` bundling pipeline.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use wasm_compose::composer::ComponentComposer;
use wasm_compose::config::{Config as ComposeConfig, Dependency as ComposeDependency};
use wasmparser::Payload;
use wit_component::{embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::Resolve;

use crate::constants::{ENTRYPOINT, NEXUS_CAPABILITIES_SECTION, WASI_SNAPSHOT_MODULE};
use crate::runtime;

/// The stdlib component core module (built with `--features component`).
const STDLIB_COMPONENT_WASM: &[u8] =
    include_bytes!("../../nxlib/stdlib/stdlib-component.wasm");

/// The nexus-host bridge core module (HTTP bridge component).
const NEXUS_HOST_BRIDGE_WASM: &[u8] =
    include_bytes!("../../nxlib/stdlib/nexus-host-bridge.wasm");

/// Full WIT source for stdlib interfaces.
const STDLIB_WIT: &str = include_str!("../../src/lib/stdlib_bundle/wit/world.wit");

/// WIT source for the nexus:cli package (imported by stdlib for net sub-crate).
const NEXUS_CLI_WIT: &str =
    include_str!("../../src/lib/stdlib_bundle/wit/deps/nexus-cli.wit");

/// WIT source for the nexus:runtime package (backtrace, lazy, conc).
/// WIT source for the wasi:cli package (run interface).
const WASI_CLI_WIT: &str = "package wasi:cli@0.2.6;\n\ninterface run {\n  run: func() -> result;\n}\n";

const NEXUS_RUNTIME_WIT: &str = "\
package nexus:runtime;\n\
\n\
interface backtrace {\n\
    capture-backtrace: func();\n\
    bt-depth: func() -> s64;\n\
    bt-frame: func(idx: s64) -> s64;\n\
}\n\
\n\
interface lazy {\n\
    lazy-spawn: func(thunk: s64, env-size: s32) -> s64;\n\
    lazy-join: func(task-id: s64) -> s64;\n\
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
    let caps = runtime::parse_nexus_capabilities(user_core_wasm);

    // Detect which stdlib interfaces and other modules the user imports.
    let import_modules = core_import_modules(user_core_wasm)?;
    let nexus_imports: Vec<&str> = import_modules
        .iter()
        .filter(|m| m.starts_with("nexus:stdlib/") || m.starts_with("nexus:runtime/"))
        .map(|s| s.as_str())
        .collect();
    let has_stdlib = nexus_imports.iter().any(|m| m.starts_with("nexus:stdlib/"));

    if nexus_imports.is_empty() {
        // No nexus imports — just encode as component directly.
        return encode_standalone_component(user_core_wasm, &caps);
    }

    // Build WIT world for the user module.
    let app_wit = build_app_wit(&nexus_imports, include_host);

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

/// All known stdlib WIT interfaces.
const ALL_STDLIB_INTERFACES: &[&str] = &[
    "nexus:stdlib/math",
    "nexus:stdlib/string-ops",
    "nexus:stdlib/stdio",
    "nexus:stdlib/filesystem",
    "nexus:stdlib/network",
    "nexus:stdlib/process",
    "nexus:stdlib/environment",
    "nexus:stdlib/clock",
    "nexus:stdlib/random",
    "nexus:stdlib/collections",
    "nexus:stdlib/bytebuffer",
    "nexus:stdlib/core",
];

/// Build a WIT world source for the user app, importing the given stdlib interfaces.
fn build_app_wit(all_imports: &[&str], include_host: bool) -> String {
    let mut wit = String::new();
    wit.push_str("package nexus:app;\n\n");
    wit.push_str("world app {\n");
    let mut seen = std::collections::HashSet::new();
    for iface in all_imports {
        // Include nexus:stdlib/* and nexus:runtime/* imports.
        if !iface.starts_with("nexus:stdlib/") && !iface.starts_with("nexus:runtime/") {
            continue;
        }
        // "nexus:stdlib/bundle" is a catch-all — expand to all interfaces.
        if *iface == "nexus:stdlib/bundle" {
            for &all in ALL_STDLIB_INTERFACES {
                if seen.insert(all) {
                    wit.push_str(&format!("    import {};\n", all));
                }
            }
        } else if seen.insert(iface) {
            wit.push_str(&format!("    import {};\n", iface));
        }
    }
    wit.push_str(&format!("    export {}: func();\n", ENTRYPOINT));
    wit.push_str("}\n");
    wit
}

/// Encode the user's core WASM as a component.
fn encode_user_component(core_wasm: &[u8], app_wit: &str, command: bool) -> Result<Vec<u8>, String> {
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

/// Compose user component with multiple dependency components via wasm-compose.
fn compose_components_multi(
    user_component: &[u8],
    deps: &[(&str, &[u8])],
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
        .map_err(|e| format!("failed to create temp compose dir: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let user_path = temp_dir.join("user.wasm");
        fs::write(&user_path, user_component)
            .map_err(|e| format!("failed to write user component: {}", e))?;

        let mut config = ComposeConfig {
            dir: temp_dir.clone(),
            search_paths: vec![temp_dir.clone()],
            disallow_imports: false,
            ..Default::default()
        };

        // Write deps and register them for interface-based matching.
        let user_imports = component_import_names(user_component)?;
        for (name, bytes) in deps {
            let dep_path = temp_dir.join(name);
            fs::write(&dep_path, bytes)
                .map_err(|e| format!("failed to write dep {}: {}", name, e))?;

            // Register this dep for each user import it can satisfy.
            let dep_file = PathBuf::from(*name);
            for import_name in &user_imports {
                if !config.dependencies.contains_key(import_name) {
                    config.dependencies.insert(
                        import_name.clone(),
                        ComposeDependency { path: dep_file.clone() },
                    );
                }
            }
        }

        ComponentComposer::new(&user_path, &config)
            .compose()
            .map_err(|e| format!("component composition failed: {e:#}"))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Compose two components: primary + single dependency.
fn compose_two(primary: &[u8], dep_name: &str, dep_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-compose2-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp compose dir: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let primary_path = temp_dir.join("primary.wasm");
        let dep_path = temp_dir.join(dep_name);
        fs::write(&primary_path, primary)
            .map_err(|e| format!("failed to write primary component: {}", e))?;
        fs::write(&dep_path, dep_bytes)
            .map_err(|e| format!("failed to write dep component: {}", e))?;

        let dep_file = PathBuf::from(dep_name);
        let mut config = ComposeConfig {
            dir: temp_dir.clone(),
            disallow_imports: false,
            ..Default::default()
        };

        // Register the dep for all primary imports (composer matches by interface).
        let primary_imports = component_import_names(primary)?;
        for import_name in &primary_imports {
            config.dependencies.insert(
                import_name.clone(),
                ComposeDependency { path: dep_file.clone() },
            );
        }

        ComponentComposer::new(&primary_path, &config)
            .compose()
            .map_err(|e| format!("compose_two failed: {e:#}"))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Compose user + stdlib + nexus-host in one step.
/// All components are placed in a temp dir and the composer auto-discovers them.
fn compose_all(
    user_component: &[u8],
    stdlib_component: &[u8],
    nexus_host_component: &[u8],
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
            if import_name.starts_with("nexus:stdlib/") {
                config.dependencies.insert(
                    import_name.clone(),
                    ComposeDependency { path: stdlib_file.clone() },
                );
            }
        }
        // Satisfy stdlib's transitive nexus:cli/nexus-host import.
        config.dependencies.insert(
            "nexus:cli/nexus-host".to_string(),
            ComposeDependency { path: nexus_host_file },
        );

        ComponentComposer::new(&user_path, &config)
            .compose()
            .map_err(|e| format!("component composition failed: {e:#}"))
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// Compose user component + stdlib component via wasm-compose.
fn compose_components(
    user_component: &[u8],
    stdlib_component: &[u8],
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
        .map_err(|e| format!("failed to create temp compose dir: {}", e))?;

    let result = (|| -> Result<Vec<u8>, String> {
        let user_path = temp_dir.join("user.wasm");
        let stdlib_path = temp_dir.join("stdlib.wasm");

        fs::write(&user_path, user_component)
            .map_err(|e| format!("failed to write user component: {}", e))?;
        fs::write(&stdlib_path, stdlib_component)
            .map_err(|e| format!("failed to write stdlib component: {}", e))?;

        // Register the stdlib component as the provider for all nexus:stdlib/*
        // imports. Each interface name (e.g. "nexus:stdlib/math") must be a
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
            if import_name.starts_with("nexus:stdlib/") {
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
fn encode_standalone_component(
    core_wasm: &[u8],
    caps: &[String],
) -> Result<Vec<u8>, String> {
    let wit_source =
        "package nexus:app;\n\nworld app {\n  export main: func();\n}\n".to_string();

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
