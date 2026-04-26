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
use crate::lang::package::WIT_NAMESPACE;
use crate::lang::stdlib::{is_package_wit_module, STDLIB_DIR};

/// The bundled stdlib WASM file path (used by wasm-merge).
fn stdlib_wasm_path() -> String {
    format!("{}/stdlib.wasm", STDLIB_DIR)
}

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
        // Package-qualified WIT-style imports (e.g. nexus:std/math) are still
        // resolved to the bundled stdlib.wasm. The WASM is rewritten before
        // merging to replace WIT module names with the physical path.
        if is_package_wit_module(module_name) {
            let stdlib_path = stdlib_wasm_path();
            out.insert(stdlib_path);
            continue;
        }
        // Skip host-provided nexus runtime/CLI modules (e.g. "nexus:runtime/backtrace").
        if module_name.starts_with(&format!("{}:", WIT_NAMESPACE)) {
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
    // Rewrite nexus:std/* WIT-style import module names to the physical
    // stdlib.wasm path so wasm-merge can resolve them as file-backed imports.
    let rewritten = rewrite_stdlib_wit_imports(&stripped);
    fs::write(&current_path, &rewritten)
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
        .arg("--rename-export-conflicts")
        .arg("--no-validation");

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

/// Rewrite package-qualified WIT-style import module names (e.g. `nexus:std/math`)
/// to the physical `nxlib/stdlib/stdlib.wasm` path so that wasm-merge can
/// resolve them. Field names (e.g. `__nx_abs_i64`) are unchanged — they match
/// stdlib.wasm exports.
fn rewrite_stdlib_wit_imports(wasm: &[u8]) -> Vec<u8> {
    use wasm_encoder::{EntityType, ImportSection, Module, RawSection};

    // Quick check: does the binary contain any package-qualified imports?
    let has_wit_imports = wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| p.ok())
        .any(|p| {
            if let Payload::ImportSection(section) = p {
                section
                    .into_iter()
                    .any(|i| i.map_or(false, |i| is_package_wit_module(i.module)))
            } else {
                false
            }
        });
    if !has_wit_imports {
        return wasm.to_vec();
    }

    let stdlib_path = stdlib_wasm_path();
    let parser = wasmparser::Parser::new(0);
    let mut module = Module::new();
    for payload in parser.parse_all(wasm) {
        let payload = match payload {
            Ok(p) => p,
            Err(_) => return wasm.to_vec(),
        };
        match &payload {
            Payload::ImportSection(section) => {
                let mut imports = ImportSection::new();
                for import in section.clone() {
                    let import = match import {
                        Ok(i) => i,
                        Err(_) => return wasm.to_vec(),
                    };
                    let from_package = is_package_wit_module(import.module);
                    let module_name = if from_package {
                        stdlib_path.as_str()
                    } else {
                        import.module
                    };
                    // Reverse WIT canonicalization: "string-length" → "__nx_string_length"
                    // so import names match stdlib.wasm exports.
                    let field_name_owned;
                    let field_name = if from_package
                        && !import.name.starts_with("__nx_")
                        && import.name != "allocate"
                        && import.name != "deallocate"
                    {
                        field_name_owned = format!("__nx_{}", import.name.replace('-', "_"));
                        field_name_owned.as_str()
                    } else {
                        import.name
                    };
                    let entity = match import.ty {
                        wasmparser::TypeRef::Func(idx) => EntityType::Function(idx),
                        wasmparser::TypeRef::Table(t) => {
                            EntityType::Table(wasm_encoder::TableType {
                                element_type: wasm_encoder::RefType {
                                    nullable: t.element_type.is_nullable(),
                                    heap_type: match t.element_type.heap_type() {
                                        wasmparser::HeapType::Abstract { shared: _, ty } => {
                                            match ty {
                                                wasmparser::AbstractHeapType::Func => {
                                                    wasm_encoder::HeapType::Abstract {
                                                        shared: false,
                                                        ty: wasm_encoder::AbstractHeapType::Func,
                                                    }
                                                }
                                                wasmparser::AbstractHeapType::Extern => {
                                                    wasm_encoder::HeapType::Abstract {
                                                        shared: false,
                                                        ty: wasm_encoder::AbstractHeapType::Extern,
                                                    }
                                                }
                                                _ => return wasm.to_vec(),
                                            }
                                        }
                                        _ => return wasm.to_vec(),
                                    },
                                },
                                minimum: t.initial,
                                maximum: t.maximum,
                                table64: t.table64,
                                shared: false,
                            })
                        }
                        wasmparser::TypeRef::Memory(m) => {
                            EntityType::Memory(wasm_encoder::MemoryType {
                                minimum: m.initial,
                                maximum: m.maximum,
                                memory64: m.memory64,
                                shared: m.shared,
                                page_size_log2: m.page_size_log2,
                            })
                        }
                        wasmparser::TypeRef::Global(g) => {
                            let val_type = match g.content_type {
                                wasmparser::ValType::I32 => wasm_encoder::ValType::I32,
                                wasmparser::ValType::I64 => wasm_encoder::ValType::I64,
                                wasmparser::ValType::F32 => wasm_encoder::ValType::F32,
                                wasmparser::ValType::F64 => wasm_encoder::ValType::F64,
                                _ => return wasm.to_vec(),
                            };
                            EntityType::Global(wasm_encoder::GlobalType {
                                val_type,
                                mutable: g.mutable,
                                shared: g.shared,
                            })
                        }
                        wasmparser::TypeRef::Tag(t) => EntityType::Tag(wasm_encoder::TagType {
                            kind: wasm_encoder::TagKind::Exception,
                            func_type_idx: t.func_type_idx,
                        }),
                    };
                    imports.import(module_name, field_name, entity);
                }
                module.section(&imports);
                continue;
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

/// Merge stub modules for remaining host imports after bundling.
/// Handles nexus:cli/nexus-host (unreachable stubs) and nexus:runtime/backtrace (no-op stubs).
pub fn merge_remaining_stubs(wasm: &[u8], wasm_merge_command: &Path) -> Result<Vec<u8>, String> {
    let imports = module_import_names(wasm)?;
    let mut result = wasm.to_vec();

    // Merge nexus-host stubs if present
    if imports.contains(NEXUS_HOST_HTTP_MODULE) {
        let stub = build_stub_module(&[
            ("host-http-request", &[I32; 9], &[]),
            ("host-http-listen", &[I32, I32], &[I64]),
            ("host-http-accept", &[I64, I32], &[]),
            ("host-http-respond", &[I64, I64, I32, I32, I32, I32], &[I32]),
            ("host-http-stop", &[I64], &[I32]),
        ]);
        result = merge_stub(&result, &stub, NEXUS_HOST_HTTP_MODULE, wasm_merge_command)?;
    }

    // Merge backtrace stubs if present
    if imports.contains("nexus:runtime/backtrace") {
        let stub = build_stub_module(&[
            ("__nx_capture_backtrace", &[], &[]),
            ("__nx_bt_depth", &[], &[I64]),
            ("__nx_bt_frame", &[I64], &[I64]),
        ]);
        result = merge_stub(
            &result,
            &stub,
            "nexus:runtime/backtrace",
            wasm_merge_command,
        )?;
    }

    Ok(result)
}

use wasm_encoder::ValType::{I32, I64};

fn build_stub_module(
    funcs: &[(&str, &[wasm_encoder::ValType], &[wasm_encoder::ValType])],
) -> Vec<u8> {
    use wasm_encoder::*;
    let mut module = Module::new();
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut codes = CodeSection::new();
    for (i, (name, params, results)) in funcs.iter().enumerate() {
        types.ty().function(params.to_vec(), results.to_vec());
        functions.function(i as u32);
        exports.export(name, ExportKind::Func, i as u32);
        let mut f = Function::new(vec![]);
        if results.is_empty() {
            f.instruction(&Instruction::End);
        } else {
            for r in *results {
                match r {
                    ValType::I32 => f.instruction(&Instruction::I32Const(0)),
                    ValType::I64 => f.instruction(&Instruction::I64Const(0)),
                    _ => f.instruction(&Instruction::I32Const(0)),
                };
            }
            f.instruction(&Instruction::End);
        }
        codes.function(&f);
    }
    module.section(&types);
    module.section(&functions);
    module.section(&exports);
    module.section(&codes);
    module.finish()
}

fn merge_stub(
    wasm: &[u8],
    stub: &[u8],
    module_name: &str,
    wasm_merge_command: &Path,
) -> Result<Vec<u8>, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-stub-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("failed to create temp dir: {}", e))?;
    let main_path = temp_dir.join("main.wasm");
    let stub_path = temp_dir.join("stub.wasm");
    let merged_path = temp_dir.join("merged.wasm");
    fs::write(&main_path, wasm).map_err(|e| format!("write main: {}", e))?;
    fs::write(&stub_path, stub).map_err(|e| format!("write stub: {}", e))?;
    let output = ProcessCommand::new(wasm_merge_command)
        .arg(&main_path)
        .arg(WASM_MERGE_MAIN_NAME)
        .arg(&stub_path)
        .arg(module_name)
        .arg("--all-features")
        .arg("--enable-tail-call")
        .arg("--enable-multimemory")
        .arg("--no-validation")
        .arg("--skip-export-conflicts")
        .arg("-o")
        .arg(&merged_path)
        .output()
        .map_err(|e| format!("wasm-merge stub: {}", e))?;
    let result = if output.status.success() {
        fs::read(&merged_path).map_err(|e| format!("read merged: {}", e))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("wasm-merge stub failed: {}", stderr.trim()))
    };
    let _ = fs::remove_dir_all(&temp_dir);
    result
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
