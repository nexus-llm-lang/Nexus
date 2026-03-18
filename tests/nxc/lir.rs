use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn lir_lowering_and_anf_conversion() {
    exec_with_stdlib(&read_fixture("nxc/test_lir.nx"));
}

#[test]
fn lir_minimal_import() {
    exec_with_stdlib(&read_fixture("nxc/test_lir_minimal.nx"));
}

#[test]
fn lir_conc_codegen() {
    exec_with_stdlib(&read_fixture("nxc/test_conc_codegen.nx"));
}
