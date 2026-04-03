//! WASM dependency bundler — resolves file-backed imports via `wasm-merge`.
//!
//! Extracted from the binary crate so that both `nexus build`, `nexus run`,
//! and the REPL can bundle stdlib / external wasm dependencies.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use wasmparser::Payload;

use crate::constants::{is_preview2_wasi_module, NEXUS_HOST_HTTP_MODULE, WASI_SNAPSHOT_MODULE};

const WASM_MERGE_PATH_ENV: &str = "NEXUS_WASM_MERGE";
pub const WASM_MERGE_MAIN_NAME: &str = "__nexus_main__";

pub struct BundleConfig {
    pub wasm_merge_command: PathBuf,
    pub allow_nexus_host_import: bool,
}

impl Default for BundleConfig {
    fn default() -> Self {
        BundleConfig {
            wasm_merge_command: resolve_wasm_merge_command(None),
            allow_nexus_host_import: true,
        }
    }
}

/// Resolve the `wasm-merge` command path from CLI override, env var, or PATH default.
pub fn resolve_wasm_merge_command(cli_override: Option<&Path>) -> PathBuf {
    if let Some(path) = cli_override {
        return path.to_path_buf();
    }
    if let Some(path) = std::env::var_os(WASM_MERGE_PATH_ENV) {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    PathBuf::from("wasm-merge")
}

/// Bundle file-backed external imports (stdlib, etc.) into the core WASM.
/// Returns the merged WASM bytes with all file-backed imports resolved.
pub fn bundle_core_wasm(wasm: &[u8], config: &BundleConfig) -> Result<Vec<u8>, String> {
    let imports = module_import_names(wasm)?;
    let unresolved = file_backed_imports(&imports, config.allow_nexus_host_import)?;
    if unresolved.is_empty() {
        return Ok(wasm.to_vec());
    }
    let candidate_modules = bundle_candidate_modules(&unresolved, config.allow_nexus_host_import)?;
    let merged = merge_dependencies_once(wasm, &candidate_modules, &config.wasm_merge_command)?;
    let merged_imports = module_import_names(&merged)?;
    let merged_unresolved = file_backed_imports(&merged_imports, config.allow_nexus_host_import)?;
    if !merged_unresolved.is_empty() {
        let unresolved_list = merged_unresolved
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "failed to resolve imports while bundling; unresolved after internal linker pass: {}",
            unresolved_list
        ));
    }
    Ok(merged)
}

/// Returns the set of unique import module names from a WASM binary.
pub fn module_import_names(wasm: &[u8]) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| format!("failed to parse wasm: {}", e))?;
        if let Payload::ImportSection(section) = payload {
            for import in section {
                let import =
                    import.map_err(|e| format!("failed to parse wasm import section: {}", e))?;
                out.insert(import.module.to_string());
            }
        }
    }
    Ok(out)
}

fn file_backed_imports(
    imports: &BTreeSet<String>,
    allow_nexus_host_import: bool,
) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for module_name in imports {
        if module_name == WASI_SNAPSHOT_MODULE {
            continue;
        }
        if module_name == NEXUS_HOST_HTTP_MODULE {
            if allow_nexus_host_import {
                continue;
            }
            return Err(format!(
                "import module '{}' is deprecated; use component builds (`nexus build`) for HTTP",
                NEXUS_HOST_HTTP_MODULE
            ));
        }
        if is_preview2_wasi_module(module_name) {
            continue;
        }
        // Skip host-provided nexus runtime modules (e.g. "nexus:runtime/backtrace").
        // NEXUS_HOST_HTTP_MODULE is handled above with its own conditional logic.
        if module_name.starts_with("nexus:") {
            continue;
        }
        let path = Path::new(module_name);
        if !path.exists() {
            return Err(format!(
                "import module '{}' is not a local wasm path; cannot bundle dynamically",
                module_name
            ));
        }
        out.insert(module_name.clone());
    }
    Ok(out)
}

fn bundle_candidate_modules(
    unresolved: &BTreeSet<String>,
    allow_nexus_host_import: bool,
) -> Result<Vec<String>, String> {
    let mut leaf = Vec::new();
    let mut non_leaf = Vec::new();
    for candidate in unresolved.iter().rev() {
        let candidate_wasm = fs::read(candidate).map_err(|e| {
            format!(
                "failed to read dependency module '{}' while resolving bundle order: {}",
                candidate, e
            )
        })?;
        let candidate_imports = module_import_names(&candidate_wasm)?;
        let candidate_unresolved =
            file_backed_imports(&candidate_imports, allow_nexus_host_import)?;
        let depends_on_other_unresolved = candidate_unresolved
            .iter()
            .any(|dep| dep != candidate && unresolved.contains(dep));
        if depends_on_other_unresolved {
            non_leaf.push(candidate.clone());
        } else {
            leaf.push(candidate.clone());
        }
    }

    leaf.extend(non_leaf);
    Ok(leaf)
}

