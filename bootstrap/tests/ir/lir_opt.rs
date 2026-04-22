use std::collections::HashSet;

use nexus::compiler::passes::hir_build::build_hir;
use nexus::compiler::passes::lir_lower::lower_mir_to_lir;
use nexus::compiler::passes::lir_opt::optimize_lir;
use nexus::intern::Symbol;
use nexus::ir::lir::{LirAtom, LirExpr, LirFunction, LirProgram, LirStmt};
use nexus::lang::parser;

fn build_lir(src: &str) -> LirProgram {
    let program = parser::parser().parse(src).unwrap();
    let mir = build_hir(&program).unwrap();
    lower_mir_to_lir(&mir, &mir.enum_defs).unwrap()
}

fn build_optimized_lir(src: &str) -> LirProgram {
    let mut lir = build_lir(src);
    optimize_lir(&mut lir);
    lir
}

// ── Validation: every atom-var ref must have a Let definition or be a param ──

fn collect_atom_refs(atom: &LirAtom, refs: &mut HashSet<Symbol>) {
    if let LirAtom::Var { name, .. } = atom {
        refs.insert(*name);
    }
}

fn collect_opt_atom_refs(atom: &Option<LirAtom>, refs: &mut HashSet<Symbol>) {
    if let Some(a) = atom {
        collect_atom_refs(a, refs);
    }
}

fn collect_expr_refs(expr: &LirExpr, refs: &mut HashSet<Symbol>) {
    match expr {
        LirExpr::Atom(a) => collect_atom_refs(a, refs),
        LirExpr::Binary { lhs, rhs, .. } => {
            collect_atom_refs(lhs, refs);
            collect_atom_refs(rhs, refs);
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            for (_, a) in args {
                collect_atom_refs(a, refs);
            }
        }
        LirExpr::Constructor { args, .. } => {
            for a in args {
                collect_atom_refs(a, refs);
            }
        }
        LirExpr::Record { fields, .. } => {
            for (_, a) in fields {
                collect_atom_refs(a, refs);
            }
        }
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            collect_atom_refs(value, refs);
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => {
            collect_atom_refs(value, refs)
        }
        LirExpr::Closure { captures, .. } => {
            for (_, a) in captures {
                collect_atom_refs(a, refs);
            }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            collect_atom_refs(callee, refs);
            for (_, a) in args {
                collect_atom_refs(a, refs);
            }
        }
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::LazySpawn { thunk, .. } => collect_atom_refs(thunk, refs),
        LirExpr::LazyJoin { task_id, .. } => collect_atom_refs(task_id, refs),
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                collect_atom_refs(a, refs);
            }
        }
    }
}

fn collect_stmt_refs(stmts: &[LirStmt], refs: &mut HashSet<Symbol>) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { expr, .. } => collect_expr_refs(expr, refs),
            LirStmt::If {
                cond,
                then_body,
                else_body,
            } => {
                collect_atom_refs(cond, refs);
                collect_stmt_refs(then_body, refs);
                collect_stmt_refs(else_body, refs);
            }
            LirStmt::IfReturn {
                cond,
                then_body,
                then_ret,
                else_body,
                else_ret,
                ..
            } => {
                collect_atom_refs(cond, refs);
                collect_stmt_refs(then_body, refs);
                collect_opt_atom_refs(then_ret, refs);
                collect_stmt_refs(else_body, refs);
                collect_opt_atom_refs(else_ret, refs);
            }
            LirStmt::Loop {
                cond_stmts,
                cond,
                body,
            } => {
                collect_stmt_refs(cond_stmts, refs);
                collect_atom_refs(cond, refs);
                collect_stmt_refs(body, refs);
            }
            LirStmt::TryCatch {
                body,
                body_ret,
                catch_body,
                catch_ret,
                ..
            } => {
                collect_stmt_refs(body, refs);
                collect_opt_atom_refs(body_ret, refs);
                collect_stmt_refs(catch_body, refs);
                collect_opt_atom_refs(catch_ret, refs);
            }
            LirStmt::Switch {
                tag,
                cases,
                default_body,
                default_ret,
                ..
            } => {
                collect_atom_refs(tag, refs);
                for case in cases {
                    collect_stmt_refs(&case.body, refs);
                    collect_opt_atom_refs(&case.ret, refs);
                }
                collect_stmt_refs(default_body, refs);
                collect_opt_atom_refs(default_ret, refs);
            }
            LirStmt::FieldUpdate { target, value, .. } => {
                collect_atom_refs(target, refs);
                collect_atom_refs(value, refs);
            }
        }
    }
}

