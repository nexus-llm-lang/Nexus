use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn typecheck_core_infrastructure() {
    exec_with_stdlib(&read_fixture("nxc/test_typecheck.nx"));
}
