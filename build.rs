use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WASM_TARGET: &str = "wasm32-wasip1";

struct WasmModule {
    manifest_rel: &'static str,
    src_rel: &'static str,
    artifact_name: &'static str,
    output_name: &'static str,
}

const MODULES: &[WasmModule] = &[
    WasmModule {
        manifest_rel: "src/lib/stdlib_bundle/Cargo.toml",
        src_rel: "src/lib/stdlib_bundle/src",
        artifact_name: "nexus_stdlib_bundle.wasm",
        output_name: "stdlib.wasm",
    },
    WasmModule {
        manifest_rel: "src/lib/nexus_host_bridge/Cargo.toml",
        src_rel: "src/lib/nexus_host_bridge",
        artifact_name: "nexus_nexus_host_bridge_wasm.wasm",
        output_name: "nexus-host-bridge.wasm",
    },
];

fn main() {
    println!("cargo:rerun-if-env-changed=NEXUS_SKIP_WASM_BUILD");
    for module in MODULES {
        println!("cargo:rerun-if-changed={}", module.manifest_rel);
        println!("cargo:rerun-if-changed={}", module.src_rel);
    }
    // Sub-crate sources feed into the stdlib bundle; track them for rebuilds.
    for sub in &[
        "src/lib/stdio/src",
        "src/lib/string/src",
        "src/lib/net/src",
        "src/lib/core/src",
        "src/lib/math/src",
        "src/lib/fs/src",
        "src/lib/random/src",
        "src/lib/clock/src",
        "src/lib/proc/src",
        "src/lib/collection/src",
        "src/lib/wasm_alloc/src",
    ] {
        println!("cargo:rerun-if-changed={}", sub);
    }

    if env::var_os("NEXUS_SKIP_WASM_BUILD").is_some() {
        println!("cargo:warning=Skipping stdlib wasm build (NEXUS_SKIP_WASM_BUILD is set)");
        return;
    }

    let repo_root =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let artifact_dir = if profile == "release" {
        "release"
    } else {
        "debug"
    };
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    for module in MODULES {
        let manifest_path = repo_root.join(module.manifest_rel);
        build_wasm_crate(&cargo, &manifest_path, &profile);
    }

    let out_dir = repo_root.join("nxlib/stdlib");
    fs::create_dir_all(&out_dir).expect("failed to create nxlib/stdlib");

    for module in MODULES {
        let manifest_parent = repo_root
            .join(module.manifest_rel)
            .parent()
            .expect("manifest path has parent")
            .to_path_buf();
        let src = manifest_parent
            .join("target")
            .join(WASM_TARGET)
            .join(artifact_dir)
            .join(module.artifact_name);
        let dst = out_dir.join(module.output_name);
        copy_wasm(&src, &dst);
    }
}

fn build_wasm_crate(cargo: &str, manifest_path: &Path, profile: &str) {
    let mut cmd = Command::new(cargo);
    cmd.arg("build")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--target")
        .arg(WASM_TARGET)
        .env_remove("CARGO_TARGET_DIR");
    if profile == "release" {
        cmd.arg("--release");
    }

    let status = cmd.status().unwrap_or_else(|e| {
        panic!(
            "failed to run cargo build for {}: {}",
            manifest_path.display(),
            e
        )
    });
    assert!(
        status.success(),
        "wasm build failed for {} (run inside `nix develop` so wasm32-wasip1 is available)",
        manifest_path.display()
    );
}

fn copy_wasm(src: &Path, dst: &Path) {
    fs::copy(src, dst).unwrap_or_else(|e| {
        panic!(
            "failed to copy wasm artifact {} -> {}: {}",
            src.display(),
            dst.display(),
            e
        )
    });
}
