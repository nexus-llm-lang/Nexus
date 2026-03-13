//! Mid-level IR — throws eliminated via static port resolution.
//! No inject/handler/port.method() — all port calls resolved to direct function calls.

use crate::intern::Symbol;
use crate::types::{BinaryOp, Literal, Sigil, Span, Type};

#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>,
    pub externals: Vec<MirExternal>,
    pub enum_defs: Vec<crate::types::EnumDef>,
}

#[derive(Debug, Clone)]
pub struct MirExternal {
    pub name: Symbol,
    pub wasm_module: Symbol,
    pub wasm_name: Symbol,
    pub params: Vec<MirParam>,
    pub ret_type: Type,
    pub throws: Type,
}

#[derive(Debug, Clone)]
pub struct MirFunction {
    pub name: Symbol,
    pub params: Vec<MirParam>,
    pub ret_type: Type,
    pub body: Vec<MirStmt>,
    pub span: Span,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct MirParam {
    pub label: Symbol,
    pub name: Symbol,
    pub typ: Type,
}

#[derive(Debug, Clone)]
pub enum MirStmt {
    Let {
        name: Symbol,
        typ: Type,
        expr: MirExpr,
    },
    Expr(MirExpr),
    Return(MirExpr),
    Assign {
        target: MirExpr,
        value: MirExpr,
    },
    Conc(Vec<MirFunction>),
    Try {
        body: Vec<MirStmt>,
        catch_param: Symbol,
        catch_body: Vec<MirStmt>,
    },
}

#[derive(Debug, Clone)]
pub enum MirExpr {
    Literal(Literal),
    Variable(Symbol),
    BinaryOp(Box<MirExpr>, BinaryOp, Box<MirExpr>),
    Call {
        func: Symbol,
        args: Vec<(Symbol, MirExpr)>,
        ret_type: Type,
    },
    Constructor {
        name: Symbol,
        args: Vec<(Option<Symbol>, MirExpr)>,
    },
    Record(Vec<(Symbol, MirExpr)>),
    Array(Vec<MirExpr>),
    Index(Box<MirExpr>, Box<MirExpr>),
    FieldAccess(Box<MirExpr>, Symbol),
    If {
        cond: Box<MirExpr>,
        then_body: Vec<MirStmt>,
        else_body: Option<Vec<MirStmt>>,
    },
    Match {
        target: Box<MirExpr>,
        cases: Vec<MirMatchCase>,
    },
    While {
        cond: Box<MirExpr>,
        body: Vec<MirStmt>,
    },
    Borrow(Symbol),
    Raise(Box<MirExpr>),
}

#[derive(Debug, Clone)]
pub struct MirMatchCase {
    pub pattern: MirPattern,
    pub body: Vec<MirStmt>,
}

#[derive(Debug, Clone)]
pub enum MirPattern {
    Literal(Literal),
    Variable(Symbol, Sigil),
    Constructor {
        name: Symbol,
        fields: Vec<(Option<Symbol>, MirPattern)>,
    },
    Record(Vec<(Symbol, MirPattern)>, bool),
    Wildcard,
}
