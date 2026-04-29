//! End-to-end test for `nexus build --polyglot` (issue nexus-n3qo).
//!
//! Builds the self-hosted compiler from `src/driver.nx` via the Rust
//! bootstrap, then drives `nexus build --polyglot` against a small fixture
//! and runs the resulting POSIX-sh + wasm self-extracting launcher.
//!
//! Acceptance covered here:
//!   - `nexus build --polyglot foo.nx -o foo` produces a runnable launcher
//!   - `chmod +x foo && ./foo` runs the program (stdout matches expected)
//!   - `--remain-manifest` emits a manifest-bearing variant that also runs
//!   - Two consecutive `--polyglot` builds of the same source produce
//!     byte-identical output (determinism contract from issue acceptance)
//!
//! Building the self-hosted compiler costs ~30s wall-clock. To amortise that,
//! the build runs once and three sub-checks share it.
//!
//! The fixture (`polyglot_hello.nx`) is a tiny stdio program: small enough
//! that the launcher boot path stays within seconds even on slow CI.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_path(prefix: &str) -> PathBuf {
    let seq = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nexus_polyglot_{}_{}_{}",
        prefix,
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
    let output = unique_path("driver").with_extension("wasm");
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

/// Run `wasmtime run <self_host_wasm> build --polyglot[ --remain-manifest] <fixture> -o <output>`.
fn run_polyglot_build(
    self_host_wasm: &Path,
    fixture: &str,
    output: &Path,
    remain_manifest: bool,
) -> std::process::Output {
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
    cmd.arg("build");
    cmd.arg("--polyglot");
    if remain_manifest {
        cmd.arg("--remain-manifest");
    }
    cmd.arg(fixture);
    cmd.arg("-o");
    cmd.arg(output);
    cmd.output()
        .expect("failed to invoke wasmtime run nexus.wasm build --polyglot")
}

fn chmod_executable(path: &Path) {
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod +x launcher");
}

fn run_launcher(launcher: &Path) -> std::process::Output {
    Command::new("sh")
        .arg(launcher)
        .output()
        .expect("spawn launcher")
}

/// End-to-end acceptance: build a polyglot launcher (default + manifest),
/// run it, and compare two builds for byte-identical output.
#[test]
fn polyglot_launcher_end_to_end() {
    let self_host_wasm = build_self_hosted_compiler();
    let fixture = "bootstrap/tests/fixtures/polyglot_hello.nx";

    // ─── Default (no manifest) ─────────────────────────────────────────────
    let launcher = unique_path("default");
    let out = run_polyglot_build(&self_host_wasm, fixture, &launcher, false);
    assert!(
        out.status.success(),
        "polyglot build failed:\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(launcher.exists(), "launcher not created at {:?}", launcher);

    chmod_executable(&launcher);
    let exec = run_launcher(&launcher);
    assert!(
        exec.status.success(),
        "launcher execution failed:\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&exec.stderr),
        String::from_utf8_lossy(&exec.stdout)
    );
    let exec_stdout = String::from_utf8_lossy(&exec.stdout).into_owned();
    assert!(
        exec_stdout.contains("polyglot-hello"),
        "expected fixture output `polyglot-hello` from launcher, got stdout:\n{exec_stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&exec.stderr)
    );

    // ─── Manifest variant ─────────────────────────────────────────────────
    let launcher_m = unique_path("manifest");
    let out_m = run_polyglot_build(&self_host_wasm, fixture, &launcher_m, true);
    assert!(
        out_m.status.success(),
        "polyglot --remain-manifest build failed:\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&out_m.stderr),
        String::from_utf8_lossy(&out_m.stdout)
    );
    chmod_executable(&launcher_m);
    let exec_m = run_launcher(&launcher_m);
    assert!(
        exec_m.status.success(),
        "manifest launcher execution failed:\nstderr: {}\nstdout: {}",
        String::from_utf8_lossy(&exec_m.stderr),
        String::from_utf8_lossy(&exec_m.stdout)
    );
    let exec_m_stdout = String::from_utf8_lossy(&exec_m.stdout).into_owned();
    assert!(
        exec_m_stdout.contains("polyglot-hello"),
        "expected fixture output `polyglot-hello` from manifest launcher, got:\n{exec_m_stdout}"
    );

    // Manifest payload must include the `#__NEXUS_PAYLOAD_MANIFEST__` marker
    // so the format-extension contract holds (issue n3qo Acceptance for
    // future multi-payload bundles).
    let launcher_m_bytes = std::fs::read(&launcher_m).expect("read manifest launcher");
    assert!(
        contains_subseq(&launcher_m_bytes, b"#__NEXUS_PAYLOAD_MANIFEST__"),
        "manifest launcher missing #__NEXUS_PAYLOAD_MANIFEST__ marker"
    );
    assert!(
        contains_subseq(&launcher_m_bytes, b"app:"),
        "manifest launcher missing `app:` entry"
    );

    // ─── Determinism: a second build must produce the same bytes ──────────
    let launcher_b = unique_path("default_b");
    let out_b = run_polyglot_build(&self_host_wasm, fixture, &launcher_b, false);
    assert!(out_b.status.success(), "second polyglot build failed");
    let bytes_a = std::fs::read(&launcher).expect("read launcher A");
    let bytes_b = std::fs::read(&launcher_b).expect("read launcher B");
    assert_eq!(
        bytes_a.len(),
        bytes_b.len(),
        "two --polyglot builds produced different launcher sizes ({} vs {})",
        bytes_a.len(),
        bytes_b.len()
    );
    assert_eq!(
        bytes_a, bytes_b,
        "two --polyglot builds of the same source must produce byte-identical output"
    );

    // Cleanup.
    let _ = std::fs::remove_file(&self_host_wasm);
    let _ = std::fs::remove_file(&launcher);
    let _ = std::fs::remove_file(&launcher_m);
    let _ = std::fs::remove_file(&launcher_b);
}

fn contains_subseq(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
