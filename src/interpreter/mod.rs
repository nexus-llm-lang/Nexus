//! Interpreter/runtime evaluator for Nexus AST.
//! REPL is colocated here because it is an interpreter-facing frontend.

pub mod repl;

use crate::lang::ast::*;
use crate::lang::stdlib::load_stdlib_nx_programs;
use crate::runtime::ExecutionCapabilities;
use chumsky::Parser;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read as IoRead, Write as IoWrite};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ureq::RequestExt;
use wasmtime::*;
use wasmtime_wasi::WasiCtxBuilder;

const NEXUS_HOST_HTTP_MODULE: &str = "nexus:cli/nexus-host";
const NEXUS_HOST_HTTP_FUNC: &str = "host-http-request";
const MAX_HTTP_URL_BYTES: usize = 8 * 1024;
const MAX_HTTP_HEADERS_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_FFI_STRING_BYTES: usize = 4 * 1024 * 1024;

struct ServerEntry {
    listener: TcpListener,
    _addr: String,
}

struct ConnectionEntry {
    stream: std::net::TcpStream,
}

type ServerTable = Arc<Mutex<Vec<Option<ServerEntry>>>>;
type ConnectionTable = Arc<Mutex<Vec<Option<ConnectionEntry>>>>;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Unit,
    Record(HashMap<String, Value>),
    Variant(String, Vec<Value>),
    Array(Arc<Mutex<Vec<Value>>>),
    Ref(Arc<Mutex<Value>>),
    NativeFunction(String),
    Function(String),
    Handler(String, Vec<Function>), // (coeffect_name, functions)
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Unit, Value::Unit) => true,
            (Value::Record(a), Value::Record(b)) => a == b,
            (Value::Variant(n1, a1), Value::Variant(n2, a2)) => n1 == n2 && a1 == a2,
            (Value::Array(a), Value::Array(b)) => match (a.lock(), b.lock()) {
                (Ok(a_lock), Ok(b_lock)) => *a_lock == *b_lock,
                _ => false,
            },
            (Value::Ref(a), Value::Ref(b)) => match (a.lock(), b.lock()) {
                (Ok(a_lock), Ok(b_lock)) => *a_lock == *b_lock,
                _ => false,
            },
            (Value::NativeFunction(a), Value::NativeFunction(b)) => a == b,
            (Value::Function(a), Value::Function(b)) => a == b,
            (Value::Handler(a, _), Value::Handler(b, _)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Unit => write!(f, "()"),
            Value::Record(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Variant(name, args) => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", a)?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Value::Array(a) => match a.lock() {
                Ok(lock) => {
                    write!(f, "[| ")?;
                    for (i, v) in lock.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, " |]")
                }
                Err(_) => write!(f, "[| <poisoned-array> |]"),
            },
            Value::Ref(_) => write!(f, "<ref>"),
            Value::NativeFunction(n) => write!(f, "<native fn {}>", n),
            Value::Function(n) => write!(f, "<fn {}>", n),
            Value::Handler(name, _) => write!(f, "<handler {}>", name),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExprResult {
    Normal(Value),
    EarlyReturn(Value),
}

#[derive(Debug, Clone)]
pub enum EvalError {
    Exception(Value),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Exception(v) => write!(f, "Unhandled exception: {}", v),
        }
    }
}

type EvalResult = Result<ExprResult, EvalError>;

fn perform_wasi_http_request(
    method: &str,
    url: &str,
    headers: &str,
    body: &str,
) -> Result<(u16, String), String> {
    let method = if method.trim().is_empty() {
        "GET"
    } else {
        method.trim()
    };
    let method = ureq::http::Method::from_bytes(method.trim().as_bytes())
        .map_err(|e| format!("invalid method: {}", e))?;
    let url = url.trim();
    if url.is_empty() {
        return Err("empty URL".to_string());
    }
    if url.len() > MAX_HTTP_URL_BYTES {
        return Err(format!("url exceeds {} bytes", MAX_HTTP_URL_BYTES));
    }
    if headers.len() > MAX_HTTP_HEADERS_BYTES {
        return Err(format!("headers exceed {} bytes", MAX_HTTP_HEADERS_BYTES));
    }
    if body.len() > MAX_HTTP_BODY_BYTES {
        return Err(format!("body exceeds {} bytes", MAX_HTTP_BODY_BYTES));
    }

    let mut request = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(url)
        .body(body.as_bytes().to_vec())
        .map_err(|e| format!("invalid request: {}", e))?;

    for line in headers.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let Ok(name) = ureq::http::header::HeaderName::from_bytes(name.trim().as_bytes()) else {
            continue;
        };
        let Ok(value) = ureq::http::HeaderValue::from_str(value.trim()) else {
            continue;
        };
        request.headers_mut().append(name, value);
    }

    let agent_config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .timeout_send_body(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = agent_config.into();

    let mut response = request
        .with_agent(&agent)
        .run()
        .map_err(|e| format!("{:?}", e))?;
    let status = response.status().as_u16();
    let response_body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("failed to read response body: {}", e))?;
    if response_body.len() > MAX_HTTP_RESPONSE_BYTES {
        return Err(format!(
            "response exceeds {} bytes",
            MAX_HTTP_RESPONSE_BYTES
        ));
    }
    Ok((status, response_body))
}

fn encode_http_bridge_error(message: impl AsRef<str>) -> String {
    format!("0\n{}", message.as_ref())
}

fn run_nexus_host_http_request(
    capabilities: &ExecutionCapabilities,
    method: &str,
    url: &str,
    headers: &str,
    body: &str,
) -> String {
    if let Err(err) = capabilities.ensure_url_allowed(url) {
        return encode_http_bridge_error(err);
    }
    match perform_wasi_http_request(method, url, headers, body) {
        Ok((status, response_body)) => format!("{}\n{}", status, response_body),
        Err(err) => encode_http_bridge_error(format!("http request failed: {}", err)),
    }
}

fn read_guest_string_from_memory(
    memory: &Memory,
    caller: &Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
    ptr: i32,
    len: i32,
) -> Option<String> {
    if len == 0 {
        return Some(String::new());
    }
    if len < 0 {
        return None;
    }
    if ptr < 0 {
        return None;
    }
    let len = len as usize;
    if len > MAX_FFI_STRING_BYTES {
        return None;
    }
    let mut bytes = vec![0u8; len];
    memory.read(caller, ptr as usize, &mut bytes).ok()?;
    match String::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) => Some(String::from_utf8_lossy(&e.into_bytes()).into_owned()),
    }
}

fn allocate_and_write_bytes(
    caller: &mut Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
    memory: &Memory,
    bytes: &[u8],
) -> Option<(i32, i32)> {
    if bytes.is_empty() {
        return Some((0, 0));
    }

    let len_i32 = i32::try_from(bytes.len()).ok()?;
    let allocate = caller.get_export("allocate").and_then(|e| e.into_func())?;
    let mut results = [Val::I32(0)];
    allocate
        .call(&mut *caller, &[Val::I32(len_i32)], &mut results)
        .ok()?;
    let ptr = match results[0] {
        Val::I32(ptr) if ptr > 0 => ptr,
        _ => return None,
    };
    memory.write(&mut *caller, ptr as usize, bytes).ok()?;
    Some((ptr, len_i32))
}

fn write_component_string_result(
    caller: &mut Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
    memory: &Memory,
    ret_ptr: i32,
    value: &str,
) {
    if ret_ptr < 0 {
        return;
    }
    let bytes = value.as_bytes();
    let (ptr, len) = match allocate_and_write_bytes(caller, memory, bytes) {
        Some((ptr, len)) => (ptr, len),
        None => (0, 0),
    };

    let mut pair = [0u8; 8];
    pair[..4].copy_from_slice(&ptr.to_le_bytes());
    pair[4..].copy_from_slice(&len.to_le_bytes());
    let _ = memory.write(&mut *caller, ret_ptr as usize, &pair);
}

fn parse_http_request(stream: &mut std::net::TcpStream) -> Option<(String, String, String, String)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);

    // Read request line: METHOD PATH HTTP/1.x\r\n
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let parts: Vec<&str> = request_line.trim_end().splitn(3, ' ').collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // Read headers until \r\n\r\n
    let mut headers = String::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
        if let Some(colon_pos) = trimmed.find(':') {
            let name = trimmed[..colon_pos].trim();
            let value = trimmed[colon_pos + 1..].trim();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
        }
        headers.push_str(trimmed);
        headers.push('\n');
    }

    // Read body based on Content-Length
    let mut body = String::new();
    if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        reader.read_exact(&mut buf).ok()?;
        body = String::from_utf8_lossy(&buf).to_string();
    }

    Some((method, path, headers, body))
}

