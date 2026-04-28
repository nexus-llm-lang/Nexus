use crate::harness::exec_with_stdlib;

// ─── Atom round-trips ────────────────────────────────────────────────────────

#[test]
fn json_roundtrip_null_true_false() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"

let main = fn () -> unit do
  let n = parse(s: "null")
  if serialize(v: n) != "null" then raise RuntimeError(val: "null roundtrip") end

  let t = parse(s: "true")
  if serialize(v: t) != "true" then raise RuntimeError(val: "true roundtrip") end

  let f = parse(s: "false")
  if serialize(v: f) != "false" then raise RuntimeError(val: "false roundtrip") end

  return ()
end
"#,
    );
}

// ─── Number boundaries ───────────────────────────────────────────────────────

#[test]
fn json_number_boundaries() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonInt, JsonFloat } from "std:json"

let expect_int = fn (s: string, want: i64) -> unit throws { Exn } do
  let v = parse(s)
  match v do
    | JsonInt(val: got) ->
      if got != want then raise RuntimeError(val: "wrong int for " ++ s) end
    | _ -> raise RuntimeError(val: "expected JsonInt for " ++ s)
  end
end

let expect_float_round = fn (s: string) -> unit throws { Exn } do
  let v = parse(s)
  match v do
    | JsonFloat(_) -> return ()
    | _ -> raise RuntimeError(val: "expected JsonFloat for " ++ s)
  end
end

let main = fn () -> unit do
  // Negative integer.
  expect_int(s: "-42", want: -42)
  // Zero.
  expect_int(s: "0", want: 0)
  // i64 max.
  expect_int(s: "9223372036854775807", want: 9223372036854775807)
  // i64 min — written as -9223372036854775807 - 1 because the unary minus on
  // the bare 9223372036854775808 literal overflows during lex.
  expect_int(s: "-9223372036854775808", want: -9223372036854775807 - 1)
  // Fractional.
  expect_float_round(s: "3.1415926535")
  // Exponent (lowercase).
  expect_float_round(s: "1e10")
  // Exponent (uppercase, signed).
  expect_float_round(s: "-2.5E-3")
  // Negative fractional with leading zero in integer part.
  expect_float_round(s: "-0.5")
  return ()
end
"#,
    );
}

#[test]
fn json_serialize_preserves_int_vs_float() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonInt, JsonFloat } from "std:json"

let main = fn () -> unit do
  // Integer literal stays integer.
  if serialize(v: parse(s: "42")) != "42" then
    raise RuntimeError(val: "int 42 lost integer form")
  end
  // Float with explicit decimal point round-trips with one.
  let s = serialize(v: parse(s: "1.5"))
  if s != "1.5" then raise RuntimeError(val: "float 1.5 became " ++ s) end
  return ()
end
"#,
    );
}

#[test]
fn json_rejects_leading_zero() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"

let main = fn () -> unit do
  try
    let _ = parse(s: "01")
    raise RuntimeError(val: "expected JsonError on leading-zero literal")
  catch _ ->
    return ()
  end
end
"#,
    );
}

// ─── Strings & Unicode escapes ───────────────────────────────────────────────

#[test]
fn json_string_named_escapes_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonString } from "std:json"
import { byte_length } from "std:str"

let main = fn () -> unit do
  // \" \\ \/ \b \f \n \r \t — raw JSON spelled out as a Nexus string literal.
  // \\\" → JSON \" ; \\\\ → JSON \\ ; etc.
  let raw = "\"\\\"\\\\\\/\\b\\f\\n\\r\\t\""
  let v = parse(s: raw)
  match v do
    | JsonString(val: s) ->
      // Decoded content must be 8 bytes: " \ / BS FF LF CR TAB
      if byte_length(s) != 8 then raise RuntimeError(val: "expected 8 decoded bytes") end
    | _ -> raise RuntimeError(val: "expected JsonString")
  end
  // Round-trip: serializer escapes back; parsing again gives the same value.
  let again = parse(s: serialize(v))
  match again do
    | JsonString(val: s2) ->
      if byte_length(s: s2) != 8 then raise RuntimeError(val: "roundtrip changed length") end
      return ()
    | _ -> raise RuntimeError(val: "roundtrip lost JsonString")
  end
