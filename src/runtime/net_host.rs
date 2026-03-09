//! Host-side implementation of `nexus:cli/nexus-host` HTTP functions for core-wasm mode.
//!
//! Provides the same host functions that `nexus_host_bridge` offers in component mode,
//! but backed by `ureq` (blocking HTTP client) so `nexus run` can execute net code directly.

use crate::constants::NEXUS_HOST_HTTP_MODULE;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read as IoRead, Write as IoWrite};
use std::net::{TcpListener, TcpStream};
use ureq::http;
use wasmtime::{Caller, Linker};

// ── Server / connection state ───────────────────────────────────────

struct ServerEntry {
    listener: TcpListener,
}

struct ConnEntry {
    stream: TcpStream,
}

thread_local! {
    static SERVERS: RefCell<HashMap<i64, ServerEntry>> = RefCell::new(HashMap::new());
    static CONNS: RefCell<HashMap<i64, ConnEntry>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<i64> = Cell::new(1);
}

fn next_id() -> i64 {
    NEXT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

// ── Memory helpers ──────────────────────────────────────────────────

fn read_wasm_string<T>(caller: &mut Caller<'_, T>, ptr: i32, len: i32) -> Option<String> {
    if len <= 0 {
        return Some(String::new());
    }
    let memory = caller.get_export("memory")?.into_memory()?;
    let data = memory.data(&*caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&data[start..end]).into_owned())
}

fn write_wasm_bytes<T>(caller: &mut Caller<'_, T>, bytes: &[u8]) -> Option<(i32, i32)> {
    // Verify memory exists before allocating
    let _ = caller.get_export("memory")?.into_memory()?;
    let alloc = caller.get_export("allocate")?.into_func()?;
    let mut results = [wasmtime::Val::I32(0)];
    alloc
        .call(
            &mut *caller,
            &[wasmtime::Val::I32(bytes.len() as i32)],
            &mut results,
        )
        .ok()?;
    let ptr = results[0].unwrap_i32();
    if ptr == 0 {
        return None;
    }
    let memory = caller.get_export("memory")?.into_memory()?;
    memory.write(&mut *caller, ptr as usize, bytes).ok()?;
    Some((ptr, bytes.len() as i32))
}

fn write_i32_pair<T>(caller: &mut Caller<'_, T>, ret_ptr: i32, a: i32, b: i32) {
    if let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) {
        let _ = memory.write(&mut *caller, ret_ptr as usize, &a.to_le_bytes());
        let _ = memory.write(&mut *caller, ret_ptr as usize + 4, &b.to_le_bytes());
    }
}

// ── HTTP client ─────────────────────────────────────────────────────

fn do_http_request(method: &str, url: &str, headers: &str, body: &str) -> String {
    let result = (|| -> Result<(u16, String, String), String> {
        let config = ureq::config::Config::builder()
            .http_status_as_error(false)
            .build();
        let agent = ureq::Agent::new_with_config(config);

        let http_method: http::Method = method
            .parse()
            .map_err(|_| format!("invalid HTTP method: {}", method))?;
        let mut builder = http::request::Builder::new().method(http_method).uri(url);

        for line in headers.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((name, value)) = line.split_once(':') {
                builder = builder.header(name.trim(), value.trim());
            }
        }

        let response = if body.is_empty() {
            let req = builder.body(()).map_err(|e: http::Error| e.to_string())?;
            agent.run(req).map_err(|e: ureq::Error| e.to_string())?
        } else {
            let req = builder
                .body(body.as_bytes().to_vec())
                .map_err(|e: http::Error| e.to_string())?;
            agent.run(req).map_err(|e: ureq::Error| e.to_string())?
        };

        let status = response.status().as_u16();
        let mut resp_headers = String::new();
        for (name, value) in response.headers() {
            resp_headers.push_str(name.as_str());
            resp_headers.push_str(": ");
            resp_headers.push_str(value.to_str().unwrap_or(""));
            resp_headers.push('\n');
        }
        let resp_body = response
            .into_body()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok((status, resp_headers, resp_body))
    })();

    match result {
        Ok((status, resp_headers, resp_body)) => {
            let hlen = resp_headers.len();
            format!("{}\n{}\n{}{}", status, hlen, resp_headers, resp_body)
        }
        Err(err) => format!("0\n0\nhttp request failed: {}", err),
    }
}

// ── TCP server ──────────────────────────────────────────────────────

fn do_listen(addr: &str) -> Result<i64, String> {
    let listener = TcpListener::bind(addr).map_err(|e| e.to_string())?;
    let id = next_id();
    SERVERS.with(|s| s.borrow_mut().insert(id, ServerEntry { listener }));
    Ok(id)
}

fn do_accept(server_id: i64) -> Result<String, String> {
    let listener_clone = SERVERS
        .with(|s| {
            s.borrow()
                .get(&server_id)
                .map(|entry| entry.listener.try_clone())
        })
        .ok_or_else(|| "invalid server id".to_string())?
        .map_err(|e| e.to_string())?;

    let (stream, _addr) = listener_clone.accept().map_err(|e| e.to_string())?;
    let peer_stream = stream.try_clone().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| e.to_string())?;
    let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
    let method = parts.first().copied().unwrap_or("");
    let path = parts.get(1).copied().unwrap_or("/");

    let mut headers_raw = String::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        headers_raw.push_str(trimmed);
        headers_raw.push('\n');
    }

    let mut body = String::new();
    if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        IoRead::read_exact(&mut reader, &mut buf).map_err(|e| e.to_string())?;
        body = String::from_utf8_lossy(&buf).into_owned();
    }

    let req_id = next_id();
    CONNS.with(|c| {
        c.borrow_mut().insert(
            req_id,
            ConnEntry {
                stream: peer_stream,
            },
        )
    });

    Ok(format!(
        "{}\n{}\n{}\n{}\n{}",
        req_id, method, path, headers_raw, body
    ))
}

