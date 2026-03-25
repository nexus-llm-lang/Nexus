use std::borrow::Cow;
use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, MemArg};

use crate::intern::Symbol;
use crate::ir::lir::{LirExpr, LirProgram, LirStmt, SwitchCase};
use crate::types::Type;

use super::emit::{emit_numeric_coercion, emit_pack_value_to_i64, memarg};
use super::error::CodegenError;
use super::function::{compile_atom, compile_expr, emit_tcmc_cons_and_loop, TcmcInfo, TcoLoop};
use super::layout::CodegenLayout;
use super::{FunctionTemps, LocalInfo, OBJECT_HEAP_GLOBAL_INDEX};

/// Emit a return value, wrapping with TCMC conditional linking if active.
/// When TCMC is active (prev != 0), links the last cell's rest field to the
/// return value and returns the head pointer instead.
#[allow(clippy::too_many_arguments)]
fn emit_return_with_tcmc(
    ret_val: &crate::ir::lir::LirAtom,
    ret_type: &Type,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &CodegenLayout,
    tcmc: Option<&TcmcInfo>,
) -> Result<(), CodegenError> {
    if let Some(tcmc) = tcmc {
        // Check if TCMC iterations happened (prev != 0)
        out.instruction(&Instruction::LocalGet(tcmc.prev_local));
        out.instruction(&Instruction::I32Const(0));
        out.instruction(&Instruction::I32Ne);
        out.instruction(&Instruction::If(BlockType::Empty));
        {
            // Link last cell's rest to base value
            out.instruction(&Instruction::LocalGet(tcmc.prev_local));
            compile_atom(ret_val, out, local_map, layout)?;
            emit_pack_value_to_i64(&ret_val.typ(), out)?;
            out.instruction(&Instruction::I64Store(memarg(
                ((tcmc.rest_field_idx + 1) * 8) as u64,
            )));
            if let Some(bt_pop_idx) = layout.bt_pop_idx {
                out.instruction(&Instruction::Call(bt_pop_idx));
            }
            // Return head of built list
            out.instruction(&Instruction::LocalGet(tcmc.head_local));
            out.instruction(&Instruction::Return);
        }
        out.instruction(&Instruction::Else);
        {
            // No TCMC iterations: return original value
            compile_atom(ret_val, out, local_map, layout)?;
            emit_numeric_coercion(&ret_val.typ(), ret_type, out)?;
            if let Some(bt_pop_idx) = layout.bt_pop_idx {
                out.instruction(&Instruction::Call(bt_pop_idx));
            }
            out.instruction(&Instruction::Return);
        }
        out.instruction(&Instruction::End);
    } else {
        // Normal return
        compile_atom(ret_val, out, local_map, layout)?;
        emit_numeric_coercion(&ret_val.typ(), ret_type, out)?;
        if let Some(bt_pop_idx) = layout.bt_pop_idx {
            out.instruction(&Instruction::Call(bt_pop_idx));
        }
        out.instruction(&Instruction::Return);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_stmt(
    stmt: &LirStmt,
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
    tcmc: Option<&TcmcInfo>,
) -> Result<(), CodegenError> {
    match stmt {
        LirStmt::Let { name, typ, expr } => {
            // TCMC interception: skip the self-call and intercept the constructor
            if let Some(tcmc) = tcmc {
                if *name == tcmc.call_var && matches!(expr, LirExpr::Call { .. }) {
                    // Skip the self-call — args are pre-saved in tcmc.call_args
                    return Ok(());
                }
                if *name == tcmc.cons_var && matches!(expr, LirExpr::Constructor { .. }) {
                    // Emit TCMC: alloc + link + param reassign + br
                    emit_tcmc_cons_and_loop(
                        tcmc, out, local_map, program, layout, temps, tco_loop,
                    )?;
                    return Ok(());
                }
            }

            compile_expr(
                expr,
                out,
                local_map,
                program,
                internal_indices,
                external_indices,
                layout,
                temps,
                function_ret_type,
                in_try,
                is_entrypoint,
                tco_loop,
            )?;
            let et = super::emit::expr_type(expr);
            if matches!(typ, Type::Unit) {
                if !matches!(et, Type::Unit) {
                    out.instruction(&Instruction::Drop);
                }
                return Ok(());
            }
            emit_numeric_coercion(&et, typ, out)?;
            let local = local_map
                .get(name)
                .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                    name: name.to_string(),
                })?;
            out.instruction(&Instruction::LocalSet(local.index));
            Ok(())
        }
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            // If opens 1 WASM block
            let inner_tco = tco_loop.map(|t| t.deeper(1));
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::If(BlockType::Empty));
            for nested in then_body {
                compile_stmt(
                    nested, out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint, inner_tco, tcmc,
                )?;
            }
            if !else_body.is_empty() {
                out.instruction(&Instruction::Else);
                for nested in else_body {
                    compile_stmt(
                        nested, out, local_map, program, internal_indices, external_indices,
                        layout, temps, function_ret_type, in_try, is_entrypoint, inner_tco,
                        tcmc,
                    )?;
                }
            }
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ret_type,
        } => {
            // IfReturn opens 1 WASM block (If)
            let inner_tco = tco_loop.map(|t| t.deeper(1));
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::If(BlockType::Empty));
            for nested in then_body {
                compile_stmt(
                    nested, out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint, inner_tco, tcmc,
                )?;
            }
            if let Some(then_ret) = then_ret {
                emit_return_with_tcmc(then_ret, ret_type, out, local_map, layout, tcmc)?;
            }
            if !else_body.is_empty() || else_ret.is_some() {
                out.instruction(&Instruction::Else);
                for nested in else_body {
                    compile_stmt(
                        nested, out, local_map, program, internal_indices, external_indices,
                        layout, temps, function_ret_type, in_try, is_entrypoint, inner_tco,
                        tcmc,
                    )?;
                }
                if let Some(else_ret) = else_ret {
                    emit_return_with_tcmc(else_ret, ret_type, out, local_map, layout, tcmc)?;
                }
            }
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_param,
            catch_param_typ: _,
            catch_body,
            catch_ret,
        } => {
            // Disable TCO and TCMC inside try-catch
            let catch_local =
                local_map
                    .get(catch_param)
                    .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                        name: catch_param.to_string(),
                    })?;

            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::GlobalSet(layout.exn_flag_global));

            out.instruction(&Instruction::Block(BlockType::Empty));
            for nested in body {
                compile_stmt(
                    nested, out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, true, is_entrypoint,
                    None, None, // TCO + TCMC disabled in try body
                )?;
                out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
                out.instruction(&Instruction::BrIf(0));
            }
            if let Some(ret) = body_ret {
                emit_return_with_tcmc(ret, function_ret_type, out, local_map, layout, None)?;
            }
            out.instruction(&Instruction::End);

            out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
            out.instruction(&Instruction::If(BlockType::Empty));
            out.instruction(&Instruction::GlobalGet(layout.exn_value_global));
            out.instruction(&Instruction::LocalSet(catch_local.index));
            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::GlobalSet(layout.exn_flag_global));

            if in_try {
                out.instruction(&Instruction::Block(BlockType::Empty));
            }
            for nested in catch_body {
                compile_stmt(
                    nested, out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint,
                    None, None, // TCO + TCMC disabled in catch body
                )?;
                if in_try {
                    out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
                    out.instruction(&Instruction::BrIf(0));
                }
            }
            if let Some(ret) = catch_ret {
                emit_return_with_tcmc(ret, function_ret_type, out, local_map, layout, None)?;
            }
            if in_try {
                out.instruction(&Instruction::End);
            }
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::Conc { tasks } => {
            let spawn_idx = layout
                .conc_spawn_idx
                .expect("conc_spawn_idx must be set for conc blocks");
            let join_idx = layout
                .conc_join_idx
                .expect("conc_join_idx must be set for conc blocks");

            for task in tasks {
                let func_idx = internal_indices
                    .get(&task.func_name)
                    .copied()
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: task.func_name.to_string(),
                    })?;
                let n_args = task.args.len() as i32;

                let args_ptr_local = temps.object_ptr_i32;
                if let Some(alloc_idx) = layout.allocate_func_idx {
                    out.instruction(&Instruction::I32Const(n_args * 8));
                    out.instruction(&Instruction::Call(alloc_idx));
                    out.instruction(&Instruction::LocalSet(args_ptr_local));
                } else {
                    out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                    out.instruction(&Instruction::LocalSet(args_ptr_local));
                }

                for (i, (_, arg)) in task.args.iter().enumerate() {
                    out.instruction(&Instruction::LocalGet(args_ptr_local));
                    compile_atom(arg, out, local_map, layout)?;
                    match arg.typ() {
                        Type::I32 | Type::Bool => {
                            out.instruction(&Instruction::I64ExtendI32U);
                        }
                        Type::F64 => {
                            out.instruction(&Instruction::I64ReinterpretF64);
                        }
                        Type::F32 => {
                            out.instruction(&Instruction::F64PromoteF32);
                            out.instruction(&Instruction::I64ReinterpretF64);
                        }
                        _ => {}
                    }
                    out.instruction(&Instruction::I64Store(MemArg {
                        offset: (i * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                }

                if layout.allocate_func_idx.is_none() {
                    out.instruction(&Instruction::LocalGet(args_ptr_local));
                    out.instruction(&Instruction::I32Const(n_args * 8));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
                }

                out.instruction(&Instruction::I32Const(func_idx as i32));
                out.instruction(&Instruction::LocalGet(args_ptr_local));
                out.instruction(&Instruction::I32Const(n_args));
                out.instruction(&Instruction::Call(spawn_idx));
            }

            out.instruction(&Instruction::Call(join_idx));
            Ok(())
        }
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            // Loop opens Block + Loop = 2 WASM blocks
            let inner_tco = tco_loop.map(|t| t.deeper(2));
            out.instruction(&Instruction::Block(BlockType::Empty));
            out.instruction(&Instruction::Loop(BlockType::Empty));
            for nested in cond_stmts {
                compile_stmt(
                    nested, out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint, inner_tco, tcmc,
                )?;
            }
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::BrIf(1));
            for nested in body {
                compile_stmt(
                    nested, out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint, inner_tco, tcmc,
                )?;
            }
            out.instruction(&Instruction::Br(0));
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::Switch {
            tag,
            cases,
            default_body,
            default_ret,
            ret_type,
        } => {
            if let Some((min_tag, _table_size)) = check_dense_tags(cases) {
                compile_switch_br_table(
                    tag, cases, default_body, default_ret, ret_type, min_tag,
                    out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint, tco_loop, tcmc,
                )
            } else {
                compile_switch_linear(
                    tag, cases, default_body, default_ret, ret_type,
                    out, local_map, program, internal_indices, external_indices,
                    layout, temps, function_ret_type, in_try, is_entrypoint, tco_loop, tcmc,
                )
            }
        }
    }
}

