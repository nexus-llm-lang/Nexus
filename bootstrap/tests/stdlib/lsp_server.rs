//! Tests for `nxlib/lsp/server.nx` — generic LSP server scaffold.
//!
//! The pure layer (`dispatch_message`, `drive_messages`) is testable
//! without I/O. The full I/O layer (`run_server`) is exercised via a
//! typecheck-only smoke test that pins the public signature; runtime
//! coverage of the I/O path requires stateful Console mocking which
//! the harness can't yet supply (same constraint as jsonrpc.rs's
//! `jsonrpc_io_layer_typechecks`).

use crate::harness::exec_with_stdlib;

// ─── Sequential lifecycle: initialize → did_open → did_change → did_close ────
//                            → shutdown → exit
//
// Acceptance criterion 1: stub handlers respond correctly to the full
// canonical LSP lifecycle in a single drive. We verify each step's
// observable side effect — response shape for requests, document buffer
// state for notifications — instead of merely asserting no trap.

#[test]
fn full_lifecycle_drives_six_messages_in_order() {
    exec_with_stdlib(
        r#"
import { parse, JsonNull, JsonInt, get_field } from "std:json"
import { Some, None } from "std:option"
import { classify } from "nxlib/jsonrpc.nx"
import { empty_state, empty_handlers, drive_messages, DriveResult,
         ServerState, LifecycleShutdown, documents_size }
  from "nxlib/lsp/server.nx"

let main = fn () -> unit do
  // Build the canonical six-message lifecycle. Each is parsed from a JSON
  // string then classified through the jsonrpc layer — that mirrors the
  // shape the I/O loop produces from real wire bytes.
  let m_init = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"processId\":4242,\"rootUri\":\"file:///tmp\",\"capabilities\":{}}}"))
  let m_open = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/a.nx\",\"languageId\":\"nexus\",\"version\":1,\"text\":\"let x = 1\"}}}"))
  let m_change = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/a.nx\",\"version\":2},\"contentChanges\":[{\"text\":\"let x = 42\"}]}}"))
  let m_close = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didClose\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/a.nx\"}}}"))
  let m_shutdown = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"shutdown\"}"))
  let m_exit = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}"))

  let msgs = [m_init, m_open, m_change, m_close, m_shutdown, m_exit]
  let result = drive_messages(state: empty_state(), msgs, handlers: empty_handlers())
  match result do | DriveResult(state, responses, notifications: _, exited) ->
    // exit notification must terminate the loop.
    if !exited then raise RuntimeError(val: "drive did not observe Exit") end
    // Final lifecycle is Shutdown (didn't get past Shutdown when exit fired).
    match state do | ServerState(lifecycle, documents, user_data: _) ->
      match lifecycle do
        | LifecycleShutdown -> ()
        | _ -> raise RuntimeError(val: "expected lifecycle = Shutdown after shutdown+exit")
      end
      // didClose removed the document, so the store is empty at exit.
      if documents_size(store: documents) != 0 then
        raise RuntimeError(val: "expected document store empty after didClose")
      end
      let _ = documents
    end
    // Two responses (initialize + shutdown). didOpen / didChange / didClose /
    // exit are notifications and produce no reply.
    match responses do
      | [r_init, r_shutdown] ->
        // initialize → success response with capabilities + id=1
        let id_init = match get_field(obj: r_init, key: "id") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "initialize response missing id")
        end
        match id_init do
          | JsonInt(val: 1) -> ()
          | _ -> raise RuntimeError(val: "initialize response id mismatch")
        end
        let result_field = match get_field(obj: r_init, key: "result") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "initialize response missing result")
        end
        let caps = match get_field(obj: result_field, key: "capabilities") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "initialize result missing capabilities")
        end
        let _ = caps
        // textDocumentSync = 1 (Full).
        let tds = match get_field(obj: caps, key: "textDocumentSync") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "capabilities missing textDocumentSync")
        end
        match tds do
          | JsonInt(val: 1) -> ()
          | _ -> raise RuntimeError(val: "expected textDocumentSync = 1 (Full)")
        end

        // shutdown → success response with id=2 and null result
        let id_sd = match get_field(obj: r_shutdown, key: "id") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "shutdown response missing id")
        end
        match id_sd do
          | JsonInt(val: 2) -> ()
          | _ -> raise RuntimeError(val: "shutdown response id mismatch")
        end
        let result_sd = match get_field(obj: r_shutdown, key: "result") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "shutdown response missing result")
        end
        match result_sd do
          | JsonNull -> ()
          | _ -> raise RuntimeError(val: "shutdown result must be null")
        end
      | _ -> raise RuntimeError(val: "expected exactly two responses (initialize + shutdown)")
    end
  end
  return ()
