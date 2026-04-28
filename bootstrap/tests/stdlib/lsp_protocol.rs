//! Tests for `nxlib/lsp/protocol.nx` — LSP wire-format type algebra plus
//! its from_json / to_json codecs. Each round-trip uses payloads modelled
//! after the LSP spec examples.

use crate::harness::exec_with_stdlib;

// ─── Position ────────────────────────────────────────────────────────────────

#[test]
fn position_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"
import { Position, position_to_json, position_from_json }
  from "nxlib/lsp/protocol.nx"

let main = fn () -> unit do
  let raw = "{\"line\":3,\"character\":7}"
  let v = parse(s: raw)
  let p = position_from_json(v)
  match p do | Position(line, character) ->
    if line != 3 then raise RuntimeError(val: "wrong line") end
    if character != 7 then raise RuntimeError(val: "wrong character") end
  end
  let back = position_to_json(p)
  if serialize(v: back) != raw then raise RuntimeError(val: "Position roundtrip lost form") end
  return ()
end
"#,
    );
}

// ─── Range / Location ────────────────────────────────────────────────────────

#[test]
fn range_and_location_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"
import { Range, Location, range_to_json, range_from_json,
         location_to_json, location_from_json }
  from "nxlib/lsp/protocol.nx"

let main = fn () -> unit do
  let raw = "{\"uri\":\"file:///tmp/a.nx\",\"range\":{\"start\":{\"line\":3,\"character\":7},\"end\":{\"line\":3,\"character\":12}}}"
  let v = parse(s: raw)
  let loc = location_from_json(v)
  let back = location_to_json(loc)
  if serialize(v: back) != raw then raise RuntimeError(val: "Location roundtrip lost form") end
  return ()
end
"#,
    );
}

// ─── PublishDiagnostics fixture ──────────────────────────────────────────────
// Lifted from the LSP spec (textDocument/publishDiagnostics example) and
// reused from json.rs's golden so the wire layer agrees with the raw codec.

#[test]
fn publish_diagnostics_roundtrip_against_lsp_fixture() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonObject, JsonField, get_field, JsonArray } from "std:json"
import { PublishDiagnosticsParams, Diagnostic, DiagSevError,
         publish_diagnostics_params_to_json, publish_diagnostics_params_from_json }
  from "nxlib/lsp/protocol.nx"
import { Some, None } from "std:option"

let main = fn () -> unit do
  // Note: serialiser emits keys in object-build order. We match that exact
  // order in our `to_json`, so the round-trip below compares byte-for-byte.
  let raw = "{\"uri\":\"file:///tmp/a.nx\",\"diagnostics\":[{\"range\":{\"start\":{\"line\":3,\"character\":7},\"end\":{\"line\":3,\"character\":12}},\"severity\":1,\"message\":\"unknown identifier\"}]}"
  let v = parse(s: raw)
  let params = publish_diagnostics_params_from_json(v)
  match params do | PublishDiagnosticsParams(uri, version, diagnostics) ->
    if uri != "file:///tmp/a.nx" then raise RuntimeError(val: "uri mismatch") end
    match version do
      | None -> ()
      | Some(_) -> raise RuntimeError(val: "expected None version")
    end
    match diagnostics do
      | [] -> raise RuntimeError(val: "empty diagnostics list")
      | d :: rest ->
        match rest do
          | [] -> ()
          | _ -> raise RuntimeError(val: "expected exactly one diagnostic")
        end
        match d do | Diagnostic(range: _, severity, code, source: _, message) ->
          match severity do
            | Some(val: DiagSevError) -> ()
            | _ -> raise RuntimeError(val: "expected DiagSevError")
          end
          match code do
            | None -> ()
            | Some(_) -> raise RuntimeError(val: "expected absent code")
          end
          if message != "unknown identifier" then
            raise RuntimeError(val: "wrong message")
          end
        end
    end
  end
  let back = publish_diagnostics_params_to_json(p: params)
  if serialize(v: back) != raw then raise RuntimeError(val: "publishDiagnostics roundtrip lost form") end
  return ()
end
"#,
    );
}

// ─── Initialize request fixture ──────────────────────────────────────────────

