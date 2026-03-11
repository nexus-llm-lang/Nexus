use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn resolve_name_resolution() {
    exec_with_stdlib(&read_fixture("test_resolve.nx"));
}