end
"#,
    );
}

// ─── Document state survives didChange ──────────────────────────────────────
//
// Acceptance criterion 2: full-content sync replaces the stored text and
// bumps the version. We open a document, change it twice, and assert the
// store mirrors the latest contentChanges payload. This is the assertion
// the issue calls out specifically — without it the test would pass even
// if didChange silently dropped its payload.

#[test]
fn did_change_full_content_replaces_stored_text() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"
import { classify } from "nxlib/jsonrpc.nx"
import { Some, None } from "std:option"
import { empty_state, empty_handlers, drive_messages, DriveResult,
         ServerState, documents_lookup, DocumentRecord }
  from "nxlib/lsp/server.nx"

let main = fn () -> unit do
  // Open with text "first", then two didChange replacing with "second" and
  // finally "third". After each change the store should reflect the latest.
  let m_open = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///x\",\"languageId\":\"nexus\",\"version\":1,\"text\":\"first\"}}}"))
  // First initialize so the lifecycle gate doesn't drop the notifications.
  let m_init = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}"))
  let m_change_1 = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{\"textDocument\":{\"uri\":\"file:///x\",\"version\":2},\"contentChanges\":[{\"text\":\"second\"}]}}"))
  let m_change_2 = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{\"textDocument\":{\"uri\":\"file:///x\",\"version\":3},\"contentChanges\":[{\"text\":\"third\"}]}}"))

  let result = drive_messages(
    state: empty_state(),
    msgs: [m_init, m_open, m_change_1, m_change_2],
    handlers: empty_handlers()
  )
  match result do | DriveResult(state, responses: _, notifications: _, exited: _) ->
    match state do | ServerState(lifecycle: _, documents, user_data: _) ->
      match documents_lookup(store: documents, uri: "file:///x") do
        | None -> raise RuntimeError(val: "document missing after didChange")
        | Some(val: DocumentRecord(uri: _, text, version)) ->
          if text != "third" then
            raise RuntimeError(val: "expected text='third' got '" ++ text ++ "'")
          end
          if version != 3 then
            raise RuntimeError(val: "expected version=3")
          end
      end
    end
  end
  return ()
end
"#,
    );
}

// ─── Pre-initialize gate: requests rejected with -32002 ─────────────────────

#[test]
fn requests_before_initialize_return_server_not_initialized() {
    exec_with_stdlib(
        r#"
import { parse, JsonValue, JsonInt, JsonString, get_field, JsonObject } from "std:json"
import { Some, None } from "std:option"
import { classify } from "nxlib/jsonrpc.nx"
import { empty_state, empty_handlers, drive_messages, DriveResult,
         server_not_initialized }
  from "nxlib/lsp/server.nx"

let main = fn () -> unit do
  // Send a `shutdown` (a request) before `initialize`. The server must
  // respond with a JSON-RPC error carrying code -32002.
  let m_shutdown = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"shutdown\"}"))
  let result = drive_messages(
    state: empty_state(),
    msgs: [m_shutdown],
    handlers: empty_handlers()
  )
  match result do | DriveResult(state: _, responses, notifications: _, exited: _) ->
    match responses do
      | [r] ->
        let err = match get_field(obj: r, key: "error") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "expected error response")
        end
        let code = match get_field(obj: err, key: "code") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "error missing code")
        end
        match code do
          | JsonInt(val: c) ->
            if c != server_not_initialized() then
              raise RuntimeError(val: "expected server_not_initialized code")
            end
          | _ -> raise RuntimeError(val: "code not int")
        end
        // id must echo the request id, not be null.
        let id = match get_field(obj: r, key: "id") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "error response missing id")
        end
        match id do
          | JsonInt(val: 42) -> ()
          | _ -> raise RuntimeError(val: "error id mismatch")
        end
      | _ -> raise RuntimeError(val: "expected exactly one response")
    end
  end
  return ()
