//! Compilation pipeline assembly and execution.

use ariadne::{Color, Label, Report, ReportKind, Source};
use std::process::ExitCode;

use crate::cli::{strip_shebang, LoadedSource};
use nexus::compiler;
use nexus::lang;

pub fn compile_loaded_source_to_core_wasm(
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
