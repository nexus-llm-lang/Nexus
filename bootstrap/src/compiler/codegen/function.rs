use std::collections::{HashMap, HashSet};

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirFunction, LirProgram, LirStmt};
use crate::types::Type;

/// TCO loop context for self-recursion-to-loop optimization.
/// When present, self-tail-calls emit param reassignment + `br` instead of `return_call`.
#[derive(Clone, Copy)]
pub(super) struct TcoLoop {
    /// Name of the current function (to detect self-tail-calls).
    pub self_name: Symbol,
    /// WASM block depth from current position to the TCO loop header.
    pub loop_depth: u32,
}

impl TcoLoop {
    /// Return a new TcoLoop with depth incremented by `n` (for entering WASM blocks).
    pub fn deeper(self, n: u32) -> Self {
        TcoLoop {
            loop_depth: self.loop_depth + n,
            ..self
        }
    }
}

/// Pre-detection info for TCMC (before WASM local allocation).
struct TcmcPreInfo {
    call_var: Symbol,
    call_args: Vec<(Symbol, LirAtom)>,
    cons_var: Symbol,
    ctor_name: Symbol,
    ctor_num_fields: usize,
    rest_field_idx: usize,
    non_rec_fields: Vec<(usize, LirAtom)>,
}

/// TCMC codegen context: tail call modulo cons(tructor) optimization.
/// Builds lists in-place by allocating Cons cells with placeholder `rest`,
/// linking them forward, and looping back via the TCO loop.
pub(super) struct TcmcInfo {
    /// WASM local index for the head pointer (i64, first cell allocated).
    pub head_local: u32,
    /// WASM local index for the previous cell pointer (i32, for field mutation).
    pub prev_local: u32,
    /// Constructor field index for the recursive result (e.g., 0 for Cons.rest).
    pub rest_field_idx: usize,
    /// LIR symbol for the Let binding of the self-call result (to skip).
    pub call_var: Symbol,
    /// LIR symbol for the Let binding of the Constructor result (to intercept).
    pub cons_var: Symbol,
    /// Self-call args saved from detection (for param reassignment).
    pub call_args: Vec<(Symbol, LirAtom)>,
    /// Constructor name (e.g., "Cons").
    pub ctor_name: Symbol,
    /// Number of constructor fields.
    pub ctor_num_fields: usize,
    /// Non-recursive constructor fields: (field_idx, value atom).
    pub non_rec_fields: Vec<(usize, LirAtom)>,
}

use super::binary::{binary_operand_type, compile_binary};
use super::emit::{
    compile_external_arg, constructor_tag, emit_alloc_object, emit_numeric_coercion,
    emit_typed_field_load, emit_typed_field_store, memarg, record_tag, type_to_wasm_valtype,
};
use super::error::CodegenError;
use super::layout::CodegenLayout;
use super::stmt::compile_stmt;
use super::string::{
    emit_canonical_string_return_unpack, emit_string_compare, emit_string_concat,
    external_uses_canonical_string_return, is_string_compare_operator, is_string_concat_operator,
    pack_string,
};
use super::{FunctionTemps, LocalInfo, OBJECT_HEAP_GLOBAL_INDEX, STRING_HEAP_GLOBAL_INDEX};
use crate::constants::ENTRYPOINT;

pub(super) fn compile_function(
    func: &LirFunction,
    program: &LirProgram,
    internal_indices: &HashMap<Symbol, u32>,
    external_indices: &HashMap<Symbol, u32>,
    layout: &CodegenLayout,
) -> Result<Function, CodegenError> {
    let mut local_map: HashMap<Symbol, LocalInfo> = HashMap::new();
    for (idx, param) in func.params.iter().enumerate() {
        let vt = type_to_wasm_valtype(&param.typ)?;
        local_map.insert(
            param.name.clone(),
            LocalInfo {
                index: idx as u32,
                val_type: vt,
            },
        );
    }

    let mut next_local_index = func.params.len() as u32;
    let mut local_decls_flat = Vec::new();
    collect_stmt_locals(
        &func.body,
        &mut local_map,
        &mut next_local_index,
        &mut local_decls_flat,
    )?;

    // Coalesce WASM locals: reuse indices for variables with non-overlapping lifetimes
    let param_count = func.params.len() as u32;
    coalesce_wasm_locals(func, &mut local_map, &mut local_decls_flat, param_count);
    // Recount next_local_index after coalescing
    next_local_index = param_count + local_decls_flat.len() as u32;

    let temps = FunctionTemps {
        packed_tmp_i64: next_local_index,
        object_ptr_i32: next_local_index + 1,
        concat_lhs_packed_i64: next_local_index + 2,
        concat_rhs_packed_i64: next_local_index + 3,
        concat_lhs_ptr_i32: next_local_index + 4,
        concat_lhs_len_i32: next_local_index + 5,
        concat_rhs_ptr_i32: next_local_index + 6,
        concat_rhs_len_i32: next_local_index + 7,
        concat_out_ptr_i32: next_local_index + 8,
        concat_out_len_i32: next_local_index + 9,
        concat_idx_i32: next_local_index + 10,
        scan_byte_i32: next_local_index + 11,
        scan_end_i32: next_local_index + 12,
        closure_ptr_i64: next_local_index + 13,
        closure_table_idx_i64: next_local_index + 14,
    };
    // Temps: 1×i64, 1×i32, 2×i64, 7×i32, 2×i32(scan), 2×i64(closure)
    local_decls_flat.extend_from_slice(&[
        ValType::I64,
        ValType::I32,
        ValType::I64,
        ValType::I64,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I32,
        ValType::I64,
        ValType::I64,
    ]);

    // TCMC: detect tail-call-modulo-constructor pattern and allocate extra locals
    // TODO: TCMC disabled pending runtime bug fix (bootstrap crash)
    let tcmc_pre: Option<TcmcPreInfo> = None; // detect_tcmc(func);
    let tcmc_info = tcmc_pre.map(|pre| {
        let head_idx = next_local_index + 15; // after FunctionTemps (15 slots)
        let prev_idx = next_local_index + 16;
        local_decls_flat.push(ValType::I64); // __tcmc_head
        local_decls_flat.push(ValType::I32); // __tcmc_prev
        TcmcInfo {
            head_local: head_idx,
            prev_local: prev_idx,
            rest_field_idx: pre.rest_field_idx,
            call_var: pre.call_var,
            cons_var: pre.cons_var,
            call_args: pre.call_args,
            ctor_name: pre.ctor_name,
            ctor_num_fields: pre.ctor_num_fields,
            non_rec_fields: pre.non_rec_fields,
        }
    });

    // RLE-compress local declarations for WASM
    let wasm_locals = local_decls_flat.iter().fold(Vec::new(), |mut acc, &vt| {
        if let Some((count, last_ty)) = acc.last_mut() {
            if *last_ty == vt {
                *count += 1;
                return acc;
            }
        }
        acc.push((1u32, vt));
        acc
    });

    let mut out = Function::new(wasm_locals);

    let is_entrypoint = func.name == ENTRYPOINT;

    // Self-recursion-to-loop: if the function contains self-tail-calls
    // or TCMC patterns, wrap the body in a WASM `loop`.
    let needs_loop = has_self_tail_call(func) || tcmc_info.is_some();
    let tco_loop = if needs_loop {
        Some(TcoLoop {
            self_name: func.name,
            loop_depth: 0,
        })
    } else {
        None
    };

    // TCMC init: set head=0, prev=0 BEFORE the loop (only runs once)
    if let Some(ref tcmc) = tcmc_info {
        out.instruction(&Instruction::I64Const(0));
        out.instruction(&Instruction::LocalSet(tcmc.head_local));
        out.instruction(&Instruction::I32Const(0));
        out.instruction(&Instruction::LocalSet(tcmc.prev_local));
    }

    if needs_loop {
        out.instruction(&Instruction::Loop(BlockType::Empty));
    }

    for stmt in &func.body {
        compile_stmt(
            stmt,
            &mut out,
            &local_map,
            program,
            internal_indices,
            external_indices,
            layout,
            &temps,
            &func.ret_type,
            false,
            is_entrypoint,
            tco_loop,
            tcmc_info.as_ref(),
        )?;
    }

    if needs_loop {
        out.instruction(&Instruction::End); // end loop
    }

    // Function epilogue: return value (with TCMC linking if active)
    if !matches!(func.ret_type, Type::Unit) {
        if let Some(ref tcmc) = tcmc_info {
            // Conditional TCMC return for fallthrough path
            let ret_vt = type_to_wasm_valtype(&func.ret_type)?;
            out.instruction(&Instruction::LocalGet(tcmc.prev_local));
            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::I32Ne);
            out.instruction(&Instruction::If(BlockType::Result(ret_vt)));
            {
                // Link last cell's rest to fallthrough value
                out.instruction(&Instruction::LocalGet(tcmc.prev_local));
                compile_atom(&func.ret, &mut out, &local_map, layout)?;
                emit_typed_field_store(
                    &func.ret.typ(),
                    ((tcmc.rest_field_idx + 1) * 8) as u64,
                    &mut out,
                )?;
                out.instruction(&Instruction::LocalGet(tcmc.head_local));
            }
            out.instruction(&Instruction::Else);
            {
                compile_atom(&func.ret, &mut out, &local_map, layout)?;
                emit_numeric_coercion(&func.ret.typ(), &func.ret_type, &mut out)?;
            }
            out.instruction(&Instruction::End);
        } else {
            compile_atom(&func.ret, &mut out, &local_map, layout)?;
            emit_numeric_coercion(&func.ret.typ(), &func.ret_type, &mut out)?;
        }
    }
    out.instruction(&Instruction::End);

    Ok(out)
}

