use ariadne::{Color, Label, Report, ReportKind, Source};
use chumsky::Parser as _;
use clap::{Parser, Subcommand};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, IsTerminal, Read};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use wasm_compose::{
    composer::ComponentComposer,
    config::{Config as ComposeConfig, Dependency as ComposeDependency},
};
use wasmparser::Payload;
use wasmtime::{Engine, Module, ValType};
use wit_component::{embed_component_metadata, ComponentEncoder, StringEncoding};
use wit_parser::Resolve;

mod compiler;
mod lang;

mod interpreter;
mod runtime;

use runtime::ExecutionCapabilities;

#[derive(Debug, Parser)]
#[command(name = "nexus")]
#[command(about = "Nexus language CLI")]
struct Cli {
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
        /// Preopen a host directory for guest filesystem access (repeatable).
        #[arg(long, value_name = "DIR")]
        preopen: Vec<PathBuf>,
    },
    /// Parse, typecheck, and build executable/component artifact.
    /// If no file is passed and stdin is piped, reads script from stdin.
    Build {
        /// Nexus source file path. Use '-' to read from stdin.
        input: Option<PathBuf>,
        /// Output path (packed executable by default, or wasm with --wasm).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Emit a component-model wasm instead of a packed executable.
        #[arg(long)]
        wasm: bool,
    },
    /// Build a single executable by embedding bundled component wasm into the current nexus binary.
    /// If no file is passed and stdin is piped, reads script from stdin.
    Pack {
        /// Nexus source file path. Use '-' to read from stdin.
        input: Option<PathBuf>,
        /// Output executable path.
        #[arg(short, long)]
        output: Option<PathBuf>,
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
    source_path: Option<PathBuf>,
}

const WASI_SNAPSHOT_MODULE: &str = "wasi_snapshot_preview1";
const NEXUS_HOST_HTTP_MODULE: &str = "nexus:cli/nexus-host";
const WASM_MERGE_MAIN_NAME: &str = "__nexus_main__";
const MAX_BUNDLE_STEPS: usize = 128;
const PACK_MAGIC: &[u8; 16] = b"NEXUS_PACK_WASM!";
const PACK_TRAILER_LEN: usize = 8 + PACK_MAGIC.len();
static TOOL_AVAILABILITY_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

fn is_preview2_wasi_module(module_name: &str) -> bool {
    module_name.starts_with("wasi:")
}

#[cfg(test)]
fn is_component_wasm(wasm: &[u8]) -> bool {
    wasmparser::Parser::is_component(wasm)
}

fn main() -> ExitCode {
    if let Some(code) = maybe_run_embedded_wasm() {
        return code;
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run {
            input,
            allow_fs,
            preopen,
        }) => {
            let capabilities = match build_execution_capabilities(allow_fs, preopen) {
                Ok(capabilities) => capabilities,
                Err(msg) => {
                    eprintln!("Capability Error: {}", msg);
                    return ExitCode::from(1);
                }
            };
            run_command(input, capabilities)
        }
        Some(Command::Build {
            input,
            output,
            wasm,
        }) => build_command(input, output, wasm),
        Some(Command::Pack { input, output }) => pack_command(input, output),
        Some(Command::Check { input }) => check_command(input),
        None => {
            if io::stdin().is_terminal() {
                interpreter::repl::start();
                ExitCode::SUCCESS
            } else {
                run_command(None, ExecutionCapabilities::deny_all())
            }
        }
    }
}

fn run_command(input: Option<PathBuf>, capabilities: ExecutionCapabilities) -> ExitCode {
    if let Some(path) = input.as_deref() {
        if path != Path::new("-") && path.extension().is_some_and(|ext| ext == "wasm") {
            eprintln!("`nexus run` executes Nexus source only, not wasm modules.");
            eprintln!("Hint: run wasm with `wasmtime run ... {}`", path.display());
            return ExitCode::from(1);
        }
    }

    if input.is_none() && io::stdin().is_terminal() {
        interpreter::repl::start();
        return ExitCode::SUCCESS;
    }

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
    if !typecheck_program(&loaded.display_name, &src, &program) {
        return ExitCode::from(1);
    }

    let mut interp = interpreter::Interpreter::new_with_capabilities(program, capabilities);
    match interp.run_function("main", vec![]) {
        Ok(interpreter::Value::Unit) => ExitCode::SUCCESS,
        Ok(value) => {
            println!("Result: {:?}", value);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            ExitCode::from(1)
        }
    }
}

