use crate::lang::ast::Program;
use crate::lang::parser;
use crate::lang::typecheck::{TypeChecker, TypeEnv};

use super::position::LineIndex;
use super::symbols;

use serde::Serialize;

// ---------------------------------------------------------------------------
// CLI-friendly JSON types (also reused by the LSP server)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub file: String,
    pub ok: bool,
    pub diagnostics: Vec<CheckDiagnostic>,
    pub symbols: Vec<CheckSymbol>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckDiagnostic {
    pub range: CheckRange,
    pub severity: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckRange {
    pub start: CheckPosition,
    pub end: CheckPosition,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckSymbol {
    pub name: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub range: CheckRange,
}

// ---------------------------------------------------------------------------
// Full analysis result (used by LSP server, not serialized as-is)
// ---------------------------------------------------------------------------

pub struct AnalysisResult {
    pub check: CheckResult,
    pub env: Option<TypeEnv>,
    pub program: Option<Program>,
    pub line_index: LineIndex,
}

// ---------------------------------------------------------------------------
// Core analysis: parse + typecheck
// ---------------------------------------------------------------------------

pub fn analyze(filename: &str, source: &str) -> AnalysisResult {
    let line_index = LineIndex::new(source);
    let mut diagnostics = Vec::new();
    let mut syms = Vec::new();
    let mut env = None;
    let mut program = None;

    let parser = parser::parser();
    match parser.parse(source) {
        Ok(prog) => {
            // Extract document symbols
            let doc_syms = symbols::extract(&prog, &line_index);
            syms = doc_syms.iter().map(|s| to_check_symbol(s)).collect();

            // Typecheck
            let mut checker = TypeChecker::new();
            match checker.check_program(&prog) {
                Ok(()) => {
                    for w in checker.take_warnings() {
                        diagnostics.push(to_check_diag(
                            &line_index,
                            &w.span,
                            "warning",
                            &w.message,
                        ));
                    }
                    env = Some(checker.env.clone());
                }
                Err(e) => {
                    for w in checker.take_warnings() {
                        diagnostics.push(to_check_diag(
                            &line_index,
                            &w.span,
                            "warning",
                            &w.message,
                        ));
                    }
                    diagnostics.push(to_check_diag(&line_index, &e.span, "error", &e.message));
                }
            }
            program = Some(prog);
        }
        Err(errors) => {
            for e in errors {
                diagnostics.push(to_check_diag(&line_index, &e.span, "error", &e.message));
            }
        }
    }

    let ok = !diagnostics.iter().any(|d| d.severity == "error");
    AnalysisResult {
        check: CheckResult {
            file: filename.to_string(),
            ok,
            diagnostics,
            symbols: syms,
        },
        env,
        program,
        line_index,
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn to_check_diag(
    idx: &LineIndex,
    span: &std::ops::Range<usize>,
    severity: &'static str,
    message: &str,
) -> CheckDiagnostic {
    let r = idx.span_to_range(span);
    CheckDiagnostic {
        range: lsp_range_to_check(r),
        severity,
        message: message.to_string(),
    }
}

fn lsp_range_to_check(r: lsp_types::Range) -> CheckRange {
    CheckRange {
        start: CheckPosition {
            line: r.start.line,
            character: r.start.character,
        },
        end: CheckPosition {
            line: r.end.line,
            character: r.end.character,
        },
    }
}

#[allow(deprecated)]
fn to_check_symbol(s: &lsp_types::DocumentSymbol) -> CheckSymbol {
    let kind = match s.kind {
        lsp_types::SymbolKind::FUNCTION => "function",
        lsp_types::SymbolKind::CONSTANT => "constant",
        lsp_types::SymbolKind::STRUCT => "type",
        lsp_types::SymbolKind::ENUM => "enum",
        lsp_types::SymbolKind::ENUM_MEMBER => "variant",
        lsp_types::SymbolKind::EVENT => "exception",
        lsp_types::SymbolKind::INTERFACE => "port",
        lsp_types::SymbolKind::METHOD => "method",
        _ => "unknown",
    };
    CheckSymbol {
        name: s.name.clone(),
        kind,
        detail: s.detail.clone(),
        range: lsp_range_to_check(s.range),
    }
}

/// Convert our CheckDiagnostic to an lsp_types::Diagnostic.
pub fn to_lsp_diagnostic(d: &CheckDiagnostic) -> lsp_types::Diagnostic {
    lsp_types::Diagnostic {
        range: lsp_types::Range {
            start: lsp_types::Position {
                line: d.range.start.line,
                character: d.range.start.character,
            },
            end: lsp_types::Position {
                line: d.range.end.line,
                character: d.range.end.character,
            },
        },
        severity: Some(if d.severity == "error" {
            lsp_types::DiagnosticSeverity::ERROR
        } else {
            lsp_types::DiagnosticSeverity::WARNING
        }),
        source: Some("nexus".to_string()),
        message: d.message.clone(),
        ..Default::default()
    }
}
