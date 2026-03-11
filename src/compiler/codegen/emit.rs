use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use crate::ir::lir::{LirAtom, LirExpr, LirExternal};
use crate::lang::ast::{BinaryOp, Type};

use super::error::CodegenError;
use super::layout::{CodegenLayout, PackedString};
use super::{FunctionTemps, LocalInfo, OBJECT_HEAP_GLOBAL_INDEX};

pub(super) fn constructor_tag(name: &str, arity: usize) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= arity as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash as i64
}

pub(super) fn record_tag(sorted_field_names: &[String]) -> i64 {
    let shape = sorted_field_names.join(",");
    hash_tag("rec", &shape, sorted_field_names.len())
}

fn hash_tag(kind: &str, name: &str, arity: usize) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in kind.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for &b in name.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= arity as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash as i64
}

pub(super) fn memarg(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 3,
        memory_index: 0,
    }
}

pub(super) fn memarg_i8() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}

pub(super) fn emit_alloc_object(
    out: &mut Function,
    temps: &FunctionTemps,
    words: usize,
    layout: &CodegenLayout,
) -> Result<(), CodegenError> {
    if !layout.object_heap_enabled {
        return Err(CodegenError::ObjectHeapRequired {
            context: "object heap allocation",
        });
    }

    if let Some(alloc_idx) = layout.allocate_func_idx {
        // Use stdlib's allocate() — goes through dlmalloc, no conflict.
        out.instruction(&Instruction::I32Const((words as i32) * 8));
        out.instruction(&Instruction::Call(alloc_idx));
        out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));
    } else {
        // Fallback bump-allocate for programs without stdlib.
        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::LocalTee(temps.object_ptr_i32));
        out.instruction(&Instruction::I32Const((words as i32) * 8));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));

        // Grow memory if the new heap pointer exceeds current memory size.
        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::MemorySize(0));
        out.instruction(&Instruction::I32Const(16));
        out.instruction(&Instruction::I32Shl);
        out.instruction(&Instruction::I32GtU);
        out.instruction(&Instruction::If(BlockType::Empty));
        {
            out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
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
        }
        out.instruction(&Instruction::End);
    }

    Ok(())
}

pub(super) fn emit_pack_value_to_i64(typ: &Type, out: &mut Function) -> Result<(), CodegenError> {
    match peel_linear(typ) {
        Type::I64
        | Type::String
        | Type::Array(_)
        | Type::List(_)
        | Type::Record(_)
        | Type::UserDefined(_, _)
        | Type::Var(_)
        | Type::Borrow(_) => Ok(()),
        Type::I32 | Type::Bool => {
            out.instruction(&Instruction::I64ExtendI32S);
            Ok(())
        }
        Type::F64 => {
            out.instruction(&Instruction::I64ReinterpretF64);
            Ok(())
        }
        Type::F32 => {
            out.instruction(&Instruction::I32ReinterpretF32);
            out.instruction(&Instruction::I64ExtendI32U);
            Ok(())
        }
        Type::Unit => {
            out.instruction(&Instruction::Drop);
            out.instruction(&Instruction::I64Const(0));
            Ok(())
        }
        other => Err(CodegenError::UnsupportedPack {
            typ: other.to_string(),
        }),
    }
}

pub(super) fn emit_unpack_i64_to_value(typ: &Type, out: &mut Function) -> Result<(), CodegenError> {
    match peel_linear(typ) {
        Type::I64
        | Type::String
        | Type::Array(_)
        | Type::List(_)
        | Type::Record(_)
        | Type::UserDefined(_, _)
        | Type::Var(_)
        | Type::Borrow(_) => Ok(()),
        Type::I32 | Type::Bool => {
            out.instruction(&Instruction::I32WrapI64);
            Ok(())
        }
        Type::F64 => {
            out.instruction(&Instruction::F64ReinterpretI64);
            Ok(())
        }
        Type::F32 => {
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::F32ReinterpretI32);
            Ok(())
        }
        Type::Unit => Err(CodegenError::UnsupportedUnpack {
            typ: "unit".to_string(),
        }),
        other => Err(CodegenError::UnsupportedUnpack {
            typ: other.to_string(),
        }),
    }
}

