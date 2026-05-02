;; Stub module for nexus:cli/nexus-host imports.
;; Provides dummy exports so nxc-produced WASMs can run standalone
;; on plain wasmtime without the nexus runtime.
;; HTTP functions trap with unreachable if actually called.
(module
  ;; host-http-accept(server_id: i64, ret_ptr: i32)
  (func (export "host-http-accept") (param i64 i32)
    unreachable)

  ;; host-http-request(method_ptr method_len url_ptr url_len headers_ptr headers_len body_ptr body_len ret_ptr: all i32)
  (func (export "host-http-request") (param i32 i32 i32 i32 i32 i32 i32 i32 i32)
    unreachable)

  ;; host-http-listen(addr_ptr: i32, addr_len: i32) -> i64
  (func (export "host-http-listen") (param i32 i32) (result i64)
    unreachable)

  ;; host-http-respond(req_id: i64, status: i64, headers_ptr: i32, headers_len: i32, body_ptr: i32, body_len: i32) -> i32
  (func (export "host-http-respond") (param i64 i64 i32 i32 i32 i32) (result i32)
    unreachable)

  ;; host-http-stop(server_id: i64) -> i32
  (func (export "host-http-stop") (param i64) (result i32)
    unreachable)

  ;; host-bridge-finalize() -> i64 — drain SERVERS/CONNS thread_locals; the
  ;; standalone path holds no host state, so this stub is a safe no-op
  ;; returning 0 dropped entries (NOT unreachable).
  (func (export "host-bridge-finalize") (result i64)
    i64.const 0)

  ;; host-http-respond-chunk-start(req_id: i64, status: i64,
  ;;   headers_ptr: i32, headers_len: i32) -> i32
  (func (export "host-http-respond-chunk-start") (param i64 i64 i32 i32) (result i32)
    unreachable)

  ;; host-http-respond-chunk-write(req_id: i64,
  ;;   body_chunk_ptr: i32, body_chunk_len: i32) -> i32
  (func (export "host-http-respond-chunk-write") (param i64 i32 i32) (result i32)
    unreachable)

  ;; host-http-respond-chunk-finish(req_id: i64) -> i32
  (func (export "host-http-respond-chunk-finish") (param i64) (result i32)
    unreachable)

  ;; host-http-request-with-options(method/url/headers/body ptr+len ×4,
  ;;   timeout_ms: i64, ret_ptr: i32)
  (func (export "host-http-request-with-options")
        (param i32 i32 i32 i32 i32 i32 i32 i32 i64 i32)
    unreachable)

  ;; host-http-cancel-accept(server_id: i64) -> i32
  (func (export "host-http-cancel-accept") (param i64) (result i32)
    unreachable)
)
