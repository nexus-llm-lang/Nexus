//! WASM-only half of the host bridge.
//!
//! Compiles only when `target_family = "wasm"`. All outgoing HTTP goes
//! through `perform_request`, which delegates to `super::url_guard` for
//! SSRF screening before any WASI call.
//!
//! ## Return-value ABI policy (host-http-*)
//!
//! - **bool result** (success/failure flag): `s32`, `1` = success, `0` = failure.
//!   Used by `host-http-respond`, `host-http-stop`.
//! - **handle**: `s64`. `-1` = failure sentinel (no valid handle has that value
//!   because `next_id()` starts at 1 and only increments). Used by
//!   `host-http-listen`.
//! - **status/handle parameter**: `s64` (HTTP status code or opaque handle id).
//! - **structured response**: `string` whose first line encodes a leading
//!   sentinel (e.g. `host-http-accept` returns `"-1\n\n\n\n{err}"` on failure,
//!   `host-http-request` returns `"0\n0\n…"`). The string carrier is required
//!   because more than one value must cross the FFI boundary.
//!
//! Mixing `s32` boolean and `s64` handle is intentional: WIT `bool` would lower
//! the same way as `s32` 1/0, but explicit `s32` keeps the WIT signature stable
//! for both component-model and core-WASM (stub) callers (see `bundler.rs`
//! `merge_remaining_stubs`, `nxlib/stdlib/nexus_host_stub.wat`).

mod bindings {
    wit_bindgen::generate!({
        world: "bridge",
        path: "wit",
        generate_all,
    });
}

use super::url_guard;
use bindings::wasi::http::outgoing_handler;
use bindings::wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
use bindings::wasi::sockets::instance_network::instance_network;
use bindings::wasi::sockets::network::{IpAddressFamily, IpSocketAddress, Ipv4SocketAddress};
use bindings::wasi::sockets::tcp::TcpSocket;
use bindings::wasi::sockets::tcp_create_socket::create_tcp_socket;
use http::Uri;
use std::cell::RefCell;
use std::collections::HashMap;

const MAX_HTTP_URL_BYTES: usize = 8 * 1024;
const MAX_HTTP_HEADERS_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// State for server operations
// ---------------------------------------------------------------------------

struct ServerEntry {
    socket: TcpSocket,
}

struct ConnEntry {
    output: bindings::wasi::io::streams::OutputStream,
    // Hold references to keep the connection alive until respond/drop
    _input: bindings::wasi::io::streams::InputStream,
    _client_socket: TcpSocket,
}

thread_local! {
    static SERVERS: RefCell<HashMap<i64, ServerEntry>> = RefCell::new(HashMap::new());
    static CONNS: RefCell<HashMap<i64, ConnEntry>> = RefCell::new(HashMap::new());
    static NEXT_ID: RefCell<i64> = RefCell::new(1);
}

fn next_id() -> i64 {
    NEXT_ID.with(|cell| {
        let v = *cell.borrow();
        *cell.borrow_mut() = v + 1;
        v
    })
}

// ---------------------------------------------------------------------------
// HTTP client helpers
// ---------------------------------------------------------------------------

fn validate_bridge_limits(url: &str, headers: &str, body: &str) -> Result<(), String> {
    if url.len() > MAX_HTTP_URL_BYTES {
        return Err(format!("url exceeds {} bytes", MAX_HTTP_URL_BYTES));
    }
    if headers.len() > MAX_HTTP_HEADERS_BYTES {
        return Err(format!("headers exceed {} bytes", MAX_HTTP_HEADERS_BYTES));
    }
    if body.len() > MAX_HTTP_BODY_BYTES {
        return Err(format!("body exceeds {} bytes", MAX_HTTP_BODY_BYTES));
    }
    Ok(())
}

fn parse_method(method: &str) -> Method {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "DELETE" => Method::Delete,
        "CONNECT" => Method::Connect,
        "OPTIONS" => Method::Options,
        "TRACE" => Method::Trace,
        "PATCH" => Method::Patch,
        other => Method::Other(other.to_string()),
    }
}