pub(super) fn unpack_packed_i64_to_ptr_len(out: &mut Function, tmp_local: u32) {
    out.instruction(&Instruction::LocalSet(tmp_local));

    out.instruction(&Instruction::LocalGet(tmp_local));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);

    out.instruction(&Instruction::LocalGet(tmp_local));
    out.instruction(&Instruction::I64Const(0xFFFF_FFFFu64 as i64));
    out.instruction(&Instruction::I64And);
    out.instruction(&Instruction::I32WrapI64);
}

pub(super) fn compile_external_arg(
    atom: &LirAtom,
    param_type: &Type,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    let param_repr = peel_linear(param_type);
    match param_repr {
        Type::String => {
            if !matches!(peel_linear(&atom.typ()), Type::String) {
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "string".to_string(),
                    got: atom.typ().to_string(),
                });
            }
            super::compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Array(_) => {
            if !is_array_like_type(&atom.typ()) {
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "array-like value".to_string(),
                    got: atom.typ().to_string(),
                });
            }
            super::compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => {
            if !is_array_like_type(&atom.typ()) {
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "array-like value".to_string(),
                    got: atom.typ().to_string(),
                });
            }
            super::compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        _ => {
            super::compile_atom(atom, out, local_map, layout)?;
            emit_numeric_coercion(&atom.typ(), param_type, out)?;
            Ok(())
        }
    }
}

pub(super) fn is_string_concat_operator(op: BinaryOp, result_type: &Type) -> bool {
    matches!(op, BinaryOp::Concat | BinaryOp::Add)
        && matches!(peel_linear(result_type), Type::String)
}

pub(super) fn emit_string_concat(
    lhs: &LirAtom,
    rhs: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    if !layout.object_heap_enabled {
        return Err(CodegenError::ObjectHeapRequired {
            context: "string concat",
        });
    }
    if !matches!(peel_linear(&lhs.typ()), Type::String)
        || !matches!(peel_linear(&rhs.typ()), Type::String)
    {
        return Err(CodegenError::StringConcatTypeMismatch {
            lhs: lhs.typ().to_string(),
            rhs: rhs.typ().to_string(),
        });
    }

    super::compile_atom(lhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
    super::compile_atom(rhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_packed_i64));

    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_len_i32));

    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_len_i32));

    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_out_len_i32));

    if let Some(alloc_idx) = layout.allocate_func_idx {
        // Use stdlib's allocate() for the concatenated string buffer.
        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
        out.instruction(&Instruction::Call(alloc_idx));
        out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
    } else {
        // Fallback bump-allocate for programs without stdlib.
        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
    }

    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::I32GeU);
    out.instruction(&Instruction::BrIf(1));

    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Load8U(memarg_i8()));
    out.instruction(&Instruction::I32Store8(memarg_i8()));

    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Const(1));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Br(0));
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
    out.instruction(&Instruction::I32GeU);
    out.instruction(&Instruction::BrIf(1));

    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Load8U(memarg_i8()));
    out.instruction(&Instruction::I32Store8(memarg_i8()));

    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Const(1));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Br(0));
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64Shl);
    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Or);
    Ok(())
}

pub(super) fn is_string_compare_operator(op: BinaryOp, lhs: &Type, rhs: &Type) -> bool {
    matches!(op, BinaryOp::Eq | BinaryOp::Ne)
        && matches!(peel_linear(lhs), Type::String)
        && matches!(peel_linear(rhs), Type::String)
}

