//! Regression tests for `str.from_char_code` / `str.from_char` UTF-8 encoding
//! (issue nexus-9gsv).
//!
//! Pre-fix the codegen intrinsic stored `code as u8` and packed `len = 1`,
//! silently truncating non-ASCII codepoints (e.g. 0xE9 → invalid 1-byte string,
//! 0x1F600 → 1-byte garbage). Post-fix it must:
//!  - emit the correct 1-4 UTF-8 bytes
//!  - raise `Exn::InvalidUnicode(code)` for surrogates (0xD800-0xDFFF), code
//!    points > 0x10FFFF, and negative inputs
//!
//! Tests assert byte-level identity by comparing against
//! `\u{...}` literals (which the lexer already encodes as canonical UTF-8).

use crate::harness::exec_with_stdlib;

#[test]
fn from_char_code_two_byte_latin1() {
    // 0xE9 ('é') must encode as 0xC3 0xA9 (2 bytes), not 0xE9 (1 byte).
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length, byte_at } from "std:str"
import { ord } from "std:char"

let main = fn () -> unit do
  let s = from_char_code(code: 233)  // 0xE9 = 'é'
  if byte_length(s) != 2 then raise RuntimeError(val: "expected 2 bytes") end
  if ord(c: byte_at(s, idx: 0)) != 195 then raise RuntimeError(val: "byte0 mismatch") end
  if ord(c: byte_at(s, idx: 1)) != 169 then raise RuntimeError(val: "byte1 mismatch") end
  // Round-trip equality with \u{e9} literal.
  if s != "\u{e9}" then raise RuntimeError(val: "literal mismatch") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_code_three_byte_bmp() {
    // 0x6F22 ('漢') must encode as 0xE6 0xBC 0xA2 (3 bytes).
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length, byte_at } from "std:str"
import { ord } from "std:char"

let main = fn () -> unit do
  let s = from_char_code(code: 28450)  // 0x6F22
  if byte_length(s) != 3 then raise RuntimeError(val: "expected 3 bytes") end
  if ord(c: byte_at(s, idx: 0)) != 230 then raise RuntimeError(val: "byte0") end
  if ord(c: byte_at(s, idx: 1)) != 188 then raise RuntimeError(val: "byte1") end
  if ord(c: byte_at(s, idx: 2)) != 162 then raise RuntimeError(val: "byte2") end
  if s != "\u{6f22}" then raise RuntimeError(val: "literal mismatch") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_code_four_byte_supplementary() {
    // 0x1F600 (grinning face) must encode as 0xF0 0x9F 0x98 0x80 (4 bytes).
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length, byte_at } from "std:str"
import { ord } from "std:char"

let main = fn () -> unit do
  let s = from_char_code(code: 128512)  // 0x1F600
  if byte_length(s) != 4 then raise RuntimeError(val: "expected 4 bytes") end
  if ord(c: byte_at(s, idx: 0)) != 240 then raise RuntimeError(val: "byte0") end
  if ord(c: byte_at(s, idx: 1)) != 159 then raise RuntimeError(val: "byte1") end
  if ord(c: byte_at(s, idx: 2)) != 152 then raise RuntimeError(val: "byte2") end
  if ord(c: byte_at(s, idx: 3)) != 128 then raise RuntimeError(val: "byte3") end
  if s != "\u{1f600}" then raise RuntimeError(val: "literal mismatch") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_code_ascii_unchanged() {
    // 'A' = 0x41 still encodes as a single byte.
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length, byte_at } from "std:str"
import { ord } from "std:char"

let main = fn () -> unit do
  let s = from_char_code(code: 65)
  if byte_length(s) != 1 then raise RuntimeError(val: "expected 1 byte") end
  if ord(c: byte_at(s, idx: 0)) != 65 then raise RuntimeError(val: "byte0") end
  if s != "A" then raise RuntimeError(val: "literal mismatch") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_code_low_surrogate_raises_invalid_unicode() {
    // 0xD800 (low end of high-surrogate range) must raise InvalidUnicode,
    // not be silently encoded.
    exec_with_stdlib(
        r#"
import { from_char_code } from "std:str"

let main = fn () -> unit do
  try
    let _s = from_char_code(code: 55296)  // 0xD800
    raise RuntimeError(val: "expected InvalidUnicode for surrogate, got string")
  catch err ->
    match err do
      | InvalidUnicode(code: c) ->
          if c != 55296 then raise RuntimeError(val: "wrong codepoint reported") end
      | _ -> raise RuntimeError(val: "wrong exception variant")
    end
    return ()
  end
end
"#,
    );
}

#[test]
fn from_char_code_high_surrogate_raises_invalid_unicode() {
    // 0xDFFF (top end of low-surrogate range) must raise InvalidUnicode.
    exec_with_stdlib(
        r#"
import { from_char_code } from "std:str"

let main = fn () -> unit do
  try
    let _ = from_char_code(code: 57343)  // 0xDFFF
    raise RuntimeError(val: "expected InvalidUnicode for surrogate")
  catch
    | InvalidUnicode(code: c) ->
        if c != 57343 then raise RuntimeError(val: "wrong codepoint reported") end
        return ()
    | _ -> raise RuntimeError(val: "wrong exception variant")
  end
end
"#,
    );
}

