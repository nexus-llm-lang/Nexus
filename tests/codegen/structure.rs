use crate::harness::compile;
use nexus::compiler::codegen::{compile_program_to_wasm_with_dwarf, compile_program_to_wasm_with_metrics};
use wasmparser::Operator;

#[test]
fn codegen_exports_wasi_cli_run_wrapper() {
    let wasm = compile(
        r#"
let main = fn () -> unit do
    return ()
end
"#,
    );
    let mut has_run_export = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::ExportSection(reader) = payload.unwrap() {
            for export in reader.into_iter().flatten() {
                if export.name == "wasi:cli/run@0.2.6#run" {
                    has_run_export = true;
                }
            }
        }
    }
    assert!(has_run_export, "should export wasi:cli/run@0.2.6#run");
}

#[test]
fn compile_metrics_reports_all_pass_durations() {
    let src = r#"
let main = fn () -> unit do
    return ()
end
"#;
    let program = nexus::lang::parser::parser().parse(src).unwrap();
    let (wasm, metrics) = compile_program_to_wasm_with_metrics(&program).unwrap();
    assert!(!wasm.is_empty());
    assert!(!metrics.hir_build.is_zero());
    assert!(!metrics.lir_lower.is_zero());
    assert!(!metrics.codegen.is_zero());
}

#[test]
fn codegen_conc_exports_tasks_and_imports_runtime() {
    let src = r#"
let main = fn () -> unit do
    let x = 1
    conc do
        task t1 do
            let a = x + 1
            return ()
        end
        task t2 do
            let b = x + 2
            return ()
        end
    end
    return ()
end
"#;
    let wasm = compile(src);
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_spawn_import = false;
    let mut has_join_import = false;
    let mut exports = std::collections::HashSet::new();

    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            wasmparser::Payload::ImportSection(reader) => {
                for import in reader.into_iter().flatten() {
                    match import.name {
                        "__nx_conc_spawn" => has_spawn_import = true,
                        "__nx_conc_join" => has_join_import = true,
                        _ => {}
                    }
                }
            }
            wasmparser::Payload::ExportSection(reader) => {
                for export in reader.into_iter().flatten() {
                    exports.insert(export.name.to_string());
                }
            }
            _ => {}
        }
    }

    assert!(has_spawn_import, "should import __nx_conc_spawn");
    assert!(has_join_import, "should import __nx_conc_join");
    assert!(exports.contains("__conc_t1"), "should export __conc_t1");
    assert!(exports.contains("__conc_t2"), "should export __conc_t2");
}

#[test]
fn codegen_conc_block_compiles_task_functions() {
    let src = r#"
let main = fn () -> unit do
    let x = 1
    conc do
        task t1 do
            let a = x + 1
            return ()
        end
        task t2 do
            let b = x + 2
            return ()
        end
    end
    return ()
end
"#;
    let wasm = compile(src);
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut func_count = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(_) = payload.unwrap() {
            func_count += 1;
        }
    }
    assert_eq!(
        func_count, 4,
        "expected 4 code entries (main + 2 tasks + wasi:cli/run wrapper), got {}",
        func_count
    );
}

// ---- Tail call instruction tests ----

#[test]
fn codegen_tail_call_emits_return_call_instruction() {
    // Self-tail-recursive calls should be optimized to loop+br (not return_call)
    let wasm = compile(
        r#"
let sum_tail = fn (n: i64, acc: i64) -> i64 do
    if n <= 0 then return acc end
    return sum_tail(n: n - 1, acc: acc + n)
end

let main = fn () -> unit do
    let _ = sum_tail(n: 10, acc: 0)
    return ()
end
"#,
    );
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_loop = false;
    let mut has_br = false;
    let mut has_return_call = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let reader = body.get_operators_reader().unwrap();
            for op in reader {
                match op.unwrap() {
                    Operator::Loop { .. } => has_loop = true,
                    Operator::Br { .. } => has_br = true,
                    Operator::ReturnCall { .. } => has_return_call = true,
                    _ => {}
                }
            }
        }
    }
    assert!(
        has_loop && has_br,
        "self-tail-recursive call should emit loop + br (TCO-to-loop)"
    );
    assert!(
        !has_return_call,
        "self-tail-recursive call should NOT emit return_call (uses loop instead)"
    );
}

