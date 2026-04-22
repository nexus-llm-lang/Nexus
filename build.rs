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
    /// Extra cargo features to pass when building (e.g. "component").
    features: &'static str,
}

const MODULES: &[WasmModule] = &[
    WasmModule {
        manifest_rel: "src/lib/stdlib_bundle/Cargo.toml",
        src_rel: "src/lib/stdlib_bundle/src",
        artifact_name: "nexus_stdlib_bundle.wasm",
        output_name: "stdlib.wasm",
        features: "",
    },
    // Component model build of stdlib: wit-bindgen canonical ABI exports.
    WasmModule {
        manifest_rel: "src/lib/stdlib_bundle/Cargo.toml",
        src_rel: "src/lib/stdlib_bundle/src",
        artifact_name: "nexus_stdlib_bundle.wasm",
        output_name: "stdlib-component.wasm",
        features: "component",
    },
    WasmModule {
        manifest_rel: "src/lib/nexus_host_bridge/Cargo.toml",
        src_rel: "src/lib/nexus_host_bridge",
        artifact_name: "nexus_nexus_host_bridge_wasm.wasm",
        output_name: "nexus-host-bridge.wasm",
        features: "",
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

    // Track WIT files for component model rebuilds.
    println!("cargo:rerun-if-changed=src/lib/stdlib_bundle/wit");

    let out_dir = repo_root.join("nxlib/stdlib");
    fs::create_dir_all(&out_dir).expect("failed to create nxlib/stdlib");

    // Build and copy each module immediately (multiple builds of the same crate
    // with different features share the same artifact path, so we must copy
    // before the next build overwrites it).
    for module in MODULES {
        let manifest_path = repo_root.join(module.manifest_rel);
        build_wasm_crate(&cargo, &manifest_path, &profile, module.features);

        let manifest_parent = manifest_path
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

        if module.output_name == "stdlib.wasm" {
            stub_nexus_cli_imports(&dst);
        }
    }
}

fn build_wasm_crate(cargo: &str, manifest_path: &Path, profile: &str, features: &str) {
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
    if !features.is_empty() {
        cmd.arg("--features").arg(features);
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

// Replace `nexus:cli/*` function imports in stdlib.wasm with local
// `unreachable` stubs. The wasm_merge self-hosting path bundles stdlib into a
// core WASM run by plain wasmtime, which can't resolve host imports — stubbing
// makes the merged output self-contained. The component build
// (stdlib-component.wasm) is not stubbed; it's composed with nexus-host.

fn stub_nexus_cli_imports(path: &Path) {
    let data = fs::read(path).unwrap_or_else(|e| {
        panic!("failed to read {}: {}", path.display(), e);
    });
    let sections = parse_sections(&data);

    let (_, imp_off, _imp_size) = *sections
        .iter()
        .find(|(id, _, _)| *id == 2)
        .expect("no import section (id=2) in stdlib.wasm");
    let imports = parse_imports(&data, imp_off);

    let has_cli = imports.iter().any(|imp| imp.module.starts_with("nexus:cli"));
    if !has_cli {
        return;
    }

    let wasi_idxs: Vec<usize> = imports
        .iter()
        .enumerate()
        .filter(|(_, imp)| imp.module.starts_with("wasi_snapshot"))
        .map(|(i, _)| i)
        .collect();
    let cli_idxs: Vec<usize> = imports
        .iter()
        .enumerate()
        .filter(|(_, imp)| imp.module.starts_with("nexus:cli"))
        .map(|(i, _)| i)
        .collect();

    let num_wasi_func = wasi_idxs.iter().filter(|&&i| imports[i].kind == 0).count();
    let num_cli_func = cli_idxs.iter().filter(|&&i| imports[i].kind == 0).count();
    let num_old_func_imports = imports.iter().filter(|imp| imp.kind == 0).count();

    // Build function index remap: old idx -> new idx.
    let mut func_remap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut old_fidx = 0u32;
    let mut wasi_fidx = 0u32;
    let mut cli_fidx = 0u32;
    for imp in &imports {
        if imp.kind != 0 {
            continue;
        }
        if imp.module.starts_with("wasi_snapshot") {
            func_remap.insert(old_fidx, wasi_fidx);
            wasi_fidx += 1;
        } else {
            func_remap.insert(old_fidx, num_wasi_func as u32 + cli_fidx);
            cli_fidx += 1;
        }
        old_fidx += 1;
    }

    // Locals shift: old local N at idx (num_old_func_imports + N), new at
    // (num_wasi_func + num_cli_func + N).
    let local_shift: i64 =
        (num_wasi_func + num_cli_func) as i64 - num_old_func_imports as i64;

    // Collect CLI function type indices for stub function types.
    let cli_func_types: Vec<u32> = cli_idxs
        .iter()
        .filter_map(|&i| {
            if imports[i].kind == 0 {
                imports[i].func_type
            } else {
                None
            }
        })
        .collect();

    // Rebuild import section: WASI imports only, verbatim.
    let mut new_imp = Vec::new();
    write_uleb128(&mut new_imp, wasi_idxs.len() as u64);
    for &i in &wasi_idxs {
        new_imp.extend_from_slice(&imports[i].raw);
    }

    // Rebuild function section: prepend stub type indices.
    let (_, func_off, func_size) = *sections
        .iter()
        .find(|(id, _, _)| *id == 3)
        .expect("no function section (id=3)");
    let (func_count, fpos) = read_uleb128(&data, func_off);
    let mut new_func = Vec::new();
    write_uleb128(&mut new_func, num_cli_func as u64 + func_count);
    for t in &cli_func_types {
        write_uleb128(&mut new_func, *t as u64);
    }
    new_func.extend_from_slice(&data[fpos..func_off + func_size]);

    // Rebuild code section: prepend stub bodies (`unreachable`, `end`).
    let (_, code_off, code_size) = *sections
        .iter()
        .find(|(id, _, _)| *id == 10)
        .expect("no code section (id=10)");
    let (code_count, mut cpos) = read_uleb128(&data, code_off);
    let mut new_code = Vec::new();
    write_uleb128(&mut new_code, num_cli_func as u64 + code_count);
    for _ in 0..num_cli_func {
        write_uleb128(&mut new_code, 3); // body size = 3
        new_code.push(0x00); // 0 locals
        new_code.push(0x00); // unreachable
        new_code.push(0x0B); // end
    }
    let _ = code_size;
    for _ in 0..code_count {
        let (body_size, body_start) = read_uleb128(&data, cpos);
        let body_end = body_start + body_size as usize;
        let new_body =
            rewrite_body(&data, body_start, body_end, &func_remap, num_old_func_imports as u32, local_shift);
        write_uleb128(&mut new_code, new_body.len() as u64);
        new_code.extend_from_slice(&new_body);
        cpos = body_end;
    }

    // Rebuild element section if present.
    let new_elem = sections
        .iter()
        .find(|(id, _, _)| *id == 9)
        .map(|(_, elem_off, _)| {
            let mut pos = *elem_off;
            let (elem_count, np) = read_uleb128(&data, pos);
            pos = np;
            let mut eb = Vec::new();
            write_uleb128(&mut eb, elem_count);
            for _ in 0..elem_count {
                let kind = data[pos];
                pos += 1;
                eb.push(kind);
                if kind == 0 {
                    // Copy offset expression until `end` (0x0B).
                    while data[pos] != 0x0B {
                        eb.push(data[pos]);
                        pos += 1;
                    }
                    eb.push(data[pos]);
                    pos += 1;
                    let (n, np) = read_uleb128(&data, pos);
                    pos = np;
                    write_uleb128(&mut eb, n);
                    for _ in 0..n {
                        let (idx, np) = read_uleb128(&data, pos);
                        pos = np;
                        let idx = idx as u32;
                        let new_idx = if (idx as usize) < num_old_func_imports {
                            *func_remap.get(&idx).unwrap_or(&idx)
                        } else {
                            ((idx as i64) + local_shift) as u32
                        };
                        write_uleb128(&mut eb, new_idx as u64);
                    }
                }
            }
            eb
        });

    // Rebuild export section.
    let (_, exp_off, _) = *sections
        .iter()
        .find(|(id, _, _)| *id == 7)
        .expect("no export section (id=7)");
    let mut pos = exp_off;
    let (exp_count, np) = read_uleb128(&data, pos);
    pos = np;
    let mut new_exp = Vec::new();
    write_uleb128(&mut new_exp, exp_count);
    for _ in 0..exp_count {
        let (nl, np) = read_uleb128(&data, pos);
        pos = np;
        write_uleb128(&mut new_exp, nl);
        new_exp.extend_from_slice(&data[pos..pos + nl as usize]);
        pos += nl as usize;
        let kind = data[pos];
        pos += 1;
        new_exp.push(kind);
        let (idx, np) = read_uleb128(&data, pos);
        pos = np;
        if kind == 0 {
            let idx = idx as u32;
            let new_idx = if (idx as usize) < num_old_func_imports {
                *func_remap.get(&idx).unwrap_or(&idx)
            } else {
                ((idx as i64) + local_shift) as u32
            };
            write_uleb128(&mut new_exp, new_idx as u64);
        } else {
            write_uleb128(&mut new_exp, idx);
        }
    }

    // Assemble output with replaced sections.
    let mut output = Vec::new();
    output.extend_from_slice(&data[..8]); // magic + version
    for (sec_id, sec_off, sec_size) in &sections {
        match sec_id {
            2 => {
                output.push(*sec_id);
                write_uleb128(&mut output, new_imp.len() as u64);
                output.extend_from_slice(&new_imp);
            }
            3 => {
                output.push(*sec_id);
                write_uleb128(&mut output, new_func.len() as u64);
                output.extend_from_slice(&new_func);
            }
            7 => {
                output.push(*sec_id);
                write_uleb128(&mut output, new_exp.len() as u64);
                output.extend_from_slice(&new_exp);
            }
            9 if new_elem.is_some() => {
                let ne = new_elem.as_ref().unwrap();
                output.push(*sec_id);
                write_uleb128(&mut output, ne.len() as u64);
                output.extend_from_slice(ne);
            }
            10 => {
                output.push(*sec_id);
                write_uleb128(&mut output, new_code.len() as u64);
                output.extend_from_slice(&new_code);
            }
            _ => {
                output.push(*sec_id);
                write_uleb128(&mut output, *sec_size as u64);
                output.extend_from_slice(&data[*sec_off..*sec_off + *sec_size]);
            }
        }
    }
    fs::write(path, &output).unwrap_or_else(|e| {
        panic!("failed to write stubbed {}: {}", path.display(), e);
    });
}

// ─── WASM binary format helpers ──────────────────────────────────────────

fn read_uleb128(data: &[u8], mut pos: usize) -> (u64, usize) {
    let mut val: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let b = data[pos];
        pos += 1;
        val |= ((b & 0x7f) as u64) << shift;
        if b < 0x80 {
            return (val, pos);
        }
        shift += 7;
    }
}

fn read_sleb128(data: &[u8], mut pos: usize) -> (i64, usize) {
    let mut val: i64 = 0;
    let mut shift: u32 = 0;
    loop {
        let b = data[pos];
        pos += 1;
        val |= ((b & 0x7f) as i64) << shift;
        shift += 7;
        if b < 0x80 {
            if shift < 64 && (b & 0x40) != 0 {
                val = val.wrapping_sub(1i64.wrapping_shl(shift));
            }
            return (val, pos);
        }
    }
}

fn write_uleb128(out: &mut Vec<u8>, mut val: u64) {
    loop {
        let b = (val & 0x7f) as u8;
        val >>= 7;
        if val != 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
            return;
        }
    }
}

fn parse_sections(data: &[u8]) -> Vec<(u8, usize, usize)> {
    let mut sections = Vec::new();
    let mut pos = 8;
    while pos < data.len() {
        let sec_id = data[pos];
        pos += 1;
        let (sec_size, np) = read_uleb128(data, pos);
        pos = np;
        sections.push((sec_id, pos, sec_size as usize));
        pos += sec_size as usize;
    }
    sections
}

struct Import {
    module: String,
    kind: u8,
    func_type: Option<u32>,
    raw: Vec<u8>,
}

fn parse_imports(data: &[u8], sec_offset: usize) -> Vec<Import> {
    let mut pos = sec_offset;
    let (count, np) = read_uleb128(data, pos);
    pos = np;
    let mut imports = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let start = pos;
        let (mod_len, np) = read_uleb128(data, pos);
        pos = np;
        let module = String::from_utf8(data[pos..pos + mod_len as usize].to_vec())
            .expect("non-utf8 module name");
        pos += mod_len as usize;
        let (name_len, np) = read_uleb128(data, pos);
        pos = np;
        pos += name_len as usize;
        let kind = data[pos];
        pos += 1;
        let mut func_type = None;
        match kind {
            0 => {
                let (t, np) = read_uleb128(data, pos);
                pos = np;
                func_type = Some(t as u32);
            }
            1 => {
                pos += 1; // reftype
                let (flags, np) = read_uleb128(data, pos);
                pos = np;
                let (_, np) = read_uleb128(data, pos);
                pos = np;
                if flags & 1 != 0 {
                    let (_, np) = read_uleb128(data, pos);
                    pos = np;
                }
            }
            2 => {
                let (flags, np) = read_uleb128(data, pos);
                pos = np;
                let (_, np) = read_uleb128(data, pos);
                pos = np;
                if flags & 1 != 0 {
                    let (_, np) = read_uleb128(data, pos);
                    pos = np;
                }
            }
            3 => {
                pos += 2;
            }
            _ => panic!("unknown import kind {}", kind),
        }
        imports.push(Import {
            module,
            kind,
            func_type,
            raw: data[start..pos].to_vec(),
        });
    }
    imports
}

