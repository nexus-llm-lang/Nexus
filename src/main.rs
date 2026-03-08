use ariadne::{Color, Label, Report, ReportKind, Source};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};
use wasm_compose::{
    composer::ComponentComposer,
    config::{Config as ComposeConfig, Dependency as ComposeDependency},
};
use wasm_encoder as wenc;
use wasmparser::Payload;
use wasmtime::{Engine, Module};
use wit_component::{embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::Resolve;

use nexus::compiler;
use nexus::compiler::bundler::{self, BundleConfig, WASM_MERGE_MAIN_NAME};
use nexus::constants::{
    Permission, ENTRYPOINT, NEXUS_CAPABILITIES_SECTION, NEXUS_HOST_HTTP_MODULE,
    WASI_SNAPSHOT_MODULE,
};
use nexus::lang;
use nexus::repl;
use nexus::runtime::{self, ExecutionCapabilities};

#[derive(Debug, Clone, clap::ValueEnum)]
enum ExplainCapabilities {
    /// Show capability names (default).
    Yes,
    /// Suppress capability output.
    None,
    /// Show wasmtime run flags needed for this binary.
    Wasmtime,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum ExplainCapabilitiesFormat {
    /// Human-readable text (default).
    Text,
    /// Machine-readable JSON.
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "nexus")]
#[command(about = "Nexus language CLI")]
struct Cli {
    /// Enable verbose structured timing output to stderr
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse, typecheck, and execute Nexus source (`.nx`) using interpreter.
    /// If no file is passed and stdin is piped, reads script from stdin.
    Run {
        /// Nexus source file path. Use '-' to read from stdin.
        input: Option<PathBuf>,
        /// Allow filesystem access.
        #[arg(long)]
        allow_fs: bool,
        /// Allow network access.
        #[arg(long)]
        allow_net: bool,
        /// Allow console I/O (print, println).
        #[arg(long)]
        allow_console: bool,
        /// Allow random number generation.
        #[arg(long)]
        allow_random: bool,
        /// Allow clock/time operations.
        #[arg(long)]
        allow_clock: bool,
        /// Allow process operations (exit, etc.).
        #[arg(long)]
        allow_proc: bool,
        /// Allow environment variable access.
        #[arg(long)]
        allow_env: bool,
        /// Preopen a host directory for guest filesystem access (repeatable).
        #[arg(long, value_name = "DIR")]
        preopen: Vec<PathBuf>,
    },
    /// Parse, typecheck, and build a WASM Component artifact.
    /// If no file is passed and stdin is piped, reads script from stdin.
    Build {
        /// Nexus source file path. Use '-' to read from stdin.
        input: Option<PathBuf>,
        /// Output path (default: main.wasm).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Override `wasm-merge` executable path for dependency bundling.
        #[arg(long, value_name = "PATH")]
        wasm_merge: Option<PathBuf>,
        /// Show capability information after build.
        #[arg(long, value_enum, default_value_t = ExplainCapabilities::Yes)]
        explain_capabilities: ExplainCapabilities,
        /// Output format for capability information.
        #[arg(long, value_enum, default_value_t = ExplainCapabilitiesFormat::Text)]
        explain_capabilities_format: ExplainCapabilitiesFormat,
    },
    /// Parse and typecheck only.
    /// If no file is passed and stdin is piped, reads script from stdin.
    Check {
        /// Nexus source file path. Use '-' to read from stdin.
        input: Option<PathBuf>,
    },
}

struct LoadedSource {
    display_name: String,
    source: String,
}

const NEXUS_HOST_BRIDGE_WASM: &[u8] = include_bytes!("../nxlib/stdlib/nexus-host-bridge.wasm");
#[cfg(test)]
fn is_component_wasm(wasm: &[u8]) -> bool {
    wasmparser::Parser::is_component(wasm)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.verbose {
        use opentelemetry::trace::TracerProvider;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let tracer = provider.tracer("nexus");
        let otel_layer = tracing_opentelemetry::OpenTelemetryLayer::new(tracer);

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false);

        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("nexus=info"))
            .with(fmt_layer)
            .with(otel_layer)
            .init();
    }

    match cli.command {
        Some(Command::Run {
            input,
            allow_fs,
            allow_net,
            allow_console,
            allow_random,
            allow_clock,
            allow_proc,
            allow_env,
            preopen,
        }) => {
            let capabilities = match build_execution_capabilities(
                allow_fs,
                allow_net,
                allow_console,
                allow_random,
                allow_clock,
                allow_proc,
                allow_env,
                preopen,
            ) {
                Ok(capabilities) => capabilities,
                Err(msg) => {
                    eprintln!("Capability Error: {}", msg);
                    return ExitCode::from(1);
                }
            };
            run_command(input, capabilities, cli.verbose)
        }
        Some(Command::Build {
            input,
            output,
            wasm_merge,
            explain_capabilities,
            explain_capabilities_format,
        }) => build_command(
            input,
            output,
            wasm_merge,
            explain_capabilities,
            explain_capabilities_format,
            cli.verbose,
        ),
        Some(Command::Check { input }) => check_command(input),
        None => {
            if io::stdin().is_terminal() {
                repl::start(ExecutionCapabilities::deny_all());
                ExitCode::SUCCESS
            } else {
                run_command(None, ExecutionCapabilities::deny_all(), cli.verbose)
            }
        }
    }
}

