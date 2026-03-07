//! Mid-level IR — effects eliminated via static port resolution.
//! No inject/handler/port.method() — all port calls resolved to direct function calls.

use crate::lang::ast::{BinaryOp, Literal, Sigil, Type};

#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: Vec<MirFunction>,
    pub externals: Vec<MirExternal>,
}

#[derive(Debug, Clone)]
pub struct MirExternal {
    pub name: String,
    pub wasm_module: String,
    pub wasm_name: String,
    pub params: Vec<MirParam>,
    pub ret_type: Type,
    pub effects: Type,
}

#[derive(Debug, Clone)]
pub struct MirFunction {
    pub name: String,
    pub params: Vec<MirParam>,
    pub ret_type: Type,
    pub body: Vec<MirStmt>,
}

#[derive(Debug, Clone)]
pub struct MirParam {
    pub label: String,
    pub name: String,
    pub typ: Type,
}

#[derive(Debug, Clone)]
pub enum MirStmt {
    Let {
        name: String,
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
        catch_param: String,
        catch_body: Vec<MirStmt>,
    },
}

#[derive(Debug, Clone)]
pub enum MirExpr {
    Literal(Literal),
    Variable(String),
    BinaryOp(Box<MirExpr>, BinaryOp, Box<MirExpr>),
    Call {
        func: String,
        args: Vec<(String, MirExpr)>,
        ret_type: Type,
    },
    Constructor {
        name: String,
        args: Vec<MirExpr>,
    },
    Record(Vec<(String, MirExpr)>),
    Array(Vec<MirExpr>),
    Index(Box<MirExpr>, Box<MirExpr>),
    FieldAccess(Box<MirExpr>, String),
    If {
        cond: Box<MirExpr>,
        then_body: Vec<MirStmt>,
        else_body: Option<Vec<MirStmt>>,
    },
    Match {
        target: Box<MirExpr>,
        cases: Vec<MirMatchCase>,
    },
    Borrow(String),
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
    Variable(String, Sigil),
    Constructor {
        name: String,
        fields: Vec<(Option<String>, MirPattern)>,
    },
    Record(Vec<(String, MirPattern)>, bool),
    Wildcard,
}