fn collect_stmt_defs(stmts: &[LirStmt], defs: &mut HashSet<Symbol>) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { name, .. } => {
                defs.insert(*name);
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_defs(then_body, defs);
                collect_stmt_defs(else_body, defs);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_defs(then_body, defs);
                collect_stmt_defs(else_body, defs);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                collect_stmt_defs(cond_stmts, defs);
                collect_stmt_defs(body, defs);
            }
            LirStmt::TryCatch {
                body,
                catch_param,
                catch_body,
                ..
            } => {
                collect_stmt_defs(body, defs);
                defs.insert(*catch_param);
                collect_stmt_defs(catch_body, defs);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    collect_stmt_defs(&case.body, defs);
                }
                collect_stmt_defs(default_body, defs);
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
}

/// Validate that every variable reference in a function has a corresponding definition.
fn validate_function_refs(func: &LirFunction) -> Vec<String> {
    let mut defs: HashSet<Symbol> = HashSet::new();
    // Params are definitions
    for p in &func.params {
        defs.insert(p.name);
    }
    collect_stmt_defs(&func.body, &mut defs);

    let mut refs: HashSet<Symbol> = HashSet::new();
    collect_stmt_refs(&func.body, &mut refs);
    collect_atom_refs(&func.ret, &mut refs);

    let mut missing: Vec<String> = refs.difference(&defs).map(|s| s.to_string()).collect();
    missing.sort();
    missing
}

