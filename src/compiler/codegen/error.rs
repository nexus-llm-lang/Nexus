use super::HirBuildError;
use super::LirLowerError;
use crate::types::BinaryOp;

#[derive(Debug, Clone, PartialEq)]
pub enum CodegenError {
    /// E2001: main function not found in ANF program
    MissingMain,
    /// E2002: unsupported binary operator for operand type
    UnsupportedBinaryOp { op: BinaryOp, operand_type: String },
    /// E2003: unsupported binary operator for operand type pair
    UnsupportedBinaryOpPair {
        op: BinaryOp,
        lhs: String,
        rhs: String,
    },
    /// E2004: unsupported wasm type
    UnsupportedWasmType { typ: String },
    /// E2005: unit cannot be represented as wasm valtype
    UnitWasmType,
    /// E2006: unsupported numeric coercion
    UnsupportedCoercion { from_type: String, to_type: String },
    /// E2007: call target not found
    CallTargetNotFound { name: String },
    /// E2008: call arity mismatch
    CallArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    /// E2010: conflicting wasm local types
    ConflictingLocalTypes { name: String },
    /// E2011: object heap not enabled
    ObjectHeapRequired { context: &'static str },
    /// E2012: cannot pack value type into object field
    UnsupportedPack { typ: String },
    /// E2013: cannot unpack object field into type
    UnsupportedUnpack { typ: String },
    /// E2014: external param type not supported
    UnsupportedExternalParamType { typ: String },
    /// E2015: external return type not supported
    UnsupportedExternalReturnType { typ: String },
    /// E2016: external call argument type mismatch
    ExternalArgTypeMismatch { expected: String, got: String },
    /// E2017: string concat expects string operands
    StringConcatTypeMismatch { lhs: String, rhs: String },
    /// E2018: string literals exist without memory configuration
    StringLiteralsWithoutMemory,
}

impl CodegenError {
    pub fn code(&self) -> &'static str {
        match self {
            CodegenError::MissingMain => "E2001",
            CodegenError::UnsupportedBinaryOp { .. } => "E2002",
            CodegenError::UnsupportedBinaryOpPair { .. } => "E2003",
            CodegenError::UnsupportedWasmType { .. } => "E2004",
            CodegenError::UnitWasmType => "E2005",
            CodegenError::UnsupportedCoercion { .. } => "E2006",
            CodegenError::CallTargetNotFound { .. } => "E2007",
            CodegenError::CallArityMismatch { .. } => "E2008",
            CodegenError::ConflictingLocalTypes { .. } => "E2010",
            CodegenError::ObjectHeapRequired { .. } => "E2011",
            CodegenError::UnsupportedPack { .. } => "E2012",
            CodegenError::UnsupportedUnpack { .. } => "E2013",
            CodegenError::UnsupportedExternalParamType { .. } => "E2014",
            CodegenError::UnsupportedExternalReturnType { .. } => "E2015",
            CodegenError::ExternalArgTypeMismatch { .. } => "E2016",
            CodegenError::StringConcatTypeMismatch { .. } => "E2017",
            CodegenError::StringLiteralsWithoutMemory => "E2018",
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = self.code();
        let msg = match self {
            CodegenError::MissingMain => "main function not found in ANF program".to_string(),
            CodegenError::UnsupportedBinaryOp { op, operand_type } => {
                format!("unsupported {} binary operator '{}'", operand_type, op)
            }
            CodegenError::UnsupportedBinaryOpPair { op, lhs, rhs } => {
                format!(
                    "unsupported binary operator '{}' for operand types ({}, {})",
                    op, lhs, rhs
                )
            }
            CodegenError::UnsupportedWasmType { typ } => {
                format!("type '{}' is not supported by current wasm codegen", typ)
            }
            CodegenError::UnitWasmType => {
                "unit cannot be represented as a local/param wasm valtype".to_string()
            }
            CodegenError::UnsupportedCoercion { from_type, to_type } => {
                format!(
                    "unsupported numeric coercion from '{}' to '{}'",
                    from_type, to_type
                )
            }
            CodegenError::CallTargetNotFound { name } => {
                format!("call target '{}' not found in lowered symbols", name)
            }
            CodegenError::CallArityMismatch {
                name,
                expected,
                got,
            } => {
                format!(
                    "call arity mismatch for '{}': expected {}, got {}",
                    name, expected, got
                )
            }
            CodegenError::ConflictingLocalTypes { name } => {
                format!("variable '{}' has conflicting wasm local types", name)
            }
            CodegenError::ObjectHeapRequired { context } => {
                format!("{} requested without object heap", context)
            }
            CodegenError::UnsupportedPack { typ } => {
                format!("cannot pack value of type '{}' into object field", typ)
            }
            CodegenError::UnsupportedUnpack { typ } => {
                format!("cannot unpack object field into type '{}'", typ)
            }
            CodegenError::UnsupportedExternalParamType { typ } => {
                format!(
                    "external param type '{}' is not supported by current wasm codegen",
                    typ
                )
            }
            CodegenError::UnsupportedExternalReturnType { typ } => {
                format!(
                    "external return type '{}' is not supported by current wasm codegen",
                    typ
                )
            }
            CodegenError::ExternalArgTypeMismatch { expected, got } => {
                format!(
                    "external call argument type mismatch: expected {}, got {}",
                    expected, got
                )
            }
            CodegenError::StringConcatTypeMismatch { lhs, rhs } => {
                format!(
                    "string concat expects string operands, got ({}, {})",
                    lhs, rhs
                )
            }
            CodegenError::StringLiteralsWithoutMemory => {
                "string literals exist without memory configuration".to_string()
            }
        };
        write!(
            f,
            "internal compiler error: {} [{}] (this is a bug; please report it)",
            msg, code
        )
    }
}

impl std::error::Error for CodegenError {}

#[derive(Debug)]
pub enum CompileError {
    HirBuild(HirBuildError),
    LirLower(LirLowerError),
    Codegen(CodegenError),
    MainSignature(String),
}

impl CompileError {
    /// Returns the source span associated with this error, if available.
    pub fn span(&self) -> Option<&crate::types::Span> {
        match self {
            CompileError::HirBuild(e) => Some(e.span()),
            CompileError::LirLower(e) => Some(e.span()),
            CompileError::Codegen(_) | CompileError::MainSignature(_) => None,
        }
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::HirBuild(e) => write!(f, "{}", e),
            CompileError::LirLower(e) => write!(f, "{}", e),
            CompileError::Codegen(e) => write!(f, "{}", e),
            CompileError::MainSignature(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for CompileError {}

/// Per-pass timing metrics from the compilation pipeline.
#[derive(Debug, Clone)]
pub struct CompileMetrics {
    pub hir_build: std::time::Duration,
    pub lir_lower: std::time::Duration,
    pub codegen: std::time::Duration,
}

impl std::fmt::Display for CompileMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.hir_build + self.lir_lower + self.codegen;
        writeln!(f, "  build      {:>8.2?}", self.hir_build)?;
        writeln!(f, "  lir_lower  {:>8.2?}", self.lir_lower)?;
        writeln!(f, "  codegen    {:>8.2?}", self.codegen)?;
        write!(f, "  total      {:>8.2?}", total)
    }
}
