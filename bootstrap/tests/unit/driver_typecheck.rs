//! End-to-end test for `nexus typecheck <file>` (issue nexus-5er0).
//!
//! Builds the self-hosted compiler from `src/driver.nx` via the Rust
//! bootstrap, then drives it through `wasmtime run` against a clean and an
//! erroneous fixture. Asserts:
//!   - clean program → exit 0
//!   - error program → exit 1 + LSP-compatible diagnostic on stderr
//!   - typecheck-only mode skips MIR/LIR/codegen (verified via verbose stage
//!     log that omits `[nxc] codegen` / `[nxc] MIR` / `[nxc] LIR`)
//!
//! The three assertions live in one `#[test]` so the heavy
//! `nexus build src/driver.nx` step (~30s) only runs once. Mirrors the
//! build-and-run pattern from `bootstrap/tests/stdlib/lsp_main.rs`.
//!
//! Two fixtures back this test:
//!   - bootstrap/tests/fixtures/typecheck_clean.nx
//!   - bootstrap/tests/fixtures/typecheck_error.nx

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_wasm_path() -> PathBuf {
    let seq = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nexus_typecheck_{}_{}.wasm",
        std::process::id(),
        seq
    ))
}

/// Build src/driver.nx via the Rust bootstrap compiler. Returns the path
/// to the produced wasm. Panics on build failure.
fn build_self_hosted_compiler() -> PathBuf {
    crate::harness::ensure_repo_root();
    let cwd = std::env::current_dir().expect("cwd");
    let nexus_bin = cwd.join("bootstrap/target/release/nexus");
    assert!(
        nexus_bin.exists(),
        "bootstrap/target/release/nexus not found — run `cargo build --release --manifest-path bootstrap/Cargo.toml` first"
    );
    let output = unique_wasm_path();
    let result = Command::new(&nexus_bin)
        .arg("build")
        .arg("src/driver.nx")
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
            "nexus build src/driver.nx failed (exit {}):\nstderr: {stderr}\nstdout: {stdout}",
            result.status
        );
    }
    output
}

/// Run `wasmtime run <self_host_wasm> [extra_args...] typecheck <fixture>`
/// and capture (exit_status, stdout, stderr). `extra_args` are inserted
/// before the `typecheck` subcommand so flags such as `--verbose` are
/// observed by the driver's argument parser.
fn run_typecheck(
    self_host_wasm: &PathBuf,
    extra_args: &[&str],
    fixture: &str,
) -> (std::process::ExitStatus, String, String) {
    let cwd = std::env::current_dir().expect("cwd");
    let cwd_str = cwd.to_str().unwrap();
    let mut cmd = Command::new("wasmtime");
    cmd.args([
        "run",
        "-W",
        "tail-call=y,exceptions=y,function-references=y,stack-switching=y,component-model=y,max-memory-size=8589934592",
        // The composed compiler component imports `wasi:http/types`; pass
        // `-S http,inherit-network` so wasmtime supplies an implementation.
        // Mirrors the flag set used by `bootstrap.sh` for stage 1/2 runs.
        "-S",
        "http,inherit-network",
        &format!("--dir={cwd_str}::."),
        &format!("--dir={}", std::env::temp_dir().display()),
    ]);
    cmd.arg(self_host_wasm);
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg("typecheck");
    cmd.arg(fixture);
    let result = cmd
        .output()
        .expect("failed to invoke wasmtime run nexus.wasm typecheck");
    (
        result.status,
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
    )
}

/// End-to-end coverage for `nexus typecheck`:
///   1. clean fixture → exit 0
///   2. error fixture → exit 1 + LSP-compatible stderr diagnostic
///   3. verbose run on clean fixture skips codegen/MIR/LIR stages
#[test]
fn typecheck_subcommand_end_to_end() {
    let self_host_wasm = build_self_hosted_compiler();

    // Clean program → exit 0.
    let (clean_status, _clean_stdout, clean_stderr) = run_typecheck(
        &self_host_wasm,
        &[],
        "bootstrap/tests/fixtures/typecheck_clean.nx",
    );
    assert!(
        clean_status.success(),
        "expected exit 0 for clean program, got {clean_status}\nstderr: {clean_stderr}"
    );

    // Error program → non-zero exit + LSP-compatible diagnostic on stderr.
    let (err_status, _err_stdout, err_stderr) = run_typecheck(
        &self_host_wasm,
        &[],
        "bootstrap/tests/fixtures/typecheck_error.nx",
    );
    assert!(
        !err_status.success(),
        "expected non-zero exit for type-error program, got success\nstderr: {err_stderr}"
    );
    assert_eq!(
        err_status.code(),
        Some(1),
        "expected exit code 1 (typecheck error), got {:?}\nstderr: {err_stderr}",
        err_status.code()
    );
    // Diagnostic must follow the existing driver format: the LSP-compatible
    // `<file>: ...message... [<code>]` line emitted by `err.format_error`.
    // We check for the file prefix and the bracketed E2001 code
    // (TypeMismatch) so the format contract surfaced by
    // `enumerate_diagnostics` (hw47.4) remains parseable for downstream LSP
    // clients.
    assert!(
        err_stderr.contains("typecheck_error.nx"),
        "expected file path in diagnostic, got: {err_stderr}"
    );
    assert!(
        err_stderr.contains("[E2001]"),
        "expected E2001 (TypeMismatch) error code in diagnostic, got: {err_stderr}"
    );

    // Verbose run on clean fixture: codegen / MIR / LIR stage lines must be
    // absent (typecheck-only short-circuits before they execute), and the
    // typecheck-only summary line must be present.
    let (verbose_status, _verbose_stdout, verbose_stderr) = run_typecheck(
        &self_host_wasm,
        &["--verbose"],
        "bootstrap/tests/fixtures/typecheck_clean.nx",
    );
    let _ = std::fs::remove_file(&self_host_wasm);
    assert!(
        verbose_status.success(),
        "verbose typecheck failed:\n{verbose_stderr}"
    );
    assert!(
        verbose_stderr.contains("[nxc] typecheck"),
        "expected verbose typecheck stage line, got stderr:\n{verbose_stderr}"
    );
    assert!(
        !verbose_stderr.contains("[nxc] codegen"),
        "typecheck-only mode must not run codegen, but verbose stderr shows it:\n{verbose_stderr}"
    );
    assert!(
        !verbose_stderr.contains("[nxc] MIR")
            && !verbose_stderr.contains("[nxc] LIR"),
        "typecheck-only mode must skip MIR/LIR lowering, but verbose stderr shows it:\n{verbose_stderr}"
    );
    assert!(
        verbose_stderr.contains("typecheck-only"),
        "expected `nxc: typecheck-only ...` summary line, got stderr:\n{verbose_stderr}"
    );
}
