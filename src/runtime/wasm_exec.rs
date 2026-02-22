use crate::runtime::ExecutionCapabilities;
use bytes::Bytes;
use http::{HeaderName, HeaderValue, Method};
use http_body_util::{BodyExt, Full};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Duration;
use wasmtime::{
    component::{Component, Linker as ComponentLinker, ResourceTable},
    Engine, Linker, Module, Store,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::types::{default_send_request_handler, OutgoingRequestConfig};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

const WASI_SNAPSHOT_MODULE: &str = "wasi_snapshot_preview1";
const NEXUS_HOST_HTTP_MODULE: &str = "nexus:cli/nexus-host";
const NEXUS_HOST_HTTP_FUNC: &str = "host-http-request";
const MAX_HTTP_URL_BYTES: usize = 8 * 1024;
const MAX_HTTP_HEADERS_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;

fn is_preview2_wasi_module(module_name: &str) -> bool {
    module_name.starts_with("wasi:")
}

fn is_component_wasm(wasm: &[u8]) -> bool {
    wasmparser::Parser::is_component(wasm)
}

struct ComponentStoreData {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
}

impl WasiView for ComponentStoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for ComponentStoreData {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

fn body_to_hyper_outgoing(body: &[u8]) -> HyperOutgoingBody {
    use std::convert::Infallible;
    Full::new(Bytes::copy_from_slice(body))
        .map_err(|never: Infallible| match never {})
        .boxed_unsync()
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
        eprintln!("net.http_request error: {}", err);
        return encode_http_bridge_error(err);
    }
    match perform_wasi_http_request(method, url, headers, body) {
        Ok((status, response_body)) => format!("{}\n{}", status, response_body),
        Err(err) => {
            eprintln!("net.http_request error: {}", err);
            encode_http_bridge_error(format!("http request failed: {}", err))
        }
    }
}

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
    let method = Method::from_str(method).map_err(|e| format!("invalid method: {}", e))?;
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
    let use_tls = url.starts_with("https://");
    let mut request = http::Request::builder()
        .method(method)
        .uri(url)
        .body(body_to_hyper_outgoing(body.as_bytes()))
        .map_err(|e| format!("invalid request: {}", e))?;

    for line in headers.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let Ok(name) = HeaderName::from_str(name.trim()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value.trim()) else {
            continue;
        };
        request.headers_mut().append(name, value);
    }

    if !request.headers().contains_key(http::header::HOST) {
        if let Some(authority) = request.uri().authority() {
            if let Ok(value) = HeaderValue::from_str(authority.as_str()) {
                request.headers_mut().insert(http::header::HOST, value);
            }
        }
    }

    let config = OutgoingRequestConfig {
        use_tls,
        connect_timeout: Duration::from_secs(10),
        first_byte_timeout: Duration::from_secs(30),
        between_bytes_timeout: Duration::from_secs(30),
    };

    wasmtime_wasi::runtime::in_tokio(async move {
        let incoming = default_send_request_handler(request, config)
            .await
            .map_err(|e| format!("{:?}", e))?;
        let status = incoming.resp.status().as_u16();
        let bytes = incoming
            .resp
            .into_body()
            .collect()
            .await
            .map_err(|e| format!("{:?}", e))?
            .to_bytes();
        if bytes.len() > MAX_HTTP_RESPONSE_BYTES {
            return Err(format!(
                "response exceeds {} bytes",
                MAX_HTTP_RESPONSE_BYTES
            ));
        }
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    })
}

fn add_nexus_host_to_component_linker(
    linker: &mut ComponentLinker<ComponentStoreData>,
    capabilities: ExecutionCapabilities,
) -> Result<(), String> {
    let mut instance = linker
        .instance(NEXUS_HOST_HTTP_MODULE)
        .map_err(|e| format!("failed to create component import instance: {}", e))?;
    instance
        .func_wrap(
            NEXUS_HOST_HTTP_FUNC,
            move |_store, (method, url, headers, body): (String, String, String, String)| {
                Ok((run_nexus_host_http_request(
                    &capabilities,
                    &method,
                    &url,
                    &headers,
                    &body,
                ),))
            },
        )
        .map_err(|e| format!("failed to add component host HTTP function: {}", e))
}