fn register_local(
    local_map: &mut HashMap<Symbol, LocalInfo>,
    next_local_index: &mut u32,
    local_decls_flat: &mut Vec<ValType>,
    name: Symbol,
    typ: &Type,
) -> Result<(), CodegenError> {
    if matches!(typ, Type::Unit) {
        return Ok(());
    }
    let vt = type_to_wasm_valtype(typ)?;
    match local_map.get(&name) {
        Some(existing) => {
            if existing.val_type != vt {
                return Err(CodegenError::ConflictingLocalTypes {
                    name: name.to_string(),
                });
            }
        }
        None => {
            local_map.insert(
                name,
                LocalInfo {
                    index: *next_local_index,
                    val_type: vt,
                },
            );
            *next_local_index += 1;
            local_decls_flat.push(vt);
        }
    }
    Ok(())
}

fn collect_stmt_locals(
    stmts: &[LirStmt],
    local_map: &mut HashMap<Symbol, LocalInfo>,
    next_local_index: &mut u32,
    local_decls_flat: &mut Vec<ValType>,
) -> Result<(), CodegenError> {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { name, typ, .. } => {
                register_local(local_map, next_local_index, local_decls_flat, *name, typ)?;
            }
            LirStmt::TryCatch {
                body,
                catch_param,
                catch_param_typ,
                catch_body,
                ..
            } => {
                register_local(
                    local_map,
                    next_local_index,
                    local_decls_flat,
                    *catch_param,
                    catch_param_typ,
                )?;
                collect_stmt_locals(body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(catch_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_locals(then_body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(else_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_locals(then_body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(else_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                collect_stmt_locals(cond_stmts, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    collect_stmt_locals(&case.body, local_map, next_local_index, local_decls_flat)?;
                }
                collect_stmt_locals(default_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::FieldUpdate { .. } => {}
        }
    }
    Ok(())
}

/// Coalesce WASM locals by reusing indices for variables with non-overlapping
/// lifetimes and the same WASM type. Uses flat sequential numbering over all
/// statements; references inside nested scopes (branches, loops) are projected
/// to the enclosing statement's position in the parent scope.
fn coalesce_wasm_locals(
    func: &LirFunction,
    local_map: &mut HashMap<Symbol, LocalInfo>,
    local_decls_flat: &mut Vec<ValType>,
    param_count: u32,
) {
    // 1. Compute liveness: (def_pos, last_use_pos) for each non-param variable.
    //    Position is a flat counter at the top-level scope. Nested scopes
    //    contribute to the enclosing stmt's position.
    let mut def_pos: HashMap<Symbol, usize> = HashMap::new();
    let mut last_use: HashMap<Symbol, usize> = HashMap::new();
    for (pos, stmt) in func.body.iter().enumerate() {
        // Record definitions
        if let LirStmt::Let { name, .. } = stmt {
            def_pos.entry(*name).or_insert(pos);
        }
        collect_defs_nested(stmt, pos, &mut def_pos);
        // Record all variable references (including in nested bodies)
        let mut refs = HashSet::new();
        collect_refs_in_stmt(stmt, &mut refs);
        for name in refs {
            last_use
                .entry(name)
                .and_modify(|e| *e = (*e).max(pos))
                .or_insert(pos);
        }
    }
    // Also include return atom references
    if let LirAtom::Var { name, .. } = &func.ret {
        let end = func.body.len();
        last_use
            .entry(*name)
            .and_modify(|e| *e = (*e).max(end))
            .or_insert(end);
    }

    // 2. Build sorted list of (variable, def_pos, end_pos, wasm_type, old_index)
    //    excluding params (they have fixed indices).
    let param_names: HashSet<Symbol> = func.params.iter().map(|p| p.name).collect();
    let mut intervals: Vec<(Symbol, usize, usize, ValType, u32)> = Vec::new();
    for (name, info) in local_map.iter() {
        if param_names.contains(name) {
            continue;
        }
        let dp = def_pos.get(name).copied().unwrap_or(0);
        let lu = last_use.get(name).copied().unwrap_or(dp);
        intervals.push((*name, dp, lu, info.val_type, info.index));
    }
    if intervals.len() < 2 {
        return; // nothing to coalesce
    }
    intervals.sort_by_key(|&(_, dp, _, _, _)| dp);

    // 3. Linear scan: greedily assign to reusable slots.
    //    A slot is reusable when its current occupant's last_use < new variable's def_pos.
    struct Slot {
        new_index: u32,
        val_type: ValType,
        end_pos: usize,
    }
    let mut slots: Vec<Slot> = Vec::new();
    let mut remap: HashMap<u32, u32> = HashMap::new(); // old_index → new_index

    for &(_, dp, lu, vt, old_idx) in &intervals {
        // Find an expired slot with matching type
        let mut found = None;
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.end_pos < dp && slot.val_type == vt {
                found = Some(i);
                break;
            }
        }
        if let Some(i) = found {
            remap.insert(old_idx, slots[i].new_index);
            slots[i].end_pos = lu;
        } else {
            let new_idx = param_count + slots.len() as u32;
            if new_idx != old_idx {
                remap.insert(old_idx, new_idx);
            }
            slots.push(Slot {
                new_index: new_idx,
                val_type: vt,
                end_pos: lu,
            });
        }
    }

    if remap.is_empty() || slots.len() >= intervals.len() {
        return; // no savings
    }

    // 4. Apply remapping to local_map
    for info in local_map.values_mut() {
        if let Some(&new_idx) = remap.get(&info.index) {
            info.index = new_idx;
        }
    }

    // 5. Rebuild local_decls_flat from slots
    local_decls_flat.clear();
    local_decls_flat.extend(slots.iter().map(|s| s.val_type));
}

/// Collect variable definitions from nested scopes, projecting them
/// to the parent scope's position.
fn collect_defs_nested(stmt: &LirStmt, pos: usize, def_pos: &mut HashMap<Symbol, usize>) {
    match stmt {
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
            for s in then_body.iter().chain(else_body.iter()) {
                if let LirStmt::Let { name, .. } = s {
                    def_pos.entry(*name).or_insert(pos);
                }
                collect_defs_nested(s, pos, def_pos);
            }
        }
        LirStmt::TryCatch {
            body,
            catch_param,
            catch_body,
            ..
        } => {
            def_pos.entry(*catch_param).or_insert(pos);
            for s in body.iter().chain(catch_body.iter()) {
                if let LirStmt::Let { name, .. } = s {
                    def_pos.entry(*name).or_insert(pos);
                }
                collect_defs_nested(s, pos, def_pos);
            }
        }
        LirStmt::Loop {
            cond_stmts, body, ..
        } => {
            for s in cond_stmts.iter().chain(body.iter()) {
                if let LirStmt::Let { name, .. } = s {
                    def_pos.entry(*name).or_insert(pos);
                }
                collect_defs_nested(s, pos, def_pos);
            }
        }
        LirStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for case in cases {
                for s in &case.body {
                    if let LirStmt::Let { name, .. } = s {
                        def_pos.entry(*name).or_insert(pos);
                    }
                    collect_defs_nested(s, pos, def_pos);
                }
            }
            for s in default_body {
                if let LirStmt::Let { name, .. } = s {
                    def_pos.entry(*name).or_insert(pos);
                }
                collect_defs_nested(s, pos, def_pos);
            }
        }
        LirStmt::Let { .. } => {}
        LirStmt::FieldUpdate { .. } => {}
    }
}

/// Collect all variable references (Var atoms) from a statement and its nested bodies.
fn collect_refs_in_stmt(stmt: &LirStmt, refs: &mut HashSet<Symbol>) {
    match stmt {
        LirStmt::Let { expr, .. } => collect_refs_in_expr(expr, refs),
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            collect_refs_in_atom(cond, refs);
            for s in then_body {
                collect_refs_in_stmt(s, refs);
            }
            for s in else_body {
                collect_refs_in_stmt(s, refs);
            }
        }
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ..
        } => {
            collect_refs_in_atom(cond, refs);
            for s in then_body {
                collect_refs_in_stmt(s, refs);
            }
            if let Some(r) = then_ret {
                collect_refs_in_atom(r, refs);
            }
            for s in else_body {
                collect_refs_in_stmt(s, refs);
            }
            if let Some(r) = else_ret {
                collect_refs_in_atom(r, refs);
            }
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_body,
            catch_ret,
            ..
        } => {
            for s in body {
                collect_refs_in_stmt(s, refs);
            }
            if let Some(r) = body_ret {
                collect_refs_in_atom(r, refs);
            }
            for s in catch_body {
                collect_refs_in_stmt(s, refs);
            }
            if let Some(r) = catch_ret {
                collect_refs_in_atom(r, refs);
            }
        }
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            for s in cond_stmts {
                collect_refs_in_stmt(s, refs);
            }
            collect_refs_in_atom(cond, refs);
            for s in body {
                collect_refs_in_stmt(s, refs);
            }
        }
        LirStmt::Switch {
            tag,
            cases,
            default_body,
            default_ret,
            ..
        } => {
            collect_refs_in_atom(tag, refs);
            for case in cases {
                for s in &case.body {
                    collect_refs_in_stmt(s, refs);
                }
                if let Some(r) = &case.ret {
                    collect_refs_in_atom(r, refs);
                }
            }
            for s in default_body {
                collect_refs_in_stmt(s, refs);
            }
            if let Some(r) = default_ret {
                collect_refs_in_atom(r, refs);
            }
        }
        LirStmt::FieldUpdate { target, value, .. } => {
            collect_refs_in_atom(target, refs);
            collect_refs_in_atom(value, refs);
        }
    }
}

fn collect_refs_in_expr(expr: &LirExpr, refs: &mut HashSet<Symbol>) {
    match expr {
        LirExpr::Atom(a) => collect_refs_in_atom(a, refs),
        LirExpr::Binary { lhs, rhs, .. } => {
            collect_refs_in_atom(lhs, refs);
            collect_refs_in_atom(rhs, refs);
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            for (_, a) in args {
                collect_refs_in_atom(a, refs);
            }
        }
        LirExpr::Constructor { args, .. } => {
            for a in args {
                collect_refs_in_atom(a, refs);
            }
        }
        LirExpr::Record { fields, .. } => {
            for (_, a) in fields {
                collect_refs_in_atom(a, refs);
            }
        }
        LirExpr::ObjectTag { value, .. } | LirExpr::ObjectField { value, .. } => {
            collect_refs_in_atom(value, refs);
        }
        LirExpr::Raise { value, .. } | LirExpr::Force { value, .. } => {
            collect_refs_in_atom(value, refs)
        }
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::Closure { captures, .. } => {
            for (_, a) in captures {
                collect_refs_in_atom(a, refs);
            }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            collect_refs_in_atom(callee, refs);
            for (_, a) in args {
                collect_refs_in_atom(a, refs);
            }
        }
        LirExpr::LazySpawn { thunk, .. } => collect_refs_in_atom(thunk, refs),
        LirExpr::LazyJoin { task_id, .. } => collect_refs_in_atom(task_id, refs),
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                collect_refs_in_atom(a, refs);
            }
        }
    }
}

