//! MIR → LIR (ANF conversion)
//!
//! Flattens complex MIR expressions into ANF form:
//! - All operands become atoms (variables or literals)
//! - Complex expressions are extracted into let-bound temporaries
//! - If/Match compiled into IfReturn chains

use crate::compiler::type_tag::constructor_tag;
use crate::intern::Symbol;
use crate::ir::lir::*;
use crate::ir::mir::*;
use crate::types::{BinaryOp, EnumDef, Literal, Span, Type, WasmRepr};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

#[derive(Debug)]
pub enum LirLowerError {
    UnsupportedExpression { detail: String, span: Span },
    FunctionMayNotReturn { name: String, span: Span },
    UnresolvedType { detail: String, span: Span },
}

impl std::fmt::Display for LirLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LirLowerError::UnsupportedExpression { detail, .. } => {
                write!(f, "Unsupported expression in LIR lowering: {}", detail)
            }
            LirLowerError::FunctionMayNotReturn { name, .. } => {
                write!(f, "Function '{}' may not return a value", name)
            }
            LirLowerError::UnresolvedType { detail, .. } => {
                write!(f, "Unresolved type in LIR lowering: {}", detail)
            }
        }
    }
}

impl LirLowerError {
    pub fn span(&self) -> &Span {
        match self {
            LirLowerError::UnsupportedExpression { span, .. }
            | LirLowerError::FunctionMayNotReturn { span, .. }
            | LirLowerError::UnresolvedType { span, .. } => span,
        }
    }
}

#[tracing::instrument(skip_all, name = "lower_mir_to_lir")]
pub fn lower_mir_to_lir(
    mir: &MirProgram,
    enum_defs: &[EnumDef],
) -> Result<LirProgram, LirLowerError> {
    let mut lowerer = LirLowerer::new(mir, enum_defs);
    let mut program = lowerer.lower()?;

    // Post-lowering: promote Call → TailCall in return positions.
    // Match case bodies emit plain Call instead of TailCall (they go through
    // lower_case_body_stmt, not lower_stmt's Return(Call) path). This pass
    // detects self-recursive calls whose result is immediately returned and
    // promotes them to TailCall for TCO.
    for func in &mut program.functions {
        promote_tail_calls_in_stmts(&mut func.body, &func.name);
    }

    Ok(program)
}

/// Promote Call → TailCall for the last Let in IfReturn/Switch bodies
/// when the result is used as the return value.
fn promote_tail_calls_in_stmts(stmts: &mut [LirStmt], self_name: &Symbol) {
    for stmt in stmts.iter_mut() {
        promote_tail_calls_in_stmt(stmt, self_name);
    }
}

fn promote_tail_calls_in_stmt(stmt: &mut LirStmt, self_name: &Symbol) {
    match stmt {
        LirStmt::IfReturn {
            then_body,
            then_ret,
            else_body,
            else_ret,
            ..
        } => {
            promote_tail_calls_in_stmts(then_body, self_name);
            promote_tail_calls_in_stmts(else_body, self_name);
            promote_last_call_to_tail_call(then_body, then_ret.as_ref(), self_name);
            promote_last_call_to_tail_call(else_body, else_ret.as_ref(), self_name);
        }
        LirStmt::If {
            then_body,
            else_body,
            ..
        } => {
            promote_tail_calls_in_stmts(then_body, self_name);
            promote_tail_calls_in_stmts(else_body, self_name);
        }
        LirStmt::Switch {
            cases,
            default_body,
            default_ret,
            ..
        } => {
            for case in cases.iter_mut() {
                promote_tail_calls_in_stmts(&mut case.body, self_name);
                promote_last_call_to_tail_call(&mut case.body, case.ret.as_ref(), self_name);
            }
            promote_tail_calls_in_stmts(default_body, self_name);
            promote_last_call_to_tail_call(default_body, default_ret.as_ref(), self_name);
        }
        LirStmt::Loop { cond_stmts, body, .. } => {
            promote_tail_calls_in_stmts(cond_stmts, self_name);
            promote_tail_calls_in_stmts(body, self_name);
        }
        LirStmt::TryCatch { body, catch_body, .. } => {
            promote_tail_calls_in_stmts(body, self_name);
            promote_tail_calls_in_stmts(catch_body, self_name);
        }
        _ => {}
    }
}

/// If the last statement in `body` is `Let { name, expr: Call { func: self_name, .. } }`
/// and `ret_atom` is `Some(Var { name: same_name })`, promote Call → TailCall.
/// Only promotes self-recursive calls to enable TCO loop optimization.
fn promote_last_call_to_tail_call(
    body: &mut [LirStmt],
    ret_atom: Option<&LirAtom>,
    self_name: &Symbol,
) {
    let ret_name = match ret_atom {
        Some(LirAtom::Var { name, .. }) => *name,
        _ => return,
    };
    if let Some(LirStmt::Let { name, expr, .. }) = body.last_mut() {
        if *name == ret_name {
            if let LirExpr::Call { func, args, typ } = expr {
                if func == self_name {
                    *expr = LirExpr::TailCall {
                        func: *func,
                        args: std::mem::take(args),
                        typ: std::mem::replace(typ, Type::Unit),
                    };
                }
            }
        }
    }
}

struct LirLowerer<'a> {
    mir: &'a MirProgram,
    enum_defs: &'a [EnumDef],
    task_functions: Vec<LirFunction>,
}

impl<'a> LirLowerer<'a> {
    fn new(mir: &'a MirProgram, enum_defs: &'a [EnumDef]) -> Self {
        LirLowerer {
            mir,
            enum_defs,
            task_functions: Vec::new(),
        }
    }

    fn lower(&mut self) -> Result<LirProgram, LirLowerError> {
        let mut functions = Vec::new();
        for func in &self.mir.functions {
            let (lir_func, task_fns) = lower_mir_function(func, self.enum_defs)?;
            functions.push(lir_func);
            self.task_functions.extend(task_fns);
        }
        functions.append(&mut self.task_functions);

        let externals = self
            .mir
            .externals
            .iter()
            .map(|ext| LirExternal {
                name: ext.name.clone(),
                wasm_module: ext.wasm_module.clone(),
                wasm_name: ext.wasm_name.clone(),
                params: ext
                    .params
                    .iter()
                    .map(|p| LirParam {
                        label: p.label.clone(),
                        name: p.name.clone(),
                        typ: p.typ.clone(),
                    })
                    .collect(),
                ret_type: ext.ret_type.clone(),
                throws: ext.throws.clone(),
            })
            .collect();

        // Closure conversion: transform funcref-target functions for uniform closure calling convention
        closure_convert(&mut functions, &self.mir.functions)?;

        Ok(LirProgram {
            functions,
            externals,
        })
    }
}

/// Lower a single MIR function to LIR, returning the function and any task functions
/// accumulated from conc blocks within its body.
fn lower_mir_function(
    func: &MirFunction,
    enum_defs: &[EnumDef],
) -> Result<(LirFunction, Vec<LirFunction>), LirLowerError> {
    let mut ctx = LowerCtx::new(enum_defs, func.source_file.clone(), func.source_line);

    // Register params in vars (both wasm and semantic types)
    for p in &func.params {
        ctx.vars.insert(p.name.clone(), wasm_type(&p.typ));
        ctx.semantic_vars.insert(p.name.clone(), p.typ.clone());
    }
    // Lower body
    let mut ret_atom = None;
    for stmt in &func.body {
        if let Some(atom) = ctx.lower_stmt(stmt, &func.ret_type)? {
            ret_atom = Some(atom);
            break;
        }
    }

    // Determine return value
    let ret = if let Some(ret) = ret_atom {
        ret
    } else if let Some(ret) = fallback_return_atom_from_terminal_stmt(&ctx.stmts) {
        ret
    } else if matches!(func.ret_type, Type::Unit) {
        LirAtom::Unit
    } else {
        // Function body doesn't explicitly return a value. This happens when
        // a match in tail position has only side-effect cases (then_ret=None).
        // Use a default placeholder — the actual returns are handled by
        // IfReturn statements within the body (nested if/match with returns).
        default_atom_for_type(&wasm_type(&func.ret_type))
    };

    let mut params: Vec<LirParam> = func
        .params
        .iter()
        .map(|p| LirParam {
            label: p.label.clone(),
            name: p.name.clone(),
            typ: p.typ.clone(),
        })
        .collect();
    params.sort_by(|a, b| a.label.cmp(&b.label));

    let task_functions = ctx.task_functions;

    Ok((
        LirFunction {
            name: func.name.clone(),
            params,
            ret_type: func.ret_type.clone(),
            requires: Type::Row(Vec::new(), None),
            throws: Type::Row(Vec::new(), None),
            body: ctx.stmts,
            ret,
            span: func.span.clone(),
            source_file: func.source_file.clone(),
            source_line: func.source_line,
        },
        task_functions,
    ))
}

/// Pre-computed per-constructor lookup data, built once from enum_defs.
struct ConstructorInfo {
    /// Index into enum_defs slice for the owning enum
    enum_idx: usize,
    /// Index into that enum's variants vec
    variant_idx: usize,
    /// Cached field labels: Some(label) or None (positional)
    field_labels: Vec<Option<Symbol>>,
    /// Cached sorted field order (def_idx → sorted_idx), None if not all-labeled or empty
    sorted_indices: Option<Vec<usize>>,
}

/// Context for lowering a single function body
struct LowerCtx<'a> {
    vars: HashMap<Symbol, Type>,
    /// Semantic types for variables (pre-wasm-lowering) — used for field access resolution
    semantic_vars: HashMap<Symbol, Type>,
    stmts: Vec<LirStmt>,
    temp_counter: usize,
    /// Reusable buffer for formatting temp variable names (avoids per-call allocation)
    temp_buf: String,
    /// Task functions lifted from conc blocks
    task_functions: Vec<LirFunction>,
    enum_defs: &'a [EnumDef],
    /// O(1) constructor lookup index: ctor_name → ConstructorInfo
    ctor_index: HashMap<String, ConstructorInfo>,
    source_file: Option<String>,
    source_line: Option<u32>,
}

// ── Maranget decision tree types and construction ────────────────

/// A row in the pattern matrix for decision tree compilation.
struct PatRow {
    pats: Vec<MirPattern>,
    case_idx: usize,
    /// Accumulated variable bindings: (var_name, scrutinee_id).
    bindings: Vec<(Symbol, usize)>,
}

/// Decision tree for compiled pattern matching (Maranget's algorithm).
enum DecTree {
    /// All patterns matched — execute case body with these bindings.
    Leaf {
        case_idx: usize,
        bindings: Vec<(Symbol, usize)>,
    },
    /// Switch on constructor tag.
    CtorSwitch {
        scrutinee_id: usize,
        branches: Vec<CtorBranch>,
        /// Fallback for wildcard/variable rows (None if exhaustive over all constructors).
        fallback: Option<Box<DecTree>>,
    },
    /// Switch on literal value.
    LitSwitch {
        scrutinee_id: usize,
        branches: Vec<(Literal, DecTree)>,
        fallback: Box<DecTree>,
    },
    /// Unconditionally decompose a record into fields (no tag check needed).
    RecordDestructure {
        scrutinee_id: usize,
        /// Sorted field names (determines extraction order).
        field_names: Vec<Symbol>,
        /// Scrutinee IDs for each extracted field.
        field_ids: Vec<usize>,
        subtree: Box<DecTree>,
    },
    /// Unreachable (should not happen with exhaustive patterns).
    Fail,
}

struct CtorBranch {
    ctor_name: Symbol,
    ctor_tag: i64,
    arity: usize,
    /// Scrutinee IDs for the extracted fields (in definition order).
    field_ids: Vec<usize>,
    subtree: DecTree,
}

