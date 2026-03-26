use std::collections::HashMap;

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
    emit_pack_value_to_i64, emit_unpack_i64_to_value, memarg, record_tag, type_to_wasm_valtype,
};
use super::error::CodegenError;
use super::layout::{bt_label, CodegenLayout};
use super::stmt::compile_stmt;
use super::string::{
    emit_string_compare, emit_string_concat, is_string_compare_operator, is_string_concat_operator,
    pack_string,
};
use super::{FunctionTemps, LocalInfo};
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
        closure_ptr_i64: next_local_index + 11,
        closure_table_idx_i64: next_local_index + 12,
    };
    // Temps: 1×i64, 1×i32, 2×i64, 7×i32, 2×i64 (closure)
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
        ValType::I64,
        ValType::I64,
    ]);

    // TCMC: detect tail-call-modulo-constructor pattern and allocate extra locals
    // TODO: TCMC disabled pending runtime bug fix (bootstrap crash)
    let tcmc_pre: Option<TcmcPreInfo> = None; // detect_tcmc(func);
    let tcmc_info = tcmc_pre.map(|pre| {
        let head_idx = next_local_index + 13; // after FunctionTemps (13 slots)
        let prev_idx = next_local_index + 14;
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

    // Backtrace: push "file:line funcname" onto call stack at entry
    if let Some(bt_push_idx) = layout.bt_push_idx {
        let label = bt_label(func);
        let packed_name = layout
            .string_literals
            .get(&label)
            .map(|p| pack_string(*p))
            .unwrap_or(0);
        out.instruction(&Instruction::I64Const(packed_name));
        out.instruction(&Instruction::Call(bt_push_idx));
    }

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

    // Backtrace: pop frame before implicit return
    if let Some(bt_pop_idx) = layout.bt_pop_idx {
        out.instruction(&Instruction::Call(bt_pop_idx));
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
                emit_pack_value_to_i64(&func.ret.typ(), &mut out)?;
                out.instruction(&Instruction::I64Store(memarg(
                    ((tcmc.rest_field_idx + 1) * 8) as u64,
                )));
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
            LirStmt::Conc { .. } => {}
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
        }
    }
    Ok(())
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
    function_ret_type: &Type,
    in_try: bool,
    is_entrypoint: bool,
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
                    if let Some(bt_pop_idx) = layout.bt_pop_idx {
                        out.instruction(&Instruction::Call(bt_pop_idx));
                    }
                    out.instruction(&Instruction::ReturnCall(callee_idx));
                } else {
                    out.instruction(&Instruction::Call(callee_idx));
                    // Propagate exception from callee if not in try
                    // (in try, the stmt-level check after each statement handles it)
                    if !in_try {
                        emit_exn_propagate(out, layout, function_ret_type, is_entrypoint);
                    }
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

                if is_tail {
                    if let Some(bt_pop_idx) = layout.bt_pop_idx {
                        out.instruction(&Instruction::Call(bt_pop_idx));
                    }
                    out.instruction(&Instruction::ReturnCall(callee_idx));
                } else {
                    out.instruction(&Instruction::Call(callee_idx));
                }
                return Ok(());
            }

            Err(CodegenError::CallTargetNotFound {
                name: func.to_string(),
            })
        }
        LirExpr::Constructor { name, args, .. } => {
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
                emit_pack_value_to_i64(&arg.typ(), out)?;
                out.instruction(&Instruction::I64Store(memarg(((idx + 1) * 8) as u64)));
            }

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
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
                emit_pack_value_to_i64(&value.typ(), out)?;
                out.instruction(&Instruction::I64Store(memarg(((idx + 1) * 8) as u64)));
            }

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
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
            out.instruction(&Instruction::I64Load(memarg(((index + 1) * 8) as u64)));
            emit_unpack_i64_to_value(typ, out)?;
            Ok(())
        }
        LirExpr::Raise { value, .. } => {
            // Freeze backtrace before raising
            if let Some(bt_freeze_idx) = layout.bt_freeze_idx {
                out.instruction(&Instruction::Call(bt_freeze_idx));
            }
            compile_atom(value, out, local_map, layout)?;
            if !matches!(value.typ(), Type::Unit) {
                out.instruction(&Instruction::GlobalSet(layout.exn_value_global));
            } else {
                out.instruction(&Instruction::I64Const(0));
                out.instruction(&Instruction::GlobalSet(layout.exn_value_global));
            }
            out.instruction(&Instruction::I32Const(1));
            out.instruction(&Instruction::GlobalSet(layout.exn_flag_global));
            if !in_try {
                // Exit the function: trap in entrypoint, return-with-dummy in others
                emit_exn_bail(out, layout, function_ret_type, is_entrypoint);
            }
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
                emit_pack_value_to_i64(&atom.typ(), out)?;
                out.instruction(&Instruction::I64Store(memarg(((i + 1) * 8) as u64)));
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
            out.instruction(&Instruction::I64Load(memarg(((index + 1) * 8) as u64)));
            // Unpack from i64 to target type
            emit_unpack_i64_to_value(typ, out)?;
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
    }
}