fn collect_refs_in_atom(atom: &LirAtom, refs: &mut HashSet<Symbol>) {
    if let LirAtom::Var { name, .. } = atom {
        refs.insert(*name);
    }
}

/// Array literals lower to `LirExpr::Record` with field names "0".."n-1"
/// (see lir_lower.rs MirExpr::Array). Plain records always have alphabetic
/// field names, so this sequence-of-decimals shape uniquely identifies an
/// array — letting the record codegen finish with `(ptr<<32) | len` packing
/// instead of the zero-extended pointer used for ordinary records.
///
/// Zero-field records (both `{}` and `[||]` lower to no-fields) keep the
/// legacy form: packing them as length-0 arrays would corrupt later record
/// field access of `{}` (low 32 bits of the atom must remain the pointer).
/// `[||]`'s length is therefore still subject to the old layout — preserved
/// as a pre-existing corner case rather than introducing new behavior here.
fn is_sequential_numeric_field_names(fields: &[(Symbol, LirAtom)]) -> bool {
    !fields.is_empty()
        && fields
            .iter()
            .enumerate()
            .all(|(idx, (name, _))| name.to_string() == idx.to_string())
}

pub(super) fn compile_expr(
    expr: &LirExpr,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    program: &LirProgram,
    internal_indices: &HashMap<Symbol, u32>,
    external_indices: &HashMap<Symbol, u32>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
    _function_ret_type: &Type,
    in_try: bool,
    _is_entrypoint: bool,
    tco_loop: Option<TcoLoop>,
) -> Result<(), CodegenError> {
    match expr {
        LirExpr::Atom(atom) => compile_atom(atom, out, local_map, layout),
        LirExpr::Binary { op, lhs, rhs, typ } => {
            if is_string_concat_operator(*op, typ) {
                return emit_string_concat(lhs, rhs, out, local_map, layout, temps);
            }
            if is_string_compare_operator(*op, &lhs.typ(), &rhs.typ()) {
                return emit_string_compare(*op, lhs, rhs, out, local_map, layout, temps);
            }
            let operand_type = binary_operand_type(*op, &lhs.typ(), &rhs.typ())?;
            compile_atom(lhs, out, local_map, layout)?;
            emit_numeric_coercion(&lhs.typ(), &operand_type, out)?;
            compile_atom(rhs, out, local_map, layout)?;
            emit_numeric_coercion(&rhs.typ(), &operand_type, out)?;
            compile_binary(*op, &operand_type, out)
        }
        LirExpr::Call { func, args, .. } | LirExpr::TailCall { func, args, .. } => {
            let is_tail = matches!(expr, LirExpr::TailCall { .. }) && !in_try;

            // Self-recursion-to-loop: self-tail-call → param reassignment + br
            if is_tail {
                if let Some(tco) = tco_loop {
                    if *func == tco.self_name {
                        let callee = program
                            .functions
                            .iter()
                            .find(|f| f.name == *func)
                            .ok_or_else(|| CodegenError::CallTargetNotFound {
                                name: func.to_string(),
                            })?;

                        // Push all arg values onto the WASM stack (in param order)
                        for ((_label, atom), param) in args.iter().zip(callee.params.iter()) {
                            compile_atom(atom, out, local_map, layout)?;
                            emit_numeric_coercion(&atom.typ(), &param.typ, out)?;
                        }
                        // Pop into params in reverse order (WASM stack is LIFO)
                        // This ensures atomic read-before-write for swaps like (a: b, b: a)
                        for param in callee.params.iter().rev() {
                            let local = local_map.get(&param.name).ok_or_else(|| {
                                CodegenError::ConflictingLocalTypes {
                                    name: param.name.to_string(),
                                }
                            })?;
                            out.instruction(&Instruction::LocalSet(local.index));
                        }
                        // No bt_pop — we stay in the same function frame
                        out.instruction(&Instruction::Br(tco.loop_depth));
                        return Ok(());
                    }
                }
            }

            if let Some(callee_idx) = internal_indices.get(func).copied() {
                let callee = program
                    .functions
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: func.to_string(),
                    })?;

                if args.len() != callee.params.len() {
                    return Err(CodegenError::CallArityMismatch {
                        name: func.to_string(),
                        expected: callee.params.len(),
                        got: args.len(),
                    });
                }

                // Args are pre-sorted by label at LIR lowering; emit in order
                for ((_label, atom), param) in args.iter().zip(callee.params.iter()) {
                    compile_atom(atom, out, local_map, layout)?;
                    emit_numeric_coercion(&atom.typ(), &param.typ, out)?;
                }
                if is_tail {
                    out.instruction(&Instruction::ReturnCall(callee_idx));
                } else {
                    out.instruction(&Instruction::Call(callee_idx));
                }
                return Ok(());
            }

            if let Some(callee_idx) = external_indices.get(func).copied() {
                let callee = program
                    .externals
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: func.to_string(),
                    })?;

                if args.len() != callee.params.len() {
                    return Err(CodegenError::CallArityMismatch {
                        name: func.to_string(),
                        expected: callee.params.len(),
                        got: args.len(),
                    });
                }

                let canonical_string_ret = external_uses_canonical_string_return(callee);

                // For canonical ABI string returns: allocate 8 bytes for retptr
                // (2 x i32 for ptr and len) before emitting args
                if canonical_string_ret {
                    // Allocate 8 bytes on the heap for the return area.
                    // Must be 4-byte aligned (stores two i32 values: ptr + len).
                    if let Some(alloc_idx) = layout.allocate_func_idx {
                        out.instruction(&Instruction::I32Const(8));
                        out.instruction(&Instruction::Call(alloc_idx));
                    } else {
                        // Bump allocator: align to 4 bytes, then bump by 8
                        // aligned = (heap_ptr + 3) & ~3
                        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                        out.instruction(&Instruction::I32Const(3));
                        out.instruction(&Instruction::I32Add);
                        out.instruction(&Instruction::I32Const(-4i32)); // ~3 = 0xFFFFFFFC
                        out.instruction(&Instruction::I32And);
                        // Save aligned ptr as return value, then advance heap
                        out.instruction(&Instruction::LocalTee(temps.object_ptr_i32));
                        out.instruction(&Instruction::I32Const(8));
                        out.instruction(&Instruction::I32Add);
                        out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
                        // Push aligned ptr as result (already in object_ptr_i32)
                        out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                    }
                    // Save retptr to a temp local
                    out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));
                }

                // Args are sorted by label; emit in external's param order
                for param in callee.params.iter() {
                    let atom = args
                        .iter()
                        .find(|(label, _)| *label == param.label)
                        .map(|(_, atom)| atom)
                        .ok_or_else(|| CodegenError::CallArityMismatch {
                            name: func.to_string(),
                            expected: callee.params.len(),
                            got: args.len(),
                        })?;
                    compile_external_arg(atom, &param.typ, out, local_map, layout, temps)?;
                }

                // For canonical ABI: push retptr as the last argument
                if canonical_string_ret {
                    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                }

                if is_tail && !canonical_string_ret {
                    // Cannot use ReturnCall with canonical string returns
                    // because we need to unpack the retptr after the call
                    out.instruction(&Instruction::ReturnCall(callee_idx));
                } else {
                    out.instruction(&Instruction::Call(callee_idx));
                }

                // For canonical ABI: unpack retptr → packed i64
                if canonical_string_ret {
                    emit_canonical_string_return_unpack(
                        out,
                        temps.object_ptr_i32,
                        temps.concat_lhs_ptr_i32,
                        temps.concat_lhs_len_i32,
                    );
                }

                return Ok(());
            }

            Err(CodegenError::CallTargetNotFound {
                name: func.to_string(),
            })
        }
        LirExpr::Constructor { name, args, .. } => {
            if args.is_empty() {
                let tag = constructor_tag(name.as_str(), 0);
                if let Some(&addr) = layout.nullary_ctor_addrs.get(&tag) {
                    // Use pre-allocated static address from data section.
                    // The match compilation can dereference this as a pointer.
                    out.instruction(&Instruction::I32Const(addr as i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    return Ok(());
                }
                // Fallback: heap allocate (shouldn't happen if layout collected correctly)
            }
            {
                emit_alloc_object(out, temps, 1 + args.len(), layout)?;

                out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                out.instruction(&Instruction::I64Const(constructor_tag(
                    name.as_str(),
                    args.len(),
                )));
                out.instruction(&Instruction::I64Store(memarg(0)));

                for (idx, arg) in args.iter().enumerate() {
                    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                    compile_atom(arg, out, local_map, layout)?;
                    emit_typed_field_store(&arg.typ(), ((idx + 1) * 8) as u64, out)?;
                }

                out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                out.instruction(&Instruction::I64ExtendI32U);
            }
            Ok(())
        }
        LirExpr::Record { fields, .. } => {
            let mut field_names: Vec<String> =
                fields.iter().map(|(name, _)| name.to_string()).collect();
            field_names.sort();
            let tag = record_tag(&field_names);

            emit_alloc_object(out, temps, 1 + fields.len(), layout)?;

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Const(tag));
            out.instruction(&Instruction::I64Store(memarg(0)));

            for (idx, (_, value)) in fields.iter().enumerate() {
                out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                compile_atom(value, out, local_map, layout)?;
                emit_typed_field_store(&value.typ(), ((idx + 1) * 8) as u64, out)?;
            }

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
            // Array literals (`[| e0, ..., e_{n-1} |]`) lower to LirExpr::Record
            // with field names "0".."n-1" (lir_lower.rs MirExpr::Array). For that
            // shape, finish with packed `(ptr<<32) | len` so externs that take
            // `&[| T |]` (e.g. __nx_array_length) see the canonical (ptr, len)
            // pair after `unpack_packed_i64_to_ptr_len`. Plain records keep the
            // zero-extended pointer (high 32 bits = 0).
            if is_sequential_numeric_field_names(fields) {
                out.instruction(&Instruction::I64Const(32));
                out.instruction(&Instruction::I64Shl);
                out.instruction(&Instruction::I64Const(fields.len() as i64));
                out.instruction(&Instruction::I64Or);
            }
            Ok(())
        }
        LirExpr::ObjectTag { value, .. } => {
            compile_atom(value, out, local_map, layout)?;
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Load(memarg(0)));
            Ok(())
        }
        LirExpr::ObjectField { value, index, typ } => {
            compile_atom(value, out, local_map, layout)?;
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            emit_typed_field_load(typ, ((index + 1) * 8) as u64, out)?;
            Ok(())
        }
        LirExpr::Raise { value, .. } => {
            let tag_idx = layout
                .exn_tag_idx
                .expect("exn_tag_idx must be set for programs with raise");
            // Capture backtrace via stack walk before throwing
            if let Some(bt_idx) = layout.capture_bt_func_idx {
                out.instruction(&Instruction::Call(bt_idx));
            }
            compile_atom(value, out, local_map, layout)?;
            if matches!(value.typ(), Type::Unit) {
                // Unit-typed raise: push i64(0) as payload
                out.instruction(&Instruction::I64Const(0));
            }
            out.instruction(&Instruction::Throw(tag_idx));
            Ok(())
        }
        LirExpr::Force { value, .. } => {
            compile_atom(value, out, local_map, layout)?;
            Ok(())
        }
        LirExpr::FuncRef { func, .. } => {
            // Allocate closure object [table_idx] (1 word)
            let table_idx = layout
                .funcref_table_indices
                .get(func)
                .copied()
                .ok_or_else(|| CodegenError::CallTargetNotFound {
                    name: func.to_string(),
                })?;
            emit_alloc_object(out, temps, 1, layout)?;
            // Store table_idx at offset 0
            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Const(table_idx as i64));
            out.instruction(&Instruction::I64Store(memarg(0)));
            // Return closure ptr as i64
            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
            Ok(())
        }
        LirExpr::Closure { func, captures, .. } => {
            // Allocate closure object [table_idx, cap0, cap1, ...] (1 + N words)
            let table_idx = layout
                .funcref_table_indices
                .get(func)
                .copied()
                .ok_or_else(|| CodegenError::CallTargetNotFound {
                    name: func.to_string(),
                })?;
            let n_words = 1 + captures.len();
            emit_alloc_object(out, temps, n_words, layout)?;
            // Store table_idx at offset 0
            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Const(table_idx as i64));
            out.instruction(&Instruction::I64Store(memarg(0)));
            // Store captures at offsets 8, 16, ...
            for (i, (_, atom)) in captures.iter().enumerate() {
                out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                compile_atom(atom, out, local_map, layout)?;
                emit_typed_field_store(&atom.typ(), ((i + 1) * 8) as u64, out)?;
            }
            // Return closure ptr as i64
            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
            Ok(())
        }
        LirExpr::ClosureEnvLoad { index, typ } => {
            // Load captured value from __env (first function parameter, local 0)
            out.instruction(&Instruction::LocalGet(0)); // __env: i64
            out.instruction(&Instruction::I32WrapI64); // convert to address
            emit_typed_field_load(typ, ((index + 1) * 8) as u64, out)?;
            Ok(())
        }
        LirExpr::CallIndirect {
            callee,
            args,
            callee_type,
            ..
        } => {
            // Look up the type index for call_indirect (with __env prefix)
            let callee_type_key = format!("{:?}", callee_type);
            let type_index = layout
                .indirect_type_indices
                .get(&callee_type_key)
                .copied()
                .ok_or_else(|| CodegenError::CallTargetNotFound {
                    name: format!("indirect call type: {:?}", callee_type),
                })?;

            // Save callee closure pointer to temp
            compile_atom(callee, out, local_map, layout)?;
            out.instruction(&Instruction::LocalSet(temps.closure_ptr_i64));

            // Load table_idx from closure[0]
            out.instruction(&Instruction::LocalGet(temps.closure_ptr_i64));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(memarg(0)));
            out.instruction(&Instruction::LocalSet(temps.closure_table_idx_i64));

            // Push __env (closure pointer) as first argument
            out.instruction(&Instruction::LocalGet(temps.closure_ptr_i64));

            // Push normal args onto stack
            if let Type::Arrow(param_types, _, _, _) = callee_type {
                for ((_, atom), (_, param_type)) in args.iter().zip(param_types.iter()) {
                    compile_atom(atom, out, local_map, layout)?;
                    emit_numeric_coercion(&atom.typ(), param_type, out)?;
                }
            } else {
                for (_, atom) in args {
                    compile_atom(atom, out, local_map, layout)?;
                }
            }

            // Push table index (i32) for call_indirect
            out.instruction(&Instruction::LocalGet(temps.closure_table_idx_i64));
            out.instruction(&Instruction::I32WrapI64);

            // Emit call_indirect
            out.instruction(&Instruction::CallIndirect {
                type_index,
                table_index: 0,
            });
            Ok(())
        }
        LirExpr::LazySpawn {
            thunk,
            num_captures,
            ..
        } => {
            // __nx_lazy_spawn(thunk_ptr: i64, num_captures: i32) -> i64
            let spawn_idx =
                layout
                    .lazy_spawn_func_idx
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: "__nx_lazy_spawn".to_string(),
                    })?;
            compile_atom(thunk, out, local_map, layout)?;
            out.instruction(&Instruction::I32Const(*num_captures as i32));
            out.instruction(&Instruction::Call(spawn_idx));
            Ok(())
        }
        LirExpr::LazyJoin { task_id, .. } => {
            // __nx_lazy_join(task_id: i64) -> i64
            let join_idx =
                layout
                    .lazy_join_func_idx
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: "__nx_lazy_join".to_string(),
                    })?;
            compile_atom(task_id, out, local_map, layout)?;
            out.instruction(&Instruction::Call(join_idx));
            Ok(())
        }
        LirExpr::Intrinsic { kind, args, .. } => {
            use crate::ir::lir::Intrinsic;
            match kind {
                Intrinsic::StringByteLength => {
                    // packed_i64 & 0xFFFFFFFF → len as i64
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    compile_atom(s, out, local_map, layout)?;
                    out.instruction(&Instruction::I64Const(0xFFFF_FFFF_u64 as i64));
                    out.instruction(&Instruction::I64And);
                    Ok(())
                }
                Intrinsic::StringByteAt => {
                    // ptr = (packed >> 32) as i32; load8_u(ptr + idx as i32)
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let idx = &args.iter().find(|(l, _)| l.as_ref() == "idx").unwrap().1;
                    compile_atom(s, out, local_map, layout)?;
                    let packed_tmp = temps.packed_tmp_i64;
                    out.instruction(&Instruction::LocalSet(packed_tmp));
                    // ptr = high 32 bits
                    out.instruction(&Instruction::LocalGet(packed_tmp));
                    out.instruction(&Instruction::I64Const(32));
                    out.instruction(&Instruction::I64ShrU);
                    out.instruction(&Instruction::I32WrapI64);
                    // idx
                    compile_atom(idx, out, local_map, layout)?;
                    out.instruction(&Instruction::I32WrapI64);
                    // ptr + idx
                    out.instruction(&Instruction::I32Add);
                    // load byte
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    // Result is i32. The LIR type says i32 (from WIT s32).
                    // But internally Nexus uses i64 for everything except i32-typed externals.
                    // The caller wraps to i64 if needed via type coercion.
                    Ok(())
                }
                Intrinsic::SkipWs => {
                    // skip_ws(s, start) → scan forward while whitespace, return end pos as i64
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    // Loop: while idx < len && is_whitespace(byte)
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    // if idx >= len: break
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1));
                    // load byte
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32));
                    // is_ws = (byte==0x20) | (byte==0x09) | (byte==0x0A) | (byte==0x0D)
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(0x20));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(0x09));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::I32Or);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(0x0A));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::I32Or);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(0x0D));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::I32Or);
                    // if NOT whitespace: break
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::BrIf(1));
                    // idx++
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // loop
                    out.instruction(&Instruction::End); // block
                                                        // result: idx as i64
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    Ok(())
                }
                Intrinsic::ScanIdent => {
                    // scan_ident(s, start) → scan while [a-zA-Z0-9_]
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1));
                    // load byte
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32));
                    // is_ident = (byte>='a' && byte<='z') || (byte>='A' && byte<='Z')
                    //         || (byte>='0' && byte<='9') || byte=='_'
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'a' as i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'z' as i32));
                    out.instruction(&Instruction::I32LeU);
                    out.instruction(&Instruction::I32And);
                    // || upper
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'A' as i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'Z' as i32));
                    out.instruction(&Instruction::I32LeU);
                    out.instruction(&Instruction::I32And);
                    out.instruction(&Instruction::I32Or);
                    // || digit
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'0' as i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'9' as i32));
                    out.instruction(&Instruction::I32LeU);
                    out.instruction(&Instruction::I32And);
                    out.instruction(&Instruction::I32Or);
                    // || underscore
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'_' as i32));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::I32Or);
                    // if NOT ident: break
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::BrIf(1));
                    // idx++
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // loop
                    out.instruction(&Instruction::End); // block
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    Ok(())
                }
                Intrinsic::ScanDigits => {
                    // scan_digits(s, start) → scan while [0-9]
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32));
                    // is_digit = byte >= '0' && byte <= '9'
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'0' as i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Const(b'9' as i32));
                    out.instruction(&Instruction::I32LeU);
                    out.instruction(&Instruction::I32And);
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::BrIf(1));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    Ok(())
                }
                Intrinsic::FindByte => {
                    // find_byte(s, start, ch) → first index of ch, or -1
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    let ch = &args.iter().find(|(l, _)| l.as_ref() == "ch").unwrap().1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    // ch → i32 (char is already i32; i64 needs wrapping)
                    compile_atom(ch, out, local_map, layout)?;
                    if matches!(ch.typ().wasm_repr(), crate::types::WasmRepr::I64) {
                        out.instruction(&Instruction::I32WrapI64);
                    }
                    out.instruction(&Instruction::LocalSet(temps.scan_end_i32));
                    // Loop: scan forward for matching byte
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1)); // not found
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::BrIf(1)); // found — exit with idx valid
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // loop
                    out.instruction(&Instruction::End); // block
                                                        // Result: idx < len ? idx as i64 : -1i64
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    out.instruction(&Instruction::I64Const(-1i64));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32LtU);
                    out.instruction(&Instruction::Select);
                    Ok(())
                }
                Intrinsic::CountNewlinesIn => {
                    // count_newlines_in(s, start, end_pos) → count of '\n' in [start, end)
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    let end_pos = &args
                        .iter()
                        .find(|(l, _)| l.as_ref() == "end_pos")
                        .unwrap()
                        .1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    // Clamp end_pos to [0, len]
                    emit_intrinsic_clamp_arg(
                        end_pos,
                        temps.scan_end_i32,
                        temps.concat_lhs_len_i32,
                        out,
                        local_map,
                        layout,
                        temps,
                    )?;
                    // count = 0
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32)); // reuse as counter
                                                                                  // Loop
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1));
                    // count += (byte == '\n')  — branchless
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Const(0x0A));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32));
                    // idx++
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    Ok(())
                }
                Intrinsic::LastNewlineIn => {
                    // last_newline_in(s, start, end_pos) → last '\n' pos in [start, end), or -1
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    let end_pos = &args
                        .iter()
                        .find(|(l, _)| l.as_ref() == "end_pos")
                        .unwrap()
                        .1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    // scan_end_i32 = clamped start (save for final comparison)
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32)); // save start
                                                                                  // Clamp end_pos, then set idx = end - 1
                    emit_intrinsic_clamp_arg(
                        end_pos,
                        temps.scan_end_i32,
                        temps.concat_lhs_len_i32,
                        out,
                        local_map,
                        layout,
                        temps,
                    )?;
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    // Backward loop
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    // if idx < start: break (not found)
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32)); // saved start
                    out.instruction(&Instruction::I32LtS);
                    out.instruction(&Instruction::BrIf(1));
                    // load byte
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Const(0x0A));
                    out.instruction(&Instruction::I32Eq);
                    out.instruction(&Instruction::BrIf(1)); // found — exit
                                                            // idx--
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // loop
                    out.instruction(&Instruction::End); // block
                                                        // idx >= start ? idx as i64 : -1i64
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I64ExtendI32S);
                    out.instruction(&Instruction::I64Const(-1i64));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32)); // saved start
                    out.instruction(&Instruction::I32GeS);
                    out.instruction(&Instruction::Select);
                    Ok(())
                }
                Intrinsic::ByteSubstring => {
                    // byte_substring(s, start, len) → new packed string from byte slice
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let start = &args.iter().find(|(l, _)| l.as_ref() == "start").unwrap().1;
                    let sub_len = &args.iter().find(|(l, _)| l.as_ref() == "len").unwrap().1;
                    emit_intrinsic_unpack_and_clamp(s, start, out, local_map, layout, temps)?;
                    // sub_len = min(requested_len, remaining_bytes)
                    // remaining = src_len - start_clamped
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::LocalSet(temps.scan_end_i32)); // remaining
                                                                                 // clamp sub_len to [0, remaining]
                    compile_atom(sub_len, out, local_map, layout)?;
                    out.instruction(&Instruction::I32WrapI64);
                    out.instruction(&Instruction::LocalTee(temps.concat_out_len_i32));
                    // max(sub_len, 0)
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::I32GeS);
                    out.instruction(&Instruction::Select);
                    out.instruction(&Instruction::LocalSet(temps.concat_out_len_i32));
                    // min(sub_len, remaining)
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32LeU);
                    out.instruction(&Instruction::Select);
                    out.instruction(&Instruction::LocalSet(temps.concat_out_len_i32));
                    // Allocate output buffer
                    if let Some(alloc_idx) = layout.allocate_func_idx {
                        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                        out.instruction(&Instruction::Call(alloc_idx));
                        out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
                    } else {
                        out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
                        out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
                        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                        out.instruction(&Instruction::I32Add);
                        out.instruction(&Instruction::GlobalSet(STRING_HEAP_GLOBAL_INDEX));
                        emit_string_heap_grow(out, layout);
                    }
                    // Copy bytes: src[start..start+sub_len] → out[0..sub_len]
                    // Reuse concat_rhs_ptr_i32 as copy index
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1));
                    // dst[i] = src[start + i]
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Store8(super::string::memarg_i8()));
                    // i++
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::End);
                    // Pack result: (out_ptr << 32) | out_len
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    out.instruction(&Instruction::I64Const(32));
                    out.instruction(&Instruction::I64Shl);
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    out.instruction(&Instruction::I64Or);
                    Ok(())
                }
                Intrinsic::StringLength => {
                    // length(s) → character count.
                    // BYTE-COUNT shortcut: this lowering only matches the
                    // contract for ASCII input. Currently unreachable —
                    // `lir_lower::try_intrinsify` skips StringLength so the
                    // call routes to the UTF-8-aware runtime extern. Kept
                    // as a placeholder for a future UTF-8 lead-byte counter.
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    compile_atom(s, out, local_map, layout)?;
                    out.instruction(&Instruction::I64Const(0xFFFF_FFFF_u64 as i64));
                    out.instruction(&Instruction::I64And);
                    Ok(())
                }
                Intrinsic::CharCode => {
                    // char_code(s, idx) → codepoint at character index as i64.
                    // BYTE-INDEX shortcut (ASCII only). Currently unreachable —
                    // see the StringLength comment above.
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let idx = &args.iter().find(|(l, _)| l.as_ref() == "idx").unwrap().1;
                    compile_atom(s, out, local_map, layout)?;
                    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::I64Const(32));
                    out.instruction(&Instruction::I64ShrU);
                    out.instruction(&Instruction::I32WrapI64);
                    compile_atom(idx, out, local_map, layout)?;
                    out.instruction(&Instruction::I32WrapI64);
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    // Result as i64 (char_code returns s64 in WIT)
                    out.instruction(&Instruction::I64ExtendI32U);
                    Ok(())
                }
                Intrinsic::CharAt => {
                    // char_at(s, idx) → char (i32) at character index.
                    // BYTE-INDEX shortcut (ASCII only). Currently unreachable —
                    // see the StringLength comment above.
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let idx = &args.iter().find(|(l, _)| l.as_ref() == "idx").unwrap().1;
                    compile_atom(s, out, local_map, layout)?;
                    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::I64Const(32));
                    out.instruction(&Instruction::I64ShrU);
                    out.instruction(&Instruction::I32WrapI64);
                    compile_atom(idx, out, local_map, layout)?;
                    out.instruction(&Instruction::I32WrapI64);
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    Ok(())
                }
                Intrinsic::FromCharCode => {
                    // from_char_code(code: i64) → UTF-8 encoded string.
                    // Validates that `code` is a valid Unicode scalar value
                    // (0..=0x10FFFF, excluding surrogates 0xD800..=0xDFFF) and
                    // raises Exn::InvalidUnicode otherwise.
                    let code = &args.iter().find(|(l, _)| l.as_ref() == "code").unwrap().1;
                    compile_atom(code, out, local_map, layout)?;
                    // Stack: code:i64
                    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
                    emit_from_codepoint_utf8(out, layout, temps)?;
                    Ok(())
                }
                Intrinsic::FromChar => {
                    // from_char(c: char) → UTF-8 encoded string.
                    // Same validation/encoding contract as FromCharCode; the
                    // input is widened from i32 to i64 before validation.
                    let c = &args.iter().find(|(l, _)| l.as_ref() == "c").unwrap().1;
                    compile_atom(c, out, local_map, layout)?;
                    if matches!(c.typ().wasm_repr(), crate::types::WasmRepr::I32) {
                        out.instruction(&Instruction::I64ExtendI32U);
                    }
                    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
                    emit_from_codepoint_utf8(out, layout, temps)?;
                    Ok(())
                }
                Intrinsic::CharOrd => {
                    // char_ord(c) → i64: just extend char (i32) to i64
                    let c = &args.iter().find(|(l, _)| l.as_ref() == "c").unwrap().1;
                    compile_atom(c, out, local_map, layout)?;
                    if matches!(c.typ().wasm_repr(), crate::types::WasmRepr::I32) {
                        out.instruction(&Instruction::I64ExtendI32U);
                    }
                    Ok(())
                }
                Intrinsic::StartsWith => {
                    // starts_with(s, prefix) → bool (i32 0/1, WIT bool = i32)
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let prefix = &args.iter().find(|(l, _)| l.as_ref() == "prefix").unwrap().1;
                    emit_intrinsic_unpack_two(s, prefix, out, local_map, layout, temps)?;
                    // if prefix_len > s_len: return 0
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
                        ValType::I32,
                    )));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GtU);
                    out.instruction(&Instruction::BrIf(0)); // → false block
                                                            // Compare bytes: loop i from 0 to prefix_len
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1)); // done → match
                                                            // if s[i] != prefix[i]: break → false
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Ne);
                    out.instruction(&Instruction::BrIf(2)); // mismatch → false block
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // loop
                    out.instruction(&Instruction::End); // inner block
                                                        // All matched → true
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::Br(1)); // exit result block with 1
                    out.instruction(&Instruction::End); // false block
                                                        // false
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::End); // result block
                    Ok(())
                }
                Intrinsic::EndsWith => {
                    // ends_with(s, suffix) → bool (i32 0/1, WIT bool = i32)
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let suffix = &args.iter().find(|(l, _)| l.as_ref() == "suffix").unwrap().1;
                    emit_intrinsic_unpack_two(s, suffix, out, local_map, layout, temps)?;
                    // if suffix_len > s_len: return 0
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
                        ValType::I32,
                    )));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GtU);
                    out.instruction(&Instruction::BrIf(0));
                    // offset = s_len - suffix_len
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::LocalSet(temps.scan_end_i32));
                    // Compare bytes: loop i from 0 to suffix_len
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1));
                    // if s[offset + i] != suffix[i]: break
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Ne);
                    out.instruction(&Instruction::BrIf(2));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // loop
                    out.instruction(&Instruction::End); // inner block
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::Br(1)); // exit result block with 1
                    out.instruction(&Instruction::End); // false block
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::End); // result block
                    Ok(())
                }
                Intrinsic::IndexOf => {
                    // index_of(s, sub) → i64: first occurrence or -1
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let sub = &args.iter().find(|(l, _)| l.as_ref() == "sub").unwrap().1;
                    emit_intrinsic_unpack_two(s, sub, out, local_map, layout, temps)?;
                    // Result block returning i64
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
                        ValType::I64,
                    )));
                    // if sub_len == 0: return 0
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::I64Const(0));
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    // if sub_len > s_len: return -1
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GtU);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::I64Const(-1i64));
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    // scan_end = s_len - sub_len + 1 (exclusive upper bound for pos)
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.scan_end_i32));
                    // pos = 0
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32)); // reuse as pos
                                                                                       // Outer loop over positions
                                                                                       // Structure: result{ outer_block{ outer_loop{ found_wrap{ inner_block{ inner_loop } } pos++ } } -1 }
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // outer block (d1)
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty)); // outer loop (d2)
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1)); // → outer block end → not found
                                                            // found_wrapper block: mismatch branches here to skip "found"
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // found_wrap (d3)
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // inner block (d4)
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty)); // inner loop (d5)
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1)); // all matched → inner block end
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Ne);
                    out.instruction(&Instruction::BrIf(2)); // mismatch → found_wrap end → pos++
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // inner loop
                    out.instruction(&Instruction::End); // inner block
                                                        // All bytes matched → return pos
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    out.instruction(&Instruction::Br(3)); // exit result block
                    out.instruction(&Instruction::End); // found_wrap
                                                        // Mismatch: next pos
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::Br(0)); // → outer loop
                    out.instruction(&Instruction::End); // outer loop
                    out.instruction(&Instruction::End); // outer block
                                                        // Not found
                    out.instruction(&Instruction::I64Const(-1i64));
                    out.instruction(&Instruction::End); // result block
                    Ok(())
                }
                Intrinsic::Contains => {
                    // contains(s, sub) → bool (i32 0/1, WIT bool = i32)
                    let s = &args.iter().find(|(l, _)| l.as_ref() == "s").unwrap().1;
                    let sub = &args.iter().find(|(l, _)| l.as_ref() == "sub").unwrap().1;
                    emit_intrinsic_unpack_two(s, sub, out, local_map, layout, temps)?;
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
                        ValType::I32,
                    )));
                    // if sub_len == 0: return true
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    // if sub_len > s_len: return false
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::I32GtU);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    // scan_end = s_len - sub_len + 1
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
                    // Structure: result{ outer_block{ outer_loop{ found_wrap{ inner_block{ inner_loop } } pos++ } } 0 }
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // outer block (d1)
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty)); // outer loop (d2)
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_end_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1)); // → outer block end → not found
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // found_wrap (d3)
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty)); // inner block (d4)
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty)); // inner loop (d5)
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
                    out.instruction(&Instruction::I32GeU);
                    out.instruction(&Instruction::BrIf(1)); // all matched → inner block end
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Load8U(super::string::memarg_i8()));
                    out.instruction(&Instruction::I32Ne);
                    out.instruction(&Instruction::BrIf(2)); // mismatch → found_wrap end → pos++
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(0));
                    out.instruction(&Instruction::End); // inner loop
                    out.instruction(&Instruction::End); // inner block
                                                        // Found
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::Br(3)); // exit result block
                    out.instruction(&Instruction::End); // found_wrap
                                                        // Mismatch: next pos
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::Br(0)); // → outer loop
                    out.instruction(&Instruction::End); // outer loop
                    out.instruction(&Instruction::End); // outer block
                                                        // Not found
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::End); // result block
                    Ok(())
                }
                Intrinsic::FromI64 => {
                    // from_i64(val) → string: decimal representation
                    let val = &args.iter().find(|(l, _)| l.as_ref() == "val").unwrap().1;
                    compile_atom(val, out, local_map, layout)?;
                    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64)); // val
                                                                                   // Result block
                    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
                        ValType::I64,
                    )));
                    // if val == 0: allocate "0", return packed
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::I64Eqz);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    emit_intrinsic_alloc_store_byte(b'0', out, layout, temps);
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    // is_negative = val < 0 → scan_byte_i32
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::I64Const(0));
                    out.instruction(&Instruction::I64LtS);
                    out.instruction(&Instruction::LocalSet(temps.scan_byte_i32));
                    // if negative: val = 0 - val
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::I64Const(0));
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::I64Sub);
                    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::End);
                    // Count digits: concat_idx_i32 = digit count
                    out.instruction(&Instruction::I32Const(0));
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64)); // tmp
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::I64Eqz);
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::I64Const(10));
                    out.instruction(&Instruction::I64DivU);
                    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::End); // loop
                                                        // total_len = digits + is_negative
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalSet(temps.concat_out_len_i32));
                    // Allocate buffer
                    if let Some(alloc_idx) = layout.allocate_func_idx {
                        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                        out.instruction(&Instruction::Call(alloc_idx));
                        out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
                    } else {
                        out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
                        out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
                        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                        out.instruction(&Instruction::I32Add);
                        out.instruction(&Instruction::GlobalSet(STRING_HEAP_GLOBAL_INDEX));
                        emit_string_heap_grow(out, layout);
                    }
                    // Write digits right-to-left
                    // write_idx = total_len - 1
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    // Restore val (abs) for digit extraction
                    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
                    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::I64Eqz);
                    out.instruction(&Instruction::I32Eqz);
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    // buf[write_idx] = '0' + (val % 10)
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::I64Const(10));
                    out.instruction(&Instruction::I64RemU);
                    out.instruction(&Instruction::I32WrapI64);
                    out.instruction(&Instruction::I32Const(b'0' as i32));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::I32Store8(super::string::memarg_i8()));
                    // val /= 10
                    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
                    out.instruction(&Instruction::I64Const(10));
                    out.instruction(&Instruction::I64DivU);
                    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
                    // write_idx--
                    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
                    out.instruction(&Instruction::I32Const(1));
                    out.instruction(&Instruction::I32Sub);
                    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
                    out.instruction(&Instruction::Br(1));
                    out.instruction(&Instruction::End);
                    out.instruction(&Instruction::End); // loop
                                                        // if negative: buf[0] = '-'
                    out.instruction(&Instruction::LocalGet(temps.scan_byte_i32));
                    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I32Const(b'-' as i32));
                    out.instruction(&Instruction::I32Store8(super::string::memarg_i8()));
                    out.instruction(&Instruction::End);
                    // Pack result: (out_ptr << 32) | total_len
                    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    out.instruction(&Instruction::I64Const(32));
                    out.instruction(&Instruction::I64Shl);
                    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
                    out.instruction(&Instruction::I64ExtendI32U);
                    out.instruction(&Instruction::I64Or);
                    out.instruction(&Instruction::End); // result block
                    Ok(())
                }
                Intrinsic::HeapMark => {
                    // Three modes:
                    // - lazy host arena (shared-memory threading): the host-side
                    //   AtomicI32 bump pointer is the *only* allocator. mark
                    //   returns the current pointer extended to i64; G0 is
                    //   not in play.
                    // - stdlib mark/reset: pack stdlib outstanding count
                    //   (upper 32 bits) alongside G0 (lower 32 bits) so
                    //   heap_reset can free both.
                    // - no allocator hook: just snapshot G0.
                    if layout.lazy_host_arena {
                        let mark_idx = layout.alloc_mark_func_idx.expect(
                            "lazy_host_arena set without alloc_mark_func_idx — \
                             codegen invariant violation",
                        );
                        out.instruction(&Instruction::Call(mark_idx));
                        out.instruction(&Instruction::I64ExtendI32U);
                    } else if let Some(mark_idx) = layout.alloc_mark_func_idx {
                        out.instruction(&Instruction::Call(mark_idx));
                        out.instruction(&Instruction::I64ExtendI32U);
                        out.instruction(&Instruction::I64Const(32));
                        out.instruction(&Instruction::I64Shl);
                        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                        out.instruction(&Instruction::I64ExtendI32U);
                        out.instruction(&Instruction::I64Or);
                    } else {
                        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                        out.instruction(&Instruction::I64ExtendI32U);
                    }
                    Ok(())
                }
                Intrinsic::HeapReset => {
                    let mark = &args.iter().find(|(l, _)| l.as_ref() == "mark").unwrap().1;
                    if layout.lazy_host_arena {
                        // Lazy host arena: mark is the saved alloc_ptr; pass
                        // its low 32 bits straight to alloc-reset. No G0 to
                        // touch.
                        let reset_idx = layout.alloc_reset_func_idx.expect(
                            "lazy_host_arena set without alloc_reset_func_idx — \
                             codegen invariant violation",
                        );
                        compile_atom(mark, out, local_map, layout)?;
                        out.instruction(&Instruction::I32WrapI64);
                        out.instruction(&Instruction::Call(reset_idx));
                    } else if let Some(reset_idx) = layout.alloc_reset_func_idx {
                        // Stdlib alloc reset takes the high 32 bits as i32.
                        compile_atom(mark, out, local_map, layout)?;
                        out.instruction(&Instruction::I64Const(32));
                        out.instruction(&Instruction::I64ShrU);
                        out.instruction(&Instruction::I32WrapI64);
                        out.instruction(&Instruction::Call(reset_idx));
                        // Then restore G0 from the low 32 bits.
                        compile_atom(mark, out, local_map, layout)?;
                        out.instruction(&Instruction::I32WrapI64);
                        out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
                    } else {
                        compile_atom(mark, out, local_map, layout)?;
                        out.instruction(&Instruction::I32WrapI64);
                        out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
                    }
                    // HeapReset's Nexus type is `unit`, whose WASM result is
                    // empty. Pushing a sentinel i64 here would mismatch the
                    // surrounding stmt's expected stack (it sees expr_type =
                    // Unit and emits no compensating Drop), so we leave the
                    // stack as the body left it.
                    Ok(())
                }
                Intrinsic::HeapSwap => {
                    // Swap G0 with new base, return old value. Out of scope for
                    // stdlib mark/reset — heap_swap is a region-pivot primitive
                    // that operates only on the bump pointer, leaving stdlib
                    // allocations under the live mark.
                    let base = &args.iter().find(|(l, _)| l.as_ref() == "base").unwrap().1;
                    out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                    out.instruction(&Instruction::I64ExtendI32U);
                    compile_atom(base, out, local_map, layout)?;
                    out.instruction(&Instruction::I32WrapI64);
                    out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
                    Ok(())
                }
            }
        }
    }
}

