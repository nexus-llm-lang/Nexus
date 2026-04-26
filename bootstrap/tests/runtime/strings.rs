use crate::harness::{compile, exec};

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
fn codegen_utf8_in_data_segment() {
    // Use a program where the string is actually used (not optimized away)
    let wasm = compile(
        r#"
import { Console, system_handler } from "std:stdio"
let main = fn () -> unit require { PermConsole } do
    inject system_handler do
    Console.println(val: "👽️ こんにちは, world!")
    end
end
"#,
    );
    // The emoji 👽 is U+1F47D = UTF-8 bytes F0 9F 91 BD
    let correct_bytes: &[u8] = &[0xF0, 0x9F, 0x91, 0xBD];
    let double_encoded: &[u8] = &[0xC3, 0xB0, 0xC2, 0x9F];
    assert!(
        !wasm.windows(4).any(|w| w == double_encoded),
        "codegen WASM contains double-encoded UTF-8 — string literal bytes were corrupted"
    );
    assert!(
        wasm.windows(4).any(|w| w == correct_bytes),
        "codegen WASM should contain raw UTF-8 bytes for emoji (F0 9F 91 BD)"
    );
}

#[test]
fn codegen_utf8_survives_bundling() {
    let wasm = compile(
        r#"
import { Console, system_handler } from "std:stdio"
let main = fn () -> unit require { PermConsole } do
    inject system_handler do
    Console.println(val: "👽️ こんにちは, world!")
    end
end
"#,
    );
    // Pre-bundle: codegen should have correct bytes
    let correct_bytes: &[u8] = &[0xF0, 0x9F, 0x91, 0xBD];
    assert!(
        wasm.windows(4).any(|w| w == correct_bytes),
        "pre-bundle WASM should contain raw UTF-8 bytes"
    );

    // Post-compose: component composition should preserve the bytes
    let composed = nexus::compiler::compose::compose_with_stdlib(&wasm)
        .expect("compose_with_stdlib should succeed");
    let double_encoded: &[u8] = &[0xC3, 0xB0, 0xC2, 0x9F];
    assert!(
        !composed.windows(4).any(|w| w == double_encoded),
        "composed WASM contains double-encoded UTF-8 — composition corrupted string data"
    );
    assert!(
        composed.windows(4).any(|w| w == correct_bytes),
        "composed WASM should contain raw UTF-8 bytes after composition"
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

// ---------------------------------------------------------------------------
// Diagnostic: Global 0 / Global 2 heap collision investigation (qfcu)
// ---------------------------------------------------------------------------

/// In bump-allocator mode (no stdlib), Global 0 (object heap) and Global 2
/// (string heap) are both initialized to heap_base. If a program creates a
/// constructor (bumping G0) and then does string concat (bumping G2), the
/// string bytes overwrite the constructor's tag at heap_base.
///
/// This test allocates a 2-variant constructor then immediately concats a
/// string. If the heaps collide, the constructor tag is corrupted and the
/// match will either take the wrong branch or trap with unreachable.
#[test]
fn diag_heap_g0_g2_collision_corrupts_constructor_tag() {
    // Regression test: G0 (object heap) and G2 (string heap) must start at
    // different addresses. String concat must not overwrite constructor tags.
    exec(
        r#"
type Val = Present(v: s64) | Absent

let use_string = fn (s: string) -> unit do
    if s == "" then raise RuntimeError(val: "empty") end
    return ()
end

let main = fn () -> unit do
    let x = Present(v: 42)
    let s = "aa" ++ "bb"
    use_string(s: s)
    match x do
        | Present(v: v) ->
            if v != 42 then
                raise RuntimeError(val: "COLLISION: field value corrupted")
            end
        | Absent ->
            raise RuntimeError(val: "COLLISION: tag flipped to Absent")
    end
    return ()
end
"#,
    );
}

/// All-non-nullary variant: forces tag-based matching (no pointer comparison
/// shortcut for nullary ctors in data section).
/// Also dumps the WASM globals for inspection.
#[test]
fn diag_heap_g0_g2_collision_all_non_nullary() {
    // Regression test: all non-nullary variants, forces tag-based matching.
    exec(
        r#"
type Thing = Alpha(x: s64) | Beta(x: s64) | Gamma(x: s64)

let use_string = fn (s: string) -> unit do
    if s == "" then raise RuntimeError(val: "empty") end
    return ()
end

let main = fn () -> unit do
    let t = Alpha(x: 42)
    let s = "aaaa" ++ "bbbb"
    use_string(s: s)
    match t do
        | Alpha(x: v) ->
            if v != 42 then raise RuntimeError(val: "Alpha.x corrupted") end
        | Beta(x: _) -> raise RuntimeError(val: "tag became Beta")
        | Gamma(x: _) -> raise RuntimeError(val: "tag became Gamma")
    end
    return ()
end
"#,
    );
}

/// Stress: allocate constructor, then do a LARGE string concat that
/// overwrites well beyond the constructor's 16 bytes.
/// Uses exec_should_trap to capture the error message.
#[test]
fn diag_heap_g0_g2_collision_large_string() {
    // Regression test: large string concat must not overwrite constructor fields.
    exec(
        r#"
type Wrap = Wrap(a: s64, b: s64, c: s64)

let build_long = fn (s: string, n: i64) -> string do
    if n <= 0 then return s end
    return build_long(s: s ++ "x", n: n - 1)
end

let main = fn () -> unit do
    let w = Wrap(a: 11, b: 22, c: 33)
    let s = build_long(s: "", n: 100)
    match w do
        | Wrap(a: a, b: b, c: c) ->
            if a != 11 then raise RuntimeError(val: "a") end
            if b != 22 then raise RuntimeError(val: "b") end
            if c != 33 then raise RuntimeError(val: "c") end
    end
    return ()
end
"#,
    );
}
