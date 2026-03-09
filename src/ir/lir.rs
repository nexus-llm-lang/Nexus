//! Low-level IR in A-normal form (ANF) — all sub-expressions are atoms.
//! This is the final IR before WASM codegen; codegen consumes LIR directly.

use crate::lang::ast::{BinaryOp, Span, Type};

#[derive(Debug, Clone, PartialEq)]
pub struct LirProgram {
    pub functions: Vec<LirFunction>,
    pub externals: Vec<LirExternal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LirExternal {
    pub name: String,
    pub wasm_module: String,
    pub wasm_name: String,
    pub params: Vec<LirParam>,
    pub ret_type: Type,
    pub throws: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LirFunction {
    pub name: String,
    pub params: Vec<LirParam>,
    pub ret_type: Type,
    pub requires: Type,
    pub throws: Type,
    pub body: Vec<LirStmt>,
    pub ret: LirAtom,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LirParam {
    pub label: String,
    pub name: String,
    pub typ: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LirStmt {
    Let {
        name: String,
        typ: Type,
        expr: LirExpr,
    },
    If {
        cond: LirAtom,
        then_body: Vec<LirStmt>,
        else_body: Vec<LirStmt>,
    },
    IfReturn {
        cond: LirAtom,
        then_body: Vec<LirStmt>,
        then_ret: LirAtom,
        else_body: Vec<LirStmt>,
        else_ret: Option<LirAtom>,
        ret_type: Type,
    },
    TryCatch {
        body: Vec<LirStmt>,
        body_ret: Option<LirAtom>,
        catch_param: String,
        catch_param_typ: Type,
        catch_body: Vec<LirStmt>,
        catch_ret: Option<LirAtom>,
    },
    Conc {
        tasks: Vec<ConcTask>,
    },
    /// Loop with condition check at the top.
    /// cond_stmts compute the break condition, then cond is checked.
    /// If cond is true, break. Otherwise, execute body and repeat.
    Loop {
        cond_stmts: Vec<LirStmt>,
        cond: LirAtom,
        body: Vec<LirStmt>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConcTask {
    pub func_name: String,
    pub args: Vec<(String, LirAtom)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LirExpr {
    Atom(LirAtom),
    Binary {
        op: BinaryOp,
        lhs: LirAtom,
        rhs: LirAtom,
        typ: Type,
    },
    Call {
        func: String,
        args: Vec<(String, LirAtom)>,
        typ: Type,
    },
    TailCall {
        func: String,
        args: Vec<(String, LirAtom)>,
        typ: Type,
    },
    Constructor {
        name: String,
        args: Vec<LirAtom>,
        typ: Type,
    },
    Record {
        fields: Vec<(String, LirAtom)>,
        typ: Type,
    },
    ObjectTag {
        value: LirAtom,
        typ: Type,
    },
    ObjectField {
        value: LirAtom,
        index: usize,
        typ: Type,
    },
    Raise {
        value: LirAtom,
        typ: Type,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum LirAtom {
    Var { name: String, typ: Type },
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Unit,
}

impl LirAtom {
    pub fn typ(&self) -> Type {
        match self {
            LirAtom::Var { typ, .. } => typ.clone(),
            LirAtom::Int(_) => Type::I64,
            LirAtom::Float(_) => Type::F64,
            LirAtom::Bool(_) => Type::Bool,
            LirAtom::String(_) => Type::String,
            LirAtom::Unit => Type::Unit,
        }
    }
}
