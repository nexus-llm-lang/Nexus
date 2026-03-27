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

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirFunction, LirProgram, LirStmt, SwitchCase};
use crate::types::{BinaryOp, Type};

/// Run all LIR optimization passes on the program (mutates in place).
pub fn optimize_lir(program: &mut LirProgram) {
    // Phase 0: Program-level function inlining (before per-function passes)
    inline_small_functions(program);

    for func in &mut program.functions {
        optimize_function(func);
    }

    // Phase 7: Identical code folding (after all per-function optimizations)
    fold_identical_functions(program);
}

fn optimize_function(func: &mut LirFunction) {
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
            LirStmt::Conc { .. } => {}
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
        LirStmt::Let { .. } | LirStmt::Conc { .. } => {}
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
        LirStmt::Conc { .. } => {}
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
            LirStmt::Let { .. } | LirStmt::Conc { .. } => {}
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
        LirStmt::Conc { tasks } => {
            for task in tasks {
                for (_, arg) in &mut task.args {
                    subst_atom(arg, subst);
                }
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
        LirExpr::Raise { value, .. } => subst_atom(value, subst),
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
        LirStmt::Conc { tasks } => {
            for task in tasks {
                for (_, arg) in &task.args {
                    count_uses_in_atom(arg, uses);
                }
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
        LirExpr::Raise { value, .. } => count_uses_in_atom(value, uses),
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
            LirStmt::If { then_body, else_body, .. }
            | LirStmt::IfReturn { then_body, else_body, .. } => {
                count += count_dead_lets(then_body, uses);
                count += count_dead_lets(else_body, uses);
            }
            LirStmt::TryCatch { body, catch_body, .. } => {
                count += count_dead_lets(body, uses);
                count += count_dead_lets(catch_body, uses);
            }
            LirStmt::Loop { cond_stmts, body, .. } => {
                count += count_dead_lets(cond_stmts, uses);
                count += count_dead_lets(body, uses);
            }
            LirStmt::Switch { cases, default_body, .. } => {
                for case in cases {
                    count += count_dead_lets(&case.body, uses);
                }
                count += count_dead_lets(default_body, uses);
            }
            LirStmt::Conc { .. } => {}
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
            LirStmt::Conc { .. } => {}
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
            | LirExpr::CallIndirect { .. }
    )
}

// ─── Unreachable Code Stripping ─────────────────────────────────────────────

/// Returns true if a statement always diverges (never falls through to the next).
fn stmt_diverges(stmt: &LirStmt) -> bool {
    match stmt {
        LirStmt::Let { expr, .. } => matches!(expr, LirExpr::Raise { .. } | LirExpr::TailCall { .. }),
        LirStmt::IfReturn {
            then_ret,
            else_ret,
            ..
        } => {
            // Diverges only if BOTH branches return (or the then branch returns
            // and there is no else branch — the else IS the continuation)
            then_ret.is_some() && else_ret.is_some()
        }
        LirStmt::Switch {
            cases,
            default_ret,
            ..
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
            LirStmt::Let { .. } | LirStmt::Conc { .. } => {}
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
            LirStmt::Let { .. } | LirStmt::Conc { .. } => {}
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
            LirStmt::Conc { .. } => {}
        }
    }
}

/// Extract let bindings that are loop-invariant from the front of a stmt list.
/// A let is invariant if:
/// - Its expression has no side effects
/// - Its expression doesn't reference any variable in `loop_defs`
/// Stops at the first non-invariant let or non-let statement.
fn extract_invariant_lets(
    stmts: &mut Vec<LirStmt>,
    loop_defs: &HashSet<Symbol>,
) -> Vec<LirStmt> {
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
        LirExpr::Record { fields, .. } => {
            fields.iter().any(|(_, a)| atom_references_any(a, vars))
        }
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            atom_references_any(value, vars)
        }
        LirExpr::Raise { value, .. } => atom_references_any(value, vars),
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => false,
        LirExpr::Closure { captures, .. } => {
            captures.iter().any(|(_, a)| atom_references_any(a, vars))
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            atom_references_any(callee, vars) || args.iter().any(|(_, a)| atom_references_any(a, vars))
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

fn devirtualize_calls_in_stmts(stmts: &mut [LirStmt], funcref_map: &HashMap<Symbol, Symbol>) -> u32 {
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
            LirStmt::Conc { .. } => {}
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
        LirStmt::Conc { tasks } => {
            for task in tasks {
                for (_, arg) in &mut task.args {
                    subst_atom(arg, subst);
                }
            }
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
        LirExpr::Raise { value, .. } => rename_atom(value, map),
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
                    redirect.insert(program.functions[idx].name, program.functions[canon_idx].name);
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
            cond_stmts,
            body,
            ..
        } => {
            hash_stmts(cond_stmts, h);
            hash_stmts(body, h);
        }
        LirStmt::Conc { tasks } => {
            tasks.len().hash(h);
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
        LirExpr::Raise { typ, .. } => format!("{:?}", typ).hash(h),
        LirExpr::FuncRef { func, .. } => func.hash(h),
        LirExpr::Closure { func, captures, .. } => {
            func.hash(h);
            captures.len().hash(h);
        }
        LirExpr::ClosureEnvLoad { index, typ } => {
            index.hash(h);
            format!("{:?}", typ).hash(h);
        }
        LirExpr::CallIndirect { args, typ, callee_type, .. } => {
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

fn exprs_structurally_equal(
    a: &LirExpr,
    b: &LirExpr,
    name_map: &HashMap<Symbol, Symbol>,
) -> bool {
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
                && aa.iter()
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
                && aa.iter()
                    .zip(ab.iter())
                    .all(|(a, b)| atoms_structurally_equal(a, b, name_map))
        }
        (
            LirExpr::ObjectTag {
                value: va,
                typ: ta,
            },
            LirExpr::ObjectTag {
                value: vb,
                typ: tb,
            },
        ) => ta == tb && atoms_structurally_equal(va, vb, name_map),
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

fn atoms_structurally_equal(
    a: &LirAtom,
    b: &LirAtom,
    name_map: &HashMap<Symbol, Symbol>,
) -> bool {
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
            LirStmt::Conc { .. } => {}
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
