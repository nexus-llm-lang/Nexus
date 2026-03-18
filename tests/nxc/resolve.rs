use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn resolve_name_resolution() {
    exec_with_stdlib(&read_fixture("nxc/test_resolve.nx"));
}
