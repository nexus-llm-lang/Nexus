use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn ast_construction_and_matching() {
    exec_with_stdlib(&read_fixture("nxc/test_ast.nx"));
}