/// Unpack two packed string i64s into ptr/len locals.
/// s → concat_lhs_ptr_i32, concat_lhs_len_i32
/// other → concat_rhs_ptr_i32, concat_rhs_len_i32
fn emit_intrinsic_unpack_two(
    s: &LirAtom,
    other: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &super::layout::CodegenLayout,
    temps: &super::FunctionTemps,
) -> Result<(), super::error::CodegenError> {
    // Unpack s → lhs ptr, len
    compile_atom(s, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_len_i32));
    // Unpack other → rhs ptr, len
    compile_atom(other, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_len_i32));
    Ok(())
}

/// Allocate 1 byte, store a single byte value, and leave packed i64 on the stack.
fn emit_intrinsic_alloc_store_byte(
    byte: u8,
    out: &mut Function,
    layout: &super::layout::CodegenLayout,
    temps: &super::FunctionTemps,
) {
    if let Some(alloc_idx) = layout.allocate_func_idx {
        out.instruction(&Instruction::I32Const(1));
        out.instruction(&Instruction::Call(alloc_idx));
        out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
    } else {
        out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(1));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::GlobalSet(STRING_HEAP_GLOBAL_INDEX));
        emit_string_heap_grow(out, layout);
    }
    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::I32Const(byte as i32));
    out.instruction(&Instruction::I32Store8(super::string::memarg_i8()));
    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64Shl);
    out.instruction(&Instruction::I64Const(1));
    out.instruction(&Instruction::I64Or);
}