// Rewrites a function body, remapping `call`, `return_call`, and `ref.func`
// targets via func_remap (or local shift for non-import locals).
fn rewrite_body(
    data: &[u8],
    start: usize,
    end: usize,
    func_remap: &std::collections::HashMap<u32, u32>,
    num_old_func_imports: u32,
    local_shift: i64,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = start;
    // Copy locals header verbatim.
    let (num_locals, np) = read_uleb128(data, pos);
    out.extend_from_slice(&data[pos..np]);
    pos = np;
    for _ in 0..num_locals {
        let (_, np) = read_uleb128(data, pos);
        let np = np + 1; // valtype byte
        out.extend_from_slice(&data[pos..np]);
        pos = np;
    }
    // Walk instructions.
    while pos < end {
        let op = data[pos];
        match op {
            0x10 | 0x12 => {
                // call | return_call — remap idx
                out.push(op);
                pos += 1;
                let old_pos = pos;
                let (idx, np) = read_uleb128(data, old_pos);
                pos = np;
                let idx = idx as u32;
                let new_idx = if idx < num_old_func_imports {
                    *func_remap.get(&idx).unwrap_or(&idx)
                } else {
                    ((idx as i64) + local_shift) as u32
                };
                if new_idx != idx {
                    write_uleb128(&mut out, new_idx as u64);
                } else {
                    out.extend_from_slice(&data[old_pos..pos]);
                }
            }
            0xD2 => {
                // ref.func — remap idx
                out.push(op);
                pos += 1;
                let old_pos = pos;
                let (idx, np) = read_uleb128(data, old_pos);
                pos = np;
                let idx = idx as u32;
                let new_idx = if idx < num_old_func_imports {
                    *func_remap.get(&idx).unwrap_or(&idx)
                } else {
                    ((idx as i64) + local_shift) as u32
                };
                if new_idx != idx {
                    write_uleb128(&mut out, new_idx as u64);
                } else {
                    out.extend_from_slice(&data[old_pos..pos]);
                }
            }
            _ => {
                out.push(op);
                pos += 1;
                pos = copy_operands(op, data, pos, &mut out);
            }
        }
    }
    out
}