/// Match emission mode.
enum MatchEmitMode {
    /// Statement position: emit IfReturn chain.
    Stmt,
    /// Expression position: assign result to variable.
    Expr {
        result_name: Symbol,
        result_type: Type,
    },
}

/// Find the leftmost column that has at least one non-wildcard, non-variable pattern.
fn find_active_column(rows: &[PatRow], n_cols: usize) -> Option<usize> {
    for col in 0..n_cols {
        for row in rows {
            match &row.pats[col] {
                MirPattern::Wildcard | MirPattern::Variable(_, _) => {}
                _ => return Some(col),
            }
        }
    }
    None
}

/// Build a decision tree from a pattern matrix using Maranget's algorithm.
fn build_decision_tree(
    rows: Vec<PatRow>,
    col_ids: &[usize],
    next_id: &mut usize,
    ctor_index: &HashMap<String, ConstructorInfo>,
    enum_defs: &[EnumDef],
) -> DecTree {
    if rows.is_empty() {
        return DecTree::Fail;
    }

    let n_cols = col_ids.len();

    // Find active column
    let active_col = find_active_column(&rows, n_cols);

    match active_col {
        None => {
            // First row is all wildcards/variables — it matches unconditionally.
            let row = &rows[0];
            let mut bindings = row.bindings.clone();
            for (col, pat) in row.pats.iter().enumerate() {
                if let MirPattern::Variable(name, _) = pat {
                    bindings.push((*name, col_ids[col]));
                }
            }
            DecTree::Leaf {
                case_idx: row.case_idx,
                bindings,
            }
        }
        Some(col) => {
            // Determine pattern kind in this column
            let first_non_wild = rows.iter().find_map(|r| match &r.pats[col] {
                MirPattern::Wildcard | MirPattern::Variable(_, _) => None,
                other => Some(other),
            });

            match first_non_wild {
                Some(MirPattern::Constructor { .. }) => build_ctor_switch(
                    rows, col, col_ids, next_id, ctor_index, enum_defs,
                ),
                Some(MirPattern::Literal(_)) => build_lit_switch(
                    rows, col, col_ids, next_id, ctor_index, enum_defs,
                ),
                Some(MirPattern::Record(_fields, _)) => {
                    // Collect all field names from record patterns at this column
                    let mut field_set: Vec<Symbol> = Vec::new();
                    for row in &rows {
                        if let MirPattern::Record(fields, _) = &row.pats[col] {
                            for (name, _) in fields {
                                if !field_set.contains(name) {
                                    field_set.push(*name);
                                }
                            }
                        }
                    }
                    field_set.sort();
                    build_record_destructure(
                        rows, col, col_ids, &field_set, next_id, ctor_index, enum_defs,
                    )
                }
                _ => unreachable!("find_active_column returned column with no active pattern"),
            }
        }
    }
}

/// Build a CtorSwitch node at the given column.
fn build_ctor_switch(
    rows: Vec<PatRow>,
    col: usize,
    col_ids: &[usize],
    next_id: &mut usize,
    ctor_index: &HashMap<String, ConstructorInfo>,
    enum_defs: &[EnumDef],
) -> DecTree {
    // Collect unique constructors in order of first appearance
    let mut seen_ctors: Vec<Symbol> = Vec::new();
    for row in &rows {
        if let MirPattern::Constructor { name, .. } = &row.pats[col] {
            if !seen_ctors.contains(name) {
                seen_ctors.push(*name);
            }
        }
    }

    let scrutinee_id = col_ids[col];
    let mut branches = Vec::new();

    for ctor_name in &seen_ctors {
        let ctor_info = ctor_index.get(ctor_name.as_str());
        let arity = ctor_info.map(|ci| ci.field_labels.len()).unwrap_or(0);
        let field_labels = ctor_info.map(|ci| &ci.field_labels);

        // Allocate scrutinee IDs for fields
        let field_ids: Vec<usize> = (0..arity).map(|_| { let id = *next_id; *next_id += 1; id }).collect();

        // Build new column IDs: replace col with field IDs
        let mut new_col_ids: Vec<usize> = col_ids[..col].to_vec();
        new_col_ids.extend_from_slice(&field_ids);
        new_col_ids.extend_from_slice(&col_ids[col + 1..]);

        // Specialize rows for this constructor
        let specialized = specialize_ctor(&rows, col, col_ids, ctor_name, arity, field_labels);

        let def_arity = ctor_info.map(|ci| ci.field_labels.len()).unwrap_or(arity);
        let tag = constructor_tag(ctor_name.as_str(), def_arity);

        let subtree = build_decision_tree(specialized, &new_col_ids, next_id, ctor_index, enum_defs);
        branches.push(CtorBranch {
            ctor_name: *ctor_name,
            ctor_tag: tag,
            arity,
            field_ids,
            subtree,
        });
    }

    // Build default matrix (rows with wildcard/variable at col)
    let default_rows = default_matrix(&rows, col, col_ids);
    let fallback = if default_rows.is_empty() {
        None
    } else {
        let mut new_col_ids: Vec<usize> = col_ids[..col].to_vec();
        new_col_ids.extend_from_slice(&col_ids[col + 1..]);
        Some(Box::new(build_decision_tree(
            default_rows,
            &new_col_ids,
            next_id,
            ctor_index,
            enum_defs,
        )))
    };

    DecTree::CtorSwitch {
        scrutinee_id,
        branches,
        fallback,
    }
}

/// Build a LitSwitch node at the given column.
fn build_lit_switch(
    rows: Vec<PatRow>,
    col: usize,
    col_ids: &[usize],
    next_id: &mut usize,
    ctor_index: &HashMap<String, ConstructorInfo>,
    enum_defs: &[EnumDef],
) -> DecTree {
    let scrutinee_id = col_ids[col];

    // Collect unique literals in order of first appearance
    let mut seen_lits: Vec<Literal> = Vec::new();
    for row in &rows {
        if let MirPattern::Literal(lit) = &row.pats[col] {
            if !seen_lits.contains(lit) {
                seen_lits.push(lit.clone());
            }
        }
    }

    // New col_ids: remove col
    let mut new_col_ids: Vec<usize> = col_ids[..col].to_vec();
    new_col_ids.extend_from_slice(&col_ids[col + 1..]);

    let mut branches = Vec::new();
    for lit in &seen_lits {
        let specialized = specialize_literal(&rows, col, col_ids, lit);
        let subtree = build_decision_tree(specialized, &new_col_ids, next_id, ctor_index, enum_defs);
        branches.push((lit.clone(), subtree));
    }

    let default_rows = default_matrix(&rows, col, col_ids);
    let fallback = build_decision_tree(default_rows, &new_col_ids, next_id, ctor_index, enum_defs);

    DecTree::LitSwitch {
        scrutinee_id,
        branches,
        fallback: Box::new(fallback),
    }
}

/// Build a RecordDestructure node at the given column.
fn build_record_destructure(
    rows: Vec<PatRow>,
    col: usize,
    col_ids: &[usize],
    field_names: &[Symbol],
    next_id: &mut usize,
    ctor_index: &HashMap<String, ConstructorInfo>,
    enum_defs: &[EnumDef],
) -> DecTree {
    let scrutinee_id = col_ids[col];
    let k = field_names.len();

    // Allocate scrutinee IDs for fields
    let field_ids: Vec<usize> = (0..k).map(|_| { let id = *next_id; *next_id += 1; id }).collect();

    // New col_ids: replace col with field IDs
    let mut new_col_ids: Vec<usize> = col_ids[..col].to_vec();
    new_col_ids.extend_from_slice(&field_ids);
    new_col_ids.extend_from_slice(&col_ids[col + 1..]);

    // Specialize each row: replace Record pattern with sub-patterns in field_names order
    let mut specialized: Vec<PatRow> = Vec::new();
    for row in &rows {
        match &row.pats[col] {
            MirPattern::Record(fields, _) => {
                // Build sub-patterns in field_names order
                let mut sub_pats: Vec<MirPattern> = vec![MirPattern::Wildcard; k];
                for (name, sub_pat) in fields {
                    if let Some(pos) = field_names.iter().position(|n| n == name) {
                        sub_pats[pos] = sub_pat.clone();
                    }
                }
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(sub_pats);
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                specialized.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Wildcard => {
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(std::iter::repeat(MirPattern::Wildcard).take(k));
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                specialized.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Variable(name, _) => {
                let mut bindings = row.bindings.clone();
                bindings.push((*name, col_ids[col]));
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(std::iter::repeat(MirPattern::Wildcard).take(k));
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                specialized.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings,
                });
            }
            _ => {} // Other patterns at a record column — skip
        }
    }

    let subtree = build_decision_tree(specialized, &new_col_ids, next_id, ctor_index, enum_defs);

    DecTree::RecordDestructure {
        scrutinee_id,
        field_names: field_names.to_vec(),
        field_ids,
        subtree: Box::new(subtree),
    }
}

/// Specialize the pattern matrix for a specific constructor at column `col`.
fn specialize_ctor(
    rows: &[PatRow],
    col: usize,
    col_ids: &[usize],
    ctor_name: &Symbol,
    arity: usize,
    field_labels: Option<&Vec<Option<Symbol>>>,
) -> Vec<PatRow> {
    let mut result = Vec::new();
    for row in rows {
        match &row.pats[col] {
            MirPattern::Constructor { name, fields } if name == ctor_name => {
                // Reorder sub-patterns to definition order
                let mut sub_pats: Vec<MirPattern> = vec![MirPattern::Wildcard; arity];
                for (pat_idx, (label, sub_pat)) in fields.iter().enumerate() {
                    let def_idx = if let Some(lbl) = label {
                        field_labels
                            .and_then(|labels| labels.iter().position(|l| l.as_ref() == Some(lbl)))
                            .unwrap_or(pat_idx)
                    } else {
                        pat_idx
                    };
                    if def_idx < arity {
                        sub_pats[def_idx] = sub_pat.clone();
                    }
                }
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(sub_pats);
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Wildcard => {
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(std::iter::repeat(MirPattern::Wildcard).take(arity));
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Variable(name, _) => {
                let mut bindings = row.bindings.clone();
                bindings.push((*name, col_ids[col]));
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(std::iter::repeat(MirPattern::Wildcard).take(arity));
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings,
                });
            }
            _ => {} // Different constructor — skip
        }
    }
    result
}

/// Specialize the pattern matrix for a specific literal at column `col`.
fn specialize_literal(
    rows: &[PatRow],
    col: usize,
    col_ids: &[usize],
    lit: &Literal,
) -> Vec<PatRow> {
    let mut result = Vec::new();
    for row in rows {
        match &row.pats[col] {
            MirPattern::Literal(l) if l == lit => {
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Wildcard => {
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Variable(name, _) => {
                let mut bindings = row.bindings.clone();
                bindings.push((*name, col_ids[col]));
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings,
                });
            }
            _ => {} // Different literal — skip
        }
    }
    result
}

/// Build the default matrix: rows with wildcard/variable at column `col`.
fn default_matrix(rows: &[PatRow], col: usize, col_ids: &[usize]) -> Vec<PatRow> {
    let mut result = Vec::new();
    for row in rows {
        match &row.pats[col] {
            MirPattern::Wildcard => {
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings: row.bindings.clone(),
                });
            }
            MirPattern::Variable(name, _) => {
                let mut bindings = row.bindings.clone();
                bindings.push((*name, col_ids[col]));
                let mut new_pats: Vec<MirPattern> = row.pats[..col].to_vec();
                new_pats.extend(row.pats[col + 1..].iter().cloned());
                result.push(PatRow {
                    pats: new_pats,
                    case_idx: row.case_idx,
                    bindings,
                });
            }
            _ => {} // Constructor/Literal/Record — skip in default
        }
    }
    result
}