#[test]
fn codegen_tail_call_in_if_branch_emits_loop_br() {
    // Self-tail-call in if-else branch should be optimized to loop+br
    let wasm = compile(
        r#"
let count_down = fn (n: i64) -> i64 do
    if n <= 0 then
        return 0
    else
        return count_down(n: n - 1)
    end
end

let main = fn () -> unit do
    let _ = count_down(n: 50)
    return ()
end
"#,
    );
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_loop = false;
    let mut has_return_call = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let reader = body.get_operators_reader().unwrap();
            for op in reader {
                match op.unwrap() {
                    Operator::Loop { .. } => has_loop = true,
                    Operator::ReturnCall { .. } => has_return_call = true,
                    _ => {}
                }
            }
        }
    }
    assert!(
        has_loop,
        "self-tail-call in if-else branch should emit loop+br (TCO-to-loop)"
    );
}

#[test]
fn codegen_non_self_tail_call_emits_return_call() {
    // Non-self tail calls (mutual recursion) should still use return_call
    let wasm = compile(
        r#"
let is_even = fn (n: i64) -> bool do
    if n == 0 then return true end
    return is_odd(n: n - 1)
end

let is_odd = fn (n: i64) -> bool do
    if n == 0 then return false end
    return is_even(n: n - 1)
end

let main = fn () -> unit do
    let _ = is_even(n: 10)
    return ()
end
"#,
    );
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_return_call = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let reader = body.get_operators_reader().unwrap();
            for op in reader {
                if matches!(op.unwrap(), Operator::ReturnCall { .. }) {
                    has_return_call = true;
                }
            }
        }
    }
    assert!(
        has_return_call,
        "non-self tail call (mutual recursion) should still use return_call"
    );
}

#[test]
fn codegen_non_tail_call_does_not_emit_return_call() {
    let wasm = compile(
        r#"
let add_one = fn (n: i64) -> i64 do
    return n + 1
end

let main = fn () -> unit do
    let x = add_one(n: 41)
    if x != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );

    let mut has_return_call = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let reader = body.get_operators_reader().unwrap();
            for op in reader {
                if matches!(op.unwrap(), Operator::ReturnCall { .. }) {
                    has_return_call = true;
                }
            }
        }
    }
    assert!(
        !has_return_call,
        "non-tail call should not emit return_call"
    );
}

// ---- match arm tail call TCO ----

#[test]
fn codegen_match_arm_tail_call_emits_tco() {
    // Self-tail-call inside match arms should be optimized (loop+br for self-recursion)
    let wasm = compile(
        r#"
let sum_list = fn (xs: [i64], acc: i64) -> i64 do
    match xs do
        case Nil -> return acc
        case h :: t -> return sum_list(xs: t, acc: acc + h)
    end
end

let main = fn () -> unit do
    let _ = sum_list(xs: [1, 2, 3], acc: 0)
    return ()
end
"#,
    );
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_loop = false;
    let mut has_br = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let reader = body.get_operators_reader().unwrap();
            for op in reader {
                match op.unwrap() {
                    Operator::Loop { .. } => has_loop = true,
                    Operator::Br { .. } => has_br = true,
                    _ => {}
                }
            }
        }
    }
    assert!(
        has_loop && has_br,
        "self-tail-recursive call in match arm should emit loop + br (TCO-to-loop)"
    );
}

#[test]
fn codegen_match_arm_mutual_tail_call_emits_return_call() {
    // Non-self tail call in match arm should emit return_call
    let wasm = compile(
        r#"
let process = fn (n: i64) -> i64 do
    return helper(n: n)
end

let helper = fn (n: i64) -> i64 do
    match n do
        case 0 -> return 0
        case _ -> return process(n: n - 1)
    end
end

let main = fn () -> unit do
    let _ = helper(n: 5)
    return ()
end
"#,
    );
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    let mut has_return_call = false;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
            let reader = body.get_operators_reader().unwrap();
            for op in reader {
                if matches!(op.unwrap(), Operator::ReturnCall { .. }) {
                    has_return_call = true;
                }
            }
        }
    }
    assert!(
        has_return_call,
        "non-self tail call in match arm should emit return_call"
    );
}

// ---- main(args) desugaring ----