fn maybe_run_embedded_wasm() -> Option<ExitCode> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_bytes = fs::read(&exe_path).ok()?;
    let (_, wasm) = split_embedded_wasm(&exe_bytes)?;
    let capabilities = match parse_packed_runtime_capabilities_from_env() {
        Ok(capabilities) => capabilities,
        Err(code) => return Some(code),
    };
    Some(runtime::wasm_exec::run_wasm_bytes(
        wasm,
        exe_path.parent(),
        &capabilities,
    ))
}

fn build_execution_capabilities(
    allow_fs: bool,
    preopen_dirs: Vec<PathBuf>,
) -> Result<ExecutionCapabilities, String> {
    let capabilities = ExecutionCapabilities {
        allow_net: false,
        allow_fs,
        preopen_dirs,
        net_allow_hosts: Vec::new(),
        net_block_hosts: Vec::new(),
    };
    capabilities.validate()?;
    Ok(capabilities)
}

fn parse_packed_runtime_capabilities_from_env() -> Result<ExecutionCapabilities, ExitCode> {
    let program_name = std::env::args()
        .next()
        .unwrap_or_else(|| "nexus-packed".to_string());
    let mut allow_fs = false;
    let mut preopen_dirs = Vec::<PathBuf>::new();
    let mut args = std::env::args_os().skip(1).peekable();

    while let Some(arg) = args.next() {
        let arg_string = arg.to_string_lossy();
        match arg_string.as_ref() {
            "--allow-fs" => allow_fs = true,
            "--preopen" => {
                let Some(dir) = args.next() else {
                    eprintln!("Packed runtime argument error: `--preopen` requires a directory");
                    eprintln!("Usage: {} [--allow-fs] [--preopen <dir>]...", program_name);
                    return Err(ExitCode::from(1));
                };
                preopen_dirs.push(PathBuf::from(dir));
            }
            "-h" | "--help" => {
                println!("Usage: {} [--allow-fs] [--preopen <dir>]...", program_name);
                println!("  --allow-fs        allow filesystem access");
                println!("  --preopen <dir>   preopen a host directory (repeatable)");
                return Err(ExitCode::SUCCESS);
            }
            _ => {
                if let Some(dir) = arg_string.strip_prefix("--preopen=") {
                    preopen_dirs.push(PathBuf::from(dir));
                    continue;
                }
                eprintln!(
                    "Packed runtime argument error: unknown argument '{}'",
                    arg_string
                );
                eprintln!("Usage: {} [--allow-fs] [--preopen <dir>]...", program_name);
                return Err(ExitCode::from(1));
            }
        }
    }

    match build_execution_capabilities(allow_fs, preopen_dirs) {
        Ok(capabilities) => Ok(capabilities),
        Err(msg) => {
            eprintln!("Packed runtime argument error: {}", msg);
            Err(ExitCode::from(1))
        }
    }
}