fn add_nexus_host_to_linker(
    linker: &mut Linker<wasmtime_wasi::p1::WasiP1Ctx>,
    capabilities: ExecutionCapabilities,
) -> Result<(), String> {
    let server_table: ServerTable = Arc::new(Mutex::new(Vec::new()));
    let conn_table: ConnectionTable = Arc::new(Mutex::new(Vec::new()));

    // --- host-http-request (existing) ---
    linker
        .func_wrap(
            NEXUS_HOST_HTTP_MODULE,
            NEXUS_HOST_HTTP_FUNC,
            move |mut caller: Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
                  method_ptr: i32,
                  method_len: i32,
                  url_ptr: i32,
                  url_len: i32,
                  headers_ptr: i32,
                  headers_len: i32,
                  body_ptr: i32,
                  body_len: i32,
                  ret_ptr: i32| {
                let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
                    return;
                };
                if url_len < 0 || url_len as usize > MAX_HTTP_URL_BYTES {
                    let msg = encode_http_bridge_error(format!(
                        "url exceeds {} bytes",
                        MAX_HTTP_URL_BYTES
                    ));
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                }
                if headers_len < 0 || headers_len as usize > MAX_HTTP_HEADERS_BYTES {
                    let msg = encode_http_bridge_error(format!(
                        "headers exceed {} bytes",
                        MAX_HTTP_HEADERS_BYTES
                    ));
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                }
                if body_len < 0 || body_len as usize > MAX_HTTP_BODY_BYTES {
                    let msg = encode_http_bridge_error(format!(
                        "body exceeds {} bytes",
                        MAX_HTTP_BODY_BYTES
                    ));
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                }

                let Some(method) =
                    read_guest_string_from_memory(&memory, &caller, method_ptr, method_len)
                else {
                    let msg = encode_http_bridge_error("invalid method string");
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                };
                let Some(url) = read_guest_string_from_memory(&memory, &caller, url_ptr, url_len)
                else {
                    let msg = encode_http_bridge_error("invalid url string");
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                };
                let Some(headers) =
                    read_guest_string_from_memory(&memory, &caller, headers_ptr, headers_len)
                else {
                    let msg = encode_http_bridge_error("invalid headers string");
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                };
                let Some(body) =
                    read_guest_string_from_memory(&memory, &caller, body_ptr, body_len)
                else {
                    let msg = encode_http_bridge_error("invalid body string");
                    write_component_string_result(&mut caller, &memory, ret_ptr, &msg);
                    return;
                };

                let raw =
                    run_nexus_host_http_request(&capabilities, &method, &url, &headers, &body);
                write_component_string_result(&mut caller, &memory, ret_ptr, &raw);
            },
        )
        .map_err(|e| format!("Failed to add nexus host HTTP import: {}", e))?;

    // --- host-http-listen ---
    {
        let st = Arc::clone(&server_table);
        linker
            .func_wrap(
                NEXUS_HOST_HTTP_MODULE,
                "host-http-listen",
                move |mut caller: Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
                      addr_ptr: i32,
                      addr_len: i32|
                      -> i64 {
                    let Some(memory) =
                        caller.get_export("memory").and_then(|e| e.into_memory())
                    else {
                        return -1;
                    };
                    let Some(addr) =
                        read_guest_string_from_memory(&memory, &caller, addr_ptr, addr_len)
                    else {
                        return -1;
                    };
                    let listener = match TcpListener::bind(&addr) {
                        Ok(l) => l,
                        Err(_) => return -1,
                    };
                    let entry = ServerEntry {
                        listener,
                        _addr: addr,
                    };
                    let mut table = match st.lock() {
                        Ok(t) => t,
                        Err(_) => return -1,
                    };
                    // Reuse vacant slot
                    for (i, slot) in table.iter_mut().enumerate() {
                        if slot.is_none() {
                            *slot = Some(entry);
                            return i as i64;
                        }
                    }
                    let idx = table.len();
                    table.push(Some(entry));
                    idx as i64
                },
            )
            .map_err(|e| format!("Failed to add host-http-listen: {}", e))?;
    }

    // --- host-http-accept ---
    {
        let st = Arc::clone(&server_table);
        let ct = Arc::clone(&conn_table);
        linker
            .func_wrap(
                NEXUS_HOST_HTTP_MODULE,
                "host-http-accept",
                move |mut caller: Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
                      server_id: i64,
                      ret_ptr: i32| {
                    let Some(memory) =
                        caller.get_export("memory").and_then(|e| e.into_memory())
                    else {
                        return;
                    };

                    let listener_clone = {
                        let table = match st.lock() {
                            Ok(t) => t,
                            Err(_) => {
                                write_component_string_result(
                                    &mut caller, &memory, ret_ptr, "",
                                );
                                return;
                            }
                        };
                        let idx = server_id as usize;
                        match table.get(idx).and_then(|s| s.as_ref()) {
                            Some(entry) => match entry.listener.try_clone() {
                                Ok(l) => l,
                                Err(_) => {
                                    write_component_string_result(
                                        &mut caller, &memory, ret_ptr, "",
                                    );
                                    return;
                                }
                            },
                            None => {
                                write_component_string_result(
                                    &mut caller, &memory, ret_ptr, "",
                                );
                                return;
                            }
                        }
                    };

                    let (mut stream, _peer) = match listener_clone.accept() {
                        Ok(pair) => pair,
                        Err(_) => {
                            write_component_string_result(
                                &mut caller, &memory, ret_ptr, "",
                            );
                            return;
                        }
                    };

                    let (method, path, headers, body) =
                        match parse_http_request(&mut stream) {
                            Some(parsed) => parsed,
                            None => {
                                write_component_string_result(
                                    &mut caller, &memory, ret_ptr, "",
                                );
                                return;
                            }
                        };

                    // Store stream in connection table
                    let req_id = {
                        let mut ct_lock = match ct.lock() {
                            Ok(t) => t,
                            Err(_) => {
                                write_component_string_result(
                                    &mut caller, &memory, ret_ptr, "",
                                );
                                return;
                            }
                        };
                        let vacant = ct_lock.iter().position(|s| s.is_none());
                        let entry = ConnectionEntry { stream };
                        match vacant {
                            Some(i) => {
                                ct_lock[i] = Some(entry);
                                i as i64
                            }
                            None => {
                                let idx = ct_lock.len() as i64;
                                ct_lock.push(Some(entry));
                                idx
                            }
                        }
                    };

                    // Wire format: "req_id\nmethod\npath\nheaders\nbody"
                    let wire = format!("{}\n{}\n{}\n{}\n{}", req_id, method, path, headers, body);
                    write_component_string_result(&mut caller, &memory, ret_ptr, &wire);
                },
            )
            .map_err(|e| format!("Failed to add host-http-accept: {}", e))?;
    }

    // --- host-http-respond ---
    {
        let ct = Arc::clone(&conn_table);
        linker
            .func_wrap(
                NEXUS_HOST_HTTP_MODULE,
                "host-http-respond",
                move |mut caller: Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
                      req_id: i64,
                      status: i64,
                      headers_ptr: i32,
                      headers_len: i32,
                      body_ptr: i32,
                      body_len: i32|
                      -> i32 {
                    let Some(memory) =
                        caller.get_export("memory").and_then(|e| e.into_memory())
                    else {
                        return 0;
                    };
                    let headers_str =
                        read_guest_string_from_memory(&memory, &caller, headers_ptr, headers_len)
                            .unwrap_or_default();
                    let body_str =
                        read_guest_string_from_memory(&memory, &caller, body_ptr, body_len)
                            .unwrap_or_default();

                    let mut ct_lock = match ct.lock() {
                        Ok(t) => t,
                        Err(_) => return 0,
                    };
                    let idx = req_id as usize;
                    let entry = if idx < ct_lock.len() {
                        ct_lock[idx].take()
                    } else {
                        None
                    };
                    let Some(mut conn) = entry else {
                        return 0;
                    };

                    let reason = match status {
                        200 => "OK",
                        201 => "Created",
                        204 => "No Content",
                        301 => "Moved Permanently",
                        302 => "Found",
                        400 => "Bad Request",
                        403 => "Forbidden",
                        404 => "Not Found",
                        405 => "Method Not Allowed",
                        500 => "Internal Server Error",
                        _ => "OK",
                    };

                    let body_bytes = body_str.as_bytes();
                    let mut response = format!("HTTP/1.1 {} {}\r\n", status, reason);
                    // Add user-provided headers (newline-separated "name:value" pairs)
                    for line in headers_str.lines() {
                        if !line.is_empty() {
                            response.push_str(line);
                            response.push_str("\r\n");
                        }
                    }
                    response.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
                    response.push_str("\r\n");

                    if conn.stream.write_all(response.as_bytes()).is_err() {
                        return 0;
                    }
                    if conn.stream.write_all(body_bytes).is_err() {
                        return 0;
                    }
                    let _ = conn.stream.flush();
                    // Stream is dropped here, closing the connection
                    1
                },
            )
            .map_err(|e| format!("Failed to add host-http-respond: {}", e))?;
    }

    // --- host-http-stop ---
    {
        let st = Arc::clone(&server_table);
        linker
            .func_wrap(
                NEXUS_HOST_HTTP_MODULE,
                "host-http-stop",
                move |_caller: Caller<'_, wasmtime_wasi::p1::WasiP1Ctx>,
                      server_id: i64|
                      -> i32 {
                    let mut table = match st.lock() {
                        Ok(t) => t,
                        Err(_) => return 0,
                    };
                    let idx = server_id as usize;
                    if idx < table.len() && table[idx].is_some() {
                        table[idx].take(); // Drop listener
                        1
                    } else {
                        0
                    }
                },
            )
            .map_err(|e| format!("Failed to add host-http-stop: {}", e))?;
    }

    Ok(())
}

