mod artifact;
mod cli;
mod driver;

use clap::Parser;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::ExitCode;

use nexus::compiler::bundler;
use nexus::repl;
use nexus::runtime::{self, ExecutionCapabilities};

use cli::{
    build_execution_capabilities, default_wasm_output_path, extract_main_requires, load_source,
    strip_shebang, Cli, Command,
};
use driver::{
    compile_loaded_source_to_wasm, compile_loaded_source_to_wasm_no_typecheck, parse_program,
    typecheck_program,
};

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
            skip_typecheck,
            guest_args,
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
            run_command(input, capabilities, cli.verbose, skip_typecheck, guest_args)
        }
        Some(Command::Build {
            input,
            output,
            wasm_merge,
            explain_capabilities,
            explain_capabilities_format,
            skip_typecheck,
        }) => build_command(
            input,
            output,
            wasm_merge,
            explain_capabilities,
            explain_capabilities_format,
            cli.verbose,
            skip_typecheck,
        ),
        Some(Command::Check { input, format }) => check_command(input, format),
        Some(Command::Lsp) => lsp_command(),
        Some(Command::Exec {
            input,
            allow_fs,
            allow_net,
            allow_console,
            allow_random,
            allow_clock,
            allow_proc,
            allow_env,
            preopen,
            guest_args,
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
            exec_command(input, capabilities, guest_args)
        }
        None => {
            if io::stdin().is_terminal() {
                repl::start(ExecutionCapabilities::deny_all());
                ExitCode::SUCCESS
            } else {
                run_command(None, ExecutionCapabilities::deny_all(), cli.verbose, false, vec![])
            }
        }
    }
}

fn run_command(
    input: Option<std::path::PathBuf>,
    capabilities: ExecutionCapabilities,
    verbose: bool,
    skip_typecheck: bool,
    guest_args: Vec<String>,
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
    let compile_fn = if skip_typecheck {
        compile_loaded_source_to_wasm_no_typecheck
    } else {
        compile_loaded_source_to_wasm
    };
    let compiled = match compile_fn(&loaded, true, &wasm_merge_command, verbose)
    {
        Ok(compiled) => compiled,
        Err(code) => return code,
    };

    let module_dir = input_path.as_deref().and_then(|p| p.parent());
    nexus::runtime::wasm_exec::run_wasm_bytes(&compiled.wasm, module_dir, &capabilities, &guest_args)
}

fn build_command(
    input: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    wasm_merge: Option<std::path::PathBuf>,
    explain: cli::ExplainCapabilities,
    format: cli::ExplainCapabilitiesFormat,
    verbose: bool,
    skip_typecheck: bool,
) -> ExitCode {
    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };

    let wasm_merge_command = bundler::resolve_wasm_merge_command(wasm_merge.as_deref());
    let compile_fn = if skip_typecheck {
        compile_loaded_source_to_wasm_no_typecheck
    } else {
        compile_loaded_source_to_wasm
    };
    let compiled = match compile_fn(&loaded, true, &wasm_merge_command, verbose)
    {
        Ok(c) => c,
        Err(code) => return code,
    };
    let final_wasm = if skip_typecheck {
        // Skip component encoding for bootstrap builds
        compiled.wasm.clone()
    } else {
        match artifact::encode_core_wasm_as_component(
            &compiled.wasm,
            compiled.app_needs_nexus_host,
        ) {
            Ok(component_wasm) => component_wasm,
            Err(msg) => {
                eprintln!("Component Encode Error: {}", msg);
                return ExitCode::from(1);
            }
        }
    };
    let output_path = output.unwrap_or_else(default_wasm_output_path);
    if let Err(e) = fs::write(&output_path, &final_wasm) {
        eprintln!("Failed to write {}: {}", output_path.display(), e);
        return ExitCode::from(1);
    }
    let output_name = output_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let caps = runtime::parse_nexus_capabilities(&final_wasm);
    artifact::print_build_result(&output_name, &caps, &explain, &format);
    ExitCode::SUCCESS
}

fn exec_command(
    input: std::path::PathBuf,
    capabilities: ExecutionCapabilities,
    guest_args: Vec<String>,
) -> ExitCode {
    let wasm = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Failed to read {}: {}", input.display(), e);
            return ExitCode::from(1);
        }
    };
    let module_dir = input.parent();
    runtime::wasm_exec::run_wasm_bytes(&wasm, module_dir, &capabilities, &guest_args)
}

fn check_command(input: Option<std::path::PathBuf>, format: cli::CheckFormat) -> ExitCode {
    match format {
        cli::CheckFormat::Json => {
            let loaded = match load_source(input) {
                Ok(loaded) => loaded,
                Err(msg) => {
                    // Even errors should be valid JSON
                    let err = serde_json::json!({
                        "file": "<stdin>",
                        "ok": false,
                        "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "severity": "error", "message": msg}],
                        "symbols": []
                    });
                    println!("{}", serde_json::to_string_pretty(&err).unwrap());
                    return ExitCode::from(1);
                }
            };
            let src = strip_shebang(loaded.source);
            let result = nexus::lsp::analyze(&loaded.display_name, &src);
            println!("{}", serde_json::to_string_pretty(&result.check).unwrap());
            if result.check.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        cli::CheckFormat::Text => {
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
    }
}

fn lsp_command() -> ExitCode {
    match nexus::lsp::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("LSP server error: {}", e);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn wasm_header_detection_distinguishes_core_and_component() {
        let core = b"\0asm\x01\0\0\0";
        let component = b"\0asm\x0d\0\x01\0";
        assert!(!artifact::is_component_wasm(core));
        assert!(artifact::is_component_wasm(component));
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
        let program = nexus::lang::ast::Program {
            definitions: vec![],
            source_file: None,
            source_text: None,
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
        let program = nexus::lang::parser::parser()
            .parse(src)
            .expect("parse should succeed");
        let requires = extract_main_requires(&program).expect("should find main requires");
        match requires {
            nexus::lang::ast::Type::Row(items, _) => {
                assert_eq!(items.len(), 1);
                assert_eq!(
                    items[0],
                    nexus::lang::ast::Type::UserDefined("PermNet".to_string(), vec![])
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
