//! End-to-end test for `src/lsp/main.nx` (nexus-hw47.8).
//!
//! Builds `lsp.wasm` via the Rust bootstrap compiler, then spawns
//! `wasmtime run lsp.wasm` with stdio attached, sends a hand-written
//! `initialize` JSON-RPC frame on stdin, and validates the
//! `InitializeResult` response on stdout.
//!
//! This is the e2e proof-of-life the hw47.8 acceptance criterion calls
//! out. Pure-handler-vtable correctness is covered separately by
//! `lsp_server.rs::*`; this test covers the wire path: framing on stdin
//! → dispatch → framing on stdout, exercising every layer of the LSP
//! pipeline that ships in lsp.wasm.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static LSP_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_lsp_wasm_path() -> PathBuf {
    let seq = LSP_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "lsp_main_{}_{}.wasm",
        std::process::id(),
        seq
    ))
}

/// Build src/lsp/main.nx via the Rust bootstrap compiler. Returns the
/// path to the produced wasm component. Panics on build failure.
fn build_lsp_wasm() -> PathBuf {
    crate::harness::ensure_repo_root();
    let cwd = std::env::current_dir().expect("cwd");
    let nexus_bin = cwd.join("bootstrap/target/release/nexus");
    assert!(
        nexus_bin.exists(),
        "bootstrap/target/release/nexus not found — run `cargo build --release --manifest-path bootstrap/Cargo.toml` first"
    );
    let output = unique_lsp_wasm_path();
    let result = Command::new(&nexus_bin)
        .arg("build")
        .arg("src/lsp/main.nx")
        .arg("-o")
        .arg(&output)
        .arg("--explain-capabilities")
        .arg("none")
        .output()
        .expect("failed to invoke nexus build");
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let _ = std::fs::remove_file(&output);
        panic!(
            "nexus build src/lsp/main.nx failed (exit {}):\nstderr: {stderr}\nstdout: {stdout}",
            result.status
        );
    }
    output
}

/// Encode a JSON body with the LSP Content-Length framing the server
/// expects on stdin.
fn frame(body: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    out.extend_from_slice(body.as_bytes());
    out
}

/// Read until we have at least one complete `Content-Length:`-framed
/// message, or `deadline` passes. Returns the (header, body) split as
/// strings; panics on parse failure or deadline miss.
fn read_one_frame(stdout: &mut std::process::ChildStdout, deadline: Instant) -> (String, String) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    while Instant::now() < deadline {
        // Non-blocking read attempt: use timeout via short polling on
        // ChildStdout. ChildStdout doesn't expose a timeout primitive
        // directly; we drain in chunks and re-check whether the
        // accumulated buffer contains a complete frame.
        match stdout.read(&mut tmp) {
            Ok(0) => {
                // EOF — child exited mid-frame. Surface as panic with
                // whatever bytes we managed to capture so test failure
                // shows the actual wire content rather than a generic
                // "no response".
                panic!(
                    "lsp.wasm closed stdout before producing a complete frame; partial buffer: {:?}",
                    String::from_utf8_lossy(&buf)
                );
            }
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) => panic!("read from lsp.wasm stdout failed: {e}"),
        }
        if let Some((header, body)) = try_split_frame(&buf) {
            return (header, body);
        }
    }
    panic!(
        "timed out waiting for response frame; buffer: {:?}",
        String::from_utf8_lossy(&buf)
    );
}

fn try_split_frame(buf: &[u8]) -> Option<(String, String)> {
    // Find the CRLF CRLF that ends the header block.
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let header_bytes = &buf[..header_end];
    let header = std::str::from_utf8(header_bytes).ok()?;
    // Locate Content-Length value (case-insensitive on the field name).
    let mut content_length: Option<usize> = None;
    for line in header.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse::<usize>().ok();
            break;
        }
    }
    let n = content_length?;
    let body_start = header_end + 4;
    if buf.len() < body_start + n {
        return None;
    }
    let body = std::str::from_utf8(&buf[body_start..body_start + n]).ok()?;
    Some((header.to_string(), body.to_string()))
}

#[test]
fn lsp_main_responds_to_initialize_over_stdio() {
    // 1) Build lsp.wasm via the Rust bootstrap compiler.
    let lsp_wasm = build_lsp_wasm();

    // 2) Spawn wasmtime against lsp.wasm. The capability flag set
    //    matches the inline documentation in src/lsp/main.nx:
    //      PermFs       — --dir=<repo-root>
    //      PermConsole  — inherited stdio (default)
    //      PermClock    — clocks default-on
    //
    //    We pipe stdin/stdout so the test writes the initialize frame
    //    and reads the response. Stderr inherits so wasmtime trap text
    //    surfaces if execution fails.
    let cwd = std::env::current_dir().expect("cwd");
    let cwd_str = cwd.to_str().unwrap();
    let mut child = Command::new("wasmtime")
        .args([
            "run",
            "-W",
            "tail-call=y,exceptions=y,function-references=y,stack-switching=y,component-model=y,max-memory-size=8589934592",
            "-S",
            "http,inherit-network",
            &format!("--dir={cwd_str}::."),
            &format!("--dir={}", std::env::temp_dir().display()),
        ])
        .arg(&lsp_wasm)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn wasmtime against lsp.wasm");

    // 3) Write a hand-rolled initialize request and an exit notification
    //    so the server terminates cleanly after replying.
    let init_body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":4242,"rootUri":"file:///tmp","capabilities":{}}}"#;
    let exit_body = r#"{"jsonrpc":"2.0","method":"exit"}"#;

    {
        let stdin = child.stdin.as_mut().expect("stdin pipe");
        stdin.write_all(&frame(init_body)).expect("write init frame");
        stdin.write_all(&frame(exit_body)).expect("write exit frame");
        stdin.flush().expect("flush stdin");
    }
    // Drop stdin so the child observes EOF after the exit notification —
    // belt-and-suspenders against any read that doesn't satisfy on the
    // pre-EOF bytes alone.
    drop(child.stdin.take());

    // 4) Read one frame back from stdout — that should be the
    //    InitializeResult response.
    let mut stdout = child.stdout.take().expect("stdout pipe");
    let deadline = Instant::now() + Duration::from_secs(60);
    let (_header, body) = read_one_frame(&mut stdout, deadline);

    // Wait for the child to terminate so the test process doesn't leak
    // wasmtime instances. The exit notification should drive a graceful
    // shutdown.
    let _ = child.wait();

    let _ = std::fs::remove_file(&lsp_wasm);

    // 5) Validate the response shape: must contain id=1 and a
    //    capabilities.textDocumentSync = 1 (Full) field.
    assert!(
        body.contains("\"id\":1"),
        "expected id=1 in response, got: {body}"
    );
    assert!(
        body.contains("\"jsonrpc\":\"2.0\""),
        "expected jsonrpc=2.0 in response, got: {body}"
    );
    assert!(
        body.contains("\"capabilities\""),
        "expected capabilities field in response, got: {body}"
    );
    assert!(
        body.contains("\"textDocumentSync\":1"),
        "expected textDocumentSync=1 (Full) in capabilities, got: {body}"
    );
    assert!(
        body.contains("\"name\":\"nexus-lsp\""),
        "expected serverInfo.name=\"nexus-lsp\", got: {body}"
    );
}