fn build_command(input: Option<PathBuf>, output: Option<PathBuf>, wasm: bool) -> ExitCode {
    if !wasm {
        let output = output.or_else(|| Some(default_build_output_path()));
        return pack_command(input, output);
    }

    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };
    let core_wasm = match compile_loaded_source_to_wasm(&loaded, true, true) {
        Ok(wasm) => wasm,
        Err(code) => return code,
    };
    let wasm = match encode_core_wasm_as_component(&core_wasm) {
        Ok(component_wasm) => component_wasm,
        Err(msg) => {
            eprintln!("Component Encode Error: {}", msg);
            return ExitCode::from(1);
        }
    };

    let output_path = output.unwrap_or_else(default_wasm_output_path);
    if let Err(e) = fs::write(&output_path, wasm) {
        eprintln!("Failed to write {}: {}", output_path.display(), e);
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn pack_command(input: Option<PathBuf>, output: Option<PathBuf>) -> ExitCode {
    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };
    let core_wasm = match compile_loaded_source_to_wasm(&loaded, true, false) {
        Ok(wasm) => wasm,
        Err(code) => return code,
    };
    let wasm = match encode_core_wasm_as_component(&core_wasm) {
        Ok(component_wasm) => component_wasm,
        Err(msg) => {
            eprintln!("Component Encode Error: {}", msg);
            return ExitCode::from(1);
        }
    };

    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Failed to resolve current executable path: {}", e);
            return ExitCode::from(1);
        }
    };
    let exe_bytes = match fs::read(&exe_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!(
                "Failed to read current executable {}: {}",
                exe_path.display(),
                e
            );
            return ExitCode::from(1);
        }
    };
    let base_exe = match split_embedded_wasm(&exe_bytes) {
        Some((base, _)) => base.to_vec(),
        None => exe_bytes,
    };

    let output_path =
        output.unwrap_or_else(|| default_packed_output_path(loaded.source_path.as_deref()));
    let packed = append_embedded_wasm(&base_exe, &wasm);
    if let Err(e) = fs::write(&output_path, packed) {
        eprintln!(
            "Failed to write packed executable {}: {}",
            output_path.display(),
            e
        );
        return ExitCode::from(1);
    }
    if let Err(e) = copy_executable_permissions(&exe_path, &output_path) {
        eprintln!(
            "Failed to set executable permissions on {}: {}",
            output_path.display(),
            e
        );
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
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
            source_path: Some(path),
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
        source_path: None,
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

fn default_build_output_path() -> PathBuf {
    PathBuf::from("main.out")
}

fn default_wasm_output_path() -> PathBuf {
    PathBuf::from("main.wasm")
}

fn default_packed_output_path(input_path: Option<&Path>) -> PathBuf {
    match input_path {
        Some(path) => path.with_extension(""),
        None => PathBuf::from("app"),
    }
}

fn compile_loaded_source_to_wasm(
    loaded: &LoadedSource,
    allow_nexus_host_import: bool,
    allow_unresolved_file_imports: bool,
) -> Result<Vec<u8>, ExitCode> {
    let src = strip_shebang(loaded.source.clone());
    let program = match parse_program(&loaded.display_name, &src) {
        Some(p) => p,
        None => return Err(ExitCode::from(1)),
    };
    if !typecheck_program(&loaded.display_name, &src, &program) {
        return Err(ExitCode::from(1));
    }

    let wasm = match compiler::codegen::compile_program_to_wasm(&program) {
        Ok(wasm) => wasm,
        Err(compiler::codegen::CompileError::Lower(e)) => {
            report_lower_error(&loaded.display_name, &src, &e);
            return Err(ExitCode::from(1));
        }
        Err(compiler::codegen::CompileError::Codegen(e)) => {
            eprintln!("Codegen Error: {}", e);
            return Err(ExitCode::from(1));
        }
    };
    match bundle_external_imports(
        &wasm,
        allow_nexus_host_import,
        allow_unresolved_file_imports,
    ) {
        Ok(wasm) => Ok(wasm),
        Err(msg) => {
            eprintln!("Bundle Error: {}", msg);
            Err(ExitCode::from(1))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainResultKind {
    Unit,
    S32,
    S64,
    F32,
    F64,
}

fn detect_main_result_kind(core_wasm: &[u8]) -> Result<MainResultKind, String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, core_wasm)
        .map_err(|e| format!("failed to inspect core wasm module: {}", e))?;

    let main_export = module
        .exports()
        .find(|export| export.name() == "main")
        .ok_or_else(|| "core wasm module has no exported function 'main'".to_string())?;

    let func = match main_export.ty() {
        wasmtime::ExternType::Func(func) => func,
        _ => {
            return Err("core wasm export 'main' is not a function".to_string());
        }
    };

    if func.params().len() != 0 {
        return Err(
            "component encoding currently requires `main` to have no parameters".to_string(),
        );
    }

    let mut results = func.results();
    let first = results.next();
    let second = results.next();
    if second.is_some() {
        return Err(
            "component encoding currently requires `main` to return at most one value".to_string(),
        );
    }

    match first {
        None => Ok(MainResultKind::Unit),
        Some(ValType::I32) => Ok(MainResultKind::S32),
        Some(ValType::I64) => Ok(MainResultKind::S64),
        Some(ValType::F32) => Ok(MainResultKind::F32),
        Some(ValType::F64) => Ok(MainResultKind::F64),
        Some(other) => Err(format!(
            "component encoding does not support `main` return type {:?}",
            other
        )),
    }
}

fn main_result_wit_suffix(kind: MainResultKind) -> &'static str {
    match kind {
        MainResultKind::Unit => "",
        MainResultKind::S32 => " -> s32",
        MainResultKind::S64 => " -> s64",
        MainResultKind::F32 => " -> float32",
        MainResultKind::F64 => " -> float64",
    }
}

fn build_nexus_host_adapter_component() -> Result<Vec<u8>, String> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let core_path = repo_root.join("nxlib/stdlib/net-host-adapter.wasm");
    if !core_path.exists() {
        return Err(format!(
            "missing stdlib adapter wasm '{}'; run `cargo build` to regenerate stdlib artifacts",
            core_path.display()
        ));
    }
    let core_wasm = fs::read(&core_path).map_err(|e| {
        format!(
            "failed to read stdlib adapter wasm '{}': {}",
            core_path.display(),
            e
        )
    })?;
    let mut encoder = ComponentEncoder::default()
        .module(&core_wasm)
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
            disallow_imports: true,
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

fn encode_core_wasm_as_component(core_wasm: &[u8]) -> Result<Vec<u8>, String> {
    let main_result = detect_main_result_kind(core_wasm)?;
    let imports = module_import_names(core_wasm)?;
    let needs_nexus_host = imports.contains(NEXUS_HOST_HTTP_MODULE);
    let wit_source = if needs_nexus_host {
        format!(
            "package nexus:cli;\n\ninterface nexus-host {{\n  host-http-request: func(method: string, url: string, headers: string, body: string) -> string;\n}}\n\nworld app {{\n  import nexus-host;\n  export main: func(){};\n  export wasi:cli/run@0.2.6;\n}}\n",
            main_result_wit_suffix(main_result)
        )
    } else {
        format!(
            "package nexus:cli;\n\nworld app {{\n  export main: func(){};\n  export wasi:cli/run@0.2.6;\n}}\n",
            main_result_wit_suffix(main_result)
        )
    };
    let wasi_cli_run_wit_source =
        "package wasi:cli@0.2.6;\n\ninterface run {\n  run: func() -> result;\n}\n";

    let mut resolve = Resolve::default();
    let app_package_id = resolve
        .push_str("app.wit", &wit_source)
        .map_err(|e| format!("failed to parse app WIT world: {}", e))?;
    let _wasi_cli_package_id = resolve
        .push_str("wasi_cli_run.wit", wasi_cli_run_wit_source)
        .map_err(|e| format!("failed to parse wasi:cli/run WIT package: {}", e))?;
    let world = resolve
        .select_world(&[app_package_id], Some("app"))
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
    let component_wasm = encoder
        .encode()
        .map_err(|e| format!("failed to encode component wasm: {}", e))?;

    if needs_nexus_host {
        let adapter_component_wasm = build_nexus_host_adapter_component()?;
        compose_component_with_nexus_host_adapter(&component_wasm, &adapter_component_wasm)
    } else {
        Ok(component_wasm)
    }
}

fn append_embedded_wasm(exe: &[u8], wasm: &[u8]) -> Vec<u8> {
    let wasm_len = u64::try_from(wasm.len()).unwrap_or(u64::MAX);
    let mut out = Vec::with_capacity(exe.len() + wasm.len() + PACK_TRAILER_LEN);
    out.extend_from_slice(exe);
    out.extend_from_slice(wasm);
    out.extend_from_slice(&wasm_len.to_le_bytes());
    out.extend_from_slice(PACK_MAGIC);
    out
}

fn split_embedded_wasm(blob: &[u8]) -> Option<(&[u8], &[u8])> {
    if blob.len() < PACK_TRAILER_LEN {
        return None;
    }
    let magic_start = blob.len() - PACK_MAGIC.len();
    if &blob[magic_start..] != PACK_MAGIC {
        return None;
    }
    let len_end = magic_start;
    let len_start = len_end.checked_sub(8)?;
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&blob[len_start..len_end]);
    let wasm_len = usize::try_from(u64::from_le_bytes(len_bytes)).ok()?;
    if wasm_len > len_start {
        return None;
    }
    let wasm_start = len_start - wasm_len;
    Some((&blob[..wasm_start], &blob[wasm_start..len_start]))
}

