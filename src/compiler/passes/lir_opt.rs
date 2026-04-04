//! LIR optimization passes.
//!
//! Runs between LIR lowering and WASM codegen:
//! 1. Switch recognition: convert IfReturn chains (from match lowering) to Switch nodes
//! 2. Known-call devirtualization: replace CallIndirect(FuncRef(f), args) with Call(f, args)
//! 3. Constant folding: evaluate Binary(Lit, op, Lit) at compile time
//! 4. Copy propagation: replace Let x = Atom(y) by substituting y for x everywhere
//! 5. Dead let elimination (fixpoint): remove Let with unused result and no side effects
//! 6. Unreachable code stripping: remove stmts after divergent stmts

use std::collections::HashMap;

use crate::compiler::type_tag::constructor_tag;
use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirFunction, LirParam, LirProgram, LirStmt, SwitchCase};
use crate::types::{BinaryOp, Span, Type};

/// Run all LIR optimization passes on the program (mutates in place).
/// When `parallel_lazy` is true, consecutive zero-arg forces are converted to
/// LazySpawn/LazyJoin pairs for parallel evaluation. Set to false for targets
/// that don't provide the `nexus:runtime/lazy` host module (e.g. component WASM).
pub fn optimize_lir(program: &mut LirProgram) {
    optimize_lir_with_opts(program, true);
}

pub fn optimize_lir_with_opts(program: &mut LirProgram, parallel_lazy: bool) {
    // Phase 0: Program-level function inlining (before per-function passes)
    inline_small_functions(program);

    // Phase 0.5: Deforestation — fuse chained list operations (map∘map, etc.)
    //   to eliminate intermediate list allocations. Runs at program level because
    //   it may generate new synthetic functions for composition.
    fuse_list_operations(program);

    for func in &mut program.functions {
        optimize_function(func, parallel_lazy);
    }

    // Phase 7: Identical code folding (after all per-function optimizations)
    fold_identical_functions(program);
}

fn optimize_function(func: &mut LirFunction, parallel_lazy: bool) {
    // 1. Recognize IfReturn chains from match lowering → Switch nodes
    recognize_switches_in_stmts(&mut func.body);
    // 1.5. Known-call devirtualization: FuncRef(f) + CallIndirect → Call(f)
    devirtualize_known_calls(&mut func.body);
    // 1.6. LICM: hoist loop-invariant let bindings out of loops
    hoist_loop_invariants(&mut func.body);
    // 2. Constant folding: Binary(Lit, op, Lit) → Atom(Lit)
    constant_fold_stmts(&mut func.body);
    // 2.5. Constant branch elimination: if Bool(true/false), replace with live branch
    eliminate_constant_branches(&mut func.body);
    // 3. Copy propagation: Let x = Atom(y) → substitute y for x
    //    Skip variables bound multiple times (match-expr result temps) — not SSA-safe.
    let multi_bound = find_multiply_bound(&func.body);
    let mut subst = HashMap::new();
    copy_propagate_stmts(&mut func.body, &mut subst, &multi_bound);
    subst_atom(&mut func.ret, &subst);
    // 3.5. Scalar replacement of aggregates: eliminate Constructor allocations
    //      when the result is only used via ObjectTag/ObjectField (never escapes).
    scalar_replace_aggregates(&mut func.body, &func.ret);
    // 3.6. Linear reuse: replace Constructor with in-place FieldUpdate when
    //      the constructor args trace back to ObjectField extractions from a
    //      provably-dead source, saving the heap allocation.
    reuse_linear_constructors(&mut func.body);
    // 4. Dead let elimination — iterate to fixpoint because removing a dead Let
    //    may make its referenced variables dead too (cascading dead code).
    for _ in 0..8 {
        let mut uses = HashMap::new();
        count_uses_in_stmts(&func.body, &mut uses);
        count_uses_in_atom(&func.ret, &mut uses);
        let dead_count = count_dead_lets(&func.body, &uses);
        if dead_count == 0 {
            break;
        }
        eliminate_dead_lets(&mut func.body, &uses);
    }
    // 5. Unreachable code stripping (remove stmts after divergent stmts)
    strip_unreachable_stmts(&mut func.body);
    // 6. Lazy parallelization: convert consecutive zero-arg CallIndirect (force) into
    //    LazySpawn/LazyJoin pairs for parallel evaluation.
    //    Only enabled when the target provides nexus:runtime/lazy host functions.
    if parallel_lazy {
        parallelize_consecutive_forces(&mut func.body);
    }
}

/// Collect names that are bound by Let more than once (across all nested scopes).
/// These are not SSA and cannot be safely copy-propagated.
use std::collections::HashSet;

fn find_multiply_bound(stmts: &[LirStmt]) -> HashSet<Symbol> {
    let mut counts: HashMap<Symbol, u32> = HashMap::new();
    count_let_bindings(stmts, &mut counts);
    counts
        .into_iter()
        .filter(|(_, c)| *c > 1)
        .map(|(n, _)| n)
        .collect()
}

fn count_let_bindings(stmts: &[LirStmt], counts: &mut HashMap<Symbol, u32>) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { name, .. } => {
                *counts.entry(*name).or_default() += 1;
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                count_let_bindings(then_body, counts);
                count_let_bindings(else_body, counts);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                count_let_bindings(then_body, counts);
                count_let_bindings(else_body, counts);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                count_let_bindings(body, counts);
                count_let_bindings(catch_body, counts);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                count_let_bindings(cond_stmts, counts);
                count_let_bindings(body, counts);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    count_let_bindings(&case.body, counts);
                }
                count_let_bindings(default_body, counts);
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
}

// ─── Switch Recognition ──────────────────────────────────────────────────────

/// Scan stmts for IfReturn chains that represent tag-based match dispatch,
/// and convert them to `LirStmt::Switch` for potential br_table codegen.
fn recognize_switches_in_stmts(stmts: &mut Vec<LirStmt>) {
    // Phase 1: Collect definitions from Let stmts in this scope.
    let mut cond_defs: HashMap<Symbol, (Symbol, i64)> = HashMap::new();
    let mut tag_defs: HashMap<Symbol, LirAtom> = HashMap::new();

    for stmt in stmts.iter() {
        if let LirStmt::Let { name, expr, .. } = stmt {
            match expr {
                LirExpr::Binary {
                    op: BinaryOp::Eq,
                    lhs,
                    rhs,
                    ..
                } => {
                    if let (LirAtom::Var { name: tv, .. }, LirAtom::Int(v)) = (lhs, rhs) {
                        cond_defs.insert(*name, (*tv, *v));
                    } else if let (LirAtom::Int(v), LirAtom::Var { name: tv, .. }) = (lhs, rhs) {
                        cond_defs.insert(*name, (*tv, *v));
                    }
                }
                LirExpr::ObjectTag { value: target, .. } => {
                    tag_defs.insert(*name, target.clone());
                }
                _ => {}
            }
        }
    }

    // Phase 2: Convert eligible IfReturn chains to Switch.
    for i in 0..stmts.len() {
        if matches!(&stmts[i], LirStmt::IfReturn { .. }) {
            if can_convert_to_switch(&stmts[i], &cond_defs, &tag_defs) {
                let placeholder = LirStmt::If {
                    cond: LirAtom::Unit,
                    then_body: vec![],
                    else_body: vec![],
                };
                let original = std::mem::replace(&mut stmts[i], placeholder);
                stmts[i] = build_switch_from_chain(original, &cond_defs);
            }
        }
    }

    // Phase 3: Recurse into nested bodies.
    for stmt in stmts.iter_mut() {
        recurse_switch_recognition(stmt);
    }
}

fn recurse_switch_recognition(stmt: &mut LirStmt) {
    match stmt {
        LirStmt::If {
            then_body,
            else_body,
            ..
        } => {
            recognize_switches_in_stmts(then_body);
            recognize_switches_in_stmts(else_body);
        }
        LirStmt::IfReturn {
            then_body,
            else_body,
            ..
        } => {
            recognize_switches_in_stmts(then_body);
            recognize_switches_in_stmts(else_body);
        }
        LirStmt::TryCatch {
            body, catch_body, ..
        } => {
            recognize_switches_in_stmts(body);
            recognize_switches_in_stmts(catch_body);
        }
        LirStmt::Loop {
            cond_stmts, body, ..
        } => {
            recognize_switches_in_stmts(cond_stmts);
            recognize_switches_in_stmts(body);
        }
        LirStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for case in cases {
                recognize_switches_in_stmts(&mut case.body);
            }
            recognize_switches_in_stmts(default_body);
        }
        LirStmt::Let { .. } => {}
        LirStmt::FieldUpdate { .. } => {}
    }
}

