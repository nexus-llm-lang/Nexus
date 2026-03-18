use crate::harness::{compile, exec_with_stdlib, read_fixture};

#[test]
fn parser_parse() {
    exec_with_stdlib(&read_fixture("nxc/test_parser.nx"));
}

#[test]
fn parser_parse_minimal() {
    exec_with_stdlib(&read_fixture("nxc/test_parser_minimal.nx"));
}

#[test]
fn parser_tokenize_only() {
    exec_with_stdlib(&read_fixture("nxc/test_parser_tokenize_only.nx"));
}
