use std::ops::Range;

pub type Span = Range<usize>;

#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
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
    String,
    Unit,
    UserDefined(String, Vec<Type>),
    Var(String),
    Arrow(Vec<(String, Type)>, Box<Type>, Box<Type>, Box<Type>), // params, return, require, effect
    Ref(Box<Type>),
    Linear(Box<Type>),                 // %T
    Row(Vec<Type>, Option<Box<Type>>), // { E1, E2 | r }
    Record(Vec<(String, Type)>),       // { x: i64, y: string }
    Array(Box<Type>),                  // [| T |]
    List(Box<Type>),                   // [T]
    Borrow(Box<Type>),                 // &T
    Handler(String, Box<Type>),         // handler Port require { ... }
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
                    _ => write!(f, " effect {}", eff)?,
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
    Constructor(String, Vec<(Option<String>, Spanned<Pattern>)>),
    Record(Vec<(String, Spanned<Pattern>)>, bool), // { x: p, _ }
    Wildcard,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: Spanned<Pattern>,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    // Integer arithmetic: + - * /
    Add,
    Sub,
    Mul,
    Div,
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
    Lambda {
        type_params: Vec<String>,
        params: Vec<Param>,
        ret_type: Type,
        requires: Type,
        effects: Type,
        body: Vec<Spanned<Stmt>>,
    },
    Raise(Box<Spanned<Expr>>), // raise "error"
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
    pub effects: Type,
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
    pub effects: Type,
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