/// Check if an IfReturn chain can be converted to a Switch.
///
/// Requirements:
/// - All non-final conditions are tag equality checks (Var defined as Eq(tag_var, Int))
/// - All tag vars are ObjectTag extractions on the same target
/// - The chain has at least 3 links (worthwhile for structured dispatch)
/// - Each else_body has exactly 0 or 1 elements (clean chain from match lowering)
/// - No else_ret values set (always None from match lowering)
fn can_convert_to_switch(
    stmt: &LirStmt,
    cond_defs: &HashMap<Symbol, (Symbol, i64)>,
    tag_defs: &HashMap<Symbol, LirAtom>,
) -> bool {
    let mut current = stmt;
    let mut target: Option<&LirAtom> = None;
    let mut case_count = 0u32;

    loop {
        match current {
            LirStmt::IfReturn {
                cond,
                then_ret,
                else_body,
                else_ret,
                ..
            } => {
                if else_ret.is_some() {
                    return false;
                }
                // Require all cases to return (binary search dispatch needs it)
                if then_ret.is_none() {
                    return false;
                }
                match cond {
                    LirAtom::Var { name, .. } => {
                        if let Some(&(tag_var, _)) = cond_defs.get(name) {
                            if let Some(tgt) = tag_defs.get(&tag_var) {
                                match target {
                                    Some(expected) if tgt != expected => return false,
                                    None => target = Some(tgt),
                                    _ => {}
                                }
                                case_count += 1;
                                match else_body.len() {
                                    1 => {
                                        current = &else_body[0];
                                        continue;
                                    }
                                    0 => break,
                                    _ => return false,
                                }
                            } else {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    LirAtom::Bool(true) => {
                        case_count += 1;
                        break;
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
    }

    case_count >= 3
}

/// Convert a verified IfReturn chain into a Switch stmt.
/// Caller must ensure `can_convert_to_switch` returned true.
fn build_switch_from_chain(stmt: LirStmt, cond_defs: &HashMap<Symbol, (Symbol, i64)>) -> LirStmt {
    let mut cases = Vec::new();
    let mut default_body = Vec::new();
    let mut default_ret = None;
    let mut tag_atom: Option<LirAtom> = None;
    let mut last_ret_type: Option<Type> = None;

    let mut current = stmt;
    loop {
        match current {
            LirStmt::IfReturn {
                cond,
                then_body,
                then_ret,
                mut else_body,
                ret_type: rt,
                ..
            } => {
                last_ret_type.get_or_insert(rt);
                match cond {
                    LirAtom::Var { name, .. } => {
                        let (tag_var, tag_value) = cond_defs[&name];
                        if tag_atom.is_none() {
                            tag_atom = Some(LirAtom::Var {
                                name: tag_var,
                                typ: Type::I64,
                            });
                        }
                        cases.push(SwitchCase {
                            tag_value,
                            body: then_body,
                            ret: then_ret,
                        });
                        if else_body.len() == 1 {
                            current = else_body.pop().unwrap();
                        } else {
                            break;
                        }
                    }
                    LirAtom::Bool(true) => {
                        default_body = then_body;
                        default_ret = then_ret;
                        break;
                    }
                    _ => unreachable!("verified by can_convert_to_switch"),
                }
            }
            _ => unreachable!("verified by can_convert_to_switch"),
        }
    }

    LirStmt::Switch {
        tag: tag_atom.expect("at least one tag case"),
        cases,
        default_body,
        default_ret,
        ret_type: last_ret_type.unwrap_or(Type::Unit),
    }
}

// ─── Constant Folding ────────────────────────────────────────────────────────

fn constant_fold_stmts(stmts: &mut [LirStmt]) {
    for stmt in stmts.iter_mut() {
        constant_fold_stmt(stmt);
    }
}

fn constant_fold_stmt(stmt: &mut LirStmt) {
    match stmt {
        LirStmt::Let { expr, .. } => {
            if let Some(folded) = try_fold_expr(expr) {
                *expr = LirExpr::Atom(folded);
            }
        }
        LirStmt::If {
            then_body,
            else_body,
            ..
        } => {
            constant_fold_stmts(then_body);
            constant_fold_stmts(else_body);
        }
        LirStmt::IfReturn {
            then_body,
            else_body,
            ..
        } => {
            constant_fold_stmts(then_body);
            constant_fold_stmts(else_body);
        }
        LirStmt::TryCatch {
            body, catch_body, ..
        } => {
            constant_fold_stmts(body);
            constant_fold_stmts(catch_body);
        }
        LirStmt::Loop {
            cond_stmts, body, ..
        } => {
            constant_fold_stmts(cond_stmts);
            constant_fold_stmts(body);
        }
        LirStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for case in cases {
                constant_fold_stmts(&mut case.body);
            }
            constant_fold_stmts(default_body);
        }
        LirStmt::FieldUpdate { .. } => {}
    }
}

fn try_fold_expr(expr: &LirExpr) -> Option<LirAtom> {
    if let LirExpr::Binary { op, lhs, rhs, .. } = expr {
        if let Some(folded) = fold_binary(*op, lhs, rhs) {
            return Some(folded);
        }
        return simplify_binary(*op, lhs, rhs);
    }
    None
}

fn fold_binary(op: BinaryOp, lhs: &LirAtom, rhs: &LirAtom) -> Option<LirAtom> {
    match (lhs, rhs) {
        (LirAtom::Int(a), LirAtom::Int(b)) => fold_int(op, *a, *b),
        (LirAtom::Float(a), LirAtom::Float(b)) => fold_float(op, *a, *b),
        (LirAtom::Bool(a), LirAtom::Bool(b)) => fold_bool(op, *a, *b),
        _ => None,
    }
}

/// Identity and absorbing element simplification (one operand literal).
fn simplify_binary(op: BinaryOp, lhs: &LirAtom, rhs: &LirAtom) -> Option<LirAtom> {
    match op {
        BinaryOp::Add => match (lhs, rhs) {
            (LirAtom::Int(0), _) => Some(rhs.clone()),
            (_, LirAtom::Int(0)) => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::Sub => match rhs {
            LirAtom::Int(0) => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::Mul => match (lhs, rhs) {
            (LirAtom::Int(0), _) | (_, LirAtom::Int(0)) => Some(LirAtom::Int(0)),
            (LirAtom::Int(1), _) => Some(rhs.clone()),
            (_, LirAtom::Int(1)) => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::Div => match rhs {
            LirAtom::Int(1) => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::And => match (lhs, rhs) {
            (LirAtom::Bool(false), _) | (_, LirAtom::Bool(false)) => Some(LirAtom::Bool(false)),
            (LirAtom::Bool(true), _) => Some(rhs.clone()),
            (_, LirAtom::Bool(true)) => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::Or => match (lhs, rhs) {
            (LirAtom::Bool(true), _) | (_, LirAtom::Bool(true)) => Some(LirAtom::Bool(true)),
            (LirAtom::Bool(false), _) => Some(rhs.clone()),
            (_, LirAtom::Bool(false)) => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::FAdd => match (lhs, rhs) {
            (LirAtom::Float(v), _) if *v == 0.0 => Some(rhs.clone()),
            (_, LirAtom::Float(v)) if *v == 0.0 => Some(lhs.clone()),
            _ => None,
        },
        BinaryOp::FMul => match (lhs, rhs) {
            (LirAtom::Float(v), _) if *v == 1.0 => Some(rhs.clone()),
            (_, LirAtom::Float(v)) if *v == 1.0 => Some(lhs.clone()),
            (LirAtom::Float(v), _) if *v == 0.0 => Some(LirAtom::Float(0.0)),
            (_, LirAtom::Float(v)) if *v == 0.0 => Some(LirAtom::Float(0.0)),
            _ => None,
        },
        _ => None,
    }
}

fn fold_int(op: BinaryOp, a: i64, b: i64) -> Option<LirAtom> {
    Some(match op {
        BinaryOp::Add => LirAtom::Int(a.wrapping_add(b)),
        BinaryOp::Sub => LirAtom::Int(a.wrapping_sub(b)),
        BinaryOp::Mul => LirAtom::Int(a.wrapping_mul(b)),
        BinaryOp::Div if b != 0 => LirAtom::Int(a.wrapping_div(b)),
        BinaryOp::Mod if b != 0 => LirAtom::Int(a.wrapping_rem(b)),
        BinaryOp::BitAnd => LirAtom::Int(a & b),
        BinaryOp::BitOr => LirAtom::Int(a | b),
        BinaryOp::BitXor => LirAtom::Int(a ^ b),
        BinaryOp::Shl => LirAtom::Int(a.wrapping_shl(b as u32)),
        BinaryOp::Shr => LirAtom::Int(a.wrapping_shr(b as u32)),
        BinaryOp::Eq => LirAtom::Bool(a == b),
        BinaryOp::Ne => LirAtom::Bool(a != b),
        BinaryOp::Lt => LirAtom::Bool(a < b),
        BinaryOp::Le => LirAtom::Bool(a <= b),
        BinaryOp::Gt => LirAtom::Bool(a > b),
        BinaryOp::Ge => LirAtom::Bool(a >= b),
        _ => return None,
    })
}

fn fold_float(op: BinaryOp, a: f64, b: f64) -> Option<LirAtom> {
    Some(match op {
        BinaryOp::FAdd => LirAtom::Float(a + b),
        BinaryOp::FSub => LirAtom::Float(a - b),
        BinaryOp::FMul => LirAtom::Float(a * b),
        BinaryOp::FDiv if b != 0.0 => LirAtom::Float(a / b),
        BinaryOp::FEq => LirAtom::Bool(a == b),
        BinaryOp::FNe => LirAtom::Bool(a != b),
        BinaryOp::FLt => LirAtom::Bool(a < b),
        BinaryOp::FLe => LirAtom::Bool(a <= b),
        BinaryOp::FGt => LirAtom::Bool(a > b),
        BinaryOp::FGe => LirAtom::Bool(a >= b),
        _ => return None,
    })
}

fn fold_bool(op: BinaryOp, a: bool, b: bool) -> Option<LirAtom> {
    Some(match op {
        BinaryOp::And => LirAtom::Bool(a && b),
        BinaryOp::Or => LirAtom::Bool(a || b),
        BinaryOp::Eq => LirAtom::Bool(a == b),
        BinaryOp::Ne => LirAtom::Bool(a != b),
        _ => return None,
    })
}

// ─── Constant Branch Elimination ─────────────────────────────────────────────

/// Replace If/IfReturn with literal conditions by the live branch body.
/// After constant folding, conditions like `Bool(true)` or `Bool(false)` may appear.
fn eliminate_constant_branches(stmts: &mut Vec<LirStmt>) {
    let mut i = 0;
    while i < stmts.len() {
        // First recurse into nested bodies
        match &mut stmts[i] {
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                eliminate_constant_branches(then_body);
                eliminate_constant_branches(else_body);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                eliminate_constant_branches(then_body);
                eliminate_constant_branches(else_body);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                eliminate_constant_branches(body);
                eliminate_constant_branches(catch_body);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                eliminate_constant_branches(cond_stmts);
                eliminate_constant_branches(body);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    eliminate_constant_branches(&mut case.body);
                }
                eliminate_constant_branches(default_body);
            }
            LirStmt::Let { .. } => {}
            LirStmt::FieldUpdate { .. } => {}
        }

        // Now check if this stmt has a constant condition
        let replacement = match &stmts[i] {
            LirStmt::If {
                cond: LirAtom::Bool(true),
                then_body,
                ..
            } => Some(then_body.clone()),
            LirStmt::If {
                cond: LirAtom::Bool(false),
                else_body,
                ..
            } => Some(else_body.clone()),
            _ => None,
        };

        if let Some(live_body) = replacement {
            stmts.splice(i..=i, live_body);
            // Don't increment — process the spliced-in stmts
            continue;
        }
        i += 1;
    }
}

// ─── Copy Propagation ────────────────────────────────────────────────────────

fn copy_propagate_stmts(
    stmts: &mut [LirStmt],
    subst: &mut HashMap<Symbol, LirAtom>,
    multi_bound: &HashSet<Symbol>,
) {
    for stmt in stmts.iter_mut() {
        copy_propagate_stmt(stmt, subst, multi_bound);
    }
}

fn copy_propagate_stmt(
    stmt: &mut LirStmt,
    subst: &mut HashMap<Symbol, LirAtom>,
    multi_bound: &HashSet<Symbol>,
) {
    match stmt {
        LirStmt::Let { name, typ, expr } => {
            subst_expr(expr, subst);
            // Only propagate copies when:
            // - Types match (the Let may perform implicit numeric coercion)
            // - The name is bound only once (multiply-bound names like match-expr
            //   result temps are not SSA and cannot be safely propagated)
            if !multi_bound.contains(name) {
                if let LirExpr::Atom(rhs) = expr {
                    let resolved = resolve_atom(rhs, subst);
                    if *typ == resolved.typ() {
                        subst.insert(*name, resolved);
                    }
                }
            }
        }
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            subst_atom(cond, subst);
            copy_propagate_stmts(then_body, subst, multi_bound);
            copy_propagate_stmts(else_body, subst, multi_bound);
        }
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ..
        } => {
            subst_atom(cond, subst);
            copy_propagate_stmts(then_body, subst, multi_bound);
            if let Some(ret) = then_ret {
                subst_atom(ret, subst);
            }
            copy_propagate_stmts(else_body, subst, multi_bound);
            if let Some(ret) = else_ret {
                subst_atom(ret, subst);
            }
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_body,
            catch_ret,
            ..
        } => {
            copy_propagate_stmts(body, subst, multi_bound);
            if let Some(ret) = body_ret {
                subst_atom(ret, subst);
            }
            copy_propagate_stmts(catch_body, subst, multi_bound);
            if let Some(ret) = catch_ret {
                subst_atom(ret, subst);
            }
        }
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            copy_propagate_stmts(cond_stmts, subst, multi_bound);
            subst_atom(cond, subst);
            copy_propagate_stmts(body, subst, multi_bound);
        }
        LirStmt::Switch {
            tag,
            cases,
            default_body,
            default_ret,
            ..
        } => {
            subst_atom(tag, subst);
            for case in cases {
                copy_propagate_stmts(&mut case.body, subst, multi_bound);
                if let Some(ret) = &mut case.ret {
                    subst_atom(ret, subst);
                }
            }
            copy_propagate_stmts(default_body, subst, multi_bound);
            if let Some(ret) = default_ret {
                subst_atom(ret, subst);
            }
        }
        LirStmt::FieldUpdate { target, value, .. } => {
            subst_atom(target, subst);
            subst_atom(value, subst);
        }
    }
}

fn subst_expr(expr: &mut LirExpr, subst: &HashMap<Symbol, LirAtom>) {
    match expr {
        LirExpr::Atom(a) => subst_atom(a, subst),
        LirExpr::Binary { lhs, rhs, .. } => {
            subst_atom(lhs, subst);
            subst_atom(rhs, subst);
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            for (_, arg) in args {
                subst_atom(arg, subst);
            }
        }
        LirExpr::Constructor { args, .. } => {
            for arg in args {
                subst_atom(arg, subst);
            }
        }
        LirExpr::Record { fields, .. } => {
            for (_, val) in fields {
                subst_atom(val, subst);
            }
        }
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            subst_atom(value, subst);
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => subst_atom(value, subst),
        LirExpr::LazySpawn { thunk, .. } => subst_atom(thunk, subst),
        LirExpr::LazyJoin { task_id, .. } => subst_atom(task_id, subst),
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                subst_atom(a, subst);
            }
        }
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::Closure { captures, .. } => {
            for (_, cap) in captures {
                subst_atom(cap, subst);
            }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            subst_atom(callee, subst);
            for (_, arg) in args {
                subst_atom(arg, subst);
            }
        }
    }
}

fn subst_atom(atom: &mut LirAtom, subst: &HashMap<Symbol, LirAtom>) {
    if let LirAtom::Var { name, .. } = atom {
        if let Some(replacement) = subst.get(name) {
            *atom = replacement.clone();
        }
    }
}

/// Resolve an atom through the substitution map transitively.
fn resolve_atom(atom: &LirAtom, subst: &HashMap<Symbol, LirAtom>) -> LirAtom {
    let mut current = atom.clone();
    for _ in 0..64 {
        if let LirAtom::Var { name, .. } = &current {
            if let Some(next) = subst.get(name) {
                current = next.clone();
                continue;
            }
        }
        break;
    }
    current
}

// ─── Dead Let Elimination ────────────────────────────────────────────────────

fn count_uses_in_stmts(stmts: &[LirStmt], uses: &mut HashMap<Symbol, u32>) {
    for stmt in stmts {
        count_uses_in_stmt(stmt, uses);
    }
}

fn count_uses_in_stmt(stmt: &LirStmt, uses: &mut HashMap<Symbol, u32>) {
    match stmt {
        LirStmt::Let { expr, .. } => count_uses_in_expr(expr, uses),
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            count_uses_in_atom(cond, uses);
            count_uses_in_stmts(then_body, uses);
            count_uses_in_stmts(else_body, uses);
        }
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ..
        } => {
            count_uses_in_atom(cond, uses);
            count_uses_in_stmts(then_body, uses);
            if let Some(ret) = then_ret {
                count_uses_in_atom(ret, uses);
            }
            count_uses_in_stmts(else_body, uses);
            if let Some(ret) = else_ret {
                count_uses_in_atom(ret, uses);
            }
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_body,
            catch_ret,
            ..
        } => {
            count_uses_in_stmts(body, uses);
            if let Some(ret) = body_ret {
                count_uses_in_atom(ret, uses);
            }
            count_uses_in_stmts(catch_body, uses);
            if let Some(ret) = catch_ret {
                count_uses_in_atom(ret, uses);
            }
        }
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            count_uses_in_stmts(cond_stmts, uses);
            count_uses_in_atom(cond, uses);
            count_uses_in_stmts(body, uses);
        }
        LirStmt::Switch {
            tag,
            cases,
            default_body,
            default_ret,
            ..
        } => {
            count_uses_in_atom(tag, uses);
            for case in cases {
                count_uses_in_stmts(&case.body, uses);
                if let Some(ret) = &case.ret {
                    count_uses_in_atom(ret, uses);
                }
            }
            count_uses_in_stmts(default_body, uses);
            if let Some(ret) = default_ret {
                count_uses_in_atom(ret, uses);
            }
        }
        LirStmt::FieldUpdate { target, value, .. } => {
            count_uses_in_atom(target, uses);
            count_uses_in_atom(value, uses);
        }
    }
}

fn count_uses_in_expr(expr: &LirExpr, uses: &mut HashMap<Symbol, u32>) {
    match expr {
        LirExpr::Atom(a) => count_uses_in_atom(a, uses),
        LirExpr::Binary { lhs, rhs, .. } => {
            count_uses_in_atom(lhs, uses);
            count_uses_in_atom(rhs, uses);
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            for (_, arg) in args {
                count_uses_in_atom(arg, uses);
            }
        }
        LirExpr::Constructor { args, .. } => {
            for arg in args {
                count_uses_in_atom(arg, uses);
            }
        }
        LirExpr::Record { fields, .. } => {
            for (_, val) in fields {
                count_uses_in_atom(val, uses);
            }
        }
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            count_uses_in_atom(value, uses);
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => {
            count_uses_in_atom(value, uses)
        }
        LirExpr::LazySpawn { thunk, .. } => count_uses_in_atom(thunk, uses),
        LirExpr::LazyJoin { task_id, .. } => count_uses_in_atom(task_id, uses),
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                count_uses_in_atom(a, uses);
            }
        }
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::Closure { captures, .. } => {
            for (_, cap) in captures {
                count_uses_in_atom(cap, uses);
            }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            count_uses_in_atom(callee, uses);
            for (_, arg) in args {
                count_uses_in_atom(arg, uses);
            }
        }
    }
}