/// Emit inline string equality comparison.
///
/// Algorithm: compare lengths first, then byte-by-byte.
/// Leaves an i32 (0 or 1) on the stack. For `Ne`, the result is inverted.
pub(super) fn emit_string_compare(
    op: BinaryOp,
    lhs: &LirAtom,
    rhs: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    // Unpack lhs: i64 -> (ptr_i32, len_i32)
    super::compile_atom(lhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
    // Unpack rhs
    super::compile_atom(rhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_packed_i64));

    // lhs ptr
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_ptr_i32));
    // lhs len
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_len_i32));
    // rhs ptr
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
    // rhs len
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_len_i32));

    // Outer block: result is i32 on stack
    //   block $done          ;; br 1 to exit with result on stack
    //     block $not_equal   ;; br 0 to jump to "not equal" path
    //       ;; if lengths differ -> br $not_equal
    //       ;; byte-by-byte compare loop -> br $not_equal on mismatch
    //       ;; fall through = equal
    //       i32.const 1
    //       br $done
    //     end $not_equal
    //     i32.const 0
    //   end $done
    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Result(
        ValType::I32,
    )));
    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));

    // Compare lengths
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
    out.instruction(&Instruction::I32Ne);
    out.instruction(&Instruction::BrIf(0)); // lengths differ -> not_equal

    // Byte-by-byte comparison loop
    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Block(wasm_encoder::BlockType::Empty));
    out.instruction(&Instruction::Loop(wasm_encoder::BlockType::Empty));

    // if idx >= len -> all bytes matched, break to "equal" path
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::I32GeU);
    out.instruction(&Instruction::BrIf(1)); // break inner block -> fall through to equal

    // Compare lhs[idx] vs rhs[idx]
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Load8U(memarg_i8()));

    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Load8U(memarg_i8()));

    out.instruction(&Instruction::I32Ne);
    out.instruction(&Instruction::BrIf(2)); // bytes differ -> not_equal (block depth 2)

    // idx++
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Const(1));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Br(0)); // continue loop
    out.instruction(&Instruction::End); // end loop
    out.instruction(&Instruction::End); // end inner block

    // Equal path: push 1, branch to done
    out.instruction(&Instruction::I32Const(1));
    out.instruction(&Instruction::Br(1)); // br $done

    out.instruction(&Instruction::End); // end $not_equal
                                        // Not equal path: push 0
    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::End); // end $done

    // For Ne, invert the result
    if op == BinaryOp::Ne {
        out.instruction(&Instruction::I32Eqz);
    }

    Ok(())
}

pub(super) fn pack_string(s: PackedString) -> i64 {
    (((s.offset as u64) << 32) | (s.len as u64)) as i64
}

pub(super) fn emit_numeric_coercion(
    from: &Type,
    to: &Type,
    out: &mut Function,
) -> Result<(), CodegenError> {
    let from = peel_linear(from);
    let to = peel_linear(to);
    if from == to {
        return Ok(());
    }
    if adt_coercion_is_noop(from, to) {
        return Ok(());
    }
    // Unit -> anything is a no-op in certain control-flow contexts
    // (e.g. void-returning call used where a value is expected)
    if matches!(from, Type::Unit) || matches!(to, Type::Unit) {
        return Ok(());
    }
    // If both types map to the same wasm valtype, no instruction needed
    // (e.g. i64 <-> UserDefined("Handle", []) -- both are ValType::I64)
    if let (Ok(wf), Ok(wt)) = (type_to_wasm_valtype(from), type_to_wasm_valtype(to)) {
        if wf == wt {
            return Ok(());
        }
    }
    match (from, to) {
        (Type::I64, Type::I32) => {
            out.instruction(&Instruction::I32WrapI64);
            Ok(())
        }
        (Type::I32, Type::I64) => {
            out.instruction(&Instruction::I64ExtendI32S);
            Ok(())
        }
        (Type::F64, Type::F32) => {
            out.instruction(&Instruction::F32DemoteF64);
            Ok(())
        }
        (Type::F32, Type::F64) => {
            out.instruction(&Instruction::F64PromoteF32);
            Ok(())
        }
        _ => Err(CodegenError::UnsupportedCoercion {
            from_type: from.to_string(),
            to_type: to.to_string(),
        }),
    }
}

pub(super) fn adt_coercion_is_noop(from: &Type, to: &Type) -> bool {
    match (from, to) {
        (Type::UserDefined(from_name, _), Type::UserDefined(to_name, _)) => from_name == to_name,
        (Type::Record(_), Type::Record(_)) => true,
        (Type::Var(_), Type::Var(_)) => true,
        (Type::Borrow(from_inner), Type::Borrow(to_inner)) => {
            adt_coercion_is_noop(peel_linear(from_inner), peel_linear(to_inner))
        }
        _ => false,
    }
}

