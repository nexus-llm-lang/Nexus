use crate::harness::compile::get_codegen_error;
use crate::harness::{exec_should_trap, try_compile};

#[test]
fn snapshot_codegen_error_unsupported_external() {
    let src = r#"
    import external "fake.wasm"
    external bad = "bad" : (val: i64) -> { x: i64 }
    let main = fn () -> unit do
        let x = bad(val: 42)
        return ()
    end
    "#;
    let err = get_codegen_error(src);
    insta::assert_snapshot!(err);
}

#[test]
fn codegen_main_non_unit_return_is_rejected() {
    let src = r#"
let main = fn () -> i64 do
    return 42
end
"#;
    let err = try_compile(src).unwrap_err();
    assert!(err.contains("main must return unit"), "got: {}", err);
}

#[test]
fn codegen_raise_compiles_and_traps() {
    let msg = exec_should_trap(
        r#"
exception Boom(i64)

let main = fn () -> unit throws { Exn } do
    let err = Boom(42)
    raise err
    return ()
end
"#,
    );
    assert!(!msg.is_empty(), "trap message should not be empty");
}

#[test]
fn codegen_exn_constructor_lowering() {
    let msg = exec_should_trap(
        r#"
let main = fn () -> unit throws { Exn } do
    raise RuntimeError(val: "test error")
    return ()
end
"#,
    );
    assert!(!msg.is_empty(), "trap message should not be empty");
}
