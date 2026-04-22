use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn qualified_imports_diamond_and_mixed() {
    exec_with_stdlib(&read_fixture("nxc/test_qualified_imports.nx"));
}
