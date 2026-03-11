use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn ast_construction_and_matching() {
    exec_with_stdlib(&read_fixture("test_ast.nx"));
}
