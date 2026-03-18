use crate::harness::exec;

#[test]
fn codegen_record_field_access() {
    exec(
        r#"
let main = fn () -> unit do
    let r = { y: 2, x: 40 }
    let v = r.x
    if v != 40 then raise RuntimeError(val: "expected 40") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_record_field_access_multiple() {
    exec(
        r#"
let main = fn () -> unit do
    let r = { a: 10, b: 32 }
    let x = r.a
    let y = r.b
    let result = x + y
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_record_field_access_then_arithmetic() {
    exec(
        r#"
let main = fn () -> unit do
    let r = { x: 20, y: 22 }
    let a = r.x
    let b = r.y
    let result = a + b
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_let_destructure_record() {
    exec(
        r#"
let main = fn () -> unit do
    let r = { x: 20, y: 22 }
    let {x: a, y: b} = r
    let result = a + b
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_let_destructure_record_multiple_uses() {
    exec(
        r#"
let main = fn () -> unit do
    let r = { a: 10, b: 20, c: 12 }
    let {a: x, b: y, c: z} = r
    let result = x + y + z
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}