#[test]
fn codegen_main_with_args_desugars_to_zero_param_wasm() {
    let wasm = compile(
        r#"
let main = fn (args: [string]) -> unit do
    return ()
end
"#,
    );
    wasmparser::Validator::new()
        .validate_all(&wasm)
        .expect("WASM should be valid");

    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::ExportSection(reader) = payload.unwrap() {
            for export in reader.into_iter().flatten() {
                if export.name == "main" {
                    assert_eq!(
                        export.kind,
                        wasmparser::ExternalKind::Func,
                        "main should be a function export"
                    );
                }
            }
        }
    }
}

#[test]
fn codegen_main_with_args_includes_proc_capability() {
    let src = r#"
let main = fn (args: [string]) -> unit do
    return ()
end
"#;
    let program = nexus::lang::parser::parser().parse(src).unwrap();
    let (wasm, _) = compile_program_to_wasm_with_metrics(&program).unwrap();

    let caps = nexus::runtime::parse_nexus_capabilities(&wasm);
    assert!(
        caps.iter().any(|c| c == "Proc"),
        "main(args) should implicitly require PermProc, got: {:?}",
        caps
    );
}

// ---- stdlib WASM module validation ----

#[test]
fn stdlib_wasm_modules_are_wasi_only_or_self_contained() {
    use std::collections::BTreeSet;
    use std::path::Path;
    use wasmparser::Payload;

    fn imported_modules(path: &Path) -> BTreeSet<String> {
        let wasm = std::fs::read(path).expect("wasm file should be readable");
        let mut out = BTreeSet::new();
        for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
            let payload = payload.expect("wasm payload should parse");
            if let Payload::ImportSection(section) = payload {
                for import in section {
                    let import = import.expect("wasm import should parse");
                    out.insert(import.module.to_string());
                }
            }
        }
        out
    }

    let stdlib_dir = Path::new("nxlib/stdlib");
    let entries = std::fs::read_dir(stdlib_dir).expect("nxlib/stdlib should exist");

    let mut checked = 0usize;
    for entry in entries {
        let entry = entry.expect("dir entry should be readable");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        checked += 1;

        let modules = imported_modules(&path);
        assert!(
            !modules.contains("nexus_host"),
            "unexpected nexus_host import in {}",
            path.display()
        );
    }

    assert!(checked > 0, "at least one stdlib wasm should be checked");
}

/// Regression test for nexus-7nm: duplicate external declarations across modules
/// that point to the same WASM function should produce a single WASM import.
#[test]
fn codegen_deduplicates_externals_by_wasm_identity() {
    use nexus::compiler::codegen::compile_lir_to_wasm;
    use nexus::intern::Symbol;
    use nexus::ir::lir::{
        LirAtom, LirExpr, LirExternal, LirFunction, LirParam, LirProgram, LirStmt,
    };
    use nexus::types::Type;

    let wasm_mod = Symbol::from("stdlib/stdlib.wasm");
    let wasm_name = Symbol::from("__nx_string_to_i64");
    let s_label = Symbol::from("s");

    // Two externals with different Nexus names but same (wasm_module, wasm_name)
    let ext_a = LirExternal {
        name: Symbol::from("mod_a.to_i64"),
        wasm_module: wasm_mod,
        wasm_name,
        params: vec![LirParam {
            label: s_label,
            name: s_label,
            typ: Type::String,
        }],
        ret_type: Type::I64,
        throws: Type::Unit,
    };
    let ext_b = LirExternal {
        name: Symbol::from("mod_b.to_i64"),
        wasm_module: wasm_mod,
        wasm_name,
        params: vec![LirParam {
            label: s_label,
            name: s_label,
            typ: Type::String,
        }],
        ret_type: Type::I64,
        throws: Type::Unit,
    };

    let main_fn = LirFunction {
        name: Symbol::from("main"),
        params: vec![],
        ret_type: Type::Unit,
        requires: Type::Unit,
        throws: Type::Unit,
        body: vec![
            LirStmt::Let {
                name: Symbol::from("x"),
                typ: Type::I64,
                expr: LirExpr::Call {
                    func: Symbol::from("mod_a.to_i64"),
                    args: vec![(s_label, LirAtom::String("42".into()))],
                    typ: Type::I64,
                },
            },
            LirStmt::Let {
                name: Symbol::from("y"),
                typ: Type::I64,
                expr: LirExpr::Call {
                    func: Symbol::from("mod_b.to_i64"),
                    args: vec![(s_label, LirAtom::String("99".into()))],
                    typ: Type::I64,
                },
            },
        ],
        ret: LirAtom::Unit,
        span: 0..0,
        source_file: None,
        source_line: None,
    };

    let program = LirProgram {
        functions: vec![main_fn],
        externals: vec![ext_a, ext_b],
    };

    let (wasm, _) = compile_lir_to_wasm(&program).expect("should compile without E2010");

    // Count WASM imports for __nx_string_to_i64
    let mut import_count = 0;
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let wasmparser::Payload::ImportSection(reader) = payload.unwrap() {
            for import in reader.into_iter().flatten() {
                if import.name == "__nx_string_to_i64" {
                    import_count += 1;
                }
            }
        }
    }
    assert_eq!(
        import_count, 1,
        "duplicate externals should produce exactly one WASM import"
    );
}

