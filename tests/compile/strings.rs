use crate::common::wasm::exec;

#[test]
fn codegen_string_return_is_supported() {
    exec(
        r#"
let main = fn () -> unit do
    let _s = "hello"
    return ()
end
"#,
    );
}

#[test]
fn codegen_string_concat_operator_is_supported() {
    exec(
        r#"
let main = fn () -> unit do
    let _msg = "foo" ++ "bar"
    return ()
end
"#,
    );
}

#[test]
fn codegen_string_eq() {
    exec(
        r#"
let main = fn () -> unit do
    if "hello" == "hello" then () else raise RuntimeError("equal strings should be ==") end
    if "hello" == "world" then raise RuntimeError("different strings should not be ==") else () end
    if "" == "" then () else raise RuntimeError("empty strings should be ==") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_string_ne() {
    exec(
        r#"
let main = fn () -> unit do
    if "hello" != "world" then () else raise RuntimeError("different strings should be !=") end
    if "hello" != "hello" then raise RuntimeError("equal strings should not be !=") else () end
    if "abc" != "abd" then () else raise RuntimeError("strings differing in last byte should be !=") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_string_eq_with_concat() {
    exec(
        r#"
let main = fn () -> unit do
    let a = "foo" ++ "bar"
    if a == "foobar" then () else raise RuntimeError("concat result should equal literal") end
    return ()
end
"#,
    );
}