#[test]
fn initialize_params_decode_and_encode_capabilities() {
    exec_with_stdlib(
        r#"
import { parse, serialize, JsonObject } from "std:json"
import { InitializeParams, initialize_params_from_json, initialize_params_to_json,
         InitializeResult, ServerCapabilities, ServerInfo, TdsFull,
         initialize_result_to_json, initialize_result_from_json }
  from "nxlib/lsp/protocol.nx"
import { Some, None } from "std:option"

let main = fn () -> unit do
  // Minimal initialize payload — we accept and round-trip processId,
  // rootUri, and an opaque capabilities sub-object.
  let raw = "{\"processId\":4242,\"rootUri\":\"file:///tmp/proj\",\"capabilities\":{\"textDocument\":{\"synchronization\":{\"didSave\":true}}}}"
  let v = parse(s: raw)
  let params = initialize_params_from_json(v)
  match params do | InitializeParams(process_id, root_uri, client_capabilities: _, trace) ->
    match process_id do
      | Some(val: pid) ->
        if pid != 4242 then raise RuntimeError(val: "wrong processId") end
      | None -> raise RuntimeError(val: "expected processId")
    end
    match root_uri do
      | Some(val: uri) ->
        if uri != "file:///tmp/proj" then raise RuntimeError(val: "wrong rootUri") end
      | None -> raise RuntimeError(val: "expected rootUri")
    end
    match trace do
      | None -> ()
      | Some(_) -> raise RuntimeError(val: "expected absent trace")
    end
  end
  // Round-trip: re-encode and re-parse must agree on the structural fields.
  let again_v = initialize_params_to_json(p: params)
  let again_str = serialize(v: again_v)
  let again_params = initialize_params_from_json(v: parse(s: again_str))
  match again_params do | InitializeParams(process_id: pid2, root_uri: ru2, client_capabilities: _, trace: _) ->
    match pid2 do
      | Some(val: n) ->
        if n != 4242 then raise RuntimeError(val: "pid lost in roundtrip") end
      | None -> raise RuntimeError(val: "pid lost in roundtrip")
    end
    match ru2 do
      | Some(val: u) ->
        if u != "file:///tmp/proj" then raise RuntimeError(val: "rootUri lost in roundtrip") end
      | None -> raise RuntimeError(val: "rootUri lost in roundtrip")
    end
  end
  // Also round-trip an InitializeResult that advertises documentSymbolProvider
  // and Full text-document sync — the two Phase 1 capabilities the Nexus LSP
  // server actually supports.
  let caps = ServerCapabilities(
    text_document_sync: Some(val: TdsFull),
    document_symbol_provider: Some(val: true),
    hover_provider: None,
    definition_provider: None,
    references_provider: None
  )
  let info = Some(val: ServerInfo(name: "nexus-lsp", version: Some(val: "0.1")))
  let result = InitializeResult(capabilities: caps, server_info: info)
  let result_v = initialize_result_to_json(r: result)
  let result_str = serialize(v: result_v)
  let result_again = initialize_result_from_json(v: parse(s: result_str))
  match result_again do | InitializeResult(capabilities: cap2, server_info: _) ->
    match cap2 do | ServerCapabilities(text_document_sync, document_symbol_provider, hover_provider: _, definition_provider: _, references_provider: _) ->
      match text_document_sync do
        | Some(val: TdsFull) -> ()
        | _ -> raise RuntimeError(val: "TextDocumentSyncKind lost")
      end
      match document_symbol_provider do
        | Some(val: true) -> ()
        | _ -> raise RuntimeError(val: "documentSymbolProvider lost")
      end
    end
  end
  return ()
end
"#,
    );
}

// ─── didChange fixture ──────────────────────────────────────────────────────

#[test]
fn did_change_full_and_incremental_changes() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"
import { TextDocumentContentChangeEvent, ContentChangeFull, ContentChangeIncremental,
         content_change_event_to_json, content_change_event_from_json,
         VersionedTextDocumentIdentifier,
         versioned_text_document_identifier_to_json,
         versioned_text_document_identifier_from_json }
  from "nxlib/lsp/protocol.nx"
import { Some, None } from "std:option"

