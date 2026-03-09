use crate::common::wasm::{compile, exec, exec_with_stdlib};
use nexus::compiler::codegen::compile_program_to_wasm_with_metrics;
use std::fs;
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
    // Verify the wasi:cli/run export exists
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
fn codegen_module_alias_call_compiles() {
    exec(
        r#"
import as math from examples/math.nx

let main = fn () -> unit do
    let result = math.add(a: 19, b: 23)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
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
    assert!(!metrics.mir_lower.is_zero());
    assert!(!metrics.lir_lower.is_zero());
    assert!(!metrics.codegen.is_zero());
}

#[test]
fn codegen_fixture_fib_works_in_wasm() {
    let src = fs::read_to_string("examples/fib.nx").expect("fixture should exist");
    exec_with_stdlib(&src);
}

#[test]
fn codegen_fixture_di_port_compiles() {
    let src = fs::read_to_string("examples/di_port.nx").expect("fixture should exist");
    exec_with_stdlib(&src);
}

#[test]
fn codegen_fixture_module_test_compiles() {
    let src = fs::read_to_string("examples/module_test.nx").expect("fixture should exist");
    exec_with_stdlib(&src);
}

#[test]
fn codegen_fixture_network_access_compiles() {
    let src = fs::read_to_string("examples/network_access.nx").expect("fixture should exist");
    let wasm = compile(&src);
    assert!(!wasm.is_empty(), "compiled wasm should not be empty");
}

#[test]
fn codegen_print_works_via_external_stdio_module() {
    exec_with_stdlib(
        r#"
import external stdlib/stdlib.wasm
external __nx_print = "__nx_print" : (val: string) -> unit

let main = fn () -> unit do
    __nx_print(val: "hello wasm")
    return ()
end
"#,
    );
}

#[test]
fn codegen_print_after_from_i64_works_via_single_string_abi_module() {
    exec_with_stdlib(
        r#"
import external stdlib/stdlib.wasm
external __nx_print = "__nx_print" : (val: string) -> unit

let main = fn () -> unit do
    let s = from_i64(val: 42)
    __nx_print(val: s)
    return ()
end
"#,
    );
}

#[test]
fn codegen_handler_reachability_resolves_port_call() {
    exec_with_stdlib(
        r#"
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.print(val: "hello")
    end
    return ()
end
"#,
    );
}

/// Regression: bundle_core_wasm must resolve stdlib memory import via wasm-merge.
#[test]
fn bundle_core_wasm_resolves_stdlib_imports() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "hello")
  end
  return ()
end
"#;
    let wasm = compile(src);
    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should resolve stdlib imports");
    let merged_imports =
        nexus::compiler::bundler::module_import_names(&merged).expect("parse merged imports");
    assert!(
        !merged_imports.iter().any(|m| m.contains("stdlib")),
        "stdlib imports should be resolved after bundling, got: {:?}",
        merged_imports
    );
}

/// Regression: conc programs with stdlib imports must also bundle successfully.
#[test]
fn bundle_core_wasm_resolves_conc_plus_stdlib() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

let work = fn () -> i64 do
  return 42
end

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    conc do
      task t1 do
        let _ = work()
      end
    end
    Console.println(val: "done")
  end
  return ()
end
"#;
    let wasm = compile(src);
    let imports = nexus::compiler::bundler::module_import_names(&wasm).expect("parse imports");
    assert!(imports.contains("nexus:runtime/conc"));
    assert!(imports.contains("nxlib/stdlib/stdlib.wasm"));

    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should succeed for conc+stdlib programs");
    let merged_imports =
        nexus::compiler::bundler::module_import_names(&merged).expect("parse merged imports");
    assert!(
        !merged_imports.iter().any(|m| m.contains("stdlib")),
        "stdlib should be resolved, got: {:?}",
        merged_imports
    );
    assert!(
        merged_imports.contains("nexus:runtime/conc"),
        "nexus:runtime/conc should remain (host-provided), got: {:?}",
        merged_imports
    );
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

    // Count internal functions via wasmparser: main + 2 tasks + wasi_cli_run wrapper = 4
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

#[test]
fn codegen_conc_block_executes_tasks_in_parallel() {
    exec(
        r#"
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
"#,
    );
}

#[test]
fn codegen_conc_fs_writes_in_parallel() {
    let src = r#"
import as fs from stdlib/fs.nx

let main = fn () -> unit require { PermFs } do
    conc do
        task write_a do
            fs.write_string(path: "nexus_conc_test_a.txt", content: "hello")
            return ()
        end
        task write_b do
            fs.write_string(path: "nexus_conc_test_b.txt", content: "world")
            return ()
        end
    end
    return ()
end
"#;
    exec_with_stdlib(src);

    // Verify files were actually written by conc tasks
    let a = fs::read_to_string("nexus_conc_test_a.txt").expect("file a should exist");
    let b = fs::read_to_string("nexus_conc_test_b.txt").expect("file b should exist");
    assert_eq!(a, "hello");
    assert_eq!(b, "world");
    let _ = fs::remove_file("nexus_conc_test_a.txt");
    let _ = fs::remove_file("nexus_conc_test_b.txt");
}

#[test]
fn stdlib_wasm_modules_are_wasi_only_or_self_contained() {
    use std::collections::BTreeSet;
    use std::path::Path;
    use wasmparser::Payload;

    fn imported_modules(path: &Path) -> BTreeSet<String> {
        let wasm = fs::read(path).expect("wasm file should be readable");
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
    let entries = fs::read_dir(stdlib_dir).expect("nxlib/stdlib should exist");

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

#[test]
fn codegen_tail_recursive_function_executes_correctly() {
    exec(
        r#"
let sum_tail = fn (n: i64, acc: i64) -> i64 do
    if n <= 0 then return acc end
    return sum_tail(n: n - 1, acc: acc + n)
end

let main = fn () -> unit do
    let result = sum_tail(n: 100, acc: 0)
    if result != 5050 then raise RuntimeError(val: "expected 5050") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_tail_call_emits_return_call_instruction() {
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
    assert!(has_return_call, "tail-recursive call should emit return_call instruction");
}

#[test]
fn codegen_tail_call_in_if_branch_emits_return_call() {
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
    assert!(has_return_call, "tail call in if-else branch should emit return_call");
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
    assert!(!has_return_call, "non-tail call should not emit return_call");
}

#[test]
fn codegen_deep_tail_recursion_does_not_overflow() {
    exec(
        r#"
let loop_n = fn (n: i64) -> i64 do
    if n <= 0 then return 0 end
    return loop_n(n: n - 1)
end

let main = fn () -> unit do
    let result = loop_n(n: 1000000)
    if result != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}