fn count_uses_in_atom(atom: &LirAtom, uses: &mut HashMap<Symbol, u32>) {
    if let LirAtom::Var { name, .. } = atom {
        *uses.entry(*name).or_default() += 1;
    }
}

fn count_dead_lets(stmts: &[LirStmt], uses: &HashMap<Symbol, u32>) -> usize {
    let mut count = 0;
    for stmt in stmts {
        match stmt {
            LirStmt::Let { name, expr, .. } => {
                let used = uses.get(name).copied().unwrap_or(0) > 0;
                if !used && !expr_has_side_effects(expr) {
                    count += 1;
                }
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                count += count_dead_lets(then_body, uses);
                count += count_dead_lets(else_body, uses);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                count += count_dead_lets(body, uses);
                count += count_dead_lets(catch_body, uses);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                count += count_dead_lets(cond_stmts, uses);
                count += count_dead_lets(body, uses);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    count += count_dead_lets(&case.body, uses);
                }
                count += count_dead_lets(default_body, uses);
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
    count
}

fn eliminate_dead_lets(stmts: &mut Vec<LirStmt>, uses: &HashMap<Symbol, u32>) {
    stmts.retain_mut(|stmt| {
        match stmt {
            LirStmt::Let { name, expr, .. } => {
                let used = uses.get(name).copied().unwrap_or(0) > 0;
                if !used && !expr_has_side_effects(expr) {
                    return false;
                }
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                eliminate_dead_lets(then_body, uses);
                eliminate_dead_lets(else_body, uses);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                eliminate_dead_lets(then_body, uses);
                eliminate_dead_lets(else_body, uses);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                eliminate_dead_lets(body, uses);
                eliminate_dead_lets(catch_body, uses);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                eliminate_dead_lets(cond_stmts, uses);
                eliminate_dead_lets(body, uses);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    eliminate_dead_lets(&mut case.body, uses);
                }
                eliminate_dead_lets(default_body, uses);
            }
            LirStmt::FieldUpdate { .. } => {}  // keep — side-effecting
        }
        true
    });
}

/// Returns true if the expression has observable side effects and must not be eliminated.
fn expr_has_side_effects(expr: &LirExpr) -> bool {
    matches!(
        expr,
        LirExpr::Call { .. }
            | LirExpr::TailCall { .. }
            | LirExpr::Raise { .. }
            | LirExpr::Force { .. }
            | LirExpr::CallIndirect { .. }
    )
}

// ─── Unreachable Code Stripping ─────────────────────────────────────────────

/// Returns true if a statement always diverges (never falls through to the next).
fn stmt_diverges(stmt: &LirStmt) -> bool {
    match stmt {
        LirStmt::Let { expr, .. } => {
            matches!(expr, LirExpr::Raise { .. } | LirExpr::TailCall { .. })
        }
        LirStmt::IfReturn {
            then_ret, else_ret, ..
        } => {
            // Diverges only if BOTH branches return (or the then branch returns
            // and there is no else branch — the else IS the continuation)
            then_ret.is_some() && else_ret.is_some()
        }
        LirStmt::Switch {
            cases, default_ret, ..
        } => {
            // Diverges if all cases AND default return
            default_ret.is_some() && cases.iter().all(|c| c.ret.is_some())
        }
        _ => false,
    }
}

/// Remove statements that follow a divergent statement in the same block.
/// Recurse into nested blocks.
fn strip_unreachable_stmts(stmts: &mut Vec<LirStmt>) {
    // Find first divergent statement
    let mut truncate_at = None;
    for (i, stmt) in stmts.iter().enumerate() {
        if stmt_diverges(stmt) && i + 1 < stmts.len() {
            truncate_at = Some(i + 1);
            break;
        }
    }
    if let Some(at) = truncate_at {
        stmts.truncate(at);
    }

    // Recurse into nested blocks
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                strip_unreachable_stmts(then_body);
                strip_unreachable_stmts(else_body);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                strip_unreachable_stmts(then_body);
                strip_unreachable_stmts(else_body);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                strip_unreachable_stmts(body);
                strip_unreachable_stmts(catch_body);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                strip_unreachable_stmts(cond_stmts);
                strip_unreachable_stmts(body);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    strip_unreachable_stmts(&mut case.body);
                }
                strip_unreachable_stmts(default_body);
            }
            LirStmt::Let { .. } => {}
            LirStmt::FieldUpdate { .. } => {}
        }
    }
}

// ─── Loop Invariant Code Motion (LICM) ──────────────────────────────────────

/// Hoist loop-invariant let bindings out of Loop statements.
/// A let binding is loop-invariant if its expression doesn't reference any
/// variable defined inside the loop and has no side effects.
fn hoist_loop_invariants(stmts: &mut Vec<LirStmt>) {
    let mut i = 0;
    while i < stmts.len() {
        // Recurse into nested bodies first
        match &mut stmts[i] {
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                hoist_loop_invariants(then_body);
                hoist_loop_invariants(else_body);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                hoist_loop_invariants(body);
                hoist_loop_invariants(catch_body);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    hoist_loop_invariants(&mut case.body);
                }
                hoist_loop_invariants(default_body);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                // Recurse into nested loops first
                hoist_loop_invariants(cond_stmts);
                hoist_loop_invariants(body);

                // Collect all variables defined inside the loop
                let mut loop_defs = HashSet::new();
                collect_defined_vars(cond_stmts, &mut loop_defs);
                collect_defined_vars(body, &mut loop_defs);

                // Extract invariant lets from cond_stmts
                let hoisted = extract_invariant_lets(cond_stmts, &loop_defs);

                // Insert hoisted lets before the Loop statement
                if !hoisted.is_empty() {
                    let n = hoisted.len();
                    stmts.splice(i..i, hoisted);
                    i += n; // skip past hoisted stmts to reach the Loop
                }
            }
            LirStmt::Let { .. } => {}
            LirStmt::FieldUpdate { .. } => {}
        }
        i += 1;
    }
}

/// Collect all variable names defined by Let bindings in the given stmts.
fn collect_defined_vars(stmts: &[LirStmt], defs: &mut HashSet<Symbol>) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { name, .. } => {
                defs.insert(*name);
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                collect_defined_vars(then_body, defs);
                collect_defined_vars(else_body, defs);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                collect_defined_vars(body, defs);
                collect_defined_vars(catch_body, defs);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                collect_defined_vars(cond_stmts, defs);
                collect_defined_vars(body, defs);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    collect_defined_vars(&case.body, defs);
                }
                collect_defined_vars(default_body, defs);
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
}

/// Extract let bindings that are loop-invariant from the front of a stmt list.
/// A let is invariant if:
/// - Its expression has no side effects
/// - Its expression doesn't reference any variable in `loop_defs`
/// Stops at the first non-invariant let or non-let statement.
fn extract_invariant_lets(stmts: &mut Vec<LirStmt>, loop_defs: &HashSet<Symbol>) -> Vec<LirStmt> {
    let mut hoisted = Vec::new();
    while !stmts.is_empty() {
        let is_invariant = if let LirStmt::Let { expr, .. } = &stmts[0] {
            !expr_has_side_effects(expr) && !expr_references_any(expr, loop_defs)
        } else {
            false
        };
        if is_invariant {
            hoisted.push(stmts.remove(0));
        } else {
            break;
        }
    }
    hoisted
}

