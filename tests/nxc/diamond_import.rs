use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn diamond_import_forwarder_uses_orig_entries() {
    exec_with_stdlib(&read_fixture("nxc/test_diamond_import.nx"));
}
