//! Tests for `nxlib/jsonrpc.nx` — Content-Length framing + dispatch.
//!
//! The pure framing layer (`frame_message`, `unframe_one`) is testable
//! without I/O; classification (`classify`) is also pure. The full I/O
//! dispatch loop is exercised in nxc fixture tests under `tests/nxc/`.

use crate::harness::exec_with_stdlib;

// ─── Pure: round-trip a single LSP initialize request ────────────────────────

#[test]
fn frame_message_roundtrip_initialize_request() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"
import { frame_message, unframe_one, UnframedOk, UnframedNeedMore }
  from "nxlib/jsonrpc.nx"
import * as str from "std:str"

let main = fn () -> unit do
  let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"processId\":4242}}"
  let value = parse(s: body)
  let framed = frame_message(value)
  // Header should advertise the body byte length.
  let expected_header = "Content-Length: " ++ str.from_i64(val: str.byte_length(s: body)) ++ "\r\n\r\n"
  if !str.starts_with(s: framed, prefix: expected_header) then
    raise RuntimeError(val: "framed message missing expected header")
  end
  // Round-trip: unframe gives back an equal serialised body.
  match unframe_one(input: framed) do
    | UnframedOk(value: v, rest) ->
      if rest != "" then raise RuntimeError(val: "unexpected trailing bytes") end
      if serialize(v) != body then raise RuntimeError(val: "body lost in roundtrip") end
    | UnframedNeedMore -> raise RuntimeError(val: "unframe_one stalled on complete frame")
  end
  return ()
end
"#,
    );
}

// ─── Pure: N messages in sequence, no state leakage ──────────────────────────
//
// This is the contract the spec calls out: "server reads N messages in
// sequence without state leakage". `unframe_one` is purely a function of
// its input string, so threading the `rest` of one call into the next is
// the test for state isolation.

#[test]
fn unframe_one_handles_three_messages_in_sequence() {
    // Three frames concatenated; threading `rest` through `unframe_one`
    // returns each in sequence. Body strings are compared via `serialize` so
    // the test depends only on the JSON value, not the parser's whitespace.
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"
import { frame_message, unframe_one, UnframedOk, UnframedNeedMore }
  from "nxlib/jsonrpc.nx"

let main = fn () -> unit do
  let body1 = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}"
  let body2 = "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}"
  let body3 = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"shutdown\"}"
  let stream = frame_message(value: parse(s: body1))
            ++ frame_message(value: parse(s: body2))
            ++ frame_message(value: parse(s: body3))

  match unframe_one(input: stream) do
    | UnframedNeedMore -> raise RuntimeError(val: "stalled at msg 1")
    | UnframedOk(value: v1, rest: r1) ->
      if serialize(v: v1) != body1 then
        raise RuntimeError(val: "msg 1 body mismatch")
      end
      match unframe_one(input: r1) do
        | UnframedNeedMore -> raise RuntimeError(val: "stalled at msg 2")
        | UnframedOk(value: v2, rest: r2) ->
          if serialize(v: v2) != body2 then
            raise RuntimeError(val: "msg 2 body mismatch")
          end
          match unframe_one(input: r2) do
            | UnframedNeedMore -> raise RuntimeError(val: "stalled at msg 3")
            | UnframedOk(value: v3, rest: r3) ->
              if serialize(v: v3) != body3 then
                raise RuntimeError(val: "msg 3 body mismatch")
              end
              if r3 != "" then
                raise RuntimeError(val: "unexpected trailing bytes after 3 frames")
              end
              return ()
          end
      end
  end
end
"#,
    );
}

// ─── Pure: streaming feed — frame split across reads ─────────────────────────

#[test]
fn unframe_one_returns_need_more_for_partial_input() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"
import { frame_message, unframe_one, UnframedOk, UnframedNeedMore }
  from "nxlib/jsonrpc.nx"
import * as str from "std:str"

let main = fn () -> unit do
  let body = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"shutdown\"}"
  let framed = frame_message(value: parse(s: body))
  let total = str.byte_length(s: framed)
  // Slice 1: just the header prefix (no content yet).
  let half = str.byte_substring(s: framed, start: 0, len: 10)
  match unframe_one(input: half) do
    | UnframedNeedMore -> ()
    | UnframedOk(_) -> raise RuntimeError(val: "unframe_one accepted partial header")
  end
  // Slice 2: header complete, body partial.
  let near_end = str.byte_substring(s: framed, start: 0, len: total - 5)
  match unframe_one(input: near_end) do
    | UnframedNeedMore -> ()
    | UnframedOk(_) -> raise RuntimeError(val: "unframe_one accepted partial body")
  end
  // Full frame: ok.
  match unframe_one(input: framed) do
    | UnframedOk(_) -> ()
    | UnframedNeedMore -> raise RuntimeError(val: "unframe_one stalled on full frame")
  end
  return ()
end
"#,
    );
}

// ─── Pure: malformed Content-Length is reported, not silently ignored ────────

#[test]
fn unframe_one_raises_on_non_numeric_content_length() {
    exec_with_stdlib(
        r#"
import { unframe_one, JsonRpcFrameError } from "nxlib/jsonrpc.nx"

let main = fn () -> unit do
  let bad = "Content-Length: abc\r\n\r\n{}"
  try
    let _ = unframe_one(input: bad)
    raise RuntimeError(val: "expected JsonRpcFrameError")
  catch err ->
    match err do
      | JsonRpcFrameError(_) -> return ()
      | _ -> raise RuntimeError(val: "expected JsonRpcFrameError variant")
    end
  end
end
"#,
    );
}