impl<'a> LowerCtx<'a> {
    fn new(
        enum_defs: &'a [EnumDef],
        source_file: Option<String>,
        source_line: Option<u32>,
    ) -> Self {
        LowerCtx {
            vars: HashMap::new(),
            semantic_vars: HashMap::new(),
            stmts: Vec::new(),
            temp_counter: 0,
            temp_buf: String::with_capacity(16),
            task_functions: Vec::new(),
            ctor_index: build_constructor_index(enum_defs),
            enum_defs,
            source_file,
            source_line,
        }
    }

    fn new_temp(&mut self) -> Symbol {
        self.temp_buf.clear();
        write!(self.temp_buf, "__t{}", self.temp_counter).unwrap();
        self.temp_counter += 1;
        Symbol::intern(&self.temp_buf)
    }

    /// Bind a complex expression to a temporary variable, returning an atom reference
    fn bind_expr_to_temp(&mut self, expr: LirExpr, typ: Type) -> LirAtom {
        let name = self.new_temp();
        self.vars.insert(name, typ.clone());
        self.stmts.push(LirStmt::Let {
            name,
            typ: typ.clone(),
            expr,
        });
        LirAtom::Var { name, typ }
    }

    /// Like bind_expr_to_temp but also registers the semantic type for pattern matching.
    fn bind_expr_to_temp_semantic(
        &mut self,
        expr: LirExpr,
        typ: Type,
        semantic_type: Type,
    ) -> LirAtom {
        let name = self.new_temp();
        self.vars.insert(name, typ.clone());
        self.semantic_vars.insert(name, semantic_type);
        self.stmts.push(LirStmt::Let {
            name,
            typ: typ.clone(),
            expr,
        });
        LirAtom::Var { name, typ }
    }