fn extract_main_requires(program: &lang::ast::Program) -> Option<&lang::ast::Type> {
    program.definitions.iter().find_map(|def| {
        if let lang::ast::TopLevel::Let(gl) = &def.node {
            if gl.name == ENTRYPOINT {
                if let lang::ast::Expr::Lambda { requires, .. } = &gl.value.node {
                    return Some(requires);
                }
            }
        }
        None
    })
}

fn run_command(
    input: Option<PathBuf>,
    capabilities: ExecutionCapabilities,
    verbose: bool,
) -> ExitCode {
    if let Some(path) = input.as_deref() {
        if path != Path::new("-") && path.extension().is_some_and(|ext| ext == "wasm") {
            eprintln!("`nexus run` executes Nexus source only, not wasm modules.");
            eprintln!("Hint: run wasm with `wasmtime run ... {}`", path.display());
            return ExitCode::from(1);
        }
    }

    if input.is_none() && io::stdin().is_terminal() {
        repl::start(capabilities);
        return ExitCode::SUCCESS;
    }

    let input_path = input.clone();
    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };

    let src = strip_shebang(loaded.source.clone());
    if let Some(program) = parse_program(&loaded.display_name, &src) {
        if let Some(requires) = extract_main_requires(&program) {
            if let Err(msg) = capabilities.validate_program_requires(requires) {
                eprintln!("Capability Error: {}", msg);
                return ExitCode::from(1);
            }
        }
    }

    // Compile and execute via wasmtime
    let wasm_merge_command = bundler::resolve_wasm_merge_command(None);
    let compiled = match compile_loaded_source_to_wasm(&loaded, true, &wasm_merge_command, verbose)
    {
        Ok(compiled) => compiled,
        Err(code) => return code,
    };

    let module_dir = input_path.as_deref().and_then(|p| p.parent());
    runtime::wasm_exec::run_wasm_bytes(&compiled.wasm, module_dir, &capabilities)
}

fn build_execution_capabilities(
    allow_fs: bool,
    allow_net: bool,
    allow_console: bool,
    allow_random: bool,
    allow_clock: bool,
    allow_proc: bool,
    allow_env: bool,
    preopen_dirs: Vec<PathBuf>,
) -> Result<ExecutionCapabilities, String> {
    let capabilities = ExecutionCapabilities {
        allow_net,
        allow_fs,
        allow_console,
        allow_random,
        allow_clock,
        allow_proc,
        allow_env,
        preopen_dirs,
        net_allow_hosts: Vec::new(),
        net_block_hosts: Vec::new(),
    };
    capabilities.validate()?;
    Ok(capabilities)
}