#[test]
fn from_char_code_above_max_raises_invalid_unicode() {
    // 0x110000 is one above the Unicode upper bound.
    exec_with_stdlib(
        r#"
import { from_char_code } from "std:str"

let main = fn () -> unit do
  try
    let _ = from_char_code(code: 1114112)  // 0x110000
    raise RuntimeError(val: "expected InvalidUnicode for >0x10FFFF")
  catch
    | InvalidUnicode(code: c) ->
        if c != 1114112 then raise RuntimeError(val: "wrong codepoint reported") end
        return ()
    | _ -> raise RuntimeError(val: "wrong exception variant")
  end
end
"#,
    );
}

#[test]
fn from_char_code_negative_raises_invalid_unicode() {
    exec_with_stdlib(
        r#"
import { from_char_code } from "std:str"

let main = fn () -> unit do
  try
    let _ = from_char_code(code: -1)
    raise RuntimeError(val: "expected InvalidUnicode for negative")
  catch
    | InvalidUnicode(code: c) ->
        if c != -1 then raise RuntimeError(val: "wrong codepoint reported") end
        return ()
    | _ -> raise RuntimeError(val: "wrong exception variant")
  end
end
"#,
    );
}

#[test]
fn from_char_code_max_valid_codepoint() {
    // 0x10FFFF — the largest valid Unicode scalar — must succeed (4 bytes).
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length } from "std:str"

let main = fn () -> unit do
  let s = from_char_code(code: 1114111)  // 0x10FFFF
  if byte_length(s) != 4 then raise RuntimeError(val: "expected 4 bytes for 0x10FFFF") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_code_just_below_surrogate_succeeds() {
    // 0xD7FF — the codepoint right below the surrogate range — must encode (3 bytes).
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length } from "std:str"

let main = fn () -> unit do
  let s = from_char_code(code: 55295)  // 0xD7FF
  if byte_length(s) != 3 then raise RuntimeError(val: "expected 3 bytes for 0xD7FF") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_code_just_above_surrogate_succeeds() {
    // 0xE000 — the codepoint right above the surrogate range — must encode (3 bytes).
    exec_with_stdlib(
        r#"
import { from_char_code, byte_length } from "std:str"

let main = fn () -> unit do
  let s = from_char_code(code: 57344)  // 0xE000
  if byte_length(s) != 3 then raise RuntimeError(val: "expected 3 bytes for 0xE000") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_two_byte() {
    // `from_char` (char-typed input) shares the encoder with `from_char_code`.
    exec_with_stdlib(
        r#"
import { from_char, byte_length } from "std:str"

let main = fn () -> unit do
  let s = from_char(c: '\u{e9}')
  if byte_length(s) != 2 then raise RuntimeError(val: "expected 2 bytes for 'é'") end
  if s != "\u{e9}" then raise RuntimeError(val: "literal mismatch") end
  return ()
end
"#,
    );
}

#[test]
fn from_char_four_byte() {
    exec_with_stdlib(
        r#"
import { from_char, byte_length } from "std:str"

let main = fn () -> unit do
  let s = from_char(c: '\u{1f600}')
  if byte_length(s) != 4 then raise RuntimeError(val: "expected 4 bytes for grinning") end
  if s != "\u{1f600}" then raise RuntimeError(val: "literal mismatch") end
  return ()
end
"#,
    );
}

// ─── Audit (acceptance #5) — read-side ASCII-only assumptions ────────────────
//
// Before this fix, `Intrinsic::CharCode`, `Intrinsic::CharAt`, and
// `Intrinsic::StringLength` short-circuited to byte semantics
// (`I32Load8U(ptr+idx)` and `packed & 0xFFFFFFFF`). The runtime extern
// (`__nx_string_*` in bootstrap/src/lib/string/src/lib.rs) instead computes
// the WIT-declared character semantics via `s.chars()`.  These three
// intrinsics are now de-registered so the compiler routes through the
// runtime extern. The tests below assert the corrected behaviour for
// non-ASCII inputs that previously diverged.

#[test]
fn audit_string_length_counts_characters_not_bytes() {
    // 'é' (U+00E9) takes 2 UTF-8 bytes — `length` must report 1.
    exec_with_stdlib(
        r#"
import { length } from "std:str"

let main = fn () -> unit do
  if length(s: "\u{e9}") != 1 then raise RuntimeError(val: "length('é') wrong") end
  if length(s: "a\u{e9}b") != 3 then raise RuntimeError(val: "length('aéb') wrong") end
  if length(s: "\u{1f600}") != 1 then raise RuntimeError(val: "length(emoji) wrong") end
  return ()
end
"#,
    );
}

#[test]
fn audit_char_at_returns_codepoint_not_byte() {
    // char_at on multi-byte string must walk by character index, not byte.
    exec_with_stdlib(
        r#"
import { char_at, char_code } from "std:str"
import { ord } from "std:char"

let main = fn () -> unit do
  let s = "a\u{e9}b"
  if ord(c: char_at(s, idx: 0)) != 97 then raise RuntimeError(val: "char_at[0]") end
  if ord(c: char_at(s, idx: 1)) != 233 then raise RuntimeError(val: "char_at[1]") end
  if ord(c: char_at(s, idx: 2)) != 98 then raise RuntimeError(val: "char_at[2]") end
  if char_code(s, idx: 1) != 233 then raise RuntimeError(val: "char_code[1]") end
  return ()
end
"#,
    );
}