/// Check if Switch case tag values form a dense integer range suitable for br_table.
fn check_dense_tags(cases: &[SwitchCase]) -> Option<(i64, usize)> {
    if cases.is_empty() {
        return None;
    }
    let mut tags: Vec<i64> = cases.iter().map(|c| c.tag_value).collect();
    tags.sort();
    tags.dedup();
    if tags.len() != cases.len() {
        return None;
    }
    let min = tags[0];
    let max = tags[tags.len() - 1];
    let range_size = match (max as u64).checked_sub(min as u64) {
        Some(diff) => diff as usize + 1,
        None => return None,
    };
    if range_size == cases.len() && range_size <= 256 {
        Some((min, range_size))
    } else {
        None
    }
}

/// Compile Switch as a linear if-else chain (fallback when tags are sparse).
#[allow(clippy::too_many_arguments)]
fn compile_switch_linear(
    tag: &crate::ir::lir::LirAtom,
    cases: &[SwitchCase],
    default_body: &[LirStmt],
    default_ret: &Option<crate::ir::lir::LirAtom>,
    ret_type: &Type,
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
    tcmc: Option<&TcmcInfo>,
) -> Result<(), CodegenError> {
    let case_tco = tco_loop.map(|t| t.deeper(1));
    for case in cases {
        compile_atom(tag, out, local_map, layout)?;
        out.instruction(&Instruction::I64Const(case.tag_value));
        out.instruction(&Instruction::I64Eq);
        out.instruction(&Instruction::If(BlockType::Empty));
        for nested in &case.body {
            compile_stmt(
                nested, out, local_map, program, internal_indices, external_indices,
                layout, temps, function_ret_type, in_try, is_entrypoint, case_tco, tcmc,
            )?;
        }
        if let Some(ret) = &case.ret {
            emit_return_with_tcmc(ret, ret_type, out, local_map, layout, tcmc)?;
        }
        out.instruction(&Instruction::End);
    }
    for nested in default_body {
        compile_stmt(
            nested, out, local_map, program, internal_indices, external_indices,
            layout, temps, function_ret_type, in_try, is_entrypoint, tco_loop, tcmc,
        )?;
    }
    if let Some(ret) = default_ret {
        emit_return_with_tcmc(ret, ret_type, out, local_map, layout, tcmc)?;
    }
    Ok(())
}

