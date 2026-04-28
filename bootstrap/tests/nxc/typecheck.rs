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