/// Executes wasm bytes and dispatches to core or component runtime automatically.
pub fn run_wasm_bytes(
    wasm: &[u8],
    module_dir: Option<&Path>,
    capabilities: &ExecutionCapabilities,
) -> ExitCode {
    if is_component_wasm(wasm) {
        return run_component_wasm_bytes(wasm, capabilities);
    }
    run_core_wasm_bytes(wasm, module_dir, capabilities)
}

fn run_component_wasm_bytes(wasm: &[u8], capabilities: &ExecutionCapabilities) -> ExitCode {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    let engine = match Engine::new(&config) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!(
                "Failed to initialize Wasmtime engine for component model: {}",
                e
            );
            return ExitCode::from(1);
        }
    };
    let component = match Component::from_binary(&engine, wasm) {
        Ok(component) => component,
        Err(e) => {
            eprintln!("Failed to load wasm component: {}", e);
            return ExitCode::from(1);
        }
    };

    let mut linker = ComponentLinker::<ComponentStoreData>::new(&engine);
    if let Err(e) = wasmtime_wasi::p2::add_to_linker_sync(&mut linker) {
        eprintln!("Failed to add WASI preview2 to component linker: {}", e);
        return ExitCode::from(1);
    }
    if let Err(e) = wasmtime_wasi_http::add_only_http_to_linker_sync(&mut linker) {
        eprintln!("Failed to add WASI HTTP to component linker: {}", e);
        return ExitCode::from(1);
    }
    if let Err(msg) = add_nexus_host_to_component_linker(&mut linker, capabilities.clone()) {
        eprintln!("Failed to add nexus host HTTP import: {}", msg);
        return ExitCode::from(1);
    }

    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    if let Err(msg) = capabilities.apply_to_wasi_builder(&mut builder) {
        eprintln!("Failed to apply capability policy: {}", msg);
        return ExitCode::from(1);
    }
    let store_data = ComponentStoreData {
        table: ResourceTable::new(),
        wasi: builder.build(),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, store_data);

    let instance = match linker.instantiate(&mut store, &component) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            return ExitCode::from(1);
        }
    };

    if let Ok(main) = instance.get_typed_func::<(), ()>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }
    if let Ok(main) = instance.get_typed_func::<(), (i32,)>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok((value,)) => {
                println!("Result: {}", value);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }
    if let Ok(main) = instance.get_typed_func::<(), (i64,)>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok((value,)) => {
                println!("Result: {}", value);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }
    if let Ok(main) = instance.get_typed_func::<(), (f64,)>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok((value,)) => {
                println!("Result: {}", value);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }

    eprintln!(
        "Runtime Error: could not call exported component function 'main' with supported signatures (() -> unit|i32|i64|f64)"
    );
    ExitCode::from(1)
}