/// Emit codegen for `from_char_code(code) → string` and `from_char(c) → string`.
///
/// Precondition: the candidate codepoint has been written to
/// `temps.packed_tmp_i64` (as a sign-extended i64).
///
/// Validates that the codepoint is a valid Unicode scalar value
/// (0..=0x10FFFF, excluding surrogates 0xD800..=0xDFFF) and raises
/// `Exn::InvalidUnicode(code)` on violation. On success, allocates a
/// 1-4 byte buffer on the string heap, writes the UTF-8 encoding, and
/// leaves the packed (ptr<<32 | len) i64 on the stack.
fn emit_from_codepoint_utf8(
    out: &mut Function,
    layout: &CodegenLayout,
    temps: &super::FunctionTemps,
) -> Result<(), CodegenError> {
    let exn_tag_idx = layout.exn_tag_idx.ok_or(CodegenError::ObjectHeapRequired {
        context: "from_char_code requires Exn tag (is the program raise-aware?)",
    })?;
    let invalid_unicode_tag = constructor_tag("InvalidUnicode", 1);
    let memarg_i8 = super::string::memarg_i8();

    // Result block returns the packed string (i64). Both the validation
    // failure path and the four UTF-8 length cases produce one i64 each.
    out.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));

    // ── Validation ─────────────────────────────────────────────────
    // Compute is_invalid = (code < 0) | (code > 0x10FFFF) | (0xD800 <= code <= 0xDFFF)
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(0));
    out.instruction(&Instruction::I64LtS);
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(0x10FFFF));
    out.instruction(&Instruction::I64GtS);
    out.instruction(&Instruction::I32Or);
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(0xD800));
    out.instruction(&Instruction::I64GeS);
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(0xDFFF));
    out.instruction(&Instruction::I64LeS);
    out.instruction(&Instruction::I32And);
    out.instruction(&Instruction::I32Or);
    // if !is_invalid → branch into the encoding path
    out.instruction(&Instruction::I32Eqz);
    out.instruction(&Instruction::If(BlockType::Empty));
    {
        // ── Valid path ─────────────────────────────────────────────
        // Branch on byte length:
        //   code < 0x80    → 1 byte
        //   code < 0x800   → 2 bytes
        //   code < 0x10000 → 3 bytes
        //   else           → 4 bytes (code <= 0x10FFFF guaranteed by validation)
        out.instruction(&Instruction::Block(BlockType::Empty)); // four_block
        out.instruction(&Instruction::Block(BlockType::Empty)); // three_block
        out.instruction(&Instruction::Block(BlockType::Empty)); // two_block
        out.instruction(&Instruction::Block(BlockType::Empty)); // one_block

        // br_table by byte-class: depth 0=one,1=two,2=three,3=four
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(0x80));
        out.instruction(&Instruction::I64LtU);
        out.instruction(&Instruction::BrIf(0)); // → one_block
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(0x800));
        out.instruction(&Instruction::I64LtU);
        out.instruction(&Instruction::BrIf(1)); // → two_block
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(0x10000));
        out.instruction(&Instruction::I64LtU);
        out.instruction(&Instruction::BrIf(2)); // → three_block
        out.instruction(&Instruction::Br(3)); // → four_block

        // ── 1-byte path ────────────────────────────────────────────
        out.instruction(&Instruction::End); // close one_block
        emit_alloc_string_buf(out, layout, temps, 1);
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        emit_pack_string_result(out, temps, 1);
        out.instruction(&Instruction::Br(4)); // exit the result block

        // ── 2-byte path ────────────────────────────────────────────
        out.instruction(&Instruction::End); // close two_block
        emit_alloc_string_buf(out, layout, temps, 2);
        // byte0 = 0xC0 | (code >> 6)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(6));
        out.instruction(&Instruction::I64ShrU);
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0xC0));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        // byte1 = 0x80 | (code & 0x3F)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(1));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0x3F));
        out.instruction(&Instruction::I32And);
        out.instruction(&Instruction::I32Const(0x80));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        emit_pack_string_result(out, temps, 2);
        out.instruction(&Instruction::Br(3));

        // ── 3-byte path ────────────────────────────────────────────
        out.instruction(&Instruction::End); // close three_block
        emit_alloc_string_buf(out, layout, temps, 3);
        // byte0 = 0xE0 | (code >> 12)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(12));
        out.instruction(&Instruction::I64ShrU);
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0xE0));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        // byte1 = 0x80 | ((code >> 6) & 0x3F)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(1));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(6));
        out.instruction(&Instruction::I64ShrU);
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0x3F));
        out.instruction(&Instruction::I32And);
        out.instruction(&Instruction::I32Const(0x80));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        // byte2 = 0x80 | (code & 0x3F)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(2));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0x3F));
        out.instruction(&Instruction::I32And);
        out.instruction(&Instruction::I32Const(0x80));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        emit_pack_string_result(out, temps, 3);
        out.instruction(&Instruction::Br(2));

        // ── 4-byte path ────────────────────────────────────────────
        out.instruction(&Instruction::End); // close four_block
        emit_alloc_string_buf(out, layout, temps, 4);
        // byte0 = 0xF0 | (code >> 18)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(18));
        out.instruction(&Instruction::I64ShrU);
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0xF0));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        // byte1 = 0x80 | ((code >> 12) & 0x3F)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(1));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(12));
        out.instruction(&Instruction::I64ShrU);
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0x3F));
        out.instruction(&Instruction::I32And);
        out.instruction(&Instruction::I32Const(0x80));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        // byte2 = 0x80 | ((code >> 6) & 0x3F)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(2));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I64Const(6));
        out.instruction(&Instruction::I64ShrU);
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0x3F));
        out.instruction(&Instruction::I32And);
        out.instruction(&Instruction::I32Const(0x80));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        // byte3 = 0x80 | (code & 0x3F)
        out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(3));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
        out.instruction(&Instruction::I32WrapI64);
        out.instruction(&Instruction::I32Const(0x3F));
        out.instruction(&Instruction::I32And);
        out.instruction(&Instruction::I32Const(0x80));
        out.instruction(&Instruction::I32Or);
        out.instruction(&Instruction::I32Store8(memarg_i8));
        emit_pack_string_result(out, temps, 4);
        out.instruction(&Instruction::Br(1));
    }
    out.instruction(&Instruction::End); // close validation if

    // ── Invalid path ────────────────────────────────────────────────
    // Capture backtrace, build Exn::InvalidUnicode(code), throw.
    if let Some(bt_idx) = layout.capture_bt_func_idx {
        out.instruction(&Instruction::Call(bt_idx));
    }
    emit_alloc_object(out, temps, 2, layout)?;
    // tag at offset 0
    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
    out.instruction(&Instruction::I64Const(invalid_unicode_tag));
    out.instruction(&Instruction::I64Store(memarg(0)));
    // code at offset 8
    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Store(memarg(8)));
    // Push payload (object ptr as i64) and throw
    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::Throw(exn_tag_idx));
    // Throw is a control-flow terminator; the result block needs an
    // i64 type, so we never fall through here. Wasm validation requires
    // a reachable continuation though — `unreachable` is the correct
    // terminator for the rest of the block.
    out.instruction(&Instruction::End); // close result block
    Ok(())
}