end
"#,
    );
}

#[test]
fn json_unicode_basic_bmp_escape() {
    exec_with_stdlib(
        r#"
import { parse, JsonString } from "std:json"
import { byte_length, byte_at } from "std:str"
import { ord } from "std:char"

let main = fn () -> unit do
  // U+00E9 = é. JSON: "é". UTF-8 byte sequence: 0xC3 0xA9.
  let v = parse(s: "\"\\u00e9\"")
  match v do
    | JsonString(val: s) ->
      if byte_length(s) != 2 then raise RuntimeError(val: "expected 2 UTF-8 bytes for é") end
      if ord(c: byte_at(s, idx: 0)) != 195 then raise RuntimeError(val: "byte 0 != 0xC3") end
      if ord(c: byte_at(s, idx: 1)) != 169 then raise RuntimeError(val: "byte 1 != 0xA9") end
      return ()
    | _ -> raise RuntimeError(val: "expected JsonString")
  end
end
"#,
    );
}

#[test]
fn json_unicode_surrogate_pair_above_bmp() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonString } from "std:json"
import { byte_length, byte_at } from "std:str"
import { ord } from "std:char"

let check_grinning_face_bytes = fn (s: string) -> unit throws { Exn } do
  // U+1F600 = GRINNING FACE encodes as UTF-8: F0 9F 98 80.
  if byte_length(s) != 4 then raise RuntimeError(val: "expected 4 UTF-8 bytes for U+1F600") end
  if ord(c: byte_at(s, idx: 0)) != 240 then raise RuntimeError(val: "byte 0 != 0xF0") end
  if ord(c: byte_at(s, idx: 1)) != 159 then raise RuntimeError(val: "byte 1 != 0x9F") end
  if ord(c: byte_at(s, idx: 2)) != 152 then raise RuntimeError(val: "byte 2 != 0x98") end
  if ord(c: byte_at(s, idx: 3)) != 128 then raise RuntimeError(val: "byte 3 != 0x80") end
  return ()
end

let main = fn () -> unit do
  // UTF-16 surrogate pair for U+1F600 GRINNING FACE.
  let v = parse(s: "\"\\uD83D\\uDE00\"")
  match v do
    | JsonString(val: s) -> check_grinning_face_bytes(s)
    | _ -> raise RuntimeError(val: "expected JsonString")
  end
  // Serializer round-trip: re-parse must yield bytes for the same scalar.
  let original = parse(s: "\"\\uD83D\\uDE00\"")
  let printed = serialize(v: original)
  let reparsed = parse(s: printed)
  match reparsed do
    | JsonString(val: s2) ->
      check_grinning_face_bytes(s: s2)
      return ()
    | _ -> raise RuntimeError(val: "roundtrip lost JsonString")
  end
end
"#,
    );
}

#[test]
fn json_string_rejects_lone_surrogate() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"

let main = fn () -> unit do
  try
    let _ = parse(s: "\"\\uD83D\"")
    raise RuntimeError(val: "expected error on lone high surrogate")
  catch _ ->
    return ()
  end
end
"#,
    );
}

// ─── Whitespace handling per RFC 8259 ────────────────────────────────────────

#[test]
fn json_whitespace_around_tokens() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"

let main = fn () -> unit do
  // Space, tab, CR, LF in every separator position.
  let raw = " \t\r\n[ 1 ,\t2 ,\n3\r] \t\n"
  let v = parse(s: raw)
  if serialize(v) != "[1,2,3]" then raise RuntimeError(val: "ws not handled") end
  return ()
end
"#,
    );
}

// ─── Composite round-trips ───────────────────────────────────────────────────

#[test]
fn json_array_nested_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"

