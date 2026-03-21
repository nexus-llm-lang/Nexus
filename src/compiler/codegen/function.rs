use std::collections::HashMap;

use wasm_encoder::{Function, Instruction, ValType};

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirFunction, LirProgram, LirStmt};
use crate::types::Type;

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
        )?;
    }

    // Backtrace: pop frame before implicit return
    if let Some(bt_pop_idx) = layout.bt_pop_idx {
        out.instruction(&Instruction::Call(bt_pop_idx));
    }

    if !matches!(func.ret_type, Type::Unit) {
        compile_atom(&func.ret, &mut out, &local_map, layout)?;
        emit_numeric_coercion(&func.ret.typ(), &func.ret_type, &mut out)?;
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
    use wasm_encoder::BlockType;
    out.instruction(&Instruction::GlobalGet(layout.exn_flag_global));
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_exn_bail(out, layout, function_ret_type, is_entrypoint);
    out.instruction(&Instruction::End);
}
