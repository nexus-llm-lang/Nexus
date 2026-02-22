use crate::lang::ast::Type;

#[derive(Debug, Clone, PartialEq)]
pub struct AnfProgram {
    pub functions: Vec<AnfFunction>,
    pub externals: Vec<AnfExternal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnfExternal {
    pub name: String,
    pub wasm_module: String,
    pub wasm_name: String,
    pub params: Vec<AnfParam>,
    pub ret_type: Type,
    pub effects: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnfFunction {
    pub name: String,
    pub params: Vec<AnfParam>,
    pub ret_type: Type,
    pub effects: Type,
    pub body: Vec<AnfStmt>,
    pub ret: AnfAtom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnfParam {
    pub label: String,
    pub name: String,
    pub typ: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnfStmt {
    Let {
        name: String,
        typ: Type,
        expr: AnfExpr,
    },
    Drop(AnfAtom),
    If {
        cond: AnfAtom,
        then_body: Vec<AnfStmt>,
        else_body: Vec<AnfStmt>,
    },
    IfReturn {
        cond: AnfAtom,
        then_body: Vec<AnfStmt>,
        then_ret: AnfAtom,
        else_body: Vec<AnfStmt>,
        else_ret: Option<AnfAtom>,
        ret_type: Type,
    },
    TryCatch {
        body: Vec<AnfStmt>,
        body_ret: Option<AnfAtom>,
        catch_param: String,
        catch_param_typ: Type,
        catch_body: Vec<AnfStmt>,
        catch_ret: Option<AnfAtom>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnfExpr {
    Atom(AnfAtom),
    Binary {
        op: String,
        lhs: AnfAtom,
        rhs: AnfAtom,
        typ: Type,
    },
    Call {
        func: String,
        args: Vec<(String, AnfAtom)>,
        typ: Type,
        perform: bool,
    },
    Constructor {
        name: String,
        args: Vec<AnfAtom>,
        typ: Type,
    },
    Record {
        fields: Vec<(String, AnfAtom)>,
        typ: Type,
    },
    ObjectTag {
        value: AnfAtom,
        typ: Type,
    },
    ObjectField {
        value: AnfAtom,
        index: usize,
        typ: Type,
    },
    Raise {
        value: AnfAtom,
        typ: Type,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnfAtom {
    Var { name: String, typ: Type },
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Unit,
}

impl AnfAtom {
    /// Returns the static Nexus type represented by this atom.
    pub fn typ(&self) -> Type {
        match self {
            AnfAtom::Var { typ, .. } => typ.clone(),
            AnfAtom::Int(_) => Type::I64,
            AnfAtom::Float(_) => Type::F64,
            AnfAtom::Bool(_) => Type::Bool,
            AnfAtom::String(_) => Type::String,
            AnfAtom::Unit => Type::Unit,
        }
    }
}
