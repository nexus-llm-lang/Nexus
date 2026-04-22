use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn rdrname_resolver_prototype() {
    exec_with_stdlib(&read_fixture("nxc/test_rdrname.nx"));
}