fn run_core_wasm_bytes(
    wasm: &[u8],
    module_dir: Option<&Path>,
    capabilities: &ExecutionCapabilities,
) -> ExitCode {
    let engine = Engine::default();
    let module = match Module::from_binary(&engine, wasm) {
        Ok(module) => module,
        Err(e) => {
            eprintln!("Failed to load wasm module: {}", e);
            return ExitCode::from(1);
        }
    };

    let mut linker = Linker::<wasmtime_wasi::p1::WasiP1Ctx>::new(&engine);
    if let Err(e) = wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx| ctx) {
        eprintln!("Failed to add WASI to linker: {}", e);
        return ExitCode::from(1);
    }
    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    if let Err(msg) = capabilities.apply_to_wasi_builder(&mut builder) {
        eprintln!("Failed to apply capability policy: {}", msg);
        return ExitCode::from(1);
    }
    let mut store = Store::new(&engine, builder.build_p1());

    let mut imported_modules = module
        .imports()
        .map(|import| import.module().to_string())
        .collect::<Vec<_>>();
    imported_modules.sort();
    imported_modules.dedup();
    for module_name in imported_modules {
        if module_name == WASI_SNAPSHOT_MODULE {
            continue;
        }
        if module_name == NEXUS_HOST_HTTP_MODULE {
            eprintln!(
                "Runtime Error: import '{}' is deprecated in core-wasm mode",
                NEXUS_HOST_HTTP_MODULE
            );
            eprintln!(
                "Hint: build as component (`nexus build --wasm`) and run with `wasmtime run` to use WASI HTTP."
            );
            return ExitCode::from(1);
        }
        if is_preview2_wasi_module(&module_name) {
            eprintln!(
                "Runtime Error: preview2/WASI import '{}' cannot run in core-wasm mode",
                module_name
            );
            eprintln!("Hint: use `nexus build --wasm` and run the output with `wasmtime run`.");
            return ExitCode::from(1);
        }

        let Some(module_dir) = module_dir else {
            eprintln!(
                "Runtime Error: unresolved embedded import module '{}' (only wasi imports are allowed in packed wasm)",
                module_name
            );
            return ExitCode::from(1);
        };
        let dep_path = {
            let raw = PathBuf::from(&module_name);
            if raw.is_absolute() {
                raw
            } else {
                module_dir.join(raw)
            }
        };
        let dep = match Module::from_file(&engine, &dep_path) {
            Ok(dep) => dep,
            Err(e) => {
                eprintln!(
                    "Failed to load dependency module '{}' (resolved as '{}'): {}",
                    module_name,
                    dep_path.display(),
                    e
                );
                return ExitCode::from(1);
            }
        };
        if let Err(e) = linker.module(&mut store, &module_name, &dep) {
            eprintln!("Failed to link dependency module '{}': {}", module_name, e);
            return ExitCode::from(1);
        }
    }

    let instance = match linker.instantiate(&mut store, &module) {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("Runtime Error: {}", e);
            return ExitCode::from(1);
        }
    };

    if let Ok(main) = instance.get_typed_func::<(), ()>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }
    if let Ok(main) = instance.get_typed_func::<(), i32>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok(value) => {
                println!("Result: {}", value);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }
    if let Ok(main) = instance.get_typed_func::<(), i64>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok(value) => {
                println!("Result: {}", value);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }
    if let Ok(main) = instance.get_typed_func::<(), f64>(&mut store, "main") {
        match main.call(&mut store, ()) {
            Ok(value) => {
                println!("Result: {}", value);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("Runtime Error: {}", e);
                return ExitCode::from(1);
            }
        }
    }

    eprintln!(
        "Runtime Error: could not call exported 'main' with supported signatures (() -> unit|i32|i64|f64)"
    );
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_bridge_rejects_oversized_body_with_explicit_error() {
        let body = "x".repeat(MAX_HTTP_BODY_BYTES + 1);
        let err = perform_wasi_http_request("POST", "https://example.com", "", &body)
            .expect_err("oversized body should be rejected before network");
        assert!(err.contains("body exceeds"), "unexpected error: {}", err);

        let capabilities = ExecutionCapabilities {
            allow_net: true,
            ..ExecutionCapabilities::deny_all()
        };
        let raw =
            run_nexus_host_http_request(&capabilities, "POST", "https://example.com", "", &body);
        assert!(
            raw.starts_with("0\nhttp request failed: body exceeds"),
            "unexpected bridge response: {}",
            raw
        );
    }

    #[test]
    fn http_bridge_ignores_network_policy_in_allow_all_mode() {
        let capabilities = ExecutionCapabilities {
            net_allow_hosts: vec!["internal.local".to_string()],
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
}
