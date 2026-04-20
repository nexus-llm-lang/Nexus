use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, MemArg, ValType};

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirExternal};
use crate::types::{Type, WasmRepr};

use super::error::CodegenError;
use super::layout::CodegenLayout;
use super::string::unpack_packed_i64_to_ptr_len;
use super::{FunctionTemps, LocalInfo, OBJECT_HEAP_GLOBAL_INDEX};

pub(super) use crate::compiler::type_tag::{constructor_tag, record_tag};

pub(super) fn memarg(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 3,
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
        out.instruction(&Instruction::I32Const((words as i32) * 8));
        out.instruction(&Instruction::Call(alloc_idx));
        out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));
    } else {
        out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
        out.instruction(&Instruction::LocalTee(temps.object_ptr_i32));
        out.instruction(&Instruction::I32Const((words as i32) * 8));
        out.instruction(&Instruction::I32Add);
        out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));

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

/// Emit a typed store to memory at the given byte offset.
/// Stack: [addr:i32, value:<native_type>] → [].
/// Uses the value's native WASM type for the store instruction, avoiding
/// the pack_to_i64 round-trip (sign-extend / reinterpret / wrap).
/// Heap slots remain 8 bytes wide; sub-64-bit types simply leave the upper bytes unused.
pub(super) fn emit_typed_field_store(
    typ: &Type,
    offset: u64,
    out: &mut Function,
) -> Result<(), CodegenError> {
    let ma = |align| MemArg {
        offset,
        align,
        memory_index: 0,
    };
    match typ.wasm_repr() {
        WasmRepr::I64 => out.instruction(&Instruction::I64Store(ma(3))),
        WasmRepr::I32 => out.instruction(&Instruction::I32Store(ma(2))),
        WasmRepr::F64 => out.instruction(&Instruction::F64Store(ma(3))),
        WasmRepr::F32 => out.instruction(&Instruction::F32Store(ma(2))),
        WasmRepr::Unit => {
            // compile_atom(Unit) pushes nothing; drop the address
            out.instruction(&Instruction::Drop)
        }
    };
    Ok(())
}

/// Emit a typed load from memory at the given byte offset.
/// Stack: [addr:i32] → [value:<native_type>].
/// Uses the field's native WASM type for the load instruction, avoiding
/// the unpack_from_i64 round-trip.
pub(super) fn emit_typed_field_load(
    typ: &Type,
    offset: u64,
    out: &mut Function,
) -> Result<(), CodegenError> {
    let ma = |align| MemArg {
        offset,
        align,
        memory_index: 0,
    };
    match typ.wasm_repr() {
        WasmRepr::I64 => out.instruction(&Instruction::I64Load(ma(3))),
        WasmRepr::I32 => out.instruction(&Instruction::I32Load(ma(2))),
        WasmRepr::F64 => out.instruction(&Instruction::F64Load(ma(3))),
        WasmRepr::F32 => out.instruction(&Instruction::F32Load(ma(2))),
        WasmRepr::Unit => {
            // Drop the address; unit loads produce no value
            out.instruction(&Instruction::Drop)
        }
    };
    Ok(())
}

