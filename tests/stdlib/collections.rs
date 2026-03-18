use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn hashmap_put_get_or_and_contains_key() {
    exec_with_stdlib(&read_fixture("hashmap_put_get_or_and_contains_key.nx"));
}

#[test]
fn hashmap_get_lookup_and_remove() {
    exec_with_stdlib(&read_fixture("hashmap_get_lookup_and_remove.nx"));
}