fn build_command(
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    wasm_merge: Option<PathBuf>,
    explain: ExplainCapabilities,
    format: ExplainCapabilitiesFormat,
    verbose: bool,
) -> ExitCode {
    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };

    let wasm_merge_command = bundler::resolve_wasm_merge_command(wasm_merge.as_deref());
    let compiled = match compile_loaded_source_to_wasm(&loaded, true, &wasm_merge_command, verbose)
    {
        Ok(c) => c,
        Err(code) => return code,
    };
    let component_wasm =
        match encode_core_wasm_as_component(&compiled.wasm, compiled.app_needs_nexus_host) {
            Ok(component_wasm) => component_wasm,
            Err(msg) => {
                eprintln!("Component Encode Error: {}", msg);
                return ExitCode::from(1);
            }
        };
    let output_path = output.unwrap_or_else(default_wasm_output_path);
    if let Err(e) = fs::write(&output_path, &component_wasm) {
        eprintln!("Failed to write {}: {}", output_path.display(), e);
        return ExitCode::from(1);
    }
    let output_name = output_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let caps = runtime::parse_nexus_capabilities(&component_wasm);
    print_build_result(&output_name, &caps, &explain, &format);
    ExitCode::SUCCESS
}

/// Maps a capability name to the wasmtime CLI flags required.
fn capability_wasmtime_flags(cap: &str) -> Vec<&'static str> {
    match Permission::from_cap_name(cap) {
        Some(Permission::Net) => vec!["--wasi", "http", "--wasi", "inherit-network"],
        Some(Permission::Fs) => vec!["--dir", "."],
        // Console, Random, Clock, Proc are provided by the wasmtime CLI by default.
        // At the API level, PermConsole explicitly maps to WasiCtxBuilder::inherit_stdio(),
        // while Clock and Random are inherent to the default Wasmtime WasiCtx.
        _ => vec![],
    }
}

fn print_build_result(
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

fn check_command(input: Option<PathBuf>) -> ExitCode {
    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };
    let src = strip_shebang(loaded.source);
    let program = match parse_program(&loaded.display_name, &src) {
        Some(p) => p,
        None => return ExitCode::from(1),
    };
    if typecheck_program(&loaded.display_name, &src, &program) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn load_source(input: Option<PathBuf>) -> Result<LoadedSource, String> {
    if let Some(path) = input {
        if path == Path::new("-") {
            return read_source_from_stdin();
        }
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        return Ok(LoadedSource {
            display_name: path.display().to_string(),
            source,
        });
    }

    if io::stdin().is_terminal() {
        return Err("No input provided (pass file path or pipe script to stdin).".to_string());
    }
    read_source_from_stdin()
}

fn read_source_from_stdin() -> Result<LoadedSource, String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed to read stdin: {}", e))?;
    Ok(LoadedSource {
        display_name: "<stdin>".to_string(),
        source: buf,
    })
}

fn strip_shebang(source: String) -> String {
    if source.starts_with("#!") {
        if let Some(pos) = source.find('\n') {
            let mut out = String::with_capacity(source.len() - pos);
            out.push('\n');
            out.push_str(&source[pos + 1..]);
            out
        } else {
            String::new()
        }
    } else {
        source
    }
}

fn default_wasm_output_path() -> PathBuf {
    PathBuf::from("main.wasm")
}

fn compile_loaded_source_to_core_wasm(
    loaded: &LoadedSource,
    verbose: bool,
) -> Result<Vec<u8>, ExitCode> {
    let src = strip_shebang(loaded.source.clone());
    let program = match parse_program(&loaded.display_name, &src) {
        Some(p) => p,
        None => return Err(ExitCode::from(1)),
    };
    if !typecheck_program(&loaded.display_name, &src, &program) {
        return Err(ExitCode::from(1));
    }

    match compiler::codegen::compile_program_to_wasm_with_metrics(&program) {
        Ok((wasm, metrics)) => {
            if verbose {
                eprintln!(
                    "compile pass metrics:
{}",
                    metrics
                );
            }
            Ok(wasm)
        }
        Err(e) => {
            if let Some(span) = e.span() {
                if !span.is_empty() {
                    let report =
                        Report::build(ReportKind::Error, &loaded.display_name, span.start)
                            .with_message(format!("Compile Error: {}", e))
                            .with_label(
                                Label::new((&loaded.display_name, span.clone()))
                                    .with_message(e.to_string())
                                    .with_color(Color::Red),
                            )
                            .finish();
                    let _ = report.print((&loaded.display_name, Source::from(&src)));
                } else {
                    eprintln!("Compile Error: {}", e);
                }
            } else {
                eprintln!("Compile Error: {}", e);
            }
            Err(ExitCode::from(1))
        }
    }
}