end
"#,
    );
}

// ─── Unknown method routes through on_other → MethodNotFound by default ──────

#[test]
fn unknown_method_returns_method_not_found() {
    exec_with_stdlib(
        r#"
import { parse, JsonInt, get_field } from "std:json"
import { Some, None } from "std:option"
import { classify, method_not_found } from "nxlib/jsonrpc.nx"
import { empty_state, empty_handlers, drive_messages, DriveResult }
  from "nxlib/lsp/server.nx"

let main = fn () -> unit do
  // First initialize, then send an unknown request.
  let m_init = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}"))
  let m_unknown = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/somethingWeird\",\"params\":{}}"))
  let result = drive_messages(
    state: empty_state(),
    msgs: [m_init, m_unknown],
    handlers: empty_handlers()
  )
  match result do | DriveResult(state: _, responses, notifications: _, exited: _) ->
    match responses do
      | [_init_resp, r_unknown] ->
        let err = match get_field(obj: r_unknown, key: "error") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "expected error for unknown method")
        end
        let code = match get_field(obj: err, key: "code") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "error missing code")
        end
        match code do
          | JsonInt(val: c) ->
            if c != method_not_found() then
              raise RuntimeError(val: "expected method_not_found code")
            end
          | _ -> raise RuntimeError(val: "code not int")
        end
      | _ -> raise RuntimeError(val: "expected exactly two responses")
    end
  end
  return ()
end
"#,
    );
}

// ─── Custom handler vtable: language-specific server overrides on_other ──────
//
// Demonstrates the vtable pattern the issue requires: a caller-supplied
// record of fn pointers customizes behavior without modifying server.nx.
// The custom on_other handles `textDocument/documentSymbol` by returning
// an empty array; the default would have returned MethodNotFound.

#[test]
fn custom_on_other_handler_intercepts_method() {
    exec_with_stdlib(
        r#"
import { parse, JsonValue, JsonArray, JsonInt, JsonNull, get_field, JsonObject } from "std:json"
import { Some, None } from "std:option"
import { classify } from "nxlib/jsonrpc.nx"
import { empty_state, empty_handlers, drive_messages, DriveResult,
         Handlers, HandlerResult, ServerState }
  from "nxlib/lsp/server.nx"

let custom_other = fn (state: ServerState, method: string, params: JsonValue) -> HandlerResult throws { Exn } do
  let _ = params
  if method == "textDocument/documentSymbol" then
    return HandlerResult(
      state,
      response: Some(val: JsonArray(items: [])),
      notifications: []
    )
  end
  return HandlerResult(state, response: None, notifications: [])
end

let main = fn () -> unit do
  let base = empty_handlers()
  let custom = match base do | Handlers(on_initialize, on_initialized, on_did_open,
                                          on_did_change, on_did_close, on_did_save,
                                          on_shutdown, on_other: _) ->
    Handlers(on_initialize, on_initialized, on_did_open, on_did_change,
             on_did_close, on_did_save, on_shutdown, on_other: custom_other)
  end

  let m_init = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}"))
  let m_doc_symbol = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/documentSymbol\",\"params\":{}}"))

  let result = drive_messages(
    state: empty_state(),
    msgs: [m_init, m_doc_symbol],
    handlers: custom
  )
  match result do | DriveResult(state: _, responses, notifications: _, exited: _) ->
    match responses do
      | [_init_r, r_sym] ->
        let result_field = match get_field(obj: r_sym, key: "result") do
          | Some(val: v) -> v
          | None -> raise RuntimeError(val: "documentSymbol response missing result (got error)")
        end
        match result_field do
          | JsonArray(items: []) -> ()
          | _ -> raise RuntimeError(val: "expected empty array")
        end
      | _ -> raise RuntimeError(val: "expected exactly two responses")
    end
  end
  return ()
end
"#,
    );
}