#[test]
fn codegen_dwarf_sections_emitted() {
    let src = r#"
let add = fn (a: i64, b: i64) -> i64 do
  return a + b
end

let main = fn () -> unit do
  let r = add(a: 1, b: 2)
  return ()
end
"#;
    let mut program = nexus::lang::parser::parser().parse(src).unwrap();
    program.source_file = Some("test.nx".to_string());
    program.source_text = Some(src.to_string());
    let wasm = compile_program_to_wasm_with_dwarf(&program).unwrap();

    let mut sections = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        if let Ok(wasmparser::Payload::CustomSection(reader)) = payload {
            sections.push(reader.name().to_string());
        }
    }
    assert!(
        sections.contains(&".debug_abbrev".to_string()),
        "expected .debug_abbrev section, got: {:?}",
        sections
    );
    assert!(
        sections.contains(&".debug_info".to_string()),
        "expected .debug_info section, got: {:?}",
        sections
    );
    assert!(
        sections.contains(&".debug_line".to_string()),
        "expected .debug_line section, got: {:?}",
        sections
    );

    // Write WASM to temp file for manual inspection with wasm-tools
    if std::env::var("NEXUS_DUMP_DWARF").is_ok() {
        std::fs::write("/tmp/nexus_dwarf_demo.wasm", &wasm).unwrap();
        eprintln!("Wrote /tmp/nexus_dwarf_demo.wasm ({} bytes)", wasm.len());
    }
}

/// Helper: collect all import (module, name) pairs from a WASM binary.
fn wasm_imports(wasm: &[u8]) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let Ok(wasmparser::Payload::ImportSection(reader)) = payload {
            for import in reader.into_iter().flatten() {
                imports.push((import.module.to_string(), import.name.to_string()));
            }
        }
    }
    imports
}

/// Try/catch WITHOUT backtrace() should NOT import __nx_capture_backtrace.
#[test]
fn notrace_elides_capture_backtrace_import() {
    let wasm = compile(
        r#"
exception Boom(i64)

let main = fn () -> unit throws { Exn } do
    try
        raise Boom(42)
    catch e ->
        return ()
    end
    return ()
end
"#,
    );
    let imports = wasm_imports(&wasm);
    assert!(
        !imports.iter().any(|(m, n)| m == "nexus:runtime/backtrace" && n == "__nx_capture_backtrace"),
        "notrace: should NOT import __nx_capture_backtrace when backtrace() is unused, got: {:?}",
        imports
    );
}

/// Try/catch WITH backtrace() SHOULD import __nx_capture_backtrace.
#[test]
fn backtrace_usage_keeps_capture_import() {
    let wasm = compile(
        r#"
import { backtrace } from "stdlib/exn.nx"

exception Boom(i64)

let main = fn () -> unit throws { Exn } do
    try
        raise Boom(42)
    catch e ->
        let frames = backtrace(exn: e)
        return ()
    end
    return ()
end
"#,
    );
    let imports = wasm_imports(&wasm);
    assert!(
        imports.iter().any(|(m, n)| m == "nexus:runtime/backtrace" && n == "__nx_capture_backtrace"),
        "should import __nx_capture_backtrace when backtrace() is used, got: {:?}",
        imports
    );
}