let main = fn () -> unit do
  // Full-content change.
  let raw_full = "{\"text\":\"let x = 1\\n\"}"
  let ev = content_change_event_from_json(v: parse(s: raw_full))
  match ev do
    | ContentChangeFull(text) ->
      if text != "let x = 1\n" then raise RuntimeError(val: "full text lost") end
    | ContentChangeIncremental(_) -> raise RuntimeError(val: "expected ContentChangeFull")
  end
  if serialize(v: content_change_event_to_json(ev)) != raw_full then
    raise RuntimeError(val: "full change roundtrip")
  end

  // Incremental change with explicit range and rangeLength.
  let raw_inc = "{\"range\":{\"start\":{\"line\":1,\"character\":2},\"end\":{\"line\":1,\"character\":5}},\"rangeLength\":3,\"text\":\"foo\"}"
  let ev2 = content_change_event_from_json(v: parse(s: raw_inc))
  match ev2 do
    | ContentChangeIncremental(range: _, range_length, text) ->
      if text != "foo" then raise RuntimeError(val: "incremental text lost") end
      match range_length do
        | Some(val: n) ->
          if n != 3 then raise RuntimeError(val: "rangeLength lost") end
        | None -> raise RuntimeError(val: "expected rangeLength")
      end
    | ContentChangeFull(_) -> raise RuntimeError(val: "expected ContentChangeIncremental")
  end
  if serialize(v: content_change_event_to_json(ev: ev2)) != raw_inc then
    raise RuntimeError(val: "incremental change roundtrip")
  end

  // Versioned identifier.
  let raw_vid = "{\"uri\":\"file:///tmp/a.nx\",\"version\":2}"
  let vid = versioned_text_document_identifier_from_json(v: parse(s: raw_vid))
  if serialize(v: versioned_text_document_identifier_to_json(id: vid)) != raw_vid then
    raise RuntimeError(val: "VersionedTextDocumentIdentifier roundtrip")
  end
  return ()
end
"#,
    );
}

// ─── DocumentSymbol with children ────────────────────────────────────────────

#[test]
fn document_symbol_nested_children_roundtrip() {
    exec_with_stdlib(
        r#"
import { parse, serialize } from "std:json"
import { DocumentSymbol, SymbolKindFunction, SymbolKindVariable,
         document_symbol_to_json, document_symbol_from_json, Range, Position }
  from "nxlib/lsp/protocol.nx"
import { Some, None } from "std:option"

let main = fn () -> unit do
  let r = Range(
    start: Position(line: 0, character: 0),
    end_: Position(line: 5, character: 0)
  )
  let inner = DocumentSymbol(
    name: "x",
    detail: None,
    kind: SymbolKindVariable,
    range: r,
    selection_range: r,
    children: []
  )
  let outer = DocumentSymbol(
    name: "main",
    detail: Some(val: "() -> unit"),
    kind: SymbolKindFunction,
    range: r,
    selection_range: r,
    children: [inner]
  )
  let v = document_symbol_to_json(s: outer)
  let s = serialize(v)
  let parsed_back = document_symbol_from_json(v: parse(s))
  match parsed_back do | DocumentSymbol(name, detail, kind: _, range: _, selection_range: _, children) ->
    if name != "main" then raise RuntimeError(val: "outer name lost") end
    match detail do
      | Some(val: d) ->
        if d != "() -> unit" then raise RuntimeError(val: "detail lost") end
      | None -> raise RuntimeError(val: "detail dropped")
    end
    match children do
      | [] -> raise RuntimeError(val: "children dropped")
      | c :: rest ->
        match rest do
          | [] -> ()
          | _ -> raise RuntimeError(val: "extra children")
        end
        match c do | DocumentSymbol(name: cname, detail: _, kind: _, range: _, selection_range: _, children: _) ->
          if cname != "x" then raise RuntimeError(val: "inner name lost") end
        end
    end
  end
  return ()
end
"#,
    );
}

// ─── Negative: missing required field ────────────────────────────────────────

#[test]
fn position_missing_field_raises_protocol_error() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"
import { position_from_json, LspProtocolError } from "nxlib/lsp/protocol.nx"

let main = fn () -> unit do
  let v = parse(s: "{\"line\":3}")
  try
    let _ = position_from_json(v)
    raise RuntimeError(val: "expected LspProtocolError")
  catch err ->
    match err do
      | LspProtocolError(_) -> return ()
      | _ -> raise RuntimeError(val: "expected LspProtocolError, got other")
    end
  end
end
"#,
    );
}

// ─── Negative: out-of-range severity ────────────────────────────────────────

#[test]
fn severity_out_of_range_raises_protocol_error() {
    exec_with_stdlib(
        r#"
import { parse } from "std:json"
import { diagnostic_from_json, LspProtocolError } from "nxlib/lsp/protocol.nx"

let main = fn () -> unit do
  let raw = "{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":0}},\"severity\":99,\"message\":\"x\"}"
  try
    let _ = diagnostic_from_json(v: parse(s: raw))
    raise RuntimeError(val: "expected LspProtocolError on severity 99")
  catch err ->
    match err do
      | LspProtocolError(_) -> return ()
      | _ -> raise RuntimeError(val: "expected LspProtocolError variant")
    end
  end
end
"#,
    );
}
