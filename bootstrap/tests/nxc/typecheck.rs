use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn typecheck_core_infrastructure() {
    exec_with_stdlib(&read_fixture("nxc/test_typecheck.nx"));
}

#[test]
fn infer_let_annotation_mismatch_raises() {
    exec_with_stdlib(&read_fixture("nxc/test_infer_let_annotation.nx"));
}

#[test]
fn lambda_capture_linearity() {
    exec_with_stdlib(&read_fixture("nxc/test_lambda_capture_linearity.nx"));
}

#[test]
fn handler_first_class() {
    exec_with_stdlib(&read_fixture("nxc/test_handler_first_class.nx"));
}

#[test]
fn match_exhaustiveness() {
    exec_with_stdlib(&read_fixture("nxc/test_match_exhaustiveness.nx"));
}

#[test]
fn throws_row_narrowing() {
    exec_with_stdlib(&read_fixture("nxc/test_throws_row_narrowing.nx"));
}

#[test]
fn call_throw_row_subsumption() {
    exec_with_stdlib(&read_fixture("nxc/test_call_throw_row_subsumption.nx"));
}

/// Covers nexus-hw47.3 (HIR span fidelity for synthesised Cons / Assign-target
/// nodes) and nexus-hw47.4 (LSP-style enumerate_diagnostics + type_at /
/// defining_position stubs).
#[test]
fn lsp_diagnostics_and_span_fidelity() {
    exec_with_stdlib(&read_fixture("nxc/test_lsp_diagnostics.nx"));
}

/// Covers nexus-hw47.9 (typecheck → publishDiagnostics): drives the LSP
/// scaffold via `drive_messages` and asserts the wire-format
/// publishDiagnostics frames produced from didOpen/didChange.
#[test]
fn lsp_publish_diagnostics_wire_format() {
    exec_with_stdlib(&read_fixture("nxc/test_lsp_publish_diagnostics.nx"));
}

/// Covers nexus-hw47.10 (DocumentSymbol tree): exercises the AST →
/// DocumentSymbol[] walker plus the textDocument/documentSymbol handler
/// dispatched through the LSP scaffold.
#[test]
fn lsp_document_symbols() {
    exec_with_stdlib(&read_fixture("nxc/test_lsp_document_symbols.nx"));
}

/// Covers nexus-avdp (multi-file import-aware diagnostics + per-URI cache):
/// the Handlers vtable advertises `require { Fs, Console }`, the importer's
/// publishDiagnostics frame inherits TypeMismatch from a buggy imported
/// module read off disk via the `Fs` cap, and same-(uri, version) replays
/// hit the diagnostic cache instead of re-running the typecheck pipeline.
#[test]
fn lsp_multifile_imports_fs_cap_and_cache() {
    exec_with_stdlib(&read_fixture("nxc/test_lsp_multifile_imports.nx"));
}