// ─── didChange with incremental sync raises (server advertises Full only) ────
//
// The scaffold rejects ContentChangeIncremental rather than silently
// dropping the edit — that's the gap the no-silent-defaults policy
// requires. The error becomes an InvalidParams response rather than
// poisoning state.

#[test]
fn did_change_with_incremental_change_returns_invalid_params() {
    // didChange is a notification, so the InvalidParams error stays in the
    // server logs (no reply channel). Instead we drive a didChange after
    // init and verify the document state was *not* mutated — proving the
    // raise short-circuited the put.
    exec_with_stdlib(
        r#"
import { parse } from "std:json"
import { Some, None } from "std:option"
import { classify } from "nxlib/jsonrpc.nx"
import { empty_state, empty_handlers, drive_messages, DriveResult,
         ServerState, documents_lookup, DocumentRecord }
  from "nxlib/lsp/server.nx"

let main = fn () -> unit do
  let m_init = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}"))
  let m_open = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///y\",\"languageId\":\"nexus\",\"version\":1,\"text\":\"original\"}}}"))
  // Incremental change carries a `range` field. server.nx must reject this.
  let m_change = classify(value: parse(s: "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{\"textDocument\":{\"uri\":\"file:///y\",\"version\":2},\"contentChanges\":[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":3}},\"text\":\"abc\"}]}}"))

  let result = drive_messages(
    state: empty_state(),
    msgs: [m_init, m_open, m_change],
    handlers: empty_handlers()
  )
  match result do | DriveResult(state, responses: _, notifications: _, exited: _) ->
    match state do | ServerState(lifecycle: _, documents, user_data: _) ->
      match documents_lookup(store: documents, uri: "file:///y") do
        | None -> raise RuntimeError(val: "document vanished")
        | Some(val: DocumentRecord(uri: _, text, version)) ->
          // text + version unchanged because the change was rejected.
          if text != "original" then
            raise RuntimeError(val: "incremental change should not have mutated text, got '" ++ text ++ "'")
          end
          if version != 1 then
            raise RuntimeError(val: "incremental change should not have bumped version")
          end
      end
    end
  end
  return ()
end
"#,
    );
}

// ─── publishDiagnostics notification frame helper ────────────────────────────

#[test]
fn make_publish_diagnostics_notification_shape() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonString, JsonObject, get_field } from "std:json"
import { Some, None } from "std:option"
import { make_publish_diagnostics_notification } from "nxlib/lsp/server.nx"
import { PublishDiagnosticsParams } from "nxlib/lsp/protocol.nx"

let main = fn () -> unit do
  let params = PublishDiagnosticsParams(
    uri: "file:///z.nx",
    version: None,
    diagnostics: []
  )
  let frame = make_publish_diagnostics_notification(params)
  // jsonrpc + method must appear; an `id` field must NOT appear (notification).
  let method = match get_field(obj: frame, key: "method") do
    | Some(val: v) -> v
    | None -> raise RuntimeError(val: "missing method")
  end
  match method do
    | JsonString(val: s) ->
      if s != "textDocument/publishDiagnostics" then
        raise RuntimeError(val: "wrong method")
      end
    | _ -> raise RuntimeError(val: "method not a string")
  end
  match get_field(obj: frame, key: "id") do
    | None -> ()
    | Some(_) -> raise RuntimeError(val: "notification must not have an id")
  end
  return ()
end
"#,
    );
}

// ─── I/O layer signature smoke ──────────────────────────────────────────────
//
// Mirrors the jsonrpc `jsonrpc_io_layer_typechecks` pattern: pin the
// public surface so signature drift fails fast. Runtime exercise of
// `run_server` requires stateful Console mocking which the harness can't
// supply yet (same constraint the jsonrpc test calls out).

#[test]
fn server_io_layer_typechecks() {
    use crate::harness::should_typecheck;
    should_typecheck(
        r#"
import { run_server, empty_state, empty_handlers } from "nxlib/lsp/server.nx"

let main = fn () -> unit require { PermConsole } do
  // Each export's signature is exercised once. The bodies below never
  // actually run (the harness's typecheck mode doesn't connect stdin).
  let _ = run_server
  let _ = empty_state
  let _ = empty_handlers
  return ()
end
"#,
    );
}