fn copy_executable_permissions(source_exe: &Path, output_path: &Path) -> Result<(), io::Error> {
    #[cfg(unix)]
    {
        let mode = fs::metadata(source_exe)?.permissions().mode();
        let mut perms = fs::metadata(output_path)?.permissions();
        perms.set_mode(mode);
        fs::set_permissions(output_path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = source_exe;
        let _ = output_path;
    }
    Ok(())
}

fn bundle_external_imports(
    wasm: &[u8],
    allow_nexus_host_import: bool,
    allow_unresolved_file_imports: bool,
) -> Result<Vec<u8>, String> {
    let mut current = wasm.to_vec();
    let mut attempts: HashMap<String, usize> = HashMap::new();
    let wasm_merge_available = tool_is_available("wasm-merge")?;

    for _ in 0..MAX_BUNDLE_STEPS {
        let imports = module_import_names(&current)?;
        let unresolved = file_backed_imports(&imports, allow_nexus_host_import)?;
        if unresolved.is_empty() {
            return Ok(current);
        }

        if !wasm_merge_available {
            if allow_unresolved_file_imports {
                let unresolved_list = unresolved.into_iter().collect::<Vec<_>>().join(", ");
                eprintln!(
                    "Bundle Warning: 'wasm-merge' not found; keeping unresolved file-backed imports: {}",
                    unresolved_list
                );
                return Ok(current);
            }
            return Err(
                "'wasm-merge' command not found; required for packed outputs with external module imports"
                    .to_string(),
            );
        }
        let module_name = unresolved.iter().next().cloned().ok_or_else(|| {
            "codegen internal error: unresolved import set unexpectedly empty".to_string()
        })?;
        let count = attempts.entry(module_name.clone()).or_insert(0);
        if *count >= 2 {
            return Err(format!(
                "failed to resolve import module '{}' while bundling; import remains unresolved after merge",
                module_name
            ));
        }
        *count += 1;

        current = merge_single_dependency(&current, &module_name)?;
    }

    Err(format!(
        "failed to bundle external imports within {} merge steps",
        MAX_BUNDLE_STEPS
    ))
}

fn module_import_names(wasm: &[u8]) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| format!("failed to parse wasm: {}", e))?;
        if let Payload::ImportSection(section) = payload {
            for import in section {
                let import =
                    import.map_err(|e| format!("failed to parse wasm import section: {}", e))?;
                out.insert(import.module.to_string());
            }
        }
    }
    Ok(out)
}

