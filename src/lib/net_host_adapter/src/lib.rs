mod bindings {
    wit_bindgen::generate!({
        world: "bridge",
        path: "wit",
        generate_all,
    });
}

use bindings::wasi::http::outgoing_handler;
use bindings::wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};

const MAX_HTTP_URL_BYTES: usize = 8 * 1024;
const MAX_HTTP_HEADERS_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;

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
    let (scheme, rest) = if let Some(rest) = trimmed.strip_prefix("https://") {
        (Scheme::Https, rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (Scheme::Http, rest)
    } else {
        return Err("unsupported URL scheme".to_string());
    };
    if rest.is_empty() {
        return Err("missing authority".to_string());
    }
    let (authority, path) = if let Some((a, p)) = rest.split_once('/') {
        (a, format!("/{}", p))
    } else {
        (rest, "/".to_string())
    };
    if authority.is_empty() {
        return Err("missing authority".to_string());
    }
    Ok((scheme, authority.to_string(), path))
}

fn parse_headers(headers: &str, authority: &str) -> Fields {
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
) -> Result<(u16, String), String> {
    validate_bridge_limits(url, headers, body)?;
    let (scheme, authority, path) = parse_url(url)?;
    let header_fields = parse_headers(headers, &authority);
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

    if let Ok(out_body) = request.body() {
        if let Ok(stream) = out_body.write() {
            let _ = stream.blocking_write_and_flush(body.as_bytes());
            drop(stream);
        }
        let _ = OutgoingBody::finish(out_body, None);
    }

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

    Ok((status, response_body))
}

struct Guest;

impl bindings::exports::nexus::cli::nexus_host::Guest for Guest {
    fn host_http_request(method: String, url: String, headers: String, body: String) -> String {
        match perform_request(&method, &url, &headers, &body) {
            Ok((status, response_body)) => format!("{}\n{}", status, response_body),
            Err(err) => format!("0\nhttp request failed: {}", err),
        }
    }
}

bindings::export!(Guest with_types_in bindings);