/// Allocate `len` bytes on the string heap, leaving the buffer pointer
/// in `temps.concat_out_ptr_i32`. Used by the UTF-8 emit helpers.
fn emit_alloc_string_buf(
    out: &mut Function,
    layout: &CodegenLayout,
    temps: &super::FunctionTemps,
    len: i32,
) {
    if let Some(alloc_idx) = layout.allocate_func_idx {
        out.instruction(&Instruction::I32Const(len));
        out.instruction(&Instruction::Call(alloc_idx));
        out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
    } else {
        out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::I32Const(len));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::GlobalSet(STRING_HEAP_GLOBAL_INDEX));
        emit_string_heap_grow(out, layout);
    }
}

/// Emit `(out_ptr << 32) | len` on the WASM stack as a packed string i64.
fn emit_pack_string_result(out: &mut Function, temps: &super::FunctionTemps, len: i64) {
    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64Shl);
    out.instruction(&Instruction::I64Const(len));
    out.instruction(&Instruction::I64Or);
}

/// Emit memory.grow if the string heap (global 0) exceeds current memory.
/// Skip when allocate_func_idx is set (stdlib manages memory).
fn emit_string_heap_grow(out: &mut Function, layout: &CodegenLayout) {
    if layout.allocate_func_idx.is_some() {
        return;
    }
    out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
    out.instruction(&Instruction::MemorySize(0));
    out.instruction(&Instruction::I32Const(16));
    out.instruction(&Instruction::I32Shl);
    out.instruction(&Instruction::I32GtU);
    out.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
    out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
    out.instruction(&Instruction::MemorySize(0));
    out.instruction(&Instruction::I32Const(16));
    out.instruction(&Instruction::I32Shl);
    out.instruction(&Instruction::I32Sub);
    out.instruction(&Instruction::I32Const(65535));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Const(16));
    out.instruction(&Instruction::I32ShrU);
    out.instruction(&Instruction::MemoryGrow(0));
    out.instruction(&Instruction::Drop);
    out.instruction(&Instruction::End);
}

