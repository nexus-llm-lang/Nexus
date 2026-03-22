//! Compilation pipeline assembly and execution.

use ariadne::{Color, Label, Report, ReportKind, Source};
use std::path::Path;
use std::process::ExitCode;
use wasmparser::Payload;

use crate::cli::{strip_shebang, LoadedSource};
use nexus::compiler;
use nexus::compiler::bundler::{self, BundleConfig};
use nexus::constants::NEXUS_HOST_HTTP_MODULE;
use nexus::lang;

use crate::artifact::{merge_backtrace_stubs, merge_nexus_host_stubs};

/// Compiled wasm with metadata about the pre-merge module.
pub struct CompiledWasm {
    pub wasm: Vec<u8>,
    /// Whether the pre-merge app module directly imported `nexus:cli/nexus-host`.
    /// The stdlib bundle always carries the host imports from the net sub-crate,
    /// but only programs that actually use net need the host adapter composed in.
    pub app_needs_nexus_host: bool,
}

fn compile_loaded_source_to_core_wasm(
    loaded: &LoadedSource,
    verbose: bool,
) -> Result<Vec<u8>, ExitCode> {
    let src = strip_shebang(loaded.source.clone());
    let mut program = match parse_program(&loaded.display_name, &src) {
        Some(p) => p,
        None => return Err(ExitCode::from(1)),
    };
    program.source_file = Some(loaded.display_name.clone());
    program.source_text = Some(src.clone());
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
                    let report = Report::build(ReportKind::Error, &loaded.display_name, span.start)
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

pub fn compile_loaded_source_to_wasm(
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
    // Satisfy nexus:runtime/backtrace imports with no-op stubs so the
    // component encoder sees no unresolved host imports.
    let merged = {
        let merged_imports = bundler::module_import_names(&merged).unwrap_or_default();
        if merged_imports.contains(nexus::runtime::backtrace::BT_HOST_MODULE) {
            match merge_backtrace_stubs(&merged, wasm_merge_command) {
                Ok(m) => m,
                Err(msg) => {
                    eprintln!("Bundle Error (bt-stub merge): {}", msg);
                    return Err(ExitCode::from(1));
                }
            }
        } else {
            merged
        }
    };
    Ok(CompiledWasm {
        wasm: merged,
        app_needs_nexus_host,
    })
}

pub fn parse_program(filename: &str, src: &str) -> Option<lang::ast::Program> {
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

pub fn typecheck_program(filename: &str, src: &str, program: &lang::ast::Program) -> bool {
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
            let mut builder = Report::build(ReportKind::Error, filename, e.span.start)
                .with_message(e.message.clone())
                .with_label(
                    Label::new((filename, e.span))
                        .with_message(e.message)
                        .with_color(Color::Red),
                );
            for (span, msg) in e.labels {
                builder = builder.with_label(
                    Label::new((filename, span))
                        .with_message(msg)
                        .with_color(Color::Blue),
                );
            }
            let report = builder.finish();
            if let Err(print_err) = report.print((filename, Source::from(src))) {
                eprintln!("Failed to render type diagnostic: {}", print_err);
            }
            false
        }
    }
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
