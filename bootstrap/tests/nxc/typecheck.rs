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

/// Covers nexus-hw47.3 (HIR span fidelity for synthesised Cons / Assign-target
/// nodes) and nexus-hw47.4 (LSP-style enumerate_diagnostics + type_at /
/// defining_position stubs).
#[test]
fn lsp_diagnostics_and_span_fidelity() {
    exec_with_stdlib(&read_fixture("nxc/test_lsp_diagnostics.nx"));
}