fn parse_url(url: &str) -> Result<(Scheme, String, String), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("empty URL".to_string());
    }

    let uri: Uri = trimmed
        .parse()
        .map_err(|e| format!("invalid URL: {}", e))?;

    let scheme = match uri.scheme_str() {
        Some("https") => Scheme::Https,
        Some("http") => Scheme::Http,
        Some(other) => return Err(format!("unsupported URL scheme: {}", other)),
        None => return Err("missing URL scheme".to_string()),
    };

    let authority = uri
        .authority()
        .map(|authority| authority.as_str().to_string())
        .ok_or_else(|| "missing authority".to_string())?;
    if authority.is_empty() {
        return Err("missing authority".to_string());
    }

    let host = uri.host().ok_or_else(|| "missing host".to_string())?;
    if let Some(reason) = url_guard::is_blocked_host(host) {
        return Err(format!(
            "blocked destination '{}': {} (SSRF protection)",
            host, reason
        ));
    }

    let path = uri
        .path_and_query()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    Ok((scheme, authority, path))
}

fn parse_http_headers(headers: &str, authority: &str) -> Fields {
    let mut has_host = false;
    let fields = Fields::new();

    for line in headers.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        let _ = fields.append(name, value.trim().as_bytes());
    }

    if !has_host {
        let _ = fields.append("Host", authority.as_bytes());
    }

    fields
}

fn perform_request(
    method: &str,
    url: &str,
    headers: &str,
    body: &str,
) -> Result<(u16, String, String), String> {
    validate_bridge_limits(url, headers, body)?;
    let (scheme, authority, path) = parse_url(url)?;
    let header_fields = parse_http_headers(headers, &authority);
    let request = OutgoingRequest::new(header_fields);

    request
        .set_method(&parse_method(method))
        .map_err(|_| "invalid method".to_string())?;
    request
        .set_scheme(Some(&scheme))
        .map_err(|_| "invalid scheme".to_string())?;
    request
        .set_authority(Some(&authority))
        .map_err(|_| "invalid authority".to_string())?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|_| "invalid path".to_string())?;

    let out_body = request
        .body()
        .map_err(|_| "failed to get request body handle".to_string())?;
    let stream = out_body
        .write()
        .map_err(|_| "failed to get body write stream".to_string())?;
    stream
        .blocking_write_and_flush(body.as_bytes())
        .map_err(|_| "failed to write request body".to_string())?;
    drop(stream);
    OutgoingBody::finish(out_body, None)
        .map_err(|_| "failed to finalize request body".to_string())?;

    let future = outgoing_handler::handle(request, None).map_err(|e| format!("{:?}", e))?;
    let pollable = future.subscribe();
    pollable.block();

    let incoming = match future.get() {
        Some(Ok(Ok(resp))) => resp,
        Some(Ok(Err(err))) => return Err(format!("{:?}", err)),
        Some(Err(_)) => return Err("response consumed".to_string()),
        None => return Err("response not ready".to_string()),
    };
    let status = incoming.status();

    let mut response_headers = String::new();
    for (name, value) in &incoming.headers().entries() {
        response_headers.push_str(name);
        response_headers.push(':');
        response_headers.push_str(&String::from_utf8_lossy(value));
        response_headers.push('\n');
    }

    let mut response_body = String::new();
    if let Ok(in_body) = incoming.consume() {
        if let Ok(stream) = in_body.stream() {
            loop {
                match stream.blocking_read(8192) {
                    Ok(chunk) if chunk.is_empty() => break,
                    Ok(chunk) => {
                        let next_len = response_body.len().saturating_add(chunk.len());
                        if next_len > MAX_HTTP_RESPONSE_BYTES {
                            return Err(format!(
                                "response exceeds {} bytes",
                                MAX_HTTP_RESPONSE_BYTES
                            ));
                        }
                        response_body.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    Err(bindings::wasi::io::streams::StreamError::Closed) => break,
                    Err(bindings::wasi::io::streams::StreamError::LastOperationFailed(_)) => break,
                }
            }
            drop(stream);
        }
        let _ = bindings::wasi::http::types::IncomingBody::finish(in_body);
    }

    Ok((status, response_headers, response_body))
}