pub(super) fn peel_linear(mut typ: &Type) -> &Type {
    while let Type::Linear(inner) = typ {
        typ = inner;
    }
    typ
}

pub(super) fn type_to_wasm_valtype(typ: &Type) -> Result<ValType, CodegenError> {
    match peel_linear(typ) {
        Type::I32 | Type::Bool => Ok(ValType::I32),
        Type::I64 => Ok(ValType::I64),
        Type::F32 => Ok(ValType::F32),
        Type::F64 => Ok(ValType::F64),
        Type::String => Ok(ValType::I64),
        Type::Array(_) => Ok(ValType::I64),
        Type::Record(_) => Ok(ValType::I64),
        Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => Ok(ValType::I64),
        Type::Borrow(inner)
            if matches!(
                peel_linear(inner),
                Type::Record(_) | Type::UserDefined(_, _) | Type::List(_) | Type::Var(_)
            ) =>
        {
            Ok(ValType::I64)
        }
        Type::UserDefined(_, _) | Type::List(_) | Type::Var(_) => Ok(ValType::I64),
        Type::Unit => Err(CodegenError::UnitWasmType),
        other => Err(CodegenError::UnsupportedWasmType {
            typ: other.to_string(),
        }),
    }
}

pub(super) fn return_type_to_wasm_result(ret: &Type) -> Result<Vec<ValType>, CodegenError> {
    match peel_linear(ret) {
        Type::Unit => Ok(vec![]),
        _ => Ok(vec![type_to_wasm_valtype(ret)?]),
    }
}

pub(super) fn external_param_types(ext: &LirExternal) -> Result<Vec<ValType>, CodegenError> {
    let mut out = Vec::new();
    for param in &ext.params {
        match peel_linear(&param.typ) {
            Type::I32 | Type::Bool => out.push(ValType::I32),
            Type::I64 => out.push(ValType::I64),
            Type::F32 => out.push(ValType::F32),
            Type::F64 => out.push(ValType::F64),
            Type::String => {
                out.push(ValType::I32);
                out.push(ValType::I32);
            }
            Type::Array(_) => {
                out.push(ValType::I32);
                out.push(ValType::I32);
            }
            Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => {
                out.push(ValType::I32);
                out.push(ValType::I32);
            }
            other => {
                return Err(CodegenError::UnsupportedExternalParamType {
                    typ: other.to_string(),
                })
            }
        }
    }
    Ok(out)
}

pub(super) fn external_return_types(ext: &LirExternal) -> Result<Vec<ValType>, CodegenError> {
    match peel_linear(&ext.ret_type) {
        Type::Unit => Ok(vec![]),
        Type::I32 | Type::Bool => Ok(vec![ValType::I32]),
        Type::I64 => Ok(vec![ValType::I64]),
        Type::F32 => Ok(vec![ValType::F32]),
        Type::F64 => Ok(vec![ValType::F64]),
        Type::String => Ok(vec![ValType::I64]),
        other => Err(CodegenError::UnsupportedExternalReturnType {
            typ: other.to_string(),
        }),
    }
}

pub(super) fn is_array_like_type(typ: &Type) -> bool {
    match peel_linear(typ) {
        Type::Array(_) => true,
        Type::Borrow(inner) => matches!(peel_linear(inner), Type::Array(_)),
        _ => false,
    }
}

pub(super) fn expr_type(expr: &LirExpr) -> Type {
    match expr {
        LirExpr::Atom(atom) => atom.typ(),
        LirExpr::Binary { typ, .. } => typ.clone(),
        LirExpr::Call { typ, .. } | LirExpr::TailCall { typ, .. } => typ.clone(),
        LirExpr::Constructor { typ, .. } => typ.clone(),
        LirExpr::Record { typ, .. } => typ.clone(),
        LirExpr::ObjectTag { typ, .. } => typ.clone(),
        LirExpr::ObjectField { typ, .. } => typ.clone(),
        LirExpr::Raise { typ, .. } => typ.clone(),
    }
}