fn validate_program(program: &LirProgram) -> Vec<(String, Vec<String>)> {
    let mut issues = Vec::new();
    for func in &program.functions {
        let missing = validate_function_refs(func);
        if !missing.is_empty() {
            issues.push((func.name.to_string(), missing));
        }
    }
    issues
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn opt_preserves_pattern_binding_refs_in_simple_match() {
    let src = r#"
type Entry = Entry(name: string, val: i64)

let lookup = fn (key: string, entries: [Entry]) -> i64 do
  match entries do
    case [] -> return 0 - 1
    case Entry(name: n, val: v) :: rest ->
      if n == key then return v end
      return lookup(key: key, entries: rest)
  end
end

let main = fn () -> unit do
  let r = lookup(key: "x", entries: [Entry(name: "x", val: 42)])
end
"#;
    let lir = build_optimized_lir(src);
    let issues = validate_program(&lir);
    assert!(
        issues.is_empty(),
        "Dangling references after optimization: {:?}",
        issues
    );
}

#[test]
fn opt_preserves_pattern_binding_refs_with_multi_variant() {
    // Mirrors opt_lookup's exact pattern: SubstEntry(name: n, atom: a) :: rest
    let src = r#"
type LirAtom =
  AtomVar(name: string, typ: i64)
  | AtomInt(val: i64)
  | AtomBool(val: bool)
  | AtomString(val: string)
  | AtomUnit

type SubstEntry = SubstEntry(name: string, atom: LirAtom)

let opt_lookup = fn (name: string, entries: [SubstEntry]) -> Option<LirAtom> do
  match entries do
    case [] -> return None
    case SubstEntry(name: n, atom: a) :: rest ->
      if n == name then return Some(val: a) end
      return opt_lookup(name: name, entries: rest)
  end
end

let main = fn () -> unit do
  let entries = [SubstEntry(name: "x", atom: AtomInt(val: 42))]
  match opt_lookup(name: "x", entries: entries) do
    case Some(val: _) -> ()
    case None -> ()
  end
end
"#;
    let lir = build_optimized_lir(src);
    let issues = validate_program(&lir);
    assert!(
        issues.is_empty(),
        "Dangling references after optimization: {:?}",
        issues
    );
}

#[test]
fn opt_preserves_refs_in_nested_if_return() {
    let src = r#"
type Pair = Pair(first: i64, second: i64)

let sum_pairs = fn (pairs: [Pair]) -> i64 do
  match pairs do
    case [] -> return 0
    case Pair(first: a, second: b) :: rest ->
      return a + b + sum_pairs(pairs: rest)
  end
end

let main = fn () -> unit do
  let r = sum_pairs(pairs: [Pair(first: 1, second: 2)])
end
"#;
    let lir = build_optimized_lir(src);
    let issues = validate_program(&lir);
    assert!(
        issues.is_empty(),
        "Dangling references after optimization: {:?}",
        issues
    );
}

#[test]
fn opt_validation_works_before_optimization() {
    // Sanity check: unoptimized LIR should also be valid
    let src = r#"
type Entry = Entry(name: string, val: i64)

let lookup = fn (key: string, entries: [Entry]) -> i64 do
  match entries do
    case [] -> return 0 - 1
    case Entry(name: n, val: v) :: rest ->
      if n == key then return v end
      return lookup(key: key, entries: rest)
  end
end

let main = fn () -> unit do
  let r = lookup(key: "x", entries: [Entry(name: "x", val: 42)])
end
"#;
    let lir = build_lir(src);
    let issues = validate_program(&lir);
    assert!(
        issues.is_empty(),
        "Dangling references in unoptimized LIR: {:?}",
        issues
    );
}

/// Test that full compilation (parse → LIR → optimize → codegen) succeeds.
/// This catches the case where optimization produces LIR that codegen can't handle.
#[test]
fn opt_full_compile_succeeds_for_pattern_match() {
    let src = r#"
type SubstEntry = SubstEntry(name: string, atom: i64)
type FoldResult = FoldResult(stmts: [i64], subst: [SubstEntry])

let opt_lookup = fn (name: string, entries: [SubstEntry]) -> Option<i64> do
  match entries do
    case [] -> return None
    case SubstEntry(name: n, atom: a) :: rest ->
      if n == name then return Some(val: a) end
      return opt_lookup(name: name, entries: rest)
  end
end

let main = fn () -> unit do
  let entries = [SubstEntry(name: "x", atom: 42)]
  match opt_lookup(name: "x", entries: entries) do
    case Some(val: _) -> ()
    case None -> ()
  end
end
"#;
    // This should not panic — if it does, the optimizer broke codegen
    let _wasm = crate::harness::compile::compile(src);
}

/// Full compile with the exact type structure from lir_opt.nx
#[test]
fn opt_full_compile_multi_variant_atom() {
    let src = r#"
type LirAtom =
  LirAtomVar(name: string, typ: i64)
  | LirAtomInt(val: i64)
  | LirAtomBool(val: bool)
  | LirAtomChar(val: i64)
  | LirAtomString(val: string)
  | LirAtomUnit

type SubstEntry = SubstEntry(name: string, atom: LirAtom)

let opt_lookup = fn (name: string, entries: [SubstEntry]) -> Option<LirAtom> do
  match entries do
    case [] -> return None
    case SubstEntry(name: n, atom: a) :: rest ->
      if n == name then return Some(val: a) end
      return opt_lookup(name: name, entries: rest)
  end
end

let opt_has_name = fn (name: string, names: [string]) -> bool do
  match names do
    case [] -> return false
    case nm :: rest ->
      if nm == name then return true end
      return opt_has_name(name: name, names: rest)
  end
end

let main = fn () -> unit do
  let entries = [SubstEntry(name: "x", atom: LirAtomInt(val: 42))]
  match opt_lookup(name: "x", entries: entries) do
    case Some(val: _) -> ()
    case None -> ()
  end
end
"#;
    let _wasm = crate::harness::compile::compile(src);
}

/// The actual program that triggers the bug (compiled as a single file for testing)
#[test]
fn opt_preserves_refs_in_optimizer_like_code() {
    let src = r#"
type LirAtom =
  LirAtomVar(name: string, typ: i64)
  | LirAtomInt(val: i64)
  | LirAtomBool(val: bool)
  | LirAtomChar(val: i64)
  | LirAtomString(val: string)
  | LirAtomUnit

type SubstEntry = SubstEntry(name: string, atom: LirAtom)
type FoldResult = FoldResult(stmts: [i64], subst: [SubstEntry])

let opt_lookup = fn (name: string, entries: [SubstEntry]) -> Option<LirAtom> do
  match entries do
    case [] -> return None
    case SubstEntry(name: n, atom: a) :: rest ->
      if n == name then return Some(val: a) end
      return opt_lookup(name: name, entries: rest)
  end
end

let opt_has_name = fn (name: string, names: [string]) -> bool do
  match names do
    case [] -> return false
    case nm :: rest ->
      if nm == name then return true end
      return opt_has_name(name: name, names: rest)
  end
end

let main = fn () -> unit do
  let entries = [SubstEntry(name: "x", atom: LirAtomInt(val: 42))]
  match opt_lookup(name: "x", entries: entries) do
    case Some(val: _) -> ()
    case None -> ()
  end
  let r = opt_has_name(name: "x", names: ["a", "b", "x"])
end
"#;
    let lir = build_optimized_lir(src);
    let issues = validate_program(&lir);
    assert!(
        issues.is_empty(),
        "Dangling references after optimization: {:?}",
        issues
    );
}
