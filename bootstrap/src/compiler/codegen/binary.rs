use wasm_encoder::{Function, Instruction};

use crate::types::{BinaryOp, Type};

use super::emit::peel_linear;
use super::error::CodegenError;

pub(super) fn compile_binary(
    op: BinaryOp,
    operand_type: &Type,
    out: &mut Function,
) -> Result<(), CodegenError> {
    match peel_linear(operand_type) {
        Type::I64 => match op {
            BinaryOp::Add => out.instruction(&Instruction::I64Add),
            BinaryOp::Sub => out.instruction(&Instruction::I64Sub),
            BinaryOp::Mul => out.instruction(&Instruction::I64Mul),
            BinaryOp::Div => out.instruction(&Instruction::I64DivS),
            BinaryOp::Mod => out.instruction(&Instruction::I64RemS),
            BinaryOp::BitAnd => out.instruction(&Instruction::I64And),
            BinaryOp::BitOr => out.instruction(&Instruction::I64Or),
            BinaryOp::BitXor => out.instruction(&Instruction::I64Xor),
            BinaryOp::Shl => out.instruction(&Instruction::I64Shl),
            BinaryOp::Shr => out.instruction(&Instruction::I64ShrS),
            BinaryOp::Eq => out.instruction(&Instruction::I64Eq),
            BinaryOp::Ne => out.instruction(&Instruction::I64Ne),
            BinaryOp::Lt => out.instruction(&Instruction::I64LtS),
            BinaryOp::Le => out.instruction(&Instruction::I64LeS),
            BinaryOp::Gt => out.instruction(&Instruction::I64GtS),
            BinaryOp::Ge => out.instruction(&Instruction::I64GeS),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "i64".to_string(),
                })
            }
        },
        Type::I32 => match op {
            BinaryOp::Add => out.instruction(&Instruction::I32Add),
            BinaryOp::Sub => out.instruction(&Instruction::I32Sub),
            BinaryOp::Mul => out.instruction(&Instruction::I32Mul),
            BinaryOp::Div => out.instruction(&Instruction::I32DivS),
            BinaryOp::Mod => out.instruction(&Instruction::I32RemS),
            BinaryOp::BitAnd => out.instruction(&Instruction::I32And),
            BinaryOp::BitOr => out.instruction(&Instruction::I32Or),
            BinaryOp::BitXor => out.instruction(&Instruction::I32Xor),
            BinaryOp::Shl => out.instruction(&Instruction::I32Shl),
            BinaryOp::Shr => out.instruction(&Instruction::I32ShrS),
            BinaryOp::Eq => out.instruction(&Instruction::I32Eq),
            BinaryOp::Ne => out.instruction(&Instruction::I32Ne),
            BinaryOp::Lt => out.instruction(&Instruction::I32LtS),
            BinaryOp::Le => out.instruction(&Instruction::I32LeS),
            BinaryOp::Gt => out.instruction(&Instruction::I32GtS),
            BinaryOp::Ge => out.instruction(&Instruction::I32GeS),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "i32".to_string(),
                })
            }
        },
        Type::Bool => match op {
            BinaryOp::Eq => out.instruction(&Instruction::I32Eq),
            BinaryOp::Ne => out.instruction(&Instruction::I32Ne),
            BinaryOp::And => out.instruction(&Instruction::I32And),
            BinaryOp::Or => out.instruction(&Instruction::I32Or),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "bool".to_string(),
                })
            }
        },
        Type::Char => match op {
            BinaryOp::Eq => out.instruction(&Instruction::I32Eq),
            BinaryOp::Ne => out.instruction(&Instruction::I32Ne),
            BinaryOp::Lt => out.instruction(&Instruction::I32LtU),
            BinaryOp::Le => out.instruction(&Instruction::I32LeU),
            BinaryOp::Gt => out.instruction(&Instruction::I32GtU),
            BinaryOp::Ge => out.instruction(&Instruction::I32GeU),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "char".to_string(),
                })
            }
        },
        Type::UserDefined(_, _) | Type::Var(_) | Type::Record(_) => match op {
            BinaryOp::Eq => out.instruction(&Instruction::I64Eq),
            BinaryOp::Ne => out.instruction(&Instruction::I64Ne),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "user-defined".to_string(),
                })
            }
        },
        Type::F64 => match op {
            BinaryOp::FAdd => out.instruction(&Instruction::F64Add),
            BinaryOp::FSub => out.instruction(&Instruction::F64Sub),
            BinaryOp::FMul => out.instruction(&Instruction::F64Mul),
            BinaryOp::FDiv => out.instruction(&Instruction::F64Div),
            BinaryOp::FEq => out.instruction(&Instruction::F64Eq),
            BinaryOp::FNe => out.instruction(&Instruction::F64Ne),
            BinaryOp::FLt => out.instruction(&Instruction::F64Lt),
            BinaryOp::FLe => out.instruction(&Instruction::F64Le),
            BinaryOp::FGt => out.instruction(&Instruction::F64Gt),
            BinaryOp::FGe => out.instruction(&Instruction::F64Ge),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "f64".to_string(),
                })
            }
        },
        Type::F32 => match op {
            BinaryOp::FAdd => out.instruction(&Instruction::F32Add),
            BinaryOp::FSub => out.instruction(&Instruction::F32Sub),
            BinaryOp::FMul => out.instruction(&Instruction::F32Mul),
            BinaryOp::FDiv => out.instruction(&Instruction::F32Div),
            BinaryOp::FEq => out.instruction(&Instruction::F32Eq),
            BinaryOp::FNe => out.instruction(&Instruction::F32Ne),
            BinaryOp::FLt => out.instruction(&Instruction::F32Lt),
            BinaryOp::FLe => out.instruction(&Instruction::F32Le),
            BinaryOp::FGt => out.instruction(&Instruction::F32Gt),
            BinaryOp::FGe => out.instruction(&Instruction::F32Ge),
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "f32".to_string(),
                })
            }
        },
        other => {
            return Err(CodegenError::UnsupportedBinaryOp {
                op,
                operand_type: other.to_string(),
            })
        }
    };
    Ok(())
}