/// Check if an expression references any variable in the given set.
fn expr_references_any(expr: &LirExpr, vars: &HashSet<Symbol>) -> bool {
    match expr {
        LirExpr::Atom(a) => atom_references_any(a, vars),
        LirExpr::Binary { lhs, rhs, .. } => {
            atom_references_any(lhs, vars) || atom_references_any(rhs, vars)
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            args.iter().any(|(_, a)| atom_references_any(a, vars))
        }
        LirExpr::Constructor { args, .. } => args.iter().any(|a| atom_references_any(a, vars)),
        LirExpr::Record { fields, .. } => fields.iter().any(|(_, a)| atom_references_any(a, vars)),
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            atom_references_any(value, vars)
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => {
            atom_references_any(value, vars)
        }
        LirExpr::LazySpawn { thunk, .. } => atom_references_any(thunk, vars),
        LirExpr::LazyJoin { task_id, .. } => atom_references_any(task_id, vars),
        LirExpr::Intrinsic { args, .. } => args.iter().any(|(_, a)| atom_references_any(a, vars)),
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => false,
        LirExpr::Closure { captures, .. } => {
            captures.iter().any(|(_, a)| atom_references_any(a, vars))
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            atom_references_any(callee, vars)
                || args.iter().any(|(_, a)| atom_references_any(a, vars))
        }
    }
}

fn atom_references_any(atom: &LirAtom, vars: &HashSet<Symbol>) -> bool {
    if let LirAtom::Var { name, .. } = atom {
        vars.contains(name)
    } else {
        false
    }
}

// ─── Known-Call Devirtualization ─────────────────────────────────────────────

/// Track FuncRef bindings and replace CallIndirect with direct Call when the
/// callee is a known FuncRef. This eliminates call_indirect overhead and
/// enables further optimizations (the FuncRef Let becomes dead code).
fn devirtualize_known_calls(stmts: &mut [LirStmt]) {
    let mut funcref_map: HashMap<Symbol, Symbol> = HashMap::new();
    collect_funcref_bindings(stmts, &mut funcref_map);
    if !funcref_map.is_empty() {
        devirtualize_calls_in_stmts(stmts, &funcref_map);
    }
}

fn collect_funcref_bindings(stmts: &[LirStmt], map: &mut HashMap<Symbol, Symbol>) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let {
                name,
                expr: LirExpr::FuncRef { func, .. },
                ..
            } => {
                map.insert(*name, *func);
            }
            // Empty-capture Closure is effectively a FuncRef
            LirStmt::Let {
                name,
                expr: LirExpr::Closure { func, captures, .. },
                ..
            } if captures.is_empty() => {
                map.insert(*name, *func);
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                collect_funcref_bindings(then_body, map);
                collect_funcref_bindings(else_body, map);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                collect_funcref_bindings(body, map);
                collect_funcref_bindings(catch_body, map);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                collect_funcref_bindings(cond_stmts, map);
                collect_funcref_bindings(body, map);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    collect_funcref_bindings(&case.body, map);
                }
                collect_funcref_bindings(default_body, map);
            }
            _ => {}
        }
    }
}

fn devirtualize_calls_in_stmts(
    stmts: &mut [LirStmt],
    funcref_map: &HashMap<Symbol, Symbol>,
) -> u32 {
    let mut count = 0;
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::Let { expr, .. } => {
                if let LirExpr::CallIndirect {
                    callee: LirAtom::Var { name, .. },
                    args,
                    typ,
                    ..
                } = expr
                {
                    if let Some(&target_func) = funcref_map.get(name) {
                        // All funcref targets have __env as first param (closure convention).
                        // Add a dummy __env arg (Int(0), never used for non-capturing closures).
                        let env_sym = Symbol::intern("__env");
                        let mut new_args = vec![(env_sym, LirAtom::Int(0))];
                        new_args.append(&mut std::mem::take(args));
                        *expr = LirExpr::Call {
                            func: target_func,
                            args: new_args,
                            typ: typ.clone(),
                        };
                        count += 1;
                    }
                }
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                count += devirtualize_calls_in_stmts(then_body, funcref_map);
                count += devirtualize_calls_in_stmts(else_body, funcref_map);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                count += devirtualize_calls_in_stmts(body, funcref_map);
                count += devirtualize_calls_in_stmts(catch_body, funcref_map);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                count += devirtualize_calls_in_stmts(cond_stmts, funcref_map);
                count += devirtualize_calls_in_stmts(body, funcref_map);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    count += devirtualize_calls_in_stmts(&mut case.body, funcref_map);
                }
                count += devirtualize_calls_in_stmts(default_body, funcref_map);
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
    count
}

// ─── Function Inlining ──────────────────────────────────────────────────────

/// Maximum number of LIR statements for a function to be considered inlineable.
const INLINE_THRESHOLD: usize = 12;

/// Inline small, non-recursive functions at call sites.
/// Only inlines functions whose bodies contain no control flow (Let-only).
fn inline_small_functions(program: &mut LirProgram) {
    // Collect inlineable function bodies (cloned for ownership)
    let inlineable: HashMap<Symbol, InlineCandidate> = program
        .functions
        .iter()
        .filter(|f| is_inlineable(f))
        .map(|f| {
            (
                f.name,
                InlineCandidate {
                    params: f.params.clone(),
                    body: f.body.clone(),
                    ret: f.ret.clone(),
                },
            )
        })
        .collect();

    if inlineable.is_empty() {
        return;
    }

    let mut inline_counter: u32 = 0;
    for func in &mut program.functions {
        let subst = inline_calls_in_stmts(&mut func.body, &inlineable, &mut inline_counter);
        // Apply inline substitutions to the function's return atom
        if !subst.is_empty() {
            if let LirAtom::Var { name, .. } = &func.ret {
                if let Some(replacement) = subst.get(name) {
                    func.ret = replacement.clone();
                }
            }
        }
    }

    // Note: we do NOT remove inlined functions because they may be called from
    // other compilation units (imported modules). The dead code will be handled
    // by wasmtime's JIT or wasm-opt.
}

struct InlineCandidate {
    params: Vec<crate::ir::lir::LirParam>,
    body: Vec<LirStmt>,
    ret: LirAtom,
}

/// A function is inlineable if:
/// - Not main or a WASI export wrapper
/// - Body contains only Let statements (no control flow)
/// - Body size ≤ INLINE_THRESHOLD
/// - Not self-recursive
/// - Body has no TailCall expressions
fn is_inlineable(func: &LirFunction) -> bool {
    let name_str = func.name.as_str();
    // Never inline entry points or exported functions
    if name_str == "main" || name_str.starts_with("__wasi_") || name_str.starts_with("__conc_") {
        return false;
    }
    if func.body.len() > INLINE_THRESHOLD {
        return false;
    }
    let name = func.name;
    for stmt in &func.body {
        match stmt {
            LirStmt::Let { expr, .. } => {
                // No self-recursion or TailCall
                match expr {
                    LirExpr::Call { func: callee, .. } if *callee == name => return false,
                    LirExpr::TailCall { .. } => return false,
                    _ => {}
                }
            }
            // Any control flow makes it non-inlineable
            _ => return false,
        }
    }
    true
}

/// Inline calls in a statement list, replacing Call expressions with the
/// inlined function body. Each inline site gets unique variable names via
/// a monotonic counter.
///
/// Instead of creating `Let result = Atom(renamed_ret)` copy bindings (which
/// can cause issues when copy propagation can't propagate due to type mismatch),
/// we substitute `result → renamed_ret` directly in subsequent statements.
fn inline_calls_in_stmts(
    stmts: &mut Vec<LirStmt>,
    inlineable: &HashMap<Symbol, InlineCandidate>,
    counter: &mut u32,
) -> HashMap<Symbol, LirAtom> {
    // Accumulated substitutions from inlined call results.
    // Applied eagerly to subsequent statements to avoid copy chains.
    let mut inline_subst: HashMap<Symbol, LirAtom> = HashMap::new();

    let mut i = 0;
    while i < stmts.len() {
        // Apply pending substitutions to the current stmt
        if !inline_subst.is_empty() {
            apply_subst_to_stmt(&mut stmts[i], &inline_subst);
        }

        let should_inline = if let LirStmt::Let {
            expr: LirExpr::Call { func, .. },
            ..
        } = &stmts[i]
        {
            inlineable.contains_key(func)
        } else {
            false
        };

        if should_inline {
            let site_id = *counter;
            *counter += 1;

            // Extract the Let statement
            let placeholder = LirStmt::Let {
                name: Symbol::intern("__placeholder"),
                typ: Type::Unit,
                expr: LirExpr::Atom(LirAtom::Unit),
            };
            let original = std::mem::replace(&mut stmts[i], placeholder);

            if let LirStmt::Let {
                name: result_name,
                typ: _result_typ,
                expr: LirExpr::Call { func, args, .. },
            } = original
            {
                let candidate = &inlineable[&func];
                let mut inserted = Vec::new();

                // 1. Bind parameters to arguments
                for (param, (_, arg_atom)) in candidate.params.iter().zip(args.iter()) {
                    let renamed_param =
                        Symbol::intern(&format!("__il{}_{}", site_id, param.name.as_str()));
                    inserted.push(LirStmt::Let {
                        name: renamed_param,
                        typ: param.typ.clone(),
                        expr: LirExpr::Atom(arg_atom.clone()),
                    });
                }

                // 2. Build rename map for body locals
                let mut rename_map: HashMap<Symbol, Symbol> = HashMap::new();
                for param in &candidate.params {
                    rename_map.insert(
                        param.name,
                        Symbol::intern(&format!("__il{}_{}", site_id, param.name.as_str())),
                    );
                }
                for stmt in &candidate.body {
                    if let LirStmt::Let { name, .. } = stmt {
                        rename_map.insert(
                            *name,
                            Symbol::intern(&format!("__il{}_{}", site_id, name.as_str())),
                        );
                    }
                }

                // 3. Clone and rename body stmts
                for body_stmt in &candidate.body {
                    let mut cloned = body_stmt.clone();
                    rename_stmt(&mut cloned, &rename_map);
                    inserted.push(cloned);
                }

                // 4. Instead of creating `Let result_name = Atom(renamed_ret)`,
                //    record result_name → renamed_ret in the substitution map.
                //    This avoids copy chains that confuse dead let elimination.
                let mut ret_atom = candidate.ret.clone();
                rename_atom(&mut ret_atom, &rename_map);
                // Also resolve through existing substitutions
                if let LirAtom::Var { name, .. } = &ret_atom {
                    if let Some(resolved) = inline_subst.get(name) {
                        ret_atom = resolved.clone();
                    }
                }
                inline_subst.insert(result_name, ret_atom);

                // Replace the single stmt with the inlined sequence
                stmts.splice(i..=i, inserted.into_iter());
                // Don't increment i — process the newly inserted stmts
                continue;
            }
        }
        // Note: we do NOT recurse into nested bodies (if/match/loop/etc.)
        // for inlining. Only top-level calls in a scope are inlined.
        i += 1;
    }
    inline_subst
}

/// Apply inline substitutions to atoms within a statement.
fn apply_subst_to_stmt(stmt: &mut LirStmt, subst: &HashMap<Symbol, LirAtom>) {
    match stmt {
        LirStmt::Let { expr, .. } => {
            subst_expr(expr, subst);
        }
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            subst_atom(cond, subst);
            apply_subst_to_stmts(then_body, subst);
            apply_subst_to_stmts(else_body, subst);
        }
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ..
        } => {
            subst_atom(cond, subst);
            apply_subst_to_stmts(then_body, subst);
            if let Some(ret) = then_ret {
                subst_atom(ret, subst);
            }
            apply_subst_to_stmts(else_body, subst);
            if let Some(ret) = else_ret {
                subst_atom(ret, subst);
            }
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_body,
            catch_ret,
            ..
        } => {
            apply_subst_to_stmts(body, subst);
            if let Some(ret) = body_ret {
                subst_atom(ret, subst);
            }
            apply_subst_to_stmts(catch_body, subst);
            if let Some(ret) = catch_ret {
                subst_atom(ret, subst);
            }
        }
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            apply_subst_to_stmts(cond_stmts, subst);
            subst_atom(cond, subst);
            apply_subst_to_stmts(body, subst);
        }
        LirStmt::Switch {
            tag,
            cases,
            default_body,
            default_ret,
            ..
        } => {
            subst_atom(tag, subst);
            for case in cases {
                apply_subst_to_stmts(&mut case.body, subst);
                if let Some(ret) = &mut case.ret {
                    subst_atom(ret, subst);
                }
            }
            apply_subst_to_stmts(default_body, subst);
            if let Some(ret) = default_ret {
                subst_atom(ret, subst);
            }
        }
        LirStmt::FieldUpdate { target, value, .. } => {
            subst_atom(target, subst);
            subst_atom(value, subst);
        }
    }
}