    /// Lower a MIR statement, optionally returning an atom for explicit returns
    fn lower_stmt(
        &mut self,
        stmt: &MirStmt,
        ret_type: &Type,
    ) -> Result<Option<LirAtom>, LirLowerError> {
        match stmt {
            MirStmt::Let { name, typ, expr } => {
                // Infer semantic type from expression BEFORE lowering
                // (MIR typ is often None/Unit for unannotated lets)
                let pre_semantic_type = {
                    let expr_inferred = self.infer_semantic_type(expr);
                    if matches!(typ, Type::Unit) || matches!(typ, Type::I64) {
                        expr_inferred
                    } else {
                        typ.clone()
                    }
                };
                let atom = self.lower_expr_to_atom(expr)?;
                let inferred = atom.typ();
                // After lowering, the result atom (if a temp var from match/if expr)
                // may have a more accurate semantic type set by lower_match_expr/lower_if_expr.
                // Prefer that over the pre-lowering inference which can't resolve pattern bindings.
                let semantic_type = match &atom {
                    LirAtom::Var { name: var_name, .. } => self
                        .semantic_vars
                        .get(var_name)
                        .cloned()
                        .unwrap_or(pre_semantic_type),
                    _ => pre_semantic_type,
                };
                self.vars.insert(name.clone(), inferred.clone());
                self.semantic_vars.insert(name.clone(), semantic_type);
                self.stmts.push(LirStmt::Let {
                    name: name.clone(),
                    typ: inferred,
                    expr: LirExpr::Atom(atom),
                });
                Ok(None)
            }
            MirStmt::Expr(expr) => match expr {
                MirExpr::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    self.lower_if_stmt(cond, then_body, else_body.as_deref(), ret_type)?;
                    Ok(None)
                }
                MirExpr::Match { target, cases } => {
                    self.lower_match_stmt(target, cases, ret_type)?;
                    Ok(None)
                }
                MirExpr::While { cond, body } => {
                    self.lower_while_stmt(cond, body, ret_type)?;
                    Ok(None)
                }
                _ => {
                    let _atom = self.lower_expr_to_atom(expr)?;
                    Ok(None)
                }
            },
            MirStmt::Return(expr) => {
                if let MirExpr::Call {
                    func,
                    args,
                    ret_type,
                } = expr
                {
                    let mut lir_args = Vec::new();
                    for (label, e) in args {
                        let atom = self.lower_expr_to_atom(e)?;
                        lir_args.push((label.clone(), atom));
                    }
                    lir_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let typ = wasm_type(ret_type);
                    let atom = self.bind_expr_to_temp(
                        LirExpr::TailCall {
                            func: func.clone(),
                            args: lir_args,
                            typ: typ.clone(),
                        },
                        typ,
                    );
                    return Ok(Some(atom));
                }
                let atom = self.lower_expr_to_atom(expr)?;
                Ok(Some(atom))
            }
            MirStmt::Assign { target, value } => {
                // Lower both sides to atoms
                let target_atom = self.lower_expr_to_atom(target)?;
                let value_atom = self.lower_expr_to_atom(value)?;
                // Emit as a let re-binding (ANF doesn't have mutation)
                if let LirAtom::Var { name, typ } = target_atom {
                    self.stmts.push(LirStmt::Let {
                        name,
                        typ,
                        expr: LirExpr::Atom(value_atom),
                    });
                }
                Ok(None)
            }
            MirStmt::Conc(tasks) => {
                let mut task_refs = Vec::new();
                for task in tasks {
                    let free_vars = collect_free_vars(&task.body);
                    let task_name = Symbol::from(format!("__conc_{}", task.name));
                    let capture_params: Vec<MirParam> = free_vars
                        .iter()
                        .map(|&name| {
                            let typ = self.semantic_vars.get(&name).cloned().ok_or_else(|| {
                                LirLowerError::UnresolvedType {
                                    detail: format!(
                                        "conc capture variable '{}' not in semantic_vars",
                                        name
                                    ),
                                    span: task.span.clone(),
                                }
                            })?;
                            Ok(MirParam {
                                label: name,
                                name,
                                typ,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let task_mir = MirFunction {
                        name: task_name,
                        params: capture_params,
                        ret_type: Type::Unit,
                        body: task.body.clone(),
                        span: task.span.clone(),
                        source_file: self.source_file.clone(),
                        source_line: self.source_line,
                        captures: vec![],
                    };
                    let (lir_func, nested_tasks) = lower_mir_function(&task_mir, self.enum_defs)?;
                    self.task_functions.push(lir_func);
                    self.task_functions.extend(nested_tasks);
                    let args: Vec<(Symbol, LirAtom)> = free_vars
                        .iter()
                        .map(|&name| {
                            let typ = self.vars.get(&name).cloned().ok_or_else(|| {
                                LirLowerError::UnresolvedType {
                                    detail: format!("conc capture variable '{}' not in vars", name),
                                    span: task.span.clone(),
                                }
                            })?;
                            Ok((name, LirAtom::Var { name, typ }))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    task_refs.push(ConcTask {
                        func_name: task_name,
                        args,
                    });
                }
                self.stmts.push(LirStmt::Conc { tasks: task_refs });
                Ok(None)
            }
            MirStmt::Try {
                body,
                catch_param,
                catch_body,
            } => {
                let (body_stmts, body_ret) = self.lower_block(body, ret_type)?;
                let mut catch_vars = self.vars.clone();
                catch_vars.insert(catch_param.clone(), Type::String);
                let mut catch_sem_vars = self.semantic_vars.clone();
                catch_sem_vars.insert(catch_param.clone(), Type::String);
                let (catch_stmts, catch_ret) =
                    self.lower_block_with_vars(catch_body, ret_type, catch_vars, catch_sem_vars)?;

                self.stmts.push(LirStmt::TryCatch {
                    body: body_stmts,
                    body_ret,
                    catch_param: catch_param.clone(),
                    catch_param_typ: Type::String,
                    catch_body: catch_stmts,
                    catch_ret,
                });
                Ok(None)
            }
        }
    }

    /// Lower an if statement (possibly producing IfReturn)
    fn lower_if_stmt(
        &mut self,
        cond: &MirExpr,
        then_body: &[MirStmt],
        else_body: Option<&[MirStmt]>,
        ret_type: &Type,
    ) -> Result<(), LirLowerError> {
        let cond_atom = self.lower_expr_to_atom(cond)?;
        let (then_stmts, then_ret) = self.lower_block(then_body, ret_type)?;

        if let Some(else_body) = else_body {
            let (else_stmts, else_ret) = self.lower_block(else_body, ret_type)?;

            if then_ret.is_some() || else_ret.is_some() {
                self.stmts.push(LirStmt::IfReturn {
                    cond: cond_atom,
                    then_body: then_stmts,
                    then_ret: Some(then_ret.unwrap_or_else(|| default_atom_for_type(ret_type))),
                    else_body: else_stmts,
                    else_ret,
                    ret_type: ret_type.clone(),
                });
            } else {
                self.stmts.push(LirStmt::If {
                    cond: cond_atom,
                    then_body: then_stmts,
                    else_body: else_stmts,
                });
            }
        } else if then_ret.is_some() {
            // then branch returns — need IfReturn even without else
            self.stmts.push(LirStmt::IfReturn {
                cond: cond_atom,
                then_body: then_stmts,
                then_ret: Some(then_ret.unwrap_or_else(|| default_atom_for_type(ret_type))),
                else_body: Vec::new(),
                else_ret: None,
                ret_type: ret_type.clone(),
            });
        } else {
            self.stmts.push(LirStmt::If {
                cond: cond_atom,
                then_body: then_stmts,
                else_body: Vec::new(),
            });
        }
        Ok(())
    }

    /// Lower an if expression in atom position (as a value).
    /// Creates a temp variable, assigns in each branch, returns the temp.
    fn lower_if_expr(
        &mut self,
        cond: &MirExpr,
        then_body: &[MirStmt],
        else_body: Option<&[MirStmt]>,
    ) -> Result<LirAtom, LirLowerError> {
        let cond_atom = self.lower_expr_to_atom(cond)?;

        // Infer result type from the then branch's last statement
        let semantic_result_type = if let Some(last) = then_body.last() {
            self.infer_stmt_type(last)
        } else {
            Type::Unit
        };
        let result_type = wasm_type(&semantic_result_type);

        let result_name = self.new_temp();
        self.vars.insert(result_name.clone(), result_type.clone());
        self.semantic_vars
            .insert(result_name.clone(), semantic_result_type);

        // Initialize with placeholder
        self.stmts.push(LirStmt::Let {
            name: result_name.clone(),
            typ: result_type.clone(),
            expr: LirExpr::Atom(default_atom_for_type(&result_type)),
        });

        // Lower branches directly — do NOT use promote_last_expr_to_return, which
        // creates synthetic Return stmts that trigger TailCall optimization and
        // incorrectly return from the enclosing function instead of just producing
        // a value for the if-expression.
        let then_stmts = self.lower_branch_for_value(then_body, result_name, &result_type)?;

        let else_stmts = if let Some(else_body) = else_body {
            self.lower_branch_for_value(else_body, result_name, &result_type)?
        } else {
            Vec::new()
        };

        self.stmts.push(LirStmt::If {
            cond: cond_atom,
            then_body: then_stmts,
            else_body: else_stmts,
        });

        Ok(LirAtom::Var {
            name: result_name,
            typ: result_type,
        })
    }

    /// Lower a branch body (then/else) for value extraction without synthetic Returns.
    /// Lowers all statements, then assigns the last expression's value to `result_name`.
    fn lower_branch_for_value(
        &mut self,
        body: &[MirStmt],
        result_name: Symbol,
        result_type: &Type,
    ) -> Result<Vec<LirStmt>, LirLowerError> {
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = self.vars.clone();
        let saved_semantic_vars = self.semantic_vars.clone();

        if !body.is_empty() {
            let last_idx = body.len() - 1;
            for stmt in &body[..last_idx] {
                self.lower_stmt(stmt, &Type::Unit)?;
            }
            // Extract the value from the last statement
            let value = match &body[last_idx] {
                MirStmt::Expr(expr) => Some(self.lower_expr_to_atom(expr)?),
                MirStmt::Return(expr) => Some(self.lower_expr_to_atom(expr)?),
                _ => {
                    self.lower_stmt(&body[last_idx], &Type::Unit)?;
                    None
                }
            };
            if let Some(val) = value {
                self.stmts.push(LirStmt::Let {
                    name: result_name,
                    typ: result_type.clone(),
                    expr: LirExpr::Atom(val),
                });
            }
        }

        let branch_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
        self.vars = saved_vars;
        self.semantic_vars = saved_semantic_vars;
        Ok(branch_stmts)
    }

    /// Lower a match statement using Maranget's decision tree algorithm.
    /// Builds a decision tree from the pattern matrix, then emits LIR.
    /// Field extractions only occur inside branches where the tag check passed,
    /// structurally preventing out-of-bounds memory access.
    fn lower_match_stmt(
        &mut self,
        target: &MirExpr,
        cases: &[MirMatchCase],
        ret_type: &Type,
    ) -> Result<(), LirLowerError> {
        let target_atom = self.lower_expr_to_atom(target)?;
        if cases.is_empty() {
            return Ok(());
        }

        let target_sem_type = match &target_atom {
            LirAtom::Var { name, .. } => self
                .semantic_vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| target_atom.typ()),
            _ => target_atom.typ(),
        };

        // Build pattern matrix and decision tree
        let rows: Vec<PatRow> = cases
            .iter()
            .enumerate()
            .map(|(i, case)| PatRow {
                pats: vec![case.pattern.clone()],
                case_idx: i,
                bindings: vec![],
            })
            .collect();
        let col_ids = vec![0usize];
        let mut next_id = 1usize;
        let tree =
            build_decision_tree(rows, &col_ids, &mut next_id, &self.ctor_index, self.enum_defs);

        // Emit the decision tree as LIR
        let mut atoms: HashMap<usize, (LirAtom, Type)> = HashMap::new();
        atoms.insert(0, (target_atom, target_sem_type));
        self.emit_decision_tree(&tree, &mut atoms, cases, ret_type, &MatchEmitMode::Stmt)?;

        Ok(())
    }

    /// Lower a while loop into a LirStmt::Loop
    fn lower_while_stmt(
        &mut self,
        cond: &MirExpr,
        body: &[MirStmt],
        ret_type: &Type,
    ) -> Result<(), LirLowerError> {
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = self.vars.clone();
        let saved_sem_vars = self.semantic_vars.clone();

        // Lower condition (produces temp stmts + a condition atom)
        let cond_atom = self.lower_expr_to_atom(cond)?;
        // Negate: break when condition is false
        let neg_cond = self.bind_expr_to_temp(
            LirExpr::Binary {
                op: BinaryOp::Eq,
                lhs: cond_atom,
                rhs: LirAtom::Bool(false),
                typ: Type::Bool,
            },
            Type::Bool,
        );
        let cond_stmts = std::mem::take(&mut self.stmts);

        // Lower body
        for stmt in body {
            self.lower_stmt(stmt, ret_type)?;
        }
        let body_stmts = std::mem::replace(&mut self.stmts, saved_stmts);

        self.vars = saved_vars;
        self.semantic_vars = saved_sem_vars;

        self.stmts.push(LirStmt::Loop {
            cond_stmts,
            cond: neg_cond,
            body: body_stmts,
        });
        Ok(())
    }

    /// Lower a match expression to an atom (for use in expression position).
    /// Creates a temp result variable and assigns to it in each branch.
    /// Lower a match expression using Maranget's decision tree algorithm.
    /// Creates a temp result variable and assigns to it in each branch.
    fn lower_match_expr(
        &mut self,
        target: &MirExpr,
        cases: &[MirMatchCase],
    ) -> Result<LirAtom, LirLowerError> {
        let target_atom = self.lower_expr_to_atom(target)?;
        if cases.is_empty() {
            return Ok(LirAtom::Unit);
        }

        let target_sem_type = match &target_atom {
            LirAtom::Var { name, .. } => self
                .semantic_vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| target_atom.typ()),
            _ => target_atom.typ(),
        };

        // Infer result type from case bodies
        let semantic_result_type = cases
            .iter()
            .find_map(|case| {
                if let Some(last) = case.body.last() {
                    let t = self.infer_stmt_type(last);
                    if matches!(t, Type::Unit) {
                        None
                    } else {
                        Some(t)
                    }
                } else {
                    None
                }
            })
            .unwrap_or(Type::Unit);
        let result_type = wasm_type(&semantic_result_type);

        let result_name = self.new_temp();
        self.vars.insert(result_name.clone(), result_type.clone());
        self.semantic_vars
            .insert(result_name.clone(), semantic_result_type);
        self.stmts.push(LirStmt::Let {
            name: result_name.clone(),
            typ: result_type.clone(),
            expr: LirExpr::Atom(default_atom_for_type(&result_type)),
        });

        // Build pattern matrix and decision tree
        let rows: Vec<PatRow> = cases
            .iter()
            .enumerate()
            .map(|(i, case)| PatRow {
                pats: vec![case.pattern.clone()],
                case_idx: i,
                bindings: vec![],
            })
            .collect();
        let col_ids = vec![0usize];
        let mut next_id = 1usize;
        let tree =
            build_decision_tree(rows, &col_ids, &mut next_id, &self.ctor_index, self.enum_defs);

        // Emit the decision tree as LIR
        let mut atoms: HashMap<usize, (LirAtom, Type)> = HashMap::new();
        atoms.insert(0, (target_atom, target_sem_type));
        self.emit_decision_tree(
            &tree,
            &mut atoms,
            cases,
            &result_type,
            &MatchEmitMode::Expr {
                result_name: result_name.clone(),
                result_type: result_type.clone(),
            },
        )?;

        Ok(LirAtom::Var {
            name: result_name,
            typ: result_type,
        })
    }

    // ── Decision tree emission ─────────────────────────────────────

    /// Emit a decision tree as LIR statements.
    /// `atoms` maps scrutinee IDs to their current (atom, semantic_type) pairs.
    fn emit_decision_tree(
        &mut self,
        tree: &DecTree,
        atoms: &mut HashMap<usize, (LirAtom, Type)>,
        cases: &[MirMatchCase],
        ret_type: &Type,
        mode: &MatchEmitMode,
    ) -> Result<(), LirLowerError> {
        match tree {
            DecTree::Fail => {
                // Unreachable for exhaustive matches — emit nothing
                Ok(())
            }
            DecTree::Leaf {
                case_idx,
                bindings,
            } => {
                self.emit_leaf(*case_idx, bindings, atoms, cases, ret_type, mode)
            }
            DecTree::CtorSwitch {
                scrutinee_id,
                branches,
                fallback,
            } => {
                self.emit_ctor_switch(
                    *scrutinee_id,
                    branches,
                    fallback.as_deref(),
                    atoms,
                    cases,
                    ret_type,
                    mode,
                )
            }
            DecTree::LitSwitch {
                scrutinee_id,
                branches,
                fallback,
            } => {
                self.emit_lit_switch(
                    *scrutinee_id,
                    branches,
                    fallback,
                    atoms,
                    cases,
                    ret_type,
                    mode,
                )
            }
            DecTree::RecordDestructure {
                scrutinee_id,
                field_names,
                field_ids,
                subtree,
            } => {
                self.emit_record_destructure(
                    *scrutinee_id,
                    field_names,
                    field_ids,
                    subtree,
                    atoms,
                    cases,
                    ret_type,
                    mode,
                )
            }
        }
    }

    /// Emit a Leaf node: variable bindings + case body.
    fn emit_leaf(
        &mut self,
        case_idx: usize,
        bindings: &[(Symbol, usize)],
        atoms: &HashMap<usize, (LirAtom, Type)>,
        cases: &[MirMatchCase],
        ret_type: &Type,
        mode: &MatchEmitMode,
    ) -> Result<(), LirLowerError> {
        let case = &cases[case_idx];

        // Set up scoped vars with bindings
        let mut case_vars = self.vars.clone();
        let mut case_sem_vars = self.semantic_vars.clone();
        for (name, scr_id) in bindings {
            if let Some((atom, sem_type)) = atoms.get(scr_id) {
                case_vars.insert(*name, atom.typ());
                case_sem_vars.insert(*name, sem_type.clone());
            }
        }

        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = std::mem::replace(&mut self.vars, case_vars);
        let saved_sem = std::mem::replace(&mut self.semantic_vars, case_sem_vars);

        // Emit binding lets
        for (name, scr_id) in bindings {
            if let Some((atom, _)) = atoms.get(scr_id) {
                self.stmts.push(LirStmt::Let {
                    name: *name,
                    typ: atom.typ(),
                    expr: LirExpr::Atom(atom.clone()),
                });
            }
        }

        match mode {
            MatchEmitMode::Stmt => {
                // Lower body in statement position
                let case_ret = self.lower_case_body_stmt(&case.body, ret_type)?;
                let leaf_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
                self.vars = saved_vars;
                self.semantic_vars = saved_sem;

                // Determine if this is a side-effect-only case
                let body_is_trivially_unit = case.body.is_empty()
                    || case
                        .body
                        .iter()
                        .all(|s| matches!(s, MirStmt::Expr(MirExpr::Literal(Literal::Unit))));

                let genuine_ret = case_ret
                    .or_else(|| fallback_return_atom_from_terminal_stmt(&leaf_stmts));

                if body_is_trivially_unit && genuine_ret.is_none() {
                    // Side-effect only — just emit the stmts inline
                    self.stmts.extend(leaf_stmts);
                } else {
                    // Has a return value — emit IfReturn with Bool(true) as condition
                    // (the branching is handled by the parent CtorSwitch/LitSwitch)
                    let then_ret = genuine_ret.map(|ret| {
                        if matches!(ret.typ(), Type::Unit) && !matches!(ret_type, Type::Unit) {
                            default_atom_for_type(ret_type)
                        } else {
                            ret
                        }
                    });
                    self.stmts.push(LirStmt::IfReturn {
                        cond: LirAtom::Bool(true),
                        then_body: leaf_stmts,
                        then_ret,
                        else_body: vec![],
                        else_ret: None,
                        ret_type: ret_type.clone(),
                    });
                }
            }
            MatchEmitMode::Expr {
                result_name,
                result_type,
            } => {
                // Lower body for value extraction
                let value = self.lower_case_body_expr(&case.body)?;
                // Assign value to result
                self.stmts.push(LirStmt::Let {
                    name: *result_name,
                    typ: result_type.clone(),
                    expr: LirExpr::Atom(value),
                });

                let leaf_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
                self.vars = saved_vars;
                self.semantic_vars = saved_sem;
                self.stmts.extend(leaf_stmts);
            }
        }
        Ok(())
    }

    /// Lower a case body in statement position.
    /// Returns the return atom if the body produces a value.
    fn lower_case_body_stmt(
        &mut self,
        body: &[MirStmt],
        ret_type: &Type,
    ) -> Result<Option<LirAtom>, LirLowerError> {
        if body.is_empty() {
            return Ok(None);
        }
        let last_idx = body.len() - 1;
        for stmt in &body[..last_idx] {
            self.lower_stmt(stmt, ret_type)?;
        }
        match &body[last_idx] {
            MirStmt::Expr(expr) => match expr {
                MirExpr::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    self.lower_if_stmt(cond, then_body, else_body.as_deref(), ret_type)?;
                    Ok(None)
                }
                MirExpr::Match { target, cases } => {
                    self.lower_match_stmt(target, cases, ret_type)?;
                    Ok(None)
                }
                MirExpr::While { cond, body: wb } => {
                    self.lower_while_stmt(cond, wb, ret_type)?;
                    Ok(None)
                }
                _ => {
                    let atom = self.lower_expr_to_atom(expr)?;
                    if matches!(atom.typ(), Type::Unit) {
                        Ok(None)
                    } else {
                        Ok(Some(atom))
                    }
                }
            },
            MirStmt::Return(expr) => {
                let atom = self.lower_expr_to_atom(expr)?;
                if matches!(atom.typ(), Type::Unit) {
                    Ok(None)
                } else {
                    Ok(Some(atom))
                }
            }
            _ => {
                self.lower_stmt(&body[last_idx], ret_type)?;
                Ok(None)
            }
        }
    }

    /// Lower a case body in expression position.
    /// Returns the value atom.
    fn lower_case_body_expr(
        &mut self,
        body: &[MirStmt],
    ) -> Result<LirAtom, LirLowerError> {
        if body.is_empty() {
            return Ok(LirAtom::Unit);
        }
        let last_idx = body.len() - 1;
        for stmt in &body[..last_idx] {
            self.lower_stmt(stmt, &Type::Unit)?;
        }
        match &body[last_idx] {
            MirStmt::Expr(expr) => self.lower_expr_to_atom(expr),
            MirStmt::Return(expr) => self.lower_expr_to_atom(expr),
            _ => {
                self.lower_stmt(&body[last_idx], &Type::Unit)?;
                Ok(LirAtom::Unit)
            }
        }
    }

    /// Emit a CtorSwitch: tag extraction + IfReturn/If chain.
    fn emit_ctor_switch(
        &mut self,
        scrutinee_id: usize,
        branches: &[CtorBranch],
        fallback: Option<&DecTree>,
        atoms: &mut HashMap<usize, (LirAtom, Type)>,
        cases: &[MirMatchCase],
        ret_type: &Type,
        mode: &MatchEmitMode,
    ) -> Result<(), LirLowerError> {
        let (scr_atom, scr_sem_type) = atoms[&scrutinee_id].clone();

        // Extract tag once
        let tag_atom = self.bind_expr_to_temp(
            LirExpr::ObjectTag {
                value: scr_atom.clone(),
                typ: Type::I64,
            },
            Type::I64,
        );

        // Pre-compute conditions for all branches
        let n = branches.len();
        let has_fallback = fallback.is_some();
        let mut branch_conds: Vec<LirAtom> = Vec::new();
        for (i, branch) in branches.iter().enumerate() {
            let is_last_no_fb = i == n - 1 && !has_fallback;
            if is_last_no_fb {
                branch_conds.push(LirAtom::Bool(true));
            } else {
                let cond = self.bind_expr_to_temp(
                    LirExpr::Binary {
                        op: BinaryOp::Eq,
                        lhs: tag_atom.clone(),
                        rhs: LirAtom::Int(branch.ctor_tag),
                        typ: Type::Bool,
                    },
                    Type::Bool,
                );
                branch_conds.push(cond);
            }
        }

        // Build chain in reverse
        // Start with fallback as the innermost else
        let mut chain_else: Vec<LirStmt> = Vec::new();
        if let Some(fb) = fallback {
            let saved = std::mem::take(&mut self.stmts);
            self.emit_decision_tree(fb, atoms, cases, ret_type, mode)?;
            chain_else = std::mem::replace(&mut self.stmts, saved);
        }

        for (_i, (branch, cond)) in branches.iter().zip(branch_conds.iter()).enumerate().rev() {
            // Emit field extractions + subtree in isolated stmts
            let saved = std::mem::take(&mut self.stmts);

            // Extract fields
            let resolved_fts = resolve_constructor_field_types(
                branch.ctor_name.as_str(),
                &scr_sem_type,
                &self.ctor_index,
                self.enum_defs,
            );
            let ctor_info_data = self.ctor_index.get(branch.ctor_name.as_str()).map(|ci| {
                (ci.sorted_indices.clone(), ci.field_labels.clone())
            });

            for fi in 0..branch.arity {
                let mem_idx = ctor_info_data
                    .as_ref()
                    .and_then(|(si, _)| si.as_ref())
                    .map(|si| si[fi])
                    .unwrap_or(fi);
                let sem_ft = resolved_fts
                    .as_ref()
                    .and_then(|fts| fts.get(fi))
                    .cloned()
                    .unwrap_or(Type::I64);
                let wasm_ft = wasm_type(&sem_ft);
                let field_atom = self.bind_expr_to_temp(
                    LirExpr::ObjectField {
                        value: scr_atom.clone(),
                        index: mem_idx,
                        typ: wasm_ft.clone(),
                    },
                    wasm_ft,
                );
                if let LirAtom::Var { name, .. } = &field_atom {
                    self.semantic_vars.insert(*name, sem_ft.clone());
                }
                atoms.insert(branch.field_ids[fi], (field_atom, sem_ft));
            }

            // Emit subtree
            self.emit_decision_tree(&branch.subtree, atoms, cases, ret_type, mode)?;

            let then_body = std::mem::replace(&mut self.stmts, saved);

            match mode {
                MatchEmitMode::Stmt => {
                    // Determine return value from the branch body
                    let then_ret = fallback_return_atom_from_terminal_stmt(&then_body)
                        .map(|ret| {
                            if matches!(ret.typ(), Type::Unit) && !matches!(ret_type, Type::Unit) {
                                default_atom_for_type(ret_type)
                            } else {
                                ret
                            }
                        });

                    // Use IfReturn for value-returning branches, If for side-effect
                    if then_ret.is_some() {
                        let stmt = LirStmt::IfReturn {
                            cond: cond.clone(),
                            then_body,
                            then_ret,
                            else_body: std::mem::take(&mut chain_else),
                            else_ret: None,
                            ret_type: ret_type.clone(),
                        };
                        chain_else = vec![stmt];
                    } else {
                        let stmt = LirStmt::If {
                            cond: cond.clone(),
                            then_body,
                            else_body: std::mem::take(&mut chain_else),
                        };
                        chain_else = vec![stmt];
                    }
                }
                MatchEmitMode::Expr { .. } => {
                    let stmt = LirStmt::If {
                        cond: cond.clone(),
                        then_body,
                        else_body: std::mem::take(&mut chain_else),
                    };
                    chain_else = vec![stmt];
                }
            }
        }

        self.stmts.extend(chain_else);
        Ok(())
    }

    /// Emit a LitSwitch: literal equality checks + If/IfReturn chain.
    fn emit_lit_switch(
        &mut self,
        scrutinee_id: usize,
        branches: &[(Literal, DecTree)],
        fallback: &DecTree,
        atoms: &mut HashMap<usize, (LirAtom, Type)>,
        cases: &[MirMatchCase],
        ret_type: &Type,
        mode: &MatchEmitMode,
    ) -> Result<(), LirLowerError> {
        let (scr_atom, _scr_sem_type) = atoms[&scrutinee_id].clone();

        // Pre-compute conditions
        let mut branch_conds: Vec<LirAtom> = Vec::new();
        for (lit, _) in branches {
            let lit_atom = literal_to_atom(lit);
            let cond = self.bind_expr_to_temp(
                LirExpr::Binary {
                    op: BinaryOp::Eq,
                    lhs: scr_atom.clone(),
                    rhs: lit_atom,
                    typ: Type::Bool,
                },
                Type::Bool,
            );
            branch_conds.push(cond);
        }

        // Build chain: start with fallback
        let saved = std::mem::take(&mut self.stmts);
        self.emit_decision_tree(fallback, atoms, cases, ret_type, mode)?;
        let mut chain_else = std::mem::replace(&mut self.stmts, saved);

        for ((_lit, subtree), cond) in branches.iter().zip(branch_conds.iter()).rev() {
            let saved = std::mem::take(&mut self.stmts);
            self.emit_decision_tree(subtree, atoms, cases, ret_type, mode)?;
            let then_body = std::mem::replace(&mut self.stmts, saved);

            match mode {
                MatchEmitMode::Stmt => {
                    let then_ret = fallback_return_atom_from_terminal_stmt(&then_body)
                        .map(|ret| {
                            if matches!(ret.typ(), Type::Unit) && !matches!(ret_type, Type::Unit) {
                                default_atom_for_type(ret_type)
                            } else {
                                ret
                            }
                        });

                    if then_ret.is_some() {
                        let stmt = LirStmt::IfReturn {
                            cond: cond.clone(),
                            then_body,
                            then_ret,
                            else_body: std::mem::take(&mut chain_else),
                            else_ret: None,
                            ret_type: ret_type.clone(),
                        };
                        chain_else = vec![stmt];
                    } else {
                        let stmt = LirStmt::If {
                            cond: cond.clone(),
                            then_body,
                            else_body: std::mem::take(&mut chain_else),
                        };
                        chain_else = vec![stmt];
                    }
                }
                MatchEmitMode::Expr { .. } => {
                    let stmt = LirStmt::If {
                        cond: cond.clone(),
                        then_body,
                        else_body: std::mem::take(&mut chain_else),
                    };
                    chain_else = vec![stmt];
                }
            }
        }

        self.stmts.extend(chain_else);
        Ok(())
    }

    /// Emit a RecordDestructure: extract fields and recurse.
    fn emit_record_destructure(
        &mut self,
        scrutinee_id: usize,
        field_names: &[Symbol],
        field_ids: &[usize],
        subtree: &DecTree,
        atoms: &mut HashMap<usize, (LirAtom, Type)>,
        cases: &[MirMatchCase],
        ret_type: &Type,
        mode: &MatchEmitMode,
    ) -> Result<(), LirLowerError> {
        let (scr_atom, scr_sem_type) = atoms[&scrutinee_id].clone();

        // Resolve record field types from semantic type
        let record_field_types: Vec<(String, Type)> =
            if let Type::Record(rt_fields) = strip_linear(&scr_sem_type) {
                let mut sorted = rt_fields.clone();
                sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
                sorted
            } else {
                Vec::new()
            };

        // Extract each field
        for (i, field_name) in field_names.iter().enumerate() {
            let (sorted_idx, field_type) = record_field_types
                .iter()
                .enumerate()
                .find(|(_, (n, _))| n == field_name.as_str())
                .map(|(idx, (_, t))| (idx, t.clone()))
                .unwrap_or((i, Type::I64));

            let wasm_ft = wasm_type(&field_type);
            let field_atom = self.bind_expr_to_temp(
                LirExpr::ObjectField {
                    value: scr_atom.clone(),
                    index: sorted_idx,
                    typ: wasm_ft.clone(),
                },
                wasm_ft,
            );
            if let LirAtom::Var { name, .. } = &field_atom {
                self.semantic_vars.insert(*name, field_type.clone());
            }
            atoms.insert(field_ids[i], (field_atom, field_type));
        }

        // Recurse on subtree
        self.emit_decision_tree(subtree, atoms, cases, ret_type, mode)
    }

    /// Lower a MIR expression to an atom (flattening complex sub-expressions)
    fn lower_expr_to_atom(&mut self, expr: &MirExpr) -> Result<LirAtom, LirLowerError> {
        match expr {
            MirExpr::Literal(lit) => Ok(literal_to_atom(lit)),
            MirExpr::Variable(name) => {
                // Use semantic type (mapped through wasm_type) when available,
                // so that e.g. String stays String rather than I64 from ObjectField
                let typ = self
                    .semantic_vars
                    .get(name)
                    .map(|st| wasm_type(st))
                    .or_else(|| self.vars.get(name).cloned())
                    .ok_or_else(|| LirLowerError::UnresolvedType {
                        detail: format!("variable '{}' not in scope", name),
                        span: 0..0,
                    })?;
                Ok(LirAtom::Var {
                    name: name.clone(),
                    typ,
                })
            }
            MirExpr::BinaryOp(lhs, op, rhs) => {
                let lhs_atom = self.lower_expr_to_atom(lhs)?;
                let rhs_atom = self.lower_expr_to_atom(rhs)?;
                let result_type = infer_binary_type(*op, &lhs_atom.typ(), &rhs_atom.typ());
                Ok(self.bind_expr_to_temp(
                    LirExpr::Binary {
                        op: *op,
                        lhs: lhs_atom,
                        rhs: rhs_atom,
                        typ: result_type.clone(),
                    },
                    result_type,
                ))
            }
            MirExpr::Call {
                func,
                args,
                ret_type,
            } => {
                // Evaluate args left-to-right, then sort by label
                let mut lir_args: Vec<(Symbol, LirAtom)> = Vec::new();
                for (label, expr) in args {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_args.push((*label, atom));
                }
                lir_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                let typ = wasm_type(ret_type);
                Ok(self.bind_expr_to_temp_semantic(
                    LirExpr::Call {
                        func: func.clone(),
                        args: lir_args,
                        typ: typ.clone(),
                    },
                    typ,
                    ret_type.clone(),
                ))
            }
            MirExpr::Constructor { name, args } => {
                // 1. Evaluate args left-to-right (preserving side-effect order)
                let mut labeled_args: Vec<(Option<Symbol>, LirAtom)> = Vec::new();
                for (label, arg) in args {
                    let atom = self.lower_expr_to_atom(arg)?;
                    labeled_args.push((*label, atom));
                }

                // 2. Fill in missing labels from enum definition (positional → labeled)
                let def_labels = self
                    .ctor_index
                    .get(name.as_str())
                    .map(|info| &info.field_labels);
                if let Some(labels) = def_labels {
                    for (i, (label, _)) in labeled_args.iter_mut().enumerate() {
                        if label.is_none() {
                            if let Some(def_label) = labels.get(i) {
                                if let Some(l) = def_label {
                                    *label = Some(*l);
                                }
                            }
                        }
                    }
                }

                // 3. Sort labeled args by label (lexicographic), unlabeled stay in order
                let all_labeled = labeled_args.iter().all(|(l, _)| l.is_some());
                if all_labeled && labeled_args.len() > 1 {
                    labeled_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                }

                // 4. Strip labels
                let lir_args: Vec<LirAtom> =
                    labeled_args.into_iter().map(|(_, atom)| atom).collect();
                let typ = Type::I64; // object pointer
                Ok(self.bind_expr_to_temp(
                    LirExpr::Constructor {
                        name: name.clone(),
                        args: lir_args,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Record(fields) => {
                let mut lir_fields = Vec::new();
                for (name, expr) in fields {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_fields.push((name.clone(), atom));
                }
                // Sort fields by name for consistent layout
                lir_fields.sort_by(|(a, _), (b, _)| a.cmp(b));
                let typ = Type::I64; // object pointer
                Ok(self.bind_expr_to_temp(
                    LirExpr::Record {
                        fields: lir_fields,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Array(items) => {
                // Arrays are encoded as records with numeric indices
                let mut lir_items = Vec::new();
                for (idx, item) in items.iter().enumerate() {
                    let atom = self.lower_expr_to_atom(item)?;
                    lir_items.push((Symbol::from(idx.to_string()), atom));
                }
                let typ = Type::I64;
                Ok(self.bind_expr_to_temp(
                    LirExpr::Record {
                        fields: lir_items,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Index(arr, idx) => {
                let arr_atom = self.lower_expr_to_atom(arr)?;
                let idx_atom = self.lower_expr_to_atom(idx)?;
                // Index is compiled as object field access with dynamic index
                // For now, emit as a call to an intrinsic
                let typ = Type::I64;
                Ok(self.bind_expr_to_temp(
                    LirExpr::Call {
                        func: Symbol::from("__array_get"),
                        args: vec![
                            (Symbol::from("arr"), arr_atom),
                            (Symbol::from("idx"), idx_atom),
                        ],
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::FieldAccess(expr, field) => {
                // Resolve the receiver's semantic type to determine field index and type
                let receiver_semantic_type = self.infer_semantic_type(expr);
                let obj_atom = self.lower_expr_to_atom(expr)?;

                let (idx, field_type) =
                    resolve_field_access(&receiver_semantic_type, field.as_str())?;

                let typ = wasm_type(&field_type);
                Ok(self.bind_expr_to_temp(
                    LirExpr::ObjectField {
                        value: obj_atom,
                        index: idx,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::If {
                cond,
                then_body,
                else_body,
            } => self.lower_if_expr(cond, then_body, else_body.as_deref()),
            MirExpr::Match { target, cases } => self.lower_match_expr(target, cases),
            MirExpr::While { .. } => Err(LirLowerError::UnsupportedExpression {
                detail: "While loop in atom position; should be lowered at statement level"
                    .to_string(),
                span: 0..0,
            }),
            MirExpr::Borrow(name) => {
                let typ =
                    self.vars
                        .get(name)
                        .cloned()
                        .ok_or_else(|| LirLowerError::UnresolvedType {
                            detail: format!("borrowed variable '{}' not in scope", name),
                            span: 0..0,
                        })?;
                Ok(LirAtom::Var {
                    name: name.clone(),
                    typ,
                })
            }
            MirExpr::Raise(expr) => {
                let atom = self.lower_expr_to_atom(expr)?;
                let typ = Type::Unit; // raise doesn't return
                Ok(self.bind_expr_to_temp(
                    LirExpr::Raise {
                        value: atom,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::FuncRef(name) => {
                let typ = Type::I64; // funcref stored as i64 closure pointer
                Ok(self.bind_expr_to_temp(
                    LirExpr::FuncRef {
                        func: *name,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Closure { func, captures } => {
                // Lower each captured variable to an atom
                let capture_atoms: Vec<(Symbol, LirAtom)> = captures
                    .iter()
                    .map(|name| {
                        let typ = self
                            .semantic_vars
                            .get(name)
                            .map(|st| wasm_type(st))
                            .or_else(|| self.vars.get(name).cloned())
                            .unwrap_or(Type::I64);
                        (*name, LirAtom::Var { name: *name, typ })
                    })
                    .collect();
                let typ = Type::I64; // closure pointer as i64
                Ok(self.bind_expr_to_temp(
                    LirExpr::Closure {
                        func: *func,
                        captures: capture_atoms,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::CallIndirect {
                callee,
                args,
                ret_type: _,
                callee_type: _,
            } => {
                let callee_atom = self.lower_expr_to_atom(callee)?;
                // Resolve the actual Arrow type from semantic_vars
                let callee_type = if let MirExpr::Variable(name) = callee.as_ref() {
                    self.semantic_vars.get(name).cloned().unwrap_or(Type::I64)
                } else {
                    Type::I64
                };
                // Infer return type from the Arrow type
                let ret_type = if let Type::Arrow(_, ret, _, _) = &callee_type {
                    *ret.clone()
                } else {
                    Type::I64
                };
                let mut lir_args: Vec<(Symbol, LirAtom)> = Vec::new();
                for (label, expr) in args {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_args.push((*label, atom));
                }
                lir_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                let typ = wasm_type(&ret_type);
                Ok(self.bind_expr_to_temp_semantic(
                    LirExpr::CallIndirect {
                        callee: callee_atom,
                        args: lir_args,
                        typ: typ.clone(),
                        callee_type,
                    },
                    typ,
                    ret_type,
                ))
            }
        }
    }

    /// Infer the semantic (pre-wasm) type of a MIR expression by looking up
    /// variable bindings in semantic_vars.
    fn infer_semantic_type(&self, expr: &MirExpr) -> Type {
        match expr {
            MirExpr::Variable(name) => self.semantic_vars.get(name).cloned().unwrap_or_else(|| {
                tracing::debug!(
                    "variable '{}' not in semantic_vars during type inference, using I64",
                    name
                );
                Type::I64
            }),
            MirExpr::Call { ret_type, .. } => ret_type.clone(),
            MirExpr::Record(fields) => {
                let mut field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|(name, expr)| (name.to_string(), self.infer_semantic_type(expr)))
                    .collect();
                field_types.sort_by(|a, b| a.0.cmp(&b.0));
                Type::Record(field_types)
            }
            MirExpr::Constructor { .. } => Type::I64,
            MirExpr::Literal(lit) => match lit {
                Literal::Int(_) => Type::I64,
                Literal::Float(_) => Type::F64,
                Literal::Bool(_) => Type::Bool,
                Literal::Char(_) => Type::Char,
                Literal::String(_) => Type::String,
                Literal::Unit => Type::Unit,
            },
            MirExpr::FieldAccess(receiver, field) => {
                let receiver_type = self.infer_semantic_type(receiver);
                resolve_field_access(&receiver_type, field.as_str())
                    .map(|(_, typ)| typ)
                    .unwrap_or_else(|e| {
                        tracing::debug!(
                            "field '{}' not resolvable on type {:?}: {}",
                            field,
                            receiver_type,
                            e
                        );
                        Type::I64
                    })
            }
            MirExpr::If { then_body, .. } => {
                // Infer from last statement of then branch
                if let Some(last) = then_body.last() {
                    self.infer_stmt_type(last)
                } else {
                    Type::Unit
                }
            }
            MirExpr::Match { cases, .. } => {
                // Infer from first case body
                if let Some(case) = cases.first() {
                    if let Some(last) = case.body.last() {
                        self.infer_stmt_type(last)
                    } else {
                        Type::Unit
                    }
                } else {
                    Type::Unit
                }
            }
            MirExpr::Array(items) => {
                let elem_type = items
                    .first()
                    .map(|e| self.infer_semantic_type(e))
                    .unwrap_or_else(|| {
                        tracing::debug!("empty array literal, using I64 element type");
                        Type::I64
                    });
                Type::Array(Box::new(elem_type))
            }
            MirExpr::Index(arr, _) => {
                let arr_type = self.infer_semantic_type(arr);
                match arr_type {
                    Type::Array(elem) => *elem,
                    other => {
                        tracing::debug!("index operation on non-array type {:?}, using I64", other);
                        Type::I64
                    }
                }
            }
            MirExpr::BinaryOp(lhs, op, _) => {
                if op.is_comparison() || matches!(op, BinaryOp::And | BinaryOp::Or) {
                    Type::Bool
                } else {
                    self.infer_semantic_type(lhs)
                }
            }
            MirExpr::While { .. } => Type::Unit,
            MirExpr::Borrow(name) => self.semantic_vars.get(name).cloned().unwrap_or_else(|| {
                tracing::debug!(
                    "borrowed variable '{}' not in semantic_vars during type inference, using I64",
                    name
                );
                Type::I64
            }),
            // Raise never returns; I64 is a placeholder (no Type::Never exists)
            MirExpr::Raise(_) => Type::I64,
            MirExpr::FuncRef(_) | MirExpr::Closure { .. } => Type::I64,
            MirExpr::CallIndirect { ret_type, .. } => ret_type.clone(),
        }
    }

    fn infer_stmt_type(&self, stmt: &MirStmt) -> Type {
        match stmt {
            MirStmt::Expr(e) | MirStmt::Return(e) => self.infer_semantic_type(e),
            MirStmt::Let { typ, .. } => typ.clone(),
            _ => Type::Unit,
        }
    }

    /// Lower a block of statements, returning (statements, optional return atom)
    fn lower_block(
        &mut self,
        stmts: &[MirStmt],
        ret_type: &Type,
    ) -> Result<(Vec<LirStmt>, Option<LirAtom>), LirLowerError> {
        let vars = self.vars.clone();
        let sem_vars = self.semantic_vars.clone();
        self.lower_block_with_vars(stmts, ret_type, vars, sem_vars)
    }

    fn lower_block_with_vars(
        &mut self,
        stmts: &[MirStmt],
        ret_type: &Type,
        vars: HashMap<Symbol, Type>,
        semantic_vars: HashMap<Symbol, Type>,
    ) -> Result<(Vec<LirStmt>, Option<LirAtom>), LirLowerError> {
        // Save state
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = std::mem::replace(&mut self.vars, vars);
        let saved_semantic_vars = std::mem::replace(&mut self.semantic_vars, semantic_vars);

        let mut ret_atom = None;
        for stmt in stmts {
            if let Some(atom) = self.lower_stmt(stmt, ret_type)? {
                ret_atom = Some(atom);
                break;
            }
        }

        let block_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
        self.vars = saved_vars;
        self.semantic_vars = saved_semantic_vars;

        Ok((block_stmts, ret_atom))
    }
}

/// Convert a literal to an atom
fn literal_to_atom(lit: &Literal) -> LirAtom {
    match lit {
        Literal::Int(i) => LirAtom::Int(*i),
        Literal::Float(f) => LirAtom::Float(*f),
        Literal::Bool(b) => LirAtom::Bool(*b),
        Literal::Char(c) => LirAtom::Char(*c),
        Literal::String(s) => LirAtom::String(s.clone()),
        Literal::Unit => LirAtom::Unit,
    }
}

/// Resolve a field access on a semantic type.
/// Returns (field_index, field_type).
/// Fields are sorted alphabetically (matching record layout in codegen).
fn resolve_field_access(
    receiver_type: &Type,
    field_name: &str,
) -> Result<(usize, Type), LirLowerError> {
    match receiver_type {
        Type::Record(fields) => {
            let mut sorted: Vec<(String, Type)> = fields.clone();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (idx, (name, typ)) in sorted.iter().enumerate() {
                if name == field_name {
                    return Ok((idx, typ.clone()));
                }
            }
            Err(LirLowerError::UnsupportedExpression {
                detail: format!(
                    "field '{}' not found in record type {:?}",
                    field_name, receiver_type
                ),
                span: 0..0,
            })
        }
        _ => Err(LirLowerError::UnsupportedExpression {
            detail: format!(
                "field access '.{}' on non-record type {:?}",
                field_name, receiver_type
            ),
            span: 0..0,
        }),
    }
}

/// Map a high-level AST type to its WASM-level representation.
/// Primitives pass through with semantic type preserved (Bool stays Bool, etc.).
/// Heap-allocated and compound types collapse to I64 (object pointer).
fn wasm_type(typ: &Type) -> Type {
    match typ {
        // Primitives: keep semantic type for downstream codegen
        Type::I32
        | Type::I64
        | Type::F32
        | Type::F64
        | Type::Bool
        | Type::Char
        | Type::String
        | Type::Unit => typ.clone(),
        // Literal types: resolve to concrete numeric type
        Type::IntLit => Type::I64,
        Type::FloatLit => Type::F64,
        // Rows are not runtime values
        Type::Row(_, _) => Type::Unit,
        // All compound/heap types: classify via wasm_repr
        _ => match typ.wasm_repr() {
            WasmRepr::I64 => Type::I64,
            WasmRepr::I32 | WasmRepr::F32 | WasmRepr::F64 | WasmRepr::Unit => {
                unreachable!("compound type {typ} has unexpected wasm_repr")
            }
        },
    }
}

/// Infer the result type of a binary operation (matches old lower.rs logic exactly)
fn infer_binary_type(op: BinaryOp, lhs: &Type, rhs: &Type) -> Type {
    if op.is_comparison() {
        return Type::Bool;
    }
    if op == BinaryOp::And || op == BinaryOp::Or {
        return Type::Bool;
    }
    if op == BinaryOp::Concat
        || (op == BinaryOp::Add && matches!(lhs, Type::String) && matches!(rhs, Type::String))
    {
        return Type::String;
    }
    if op.is_float_op()
        || matches!(lhs, Type::F32 | Type::F64)
        || matches!(rhs, Type::F32 | Type::F64)
    {
        if matches!(lhs, Type::F32) || matches!(rhs, Type::F32) {
            Type::F32
        } else {
            Type::F64
        }
    } else if matches!(lhs, Type::I32) || matches!(rhs, Type::I32) {
        Type::I32
    } else {
        Type::I64
    }
}

/// Check if the terminal statement in a block is a genuine return-containing
/// statement (IfReturn or TryCatch with returns). Unlike `fallback_return_atom_from_terminal_stmt`,
/// this does NOT match trailing Let bindings, which are not actual returns.
/// Try to extract a return atom from the last statement in a block.
/// Only matches genuine return-producing statements (IfReturn, TryCatch).
/// Does NOT match Let bindings — a trailing `let x = expr` is a side effect,
/// not a return value. Treating Let as a return was the root cause of the
/// spurious-return bug in match statements.
fn fallback_return_atom_from_terminal_stmt(stmts: &[LirStmt]) -> Option<LirAtom> {
    match stmts.last()? {
        LirStmt::IfReturn { then_ret, .. } => then_ret.clone(),
        LirStmt::TryCatch {
            catch_ret,
            body_ret,
            ..
        } => catch_ret.clone().or_else(|| body_ret.clone()),
        _ => None,
    }
}

fn strip_linear(typ: &Type) -> &Type {
    match typ {
        Type::Linear(inner) => strip_linear(inner),
        other => other,
    }
}

/// Produce a default zero-value atom for a given WASM type (used as placeholder).
fn default_atom_for_type(typ: &Type) -> LirAtom {
    match typ {
        Type::I32 | Type::I64 => LirAtom::Int(0),
        Type::F32 | Type::F64 => LirAtom::Float(0.0),
        Type::Bool => LirAtom::Bool(false),
        Type::Char => LirAtom::Char('\0'),
        Type::String => LirAtom::String(String::new()),
        Type::Unit => LirAtom::Unit,
        _ => LirAtom::Int(0), // heap pointer types default to 0
    }
}

/// Collect free variables in a MIR statement block (referenced but not defined).
/// Returns names in a stable order.
fn collect_free_vars(body: &[MirStmt]) -> Vec<Symbol> {
    let mut defined = HashSet::new();
    let mut referenced = Vec::new();
    let mut seen = HashSet::new();
    for stmt in body {
        collect_mir_stmt_refs(stmt, &mut defined, &mut referenced, &mut seen);
    }
    referenced
        .into_iter()
        .filter(|name| !defined.contains(name))
        .collect()
}

fn collect_mir_stmt_refs(
    stmt: &MirStmt,
    defined: &mut HashSet<Symbol>,
    referenced: &mut Vec<Symbol>,
    seen: &mut HashSet<Symbol>,
) {
    match stmt {
        MirStmt::Let { name, expr, .. } => {
            collect_mir_expr_refs(expr, referenced, seen);
            defined.insert(*name);
        }
        MirStmt::Expr(expr) | MirStmt::Return(expr) => {
            collect_mir_expr_refs(expr, referenced, seen);
        }
        MirStmt::Assign { target, value } => {
            collect_mir_expr_refs(target, referenced, seen);
            collect_mir_expr_refs(value, referenced, seen);
        }
        MirStmt::Conc(tasks) => {
            for task in tasks {
                for s in &task.body {
                    collect_mir_stmt_refs(s, defined, referenced, seen);
                }
            }
        }
        MirStmt::Try {
            body,
            catch_param,
            catch_body,
        } => {
            for s in body {
                collect_mir_stmt_refs(s, defined, referenced, seen);
            }
            defined.insert(*catch_param);
            for s in catch_body {
                collect_mir_stmt_refs(s, defined, referenced, seen);
            }
        }
    }
}

fn collect_mir_expr_refs(expr: &MirExpr, referenced: &mut Vec<Symbol>, seen: &mut HashSet<Symbol>) {
    match expr {
        MirExpr::Variable(name) | MirExpr::Borrow(name) => {
            if seen.insert(*name) {
                referenced.push(*name);
            }
        }
        MirExpr::BinaryOp(lhs, _, rhs) => {
            collect_mir_expr_refs(lhs, referenced, seen);
            collect_mir_expr_refs(rhs, referenced, seen);
        }
        MirExpr::Call { args, .. } => {
            for (_, arg) in args {
                collect_mir_expr_refs(arg, referenced, seen);
            }
        }
        MirExpr::Constructor { args, .. } => {
            for (_, arg) in args {
                collect_mir_expr_refs(arg, referenced, seen);
            }
        }
        MirExpr::Record(fields) => {
            for (_, expr) in fields {
                collect_mir_expr_refs(expr, referenced, seen);
            }
        }
        MirExpr::Array(items) => {
            for item in items {
                collect_mir_expr_refs(item, referenced, seen);
            }
        }
        MirExpr::Index(arr, idx) => {
            collect_mir_expr_refs(arr, referenced, seen);
            collect_mir_expr_refs(idx, referenced, seen);
        }
        MirExpr::FieldAccess(expr, _) | MirExpr::Raise(expr) => {
            collect_mir_expr_refs(expr, referenced, seen);
        }
        MirExpr::If {
            cond,
            then_body,
            else_body,
        } => {
            collect_mir_expr_refs(cond, referenced, seen);
            for s in then_body {
                let mut defined = HashSet::new();
                collect_mir_stmt_refs(s, &mut defined, referenced, seen);
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    let mut defined = HashSet::new();
                    collect_mir_stmt_refs(s, &mut defined, referenced, seen);
                }
            }
        }
        MirExpr::Match { target, cases } => {
            collect_mir_expr_refs(target, referenced, seen);
            for case in cases {
                collect_mir_pattern_defs(&case.pattern, seen);
                for s in &case.body {
                    let mut defined = HashSet::new();
                    collect_mir_stmt_refs(s, &mut defined, referenced, seen);
                }
            }
        }
        MirExpr::While { cond, body } => {
            collect_mir_expr_refs(cond, referenced, seen);
            for s in body {
                let mut defined = HashSet::new();
                collect_mir_stmt_refs(s, &mut defined, referenced, seen);
            }
        }
        MirExpr::FuncRef(_) | MirExpr::Literal(_) => {}
        MirExpr::Closure { captures, .. } => {
            for name in captures {
                if seen.insert(*name) {
                    referenced.push(*name);
                }
            }
        }
        MirExpr::CallIndirect { callee, args, .. } => {
            collect_mir_expr_refs(callee, referenced, seen);
            for (_, arg) in args {
                collect_mir_expr_refs(arg, referenced, seen);
            }
        }
    }
}

fn collect_mir_pattern_defs(pattern: &MirPattern, seen: &mut HashSet<Symbol>) {
    match pattern {
        MirPattern::Variable(name, _) => {
            seen.insert(*name);
        }
        MirPattern::Constructor { fields, .. } => {
            for (_, pat) in fields {
                collect_mir_pattern_defs(pat, seen);
            }
        }
        MirPattern::Record(fields, _) => {
            for (_, pat) in fields {
                collect_mir_pattern_defs(pat, seen);
            }
        }
        MirPattern::Wildcard | MirPattern::Literal(_) => {}
    }
}

/// Build a HashMap from constructor name → ConstructorInfo for O(1) lookups.
/// Later entries in enum_defs shadow earlier ones (user defs over stdlib).
fn build_constructor_index(enum_defs: &[EnumDef]) -> HashMap<String, ConstructorInfo> {
    let mut index = HashMap::new();
    // Forward iteration: later defs overwrite earlier ones (same as rfind semantics)
    for (enum_idx, def) in enum_defs.iter().enumerate() {
        for (variant_idx, variant) in def.variants.iter().enumerate() {
            let field_labels: Vec<Option<Symbol>> = variant
                .fields
                .iter()
                .map(|(label, _)| label.as_ref().map(|l| Symbol::from(l.as_str())))
                .collect();

            let sorted_indices = {
                let all_labeled = variant.fields.iter().all(|(l, _)| l.is_some());
                if !all_labeled || variant.fields.is_empty() {
                    None
                } else {
                    let mut labeled: Vec<(usize, &str)> = variant
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, (l, _))| (i, l.as_ref().unwrap().as_str()))
                        .collect();
                    labeled.sort_by(|a, b| a.1.cmp(b.1));
                    let mut mapping = vec![0usize; labeled.len()];
                    for (sorted_idx, (def_idx, _)) in labeled.iter().enumerate() {
                        mapping[*def_idx] = sorted_idx;
                    }
                    Some(mapping)
                }
            };

            index.insert(
                variant.name.clone(),
                ConstructorInfo {
                    enum_idx,
                    variant_idx,
                    field_labels,
                    sorted_indices,
                },
            );
        }
    }
    index
}

/// Resolve the concrete field types for a constructor variant, applying type parameter
/// substitution from the matched enum type.
///
/// For example, matching `Cons(v, rest)` against `List<String>` resolves:
///   v → String, rest → List<String>
fn resolve_constructor_field_types(
    ctor_name: &str,
    matched_type: &Type,
    ctor_index: &HashMap<String, ConstructorInfo>,
    enum_defs: &[EnumDef],
) -> Option<Vec<Type>> {
    // Extract the enum name and type arguments from the matched type
    let (enum_name, type_args) = match strip_linear(matched_type) {
        Type::UserDefined(name, args) => (name.clone(), args.clone()),
        Type::List(inner) => ("List".to_string(), vec![inner.as_ref().clone()]),
        _ => return None,
    };

    let info = ctor_index.get(ctor_name)?;
    let enum_def = &enum_defs[info.enum_idx];

    // Verify this constructor belongs to the expected enum
    if enum_def.name != enum_name {
        return None;
    }

    let variant = &enum_def.variants[info.variant_idx];

    // Build substitution map: type_param → concrete_type
    let subst: HashMap<String, Type> = enum_def
        .type_params
        .iter()
        .zip(type_args.iter())
        .map(|(param, arg)| (param.clone(), arg.clone()))
        .collect();

    // Apply substitution to each field type
    Some(
        variant
            .fields
            .iter()
            .map(|(_, ft)| apply_type_subst(ft, &subst))
            .collect(),
    )
}

/// Recursively apply a type parameter substitution.
fn apply_type_subst(typ: &Type, subst: &HashMap<String, Type>) -> Type {
    match typ {
        Type::Var(name) => subst.get(name).cloned().unwrap_or_else(|| typ.clone()),
        Type::UserDefined(name, args) => {
            // Check if it's actually a type variable reference with no args
            if args.is_empty() {
                if let Some(concrete) = subst.get(name) {
                    return concrete.clone();
                }
            }
            Type::UserDefined(
                name.clone(),
                args.iter().map(|a| apply_type_subst(a, subst)).collect(),
            )
        }
        Type::List(inner) => Type::List(Box::new(apply_type_subst(inner, subst))),
        Type::Array(inner) => Type::Array(Box::new(apply_type_subst(inner, subst))),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), apply_type_subst(t, subst)))
                .collect(),
        ),
        Type::Arrow(params, ret, req, eff) => Type::Arrow(
            params
                .iter()
                .map(|(n, t)| (n.clone(), apply_type_subst(t, subst)))
                .collect(),
            Box::new(apply_type_subst(ret, subst)),
            Box::new(apply_type_subst(req, subst)),
            Box::new(apply_type_subst(eff, subst)),
        ),
        Type::Linear(inner) => Type::Linear(Box::new(apply_type_subst(inner, subst))),
        Type::Row(types, rest) => Type::Row(
            types.iter().map(|t| apply_type_subst(t, subst)).collect(),
            rest.as_ref().map(|r| Box::new(apply_type_subst(r, subst))),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(apply_type_subst(inner, subst))),
        Type::Borrow(inner) => Type::Borrow(Box::new(apply_type_subst(inner, subst))),
        // Primitive types (I32, I64, F32, F64, Bool, String, Unit, etc.): no substitution
        _ => typ.clone(),
    }
}

// ────────────────────────────────────────────────────────────────
// Closure conversion: ensure all funcref-target functions have a
// uniform `(__env: i64, params...) -> ret` calling convention.
// ────────────────────────────────────────────────────────────────

/// Collect all function names that are used as FuncRef or Closure targets in the LIR.
fn collect_lir_funcref_targets(functions: &[LirFunction]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    fn scan_expr(expr: &LirExpr, targets: &mut HashSet<Symbol>) {
        match expr {
            LirExpr::FuncRef { func, .. } | LirExpr::Closure { func, .. } => {
                targets.insert(*func);
            }
            _ => {}
        }
    }
    fn scan_stmt(stmt: &LirStmt, targets: &mut HashSet<Symbol>) {
        match stmt {
            LirStmt::Let { expr, .. } => scan_expr(expr, targets),
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    scan_stmt(s, targets);
                }
                for s in else_body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    scan_stmt(s, targets);
                }
                for s in else_body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                for s in body {
                    scan_stmt(s, targets);
                }
                for s in catch_body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::Conc { .. } => {}
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                for s in cond_stmts {
                    scan_stmt(s, targets);
                }
                for s in body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for c in cases {
                    for s in &c.body {
                        scan_stmt(s, targets);
                    }
                }
                for s in default_body {
                    scan_stmt(s, targets);
                }
            }
        }
    }
    for func in functions {
        for stmt in &func.body {
            scan_stmt(stmt, &mut targets);
        }
    }
    targets
}

/// Perform closure conversion on all funcref-target functions.
///
/// - Capturing lambdas: replace capture params with `__env: i64`, prepend ClosureEnvLoad
/// - Non-capturing lambdas: add `__env: i64` as first param (unused)
/// - Named functions: generate a thin wrapper with `__env: i64`
fn closure_convert(
    functions: &mut Vec<LirFunction>,
    mir_functions: &[MirFunction],
) -> Result<(), LirLowerError> {
    let targets = collect_lir_funcref_targets(functions);
    if targets.is_empty() {
        return Ok(());
    }

    let env_sym = Symbol::from("__env");

    // Build a set of lambda function names and their capture info from MIR
    let mir_captures: HashMap<Symbol, &[Symbol]> = mir_functions
        .iter()
        .filter(|f| !f.captures.is_empty())
        .map(|f| (f.name, f.captures.as_slice()))
        .collect();

    // O(1) function lookup by name → index
    let func_index: HashMap<Symbol, usize> = functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name, i))
        .collect();

    // Collect names of functions that need wrappers (non-lambda funcref targets)
    let mut wrapper_targets: Vec<Symbol> = Vec::new();

    for &target in &targets {
        let is_lambda = target.as_str().starts_with("__lambda_");
        if is_lambda {
            // Transform the lambda function in-place
            if let Some(&idx) = func_index.get(&target) {
                let func = &mut functions[idx];
                if let Some(&captures) = mir_captures.get(&target) {
                    // Capturing lambda: replace capture params with __env, add ClosureEnvLoad preamble
                    let n_captures = captures.len();

                    // Extract capture param types (the first N params)
                    let capture_types: Vec<Type> = func.params[..n_captures]
                        .iter()
                        .map(|p| p.typ.clone())
                        .collect();

                    // Remove capture params, add __env as first
                    func.params = std::iter::once(LirParam {
                        label: env_sym,
                        name: env_sym,
                        typ: Type::I64,
                    })
                    .chain(func.params[n_captures..].iter().cloned())
                    .collect();

                    // Prepend ClosureEnvLoad for each capture
                    let mut preamble: Vec<LirStmt> = captures
                        .iter()
                        .enumerate()
                        .map(|(i, &cap_name)| LirStmt::Let {
                            name: cap_name,
                            typ: wasm_type(&capture_types[i]),
                            expr: LirExpr::ClosureEnvLoad {
                                index: i,
                                typ: capture_types[i].clone(),
                            },
                        })
                        .collect();
                    preamble.append(&mut func.body);
                    func.body = preamble;
                } else {
                    // Non-capturing lambda: just add __env as first param
                    func.params.insert(
                        0,
                        LirParam {
                            label: env_sym,
                            name: env_sym,
                            typ: Type::I64,
                        },
                    );
                }
            }
        } else {
            // Named function: needs a wrapper
            wrapper_targets.push(target);
        }
    }

    // Generate wrapper functions for named funcref targets
    let func_index: HashMap<Symbol, usize> = functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name, i))
        .collect();
    for target in wrapper_targets {
        if let Some(&idx) = func_index.get(&target) {
            let original = &functions[idx];
            let wrapper_name = Symbol::from(format!("__closure_wrap_{}", target));
            let result_sym = Symbol::from("__closure_wrap_result");

            // Wrapper params: __env + original params
            let wrapper_params: Vec<LirParam> = std::iter::once(LirParam {
                label: env_sym,
                name: env_sym,
                typ: Type::I64,
            })
            .chain(original.params.iter().cloned())
            .collect();

            // Build call args from original params (skip __env)
            let call_args: Vec<(Symbol, LirAtom)> = original
                .params
                .iter()
                .map(|p| {
                    (
                        p.label,
                        LirAtom::Var {
                            name: p.name,
                            typ: p.typ.clone(),
                        },
                    )
                })
                .collect();

            let ret_type = original.ret_type.clone();
            let wasm_ret = wasm_type(&ret_type);

            let body = if matches!(wasm_ret, Type::Unit) {
                vec![LirStmt::Let {
                    name: result_sym,
                    typ: wasm_ret.clone(),
                    expr: LirExpr::Call {
                        func: target,
                        args: call_args,
                        typ: wasm_ret.clone(),
                    },
                }]
            } else {
                vec![LirStmt::Let {
                    name: result_sym,
                    typ: wasm_ret.clone(),
                    expr: LirExpr::Call {
                        func: target,
                        args: call_args,
                        typ: wasm_ret.clone(),
                    },
                }]
            };

            let ret_atom = if matches!(wasm_ret, Type::Unit) {
                LirAtom::Unit
            } else {
                LirAtom::Var {
                    name: result_sym,
                    typ: wasm_ret,
                }
            };

            let wrapper = LirFunction {
                name: wrapper_name,
                params: wrapper_params,
                ret_type: ret_type.clone(),
                requires: Type::Row(Vec::new(), None),
                throws: Type::Row(Vec::new(), None),
                body,
                ret: ret_atom,
                span: 0..0,
                source_file: None,
                source_line: None,
            };

            functions.push(wrapper);

            // Update FuncRef nodes: FuncRef(target) → FuncRef(wrapper_name)
            update_funcref_targets(functions, target, wrapper_name);
        }
    }

    Ok(())
}

/// Update all FuncRef nodes for a given target to point to a new name.
fn update_funcref_targets(functions: &mut [LirFunction], old_name: Symbol, new_name: Symbol) {
    fn update_expr(expr: &mut LirExpr, old: Symbol, new: Symbol) {
        match expr {
            LirExpr::FuncRef { func, .. } if *func == old => {
                *func = new;
            }
            _ => {}
        }
    }
    fn update_stmt(stmt: &mut LirStmt, old: Symbol, new: Symbol) {
        match stmt {
            LirStmt::Let { expr, .. } => update_expr(expr, old, new),
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    update_stmt(s, old, new);
                }
                for s in else_body {
                    update_stmt(s, old, new);
                }
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    update_stmt(s, old, new);
                }
                for s in else_body {
                    update_stmt(s, old, new);
                }
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                for s in body {
                    update_stmt(s, old, new);
                }
                for s in catch_body {
                    update_stmt(s, old, new);
                }
            }
            LirStmt::Conc { .. } => {}
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                for s in cond_stmts {
                    update_stmt(s, old, new);
                }
                for s in body {
                    update_stmt(s, old, new);
                }
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for c in cases {
                    for s in &mut c.body {
                        update_stmt(s, old, new);
                    }
                }
                for s in default_body {
                    update_stmt(s, old, new);
                }
            }
        }
    }
    for func in functions.iter_mut() {
        for stmt in &mut func.body {
            update_stmt(stmt, old_name, new_name);
        }
    }
}