fn runtime_error(msg: impl Into<String>) -> EvalError {
    EvalError::Exception(Value::Variant(
        "RuntimeError".to_string(),
        vec![Value::String(msg.into())],
    ))
}

fn invalid_index_error(index: i64) -> EvalError {
    EvalError::Exception(Value::Variant(
        "InvalidIndex".to_string(),
        vec![Value::Int(index)],
    ))
}

#[derive(Debug, Clone)]
pub struct Env {
    vars: HashMap<String, Value>,
    parent: Option<Arc<Env>>,
}

impl Env {
    /// Creates an empty environment with no parent scope.
    pub fn new() -> Self {
        Env {
            vars: HashMap::new(),
            parent: None,
        }
    }

    /// Creates a child environment that can read bindings from `parent`.
    pub fn extend(parent: Env) -> Self {
        Env {
            vars: HashMap::new(),
            parent: Some(Arc::new(parent)),
        }
    }

    /// Looks up a value by name, searching parent scopes when necessary.
    pub fn get(&self, name: &str) -> Option<Value> {
        match self.vars.get(name) {
            Some(v) => Some(v.clone()),
            None => self.parent.as_ref().and_then(|p| p.get(name)),
        }
    }

    /// Defines or overwrites a binding in the current environment scope.
    pub fn define(&mut self, name: String, value: Value) {
        self.vars.insert(name, value);
    }
}

#[derive(Clone)]
pub struct Interpreter {
    pub functions: HashMap<String, Function>,
    pub enums: HashMap<String, EnumDef>,
    pub exceptions: HashMap<String, ExceptionDef>,
    pub closures: HashMap<String, Env>,
    pub top_level_values: HashMap<String, Value>,
    pub handlers: HashMap<String, Vec<Function>>,
    pub native_functions: HashMap<String, Arc<dyn Fn(&[Value]) -> EvalResult + Send + Sync>>,
    pub external_functions: HashMap<String, (String, Type)>, // wasm_name, type
    pub engine: Engine,
    pub wasm_modules: HashMap<String, Module>,
    pub wasm_store: Arc<Mutex<Store<wasmtime_wasi::p1::WasiP1Ctx>>>,
    pub wasm_instances: Arc<Mutex<Vec<Instance>>>,
    pub modules: HashMap<String, Interpreter>,
    pub lambda_counter: usize,
    pub init_error: Option<EvalError>,
}

impl Interpreter {
    /// Builds an interpreter instance and loads stdlib `.nx` modules.
    pub fn new(program: Program) -> Self {
        Self::new_with_capabilities(program, ExecutionCapabilities::permissive_legacy())
    }

    /// Builds an interpreter instance with an explicit capability policy.
    pub fn new_with_capabilities(program: Program, capabilities: ExecutionCapabilities) -> Self {
        Self::new_with_stdlib(program, true, capabilities)
    }

    fn new_with_stdlib(
        program: Program,
        load_stdlib: bool,
        capabilities: ExecutionCapabilities,
    ) -> Self {
        match Self::try_new_with_stdlib(program, load_stdlib, capabilities) {
            Ok(interpreter) => interpreter,
            Err(init_error) => Self::with_init_error(init_error),
        }
    }

    fn with_init_error(init_error: EvalError) -> Self {
        let engine = Engine::default();
        let wasi = WasiCtxBuilder::new().build_p1();
        let store = Store::new(&engine, wasi);
        Interpreter {
            functions: HashMap::new(),
            enums: HashMap::new(),
            exceptions: HashMap::new(),
            closures: HashMap::new(),
            top_level_values: HashMap::new(),
            handlers: HashMap::new(),
            native_functions: HashMap::new(),
            external_functions: HashMap::new(),
            engine,
            wasm_modules: HashMap::new(),
            wasm_store: Arc::new(Mutex::new(store)),
            wasm_instances: Arc::new(Mutex::new(Vec::new())),
            modules: HashMap::new(),
            lambda_counter: 0,
            init_error: Some(init_error),
        }
    }

    fn ensure_ready(&self) -> Result<(), EvalError> {
        if let Some(err) = &self.init_error {
            return Err(err.clone());
        }
        Ok(())
    }

    fn try_new_with_stdlib(
        program: Program,
        load_stdlib: bool,
        capabilities: ExecutionCapabilities,
    ) -> Result<Self, EvalError> {
        let mut functions = HashMap::new();
        let mut enums = HashMap::new();
        let mut exceptions = HashMap::new();
        let mut top_level_values = HashMap::new();
        let handlers = HashMap::new();
        let mut external_functions = HashMap::new();
        let mut modules = HashMap::new();
        let native_functions: HashMap<String, Arc<dyn Fn(&[Value]) -> EvalResult + Send + Sync>> =
            HashMap::new();

        let engine = Engine::default();
        let mut wasm_modules = HashMap::new();

        let mut builder = WasiCtxBuilder::new();
        builder.inherit_stdio();
        capabilities
            .apply_to_wasi_builder(&mut builder)
            .map_err(|e| runtime_error(format!("Failed to apply capability policy: {}", e)))?;
        let wasi = builder.build_p1();
        let mut store = Store::new(&engine, wasi);
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s| s)
            .map_err(|e| runtime_error(format!("Failed to add WASI to linker: {}", e)))?;
        add_nexus_host_to_linker(&mut linker, capabilities.clone())
            .map_err(|e| runtime_error(format!("Failed to add nexus host HTTP import: {}", e)))?;

        let mut wasm_instances = Vec::new();

        let mut all_definitions = Vec::new();
        if load_stdlib {
            let stdlib_programs = load_stdlib_nx_programs()
                .map_err(|e| runtime_error(format!("Failed to load stdlib sources: {}", e)))?;
            for (_, stdlib_program) in stdlib_programs {
                all_definitions.extend(stdlib_program.definitions);
            }
        }
        all_definitions.extend(program.definitions);