fn merge_dependencies_once(
    current_wasm: &[u8],
    module_names: &[String],
    wasm_merge_command: &Path,
) -> Result<Vec<u8>, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-bundle-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("failed to create temp bundle directory: {}", e))?;

    let current_path = temp_dir.join("current.wasm");
    let merged_path = temp_dir.join("merged.wasm");
    // Strip .debug_* custom sections before merge — wasm-merge v124 crashes (SIGSEGV)
    // when it encounters DWARF debug sections. They will be re-emitted post-bundle
    // if needed (currently unbundled output only).
    let stripped = strip_debug_sections(current_wasm);
    fs::write(&current_path, &stripped)
        .map_err(|e| format!("failed to write temporary wasm: {}", e))?;

    let mut command = ProcessCommand::new(wasm_merge_command);
    command.arg(&current_path).arg(WASM_MERGE_MAIN_NAME);
    for module_name in module_names {
        let dep_path = PathBuf::from(module_name).canonicalize().map_err(|e| {
            format!(
                "failed to resolve import module '{}' as a filesystem path: {}",
                module_name, e
            )
        })?;
        command.arg(dep_path).arg(module_name);
    }
    command
        .arg("--all-features")
        .arg("--enable-tail-call")
        .arg("--enable-exception-handling")
        .arg("--enable-multimemory")
        .arg("-o")
        .arg(&merged_path)
        .arg("--rename-export-conflicts");

    let output = command.output().map_err(|e| {
        format!(
            "failed to execute '{}' while bundling dependencies: {} (use `--wasm-merge PATH` or {} env var)",
            wasm_merge_command.display(),
            e,
            WASM_MERGE_PATH_ENV
        )
    })?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let detail = if stderr.is_empty() {
            format!("exit status {}", output.status)
        } else {
            format!("exit status {}: {}", output.status, stderr)
        };
        return Err(format!(
            "external wasm linker '{}' failed while bundling [{}] ({})",
            wasm_merge_command.display(),
            module_names.join(", "),
            detail
        ));
    }

    let merged =
        fs::read(&merged_path).map_err(|e| format!("failed to read merged wasm output: {}", e))?;
    let _ = fs::remove_dir_all(&temp_dir);
    Ok(merged)
}

/// Remove `.debug_*` custom sections from a WASM binary.
/// Needed because wasm-merge v124 crashes on DWARF sections.
fn strip_debug_sections(wasm: &[u8]) -> Vec<u8> {
    use wasm_encoder::{Module, RawSection};
    let parser = wasmparser::Parser::new(0);
    let mut module = Module::new();
    for payload in parser.parse_all(wasm) {
        let payload = match payload {
            Ok(p) => p,
            Err(_) => return wasm.to_vec(), // fallback: return as-is
        };
        match &payload {
            Payload::CustomSection(reader) if reader.name().starts_with(".debug_") => {
                continue; // skip debug sections
            }
            _ => {}
        }
        if let Some((id, range)) = payload.as_section() {
            module.section(&RawSection {
                id,
                data: &wasm[range],
            });
        }
    }
    module.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backed_imports_rejects_legacy_nexus_host_module() {
        let mut imports = BTreeSet::new();
        imports.insert(WASI_SNAPSHOT_MODULE.to_string());
        imports.insert("wasi:http/outgoing-handler@0.2.0".to_string());
        imports.insert(NEXUS_HOST_HTTP_MODULE.to_string());

        let err = file_backed_imports(&imports, false)
            .expect_err("legacy nexus host module should be rejected");
        assert!(
            err.contains(NEXUS_HOST_HTTP_MODULE),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn file_backed_imports_skips_nexus_runtime_modules() {
        let mut imports = BTreeSet::new();
        imports.insert(WASI_SNAPSHOT_MODULE.to_string());
        imports.insert("nexus:runtime/backtrace".to_string());

        let result = file_backed_imports(&imports, true).expect("should succeed");
        assert!(
            result.is_empty(),
            "nexus:runtime/backtrace should be skipped, got: {:?}",
            result
        );
    }
}
