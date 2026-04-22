use crate::harness::{exec_with_stdlib, read_fixture};

/// Regression: Nexus-level wrapper functions (like bytebuffer.copy_range)
/// must survive DCE and be emitted with their canonical name when called
/// through a qualified import alias.
#[test]
fn transitive_wrapper_resolves() {
    exec_with_stdlib(&read_fixture("nxc/test_transitive_wrapper.nx"));
}
