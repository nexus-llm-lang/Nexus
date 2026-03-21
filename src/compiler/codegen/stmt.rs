use std::borrow::Cow;
use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, MemArg};

use crate::intern::Symbol;
use crate::ir::lir::{LirProgram, LirStmt, SwitchCase};
use crate::types::Type;

use super::emit::emit_numeric_coercion;
use super::error::CodegenError;
use super::function::{compile_atom, compile_expr};
use super::layout::CodegenLayout;
use super::{FunctionTemps, LocalInfo, OBJECT_HEAP_GLOBAL_INDEX};

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
) -> Result<(), CodegenError> {
    match stmt {
        LirStmt::Let { name, typ, expr } => {
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
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::If(BlockType::Empty));
            for nested in then_body {
                compile_stmt(
                    nested,
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
                )?;
            }
            if !else_body.is_empty() {
                out.instruction(&Instruction::Else);
                for nested in else_body {
                    compile_stmt(
                        nested,
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
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::If(BlockType::Empty));
            for nested in then_body {
                compile_stmt(
                    nested,
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
                )?;
            }
            if let Some(then_ret) = then_ret {
                compile_atom(then_ret, out, local_map, layout)?;
                emit_numeric_coercion(&then_ret.typ(), ret_type, out)?;
                if let Some(bt_pop_idx) = layout.bt_pop_idx {
                    out.instruction(&Instruction::Call(bt_pop_idx));
                }
                out.instruction(&Instruction::Return);
            }
            if !else_body.is_empty() || else_ret.is_some() {
                out.instruction(&Instruction::Else);
                for nested in else_body {
                    compile_stmt(
                        nested,
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
                    )?;
                }
                if let Some(else_ret) = else_ret {
                    compile_atom(else_ret, out, local_map, layout)?;
                    emit_numeric_coercion(&else_ret.typ(), ret_type, out)?;
                    if let Some(bt_pop_idx) = layout.bt_pop_idx {
                        out.instruction(&Instruction::Call(bt_pop_idx));
                    }
                    out.instruction(&Instruction::Return);
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
            let catch_local =
                local_map
                    .get(catch_param)
                    .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                        name: catch_param.to_string(),
                    })?;

            // Reset global exception flag at try entry
            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::GlobalSet(layout.exn_flag_global));

            out.instruction(&Instruction::Block(BlockType::Empty));
            for nested in body {
                compile_stmt(
                    nested,
                    out,
                    local_map,
                    program,
                    internal_indices,
                    external_indices,
                    layout,
                    temps,
                    function_ret_type,
                    true,
                    is_entrypoint,
                )?;
                out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
                out.instruction(&Instruction::BrIf(0));
            }
            if let Some(ret) = body_ret {
                compile_atom(ret, out, local_map, layout)?;
                emit_numeric_coercion(&ret.typ(), function_ret_type, out)?;
                if let Some(bt_pop_idx) = layout.bt_pop_idx {
                    out.instruction(&Instruction::Call(bt_pop_idx));
                }
                out.instruction(&Instruction::Return);
            }
            out.instruction(&Instruction::End);

            // Check global flag: if exception was raised, run catch
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
                    nested,
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
                )?;
                if in_try {
                    out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
                    out.instruction(&Instruction::BrIf(0));
                }
            }
            if let Some(ret) = catch_ret {
                compile_atom(ret, out, local_map, layout)?;
                emit_numeric_coercion(&ret.typ(), function_ret_type, out)?;
                if let Some(bt_pop_idx) = layout.bt_pop_idx {
                    out.instruction(&Instruction::Call(bt_pop_idx));
                }
                out.instruction(&Instruction::Return);
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

                // Allocate space for args on the heap
                let args_ptr_local = temps.object_ptr_i32;
                if let Some(alloc_idx) = layout.allocate_func_idx {
                    out.instruction(&Instruction::I32Const(n_args * 8));
                    out.instruction(&Instruction::Call(alloc_idx));
                    out.instruction(&Instruction::LocalSet(args_ptr_local));
                } else {
                    out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                    out.instruction(&Instruction::LocalSet(args_ptr_local));
                }

                // Write each captured arg as i64 to the heap
                for (i, (_, arg)) in task.args.iter().enumerate() {
                    out.instruction(&Instruction::LocalGet(args_ptr_local));
                    compile_atom(arg, out, local_map, layout)?;
                    // Widen to i64 if needed
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
                        _ => {} // i64, string (packed i64), objects (i64 ptr)
                    }
                    out.instruction(&Instruction::I64Store(MemArg {
                        offset: (i * 8) as u64,
                        align: 3,
                        memory_index: 0,
                    }));
                }

                // Advance heap pointer (only for bump allocator)
                if layout.allocate_func_idx.is_none() {
                    out.instruction(&Instruction::LocalGet(args_ptr_local));
                    out.instruction(&Instruction::I32Const(n_args * 8));
                    out.instruction(&Instruction::I32Add);
                    out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
                }

                // Call __nx_conc_spawn(func_idx, args_ptr, n_args)
                out.instruction(&Instruction::I32Const(func_idx as i32));
                out.instruction(&Instruction::LocalGet(args_ptr_local));
                out.instruction(&Instruction::I32Const(n_args));
                out.instruction(&Instruction::Call(spawn_idx));
            }

            // Call __nx_conc_join()
            out.instruction(&Instruction::Call(join_idx));
            Ok(())
        }
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            out.instruction(&Instruction::Block(BlockType::Empty));
            out.instruction(&Instruction::Loop(BlockType::Empty));
            // Evaluate condition preamble
            for nested in cond_stmts {
                compile_stmt(
                    nested,
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
                )?;
            }
            // Check break condition
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::BrIf(1)); // break to outer block
                                                    // Body
            for nested in body {
                compile_stmt(
                    nested,
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
                )?;
            }
            out.instruction(&Instruction::Br(0)); // continue to loop head
            out.instruction(&Instruction::End); // end loop
            out.instruction(&Instruction::End); // end block
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
                    tag,
                    cases,
                    default_body,
                    default_ret,
                    ret_type,
                    min_tag,
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
                )
            } else {
                compile_switch_linear(
                    tag,
                    cases,
                    default_body,
                    default_ret,
                    ret_type,
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
                )
            }
        }
    }
}