/// Emit the common prefix for string scanning intrinsics:
/// 1. Unpack packed i64 → ptr (concat_lhs_ptr_i32), len (concat_lhs_len_i32)
/// 2. Clamp start to [0, len] → concat_idx_i32
fn emit_intrinsic_unpack_and_clamp(
    s: &LirAtom,
    start: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &super::layout::CodegenLayout,
    temps: &super::FunctionTemps,
) -> Result<(), super::error::CodegenError> {
    // Unpack s → ptr, len
    compile_atom(s, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.packed_tmp_i64));
    // ptr = high 32 bits
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_ptr_i32));
    // len = low 32 bits
    out.instruction(&Instruction::LocalGet(temps.packed_tmp_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_len_i32));
    // Clamp start → idx in [0, len]
    emit_intrinsic_clamp_arg(
        start,
        temps.concat_idx_i32,
        temps.concat_lhs_len_i32,
        out,
        local_map,
        layout,
        temps,
    )
}

/// Clamp an i64 argument to [0, upper_bound_local] and store into dest_local (i32).
fn emit_intrinsic_clamp_arg(
    arg: &LirAtom,
    dest_local: u32,
    upper_bound_local: u32,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &super::layout::CodegenLayout,
    _temps: &super::FunctionTemps,
) -> Result<(), super::error::CodegenError> {
    compile_atom(arg, out, local_map, layout)?;
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(dest_local));
    // max(val, 0): select(val, 0, val >= 0)
    out.instruction(&Instruction::LocalGet(dest_local));
    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalGet(dest_local));
    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::I32GeS);
    out.instruction(&Instruction::Select);
    out.instruction(&Instruction::LocalSet(dest_local));
    // min(val, upper): select(val, upper, val <= upper)
    out.instruction(&Instruction::LocalGet(dest_local));
    out.instruction(&Instruction::LocalGet(upper_bound_local));
    out.instruction(&Instruction::LocalGet(dest_local));
    out.instruction(&Instruction::LocalGet(upper_bound_local));
    out.instruction(&Instruction::I32LeU);
    out.instruction(&Instruction::Select);
    out.instruction(&Instruction::LocalSet(dest_local));
    Ok(())
}