fn apply_subst_to_stmts(stmts: &mut [LirStmt], subst: &HashMap<Symbol, LirAtom>) {
    for stmt in stmts.iter_mut() {
        apply_subst_to_stmt(stmt, subst);
    }
}

fn rename_stmt(stmt: &mut LirStmt, map: &HashMap<Symbol, Symbol>) {
    match stmt {
        LirStmt::Let { name, expr, .. } => {
            if let Some(&new_name) = map.get(name) {
                *name = new_name;
            }
            rename_expr(expr, map);
        }
        _ => {} // Inlined bodies are Let-only, but be safe
    }
}

fn rename_expr(expr: &mut LirExpr, map: &HashMap<Symbol, Symbol>) {
    match expr {
        LirExpr::Atom(a) => rename_atom(a, map),
        LirExpr::Binary { lhs, rhs, .. } => {
            rename_atom(lhs, map);
            rename_atom(rhs, map);
        }
        LirExpr::Call { args, .. } => {
            for (_, arg) in args {
                rename_atom(arg, map);
            }
        }
        LirExpr::TailCall { args, .. } => {
            for (_, arg) in args {
                rename_atom(arg, map);
            }
        }
        LirExpr::Constructor { args, .. } => {
            for arg in args {
                rename_atom(arg, map);
            }
        }
        LirExpr::Record { fields, .. } => {
            for (_, val) in fields {
                rename_atom(val, map);
            }
        }
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            rename_atom(value, map);
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => rename_atom(value, map),
        LirExpr::LazySpawn { thunk, .. } => rename_atom(thunk, map),
        LirExpr::LazyJoin { task_id, .. } => rename_atom(task_id, map),
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                rename_atom(a, map);
            }
        }
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::Closure { captures, .. } => {
            for (_, cap) in captures {
                rename_atom(cap, map);
            }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            rename_atom(callee, map);
            for (_, arg) in args {
                rename_atom(arg, map);
            }
        }
    }
}

fn rename_atom(atom: &mut LirAtom, map: &HashMap<Symbol, Symbol>) {
    if let LirAtom::Var { name, .. } = atom {
        if let Some(&new_name) = map.get(name) {
            *name = new_name;
        }
    }
}

// ─── Identical Code Folding (ICF) ───────────────────────────────────────────

/// Detect functions with structurally identical bodies (modulo variable names)
/// and merge them by redirecting all calls to a single canonical version.
/// Particularly effective for enum constructors and small wrapper functions.
fn fold_identical_functions(program: &mut LirProgram) {
    // 1. Compute structural fingerprints for each function.
    //    The fingerprint ignores variable names but captures the structure:
    //    types, operations, call targets, literal values, and nesting.
    let mut fingerprints: HashMap<u64, Vec<usize>> = HashMap::new();
    for (idx, func) in program.functions.iter().enumerate() {
        let fp = structural_fingerprint(func);
        fingerprints.entry(fp).or_default().push(idx);
    }

    // 2. For groups with identical fingerprints, verify structural equality
    //    and build a redirect map (duplicate → canonical).
    let mut redirect: HashMap<Symbol, Symbol> = HashMap::new();
    for (_fp, indices) in &fingerprints {
        if indices.len() < 2 {
            continue;
        }
        // Pairwise comparison within group — first match wins as canonical
        let mut canonical: Vec<usize> = Vec::new();
        for &idx in indices {
            let mut found = false;
            for &canon_idx in &canonical {
                if functions_structurally_equal(
                    &program.functions[canon_idx],
                    &program.functions[idx],
                ) {
                    redirect.insert(
                        program.functions[idx].name,
                        program.functions[canon_idx].name,
                    );
                    found = true;
                    break;
                }
            }
            if !found {
                canonical.push(idx);
            }
        }
    }

    if redirect.is_empty() {
        return;
    }

    // 3. Rewrite all Call targets using the redirect map.
    for func in &mut program.functions {
        redirect_calls_in_stmts(&mut func.body, &redirect);
    }

    // 4. Remove redirected functions (they're now unused).
    //    Keep exported functions (__conc_, __wasi_, main) that the runtime calls by name.
    program.functions.retain(|f| {
        if !redirect.contains_key(&f.name) {
            return true;
        }
        let name = f.name.as_str();
        name == "main"
            || name.starts_with("__conc_")
            || name.starts_with("__wasi_")
            || name.starts_with("__closure_wrap_")
    });
}

/// Compute a structural hash of a function, ignoring variable names.
fn structural_fingerprint(func: &LirFunction) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();

    // Hash param count and types (not names)
    func.params.len().hash(&mut h);
    for p in &func.params {
        format!("{:?}", p.typ).hash(&mut h);
    }
    format!("{:?}", func.ret_type).hash(&mut h);

    // Hash body structure
    hash_stmts(&func.body, &mut h);

    // Hash return atom structure (not var name)
    hash_atom_structure(&func.ret, &mut h);

    h.finish()
}

fn hash_stmts(stmts: &[LirStmt], h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    stmts.len().hash(h);
    for stmt in stmts {
        hash_stmt(stmt, h);
    }
}

fn hash_stmt(stmt: &LirStmt, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    std::mem::discriminant(stmt).hash(h);
    match stmt {
        LirStmt::Let { typ, expr, .. } => {
            format!("{:?}", typ).hash(h);
            hash_expr(expr, h);
        }
        LirStmt::If {
            then_body,
            else_body,
            ..
        } => {
            hash_stmts(then_body, h);
            hash_stmts(else_body, h);
        }
        LirStmt::IfReturn {
            then_body,
            then_ret,
            else_body,
            else_ret,
            ret_type,
            ..
        } => {
            hash_stmts(then_body, h);
            then_ret.is_some().hash(h);
            if let Some(r) = then_ret {
                hash_atom_structure(r, h);
            }
            hash_stmts(else_body, h);
            else_ret.is_some().hash(h);
            if let Some(r) = else_ret {
                hash_atom_structure(r, h);
            }
            format!("{:?}", ret_type).hash(h);
        }
        LirStmt::Switch {
            cases,
            default_body,
            default_ret,
            ret_type,
            ..
        } => {
            cases.len().hash(h);
            for case in cases {
                case.tag_value.hash(h);
                hash_stmts(&case.body, h);
                case.ret.is_some().hash(h);
                if let Some(r) = &case.ret {
                    hash_atom_structure(r, h);
                }
            }
            hash_stmts(default_body, h);
            default_ret.is_some().hash(h);
            format!("{:?}", ret_type).hash(h);
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_body,
            catch_ret,
            catch_param_typ,
            ..
        } => {
            hash_stmts(body, h);
            body_ret.is_some().hash(h);
            hash_stmts(catch_body, h);
            catch_ret.is_some().hash(h);
            format!("{:?}", catch_param_typ).hash(h);
        }
        LirStmt::Loop {
            cond_stmts, body, ..
        } => {
            hash_stmts(cond_stmts, h);
            hash_stmts(body, h);
        }
        LirStmt::FieldUpdate { target, byte_offset, value, .. } => {
            hash_atom_structure(target, h);
            byte_offset.hash(h);
            hash_atom_structure(value, h);
        }
    }
}