// ---------------------------------------------------------------------------
// Server helpers
// ---------------------------------------------------------------------------

fn parse_socket_address(addr: &str) -> Result<IpSocketAddress, String> {
    let (host, port_str) = addr
        .rsplit_once(':')
        .ok_or_else(|| "missing port in address".to_string())?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| "invalid port number".to_string())?;

    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return Err(format!("invalid IPv4 address: {}", host));
    }
    let octets: Result<Vec<u8>, _> = parts.iter().map(|s| s.parse::<u8>()).collect();
    let octets = octets.map_err(|_| "invalid IPv4 octet".to_string())?;

    Ok(IpSocketAddress::Ipv4(Ipv4SocketAddress {
        port,
        address: (octets[0], octets[1], octets[2], octets[3]),
    }))
}

fn do_listen(addr: &str) -> Result<i64, String> {
    let socket_addr = parse_socket_address(addr)?;
    let family = match &socket_addr {
        IpSocketAddress::Ipv4(_) => IpAddressFamily::Ipv4,
        IpSocketAddress::Ipv6(_) => IpAddressFamily::Ipv6,
    };

    let socket = create_tcp_socket(family).map_err(|e| format!("create socket: {:?}", e))?;
    let network = instance_network();

    socket
        .start_bind(&network, socket_addr)
        .map_err(|e| format!("bind: {:?}", e))?;
    socket.subscribe().block();
    socket
        .finish_bind()
        .map_err(|e| format!("finish bind: {:?}", e))?;

    socket
        .start_listen()
        .map_err(|e| format!("listen: {:?}", e))?;
    socket.subscribe().block();
    socket
        .finish_listen()
        .map_err(|e| format!("finish listen: {:?}", e))?;

    let id = next_id();
    SERVERS.with(|servers| {
        servers.borrow_mut().insert(id, ServerEntry { socket });
    });
    Ok(id)
}

fn do_accept(server_id: i64) -> Result<String, String> {
    // Block until a connection is available, then accept it.
    let (client_socket, input, output) = SERVERS.with(|servers| {
        let servers = servers.borrow();
        let entry = servers
            .get(&server_id)
            .ok_or_else(|| "invalid server id".to_string())?;

        // Poll until accept succeeds
        loop {
            match entry.socket.accept() {
                Ok(result) => return Ok(result),
                Err(bindings::wasi::sockets::network::ErrorCode::WouldBlock) => {
                    entry.socket.subscribe().block();
                }
                Err(e) => return Err(format!("accept: {:?}", e)),
            }
        }
    })?;

    // Read the HTTP request from the input stream
    let req_data = read_http_request(&input)?;

    let req_id = next_id();
    CONNS.with(|conns| {
        conns.borrow_mut().insert(
            req_id,
            ConnEntry {
                output,
                _input: input,
                _client_socket: client_socket,
            },
        );
    });

    // Wire format: "{req_id}\n{method}\n{path}\n{headers}\n{body}"
    Ok(format!(
        "{}\n{}\n{}\n{}\n{}",
        req_id, req_data.method, req_data.path, req_data.headers, req_data.body
    ))
}

struct HttpRequestData {
    method: String,
    path: String,
    headers: String,
    body: String,
}

