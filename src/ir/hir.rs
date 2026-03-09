//! High-level IR — name-resolved AST with modules flattened.
//! All identifiers are fully qualified. Handlers collected.

use crate::lang::ast::{BinaryOp, EnumDef, Literal, Sigil, Span, Type};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct HirProgram {
    pub ports: Vec<HirPort>,
    pub functions: Vec<HirFunction>,
    pub externals: Vec<HirExternal>,
    /// binding_name → handler binding (port name + method implementations)
    pub handler_bindings: HashMap<String, HirHandlerBinding>,
    /// All enum definitions (stdlib + user-defined + imported)
    pub enum_defs: Vec<EnumDef>,
}

#[derive(Debug, Clone)]
pub struct HirHandlerBinding {
    pub port_name: String,
    pub functions: Vec<HirFunction>,
}

#[derive(Debug, Clone)]
pub struct HirPort {
    pub name: String,
    pub functions: Vec<HirPortMethod>,
}

#[derive(Debug, Clone)]
pub struct HirPortMethod {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct HirExternal {
    pub name: String,
    pub wasm_module: String,
    pub wasm_name: String,
    pub params: Vec<HirParam>,
    pub ret_type: Type,
    pub effects: Type,
}

#[derive(Debug, Clone)]
pub struct HirFunction {
    pub name: String, // fully qualified: "module::fn_name"
    pub params: Vec<HirParam>,
    pub ret_type: Type,
    pub body: Vec<HirStmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: String,
    pub label: String,
    pub typ: Type,
}

#[derive(Debug, Clone)]
pub enum HirStmt {
    Let {
        name: String,
        typ: Option<Type>,
        value: HirExpr,
    },
    Expr(HirExpr),
    Return(HirExpr),
    Assign {
        target: HirExpr,
        value: HirExpr,
    },
    Conc(Vec<HirFunction>),
    Try {
        body: Vec<HirStmt>,
        catch_param: String,
        catch_body: Vec<HirStmt>,
    },
    Inject {
        handlers: Vec<String>,
        body: Vec<HirStmt>,
    },
}

#[derive(Debug, Clone)]
pub enum HirExpr {
    Literal(Literal),
    Variable(String, Sigil),
    BinaryOp(Box<HirExpr>, BinaryOp, Box<HirExpr>),
    Borrow(String, Sigil),
    Call {
        func: String,
        args: Vec<(String, HirExpr)>,
    },
    Constructor {
        variant: String,
        args: Vec<HirExpr>,
    },
    Record(Vec<(String, HirExpr)>),
    Array(Vec<HirExpr>),
    Index(Box<HirExpr>, Box<HirExpr>),
    FieldAccess(Box<HirExpr>, String),
    If {
        cond: Box<HirExpr>,
        then_branch: Vec<HirStmt>,
        else_branch: Option<Vec<HirStmt>>,
    },
    Match {
        target: Box<HirExpr>,
        cases: Vec<HirMatchCase>,
    },
    While {
        cond: Box<HirExpr>,
        body: Vec<HirStmt>,
    },
    For {
        var: String,
        start: Box<HirExpr>,
        end_expr: Box<HirExpr>,
        body: Vec<HirStmt>,
    },
    Lambda {
        params: Vec<HirParam>,
        ret_type: Type,
        body: Vec<HirStmt>,
    },
    Raise(Box<HirExpr>),
    External(String, Vec<String>, Type),
    Handler {
        functions: Vec<HirFunction>,
    },
}

#[derive(Debug, Clone)]
pub struct HirMatchCase {
    pub pattern: HirPattern,
    pub body: Vec<HirStmt>,
}

#[derive(Debug, Clone)]
pub enum HirPattern {
    Literal(Literal),
    Variable(String, Sigil),
    Constructor {
        variant: String,
        fields: Vec<(Option<String>, HirPattern)>,
    },
    Record(Vec<(String, HirPattern)>, bool),
    Wildcard,
}
