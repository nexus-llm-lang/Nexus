#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I64,
    Bool,
    Str,
    Unit,
    Result(Box<Type>, Box<Type>),
    UserDefined(String, Vec<Type>),
    Var(String),
    Arrow(Vec<Type>, Box<Type>, Box<Type>),
    Ref(Box<Type>),
    Linear(Box<Type>), // %T
    Row(Vec<Type>, Option<Box<Type>>), // { E1, E2 | r }
    Record(Vec<(String, Type)>), // { x: i64, y: str }
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
    Bool(bool),
    String(String), // Includes RawString
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Literal),
    Variable(String, Sigil), // e.g. case Ok(%new_tx)
    Constructor(String, Vec<Pattern>),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Variable(String, Sigil),
    // Binary operations (e.g. +, -) are allowed in expressions
    BinaryOp(Box<Expr>, String, Box<Expr>),
    // Function calls
    Call {
        func: String,
        args: Vec<(String, Expr)>, // label, value
        perform: bool,             // true if 'perform' keyword is used
    },
    Constructor(String, Vec<Expr>),
    Record(Vec<(String, Expr)>),
    FieldAccess(Box<Expr>, String),
    // If and Match can be expressions or statements.
    // In many FP languages they are expressions.
    If {
        cond: Box<Expr>,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
    },
    Match {
        target: Box<Expr>,
        cases: Vec<MatchCase>,
    },
    Raise(Box<Expr>), // raise "error"
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        sigil: Sigil,
        typ: Option<Type>,
        value: Expr,
    },
    Expr(Expr), // For side-effecting calls or match/if used as statement
    Return(Expr),
    // Assignment for mutable variables: ~counter <- ~counter + 1
    Assign {
        name: String,
        sigil: Sigil,
        value: Expr,
    },
    // Concurrent block
    Conc(Vec<Function>), // 'task' blocks look like functions/closures
    Try {
        body: Vec<Stmt>,
        catch_param: String,
        catch_body: Vec<Stmt>,
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
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<(String, Type)>,
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
    Import(Import),
    Port(Port),
    Handler(Handler),
    Comment,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub definitions: Vec<TopLevel>,
}