fn file_backed_imports(
    imports: &BTreeSet<String>,
    allow_nexus_host_import: bool,
) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for module_name in imports {
        if module_name == WASI_SNAPSHOT_MODULE {
            continue;
        }
        if module_name == NEXUS_HOST_HTTP_MODULE {
            if allow_nexus_host_import {
                continue;
            }
            return Err(format!(
                "import module '{}' is deprecated; use component builds (`nexus build --wasm`) for HTTP",
                NEXUS_HOST_HTTP_MODULE
            ));
        }
        if is_preview2_wasi_module(module_name) {
            continue;
        }
        let path = Path::new(module_name);
        if !path.exists() {
            return Err(format!(
                "import module '{}' is not a local wasm path; cannot bundle dynamically",
                module_name
            ));
        }
        out.insert(module_name.clone());
    }
    Ok(out)
}

fn tool_is_available(tool: &str) -> Result<bool, String> {
    let cache = TOOL_AVAILABILITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.get(tool) {
            return Ok(*cached);
        }
    }

    match ProcessCommand::new(tool).arg("--help").output() {
        Ok(_) => {
            if let Ok(mut guard) = cache.lock() {
                guard.insert(tool.to_string(), true);
            }
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Ok(mut guard) = cache.lock() {
                guard.insert(tool.to_string(), false);
            }
            Ok(false)
        }
        Err(e) => Err(format!("failed to execute '{}': {}", tool, e)),
    }
}

fn merge_single_dependency(current_wasm: &[u8], module_name: &str) -> Result<Vec<u8>, String> {
    let dep_path = PathBuf::from(module_name).canonicalize().map_err(|e| {
        format!(
            "failed to resolve import module '{}' as a filesystem path: {}",
            module_name, e
        )
    })?;

    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-bundle-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp bundle directory: {}", e))?;

    let current_path = temp_dir.join("current.wasm");
    let merged_path = temp_dir.join("merged.wasm");
    fs::write(&current_path, current_wasm)
        .map_err(|e| format!("failed to write temporary wasm: {}", e))?;

    let output = ProcessCommand::new("wasm-merge")
        .arg(&current_path)
        .arg(WASM_MERGE_MAIN_NAME)
        .arg(&dep_path)
        .arg(module_name)
        .arg("--all-features")
        .arg("-o")
        .arg(&merged_path)
        .arg("--skip-export-conflicts")
        .output()
        .map_err(|e| format!("failed to execute wasm-merge: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!(
            "wasm-merge failed while bundling '{}': {}\n{}",
            module_name,
            stderr.trim(),
            stdout.trim()
        ));
    }

    let merged = fs::read(&merged_path).map_err(|e| {
        format!(
            "failed to read merged wasm for dependency '{}': {}",
            module_name, e
        )
    })?;
    let _ = fs::remove_dir_all(&temp_dir);
    Ok(merged)
}