fn do_respond(req_id: i64, status: i64, headers: &str, body: &str) -> Result<(), String> {
    let entry = CONNS
        .with(|c| c.borrow_mut().remove(&req_id))
        .ok_or_else(|| "invalid request id".to_string())?;

    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let mut response = format!("HTTP/1.1 {} {}\r\n", status, status_text);
    for line in headers.lines() {
        let line = line.trim();
        if !line.is_empty() {
            if let Some((name, value)) = line.split_once(':') {
                response.push_str(name.trim());
                response.push_str(": ");
                response.push_str(value.trim());
                response.push_str("\r\n");
            }
        }
    }
    response.push_str("Connection: close\r\n");
    response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    response.push_str("\r\n");
    response.push_str(body);

    let mut stream = entry.stream;
    stream
        .write_all(response.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn do_stop(server_id: i64) -> Result<(), String> {
    SERVERS
        .with(|s| s.borrow_mut().remove(&server_id))
        .ok_or_else(|| "invalid server id".to_string())?;
    Ok(())
}

// ── Linker registration ─────────────────────────────────────────────

/// Returns true if the WASM bytes import from the nexus-host HTTP module.
pub fn needs_net_host(wasm_bytes: &[u8]) -> bool {
    crate::runtime::conc::imports_module(wasm_bytes, NEXUS_HOST_HTTP_MODULE)
}

/// Add `nexus:cli/nexus-host` host functions to a core-wasm linker.
pub fn add_net_host_to_linker<T: 'static>(linker: &mut Linker<T>) -> Result<(), String> {
    // host-http-request(method_ptr, method_len, url_ptr, url_len,
    //                   headers_ptr, headers_len, body_ptr, body_len, ret_ptr)
    linker
        .func_wrap(
            NEXUS_HOST_HTTP_MODULE,
            "host-http-request",
            |mut caller: Caller<'_, T>,
             method_ptr: i32,
             method_len: i32,
             url_ptr: i32,
             url_len: i32,
             headers_ptr: i32,
             headers_len: i32,
             body_ptr: i32,
             body_len: i32,
             ret_ptr: i32| {
                let method =
                    read_wasm_string(&mut caller, method_ptr, method_len).unwrap_or_default();
                let url = read_wasm_string(&mut caller, url_ptr, url_len).unwrap_or_default();
                let headers =
                    read_wasm_string(&mut caller, headers_ptr, headers_len).unwrap_or_default();
                let body = read_wasm_string(&mut caller, body_ptr, body_len).unwrap_or_default();

                let result = do_http_request(&method, &url, &headers, &body);

                if let Some((ptr, len)) = write_wasm_bytes(&mut caller, result.as_bytes()) {
                    write_i32_pair(&mut caller, ret_ptr, ptr, len);
                } else {
                    write_i32_pair(&mut caller, ret_ptr, 0, 0);
                }
            },
        )
        .map_err(|e| e.to_string())?;

    // host-http-listen(addr_ptr, addr_len) -> i64
    linker
        .func_wrap(
            NEXUS_HOST_HTTP_MODULE,
            "host-http-listen",
            |mut caller: Caller<'_, T>, addr_ptr: i32, addr_len: i32| -> i64 {
                let addr = read_wasm_string(&mut caller, addr_ptr, addr_len).unwrap_or_default();
                do_listen(&addr).unwrap_or(-1)
            },
        )
        .map_err(|e| e.to_string())?;

    // host-http-accept(server_id, ret_ptr)
    linker
        .func_wrap(
            NEXUS_HOST_HTTP_MODULE,
            "host-http-accept",
            |mut caller: Caller<'_, T>, server_id: i64, ret_ptr: i32| {
                let result = do_accept(server_id).unwrap_or_else(|e| format!("-1\n\n\n\n{}", e));
                if let Some((ptr, len)) = write_wasm_bytes(&mut caller, result.as_bytes()) {
                    write_i32_pair(&mut caller, ret_ptr, ptr, len);
                } else {
                    write_i32_pair(&mut caller, ret_ptr, 0, 0);
                }
            },
        )
        .map_err(|e| e.to_string())?;

    // host-http-respond(req_id, status, headers_ptr, headers_len, body_ptr, body_len) -> i32
    linker
        .func_wrap(
            NEXUS_HOST_HTTP_MODULE,
            "host-http-respond",
            |mut caller: Caller<'_, T>,
             req_id: i64,
             status: i64,
             headers_ptr: i32,
             headers_len: i32,
             body_ptr: i32,
             body_len: i32|
             -> i32 {
                let headers =
                    read_wasm_string(&mut caller, headers_ptr, headers_len).unwrap_or_default();
                let body = read_wasm_string(&mut caller, body_ptr, body_len).unwrap_or_default();
                match do_respond(req_id, status, &headers, &body) {
                    Ok(()) => 1,
                    Err(_) => 0,
                }
            },
        )
        .map_err(|e| e.to_string())?;

    // host-http-stop(server_id) -> i32
    linker
        .func_wrap(
            NEXUS_HOST_HTTP_MODULE,
            "host-http-stop",
            |_caller: Caller<'_, T>, server_id: i64| -> i32 {
                match do_stop(server_id) {
                    Ok(()) => 1,
                    Err(_) => 0,
                }
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