// Copies operand bytes for `op` (already written to `out`) and returns the new pos.
fn copy_operands(op: u8, data: &[u8], mut pos: usize, out: &mut Vec<u8>) -> usize {
    match op {
        0x02 | 0x03 | 0x04 => {
            let b = data[pos];
            if b == 0x40 || b >= 0x60 {
                out.push(b);
                pos += 1;
            } else {
                let (_, np) = read_sleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
            }
        }
        0x06 => {
            let b = data[pos];
            if b == 0x40 || b >= 0x60 {
                out.push(b);
                pos += 1;
            } else {
                let (_, np) = read_sleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
            }
            let (nh, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
            for _ in 0..nh {
                let (_, np) = read_uleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
                let (_, np) = read_uleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
            }
        }
        0x08 | 0x0C | 0x0D | 0x20 | 0x21 | 0x22 | 0x23 | 0x24 => {
            let (_, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
        }
        0x0E => {
            let (cnt, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
            for _ in 0..(cnt + 1) {
                let (_, np) = read_uleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
            }
        }
        0x11 => {
            let (_, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
            let (_, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
        }
        0x28..=0x3E => {
            let (_, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
            let (_, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
        }
        0x3F | 0x40 => {
            out.push(data[pos]);
            pos += 1;
        }
        0x41 => {
            let (_, np) = read_sleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
        }
        0x42 => {
            let (_, np) = read_sleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
        }
        0x43 => {
            out.extend_from_slice(&data[pos..pos + 4]);
            pos += 4;
        }
        0x44 => {
            out.extend_from_slice(&data[pos..pos + 8]);
            pos += 8;
        }
        0xFC => {
            let (sub_op, np) = read_uleb128(data, pos);
            out.extend_from_slice(&data[pos..np]);
            pos = np;
            if sub_op <= 7 {
                // no operand
            } else if sub_op == 10 {
                out.extend_from_slice(&data[pos..pos + 2]);
                pos += 2;
            } else if sub_op == 11 {
                out.push(data[pos]);
                pos += 1;
            } else if sub_op == 8 || sub_op == 9 {
                let (_, np) = read_uleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
                if sub_op == 8 {
                    out.push(data[pos]);
                    pos += 1;
                }
            } else if (12..=17).contains(&sub_op) {
                let (_, np) = read_uleb128(data, pos);
                out.extend_from_slice(&data[pos..np]);
                pos = np;
                if sub_op == 12 || sub_op == 14 {
                    let (_, np) = read_uleb128(data, pos);
                    out.extend_from_slice(&data[pos..np]);
                    pos = np;
                }
            }
        }
        _ => {}
    }
    pos
}
