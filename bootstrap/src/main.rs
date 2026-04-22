mod artifact;
mod cli;
mod driver;

use clap::Parser;
use std::fs;
use std::process::ExitCode;

use nexus::runtime;

use cli::{default_wasm_output_path, load_source, Cli, Command};

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
        Some(Command::Build {
            input,
            output,
            explain_capabilities,
            explain_capabilities_format,
            ..
        }) => build_command(
            input,
            output,
            explain_capabilities,
            explain_capabilities_format,
            cli.verbose,
        ),
        Some(Command::Compose { input, output }) => compose_command(input, output),
        None => {
            eprintln!("No command specified. Use `nexus build <file>` or `nexus --help`.");
            ExitCode::from(1)
        }
    }
}

fn compose_command(input: std::path::PathBuf, output: Option<std::path::PathBuf>) -> ExitCode {
    let core_wasm = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Error: failed to read {}: {}", input.display(), e);
            return ExitCode::from(1);
        }
    };
    match nexus::compiler::compose::compose_with_stdlib_and_host(&core_wasm) {
        Ok(component) => {
            let out_path = output.unwrap_or_else(|| input.with_extension("component.wasm"));
            if let Err(e) = std::fs::write(&out_path, &component) {
                eprintln!("Error: failed to write {}: {}", out_path.display(), e);
                return ExitCode::from(1);
            }
            eprintln!(
                "Composed: {} ({} bytes)",
                out_path.display(),
                component.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: composition failed: {}", e);
            ExitCode::from(1)
        }
    }
}

fn build_command(
    input: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    explain: cli::ExplainCapabilities,
    format: cli::ExplainCapabilitiesFormat,
    verbose: bool,
) -> ExitCode {
    let loaded = match load_source(input) {
        Ok(loaded) => loaded,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(1);
        }
    };

    let core_wasm = match driver::compile_loaded_source_to_core_wasm(&loaded, verbose) {
        Ok(wasm) => wasm,
        Err(code) => return code,
    };
    let final_wasm = match nexus::compiler::compose::compose_with_stdlib_and_host(&core_wasm) {
        Ok(component_wasm) => {
            // Save component to bootstrap cache so bootstrap.sh can skip stage0 recompile.
            if loaded.display_name.ends_with("src/driver.nx") {
                let cache_dir = std::path::Path::new("target/nexus");
                let _ = fs::create_dir_all(cache_dir);
                let _ = fs::write(cache_dir.join("nexus.wasm"), &component_wasm);
            }
            component_wasm
        }
        Err(msg) => {
            eprintln!("Composition Error: {}", msg);
            return ExitCode::from(1);
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
    fn build_default_output_name_is_main_wasm() {
        assert_eq!(default_wasm_output_path(), PathBuf::from("main.wasm"));
    }
}
