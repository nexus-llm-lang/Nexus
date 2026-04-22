use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn mir_lowering_and_port_resolution() {
    exec_with_stdlib(&read_fixture("nxc/test_mir.nx"));
}
