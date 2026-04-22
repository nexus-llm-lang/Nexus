use crate::harness::{exec_with_stdlib, read_fixture};

/// Regression: nxc/driver.nx's main uses `inject ... do try ... catch -> ... end end`.
/// Under canonical naming the wasm emitter must produce a body valid for
/// the `(func)` signature of main (no return value).
#[test]
fn inject_try_catch_compiles() {
    exec_with_stdlib(&read_fixture("nxc/test_inject_try_catch.nx"));
}