/// Compiled wasm with metadata about the pre-merge module.
struct CompiledWasm {
    wasm: Vec<u8>,
    /// Whether the pre-merge app module directly imported `nexus:cli/nexus-host`.
    /// The stdlib bundle always carries the host imports from the net sub-crate,
    /// but only programs that actually use net need the host adapter composed in.
    app_needs_nexus_host: bool,
}

fn compile_loaded_source_to_wasm(
    loaded: &LoadedSource,
    allow_nexus_host_import: bool,
    wasm_merge_command: &Path,
    verbose: bool,
) -> Result<CompiledWasm, ExitCode> {
    let wasm = match compile_loaded_source_to_core_wasm(loaded, verbose) {
        Ok(wasm) => wasm,
        Err(code) => return Err(code),
    };
    // The stdlib bundle always carries nexus:cli/nexus-host imports from the
    // net sub-crate, but only programs that actually call net FFI functions
    // (e.g. __nx_http_get) need the host adapter composed in.
    let app_needs_nexus_host = module_uses_net_ffi(&wasm);
    let config = BundleConfig {
        wasm_merge_command: wasm_merge_command.to_path_buf(),
        allow_nexus_host_import,
    };
    let merged = match bundler::bundle_core_wasm(&wasm, &config) {
        Ok(wasm) => wasm,
        Err(msg) => {
            eprintln!("Bundle Error: {}", msg);
            return Err(ExitCode::from(1));
        }
    };
    // When the app doesn't use net, the merged module still carries
    // nexus:cli/nexus-host imports from the bundle.  Satisfy them with
    // stub (unreachable) implementations so the component encoder sees
    // no unresolved host imports.
    let merged = if !app_needs_nexus_host {
        let merged_imports = bundler::module_import_names(&merged).unwrap_or_default();
        if merged_imports.contains(NEXUS_HOST_HTTP_MODULE) {
            match merge_nexus_host_stubs(&merged, wasm_merge_command) {
                Ok(m) => m,
                Err(msg) => {
                    eprintln!("Bundle Error (stub merge): {}", msg);
                    return Err(ExitCode::from(1));
                }
            }
        } else {
            merged
        }
    } else {
        merged
    };
    Ok(CompiledWasm {
        wasm: merged,
        app_needs_nexus_host,
    })
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

fn encode_core_wasm_as_component(
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
fn merge_nexus_host_stubs(wasm: &[u8], wasm_merge_command: &Path) -> Result<Vec<u8>, String> {
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

/// Returns true if the wasm module imports any function whose name starts with
/// `__nx_http` — i.e. it actually uses the net FFI.  Used to decide whether the
/// nexus-host adapter component should be composed in.
fn module_uses_net_ffi(wasm: &[u8]) -> bool {
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let Ok(payload) = payload else {
            continue;
        };
        if let Payload::ImportSection(section) = payload {
            for import in section {
                let Ok(import) = import else {
                    continue;
                };
                if import.name.starts_with("__nx_http") {
                    return true;
                }
            }
        }
    }
    false
}

fn parse_program(filename: &str, src: &str) -> Option<lang::ast::Program> {
    let parser = lang::parser::parser();
    let result = parser.parse(src);

    match result {
        Ok(program) => Some(program),
        Err(errors) => {
            for err in errors {
                let report = Report::build(ReportKind::Error, filename, err.span.start)
                    .with_message(&err.message)
                    .with_label(
                        Label::new((filename, err.span.clone()))
                            .with_message(&err.message)
                            .with_color(Color::Red),
                    )
                    .finish();
                if let Err(print_err) = report.print((filename, Source::from(src))) {
                    eprintln!("Failed to render parse diagnostic: {}", print_err);
                }
            }
            None
        }
    }
}

fn typecheck_program(filename: &str, src: &str, program: &lang::ast::Program) -> bool {
    let mut checker = lang::typecheck::TypeChecker::new();
    match checker.check_program(program) {
        Ok(_) => {
            for warning in checker.take_warnings() {
                let report = Report::build(ReportKind::Warning, filename, warning.span.start)
                    .with_message(warning.message.clone())
                    .with_label(
                        Label::new((filename, warning.span))
                            .with_message(warning.message)
                            .with_color(Color::Yellow),
                    )
                    .finish();
                if let Err(print_err) = report.print((filename, Source::from(src))) {
                    eprintln!("Failed to render type warning: {}", print_err);
                }
            }
            true
        }
        Err(e) => {
            let report = Report::build(ReportKind::Error, filename, e.span.start)
                .with_message(e.message.clone())
                .with_label(
                    Label::new((filename, e.span))
                        .with_message(e.message)
                        .with_color(Color::Red),
                )
                .finish();
            if let Err(print_err) = report.print((filename, Source::from(src))) {
                eprintln!("Failed to render type diagnostic: {}", print_err);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_header_detection_distinguishes_core_and_component() {
        let core = b"\0asm\x01\0\0\0";
        let component = b"\0asm\x0d\0\x01\0";
        assert!(!is_component_wasm(core));
        assert!(is_component_wasm(component));
    }

    #[test]
    fn cli_build_wasm_merge_flag_is_supported() {
        let cli = Cli::try_parse_from([
            "nexus",
            "build",
            "example.nx",
            "--wasm-merge",
            "/opt/bin/wasm-merge",
        ])
        .expect("`--wasm-merge` should be accepted");
        match cli.command {
            Some(Command::Build { wasm_merge, .. }) => {
                assert_eq!(wasm_merge, Some(PathBuf::from("/opt/bin/wasm-merge")));
            }
            other => panic!("unexpected parsed command: {:?}", other),
        }
    }

    #[test]
    fn cli_build_wasm_flag_is_rejected() {
        let err = Cli::try_parse_from(["nexus", "build", "example.nx", "--wasm"])
            .expect_err("`--wasm` should not be accepted (component is now the only mode)");
        let msg = err.to_string();
        assert!(
            msg.contains("--wasm"),
            "error message should mention removed flag, got: {}",
            msg
        );
    }

    #[test]
    fn build_default_output_name_is_main_wasm() {
        assert_eq!(default_wasm_output_path(), PathBuf::from("main.wasm"));
    }

    #[test]
    fn cli_run_capability_flags_are_supported() {
        let cli = Cli::try_parse_from([
            "nexus",
            "run",
            "example.nx",
            "--allow-fs",
            "--preopen",
            "/tmp",
        ])
        .expect("run capability flags should be accepted");
        match cli.command {
            Some(Command::Run {
                allow_fs, preopen, ..
            }) => {
                assert!(allow_fs);
                assert_eq!(preopen, vec![PathBuf::from("/tmp")]);
            }
            other => panic!("unexpected parsed command: {:?}", other),
        }
    }

    #[test]
    fn extract_main_requires_returns_none_when_no_main() {
        let program = lang::ast::Program {
            definitions: vec![],
        };
        assert!(extract_main_requires(&program).is_none());
    }

    #[test]
    fn extract_main_requires_returns_requires_clause() {
        let src = r#"
        let main = fn () -> unit require { PermNet } do
            return ()
        end
        "#;
        let program = lang::parser::parser()
            .parse(src)
            .expect("parse should succeed");
        let requires = extract_main_requires(&program).expect("should find main requires");
        match requires {
            lang::ast::Type::Row(items, _) => {
                assert_eq!(items.len(), 1);
                assert_eq!(
                    items[0],
                    lang::ast::Type::UserDefined("PermNet".to_string(), vec![])
                );
            }
            other => panic!("expected Row, got {:?}", other),
        }
    }

    #[test]
    fn execution_capabilities_reject_preopen_without_allow_fs() {
        let err = build_execution_capabilities(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            vec![PathBuf::from("/tmp")],
        )
        .expect_err("preopen without --allow-fs should be rejected");
        assert!(
            err.contains("--preopen"),
            "expected --preopen validation message, got: {}",
            err
        );
    }
}
