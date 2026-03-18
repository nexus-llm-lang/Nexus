//! Core types shared across all compiler layers (AST, HIR, MIR, LIR, codegen).
//!
//! These types were extracted from `lang::ast` because they represent fundamental
//! language concepts used by every stage of the pipeline, not just the AST.

use std::ops::Range;

pub type Span = Range<usize>;

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

/// WASM-level value representation for a Nexus type.
///
/// Single source of truth for "what WASM value type does this Nexus type use at runtime?"
/// Adding a new `Type` variant requires updating only `Type::wasm_repr()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmRepr {
    /// i32: Bool, Char, I32
    I32,
    /// i64: I64, String, and all heap-allocated types (Array, List, Record, etc.)
    I64,
    /// f32: F32
    F32,
    /// f64: F64
    F64,
    /// No runtime value: Unit, Row
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I32,
    I64,
    F32,
    F64,
    IntLit,
    FloatLit,
    Bool,
    Char,
    String,
    Unit,
    UserDefined(String, Vec<Type>),
    Var(String),
    Arrow(Vec<(String, Type)>, Box<Type>, Box<Type>, Box<Type>), // params, return, require, throws
    Ref(Box<Type>),
    Linear(Box<Type>),                 // %T
    Row(Vec<Type>, Option<Box<Type>>), // { E1, E2 | r }
    Record(Vec<(String, Type)>),       // { x: i64, y: string }
    Array(Box<Type>),                  // [| T |]
    List(Box<Type>),                   // [T]
    Borrow(Box<Type>),                 // &T
    Handler(String, Box<Type>),        // handler Port require { ... }
}

impl Type {
    /// Classify this type into its WASM-level value representation.
    ///
    /// This is the single dispatcher for "is this type primitive or heap?"
    /// All codegen paths that need this classification should use this method.
    pub fn wasm_repr(&self) -> WasmRepr {
        match self {
            Type::Linear(inner) => inner.wasm_repr(),
            Type::I32 | Type::Bool | Type::Char => WasmRepr::I32,
            Type::I64
            | Type::IntLit
            | Type::String
            | Type::Array(_)
            | Type::List(_)
            | Type::Record(_)
            | Type::UserDefined(_, _)
            | Type::Var(_)
            | Type::Borrow(_)
            | Type::Ref(_)
            | Type::Handler(_, _)
            | Type::Arrow(_, _, _, _) => WasmRepr::I64,
            Type::F32 => WasmRepr::F32,
            Type::F64 | Type::FloatLit => WasmRepr::F64,
            Type::Unit | Type::Row(_, _) => WasmRepr::Unit,
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::I32 => write!(f, "i32"),
            Type::I64 => write!(f, "i64"),
            Type::F32 => write!(f, "f32"),
            Type::F64 => write!(f, "f64"),
            Type::IntLit => write!(f, "i64"),
            Type::FloatLit => write!(f, "f64"),
            Type::Bool => write!(f, "bool"),
            Type::Char => write!(f, "char"),
            Type::String => write!(f, "string"),
            Type::Unit => write!(f, "unit"),
            Type::UserDefined(name, args) => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Type::Var(name) => write!(f, "{}", name),
            Type::Arrow(params, ret, req, eff) => {
                write!(f, "(")?;
                for (i, (name, typ)) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, typ)?;
                }
                write!(f, ") -> {}", ret)?;
                match &**req {
                    Type::Row(reqs, tail) if reqs.is_empty() && tail.is_none() => {}
                    _ => write!(f, " require {}", req)?,
                }
                match &**eff {
                    Type::Row(effs, tail) if effs.is_empty() && tail.is_none() => {}
                    _ => write!(f, " throws {}", eff)?,
                }
                Ok(())
            }
            Type::Ref(t) => write!(f, "~{}", t),
            Type::Linear(t) => write!(f, "%{}", t),
            Type::Row(effs, tail) => {
                write!(f, "{{")?;
                for (i, eff) in effs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", eff)?;
                }
                if let Some(t) = tail {
                    if !effs.is_empty() {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", t)?;
                }
                write!(f, "}}")
            }
            Type::Record(fields) => {
                write!(f, "{{")?;
                for (i, (name, typ)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, typ)?;
                }
                write!(f, "}}")
            }
            Type::Array(t) => write!(f, "[| {} |]", t),
            Type::List(t) => write!(f, "[{}]", t),
            Type::Borrow(t) => write!(f, "&{}", t),
            Type::Handler(name, req) => {
                write!(f, "handler {}", name)?;
                match &**req {
                    Type::Row(reqs, tail) if reqs.is_empty() && tail.is_none() => {}
                    _ => write!(f, " require {}", req)?,
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Sigil {
    Immutable,
    Mutable, // ~
    Linear,  // %
    Borrow,  // &
}

impl Sigil {
    /// Returns the source-level sigil prefix (`"", "~", "%"`) for this binding kind.
    pub fn to_prefix(&self) -> &'static str {
        match self {
            Sigil::Immutable => "",
            Sigil::Mutable => "~",
            Sigil::Linear => "%",
            Sigil::Borrow => "&",
        }
    }

    /// Builds the canonical variable key used in environments.
    pub fn get_key(&self, name: &str) -> String {
        format!("{}{}", self.to_prefix(), name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    String(String), // Includes RawString
    Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    // Integer arithmetic: + - * / %
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Bitwise: band bor bxor bshl bshr
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    // Float arithmetic: +. -. *. /.
    FAdd,
    FSub,
    FMul,
    FDiv,
    // Integer comparison: == != < <= > >=
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Float comparison: ==. !=. <. <=. >. >=.
    FEq,
    FNe,
    FLt,
    FLe,
    FGt,
    FGe,
    // String concatenation: ++
    Concat,
    // Boolean: && ||
    And,
    Or,
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::BitAnd => "band",
            BinaryOp::BitOr => "bor",
            BinaryOp::BitXor => "bxor",
            BinaryOp::Shl => "bshl",
            BinaryOp::Shr => "bshr",
            BinaryOp::FAdd => "+.",
            BinaryOp::FSub => "-.",
            BinaryOp::FMul => "*.",
            BinaryOp::FDiv => "/.",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::FEq => "==.",
            BinaryOp::FNe => "!=.",
            BinaryOp::FLt => "<.",
            BinaryOp::FLe => "<=.",
            BinaryOp::FGt => ">.",
            BinaryOp::FGe => ">=.",
            BinaryOp::Concat => "++",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        };
        write!(f, "{}", s)
    }
}

impl BinaryOp {
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::FEq
                | BinaryOp::FNe
                | BinaryOp::FLt
                | BinaryOp::FLe
                | BinaryOp::FGt
                | BinaryOp::FGe
        )
    }

    pub fn is_float_op(self) -> bool {
        matches!(
            self,
            BinaryOp::FAdd
                | BinaryOp::FSub
                | BinaryOp::FMul
                | BinaryOp::FDiv
                | BinaryOp::FEq
                | BinaryOp::FNe
                | BinaryOp::FLt
                | BinaryOp::FLe
                | BinaryOp::FGt
                | BinaryOp::FGe
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub is_public: bool,
    pub is_opaque: bool,
    pub type_params: Vec<String>,
    pub variants: Vec<VariantDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<(Option<String>, Type)>,
}
