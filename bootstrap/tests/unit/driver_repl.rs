//! End-to-end test for `nexus repl` (issue nexus-eq7e).
//!
//! Builds the self-hosted compiler from `src/driver.nx` via the Rust
//! bootstrap, then drives the REPL through `wasmtime run` with a scripted
//! stdin. Verifies all seven Acceptance scenarios:
//!
//!   1. `nexus repl` starts and prints the welcome banner / `> ` prompt
//!   2. `1 + 2` evaluates to `3`
//!   3. `let x = 10` produces no stdout output, then `x` prints `10`
//!   4. `:type x` prints `i64`
//!   5. a type error surfaces in LSP-compatible `... [E2001]` format on stderr
//!   6. `:quit` (and EOF) terminate with exit code 0
//!   7. all of the above are driven by a single stdin script
//!
//! The build step (~30s wall-clock) is shared with the typecheck E2E test
//! by re-using the same bootstrap binary; this test invokes the binary
//! directly rather than going through `cargo build` again.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_wasm_path() -> PathBuf {
    let seq = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nexus_repl_{}_{}.wasm",
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

/// Spawn `wasmtime run <self_host_wasm> repl` with the given stdin script
/// and capture (exit_status, stdout, stderr).
fn run_repl(self_host_wasm: &PathBuf, stdin_script: &str) -> (std::process::ExitStatus, String, String) {
    let cwd = std::env::current_dir().expect("cwd");
    let cwd_str = cwd.to_str().unwrap();
    let mut cmd = Command::new("wasmtime");
    cmd.args([
        "run",
        "-W",
        "tail-call=y,exceptions=y,function-references=y,stack-switching=y,component-model=y,max-memory-size=8589934592",
        "-S",
        "http,inherit-network",
        &format!("--dir={cwd_str}::."),
        &format!("--dir={}", std::env::temp_dir().display()),
    ]);
    cmd.arg(self_host_wasm);
    cmd.arg("repl");
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn wasmtime run repl");
    {
        let stdin = child.stdin.as_mut().expect("child stdin");
        stdin
            .write_all(stdin_script.as_bytes())
            .expect("write stdin script");
    }
    let result = child.wait_with_output().expect("wait_with_output");
    (
        result.status,
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
    )
}

/// Drive the REPL through every Acceptance scenario in one stdin script.
/// Splitting into separate `#[test]` functions would re-pay the ~30s build
/// cost for each — so the assertions are folded into one test.
#[test]
fn repl_subcommand_end_to_end() {
    let self_host_wasm = build_self_hosted_compiler();
    let script = "\
1 + 2
let x = 10
x
:type x
1 + true
:quit
";
    let (status, stdout, stderr) = run_repl(&self_host_wasm, script);
    let _ = std::fs::remove_file(&self_host_wasm);

    // Acceptance 1 — banner + prompt rendered
    assert!(
        stdout.contains("Nexus REPL"),
        "expected REPL welcome banner on stdout, got:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("> "),
        "expected `> ` prompt on stdout, got:\nstdout: {stdout}"
    );

    // Acceptance 2 — `1 + 2` -> `3`
    assert!(
        stdout.contains("3"),
        "expected `3` for `1 + 2`, got stdout:\n{stdout}"
    );

    // Acceptance 3 — `let x = 10` is silent; `x` prints `10`
    assert!(
        stdout.contains("10"),
        "expected `10` after `x` lookup, got stdout:\n{stdout}"
    );
    // Make sure the `let` line itself did not echo a value (no spurious `()`
    // or `10` appearing too early). Heuristic: count occurrences of "10" —
    // exactly one from the `x` lookup.
    let ten_count = stdout.matches("10").count();
    assert_eq!(
        ten_count, 1,
        "expected exactly one `10` on stdout (from `x` lookup, not from the `let`), got {ten_count} occurrences:\n{stdout}"
    );

    // Acceptance 4 — `:type x` -> `i64`
    assert!(
        stdout.contains("i64"),
        "expected `i64` for `:type x`, got stdout:\n{stdout}"
    );

    // Acceptance 5 — type error emitted in LSP-compatible format on stderr.
    // `enumerate_diagnostics` (hw47.4) tags TypeMismatch with code E2001;
    // the REPL routes the same `format_error` text through stderr so any
    // LSP-aware downstream tool can parse the bracketed code.
    assert!(
        stderr.contains("[E2001]"),
        "expected E2001 (TypeMismatch) on stderr for `1 + true`, got stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("type error"),
        "expected `type error` on stderr, got:\n{stderr}"
    );

    // Acceptance 6 — `:quit` exits with status 0
    assert!(
        status.success(),
        "expected exit 0 from `:quit`, got {status}\nstderr: {stderr}"
    );
}

/// Confirm EOF (empty stdin) terminates the REPL with exit 0. This is the
/// other half of Acceptance #6 — running without typing `:quit`.
#[test]
fn repl_subcommand_eof_exits_clean() {
    let self_host_wasm = build_self_hosted_compiler();
    // Empty stdin: read_line() returns the empty string immediately, the
    // REPL treats that as EOF and returns from the read loop.
    let (status, _stdout, stderr) = run_repl(&self_host_wasm, "");
    let _ = std::fs::remove_file(&self_host_wasm);
    assert!(
        status.success(),
        "expected exit 0 on EOF, got {status}\nstderr: {stderr}"
    );
}
