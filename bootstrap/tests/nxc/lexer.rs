use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn lexer_tokenize() {
    exec_with_stdlib(&read_fixture("nxc/test_lexer.nx"));
}