pub(super) fn compile_external_arg(
    atom: &LirAtom,
    param_type: &Type,
    out: &mut Function,
    local_map: &HashMap<Symbol, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    use super::function::compile_atom;
    let param_repr = peel_linear(param_type);
    match param_repr {
        Type::String => {
            if !matches!(peel_linear(&atom.typ()), Type::String) {
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "string".to_string(),
                    got: atom.typ().to_string(),
                });
            }
            compile_atom(atom, out, local_map, layout)?;
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
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => {
            // A borrow of a linear array is represented as i64 at the WASM level
            // (packed pointer). The param_type already confirms this is a valid
            // borrow-of-array, so we just compile the atom and unpack.
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        _ => {
            compile_atom(atom, out, local_map, layout)?;
            emit_numeric_coercion(&atom.typ(), param_type, out)?;
            Ok(())
        }
    }
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
    if matches!(from, Type::Unit) || matches!(to, Type::Unit) {
        return Ok(());
    }
    if let (Ok(wf), Ok(wt)) = (type_to_wasm_valtype(from), type_to_wasm_valtype(to)) {
        if wf == wt {
            return Ok(());
        }
    }
    match (from, to) {
        (Type::I64, Type::I32) | (Type::I64, Type::Bool) => {
            out.instruction(&Instruction::I32WrapI64);
            Ok(())
        }
        (Type::I32, Type::I64) | (Type::Bool, Type::I64) => {
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

fn adt_coercion_is_noop(from: &Type, to: &Type) -> bool {
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
    while let Type::Linear(inner) | Type::Lazy(inner) = typ {
        typ = inner;
    }
    typ
}

pub(super) fn type_to_wasm_valtype(typ: &Type) -> Result<ValType, CodegenError> {
    match typ.wasm_repr() {
        WasmRepr::I32 => Ok(ValType::I32),
        WasmRepr::I64 => Ok(ValType::I64),
        WasmRepr::F32 => Ok(ValType::F32),
        WasmRepr::F64 => Ok(ValType::F64),
        WasmRepr::Unit => Err(CodegenError::UnitWasmType),
    }
}

pub(super) fn return_type_to_wasm_result(ret: &Type) -> Result<Vec<ValType>, CodegenError> {
    match ret.wasm_repr() {
        WasmRepr::Unit => Ok(vec![]),
        _ => Ok(vec![type_to_wasm_valtype(ret)?]),
    }
}

pub(super) fn external_param_types(ext: &LirExternal) -> Result<Vec<ValType>, CodegenError> {
    use super::string::string_abi_for_external;
    let abi = string_abi_for_external(ext);
    let mut out = Vec::new();
    for param in &ext.params {
        match peel_linear(&param.typ) {
            Type::I32 | Type::Bool | Type::Char => out.push(ValType::I32),
            Type::I64 => out.push(ValType::I64),
            Type::F32 => out.push(ValType::F32),
            Type::F64 => out.push(ValType::F64),
            Type::String => {
                // Both ABIs pass strings as (ptr: i32, len: i32)
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
    // Canonical ABI: string returns use a retptr as the last parameter
    if abi == super::string::StringABI::Canonical
        && matches!(peel_linear(&ext.ret_type), Type::String)
    {
        out.push(ValType::I32); // retptr
    }
    Ok(out)
}

pub(super) fn external_return_types(ext: &LirExternal) -> Result<Vec<ValType>, CodegenError> {
    use super::string::string_abi_for_external;
    let abi = string_abi_for_external(ext);
    // FFI ABI boundary: only a restricted set of types can be returned from externals.
    match peel_linear(&ext.ret_type) {
        Type::Unit => Ok(vec![]),
        Type::I32 | Type::Bool | Type::Char => Ok(vec![ValType::I32]),
        Type::I64 => Ok(vec![ValType::I64]),
        Type::F32 => Ok(vec![ValType::F32]),
        Type::F64 => Ok(vec![ValType::F64]),
        Type::String => match abi {
            // Packed ABI: string returned as packed i64 in a single value
            super::string::StringABI::Packed => Ok(vec![ValType::I64]),
            // Canonical ABI: string written to retptr, function returns void
            super::string::StringABI::Canonical => Ok(vec![]),
        },
        other => Err(CodegenError::UnsupportedExternalReturnType {
            typ: other.to_string(),
        }),
    }
}

fn is_array_like_type(typ: &Type) -> bool {
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
        LirExpr::Force { typ, .. } => typ.clone(),
        LirExpr::FuncRef { typ, .. } => typ.clone(),
        LirExpr::Closure { typ, .. } => typ.clone(),
        LirExpr::ClosureEnvLoad { typ, .. } => typ.clone(),
        LirExpr::CallIndirect { typ, .. } => typ.clone(),
        LirExpr::LazySpawn { typ, .. } => typ.clone(),
        LirExpr::LazyJoin { typ, .. } => typ.clone(),
        LirExpr::Intrinsic { typ, .. } => typ.clone(),
    }
}