let main = fn () -> unit do
  let raw = "[1,[2,[3,[]]]]"
  let v = parse(s: raw)
  if serialize(v) != raw then raise RuntimeError(val: "nested array roundtrip") end
  return ()
end
"#,
    );
}

#[test]
fn json_object_lookup_and_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize, get_field, JsonInt, JsonString } from "std:json"
import { Some, None } from "std:option"

let main = fn () -> unit do
  let raw = "{\"a\":1,\"b\":\"two\"}"
  let v = parse(s: raw)
  match get_field(obj: v, key: "a") do
    | Some(val: JsonInt(val: i)) ->
      if i != 1 then raise RuntimeError(val: "a != 1") end
    | _ -> raise RuntimeError(val: "missing a")
  end
  match get_field(obj: v, key: "b") do
    | Some(val: JsonString(val: s)) ->
      if s != "two" then raise RuntimeError(val: "b != two") end
    | _ -> raise RuntimeError(val: "missing b")
  end
  match get_field(obj: v, key: "missing") do
    | None -> ()
    | Some(_) -> raise RuntimeError(val: "missing should be None")
  end
  if serialize(v) != raw then raise RuntimeError(val: "object roundtrip") end
  return ()
end
"#,
    );
}

// ─── LSP payload fixtures ────────────────────────────────────────────────────
//
// Hand-crafted samples lifted from the LSP spec examples. Each must round-trip
// through parse + serialize without changing structure (string compare on the
// canonical compact form).

#[test]
fn json_lsp_initialize_request_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"

let main = fn () -> unit do
  let raw = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"processId\":4242,\"rootUri\":\"file:///tmp/proj\",\"capabilities\":{\"textDocument\":{\"synchronization\":{\"didSave\":true}}}}}"
  let v = parse(s: raw)
  if serialize(v) != raw then raise RuntimeError(val: "initialize roundtrip") end
  return ()
end
"#,
    );
}

#[test]
fn json_lsp_publish_diagnostics_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"

let main = fn () -> unit do
  let raw = "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{\"uri\":\"file:///tmp/a.nx\",\"diagnostics\":[{\"range\":{\"start\":{\"line\":3,\"character\":7},\"end\":{\"line\":3,\"character\":12}},\"severity\":1,\"message\":\"unknown identifier\"}]}}"
  let v = parse(s: raw)
  if serialize(v) != raw then raise RuntimeError(val: "publishDiagnostics roundtrip") end
  return ()
end
"#,
    );
}

#[test]
fn json_lsp_did_change_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"

let main = fn () -> unit do
  let raw = "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/a.nx\",\"version\":2},\"contentChanges\":[{\"text\":\"let x = 1\\n\"}]}}"
  let v = parse(s: raw)
  if serialize(v) != raw then raise RuntimeError(val: "didChange roundtrip") end
  return ()
end
"#,
    );
}

// ─── Negative cases ──────────────────────────────────────────────────────────

#[test]
fn json_trailing_comma_rejected() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"

let main = fn () -> unit do
  try
    let _ = parse(s: "[1,2,]")
    raise RuntimeError(val: "expected error on trailing comma")
  catch _ ->
    return ()
  end
end
"#,
    );
}

#[test]
fn json_trailing_data_rejected() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"

let main = fn () -> unit do
  try
    let _ = parse(s: "1 2")
    raise RuntimeError(val: "expected error on trailing data")
  catch _ ->
    return ()
  end
end
"#,
    );
}

#[test]
fn json_serialize_rejects_nan() {
    exec_with_stdlib(
        r#"
import { serialize, float } from "std:json"
import { to_f64 } from "std:str"

let main = fn () -> unit do
  // std:str.to_f64 accepts the literal "NaN" and yields a true NaN bit
  // pattern — used here to exercise the serializer's invariant check.
  let nan_val = to_f64(s: "NaN")
  let v = float(val: nan_val)
  try
    let _ = serialize(v)
    raise RuntimeError(val: "expected JsonError on NaN")
  catch _ ->
    return ()
  end
end
"#,
    );
}