fn hash_expr(expr: &LirExpr, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    std::mem::discriminant(expr).hash(h);
    match expr {
        LirExpr::Atom(a) => hash_atom_structure(a, h),
        LirExpr::Binary { op, typ, .. } => {
            op.hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::Call { func, args, typ } => {
            func.hash(h);
            args.len().hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::TailCall { func, args, typ } => {
            func.hash(h);
            args.len().hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::Constructor { name, args, typ } => {
            name.hash(h);
            args.len().hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::Record { fields, typ } => {
            fields.len().hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::ObjectTag { typ, .. } => format!("{:?}", typ).hash(h),
        LirExpr::ObjectField { index, typ, .. } => {
            index.hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::Raise { typ, .. } | LirExpr::Force { typ, .. } => format!("{:?}", typ).hash(h),
        LirExpr::LazySpawn { num_captures, typ, .. } => {
            num_captures.hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::LazyJoin { typ, .. } => format!("{:?}", typ).hash(h),
        LirExpr::Intrinsic { kind, typ, .. } => {
            format!("{:?}{:?}", kind, typ).hash(h);
        }
        LirExpr::FuncRef { func, .. } => func.hash(h),
        LirExpr::Closure { func, captures, .. } => {
            func.hash(h);
            captures.len().hash(h);
        }
        LirExpr::ClosureEnvLoad { index, typ } => {
            index.hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::CallIndirect {
            args,
            typ,
            callee_type,
            ..
        } => {
            args.len().hash(h);
            format!("{:?}", typ).hash(h);
            format!("{:?}", callee_type).hash(h);
        }
    }
}

/// Hash the structure of an atom, ignoring variable names but keeping
/// the type and literal values.
fn hash_atom_structure(atom: &LirAtom, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    std::mem::discriminant(atom).hash(h);
    match atom {
        LirAtom::Var { typ, .. } => format!("{:?}", typ).hash(h),
        LirAtom::Int(v) => v.hash(h),
        LirAtom::Float(v) => v.to_bits().hash(h),
        LirAtom::Bool(v) => v.hash(h),
        LirAtom::Char(v) => v.hash(h),
        LirAtom::String(v) => v.hash(h),
        LirAtom::Unit => {}
    }
}

/// Check structural equality of two functions, ignoring variable names.
/// Both functions must have the same param/return types, same body structure,
/// and same operations (calls to the same functions, same constructors, etc).
fn functions_structurally_equal(a: &LirFunction, b: &LirFunction) -> bool {
    if a.params.len() != b.params.len() || a.ret_type != b.ret_type {
        return false;
    }
    for (pa, pb) in a.params.iter().zip(b.params.iter()) {
        if pa.typ != pb.typ {
            return false;
        }
    }
    if a.body.len() != b.body.len() {
        return false;
    }
    // Build a variable-name mapping: a's names → b's names
    let mut name_map: HashMap<Symbol, Symbol> = HashMap::new();
    for (pa, pb) in a.params.iter().zip(b.params.iter()) {
        name_map.insert(pa.name, pb.name);
    }
    stmts_structurally_equal(&a.body, &b.body, &mut name_map)
        && atoms_structurally_equal(&a.ret, &b.ret, &name_map)
}

fn stmts_structurally_equal(
    a: &[LirStmt],
    b: &[LirStmt],
    name_map: &mut HashMap<Symbol, Symbol>,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (sa, sb) in a.iter().zip(b.iter()) {
        if std::mem::discriminant(sa) != std::mem::discriminant(sb) {
            return false;
        }
        match (sa, sb) {
            (
                LirStmt::Let {
                    name: na,
                    typ: ta,
                    expr: ea,
                },
                LirStmt::Let {
                    name: nb,
                    typ: tb,
                    expr: eb,
                },
            ) => {
                if ta != tb || !exprs_structurally_equal(ea, eb, name_map) {
                    return false;
                }
                name_map.insert(*na, *nb);
            }
            (
                LirStmt::If {
                    cond: ca,
                    then_body: ta,
                    else_body: ea,
                },
                LirStmt::If {
                    cond: cb,
                    then_body: tb,
                    else_body: eb,
                },
            ) => {
                if !atoms_structurally_equal(ca, cb, name_map)
                    || !stmts_structurally_equal(ta, tb, name_map)
                    || !stmts_structurally_equal(ea, eb, name_map)
                {
                    return false;
                }
            }
            // For other stmt types, fall back to Debug equality (conservative)
            _ => {
                if format!("{:?}", sa) != format!("{:?}", sb) {
                    return false;
                }
            }
        }
    }
    true
}

fn exprs_structurally_equal(a: &LirExpr, b: &LirExpr, name_map: &HashMap<Symbol, Symbol>) -> bool {
    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        return false;
    }
    match (a, b) {
        (LirExpr::Atom(aa), LirExpr::Atom(ab)) => atoms_structurally_equal(aa, ab, name_map),
        (
            LirExpr::Binary {
                op: oa,
                lhs: la,
                rhs: ra,
                typ: ta,
            },
            LirExpr::Binary {
                op: ob,
                lhs: lb,
                rhs: rb,
                typ: tb,
            },
        ) => {
            oa == ob
                && ta == tb
                && atoms_structurally_equal(la, lb, name_map)
                && atoms_structurally_equal(ra, rb, name_map)
        }
        (
            LirExpr::Call {
                func: fa,
                args: aa,
                typ: ta,
            },
            LirExpr::Call {
                func: fb,
                args: ab,
                typ: tb,
            },
        ) => {
            fa == fb
                && ta == tb
                && aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|((_, a), (_, b))| atoms_structurally_equal(a, b, name_map))
        }
        (
            LirExpr::Constructor {
                name: na,
                args: aa,
                typ: ta,
            },
            LirExpr::Constructor {
                name: nb,
                args: ab,
                typ: tb,
            },
        ) => {
            na == nb
                && ta == tb
                && aa.len() == ab.len()
                && aa
                    .iter()
                    .zip(ab.iter())
                    .all(|(a, b)| atoms_structurally_equal(a, b, name_map))
        }
        (LirExpr::ObjectTag { value: va, typ: ta }, LirExpr::ObjectTag { value: vb, typ: tb }) => {
            ta == tb && atoms_structurally_equal(va, vb, name_map)
        }
        (
            LirExpr::ObjectField {
                value: va,
                index: ia,
                typ: ta,
            },
            LirExpr::ObjectField {
                value: vb,
                index: ib,
                typ: tb,
            },
        ) => ia == ib && ta == tb && atoms_structurally_equal(va, vb, name_map),
        // Conservative fallback for other variants
        _ => format!("{:?}", a) == format!("{:?}", b),
    }
}

fn atoms_structurally_equal(a: &LirAtom, b: &LirAtom, name_map: &HashMap<Symbol, Symbol>) -> bool {
    match (a, b) {
        (LirAtom::Var { name: na, typ: ta }, LirAtom::Var { name: nb, typ: tb }) => {
            ta == tb && name_map.get(na).map_or(*na == *nb, |mapped| *mapped == *nb)
        }
        (LirAtom::Int(a), LirAtom::Int(b)) => a == b,
        (LirAtom::Float(a), LirAtom::Float(b)) => a == b,
        (LirAtom::Bool(a), LirAtom::Bool(b)) => a == b,
        (LirAtom::Char(a), LirAtom::Char(b)) => a == b,
        (LirAtom::String(a), LirAtom::String(b)) => a == b,
        (LirAtom::Unit, LirAtom::Unit) => true,
        _ => false,
    }
}

/// Rewrite Call targets in all statements using the redirect map.
fn redirect_calls_in_stmts(stmts: &mut [LirStmt], redirect: &HashMap<Symbol, Symbol>) {
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::Let { expr, .. } => {
                redirect_calls_in_expr(expr, redirect);
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                redirect_calls_in_stmts(then_body, redirect);
                redirect_calls_in_stmts(else_body, redirect);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                redirect_calls_in_stmts(body, redirect);
                redirect_calls_in_stmts(catch_body, redirect);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                redirect_calls_in_stmts(cond_stmts, redirect);
                redirect_calls_in_stmts(body, redirect);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    redirect_calls_in_stmts(&mut case.body, redirect);
                }
                redirect_calls_in_stmts(default_body, redirect);
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
}

fn redirect_calls_in_expr(expr: &mut LirExpr, redirect: &HashMap<Symbol, Symbol>) {
    match expr {
        LirExpr::Call { func, .. } | LirExpr::TailCall { func, .. } => {
            if let Some(&target) = redirect.get(func) {
                *func = target;
            }
        }
        LirExpr::FuncRef { func, .. } | LirExpr::Closure { func, .. } => {
            if let Some(&target) = redirect.get(func) {
                *func = target;
            }
        }
        _ => {}
    }
}

// ── Lazy parallelization ────────────────────────────────────────────────────

/// Detect 2+ consecutive zero-arg CallIndirect statements (lazy force) and
/// convert them to LazySpawn/LazyJoin pairs for parallel evaluation.
fn parallelize_consecutive_forces(stmts: &mut Vec<LirStmt>) {
    // Recursively process nested blocks first
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                parallelize_consecutive_forces(then_body);
                parallelize_consecutive_forces(else_body);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                parallelize_consecutive_forces(then_body);
                parallelize_consecutive_forces(else_body);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                parallelize_consecutive_forces(cond_stmts);
                parallelize_consecutive_forces(body);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for c in cases {
                    parallelize_consecutive_forces(&mut c.body);
                }
                parallelize_consecutive_forces(default_body);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                parallelize_consecutive_forces(body);
                parallelize_consecutive_forces(catch_body);
            }
            LirStmt::Let { .. } => {}
            LirStmt::FieldUpdate { .. } => {}
        }
    }

    // Build a map: variable name → capture count (from Closure creation stmts)
    let mut closure_capture_counts: HashMap<Symbol, u32> = HashMap::new();
    for stmt in stmts.iter() {
        if let LirStmt::Let {
            name,
            expr: LirExpr::Closure { captures, .. },
            ..
        } = stmt
        {
            closure_capture_counts.insert(*name, captures.len() as u32);
        }
        // FuncRef has zero captures
        if let LirStmt::Let {
            name,
            expr: LirExpr::FuncRef { .. },
            ..
        } = stmt
        {
            closure_capture_counts.insert(*name, 0);
        }
    }

    // Find runs of 2+ consecutive zero-arg CallIndirect (force) stmts
    let mut new_stmts = Vec::with_capacity(stmts.len());
    let mut i = 0;
    while i < stmts.len() {
        let run_start = i;
        // Collect consecutive force calls
        let mut force_run: Vec<(Symbol, LirAtom, Type, u32)> = Vec::new();
        while i < stmts.len() {
            if let LirStmt::Let {
                name,
                expr: LirExpr::CallIndirect { callee, args, typ, .. },
                ..
            } = &stmts[i]
            {
                if args.is_empty() {
                    let num_captures = match callee {
                        LirAtom::Var { name: thunk_name, .. } => {
                            closure_capture_counts.get(thunk_name).copied().unwrap_or(0)
                        }
                        _ => 0,
                    };
                    force_run.push((*name, callee.clone(), typ.clone(), num_captures));
                    i += 1;
                    continue;
                }
            }
            break;
        }

        if force_run.len() >= 2 {
            // Emit LazySpawn for each thunk
            let mut task_ids: Vec<(Symbol, Symbol, Type)> = Vec::new();
            for (result_name, thunk_atom, result_type, num_captures) in &force_run {
                let tid_name = Symbol::from(format!("__lazy_tid_{}", result_name).as_str());
                new_stmts.push(LirStmt::Let {
                    name: tid_name,
                    typ: Type::I64,
                    expr: LirExpr::LazySpawn {
                        thunk: thunk_atom.clone(),
                        num_captures: *num_captures,
                        typ: Type::I64,
                    },
                });
                task_ids.push((tid_name, *result_name, result_type.clone()));
            }
            // Emit LazyJoin for each, in order
            for (tid_name, result_name, result_type) in task_ids {
                new_stmts.push(LirStmt::Let {
                    name: result_name,
                    typ: result_type.clone(),
                    expr: LirExpr::LazyJoin {
                        task_id: LirAtom::Var {
                            name: tid_name,
                            typ: Type::I64,
                        },
                        typ: result_type,
                    },
                });
            }
        } else {
            // Not a parallelizable run — emit original stmts
            for stmt in stmts[run_start..i].iter() {
                new_stmts.push(stmt.clone());
            }
            if i == run_start {
                // No progress — push current stmt and advance
                new_stmts.push(stmts[i].clone());
                i += 1;
            }
        }
    }
    *stmts = new_stmts;
}

// ─── Linear reuse: in-place Constructor update ─────────────────────────────

/// Replace Constructor allocations with in-place FieldUpdate when the constructor
/// args trace back to ObjectField extractions from a source that is dead afterward.
///
/// Detects patterns like:
///   let f0 = ObjectField(src, 0)
///   let f1 = ObjectField(src, 1)
///   let new_val = Call(g, [f0])
///   let result = Constructor("Cons", [new_val, f1])
///
/// Transforms to:
// ─── Scalar Replacement of Aggregates (SRA) ────────────────────────────────

/// Eliminate Constructor heap allocations when the result is only accessed
/// through ObjectTag / ObjectField (never escapes to calls, returns, etc.).
///
///   let x = Constructor("Some", [v])   ← HEAP ALLOC
///   let tag = ObjectTag(x)              ← tag read
///   let field = ObjectField(x, 0)       ← field read
///
// ─── Deforestation: fuse chained list operations ────────────────────────────

/// Fuse consecutive list operations to eliminate intermediate list allocations.
/// Currently handles:
///   - map∘map: `map(f, map(g, xs))` → `map(f∘g, xs)` (single traversal)
///   - reverse∘reverse: `reverse(reverse(xs))` → `xs` (identity)
fn fuse_list_operations(program: &mut LirProgram) {
    // Identify stdlib list function names from externals.
    let mut map_funcs = HashSet::new();
    let mut reverse_funcs = HashSet::new();
    for ext in &program.externals {
        if ext.wasm_name.as_str() == "map" {
            map_funcs.insert(ext.name);
        }
        if ext.wasm_name.as_str() == "reverse" {
            reverse_funcs.insert(ext.name);
        }
    }

    if map_funcs.is_empty() && reverse_funcs.is_empty() {
        return;
    }

    // Pre-collect function signatures (name → params, ret_type) to avoid borrow conflicts.
    let func_sigs: HashMap<Symbol, (Vec<LirParam>, Type)> = program
        .functions
        .iter()
        .map(|f| (f.name, (f.params.clone(), f.ret_type.clone())))
        .collect();

    let mut new_funcs: Vec<LirFunction> = Vec::new();
    let mut compose_counter = 0u32;

    for func in &mut program.functions {
        fuse_in_stmts(
            &mut func.body,
            &map_funcs,
            &reverse_funcs,
            &func_sigs,
            &mut new_funcs,
            &mut compose_counter,
        );
    }

    program.functions.extend(new_funcs);
}

fn fuse_in_stmts(
    stmts: &mut Vec<LirStmt>,
    map_funcs: &HashSet<Symbol>,
    reverse_funcs: &HashSet<Symbol>,
    func_sigs: &HashMap<Symbol, (Vec<LirParam>, Type)>,
    new_funcs: &mut Vec<LirFunction>,
    counter: &mut u32,
) {
    // Recurse into sub-bodies first.
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::If { then_body, else_body, .. }
            | LirStmt::IfReturn { then_body, else_body, .. } => {
                fuse_in_stmts(then_body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
                fuse_in_stmts(else_body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
            }
            LirStmt::Switch { cases, default_body, .. } => {
                for c in cases.iter_mut() {
                    fuse_in_stmts(&mut c.body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
                }
                fuse_in_stmts(default_body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
            }
            LirStmt::TryCatch { body, catch_body, .. } => {
                fuse_in_stmts(body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
                fuse_in_stmts(catch_body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
            }
            LirStmt::Loop { cond_stmts, body, .. } => {
                fuse_in_stmts(cond_stmts, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
                fuse_in_stmts(body, map_funcs, reverse_funcs, func_sigs, new_funcs, counter);
            }
            _ => {}
        }
    }

    // Build definition map: var → what it was bound to.
    let mut var_defs: HashMap<Symbol, &LirExpr> = HashMap::new();
    for stmt in stmts.iter() {
        if let LirStmt::Let { name, expr, .. } = stmt {
            var_defs.insert(*name, expr);
        }
    }

    // Count uses of each variable.
    let mut uses: HashMap<Symbol, u32> = HashMap::new();
    count_uses_in_stmts(stmts, &mut uses);

    // Collect transformations: (stmt_index, replacement_stmts)
    // Each entry replaces the stmt at `index` with a sequence of new stmts.
    let mut transforms: Vec<(usize, Vec<LirStmt>)> = Vec::new();

    for (i, stmt) in stmts.iter().enumerate() {
        let LirStmt::Let { name, typ, expr } = stmt else { continue };

        // ── reverse∘reverse elimination ──
        if let LirExpr::Call { func, args, .. } = expr {
            if reverse_funcs.contains(func) {
                let xs_arg = args.iter().find(|(l, _)| l.as_str() == "xs");
                if let Some((_, LirAtom::Var { name: inner_name, .. })) = xs_arg {
                    if uses.get(inner_name).copied().unwrap_or(0) == 1 {
                        if let Some(LirExpr::Call { func: inner_func, args: inner_args, .. }) = var_defs.get(inner_name) {
                            if reverse_funcs.contains(inner_func) {
                                if let Some((_, original_xs)) = inner_args.iter().find(|(l, _)| l.as_str() == "xs") {
                                    transforms.push((i, vec![LirStmt::Let {
                                        name: *name,
                                        typ: typ.clone(),
                                        expr: LirExpr::Atom(original_xs.clone()),
                                    }]));
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── map∘map fusion ──
        let LirExpr::Call { func: outer_map, args: outer_args, typ: call_typ, .. } = expr else { continue };
        if !map_funcs.contains(outer_map) { continue; }
        let Some((_, outer_xs)) = outer_args.iter().find(|(l, _)| l.as_str() == "xs") else { continue };
        let Some((_, outer_f)) = outer_args.iter().find(|(l, _)| l.as_str() == "f") else { continue };

        let LirAtom::Var { name: inner_name, .. } = outer_xs else { continue };
        if uses.get(inner_name).copied().unwrap_or(0) != 1 { continue; }
        let Some(LirExpr::Call { func: inner_map, args: inner_args, .. }) = var_defs.get(inner_name) else { continue };
        if !map_funcs.contains(inner_map) { continue; }

        let Some((_, inner_xs)) = inner_args.iter().find(|(l, _)| l.as_str() == "xs") else { continue };
        let Some((_, inner_f)) = inner_args.iter().find(|(l, _)| l.as_str() == "f") else { continue };

        // Both f and g must trace to FuncRef (known functions, no captures).
        let LirAtom::Var { name: outer_f_var, .. } = outer_f else { continue };
        let LirAtom::Var { name: inner_f_var, .. } = inner_f else { continue };
        let Some(LirExpr::FuncRef { func: f_name, .. }) = var_defs.get(outer_f_var) else { continue };
        let Some(LirExpr::FuncRef { func: g_name, .. }) = var_defs.get(inner_f_var) else { continue };

        // Look up function signatures.
        let Some((f_params, f_ret)) = func_sigs.get(f_name) else { continue };
        let Some((g_params, g_ret)) = func_sigs.get(g_name) else { continue };
        if f_params.is_empty() || g_params.is_empty() { continue; }

        // Generate: fn __compose_N(val) = f(g(val))
        let compose_name = Symbol::from(format!("__compose_{}", counter));
        let compose_ref = Symbol::from(format!("__compose_ref_{}", counter));
        *counter += 1;

        let val_param = LirParam {
            label: g_params[0].label,
            name: g_params[0].name,
            typ: g_params[0].typ.clone(),
        };
        let g_result = Symbol::from("__g_result");
        let f_result = Symbol::from("__f_result");

        new_funcs.push(LirFunction {
            name: compose_name,
            params: vec![val_param.clone()],
            ret_type: f_ret.clone(),
            requires: Type::Unit,
            throws: Type::Unit,
            body: vec![
                LirStmt::Let {
                    name: g_result,
                    typ: g_ret.clone(),
                    expr: LirExpr::Call {
                        func: *g_name,
                        args: vec![(g_params[0].label, LirAtom::Var { name: val_param.name, typ: val_param.typ.clone() })],
                        typ: g_ret.clone(),
                    },
                },
                LirStmt::Let {
                    name: f_result,
                    typ: f_ret.clone(),
                    expr: LirExpr::Call {
                        func: *f_name,
                        args: vec![(f_params[0].label, LirAtom::Var { name: g_result, typ: g_ret.clone() })],
                        typ: f_ret.clone(),
                    },
                },
            ],
            ret: LirAtom::Var { name: f_result, typ: f_ret.clone() },
            span: Span::default(),
            source_file: None,
            source_line: None,
        });

        // Replace outer map with: let ref = FuncRef(compose); let result = map(ref, original_xs)
        let f_label = outer_args.iter().find(|(l, _)| l.as_str() == "f").unwrap().0;
        let xs_label = outer_args.iter().find(|(l, _)| l.as_str() == "xs").unwrap().0;
        transforms.push((i, vec![
            LirStmt::Let {
                name: compose_ref,
                typ: Type::I64,
                expr: LirExpr::FuncRef { func: compose_name, typ: Type::I64 },
            },
            LirStmt::Let {
                name: *name,
                typ: typ.clone(),
                expr: LirExpr::Call {
                    func: *outer_map,
                    args: vec![
                        (f_label, LirAtom::Var { name: compose_ref, typ: Type::I64 }),
                        (xs_label, inner_xs.clone()),
                    ],
                    typ: call_typ.clone(),
                },
            },
        ]));
    }

    // Apply transforms in reverse order to preserve indices.
    for (idx, replacement) in transforms.into_iter().rev() {
        stmts.splice(idx..=idx, replacement);
    }
}

// ─── Scalar Replacement of Aggregates (SRA) ────────────────────────────────

/// Becomes (after SRA + constant folding + DCE):
///   let tag = Atom(Int(SOME_TAG))       ← known constant
///   let field = Atom(v)                 ← original arg
///   (x is dead → removed by DCE)
fn scalar_replace_aggregates(stmts: &mut Vec<LirStmt>, func_ret: &LirAtom) {
    // Phase 1: collect Constructor definitions
    let mut ctors: HashMap<Symbol, (i64, Vec<LirAtom>)> = HashMap::new();
    collect_ctor_defs_in_stmts(stmts, &mut ctors);
    if ctors.is_empty() {
        return;
    }

    // Phase 2: mark escaped constructors (used outside ObjectTag/ObjectField)
    let mut escaped = HashSet::new();
    if let LirAtom::Var { name, .. } = func_ret {
        if ctors.contains_key(name) {
            escaped.insert(*name);
        }
    }
    mark_escapes_in_stmts(stmts, &ctors, &mut escaped);
    for e in &escaped {
        ctors.remove(e);
    }
    if ctors.is_empty() {
        return;
    }

    // Phase 3: replace ObjectTag/ObjectField with direct values
    apply_sra_in_stmts(stmts, &ctors);
}

fn collect_ctor_defs_in_stmts(stmts: &[LirStmt], ctors: &mut HashMap<Symbol, (i64, Vec<LirAtom>)>) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let {
                name,
                expr: LirExpr::Constructor {
                    name: ctor_name,
                    args,
                    ..
                },
                ..
            } => {
                let tag = constructor_tag(ctor_name.as_str(), args.len());
                ctors.insert(*name, (tag, args.clone()));
            }
            LirStmt::If { then_body, else_body, .. }
            | LirStmt::IfReturn { then_body, else_body, .. } => {
                collect_ctor_defs_in_stmts(then_body, ctors);
                collect_ctor_defs_in_stmts(else_body, ctors);
            }
            LirStmt::Switch { cases, default_body, .. } => {
                for c in cases {
                    collect_ctor_defs_in_stmts(&c.body, ctors);
                }
                collect_ctor_defs_in_stmts(default_body, ctors);
            }
            LirStmt::TryCatch { body, catch_body, .. } => {
                collect_ctor_defs_in_stmts(body, ctors);
                collect_ctor_defs_in_stmts(catch_body, ctors);
            }
            LirStmt::Loop { cond_stmts, body, .. } => {
                collect_ctor_defs_in_stmts(cond_stmts, ctors);
                collect_ctor_defs_in_stmts(body, ctors);
            }
            _ => {}
        }
    }
}

/// Mark constructors as escaped if they appear in any position other than
/// ObjectTag.value or ObjectField.value.
fn mark_escapes_in_stmts(
    stmts: &[LirStmt],
    ctors: &HashMap<Symbol, (i64, Vec<LirAtom>)>,
    escaped: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { expr, .. } => mark_escapes_in_expr(expr, ctors, escaped),
            LirStmt::FieldUpdate { target, value, .. } => {
                mark_atom_escape(target, ctors, escaped);
                mark_atom_escape(value, ctors, escaped);
            }
            LirStmt::If { cond, then_body, else_body, .. } => {
                mark_atom_escape(cond, ctors, escaped);
                mark_escapes_in_stmts(then_body, ctors, escaped);
                mark_escapes_in_stmts(else_body, ctors, escaped);
            }
            LirStmt::IfReturn {
                cond, then_body, then_ret, else_body, else_ret, ..
            } => {
                mark_atom_escape(cond, ctors, escaped);
                mark_escapes_in_stmts(then_body, ctors, escaped);
                mark_opt_atom_escape(then_ret, ctors, escaped);
                mark_escapes_in_stmts(else_body, ctors, escaped);
                mark_opt_atom_escape(else_ret, ctors, escaped);
            }
            LirStmt::Switch { tag, cases, default_body, default_ret, .. } => {
                mark_atom_escape(tag, ctors, escaped);
                for c in cases {
                    mark_escapes_in_stmts(&c.body, ctors, escaped);
                    mark_opt_atom_escape(&c.ret, ctors, escaped);
                }
                mark_escapes_in_stmts(default_body, ctors, escaped);
                mark_opt_atom_escape(default_ret, ctors, escaped);
            }
            LirStmt::TryCatch { body, body_ret, catch_body, catch_ret, .. } => {
                mark_escapes_in_stmts(body, ctors, escaped);
                mark_opt_atom_escape(body_ret, ctors, escaped);
                mark_escapes_in_stmts(catch_body, ctors, escaped);
                mark_opt_atom_escape(catch_ret, ctors, escaped);
            }
            LirStmt::Loop { cond, cond_stmts, body, .. } => {
                mark_atom_escape(cond, ctors, escaped);
                mark_escapes_in_stmts(cond_stmts, ctors, escaped);
                mark_escapes_in_stmts(body, ctors, escaped);
            }
        }
    }
}

fn mark_escapes_in_expr(
    expr: &LirExpr,
    ctors: &HashMap<Symbol, (i64, Vec<LirAtom>)>,
    escaped: &mut HashSet<Symbol>,
) {
    // ObjectTag/ObjectField use the ctor var safely — don't mark as escaped.
    match expr {
        LirExpr::ObjectTag { .. } | LirExpr::ObjectField { .. } => {}
        LirExpr::Atom(a) => mark_atom_escape(a, ctors, escaped),
        LirExpr::Binary { lhs, rhs, .. } => {
            mark_atom_escape(lhs, ctors, escaped);
            mark_atom_escape(rhs, ctors, escaped);
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            for (_, a) in args { mark_atom_escape(a, ctors, escaped); }
        }
        LirExpr::Constructor { args, .. } => {
            for a in args { mark_atom_escape(a, ctors, escaped); }
        }
        LirExpr::Record { fields, .. } => {
            for (_, a) in fields { mark_atom_escape(a, ctors, escaped); }
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => {
            mark_atom_escape(value, ctors, escaped);
        }
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::Closure { captures, .. } => {
            for (_, a) in captures { mark_atom_escape(a, ctors, escaped); }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            mark_atom_escape(callee, ctors, escaped);
            for (_, a) in args { mark_atom_escape(a, ctors, escaped); }
        }
        LirExpr::LazySpawn { thunk, .. } => mark_atom_escape(thunk, ctors, escaped),
        LirExpr::LazyJoin { task_id, .. } => mark_atom_escape(task_id, ctors, escaped),
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                mark_atom_escape(a, ctors, escaped);
            }
        }
    }
}

fn mark_atom_escape(
    atom: &LirAtom,
    ctors: &HashMap<Symbol, (i64, Vec<LirAtom>)>,
    escaped: &mut HashSet<Symbol>,
) {
    if let LirAtom::Var { name, .. } = atom {
        if ctors.contains_key(name) {
            escaped.insert(*name);
        }
    }
}

fn mark_opt_atom_escape(
    atom: &Option<LirAtom>,
    ctors: &HashMap<Symbol, (i64, Vec<LirAtom>)>,
    escaped: &mut HashSet<Symbol>,
) {
    if let Some(a) = atom {
        mark_atom_escape(a, ctors, escaped);
    }
}

/// Replace ObjectTag(ctor_var) → Atom(Int(tag)) and
/// ObjectField(ctor_var, i) → Atom(args[i]) for non-escaped constructors.
fn apply_sra_in_stmts(stmts: &mut [LirStmt], ctors: &HashMap<Symbol, (i64, Vec<LirAtom>)>) {
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::Let { expr, .. } => apply_sra_in_expr(expr, ctors),
            LirStmt::If { then_body, else_body, .. } => {
                apply_sra_in_stmts(then_body, ctors);
                apply_sra_in_stmts(else_body, ctors);
            }
            LirStmt::IfReturn { then_body, else_body, .. } => {
                apply_sra_in_stmts(then_body, ctors);
                apply_sra_in_stmts(else_body, ctors);
            }
            LirStmt::Switch { cases, default_body, .. } => {
                for c in cases.iter_mut() {
                    apply_sra_in_stmts(&mut c.body, ctors);
                }
                apply_sra_in_stmts(default_body, ctors);
            }
            LirStmt::TryCatch { body, catch_body, .. } => {
                apply_sra_in_stmts(body, ctors);
                apply_sra_in_stmts(catch_body, ctors);
            }
            LirStmt::Loop { cond_stmts, body, .. } => {
                apply_sra_in_stmts(cond_stmts, ctors);
                apply_sra_in_stmts(body, ctors);
            }
            _ => {}
        }
    }
}

fn apply_sra_in_expr(expr: &mut LirExpr, ctors: &HashMap<Symbol, (i64, Vec<LirAtom>)>) {
    match expr {
        LirExpr::ObjectTag {
            value: LirAtom::Var { name, .. },
            ..
        } => {
            if let Some((tag, _)) = ctors.get(name) {
                *expr = LirExpr::Atom(LirAtom::Int(*tag));
            }
        }
        LirExpr::ObjectField {
            value: LirAtom::Var { name, .. },
            index,
            ..
        } => {
            if let Some((_, args)) = ctors.get(name) {
                if *index < args.len() {
                    *expr = LirExpr::Atom(args[*index].clone());
                }
            }
        }
        _ => {}
    }
}

// ─── Linear reuse: in-place Constructor update ─────────────────────────────

///   let f0 = ObjectField(src, 0)
///   let f1 = ObjectField(src, 1)
///   let new_val = Call(g, [f0])
///   FieldUpdate(src, offset=8, new_val)   ← only changed field
///   let result = Atom(src)                ← reuse pointer (no alloc)
fn reuse_linear_constructors(stmts: &mut Vec<LirStmt>) {
    reuse_in_stmts(stmts);
}

fn reuse_in_stmts(stmts: &mut Vec<LirStmt>) {
    // First, recurse into sub-bodies so inner scopes are optimized bottom-up.
    for stmt in stmts.iter_mut() {
        match stmt {
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                reuse_in_stmts(then_body);
                reuse_in_stmts(else_body);
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                reuse_in_stmts(then_body);
                reuse_in_stmts(else_body);
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for c in cases.iter_mut() {
                    reuse_in_stmts(&mut c.body);
                }
                reuse_in_stmts(default_body);
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                reuse_in_stmts(body);
                reuse_in_stmts(catch_body);
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                reuse_in_stmts(cond_stmts);
                reuse_in_stmts(body);
            }
            _ => {}
        }
    }

    // Now optimize the flat statement list.
    // Phase 1: collect ObjectField definitions.
    //   var_name → (source_atom, field_index)
    let mut field_defs: HashMap<Symbol, (LirAtom, usize)> = HashMap::new();

    // Phase 2: build the replacement list.
    //   (stmt_index, source_atom, ctor_tag, updates, result_name, result_typ)
    let mut replacements: Vec<(
        usize,
        LirAtom,
        i64,
        Vec<(u64, LirAtom, Type)>,
        Symbol,
        Type,
    )> = Vec::new();

    for (i, stmt) in stmts.iter().enumerate() {
        match stmt {
            LirStmt::Let {
                name,
                expr:
                    LirExpr::ObjectField {
                        value, index, ..
                    },
                ..
            } => {
                field_defs.insert(*name, (value.clone(), *index));
            }
            LirStmt::Let {
                name,
                typ,
                expr:
                    LirExpr::Constructor {
                        name: ctor_name,
                        args,
                        ..
                    },
            } => {
                if let Some(reuse) =
                    try_find_reuse(ctor_name, args, &field_defs, stmts, i)
                {
                    replacements.push((
                        i,
                        reuse.source,
                        reuse.tag,
                        reuse.updates,
                        *name,
                        typ.clone(),
                    ));
                }
            }
            _ => {}
        }
    }

    // Phase 3: apply replacements in reverse order to preserve indices.
    for (idx, source, tag, updates, result_name, result_typ) in replacements.into_iter().rev() {
        // Remove the original Constructor Let.
        stmts.remove(idx);

        // Insert: tag write + field updates + let result = Atom(source)
        let mut insert_pos = idx;

        // Write the tag (always, for safety — handles cross-constructor reuse).
        stmts.insert(
            insert_pos,
            LirStmt::FieldUpdate {
                target: source.clone(),
                byte_offset: 0,
                value: LirAtom::Int(tag),
                value_typ: Type::I64,
            },
        );
        insert_pos += 1;

        // Write changed fields.
        for (byte_offset, value, value_typ) in updates {
            stmts.insert(
                insert_pos,
                LirStmt::FieldUpdate {
                    target: source.clone(),
                    byte_offset,
                    value,
                    value_typ,
                },
            );
            insert_pos += 1;
        }

        // Reuse the source pointer.
        stmts.insert(
            insert_pos,
            LirStmt::Let {
                name: result_name,
                typ: result_typ,
                expr: LirExpr::Atom(source),
            },
        );
    }
}

struct ReuseCandidate {
    source: LirAtom,
    tag: i64,
    /// (byte_offset, new_value, value_type) for each changed field.
    updates: Vec<(u64, LirAtom, Type)>,
}

/// Check if a Constructor can reuse memory from an existing heap object.
fn try_find_reuse(
    ctor_name: &Symbol,
    args: &[LirAtom],
    field_defs: &HashMap<Symbol, (LirAtom, usize)>,
    stmts: &[LirStmt],
    ctor_pos: usize,
) -> Option<ReuseCandidate> {
    if args.is_empty() {
        return None; // Zero-arity constructors (Nil, None) — nothing to reuse
    }

    // Find a common source: at least one arg must trace to ObjectField(src, _).
    let mut source: Option<LirAtom> = None;
    let mut source_name: Option<Symbol> = None;
    let mut max_field_idx: usize = 0;

    for arg in args {
        if let LirAtom::Var { name, .. } = arg {
            if let Some((src_atom, _field_idx)) = field_defs.get(name) {
                let src_sym = match src_atom {
                    LirAtom::Var { name: s, .. } => *s,
                    _ => continue,
                };
                match &source_name {
                    None => {
                        source = Some(src_atom.clone());
                        source_name = Some(src_sym);
                    }
                    Some(existing) if *existing == src_sym => {} // same source
                    _ => return None, // different sources — can't reuse one object
                }
            }
        }
    }

    let source = source?;
    let source_sym = source_name?;

    // Verify: all ObjectField extractions from this source have consecutive indices
    // covering [0..arity). This ensures the source has exactly `arity` fields.
    let arity = args.len();
    let mut covered = vec![false; arity];
    for (src_atom, field_idx) in field_defs.values() {
        if let LirAtom::Var { name, .. } = src_atom {
            if *name == source_sym && *field_idx < arity {
                covered[*field_idx] = true;
                if *field_idx > max_field_idx {
                    max_field_idx = *field_idx;
                }
            }
        }
    }
    // All field indices must be covered (source has at least `arity` fields).
    if !covered.iter().all(|c| *c) {
        return None;
    }

    // Safety check: source must not appear in any statement after the Constructor.
    let mut src_used_after = false;
    for stmt in &stmts[ctor_pos + 1..] {
        if atom_mentions_var_in_stmt(stmt, source_sym) {
            src_used_after = true;
            break;
        }
    }
    if src_used_after {
        return None;
    }

    // Build updates: only changed fields.
    let tag = constructor_tag(ctor_name.as_str(), arity);
    let mut updates = Vec::new();
    for (idx, arg) in args.iter().enumerate() {
        let is_unchanged = if let LirAtom::Var { name, .. } = arg {
            field_defs
                .get(name)
                .map_or(false, |(src, fi)| {
                    matches!(src, LirAtom::Var { name: s, .. } if *s == source_sym)
                        && *fi == idx
                })
        } else {
            false
        };
        if !is_unchanged {
            let byte_offset = ((idx + 1) * 8) as u64;
            updates.push((byte_offset, arg.clone(), arg.typ()));
        }
    }

    Some(ReuseCandidate {
        source,
        tag,
        updates,
    })
}

/// Check if a statement (non-recursively into sub-bodies) mentions a variable.
fn atom_mentions_var_in_stmt(stmt: &LirStmt, var: Symbol) -> bool {
    let check = |atom: &LirAtom| matches!(atom, LirAtom::Var { name, .. } if *name == var);
    match stmt {
        LirStmt::Let { expr, .. } => atom_mentions_var_in_expr(expr, var),
        LirStmt::FieldUpdate {
            target, value, ..
        } => check(target) || check(value),
        LirStmt::If { cond, .. }
        | LirStmt::IfReturn { cond, .. } => check(cond),
        LirStmt::Loop { cond, .. } => check(cond),
        LirStmt::Switch { tag, .. } => check(tag),
        LirStmt::TryCatch { .. } => false,
    }
}

fn atom_mentions_var_in_expr(expr: &LirExpr, var: Symbol) -> bool {
    let check = |atom: &LirAtom| matches!(atom, LirAtom::Var { name, .. } if *name == var);
    match expr {
        LirExpr::Atom(a) => check(a),
        LirExpr::Binary { lhs, rhs, .. } => check(lhs) || check(rhs),
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            args.iter().any(|(_, a)| check(a))
        }
        LirExpr::Constructor { args, .. } => args.iter().any(check),
        LirExpr::Record { fields, .. } => fields.iter().any(|(_, a)| check(a)),
        LirExpr::ObjectTag { value, .. }
        | LirExpr::ObjectField { value, .. }
        | LirExpr::Force { value, .. }
        | LirExpr::Raise { value, .. } => check(value),
        LirExpr::Closure { captures, .. } => captures.iter().any(|(_, a)| check(a)),
        LirExpr::ClosureEnvLoad { .. } => false,
        LirExpr::FuncRef { .. } => false,
        LirExpr::CallIndirect { callee, args, .. } => {
            check(callee) || args.iter().any(|(_, a)| check(a))
        }
        LirExpr::LazySpawn { thunk, .. } => check(thunk),
        LirExpr::LazyJoin { task_id, .. } => check(task_id),
        LirExpr::Intrinsic { args, .. } => args.iter().any(|(_, a)| check(a)),
    }
}
