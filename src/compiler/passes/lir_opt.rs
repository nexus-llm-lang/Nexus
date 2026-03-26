//! LIR optimization passes.
//!
//! Runs between LIR lowering and WASM codegen:
//! 1. Switch recognition: convert IfReturn chains (from match lowering) to Switch nodes
//! 2. Constant folding: evaluate Binary(Lit, op, Lit) at compile time
//! 3. Copy propagation: replace Let x = Atom(y) by substituting y for x everywhere
//! 4. Dead let elimination: remove Let with unused result and no side effects

use std::collections::HashMap;

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirFunction, LirProgram, LirStmt, SwitchCase};
use crate::types::{BinaryOp, Type};

/// Run all LIR optimization passes on the program (mutates in place).
pub fn optimize_lir(program: &mut LirProgram) {
    for func in &mut program.functions {
        optimize_function(func);
    }
}

fn optimize_function(func: &mut LirFunction) {
    // 1. Recognize IfReturn chains from match lowering → Switch nodes
    recognize_switches_in_stmts(&mut func.body);
    // 2. Constant folding: Binary(Lit, op, Lit) → Atom(Lit)
    constant_fold_stmts(&mut func.body);
    // 3. Copy propagation: Let x = Atom(y) → substitute y for x
    //    Skip variables bound multiple times (match-expr result temps) — not SSA-safe.
    let multi_bound = find_multiply_bound(&func.body);
    let mut subst = HashMap::new();
    copy_propagate_stmts(&mut func.body, &mut subst, &multi_bound);
    subst_atom(&mut func.ret, &subst);
    // 4. Dead let elimination
    let mut uses = HashMap::new();
    count_uses_in_stmts(&func.body, &mut uses);
    count_uses_in_atom(&func.ret, &mut uses);
    eliminate_dead_lets(&mut func.body, &uses);
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
        return fold_binary(*op, lhs, rhs);
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