pub(super) fn compile_atom(
    atom: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &CodegenLayout,
) -> Result<(), CodegenError> {
    match atom {
        LirAtom::Var { name, typ } => {
            // Unit-typed variables have no WASM representation — skip silently.
            if matches!(typ, Type::Unit) {
                return Ok(());
            }
            let local = local_map
                .get(name)
                .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                    name: name.to_string(),
                })?;
            out.instruction(&Instruction::LocalGet(local.index));
            Ok(())
        }
        LirAtom::Int(i) => {
            out.instruction(&Instruction::I64Const(*i));
            Ok(())
        }
        LirAtom::Float(f) => {
            out.instruction(&Instruction::F64Const((*f).into()));
            Ok(())
        }
        LirAtom::Bool(b) => {
            out.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            Ok(())
        }
        LirAtom::Char(c) => {
            out.instruction(&Instruction::I32Const(*c as i32));
            Ok(())
        }
        LirAtom::String(s) => {
            let packed = layout
                .string_literals
                .get(s)
                .copied()
                .ok_or_else(|| CodegenError::StringLiteralsWithoutMemory)?;
            out.instruction(&Instruction::I64Const(pack_string(packed)));
            Ok(())
        }
        LirAtom::Unit => Ok(()),
    }
}

/// Check if a function contains any self-tail-calls (TailCall where func == self.name).
/// Used to decide whether to wrap the function body in a TCO loop.
fn has_self_tail_call(func: &LirFunction) -> bool {
    stmts_have_self_tail_call(&func.body, &func.name)
}

fn stmts_have_self_tail_call(stmts: &[LirStmt], self_name: &Symbol) -> bool {
    stmts.iter().any(|s| stmt_has_self_tail_call(s, self_name))
}

fn stmt_has_self_tail_call(stmt: &LirStmt, self_name: &Symbol) -> bool {
    match stmt {
        LirStmt::Let { expr, .. } => matches!(
            expr,
            LirExpr::TailCall { func, .. } if func == self_name
        ),
        LirStmt::If {
            then_body,
            else_body,
            ..
        } => {
            stmts_have_self_tail_call(then_body, self_name)
                || stmts_have_self_tail_call(else_body, self_name)
        }
        LirStmt::IfReturn {
            then_body,
            else_body,
            ..
        } => {
            stmts_have_self_tail_call(then_body, self_name)
                || stmts_have_self_tail_call(else_body, self_name)
        }
        LirStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            cases
                .iter()
                .any(|c| stmts_have_self_tail_call(&c.body, self_name))
                || stmts_have_self_tail_call(default_body, self_name)
        }
        LirStmt::Loop {
            cond_stmts, body, ..
        } => {
            stmts_have_self_tail_call(cond_stmts, self_name)
                || stmts_have_self_tail_call(body, self_name)
        }
        // TryCatch: TCO is disabled inside try blocks, so don't look there.
        _ => false,
    }
}

// ── TCMC: Tail Call Modulo Constructor ──────────────────────────────────

/// Emit the TCMC sequence: allocate constructor with placeholder rest,
/// link to previous cell, update prev, reassign params, and br to loop.
pub(super) fn emit_tcmc_cons_and_loop(
    tcmc: &TcmcInfo,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    program: &LirProgram,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
    tco_loop: Option<TcoLoop>,
) -> Result<(), CodegenError> {
    // 1. Allocate constructor cell (tag + fields)
    emit_alloc_object(out, temps, 1 + tcmc.ctor_num_fields, layout)?;

    // Store tag
    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
    out.instruction(&Instruction::I64Const(constructor_tag(
        tcmc.ctor_name.as_str(),
        tcmc.ctor_num_fields,
    )));
    out.instruction(&Instruction::I64Store(memarg(0)));

    // Store rest field = 0 (placeholder, filled by next iteration or base case)
    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
    out.instruction(&Instruction::I64Const(0));
    out.instruction(&Instruction::I64Store(memarg(
        ((tcmc.rest_field_idx + 1) * 8) as u64,
    )));

    // Store non-recursive fields
    for &(idx, ref atom) in &tcmc.non_rec_fields {
        out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
        compile_atom(atom, out, local_map, layout)?;
        emit_typed_field_store(&atom.typ(), ((idx + 1) * 8) as u64, out)?;
    }

    // 2. Link to previous cell (or set head if first)
    out.instruction(&Instruction::LocalGet(tcmc.prev_local));
    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::I32Ne);
    out.instruction(&Instruction::If(BlockType::Empty));
    {
        // prev.rest = current cell pointer
        out.instruction(&Instruction::LocalGet(tcmc.prev_local));
        out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
        out.instruction(&Instruction::I64ExtendI32U);
        out.instruction(&Instruction::I64Store(memarg(
            ((tcmc.rest_field_idx + 1) * 8) as u64,
        )));
    }
    out.instruction(&Instruction::Else);
    {
        // First cell: save as head
        out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
        out.instruction(&Instruction::I64ExtendI32U);
        out.instruction(&Instruction::LocalSet(tcmc.head_local));
    }
    out.instruction(&Instruction::End);

    // 3. Update prev = current cell
    out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
    out.instruction(&Instruction::LocalSet(tcmc.prev_local));

    // 4. Reassign params from saved call args + br to loop
    if let Some(tco) = tco_loop {
        let callee = program
            .functions
            .iter()
            .find(|f| f.name == tco.self_name)
            .ok_or_else(|| CodegenError::CallTargetNotFound {
                name: tco.self_name.to_string(),
            })?;

        // Push all arg values onto the WASM stack
        for ((_label, atom), param) in tcmc.call_args.iter().zip(callee.params.iter()) {
            compile_atom(atom, out, local_map, layout)?;
            emit_numeric_coercion(&atom.typ(), &param.typ, out)?;
        }
        // Pop into params in reverse order
        for param in callee.params.iter().rev() {
            let local =
                local_map
                    .get(&param.name)
                    .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                        name: param.name.to_string(),
                    })?;
            out.instruction(&Instruction::LocalSet(local.index));
        }
        out.instruction(&Instruction::Br(tco.loop_depth));
    }

    Ok(())
}