/// Check if Switch case tag values form a dense integer range suitable for br_table.
/// Returns (min_tag, table_size) if dense, None otherwise.
fn check_dense_tags(cases: &[SwitchCase]) -> Option<(i64, usize)> {
    if cases.is_empty() {
        return None;
    }
    let mut tags: Vec<i64> = cases.iter().map(|c| c.tag_value).collect();
    tags.sort();
    tags.dedup();
    if tags.len() != cases.len() {
        return None; // duplicate tags
    }
    let min = tags[0];
    let max = tags[tags.len() - 1];
    // Guard against i64 overflow in range computation
    let range_size = match (max as u64).checked_sub(min as u64) {
        Some(diff) => diff as usize + 1,
        None => return None,
    };
    // Only use br_table if perfectly dense and range fits comfortably
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
) -> Result<(), CodegenError> {
    for case in cases {
        compile_atom(tag, out, local_map, layout)?;
        out.instruction(&Instruction::I64Const(case.tag_value));
        out.instruction(&Instruction::I64Eq);
        out.instruction(&Instruction::If(BlockType::Empty));
        for nested in &case.body {
            compile_stmt(
                nested,
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
            )?;
        }
        if let Some(ret) = &case.ret {
            compile_atom(ret, out, local_map, layout)?;
            emit_numeric_coercion(&ret.typ(), ret_type, out)?;
            if let Some(bt_pop_idx) = layout.bt_pop_idx {
                out.instruction(&Instruction::Call(bt_pop_idx));
            }
            out.instruction(&Instruction::Return);
        }
        out.instruction(&Instruction::End);
    }
    // Default body
    for nested in default_body {
        compile_stmt(
            nested,
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
        )?;
    }
    if let Some(ret) = default_ret {
        compile_atom(ret, out, local_map, layout)?;
        emit_numeric_coercion(&ret.typ(), ret_type, out)?;
        if let Some(bt_pop_idx) = layout.bt_pop_idx {
            out.instruction(&Instruction::Call(bt_pop_idx));
        }
        out.instruction(&Instruction::Return);
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
) -> Result<(), CodegenError> {
    let n = cases.len();

    // Sort cases by tag value to build correct br_table mapping
    let mut sorted_indices: Vec<usize> = (0..n).collect();
    sorted_indices.sort_by_key(|&i| cases[i].tag_value);

    // Build br_table targets: for index i (= tag - min_tag),
    // jump to the block depth for sorted_cases[i].
    // Depth 0 = default (innermost), depth k = sorted_cases[n-k] (outermost = sorted[0])
    let mut targets = vec![0u32; n]; // default (0) for any gaps
    for (sorted_pos, &case_idx) in sorted_indices.iter().enumerate() {
        let index = (cases[case_idx].tag_value - min_tag) as usize;
        targets[index] = (n - sorted_pos) as u32;
    }
    let default_target = 0u32;

    // Open n+1 blocks: n for cases + 1 for default
    for _ in 0..=n {
        out.instruction(&Instruction::Block(BlockType::Empty));
    }

    // Compute index = i32.wrap_i64(tag - min_tag)
    compile_atom(tag, out, local_map, layout)?;
    out.instruction(&Instruction::I64Const(min_tag));
    out.instruction(&Instruction::I64Sub);
    out.instruction(&Instruction::I32WrapI64);

    // br_table dispatch
    out.instruction(&Instruction::BrTable(Cow::Owned(targets), default_target));

    // Close default block → emit default body
    out.instruction(&Instruction::End);
    for nested in default_body {
        compile_stmt(
            nested,
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
        )?;
    }
    if let Some(ret) = default_ret {
        compile_atom(ret, out, local_map, layout)?;
        emit_numeric_coercion(&ret.typ(), ret_type, out)?;
        if let Some(bt_pop_idx) = layout.bt_pop_idx {
            out.instruction(&Instruction::Call(bt_pop_idx));
        }
        out.instruction(&Instruction::Return);
    }

    // Close case blocks in reverse sorted order + emit bodies
    for &case_idx in sorted_indices.iter().rev() {
        out.instruction(&Instruction::End);
        for nested in &cases[case_idx].body {
            compile_stmt(
                nested,
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
            )?;
        }
        if let Some(ret) = &cases[case_idx].ret {
            compile_atom(ret, out, local_map, layout)?;
            emit_numeric_coercion(&ret.typ(), ret_type, out)?;
            if let Some(bt_pop_idx) = layout.bt_pop_idx {
                out.instruction(&Instruction::Call(bt_pop_idx));
            }
            out.instruction(&Instruction::Return);
        }
    }

    Ok(())
}