pub(super) fn compile_atom(
    atom: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &CodegenLayout,
) -> Result<(), CodegenError> {
    match atom {
        LirAtom::Var { name, .. } => {
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

/// Emit an early exit for uncaught exceptions.
/// - In entrypoint (main): trap via Unreachable (uncaught exception = fatal)
/// - In other functions: return with a dummy value so caller can check the flag
fn emit_exn_bail(out: &mut Function, layout: &CodegenLayout, ret_type: &Type, is_entrypoint: bool) {
    if is_entrypoint {
        out.instruction(&Instruction::Unreachable);
    } else {
        if let Some(bt_pop_idx) = layout.bt_pop_idx {
            out.instruction(&Instruction::Call(bt_pop_idx));
        }
        match ret_type {
            Type::Unit => {}
            Type::I32 | Type::Bool | Type::Char => {
                out.instruction(&Instruction::I32Const(0));
            }
            Type::F64 => {
                out.instruction(&Instruction::F64Const(0.0.into()));
            }
            Type::F32 => {
                out.instruction(&Instruction::F32Const(0.0.into()));
            }
            _ => {
                // I64, String, ADTs (all represented as i64)
                out.instruction(&Instruction::I64Const(0));
            }
        }
        out.instruction(&Instruction::Return);
    }
}

/// After a non-tail Call when NOT in a try block: check the global exception
/// flag and propagate by bailing out if the callee raised.
fn emit_exn_propagate(
    out: &mut Function,
    layout: &CodegenLayout,
    function_ret_type: &Type,
    is_entrypoint: bool,
) {
    out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_exn_bail(out, layout, function_ret_type, is_entrypoint);
    out.instruction(&Instruction::End);
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
        // Conc: task bodies don't contain tail calls.
        _ => false,
    }
}

// ── TCMC: Tail Call Modulo Constructor ──────────────────────────────────

/// Detect the TCMC pattern: `Let A = Call(self, args)` followed by
/// `Let B = Constructor(name, [..A..])` where A is one of the constructor args.
fn detect_tcmc(func: &LirFunction) -> Option<TcmcPreInfo> {
    find_tcmc_in_stmts(&func.body, &func.name)
}

fn find_tcmc_in_stmts(stmts: &[LirStmt], self_name: &Symbol) -> Option<TcmcPreInfo> {
    // Check consecutive pairs of Let bindings
    for i in 0..stmts.len().saturating_sub(1) {
        if let LirStmt::Let {
            name: call_name,
            expr: LirExpr::Call { func, args, .. },
            ..
        } = &stmts[i]
        {
            if func == self_name {
                if let LirStmt::Let {
                    expr:
                        LirExpr::Constructor {
                            name: ctor_name,
                            args: ctor_args,
                            ..
                        },
                    name: cons_name,
                    ..
                } = &stmts[i + 1]
                {
                    if let Some(rest_idx) = ctor_args
                        .iter()
                        .position(|a| matches!(a, LirAtom::Var { name, .. } if name == call_name))
                    {
                        let non_rec_fields: Vec<(usize, LirAtom)> = ctor_args
                            .iter()
                            .enumerate()
                            .filter(|(j, _)| *j != rest_idx)
                            .map(|(j, a)| (j, a.clone()))
                            .collect();

                        return Some(TcmcPreInfo {
                            call_var: *call_name,
                            call_args: args.clone(),
                            cons_var: *cons_name,
                            ctor_name: *ctor_name,
                            ctor_num_fields: ctor_args.len(),
                            rest_field_idx: rest_idx,
                            non_rec_fields,
                        });
                    }
                }
            }
        }
    }

    // Recurse into branches
    for stmt in stmts {
        let result = match stmt {
            LirStmt::If {
                then_body,
                else_body,
                ..
            }
            | LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => find_tcmc_in_stmts(then_body, self_name)
                .or_else(|| find_tcmc_in_stmts(else_body, self_name)),
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => cases
                .iter()
                .find_map(|c| find_tcmc_in_stmts(&c.body, self_name))
                .or_else(|| find_tcmc_in_stmts(default_body, self_name)),
            LirStmt::Loop {
                cond_stmts, body, ..
            } => find_tcmc_in_stmts(cond_stmts, self_name)
                .or_else(|| find_tcmc_in_stmts(body, self_name)),
            _ => None,
        };
        if result.is_some() {
            return result;
        }
    }

    None
}

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
        emit_pack_value_to_i64(&atom.typ(), out)?;
        out.instruction(&Instruction::I64Store(memarg(((idx + 1) * 8) as u64)));
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
