use crate::common::wasm::{exec_with_stdlib, read_fixture};

#[test]
fn hashmap_put_get_or_and_contains_key() {
    let mut src = read_fixture("hashmap_put_get_or_and_contains_key.nx");
    src = convert_fixture_to_assert(&src, 126);
    exec_with_stdlib(&src);
}

#[test]
fn hashmap_get_lookup_and_remove() {
    let mut src = read_fixture("hashmap_get_lookup_and_remove.nx");
    src = convert_fixture_to_assert(&src, 71);
    exec_with_stdlib(&src);
}

fn convert_fixture_to_assert(src: &str, expected: i64) -> String {
    let converted = src.replace(
        "let main = fn () -> i64 do",
        "let __original_main = fn () -> i64 do",
    );
    format!(
        r#"{}

let main = fn () -> unit do
    let result = __original_main()
    if result != {} then raise RuntimeError(val: "unexpected result") end
    return ()
end
"#,
        converted, expected
    )
}
