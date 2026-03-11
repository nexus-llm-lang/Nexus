use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn char_classification() {
    exec_with_stdlib(&read_fixture("test_char_classification.nx"));
}