/// Compile Switch using WASM br_table for O(1) dispatch (dense tags only).
///
/// Block layout (N = cases.len()):
/// ```text
///   block $case_0        ;; depth N from br_table
///     block $case_1      ;; depth N-1
///       ...
///         block $case_N-1  ;; depth 1
///           block $default ;; depth 0
///             ;; compute (tag - min_tag) as i32, br_table dispatch
///           end $default → default body here
///         end $case_N-1 → case N-1 body here
///       ...
///     end $case_1 → case 1 body here
///   end $case_0 → case 0 body here
/// ```
#[allow(clippy::too_many_arguments)]
fn compile_switch_br_table(
    tag: &crate::ir::lir::LirAtom,
    cases: &[SwitchCase],
    default_body: &[LirStmt],
    default_ret: &Option<crate::ir::lir::LirAtom>,
    ret_type: &Type,
    min_tag: i64,
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
    tcmc: Option<&TcmcInfo>,
) -> Result<(), CodegenError> {
    let n = cases.len();

    let mut sorted_indices: Vec<usize> = (0..n).collect();
    sorted_indices.sort_by_key(|&i| cases[i].tag_value);

    let mut targets = vec![0u32; n];
    for (sorted_pos, &case_idx) in sorted_indices.iter().enumerate() {
        let index = (cases[case_idx].tag_value - min_tag) as usize;
        targets[index] = (n - sorted_pos) as u32;
    }
    let default_target = 0u32;

    for _ in 0..=n {
        out.instruction(&Instruction::Block(BlockType::Empty));
    }

    compile_atom(tag, out, local_map, layout)?;
    out.instruction(&Instruction::I64Const(min_tag));
    out.instruction(&Instruction::I64Sub);
    out.instruction(&Instruction::I32WrapI64);

    out.instruction(&Instruction::BrTable(Cow::Owned(targets), default_target));

    // Close default block → emit default body
    out.instruction(&Instruction::End);
    let default_tco = tco_loop.map(|t| t.deeper(n as u32));
    for nested in default_body {
        compile_stmt(
            nested, out, local_map, program, internal_indices, external_indices,
            layout, temps, function_ret_type, in_try, is_entrypoint, default_tco, tcmc,
        )?;
    }
    if let Some(ret) = default_ret {
        emit_return_with_tcmc(ret, ret_type, out, local_map, layout, tcmc)?;
    }

    // Close case blocks in reverse sorted order + emit bodies
    for (j, &case_idx) in sorted_indices.iter().rev().enumerate() {
        out.instruction(&Instruction::End);
        let case_tco = tco_loop.map(|t| t.deeper((n - 1 - j) as u32));
        for nested in &cases[case_idx].body {
            compile_stmt(
                nested, out, local_map, program, internal_indices, external_indices,
                layout, temps, function_ret_type, in_try, is_entrypoint, case_tco, tcmc,
            )?;
        }
        if let Some(ret) = &cases[case_idx].ret {
            emit_return_with_tcmc(ret, ret_type, out, local_map, layout, tcmc)?;
        }
    }

    Ok(())
}