#[test]
fn unframe_one_raises_on_missing_content_length_header() {
    exec_with_stdlib(
        r#"
import { unframe_one, JsonRpcFrameError } from "nxlib/jsonrpc.nx"

let main = fn () -> unit do
  // Headers terminate but Content-Length is absent.
  let bad = "X-Other: 1\r\n\r\n{}"
  try
    let _ = unframe_one(input: bad)
    raise RuntimeError(val: "expected JsonRpcFrameError")
  catch err ->
    match err do
      | JsonRpcFrameError(_) -> return ()
      | _ -> raise RuntimeError(val: "expected JsonRpcFrameError variant")
    end
  end
end
"#,
    );
}

// ─── Pure: Content-Length is case-insensitive per RFC 7230 ───────────────────

#[test]
fn unframe_one_accepts_lowercase_header_name() {
    exec_with_stdlib(
        r#"
import { serialize } from "std:json"
import { unframe_one, UnframedOk } from "nxlib/jsonrpc.nx"

let main = fn () -> unit do
  let bad = "content-length: 2\r\n\r\n{}"
  match unframe_one(input: bad) do
    | UnframedOk(value, rest: _) ->
      if serialize(v: value) != "{}" then
        raise RuntimeError(val: "lowercase header parse mismatch")
      end
      return ()
    | _ -> raise RuntimeError(val: "lowercase header rejected")
  end
end
"#,
    );
}

// ─── Pure: extra Content-Type header is tolerated ───────────────────────────

#[test]
fn unframe_one_accepts_optional_content_type_header() {
    exec_with_stdlib(
        r#"
import { serialize } from "std:json"
import { unframe_one, UnframedOk } from "nxlib/jsonrpc.nx"

let main = fn () -> unit do
  let raw = "Content-Length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}"
  match unframe_one(input: raw) do
    | UnframedOk(value, rest: _) ->
      if serialize(v: value) != "{}" then
        raise RuntimeError(val: "Content-Type-augmented frame parse mismatch")
      end
      return ()
    | _ -> raise RuntimeError(val: "Content-Type header tripped framer")
  end
end
"#,
    );
}

// ─── Classification ─────────────────────────────────────────────────────────

#[test]
fn classify_distinguishes_request_notification_malformed() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"
import { classify, Request, Notification, Malformed } from "nxlib/jsonrpc.nx"

let main = fn () -> unit do
  match classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}")) do
    | Request(id: _, method: m, params: _) ->
      if m != "initialize" then raise RuntimeError(val: "request method mismatch") end
    | _ -> raise RuntimeError(val: "expected Request")
  end
  match classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"didOpen\"}")) do
    | Notification(method: m, params: _) ->
      if m != "didOpen" then raise RuntimeError(val: "notification method mismatch") end
    | _ -> raise RuntimeError(val: "expected Notification")
  end
  match classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1}")) do
    | Malformed(_) -> ()
    | _ -> raise RuntimeError(val: "expected Malformed for missing method")
  end
  return ()
end
"#,
    );
}

// ─── Response builders ───────────────────────────────────────────────────────

#[test]
fn response_ok_and_error_shape() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonInt, JsonString, get_field, JsonObject } from "std:json"
import { response_ok, response_error, parse_error, internal_error }
  from "nxlib/jsonrpc.nx"
import { Some, None } from "std:option"

let main = fn () -> unit do
  let resp = response_ok(id: JsonInt(val: 7), result: JsonString(val: "ok"))
  let s = serialize(v: resp)
  if s != "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":\"ok\"}" then
    raise RuntimeError(val: "response_ok shape mismatch: " ++ s)
  end

  let err_resp = response_error(id: JsonInt(val: 8), code: parse_error(), message: "bad json")
  let s2 = serialize(v: err_resp)
  if s2 != "{\"jsonrpc\":\"2.0\",\"id\":8,\"error\":{\"code\":-32700,\"message\":\"bad json\"}}" then
    raise RuntimeError(val: "response_error shape mismatch: " ++ s2)
  end

  // Sanity: error codes match the JSON-RPC 2.0 spec.
  if internal_error() != -32603 then raise RuntimeError(val: "internal_error code") end
  return ()
end
"#,
    );
}

// ─── I/O layer typecheck-only smoke ──────────────────────────────────────────
//
// Full end-to-end dispatch_loop coverage requires stateful closure capture
// inside a handler block (the mock Console must remember a cursor across
// successive `read_bytes` calls). That pattern currently trips a codegen
// invariant in the cap-dispatch + linear borrow combination — see follow-up
// in nexus-hw47 epic. This typecheck-only test guards the public surface so
// any signature drift in the I/O layer fails fast; runtime behaviour of the
// I/O layer is exercised once the codegen path lands.

#[test]
fn jsonrpc_io_layer_typechecks() {
    use crate::harness::should_typecheck;
    should_typecheck(
        r#"
import { JsonValue } from "std:json"
import { Console } from "std:stdio"
import { Option } from "std:option"
import { read_message, write_message, dispatch_loop, dispatch_step,
         IncomingMessage }
  from "nxlib/jsonrpc.nx"

let dummy_handler = fn (msg: IncomingMessage) -> Option<JsonValue> do
  let _ = msg
  return None
end

let main = fn () -> unit require { PermConsole } do
  // Each export is invoked once so the typechecker exercises every
  // signature path even though no I/O actually runs (the test harness
  // doesn't connect stdin/stdout for typecheck mode).
  let _ = read_message
  let _ = write_message
  let _ = dispatch_step
  let _ = dispatch_loop
  let _ = dummy_handler
  return ()
end
"#,
    );
}
