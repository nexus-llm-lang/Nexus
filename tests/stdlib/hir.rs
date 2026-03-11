use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn hir_build_and_reachability() {
    exec_with_stdlib(&read_fixture("test_hir.nx"));
}
