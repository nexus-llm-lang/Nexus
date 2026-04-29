//! Smoke tests for `std:test/{assert,property,snapshot}` — wires the
//! fixture files in `bootstrap/tests/fixtures/test_test_lib_*.nx`
//! through the existing `exec_with_stdlib` harness so any regression in
//! the test library is observable from `cargo test --release`.

use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn test_lib_assert_smoke() {
    let src = read_fixture("test_test_lib_assert.nx");
    exec_with_stdlib(&src);
}

#[test]
fn test_lib_property_smoke() {
    let src = read_fixture("test_test_lib_property.nx");
    exec_with_stdlib(&src);
}

#[test]
fn test_lib_snapshot_smoke() {
    let src = read_fixture("test_test_lib_snapshot.nx");
    exec_with_stdlib(&src);
}
