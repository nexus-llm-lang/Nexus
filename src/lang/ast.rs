// Re-export core types so that `use super::ast::*` / `use crate::lang::ast::*` continues
// to work for modules that depend on both AST-specific and shared types.
pub use crate::types::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub sigil: Sigil, // Arguments might have sigils too? e.g. tx: %Tx
    pub typ: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Literal),
    Variable(String, Sigil), // e.g. case Ok(%new_tx)
    Constructor(String, Vec<(Option<String>, Spanned<Pattern>)>),
    Record(Vec<(String, Spanned<Pattern>)>, bool), // { x: p, _ }
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: Spanned<Pattern>,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Variable(String, Sigil),
    // Binary operations (e.g. +, -) are allowed in expressions
    BinaryOp(Box<Spanned<Expr>>, BinaryOp, Box<Spanned<Expr>>),
    Borrow(String, Sigil), // borrow %x
    // Function calls
    Call {
        func: String,
        args: Vec<(String, Spanned<Expr>)>, // label, value
    },
    Constructor(String, Vec<(Option<String>, Spanned<Expr>)>),
    Record(Vec<(String, Spanned<Expr>)>),
    Array(Vec<Spanned<Expr>>),                     // [| 1, 2, 3 |]
    List(Vec<Spanned<Expr>>),                      // [1, 2, 3]
    Index(Box<Spanned<Expr>>, Box<Spanned<Expr>>), // a[i]
    FieldAccess(Box<Spanned<Expr>>, String),
    // If and Match can be expressions or statements.
    // In many FP languages they are expressions.
    If {
        cond: Box<Spanned<Expr>>,
        then_branch: Vec<Spanned<Stmt>>,
        else_branch: Option<Vec<Spanned<Stmt>>>,
    },
    Match {
        target: Box<Spanned<Expr>>,
        cases: Vec<MatchCase>,
    },
    While {
        cond: Box<Spanned<Expr>>,
        body: Vec<Spanned<Stmt>>,
    },
    For {
        var: String,
        start: Box<Spanned<Expr>>,
        end_expr: Box<Spanned<Expr>>,
        body: Vec<Spanned<Stmt>>,
    },
    Lambda {
        type_params: Vec<String>,
        params: Vec<Param>,
        ret_type: Type,
        requires: Type,
        throws: Type,
        body: Vec<Spanned<Stmt>>,
    },
    Raise(Box<Spanned<Expr>>),           // raise "error"
    External(String, Vec<String>, Type), // external "wasm_symbol" : <T> arrow_type
    // handler Port [require { ... }] do fn ... end end — coeffect handler as expression
    Handler {
        coeffect_name: String,
        requires: Type,
        functions: Vec<Function>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        sigil: Sigil,
        typ: Option<Type>,
        value: Spanned<Expr>,
    },
    Expr(Spanned<Expr>), // For side-effecting calls or match/if used as statement
    Return(Spanned<Expr>),
    // Assignment: target <- value
    Assign {
        target: Spanned<Expr>,
        value: Spanned<Expr>,
    },
    // Concurrent block
    Conc(Vec<Function>), // 'task' blocks look like functions/closures
    Try {
        body: Vec<Spanned<Stmt>>,
        catch_param: String,
        catch_body: Vec<Spanned<Stmt>>,
    },
    // inject handler_var, ... do body end
    Inject {
        handlers: Vec<String>,
        body: Vec<Spanned<Stmt>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub is_public: bool,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub ret_type: Type,
    pub requires: Type,
    pub throws: Type,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub is_public: bool,
    pub type_params: Vec<String>,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExceptionDef {
    pub name: String,
    pub is_public: bool,
    pub fields: Vec<(Option<String>, Type)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: String,
    pub alias: Option<String>,
    pub items: Vec<String>,
    pub is_external: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: String,
    pub is_public: bool,
    pub functions: Vec<FunctionSignature>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionSignature {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_type: Type,
    pub requires: Type,
    pub throws: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlobalLet {
    pub name: String,
    pub is_public: bool,
    pub typ: Option<Type>,
    pub value: Spanned<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    TypeDef(TypeDef),
    Enum(EnumDef),
    Exception(ExceptionDef),
    Import(Import),
    Port(Port),
    Let(GlobalLet),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub definitions: Vec<Spanned<TopLevel>>,
}