fn parse_program(filename: &str, src: &str) -> Option<lang::ast::Program> {
    let parser = lang::parser::parser();
    let result = parser.parse(src.to_string());

    match result {
        Ok(program) => Some(program),
        Err(errors) => {
            for err in errors {
                let report = Report::build(ReportKind::Error, filename, err.span().start)
                    .with_message(format!("{:?}", err))
                    .with_label(
                        Label::new((filename, err.span()))
                            .with_message(format!("{}", err))
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
        Ok(_) => true,
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

fn report_lower_error(filename: &str, src: &str, err: &compiler::lower::LowerError) {
    if let Some(span) = &err.span {
        let report = Report::build(ReportKind::Error, filename, span.start)
            .with_message(err.message.clone())
            .with_label(
                Label::new((filename, span.clone()))
                    .with_message(err.message.clone())
                    .with_color(Color::Red),
            )
            .finish();
        if let Err(print_err) = report.print((filename, Source::from(src))) {
            eprintln!("Failed to render lowering diagnostic: {}", print_err);
        }
    } else {
        eprintln!("Lowering Error: {}", err.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_wasm_roundtrip() {
        let exe = b"nexus-binary";
        let wasm = b"\0asm\x01\0\0\0payload";
        let packed = append_embedded_wasm(exe, wasm);
        let (base, extracted) =
            split_embedded_wasm(&packed).expect("packed buffer should contain embedded wasm");
        assert_eq!(base, exe);
        assert_eq!(extracted, wasm);
    }

    #[test]
    fn split_embedded_wasm_rejects_invalid_trailer() {
        let blob = b"not-packed";
        assert!(split_embedded_wasm(blob).is_none());
    }

    #[test]
    fn wasm_header_detection_distinguishes_core_and_component() {
        let core = b"\0asm\x01\0\0\0";
        let component = b"\0asm\x0d\0\x01\0";
        assert!(!is_component_wasm(core));
        assert!(is_component_wasm(component));
    }

    #[test]
    fn file_backed_imports_rejects_legacy_nexus_host_module() {
        let mut imports = BTreeSet::new();
        imports.insert(WASI_SNAPSHOT_MODULE.to_string());
        imports.insert("wasi:http/outgoing-handler@0.2.0".to_string());
        imports.insert(NEXUS_HOST_HTTP_MODULE.to_string());

        let err = file_backed_imports(&imports, false)
            .expect_err("legacy nexus host module should be rejected");
        assert!(
            err.contains(NEXUS_HOST_HTTP_MODULE),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn cli_build_wasm_flag_is_supported() {
        let cli = Cli::try_parse_from(["nexus", "build", "example.nx", "--wasm"])
            .expect("`--wasm` should be accepted");
        match cli.command {
            Some(Command::Build { wasm, .. }) => assert!(wasm),
            other => panic!("unexpected parsed command: {:?}", other),
        }
    }

    #[test]
    fn cli_build_defaults_to_pack_mode_without_wasm_flag() {
        let cli = Cli::try_parse_from(["nexus", "build", "example.nx"])
            .expect("plain build should be accepted");
        match cli.command {
            Some(Command::Build { wasm, .. }) => assert!(!wasm),
            other => panic!("unexpected parsed command: {:?}", other),
        }
    }

    #[test]
    fn cli_build_component_flag_is_rejected() {
        let err = Cli::try_parse_from(["nexus", "build", "example.nx", "--component"])
            .expect_err("`--component` should not be accepted");
        let msg = err.to_string();
        assert!(
            msg.contains("--component"),
            "error message should mention removed flag, got: {}",
            msg
        );
    }

    #[test]
    fn cli_pack_component_flag_is_rejected() {
        let err = Cli::try_parse_from(["nexus", "pack", "example.nx", "--component"])
            .expect_err("`pack --component` should not be accepted");
        let msg = err.to_string();
        assert!(
            msg.contains("--component"),
            "error message should mention removed flag, got: {}",
            msg
        );
    }

    #[test]
    fn build_default_output_names_are_main_out_and_main_wasm() {
        assert_eq!(default_build_output_path(), PathBuf::from("main.out"));
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
    fn execution_capabilities_reject_preopen_without_allow_fs() {
        let err = build_execution_capabilities(false, vec![PathBuf::from("/tmp")])
            .expect_err("preopen without --allow-fs should be rejected");
        assert!(
            err.contains("--preopen"),
            "expected --preopen validation message, got: {}",
            err
        );
    }
}