fn read_http_request(
    input: &bindings::wasi::io::streams::InputStream,
) -> Result<HttpRequestData, String> {
    let mut buf = Vec::new();

    // Read until we find \r\n\r\n (end of HTTP headers)
    let header_end = loop {
        input.subscribe().block();
        match input.read(4096) {
            Ok(chunk) if chunk.is_empty() => {
                return Err("connection closed before headers complete".to_string());
            }
            Ok(chunk) => {
                buf.extend_from_slice(&chunk);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos;
                }
                if buf.len() > MAX_HTTP_HEADERS_BYTES {
                    return Err("request headers too large".to_string());
                }
            }
            Err(_) => return Err("failed to read request".to_string()),
        }
    };

    let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let body_start = header_end + 4;

    // Parse request line: "METHOD /path HTTP/1.x"
    let mut lines = header_str.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    // Collect headers as "name:value\n" pairs
    let mut headers_out = String::new();
    let mut content_length: usize = 0;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value
                    .parse()
                    .map_err(|_| format!("invalid Content-Length header: '{}'", value))?;
            }
            headers_out.push_str(name);
            headers_out.push(':');
            headers_out.push_str(value);
            headers_out.push('\n');
        }
    }

    // Read body if Content-Length > 0
    let mut body_buf: Vec<u8> = buf[body_start..].to_vec();
    while body_buf.len() < content_length {
        input.subscribe().block();
        match input.read(4096) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => body_buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    body_buf.truncate(content_length);
    let body = String::from_utf8_lossy(&body_buf).to_string();

    Ok(HttpRequestData {
        method,
        path,
        headers: headers_out,
        body,
    })
}

fn do_respond(req_id: i64, status: i64, headers: &str, body: &str) -> Result<(), String> {
    // Build the wire response before touching CONNS so allocation errors
    // cannot orphan the entry. Once removed, the entry is owned locally and
    // unconditionally dropped on every exit path — including write failure —
    // closing the streams and client socket.
    let mut response = format!(
        "HTTP/1.1 {} {}\r\n",
        status,
        url_guard::status_reason(status)
    );
    response.push_str(&format!("Content-Length: {}\r\n", body.len()));

    for line in headers.lines() {
        let line = line.trim();
        if !line.is_empty() {
            // Convert "name:value" to "name: value\r\n"
            if let Some((name, value)) = line.split_once(':') {
                response.push_str(name.trim());
                response.push_str(": ");
                response.push_str(value.trim());
                response.push_str("\r\n");
            }
        }
    }
    response.push_str("Connection: close\r\n");
    response.push_str("\r\n");
    response.push_str(body);

    let entry = CONNS.with(|conns| {
        conns
            .borrow_mut()
            .remove(&req_id)
            .ok_or_else(|| "invalid request id".to_string())
    })?;

    let write_result = entry
        .output
        .blocking_write_and_flush(response.as_bytes())
        .map_err(|_| "failed to write response".to_string());

    // Dropping entry closes streams and client socket regardless of write outcome.
    drop(entry);
    write_result
}

fn do_stop(server_id: i64) -> Result<(), String> {
    SERVERS.with(|servers| {
        servers
            .borrow_mut()
            .remove(&server_id)
            .ok_or_else(|| "invalid server id".to_string())
    })?;
    // Dropping the ServerEntry closes the TCP listener socket
    Ok(())
}

// ---------------------------------------------------------------------------
// Guest implementation
// ---------------------------------------------------------------------------

struct Guest;

impl bindings::exports::nexus::cli::nexus_host::Guest for Guest {
    fn host_http_request(method: String, url: String, headers: String, body: String) -> String {
        match perform_request(&method, &url, &headers, &body) {
            Ok((status, response_headers, response_body)) => {
                let hlen = response_headers.len();
                format!("{}\n{}\n{}{}", status, hlen, response_headers, response_body)
            }
            Err(err) => format!("0\n0\nhttp request failed: {}", err),
        }
    }

    fn host_http_listen(addr: String) -> i64 {
        match do_listen(&addr) {
            Ok(id) => id,
            Err(_e) => {
                // Error details are not propagable via i64 return;
                // the caller (stdlib) checks for -1 as failure sentinel
                -1
            }
        }
    }

    fn host_http_accept(server_id: i64) -> String {
        match do_accept(server_id) {
            Ok(s) => s,
            Err(e) => format!("-1\n\n\n\n{}", e),
        }
    }

    fn host_http_respond(req_id: i64, status: i64, headers: String, body: String) -> i32 {
        match do_respond(req_id, status, &headers, &body) {
            Ok(()) => 1,
            Err(_) => 0,
        }
    }

    fn host_http_stop(server_id: i64) -> i32 {
        match do_stop(server_id) {
            Ok(()) => 1,
            Err(_) => 0,
        }
    }
}

bindings::export!(Guest with_types_in bindings);
