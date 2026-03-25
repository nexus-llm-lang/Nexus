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
)
