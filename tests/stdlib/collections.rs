use crate::harness::{exec_with_stdlib, read_fixture};

#[test]
fn hashmap_put_get_or_and_contains_key() {
    exec_with_stdlib(&read_fixture("hashmap_put_get_or_and_contains_key.nx"));
}

#[test]
fn hashmap_get_lookup_and_remove() {
    exec_with_stdlib(&read_fixture("hashmap_get_lookup_and_remove.nx"));
}

#[test]
fn stringmap_put_get_and_contains() {
    exec_with_stdlib(&read_fixture("stringmap_put_get_and_contains.nx"));
}

#[test]
fn stringmap_get_lookup_and_remove() {
    exec_with_stdlib(&read_fixture("stringmap_get_lookup_and_remove.nx"));
}

#[test]
fn stringmap_keys_and_values() {
    exec_with_stdlib(&read_fixture("stringmap_keys_and_values.nx"));
}
