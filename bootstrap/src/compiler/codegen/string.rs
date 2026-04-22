use std::collections::HashMap;

use wasm_encoder::{Function, Instruction, MemArg, ValType};

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExternal};
use crate::types::{BinaryOp, Type};

use super::emit::peel_linear;
use super::error::CodegenError;
use super::function::compile_atom;
use super::layout::{CodegenLayout, PackedString};
use super::{FunctionTemps, LocalInfo, STRING_HEAP_GLOBAL_INDEX};

/// String ABI mode for cross-boundary function calls.
///
/// Controls how strings are passed across the component boundary:
/// - `Packed`: Internal i64 encoding `(ptr << 32) | len`. Used for core WASM module bundling
///   where caller and callee share the same linear memory.
/// - `Canonical`: Component model canonical ABI. Strings passed as `(ptr: i32, len: i32)`,
///   returns use a retptr parameter. Used for component model boundaries where each
///   component has its own linear memory and strings are copied via canonical lift/lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringABI {
    /// Packed i64: `(ptr << 32) | len`. Shared linear memory.
    Packed,
    /// Canonical ABI: two i32s `(ptr, len)`, retptr for string returns.
    Canonical,
}

/// Determine the string ABI for an external function based on its import module.
///
/// Component-model imports use WIT-style identifiers (containing `:`),
/// e.g. `nexus:stdlib/string-ops`. File-path imports (e.g. `stdlib/stdlib.wasm`)
pub(super) fn string_abi_for_external(ext: &LirExternal) -> StringABI {
    let module = ext.wasm_module.as_ref();
    if module.contains(':') {
        StringABI::Canonical
    } else {
        StringABI::Packed
    }
}

/// Check if an external function returns a string via the canonical ABI retptr convention.
pub(super) fn external_uses_canonical_string_return(ext: &LirExternal) -> bool {
    string_abi_for_external(ext) == StringABI::Canonical
        && matches!(peel_linear(&ext.ret_type), Type::String)
}

pub(super) fn pack_string(s: PackedString) -> i64 {
    (((s.offset as u64) << 32) | (s.len as u64)) as i64
}

/// Emit WASM instructions for a canonical ABI string return.
///
/// After calling a canonical-ABI function that returns a string via retptr:
/// 1. Load ptr (i32) from retptr+0
/// 2. Load len (i32) from retptr+4
/// 3. Pack into i64 for internal use
///
/// `retptr_local` is the i32 local holding the return pointer address.
pub(super) fn emit_canonical_string_return_unpack(
    out: &mut Function,
    retptr_local: u32,
    tmp_ptr: u32,
    tmp_len: u32,
) {
    // Load ptr from retptr+0
    out.instruction(&Instruction::LocalGet(retptr_local));
    out.instruction(&Instruction::I32Load(MemArg {
        offset: 0,
        align: 2, // 4-byte aligned
        memory_index: 0,
    }));
    out.instruction(&Instruction::LocalSet(tmp_ptr));

    // Load len from retptr+4
    out.instruction(&Instruction::LocalGet(retptr_local));
    out.instruction(&Instruction::I32Load(MemArg {
        offset: 4,
        align: 2,
        memory_index: 0,
    }));
    out.instruction(&Instruction::LocalSet(tmp_len));

    // Pack into i64
    out.instruction(&Instruction::LocalGet(tmp_ptr));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64Shl);
    out.instruction(&Instruction::LocalGet(tmp_len));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Or);
}

pub(super) fn memarg_i8() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
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

pub(super) fn is_string_concat_operator(op: BinaryOp, result_type: &Type) -> bool {
    matches!(op, BinaryOp::Concat | BinaryOp::Add)
        && matches!(peel_linear(result_type), Type::String)
}

pub(super) fn is_string_compare_operator(op: BinaryOp, lhs: &Type, rhs: &Type) -> bool {
    matches!(op, BinaryOp::Eq | BinaryOp::Ne)
        && matches!(peel_linear(lhs), Type::String)
        && matches!(peel_linear(rhs), Type::String)
}

pub(super) fn emit_string_concat(
    lhs: &LirAtom,
    rhs: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
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

    compile_atom(lhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
    compile_atom(rhs, out, local_map, layout)?;
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
        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
        out.instruction(&Instruction::Call(alloc_idx));
        out.instruction(&Instruction::LocalSet(temps.concat_out_ptr_i32));
    } else {
        out.instruction(&Instruction::GlobalGet(STRING_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
        out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::GlobalSet(STRING_HEAP_GLOBAL_INDEX));
        // Grow memory if string heap exceeds current size
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

    // Copy lhs bytes
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

    // Copy rhs bytes
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

/// Emit inline string equality comparison.
///
/// Algorithm: compare lengths first, then byte-by-byte.
/// Leaves an i32 (0 or 1) on the stack. For `Ne`, the result is inverted.
pub(super) fn emit_string_compare(
    op: BinaryOp,
    lhs: &LirAtom,
    rhs: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    // Unpack lhs
    compile_atom(lhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
    // Unpack rhs
    compile_atom(rhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_packed_i64));

    // lhs ptr (relative + str_base)
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_ptr_i32));
    // lhs len
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_len_i32));
    // rhs ptr (relative + str_base)
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
    // rhs len
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_len_i32));

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

    // if idx >= len -> all bytes matched
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::I32GeU);
    out.instruction(&Instruction::BrIf(1));

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
    out.instruction(&Instruction::BrIf(2)); // bytes differ -> not_equal

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
