use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn hir_build_and_reachability() {
    exec_with_stdlib(&read_fixture("nxc/test_hir.nx"));
}
