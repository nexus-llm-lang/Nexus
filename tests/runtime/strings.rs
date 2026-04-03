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
import { Console, system_handler } from "stdlib/stdio.nx"
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
import { Console, system_handler } from "stdlib/stdio.nx"
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

    // Post-bundle: wasm-merge should preserve the bytes
    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should succeed");
    let double_encoded: &[u8] = &[0xC3, 0xB0, 0xC2, 0x9F];
    assert!(
        !merged.windows(4).any(|w| w == double_encoded),
        "bundled WASM contains double-encoded UTF-8 — wasm-merge corrupted string data"
    );
    assert!(
        merged.windows(4).any(|w| w == correct_bytes),
        "bundled WASM should contain raw UTF-8 bytes after merge"
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
