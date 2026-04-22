use crate::harness::{exec_with_stdlib, read_fixture};

/// Regression test: canonical naming must work when a function body uses an
/// imported symbol that is declared BELOW its usage in the source file.
#[test]
fn import_after_use_resolves() {
    exec_with_stdlib(&read_fixture("nxc/test_import_after_use.nx"));
}
