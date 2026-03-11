use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn lexer_tokenize() {
    exec_with_stdlib(&read_fixture("test_lexer.nx"));
}
