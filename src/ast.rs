use std::ops::Range;

pub type Span = Range<usize>;

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I64,
    Float,
    Bool,
    Str,
    Unit,
    Result(Box<Type>, Box<Type>),
    UserDefined(String, Vec<Type>),
    Var(String),
    Arrow(Vec<(String, Type)>, Box<Type>, Box<Type>),
    Ref(Box<Type>),
    Linear(Box<Type>), // %T
    Row(Vec<Type>, Option<Box<Type>>), // { E1, E2 | r }
    Record(Vec<(String, Type)>), // { x: i64, y: str }
    List(Box<Type>), // [T]
    Array(Box<Type>), // [| T |]
    Borrow(Box<Type>), // &T
}

#[derive(Debug, Clone, PartialEq)]
pub enum Sigil {
    Immutable,
    Mutable, // ~
    Linear,  // %
}

impl Sigil {
    pub fn to_prefix(&self) -> &'static str {
        match self {
            Sigil::Immutable => "",
            Sigil::Mutable => "~",
            Sigil::Linear => "%",
        }
    }

    pub fn get_key(&self, name: &str) -> String {
        format!("{}{}", self.to_prefix(), name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub sigil: Sigil, // Arguments might have sigils too? e.g. tx: %Tx
    pub typ: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String), // Includes RawString
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Literal),
    Variable(String, Sigil), // e.g. case Ok(%new_tx)
    Constructor(String, Vec<Spanned<Pattern>>),
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
    BinaryOp(Box<Spanned<Expr>>, String, Box<Spanned<Expr>>),
    Borrow(String, Sigil), // borrow %x
    // Function calls
    Call {
        func: String,
        args: Vec<(String, Spanned<Expr>)>, // label, value
        perform: bool,             // true if 'perform' keyword is used
    },
    Constructor(String, Vec<Spanned<Expr>>),
    Record(Vec<(String, Spanned<Expr>)>),
    List(Vec<Spanned<Expr>>), // [1, 2, 3]
    Array(Vec<Spanned<Expr>>), // [| 1, 2, 3 |]
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
    Raise(Box<Spanned<Expr>>), // raise "error"
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
    Comment,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub is_public: bool,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub ret_type: Type,
    pub effects: Type,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<VariantDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub module: String, // quoted string
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub name: String,
    pub functions: Vec<FunctionSignature>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub name: String,
    pub port_name: String,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionSignature {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_type: Type,
    pub effects: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Function(Function),
    TypeDef(TypeDef),
    Enum(EnumDef),
    Import(Import),
    Port(Port),
    Handler(Handler),
    Comment,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub definitions: Vec<Spanned<TopLevel>>,
}