pub(super) fn binary_operand_type(
    op: BinaryOp,
    lhs: &Type,
    rhs: &Type,
) -> Result<Type, CodegenError> {
    let lhs = peel_linear(lhs);
    let rhs = peel_linear(rhs);
    if matches!(op, BinaryOp::Concat | BinaryOp::Add)
        && matches!(lhs, Type::String)
        && matches!(rhs, Type::String)
    {
        return Ok(Type::String);
    }
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
        if matches!(lhs, Type::Char) && matches!(rhs, Type::Char) {
            return Ok(Type::Char);
        }
        if matches!(lhs, Type::String) && matches!(rhs, Type::String) {
            return Ok(Type::String);
        }
        if matches!(lhs, Type::Bool) && matches!(rhs, Type::Bool) {
            return Ok(Type::Bool);
        }
        if matches!(lhs, Type::UserDefined(_, _) | Type::Var(_))
            && matches!(rhs, Type::UserDefined(_, _) | Type::Var(_))
        {
            return Ok(lhs.clone());
        }
        if matches!(lhs, Type::Record(_)) && matches!(rhs, Type::Record(_)) {
            return Ok(lhs.clone());
        }
    }
    if matches!(
        op,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    ) {
        if matches!(lhs, Type::Char) && matches!(rhs, Type::Char) {
            return Ok(Type::Char);
        }
    }
    if matches!(op, BinaryOp::And | BinaryOp::Or) {
        if matches!(lhs, Type::Bool) && matches!(rhs, Type::Bool) {
            return Ok(Type::Bool);
        }
    }
    if matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
    ) {
        if matches!(lhs, Type::I32) || matches!(rhs, Type::I32) {
            return Ok(Type::I32);
        }
        if matches!(lhs, Type::I64) || matches!(rhs, Type::I64) {
            return Ok(Type::I64);
        }
    }
    if op.is_float_op() {
        if matches!(lhs, Type::F32) || matches!(rhs, Type::F32) {
            return Ok(Type::F32);
        }
        if matches!(lhs, Type::F64) || matches!(rhs, Type::F64) {
            return Ok(Type::F64);
        }
    }
    Err(CodegenError::UnsupportedBinaryOpPair {
        op,
        lhs: lhs.to_string(),
        rhs: rhs.to_string(),
    })
}
