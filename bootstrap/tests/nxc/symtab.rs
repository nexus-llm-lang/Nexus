use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn symtab_scope_and_lookup() {
    exec_with_stdlib(&read_fixture("nxc/test_symtab.nx"));
}
