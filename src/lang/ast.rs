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

/// A single arm of a selective catch block.
/// Each arm matches a specific exception pattern (constructor or wildcard).
#[derive(Debug, Clone, PartialEq)]
pub struct CatchArm {
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
    Force(Box<Spanned<Expr>>),           // @expr (evaluate/force lazy)
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
    Try {
        body: Vec<Spanned<Stmt>>,
        catch_arms: Vec<CatchArm>,
    },
    // inject handler_var, ... do body end
    Inject {
        handlers: Vec<String>,
        body: Vec<Spanned<Stmt>>,
    },
    // Destructuring let: `let {x, y} = expr` or `let Some(v) = expr`
    // Desugared to a single-case match during HIR lowering.
    LetPattern {
        pattern: Spanned<Pattern>,
        value: Spanned<Expr>,
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

/// Groups multiple exception types under a single name (Phase 2).
/// `exception group IOError = NotFound | PermDenied`
#[derive(Debug, Clone, PartialEq)]
pub struct ExceptionGroupDef {
    pub name: String,
    pub is_public: bool,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: String,
    pub alias: Option<String>,
    pub items: Vec<ImportItem>,
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
    ExceptionGroup(ExceptionGroupDef),
    Import(Import),
    Port(Port),
    Let(GlobalLet),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub definitions: Vec<Spanned<TopLevel>>,
    /// Source file path — set by the driver after parsing.
    pub source_file: Option<String>,
    /// Raw source text — needed for byte-offset → line-number conversion.
    pub source_text: Option<String>,
}