        for def in &all_definitions {
            match &def.node {
                TopLevel::Let(gl) => match &gl.value.node {
                    Expr::Lambda {
                        type_params,
                        params,
                        ret_type,
                        requires,
                        effects,
                        body,
                    } => {
                        functions.insert(
                            gl.name.clone(),
                            Function {
                                name: gl.name.clone(),
                                is_public: gl.is_public,
                                type_params: type_params.clone(),
                                params: params.clone(),
                                ret_type: ret_type.clone(),
                                requires: requires.clone(),
                                effects: effects.clone(),
                                body: body.clone(),
                            },
                        );
                    }
                    Expr::External(wasm_name, _, typ) => {
                        external_functions
                            .insert(gl.name.clone(), (wasm_name.clone(), typ.clone()));
                    }
                    Expr::Handler {
                        coeffect_name,
                        functions,
                    } => {
                        top_level_values.insert(
                            gl.name.clone(),
                            Value::Handler(coeffect_name.clone(), functions.clone()),
                        );
                    }
                    _ => {}
                },
                TopLevel::Enum(ed) => {
                    enums.insert(ed.name.clone(), ed.clone());
                }
                TopLevel::Exception(ex) => {
                    exceptions.insert(ex.name.clone(), ex.clone());
                }
                // Handlers are now expression-level (coeffect model);
                // they get registered via inject blocks at runtime.
                TopLevel::Import(import) => {
                    if import.is_external {
                        let module = Module::from_file(&engine, &import.path).map_err(|e| {
                            runtime_error(format!(
                                "Failed to load wasm module '{}': {}",
                                import.path, e
                            ))
                        })?;
                        wasm_modules.insert(import.path.clone(), module.clone());
                        let instance = linker.instantiate(&mut store, &module).map_err(|e| {
                            runtime_error(format!(
                                "Failed to instantiate wasm module '{}': {}",
                                import.path, e
                            ))
                        })?;
                        wasm_instances.push(instance);
                    } else {
                        let src = std::fs::read_to_string(&import.path).map_err(|e| {
                            runtime_error(format!("Failed to read module '{}': {}", import.path, e))
                        })?;
                        let p = crate::lang::parser::parser().parse(src).map_err(|errs| {
                            runtime_error(format!(
                                "Failed to parse module '{}': {:?}",
                                import.path, errs
                            ))
                        })?;

                        // Imported modules should not recursively preload stdlib again.
                        // They can still import what they need explicitly.
                        let mut sub_interp =
                            Interpreter::try_new_with_stdlib(p, false, capabilities.clone())?;
                        // Imported modules still need constructor metadata (e.g. List/Nil/Cons)
                        // that is preloaded in the parent interpreter.
                        for (name, ed) in &enums {
                            sub_interp
                                .enums
                                .entry(name.clone())
                                .or_insert_with(|| ed.clone());
                        }
                        for (name, ex) in &exceptions {
                            sub_interp
                                .exceptions
                                .entry(name.clone())
                                .or_insert_with(|| ex.clone());
                        }

                        if !import.items.is_empty() {
                            for item in &import.items {
                                if let Some(f) = sub_interp.functions.get(item) {
                                    functions.insert(item.clone(), f.clone());
                                }
                                if let Some(f) = sub_interp.external_functions.get(item) {
                                    external_functions.insert(item.clone(), f.clone());
                                }
                                if let Some(v) = sub_interp.top_level_values.get(item) {
                                    top_level_values.insert(item.clone(), v.clone());
                                }
                            }
                        } else {
                            let alias = import.alias.clone().unwrap_or_else(|| {
                                std::path::Path::new(&import.path)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or(&import.path)
                                    .to_string()
                            });
                            modules.insert(alias, sub_interp);
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(Interpreter {
            functions,
            enums,
            exceptions,
            closures: HashMap::new(),
            top_level_values,
            handlers,
            native_functions,
            external_functions,
            engine,
            wasm_modules,
            wasm_store: Arc::new(Mutex::new(store)),
            wasm_instances: Arc::new(Mutex::new(wasm_instances)),
            modules,
            lambda_counter: 0,
            init_error: None,
        })
    }

    fn spawn_task_interpreter(&self) -> Self {
        Interpreter {
            functions: self.functions.clone(),
            enums: self.enums.clone(),
            exceptions: self.exceptions.clone(),
            closures: self.closures.clone(),
            top_level_values: self.top_level_values.clone(),
            handlers: self.handlers.clone(),
            native_functions: self.native_functions.clone(),
            external_functions: self.external_functions.clone(),
            engine: self.engine.clone(),
            wasm_modules: self.wasm_modules.clone(),
            wasm_store: Arc::clone(&self.wasm_store),
            wasm_instances: Arc::clone(&self.wasm_instances),
            modules: self.modules.clone(),
            lambda_counter: self.lambda_counter,
            init_error: self.init_error.clone(),
        }
    }

    /// Evaluates one REPL statement against an existing environment.
    pub fn eval_repl_stmt(&mut self, stmt: &Spanned<Stmt>, env: &mut Env) -> EvalResult {
        self.ensure_ready()?;
        match &stmt.node {
            Stmt::Expr(expr) => self.eval_expr(expr, env),
            _ => self.eval_body(&[stmt.clone()], env),
        }
    }

    fn register_lambda(
        &mut self,
        function: Function,
        mut captured_env: Env,
        self_binding: Option<String>,
    ) -> String {
        let name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;

        let mut lambda_fn = function;
        lambda_fn.name = name.clone();
        if let Some(binding_name) = self_binding {
            captured_env.define(binding_name, Value::Function(name.clone()));
        }
        self.functions.insert(name.clone(), lambda_fn);
        self.closures.insert(name.clone(), captured_env);
        name
    }

    fn labeled_to_positional_for_params(
        params: &[Param],
        labeled_args: &[(String, Value)],
        callee: &str,
    ) -> Result<Vec<Value>, EvalError> {
        if params.len() != labeled_args.len() {
            return Err(runtime_error(format!(
                "Arity mismatch in {}: expected {}, got {}",
                callee,
                params.len(),
                labeled_args.len()
            )));
        }

        let mut remaining = labeled_args.to_vec();
        let mut ordered = Vec::with_capacity(params.len());
        for param in params {
            let idx = remaining
                .iter()
                .position(|(label, _)| label == &param.name)
                .ok_or_else(|| {
                    runtime_error(format!("Missing label '{}' in {}", param.name, callee))
                })?;
            let (_, value) = remaining.remove(idx);
            ordered.push(value);
        }

        if let Some((extra, _)) = remaining.first() {
            return Err(runtime_error(format!(
                "Unknown label '{}' in call to {}",
                extra, callee
            )));
        }

        Ok(ordered)
    }

    fn labeled_to_positional_for_arrow_params(
        params: &[(String, Type)],
        labeled_args: &[(String, Value)],
        callee: &str,
    ) -> Result<Vec<Value>, EvalError> {
        if params.len() != labeled_args.len() {
            return Err(runtime_error(format!(
                "Arity mismatch in {}: expected {}, got {}",
                callee,
                params.len(),
                labeled_args.len()
            )));
        }

        let mut remaining = labeled_args.to_vec();
        let mut ordered = Vec::with_capacity(params.len());
        for (p_name, _) in params {
            let idx = remaining
                .iter()
                .position(|(label, _)| label == p_name)
                .ok_or_else(|| {
                    runtime_error(format!("Missing label '{}' in {}", p_name, callee))
                })?;
            let (_, value) = remaining.remove(idx);
            ordered.push(value);
        }

        if let Some((extra, _)) = remaining.first() {
            return Err(runtime_error(format!(
                "Unknown label '{}' in call to {}",
                extra, callee
            )));
        }

        Ok(ordered)
    }

    fn labeled_to_positional_call_order(labeled_args: &[(String, Value)]) -> Vec<Value> {
        labeled_args.iter().map(|(_, v)| v.clone()).collect()
    }

    /// Executes a named Nexus function with already-evaluated arguments.
    pub fn run_function(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        self.ensure_ready().map_err(|e| e.to_string())?;
        let func = self
            .functions
            .get(name)
            .ok_or_else(|| format!("Function '{}' not found", name))?
            .clone();

        if func.params.len() != args.len() {
            return Err(format!(
                "Arity mismatch: expected {}, got {}",
                func.params.len(),
                args.len()
            ));
        }

        let mut env = if let Some(captured_env) = self.closures.get(name).cloned() {
            Env::extend(captured_env)
        } else {
            Env::new()
        };
        for (global_name, global_value) in &self.top_level_values {
            if env.get(global_name).is_none() {
                env.define(global_name.clone(), global_value.clone());
            }
        }
        for (param, arg) in func.params.iter().zip(args.iter()) {
            env.define(param.name.clone(), arg.clone());
        }

        let result = self
            .eval_body(&func.body, &mut env)
            .map_err(|e| e.to_string())?;
        match result {
            ExprResult::Normal(v) => Ok(v),
            ExprResult::EarlyReturn(v) => Ok(v),
        }
    }

    fn run_external_function(&self, wasm_name: &str, typ: &Type, args: Vec<Value>) -> EvalResult {
        self.ensure_ready()?;
        let mut store = self
            .wasm_store
            .lock()
            .map_err(|_| runtime_error("Wasm store lock poisoned"))?;

        let mut func_with_inst = None;
        let instances = self
            .wasm_instances
            .lock()
            .map_err(|_| runtime_error("Wasm instance lock poisoned"))?;
        for instance in instances.iter() {
            if let Some(f) = instance.get_func(&mut *store, wasm_name) {
                func_with_inst = Some((f, *instance));
                break;
            }
        }

        let (params, ret_type) = if let Type::Arrow(p, r, _, _) = typ {
            (p, r)
        } else {
            return Err(runtime_error("External function must have arrow type"));
        };

        if args.len() != params.len() {
            return Err(runtime_error(format!(
                "Arity mismatch: expected {}, got {}",
                params.len(),
                args.len()
            )));
        }

        let (func, func_instance) = if let Some(found) = func_with_inst {
            found
        } else {
            return Err(runtime_error(format!(
                "Wasm function {} not found in any loaded instance",
                wasm_name
            )));
        };

        let mut wasm_args = Vec::new();
        let mut transient_allocations: Vec<(i32, i32)> = Vec::new();
        let build_args_result: Result<(), EvalError> = (|| {
            for ((p_name, p_type), v) in params.iter().zip(args.into_iter()) {
                match (p_type, v) {
                    (Type::I32, Value::Int(i)) => {
                        let converted = i32::try_from(i).map_err(|_| {
                            runtime_error(format!(
                                "Parameter '{}' expects i32, but {} overflows i32",
                                p_name, i
                            ))
                        })?;
                        wasm_args.push(Val::I32(converted));
                    }
                    (Type::Bool, Value::Bool(b)) => wasm_args.push(Val::I32(if b { 1 } else { 0 })),
                    (Type::I64, Value::Int(i)) => wasm_args.push(Val::I64(i)),
                    (Type::F32, Value::Float(f)) => wasm_args.push(Val::F32((f as f32).to_bits())),
                    (Type::F64, Value::Float(f)) => wasm_args.push(Val::F64(f.to_bits())),
                    (Type::String, Value::String(s)) => {
                        let (ptr, len) =
                            self.pass_string_to_wasm(&s, &mut *store, &func_instance)?;
                        transient_allocations.push((ptr, len));
                        wasm_args.push(Val::I32(ptr));
                        wasm_args.push(Val::I32(len));
                    }
                    (Type::Array(_), Value::Array(arr)) => {
                        let lock = arr.lock().map_err(|_| {
                            runtime_error(format!("Parameter '{}' array lock poisoned", p_name))
                        })?;
                        let len = i32::try_from(lock.len()).map_err(|_| {
                            runtime_error(format!(
                                "Parameter '{}' array length overflows i32",
                                p_name
                            ))
                        })?;
                        // Current ABI passes array metadata only.
                        wasm_args.push(Val::I32(0));
                        wasm_args.push(Val::I32(len));
                    }
                    (Type::Borrow(inner), Value::Array(arr))
                        if matches!(inner.as_ref(), Type::Array(_)) =>
                    {
                        let lock = arr.lock().map_err(|_| {
                            runtime_error(format!("Parameter '{}' array lock poisoned", p_name))
                        })?;
                        let len = i32::try_from(lock.len()).map_err(|_| {
                            runtime_error(format!(
                                "Parameter '{}' array length overflows i32",
                                p_name
                            ))
                        })?;
                        wasm_args.push(Val::I32(0));
                        wasm_args.push(Val::I32(len));
                    }
                    (expected, actual) => {
                        return Err(runtime_error(format!(
                            "Unsupported FFI arg for '{}': expected {}, got {:?}",
                            p_name, expected, actual
                        )))
                    }
                }
            }
            Ok(())
        })();
        if let Err(err) = build_args_result {
            let _ = self.cleanup_transient_allocations(
                &transient_allocations,
                &mut *store,
                &func_instance,
            );
            return Err(err);
        }

        let mut results: Vec<Val> = func
            .ty(&mut *store)
            .results()
            .map(|vt| match vt {
                ValType::I32 => Val::I32(0),
                ValType::I64 => Val::I64(0),
                ValType::F32 => Val::F32(0),
                ValType::F64 => Val::F64(0),
                _ => Val::I64(0),
            })
            .collect();
        let call_result = func.call(&mut *store, &wasm_args, &mut results);
        let cleanup_result =
            self.cleanup_transient_allocations(&transient_allocations, &mut *store, &func_instance);
        if let Err(e) = call_result {
            return Err(runtime_error(format!("Wasm call failed: {}", e)));
        }
        cleanup_result?;

        if results.is_empty() {
            Ok(ExprResult::Normal(Value::Unit))
        } else {
            match (ret_type.as_ref(), results[0].clone()) {
                (Type::I32, Val::I32(i)) => Ok(ExprResult::Normal(Value::Int(i as i64))),
                (Type::Bool, Val::I32(i)) => Ok(ExprResult::Normal(Value::Bool(i != 0))),
                (Type::I64, Val::I64(i)) => Ok(ExprResult::Normal(Value::Int(i))),
                (Type::F32, Val::F32(f)) => {
                    Ok(ExprResult::Normal(Value::Float(f32::from_bits(f) as f64)))
                }
                (Type::F64, Val::F64(f)) => Ok(ExprResult::Normal(Value::Float(f64::from_bits(f)))),
                (Type::String, Val::I64(packed)) => {
                    let s = self.read_string_from_wasm(packed, &mut *store, &func_instance)?;
                    Ok(ExprResult::Normal(Value::String(s)))
                }
                (Type::Unit, _) => Ok(ExprResult::Normal(Value::Unit)),
                (expected, actual) => Err(runtime_error(format!(
                    "Wasm return type mismatch: declared {}, actual {:?}",
                    expected, actual
                ))),
            }
        }
    }

    fn deallocate_wasm_memory(
        &self,
        ptr: i32,
        len: i32,
        store: &mut Store<wasmtime_wasi::p1::WasiP1Ctx>,
        instance: &Instance,
    ) -> Result<(), EvalError> {
        if ptr == 0 || len <= 0 {
            return Ok(());
        }
        let dealloc = instance.get_func(&mut *store, "deallocate").ok_or_else(|| {
            runtime_error(
                "Wasm instance must export 'deallocate(ptr: i32, size: i32) -> unit' for FFI strings",
            )
        })?;
        dealloc
            .call(&mut *store, &[Val::I32(ptr), Val::I32(len)], &mut [])
            .map_err(|e| runtime_error(format!("deallocate failed: {}", e)))
    }

    fn cleanup_transient_allocations(
        &self,
        allocations: &[(i32, i32)],
        store: &mut Store<wasmtime_wasi::p1::WasiP1Ctx>,
        instance: &Instance,
    ) -> Result<(), EvalError> {
        for &(ptr, len) in allocations {
            self.deallocate_wasm_memory(ptr, len, store, instance)?;
        }
        Ok(())
    }

    fn pass_string_to_wasm(
        &self,
        s: &str,
        store: &mut Store<wasmtime_wasi::p1::WasiP1Ctx>,
        instance: &Instance,
    ) -> Result<(i32, i32), EvalError> {
        if s.len() > MAX_FFI_STRING_BYTES {
            return Err(runtime_error(format!(
                "ffi string argument exceeds {} bytes",
                MAX_FFI_STRING_BYTES
            )));
        }
        let len = i32::try_from(s.len())
            .map_err(|_| runtime_error("string argument length overflows i32"))?;
        let alloc = instance.get_func(&mut *store, "allocate").ok_or_else(|| {
            runtime_error("Wasm instance must export 'allocate(i32) -> i32' to receive strings")
        })?;

        let mut results = [Val::I32(0)];
        alloc
            .call(&mut *store, &[Val::I32(len)], &mut results)
            .map_err(|e| runtime_error(format!("allocate failed: {}", e)))?;

        let ptr = match results[0] {
            Val::I32(p) => p,
            _ => return Err(runtime_error("allocate must return i32")),
        };
        if ptr == 0 && len > 0 {
            return Err(runtime_error(
                "allocate returned null pointer for non-empty string",
            ));
        }

        let mem = instance
            .get_memory(&mut *store, "memory")
            .ok_or_else(|| runtime_error("Wasm instance must export 'memory'"))?;

        if let Err(e) = mem.write(&mut *store, ptr as usize, s.as_bytes()) {
            let _ = self.deallocate_wasm_memory(ptr, len, store, instance);
            return Err(runtime_error(format!("memory write failed: {}", e)));
        }

        Ok((ptr, len))
    }

    fn read_string_from_wasm(
        &self,
        packed: i64,
        store: &mut Store<wasmtime_wasi::p1::WasiP1Ctx>,
        instance: &Instance,
    ) -> Result<String, EvalError> {
        let raw = packed as u64;
        let ptr_bits = (raw >> 32) as u32;
        let len = (raw & 0xFFFF_FFFF) as usize;
        if len == 0 {
            return Ok(String::new());
        }
        if len > MAX_FFI_STRING_BYTES {
            return Err(runtime_error(format!(
                "ffi string result exceeds {} bytes",
                MAX_FFI_STRING_BYTES
            )));
        }
        let len_i32 =
            i32::try_from(len).map_err(|_| runtime_error("ffi string length overflows i32"))?;
        let ptr_i32 = ptr_bits as i32;

        let mem = instance
            .get_memory(&mut *store, "memory")
            .ok_or_else(|| runtime_error("Wasm instance must export 'memory'"))?;
        let start = ptr_bits as usize;
        let end = start
            .checked_add(len)
            .ok_or_else(|| runtime_error("ffi string pointer range overflow"))?;
        let decoded_result = {
            let data = mem.data(&*store);
            match data.get(start..end) {
                Some(bytes) => std::str::from_utf8(bytes)
                    .map(|s| s.to_owned())
                    .map_err(|e| runtime_error(format!("invalid utf-8 from wasm: {}", e))),
                None => Err(runtime_error("memory read failed: out of bounds")),
            }
        };
        let dealloc_result = self.deallocate_wasm_memory(ptr_i32, len_i32, store, instance);
        let decoded = decoded_result?;
        dealloc_result?;
        Ok(decoded)
    }

    fn eval_body(&mut self, body: &[Spanned<Stmt>], env: &mut Env) -> EvalResult {
        for stmt in body {
            match &stmt.node {
                Stmt::Let {
                    name, sigil, value, ..
                } => {
                    if let Expr::Lambda {
                        type_params,
                        params,
                        ret_type,
                        requires,
                        effects,
                        body,
                    } = &value.node
                    {
                        let self_binding = if matches!(sigil, Sigil::Immutable) {
                            Some(name.clone())
                        } else {
                            None
                        };
                        let fn_name = self.register_lambda(
                            Function {
                                name: String::new(),
                                is_public: false,
                                type_params: type_params.clone(),
                                params: params.clone(),
                                ret_type: ret_type.clone(),
                                requires: requires.clone(),
                                effects: effects.clone(),
                                body: body.clone(),
                            },
                            env.clone(),
                            self_binding,
                        );
                        let val = Value::Function(fn_name);
                        let final_val = if let Sigil::Mutable = sigil {
                            Value::Ref(Arc::new(Mutex::new(val)))
                        } else {
                            val
                        };
                        env.define(sigil.get_key(name), final_val);
                        continue;
                    }

                    let res = self.eval_expr(value, env)?;
                    match res {
                        ExprResult::Normal(val) => {
                            let final_val = if let Sigil::Mutable = sigil {
                                Value::Ref(Arc::new(Mutex::new(val)))
                            } else {
                                val
                            };
                            env.define(sigil.get_key(name), final_val);
                        }
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }

                Stmt::Return(expr) => {
                    let res = self.eval_expr(expr, env)?;
                    match res {
                        ExprResult::Normal(val) => return Ok(ExprResult::EarlyReturn(val)),
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }
                Stmt::Expr(expr) => {
                    let res = self.eval_expr(expr, env)?;
                    if let ExprResult::EarlyReturn(_) = res {
                        return Ok(res);
                    }
                }
                Stmt::Conc(tasks) => {
                    let mut thread_handles = Vec::new();
                    let task_runtime = self.spawn_task_interpreter();
                    for task in tasks {
                        let task_interp = task_runtime.clone();
                        let mut task_env = env.clone();
                        let task_node = task.clone();
                        let handle = std::thread::spawn(move || {
                            let mut task_interp = task_interp;
                            task_interp.eval_body(&task_node.body, &mut task_env)
                        });
                        thread_handles.push(handle);
                    }
                    for handle in thread_handles {
                        match handle.join() {
                            Ok(res) => {
                                if let Err(e) = res {
                                    return Err(e);
                                }
                            }
                            Err(_) => return Err(runtime_error("Task panicked")),
                        }
                    }
                }
                Stmt::Try {
                    body,
                    catch_param,
                    catch_body,
                } => {
                    let res = self.eval_body(body, env);
                    match res {
                        Ok(ExprResult::EarlyReturn(val)) => {
                            return Ok(ExprResult::EarlyReturn(val))
                        }
                        Ok(ExprResult::Normal(_)) => {}
                        Err(EvalError::Exception(exn)) => {
                            let mut catch_env = Env::extend(env.clone());
                            catch_env.define(catch_param.clone(), exn);
                            let catch_res = self.eval_body(catch_body, &mut catch_env)?;
                            if let ExprResult::EarlyReturn(v) = catch_res {
                                return Ok(ExprResult::EarlyReturn(v));
                            }
                        }
                    }
                }
                Stmt::Assign { target, value } => {
                    let val_res = self.eval_expr(value, env)?;
                    let val = match val_res {
                        ExprResult::Normal(v) => v,
                        ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                    };

                    match &target.node {
                        Expr::Variable(name, sigil) => {
                            let key = sigil.get_key(name);
                            if let Some(target_val) = env.get(&key) {
                                if let Value::Ref(r) = target_val {
                                    let mut lock = r.lock().map_err(|_| {
                                        runtime_error(format!(
                                            "Mutable reference '{}' lock poisoned",
                                            name
                                        ))
                                    })?;
                                    *lock = val;
                                } else {
                                    return Err(runtime_error(format!(
                                        "Cannot assign to immutable variable {}",
                                        name
                                    )));
                                }
                            } else {
                                return Err(runtime_error(format!("Variable {} not found", key)));
                            }
                        }
                        Expr::Index(arr, idx) => {
                            let arr_res = self.eval_expr(arr, env)?;
                            let idx_res = self.eval_expr(idx, env)?;
                            match (arr_res, idx_res) {
                                (
                                    ExprResult::Normal(Value::Array(a)),
                                    ExprResult::Normal(Value::Int(i)),
                                ) => {
                                    if i < 0 {
                                        return Err(invalid_index_error(i));
                                    }
                                    let mut l = a
                                        .lock()
                                        .map_err(|_| runtime_error("Array lock poisoned"))?;
                                    let idx = i as usize;
                                    if idx < l.len() {
                                        l[idx] = val;
                                    } else {
                                        return Err(invalid_index_error(i));
                                    }
                                }
                                (ExprResult::EarlyReturn(v), _)
                                | (_, ExprResult::EarlyReturn(v)) => {
                                    return Ok(ExprResult::EarlyReturn(v))
                                }
                                _ => return Err(runtime_error("Invalid array assignment")),
                            }
                        }
                        _ => return Err(runtime_error("Invalid assignment target")),
                    }
                }
                Stmt::Inject { handlers, body } => {
                    // Save current handlers, push injected ones, evaluate body, restore
                    let saved = self.handlers.clone();
                    for handler_name in handlers {
                        let key = handler_name.clone();
                        if let Some(val) = env.get(&key) {
                            if let Value::Handler(coeffect_name, fns) = val {
                                self.handlers.insert(coeffect_name.clone(), fns.clone());
                            }
                        }
                    }
                    let res = self.eval_body(body, env);
                    self.handlers = saved;
                    match res? {
                        ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                        ExprResult::Normal(_) => {}
                    }
                }
                Stmt::Comment => continue,
            }
        }
        Ok(ExprResult::Normal(Value::Unit))
    }

    fn get_variant_fields(&self, name: &str) -> Option<Vec<(Option<String>, Type)>> {
        for ed in self.enums.values() {
            if let Some(v) = ed.variants.iter().find(|v| v.name == name) {
                return Some(v.fields.clone());
            }
        }
        if let Some(ex) = self.exceptions.get(name) {
            return Some(ex.fields.clone());
        }
        if name == "RuntimeError" {
            return Some(vec![(Some("val".into()), Type::String)]);
        }
        if name == "InvalidIndex" {
            return Some(vec![(Some("val".into()), Type::I64)]);
        }
        None
    }

    fn eval_expr(&mut self, expr: &Spanned<Expr>, env: &mut Env) -> EvalResult {
        match &expr.node {
            Expr::Literal(lit) => Ok(ExprResult::Normal(match lit {
                Literal::Int(i) => Value::Int(*i),
                Literal::Float(f) => Value::Float(*f),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::String(s) => Value::String(s.clone()),
                Literal::Unit => Value::Unit,
            })),
            Expr::Variable(name, sigil) => {
                let key = sigil.get_key(name);
                if let Some(val) = env.get(&key) {
                    match (sigil, &val) {
                        (Sigil::Mutable, Value::Ref(r)) => {
                            let lock = r.lock().map_err(|_| {
                                runtime_error(format!("Mutable reference '{}' lock poisoned", name))
                            })?;
                            return Ok(ExprResult::Normal(lock.clone()));
                        }
                        (Sigil::Mutable, _) => {
                            return Err(runtime_error(format!(
                                "Variable {} is not a ref, cannot dereference with ~",
                                name
                            )))
                        }
                        _ => return Ok(ExprResult::Normal(val)),
                    }
                }
                if self.functions.contains_key(&key) {
                    return Ok(ExprResult::Normal(Value::Function(key)));
                }
                if self.native_functions.contains_key(&key) {
                    return Ok(ExprResult::Normal(Value::NativeFunction(key)));
                }
                Err(runtime_error(format!("Variable '{}' not found", key)))
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                let l = self.eval_expr(lhs, env)?;
                let r = self.eval_expr(rhs, env)?;
                match (l, r) {
                    (ExprResult::Normal(l_val), ExprResult::Normal(r_val)) => {
                        match (l_val, op.as_str(), r_val) {
                            (Value::Int(a), "+", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a + b)))
                            }
                            (Value::String(a), "++", Value::String(b)) => {
                                Ok(ExprResult::Normal(Value::String(a + &b)))
                            }
                            (Value::Int(a), "-", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a - b)))
                            }
                            (Value::Int(a), "*", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a * b)))
                            }
                            (Value::Int(a), "/", Value::Int(b)) => {
                                if b == 0 {
                                    Err(runtime_error("division by zero"))
                                } else {
                                    Ok(ExprResult::Normal(Value::Int(a / b)))
                                }
                            }
                            (Value::Int(a), "==", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a == b)))
                            }
                            (Value::Int(a), "!=", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a != b)))
                            }
                            (Value::Int(a), "<", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a < b)))
                            }
                            (Value::Int(a), ">", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a > b)))
                            }
                            (Value::Int(a), "<=", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a <= b)))
                            }
                            (Value::Int(a), ">=", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a >= b)))
                            }
                            (Value::Float(a), "+.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a + b)))
                            }
                            (Value::Float(a), "-.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a - b)))
                            }
                            (Value::Float(a), "*.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a * b)))
                            }
                            (Value::Float(a), "/.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a / b)))
                            }
                            (Value::Float(a), "==.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a == b)))
                            }
                            (Value::Float(a), "!=.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a != b)))
                            }
                            (Value::Float(a), "<.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a < b)))
                            }
                            (Value::Float(a), ">.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a > b)))
                            }
                            (Value::Float(a), "<=.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a <= b)))
                            }
                            (Value::Float(a), ">=.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a >= b)))
                            }
                            (Value::String(a), "+", Value::String(b)) => {
                                Ok(ExprResult::Normal(Value::String(a + &b)))
                            }
                            (l, op, r) => Err(runtime_error(format!(
                                "Invalid binary op: {:?} {} {:?}",
                                l, op, r
                            ))),
                        }
                    }
                    (ExprResult::EarlyReturn(v), _) | (_, ExprResult::EarlyReturn(v)) => {
                        Ok(ExprResult::EarlyReturn(v))
                    }
                }
            }
            Expr::Borrow(name, sigil) => {
                let key = sigil.get_key(name);
                let val = env
                    .get(&key)
                    .ok_or_else(|| runtime_error(format!("Variable '{}' not found", key)))?;
                Ok(ExprResult::Normal(val))
            }
            Expr::Call { func, args, .. } => {
                let mut evaluated_args: Vec<(String, Value)> = Vec::new();
                for (label, arg_expr) in args {
                    let res = self.eval_expr(arg_expr, env)?;
                    match res {
                        ExprResult::Normal(val) => evaluated_args.push((label.clone(), val)),
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }

                if let Some(val) = env.get(func) {
                    match val {
                        Value::NativeFunction(name) => {
                            if let Some(f) = self.native_functions.get(&name) {
                                let positional =
                                    Interpreter::labeled_to_positional_call_order(&evaluated_args);
                                return f(&positional);
                            } else {
                                return Err(runtime_error(format!(
                                    "Native function '{}' not found",
                                    name
                                )));
                            }
                        }
                        Value::Function(name) => {
                            let target = self
                                .functions
                                .get(&name)
                                .ok_or_else(|| {
                                    runtime_error(format!("Function '{}' not found", name))
                                })?
                                .clone();
                            let positional = Interpreter::labeled_to_positional_for_params(
                                &target.params,
                                &evaluated_args,
                                &name,
                            )?;
                            let res = self
                                .run_function(&name, positional)
                                .map_err(runtime_error)?;
                            return Ok(ExprResult::Normal(res));
                        }
                        _ => {}
                    }
                }

                if let Some(f) = self.native_functions.get(func) {
                    let positional = Interpreter::labeled_to_positional_call_order(&evaluated_args);
                    return f(&positional);
                }

                if let Some((wasm_name, typ)) = self.external_functions.get(func).cloned() {
                    let positional = if let Type::Arrow(params, _, _, _) = &typ {
                        Interpreter::labeled_to_positional_for_arrow_params(
                            params,
                            &evaluated_args,
                            func,
                        )?
                    } else {
                        return Err(runtime_error("External function must have arrow type"));
                    };
                    return self.run_external_function(&wasm_name, &typ, positional);
                }

                if let Some(pos) = func.find('.') {
                    let mod_name = &func[..pos];
                    let item_name = &func[pos + 1..];
                    let mut callback_defs = Vec::new();
                    for (_, value) in &evaluated_args {
                        if let Value::Function(name) = value {
                            if let Some(def) = self.functions.get(name).cloned() {
                                callback_defs.push((name.clone(), def));
                            }
                        }
                    }

                    if let Some(sub_interp) = self.modules.get_mut(mod_name) {
                        for (name, def) in &callback_defs {
                            sub_interp
                                .functions
                                .entry(name.clone())
                                .or_insert_with(|| def.clone());
                        }
                        if let Some(target) = sub_interp.functions.get(item_name).cloned() {
                            let positional = Interpreter::labeled_to_positional_for_params(
                                &target.params,
                                &evaluated_args,
                                &format!("{}.{}", mod_name, item_name),
                            )?;
                            let res = sub_interp
                                .run_function(item_name, positional)
                                .map_err(runtime_error)?;
                            return Ok(ExprResult::Normal(res));
                        }
                        if let Some((wasm_name, typ)) =
                            sub_interp.external_functions.get(item_name).cloned()
                        {
                            let positional = if let Type::Arrow(params, _, _, _) = &typ {
                                Interpreter::labeled_to_positional_for_arrow_params(
                                    params,
                                    &evaluated_args,
                                    &format!("{}.{}", mod_name, item_name),
                                )?
                            } else {
                                return Err(runtime_error(
                                    "External function must have arrow type",
                                ));
                            };
                            return sub_interp.run_external_function(&wasm_name, &typ, positional);
                        }
                        return Err(runtime_error(format!(
                            "Function '{}.{}' not found",
                            mod_name, item_name
                        )));
                    }

                    if let Some(handler_fns) = self.handlers.get(mod_name).cloned() {
                        if let Some(target_func) = handler_fns.iter().find(|f| f.name == item_name)
                        {
                            let positional = Interpreter::labeled_to_positional_for_params(
                                &target_func.params,
                                &evaluated_args,
                                &format!("{}.{}", mod_name, item_name),
                            )?;
                            let mut handler_env = Env::new();
                            for (param, arg) in target_func.params.iter().zip(positional.iter()) {
                                handler_env.define(param.name.clone(), arg.clone());
                            }
                            let res = self.eval_body(&target_func.body, &mut handler_env)?;
                            let val = match res {
                                ExprResult::Normal(v) => v,
                                ExprResult::EarlyReturn(v) => v,
                            };
                            return Ok(ExprResult::Normal(val));
                        }
                    }
                }

                let target = self
                    .functions
                    .get(func)
                    .ok_or_else(|| runtime_error(format!("Function '{}' not found", func)))?
                    .clone();
                let positional = Interpreter::labeled_to_positional_for_params(
                    &target.params,
                    &evaluated_args,
                    func,
                )?;
                let res = self.run_function(func, positional).map_err(runtime_error)?;
                Ok(ExprResult::Normal(res))
            }
            Expr::Constructor(name, args) => {
                let fields = self.get_variant_fields(name).ok_or_else(|| {
                    EvalError::Exception(Value::Variant(
                        "RuntimeError".to_string(),
                        vec![Value::String(format!("Unknown constructor {}", name))],
                    ))
                })?;

                let mut evaluated_args = Vec::new();
                for (label, arg_expr) in args {
                    let res = self.eval_expr(arg_expr, env)?;
                    match res {
                        ExprResult::Normal(val) => evaluated_args.push((label.clone(), val)),
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }

                let mut vals = vec![None; fields.len()];
                for (label, val) in evaluated_args {
                    if let Some(l) = label {
                        if let Some(idx) = fields.iter().position(|f| f.0.as_ref() == Some(&l)) {
                            vals[idx] = Some(val);
                        }
                    } else if let Some(idx) = vals.iter().position(|v| v.is_none()) {
                        vals[idx] = Some(val);
                    }
                }

                let final_vals = vals.into_iter().map(|v| v.unwrap_or(Value::Unit)).collect();
                Ok(ExprResult::Normal(Value::Variant(name.clone(), final_vals)))
            }
            Expr::Record(fields) => {
                let mut map = HashMap::new();
                for (name, val_expr) in fields {
                    let res = self.eval_expr(val_expr, env)?;
                    match res {
                        ExprResult::Normal(val) => {
                            map.insert(name.clone(), val);
                        }
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }
                Ok(ExprResult::Normal(Value::Record(map)))
            }
            Expr::Array(exprs) => {
                let mut vals = Vec::new();
                for e in exprs {
                    match self.eval_expr(e, env)? {
                        ExprResult::Normal(v) => vals.push(v),
                        ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                    }
                }
                Ok(ExprResult::Normal(Value::Array(Arc::new(Mutex::new(vals)))))
            }
            Expr::Index(arr, idx) => {
                let arr_res = self.eval_expr(arr, env)?;
                let idx_res = self.eval_expr(idx, env)?;
                match (arr_res, idx_res) {
                    (ExprResult::Normal(arr_val), ExprResult::Normal(Value::Int(i))) => {
                        if i < 0 {
                            return Err(invalid_index_error(i));
                        }
                        let idx = i as usize;
                        match arr_val {
                            Value::Array(a) => {
                                let l =
                                    a.lock().map_err(|_| runtime_error("Array lock poisoned"))?;
                                if idx < l.len() {
                                    Ok(ExprResult::Normal(l[idx].clone()))
                                } else {
                                    Err(invalid_index_error(i))
                                }
                            }
                            _ => Err(runtime_error("Cannot index non-array value")),
                        }
                    }
                    (ExprResult::EarlyReturn(v), _) | (_, ExprResult::EarlyReturn(v)) => {
                        Ok(ExprResult::EarlyReturn(v))
                    }
                    _ => Err(runtime_error("Index must be an integer")),
                }
            }
            Expr::FieldAccess(receiver, field_name) => {
                let res = self.eval_expr(receiver, env)?;
                match res {
                    ExprResult::Normal(Value::Record(map)) => {
                        if let Some(v) = map.get(field_name) {
                            Ok(ExprResult::Normal(v.clone()))
                        } else {
                            Err(runtime_error(format!(
                                "Field {} not found in record",
                                field_name
                            )))
                        }
                    }
                    ExprResult::Normal(v) => Err(runtime_error(format!(
                        "Cannot access field {} on non-record value {:?}",
                        field_name, v
                    ))),
                    ExprResult::EarlyReturn(v) => Ok(ExprResult::EarlyReturn(v)),
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_expr(cond, env)?;
                match c {
                    ExprResult::Normal(Value::Bool(b)) => {
                        if b {
                            self.eval_body(then_branch, env)
                        } else if let Some(else_branch) = else_branch {
                            self.eval_body(else_branch, env)
                        } else {
                            Ok(ExprResult::Normal(Value::Unit))
                        }
                    }
                    ExprResult::Normal(_) => Err(runtime_error("If condition must be bool")),
                    ExprResult::EarlyReturn(v) => Ok(ExprResult::EarlyReturn(v)),
                }
            }
            Expr::Match { target, cases } => {
                let val_res = self.eval_expr(target, env)?;
                let val = match val_res {
                    ExprResult::Normal(v) => v,
                    ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                };

                for case in cases {
                    if let Some(bindings) = self.match_pattern(&case.pattern, &val) {
                        let mut new_env = Env::extend(env.clone());
                        for (k, v) in bindings {
                            new_env.define(k, v);
                        }
                        return self.eval_body(&case.body, &mut new_env);
                    }
                }
                Err(runtime_error("No match found"))
            }
            Expr::Lambda {
                type_params,
                params,
                ret_type,
                requires,
                effects,
                body,
            } => {
                let fn_name = self.register_lambda(
                    Function {
                        name: String::new(),
                        is_public: false,
                        type_params: type_params.clone(),
                        params: params.clone(),
                        ret_type: ret_type.clone(),
                        requires: requires.clone(),
                        effects: effects.clone(),
                        body: body.clone(),
                    },
                    env.clone(),
                    None,
                );
                Ok(ExprResult::Normal(Value::Function(fn_name)))
            }
            Expr::External(_wasm_name, _, _typ) => {
                // External expression itself evaluates to a function handle if we want,
                // but since it's only allowed in `let`, we can handle it there.
                // However, for completeness in `eval_expr`:
                Ok(ExprResult::Normal(Value::Unit)) // Or some meaningful value
            }
            Expr::Handler {
                coeffect_name,
                functions,
            } => Ok(ExprResult::Normal(Value::Handler(
                coeffect_name.clone(),
                functions.clone(),
            ))),
            Expr::Raise(expr) => {
                let val_res = self.eval_expr(expr, env)?;
                let val = match val_res {
                    ExprResult::Normal(v) => v,
                    ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                };
                Err(EvalError::Exception(val))
            }
        }
    }

    fn match_pattern(
        &self,
        pattern: &Spanned<Pattern>,
        val: &Value,
    ) -> Option<HashMap<String, Value>> {
        match (&pattern.node, val) {
            (Pattern::Variable(name, sigil), v) => {
                let mut map = HashMap::new();
                map.insert(sigil.get_key(name), v.clone());
                Some(map)
            }
            (Pattern::Wildcard, _) => Some(HashMap::new()),
            (Pattern::Literal(lit), v) => match (lit, v) {
                (Literal::Int(a), Value::Int(b)) if a == b => Some(HashMap::new()),
                (Literal::Float(a), Value::Float(b)) if (a - b).abs() < f64::EPSILON => {
                    Some(HashMap::new())
                }
                (Literal::Bool(a), Value::Bool(b)) if a == b => Some(HashMap::new()),
                (Literal::String(a), Value::String(b)) if a == b => Some(HashMap::new()),
                (Literal::Unit, Value::Unit) => Some(HashMap::new()),
                _ => None,
            },
            (Pattern::Constructor(name, pats), Value::Variant(vname, vals)) => {
                if name == vname {
                    let fields = self.get_variant_fields(name)?;
                    if fields.len() != vals.len() {
                        return None;
                    }

                    let mut matched = vec![None; fields.len()];
                    for (label, pat) in pats {
                        if let Some(l) = label {
                            if let Some(idx) = fields.iter().position(|f| f.0.as_ref() == Some(l)) {
                                matched[idx] = Some(pat);
                            }
                        } else if let Some(idx) = matched.iter().position(|m| m.is_none()) {
                            matched[idx] = Some(pat);
                        }
                    }

                    let mut bindings = HashMap::new();
                    for (i, p_opt) in matched.into_iter().enumerate() {
                        if let Some(p) = p_opt {
                            if let Some(b) = self.match_pattern(p, &vals[i]) {
                                bindings.extend(b);
                            } else {
                                return None;
                            }
                        }
                    }
                    Some(bindings)
                } else {
                    None
                }
            }
            (Pattern::Record(pat_fields, _), Value::Record(map)) => {
                let mut bindings = HashMap::new();
                for (name, pat) in pat_fields {
                    if let Some(v) = map.get(name) {
                        if let Some(b) = self.match_pattern(pat, v) {
                            bindings.extend(b);
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Some(bindings)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreter_init_error_is_reported_not_panic() {
        let src = r#"
import external /definitely/missing_module_for_test.wasm

let main = fn () -> i64 do
  return 0
endfn
"#;
        let program = crate::lang::parser::parser()
            .parse(src)
            .expect("test source should parse");
        let mut interpreter = Interpreter::new(program);
        let err = interpreter
            .run_function("main", vec![])
            .expect_err("init error should surface through run_function");
        assert!(err.contains("missing_module_for_test.wasm"));
    }

    #[test]
    fn spawn_task_interpreter_reuses_wasmtime_runtime() {
        let interpreter = Interpreter::new(Program {
            definitions: Vec::new(),
        });
        assert!(
            interpreter.init_error.is_none(),
            "interpreter init should succeed for empty program"
        );
        let task_interp = interpreter.spawn_task_interpreter();
        assert!(Arc::ptr_eq(
            &interpreter.wasm_store,
            &task_interp.wasm_store
        ));
        assert!(Arc::ptr_eq(
            &interpreter.wasm_instances,
            &task_interp.wasm_instances
        ));
    }

    #[test]
    fn http_bridge_rejects_oversized_url_with_explicit_error() {
        let url = format!("https://example.com/{}", "a".repeat(MAX_HTTP_URL_BYTES + 1));
        let err = perform_wasi_http_request("GET", &url, "", "")
            .expect_err("oversized url should be rejected before network");
        assert!(err.contains("url exceeds"), "unexpected error: {}", err);

        let capabilities = ExecutionCapabilities {
            allow_net: true,
            ..ExecutionCapabilities::deny_all()
        };
        let raw = run_nexus_host_http_request(&capabilities, "GET", &url, "", "");
        assert!(
            raw.starts_with("0\nhttp request failed: url exceeds"),
            "unexpected bridge response: {}",
            raw
        );
    }

    #[test]
    fn http_bridge_ignores_network_policy_in_allow_all_mode() {
        let capabilities = ExecutionCapabilities {
            net_block_hosts: vec!["example.com".to_string()],
            ..ExecutionCapabilities::deny_all()
        };
        let raw = run_nexus_host_http_request(&capabilities, "GET", "", "", "");
        assert!(
            raw.starts_with("0\nhttp request failed: empty URL"),
            "unexpected bridge response: {}",
            raw
        );
    }

    #[test]
    fn integer_division_by_zero_returns_runtime_error() {
        let src = r#"
let main = fn () -> i64 do
  return 1 / 0
endfn
"#;
        let program = crate::lang::parser::parser()
            .parse(src)
            .expect("test source should parse");
        let mut interpreter = Interpreter::new(program);
        let err = interpreter
            .run_function("main", vec![])
            .expect_err("division by zero should return runtime error");
        assert!(
            err.contains("division by zero"),
            "unexpected error: {}",
            err
        );
    }
}
