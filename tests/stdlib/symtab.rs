use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn symtab_scope_and_lookup() {
    exec_with_stdlib(&read_fixture("test_symtab.nx"));
}
