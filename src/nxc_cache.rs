//! Auto-build and cache management for nxc_driver.wasm.
//!
//! On first invocation (or when nxc/ sources change), compiles nxc/driver.nx
//! via the Rust compiler pipeline and caches the resulting WASM artifact under
//! `target/nxc/`.  Subsequent calls return the cached path immediately.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::cli::LoadedSource;
use crate::driver::compile_loaded_source_to_wasm;
use nexus::compiler::bundler;

/// Subdirectory under `target/` for cached nxc build artifacts.
const CACHE_DIR: &str = "target/nxc";
const CACHE_WASM: &str = "nxc_driver.wasm";
const CACHE_HASH_FILE: &str = "sources.hash";

/// Source directories whose `.nx` files feed into nxc_driver.wasm.
const SOURCE_DIRS: &[&str] = &["nxc", "nxlib/stdlib"];

/// Entry point for the self-hosted compiler.
const NXC_ENTRY: &str = "nxc/driver.nx";

/// Ensure nxc_driver.wasm is built and cached.  Returns the path to the
/// cached WASM file, building it from source if the cache is stale or missing.
pub fn ensure_nxc_driver(project_root: &Path, verbose: bool) -> Result<PathBuf, String> {
    let cache_dir = project_root.join(CACHE_DIR);
    let wasm_path = cache_dir.join(CACHE_WASM);

    let current_hash = compute_sources_hash(project_root)
        .map_err(|e| format!("Failed to hash nxc sources: {}", e))?;
    let current_hex = format!("{:016x}", current_hash);

    if is_cache_valid(&cache_dir, &current_hex) {
        if verbose {
            eprintln!("[nxc] Cache hit — using target/nxc/nxc_driver.wasm");
        }
        return Ok(wasm_path);
    }

    eprintln!("[nxc] Building nxc_driver.wasm (cache miss)...");

    fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create {}: {}", cache_dir.display(), e))?;

    let entry = project_root.join(NXC_ENTRY);
    let source = fs::read_to_string(&entry)
        .map_err(|e| format!("Failed to read {}: {}", entry.display(), e))?;
    let loaded = LoadedSource {
        display_name: entry.display().to_string(),
        source,
    };

    let wasm_merge_command = bundler::resolve_wasm_merge_command(None);
    let compiled = compile_loaded_source_to_wasm(&loaded, true, &wasm_merge_command, verbose)
        .map_err(|_| "Failed to compile nxc/driver.nx".to_string())?;

    fs::write(&wasm_path, &compiled.wasm)
        .map_err(|e| format!("Failed to write {}: {}", wasm_path.display(), e))?;

    let hash_path = cache_dir.join(CACHE_HASH_FILE);
    fs::write(&hash_path, format!("{}\n", current_hex))
        .map_err(|e| format!("Failed to write hash file: {}", e))?;

    eprintln!(
        "[nxc] Cached nxc_driver.wasm ({} bytes)",
        compiled.wasm.len()
    );

    Ok(wasm_path)
}

/// Check whether the cached WASM file exists and its source hash matches.
fn is_cache_valid(cache_dir: &Path, expected_hex: &str) -> bool {
    let wasm_path = cache_dir.join(CACHE_WASM);
    let hash_path = cache_dir.join(CACHE_HASH_FILE);

    if !wasm_path.exists() {
        return false;
    }

    let stored = match fs::read_to_string(&hash_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    stored.trim() == expected_hex
}

/// Compute a deterministic hash over all `.nx` files in [`SOURCE_DIRS`].
fn compute_sources_hash(project_root: &Path) -> std::io::Result<u64> {
    let mut hasher = DefaultHasher::new();
    let mut paths: Vec<PathBuf> = Vec::new();

    for dir in SOURCE_DIRS {
        let dir_path = project_root.join(dir);
        if dir_path.is_dir() {
            collect_nx_files(&dir_path, &mut paths)?;
        }
    }

    // Sort for determinism across platforms / filesystem ordering.
    paths.sort();

    for path in &paths {
        let rel = path.strip_prefix(project_root).unwrap_or(path);
        rel.to_string_lossy().hash(&mut hasher);
        let contents = fs::read(path)?;
        contents.hash(&mut hasher);
    }

    Ok(hasher.finish())
}

/// Recursively collect all `.nx` files under `dir`.
fn collect_nx_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_nx_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "nx") {
            out.push(path);
        }
    }
    Ok(())
}
