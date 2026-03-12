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
fn parser_dump_wasm() {
    let wasm = compile(&read_fixture("nxc/test_parser.nx"));
    std::fs::write("/tmp/parser_test.wasm", &wasm).unwrap();
    eprintln!("Wrote {} bytes to /tmp/parser_test.wasm", wasm.len());
}
