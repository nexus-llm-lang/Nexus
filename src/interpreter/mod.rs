use crate::ast::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

mod stdlib;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Unit,
    Record(HashMap<String, Value>),
    Variant(String, Vec<Value>),
    List(Vec<Value>),
    Array(Rc<RefCell<Vec<Value>>>),
    Ref(Rc<RefCell<Value>>),
}

#[derive(Debug, Clone)]
pub enum ExprResult {
    Normal(Value),
    EarlyReturn(Value),
}

type EvalResult = Result<ExprResult, String>;

#[derive(Debug, Clone)]
pub struct Env {
    vars: HashMap<String, Value>,
    parent: Option<Box<Env>>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            vars: HashMap::new(),
            parent: None,
        }
    }

    pub fn extend(parent: Env) -> Self {
        Env {
            vars: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        match self.vars.get(name) {
            Some(v) => Some(v.clone()),
            None => self.parent.as_ref().and_then(|p| p.get(name)),
        }
    }

    pub fn define(&mut self, name: String, value: Value) {
        self.vars.insert(name, value);
    }
}

pub struct Interpreter {
    pub functions: HashMap<String, Function>,
    pub handlers: HashMap<String, Handler>,
}

impl Interpreter {
    pub fn new(program: Program) -> Self {
        let mut functions = HashMap::new();
        let mut handlers = HashMap::new();
        for def in program.definitions {
            match def.node {
                TopLevel::Function(func) => {
                    functions.insert(func.name.clone(), func);
                }
                TopLevel::Handler(handler) => {
                    handlers.insert(handler.port_name.clone(), handler);
                }
                _ => {}
            }
        }
        Interpreter { functions, handlers }
    }

    pub fn eval_repl_stmt(&mut self, stmt: &Spanned<Stmt>, env: &mut Env) -> EvalResult {
        match &stmt.node {
            Stmt::Expr(expr) => self.eval_expr(expr, env),
            _ => self.eval_body(&[stmt.clone()], env),
        }
    }

    pub fn run_function(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        let func = self
            .functions
            .get(name)
            .ok_or_else(|| format!("Function '{}' not found", name))?
            .clone();

        if func.params.len() != args.len() {
            return Err(format!(
                "Arity mismatch: expected {}, got {}",
                func.params.len(),
                args.len()
            ));
        }

        let mut env = Env::new();
        for (param, arg) in func.params.iter().zip(args.iter()) {
            env.define(param.name.clone(), arg.clone());
        }

        let result = self.eval_body(&func.body, &mut env)?;
        match result {
            ExprResult::Normal(v) => Ok(v),
            ExprResult::EarlyReturn(v) => Ok(v),
        }
    }

    fn eval_body(&mut self, body: &[Spanned<Stmt>], env: &mut Env) -> EvalResult {
        for stmt in body {
            match &stmt.node {
                                Stmt::Let { name, sigil, value, .. } => {
                                    let res = self.eval_expr(value, env)?;
                                    match res {
                                        ExprResult::Normal(val) => {
                                            let final_val = if let Sigil::Mutable = sigil {
                                                Value::Ref(Rc::new(RefCell::new(val)))
                                            } else {
                                                val
                                            };
                                            env.define(sigil.get_key(name), final_val);
                                        },
                                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                                    }
                                }
                
                Stmt::Return(expr) => {
                    let res = self.eval_expr(expr, env)?;
                    match res {
                        ExprResult::Normal(val) => return Ok(ExprResult::EarlyReturn(val)),
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }
                Stmt::Expr(expr) => {
                    let res = self.eval_expr(expr, env)?;
                    if let ExprResult::EarlyReturn(_) = res {
                        return Ok(res);
                    }
                }
                Stmt::Conc(tasks) => {
                    for task in tasks {
                        let _ = self.eval_body(&task.body, env)?;
                    }
                }
                Stmt::Try { body, catch_param, catch_body } => {
                    let res = self.eval_body(body, env);
                    match res {
                        Ok(ExprResult::EarlyReturn(val)) => return Ok(ExprResult::EarlyReturn(val)),
                        Ok(ExprResult::Normal(_)) => {},
                        Err(msg) => {
                            let mut catch_env = Env::extend(env.clone());
                            catch_env.define(catch_param.clone(), Value::String(msg));
                            let catch_res = self.eval_body(catch_body, &mut catch_env)?;
                            if let ExprResult::EarlyReturn(v) = catch_res {
                                return Ok(ExprResult::EarlyReturn(v));
                            }
                        }
                    }
                }
                Stmt::Assign { target, value } => {
                    let val_res = self.eval_expr(value, env)?;
                    let val = match val_res {
                        ExprResult::Normal(v) => v,
                        ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                    };

                    match &target.node {
                        Expr::Variable(name, sigil) => {
                            let key = sigil.get_key(name);
                            if let Some(target_val) = env.get(&key) {
                                if let Value::Ref(r) = target_val {
                                    *r.borrow_mut() = val;
                                } else {
                                    return Err(format!("Cannot assign to immutable variable {}", name));
                                }
                            } else {
                                return Err(format!("Variable {} not found", key));
                            }
                        }
                        Expr::Index(arr, idx) => {
                            let arr_res = self.eval_expr(arr, env)?;
                            let idx_res = self.eval_expr(idx, env)?;
                            match (arr_res, idx_res) {
                                (ExprResult::Normal(Value::Array(a)), ExprResult::Normal(Value::Int(i))) => {
                                    let mut l = a.borrow_mut();
                                    let idx = i as usize;
                                    if idx < l.len() {
                                        l[idx] = val;
                                    } else {
                                        return Err(format!("Index out of bounds: {}", idx));
                                    }
                                }
                                (ExprResult::EarlyReturn(v), _) | (_, ExprResult::EarlyReturn(v)) => return Ok(ExprResult::EarlyReturn(v)),
                                _ => return Err("Invalid array assignment".to_string()),
                            }
                        }
                        _ => return Err("Invalid assignment target".to_string()),
                    }
                }
                Stmt::Comment => continue,
            }
        }
        Ok(ExprResult::Normal(Value::Unit))
    }

    fn eval_expr(&mut self, expr: &Spanned<Expr>, env: &mut Env) -> EvalResult {
        match &expr.node {
            Expr::Literal(lit) => Ok(ExprResult::Normal(match lit {
                Literal::Int(i) => Value::Int(*i),
                Literal::Float(f) => Value::Float(*f),
                Literal::Bool(b) => Value::Bool(*b),
                Literal::String(s) => Value::String(s.clone()),
                Literal::Unit => Value::Unit,
            })),
            Expr::Variable(name, sigil) => {
                let key = sigil.get_key(name);
                let val = env
                    .get(&key)
                    .ok_or_else(|| format!("Variable '{}' not found", key))?;
                match (sigil, &val) {
                    (Sigil::Mutable, Value::Ref(r)) => Ok(ExprResult::Normal(r.borrow().clone())),
                    (Sigil::Mutable, _) => Err(format!(
                        "Variable {} is not a ref, cannot dereference with ~",
                        name
                    )),
                    _ => Ok(ExprResult::Normal(val)),
                }
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                let l = self.eval_expr(lhs, env)?;
                let r = self.eval_expr(rhs, env)?;
                match (l, r) {
                    (ExprResult::Normal(l_val), ExprResult::Normal(r_val)) => {
                        match (l_val, op.as_str(), r_val) {
                            (Value::Int(a), "+", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a + b)))
                            }
                            (Value::Int(a), "-", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a - b)))
                            }
                            (Value::Int(a), "*", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a * b)))
                            }
                            (Value::Int(a), "/", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Int(a / b)))
                            }
                            (Value::Int(a), "==", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a == b)))
                            }
                            (Value::Int(a), "!=", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a != b)))
                            }
                            (Value::Int(a), "<", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a < b)))
                            }
                            (Value::Int(a), ">", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a > b)))
                            }
                            (Value::Int(a), "<=", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a <= b)))
                            }
                            (Value::Int(a), ">=", Value::Int(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a >= b)))
                            }
                            (Value::Float(a), "+.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Float(a + b))),
                            (Value::Float(a), "-.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Float(a - b))),
                            (Value::Float(a), "*.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Float(a * b))),
                            (Value::Float(a), "/.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Float(a / b))),
                            (Value::Float(a), "==.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Bool(a == b))),
                            (Value::Float(a), "!=.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Bool(a != b))),
                            (Value::Float(a), "<.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Bool(a < b))),
                            (Value::Float(a), ">.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Bool(a > b))),
                            (Value::Float(a), "<=.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Bool(a <= b))),
                            (Value::Float(a), ">=.", Value::Float(b)) => Ok(ExprResult::Normal(Value::Bool(a >= b))),
                            (Value::String(a), "+", Value::String(b)) => {
                                Ok(ExprResult::Normal(Value::String(a + &b)))
                            }
                            (l, op, r) => Err(format!("Invalid binary op: {:?} {} {:?}", l, op, r)),
                        }
                    }
                    (ExprResult::EarlyReturn(v), _) | (_, ExprResult::EarlyReturn(v)) => {
                        Ok(ExprResult::EarlyReturn(v))
                    }
                }
            }
            Expr::Borrow(name, sigil) => {
                let key = sigil.get_key(name);
                let val = env
                    .get(&key)
                    .ok_or_else(|| format!("Variable '{}' not found", key))?;
                Ok(ExprResult::Normal(val))
            }
            Expr::Call { func, args, .. } => {
                let mut evaluated_args = Vec::new();
                for (_, arg_expr) in args {
                    let res = self.eval_expr(arg_expr, env)?;
                    match res {
                        ExprResult::Normal(val) => evaluated_args.push(val),
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }

                if let Some(pos) = func.find('.') {
                    let port_name = &func[..pos];
                    let func_name = &func[pos + 1..];

                    if let Some(handler) = self.handlers.get(port_name).cloned() {
                        if let Some(target_func) =
                            handler.functions.iter().find(|f| f.name == func_name)
                        {
                            let mut handler_env = Env::new();
                            for (param, arg) in target_func.params.iter().zip(evaluated_args.iter())
                            {
                                handler_env.define(param.name.clone(), arg.clone());
                            }
                            let res = self.eval_body(&target_func.body, &mut handler_env)?;
                            let val = match res {
                                ExprResult::Normal(v) => v,
                                ExprResult::EarlyReturn(v) => v,
                            };
                            return Ok(ExprResult::Normal(val));
                        }
                    }
                }
                if let Some(res) = stdlib::handle_call(func, &evaluated_args) {
                    return res;
                }

                let res = self.run_function(func, evaluated_args)?;
                Ok(ExprResult::Normal(res))
            }
            Expr::Constructor(name, args) => {
                let mut vals = Vec::new();
                for arg in args {
                    let res = self.eval_expr(arg, env)?;
                    match res {
                        ExprResult::Normal(val) => vals.push(val),
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }
                Ok(ExprResult::Normal(Value::Variant(name.clone(), vals)))
            }
            Expr::Record(fields) => {
                let mut map = HashMap::new();
                for (name, val_expr) in fields {
                    let res = self.eval_expr(val_expr, env)?;
                    match res {
                        ExprResult::Normal(val) => {
                            map.insert(name.clone(), val);
                        }
                        ExprResult::EarlyReturn(val) => return Ok(ExprResult::EarlyReturn(val)),
                    }
                }
                Ok(ExprResult::Normal(Value::Record(map)))
            }
            Expr::List(exprs) => {
                let mut vals = Vec::new();
                for e in exprs {
                    match self.eval_expr(e, env)? {
                        ExprResult::Normal(v) => vals.push(v),
                        ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                    }
                }
                Ok(ExprResult::Normal(Value::List(vals)))
            }
            Expr::Array(exprs) => {
                let mut vals = Vec::new();
                for e in exprs {
                    match self.eval_expr(e, env)? {
                        ExprResult::Normal(v) => vals.push(v),
                        ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                    }
                }
                Ok(ExprResult::Normal(Value::Array(Rc::new(RefCell::new(vals)))))
            }
            Expr::Index(arr, idx) => {
                let arr_res = self.eval_expr(arr, env)?;
                let idx_res = self.eval_expr(idx, env)?;
                match (arr_res, idx_res) {
                    (ExprResult::Normal(arr_val), ExprResult::Normal(Value::Int(i))) => {
                        let i = i as usize;
                        match arr_val {
                            Value::List(l) => {
                                if i < l.len() { Ok(ExprResult::Normal(l[i].clone())) }
                                else { Err(format!("Index out of bounds: {}", i)) }
                            }
                            Value::Array(a) => {
                                let l = a.borrow();
                                if i < l.len() { Ok(ExprResult::Normal(l[i].clone())) }
                                else { Err(format!("Index out of bounds: {}", i)) }
                            }
                            _ => Err("Cannot index non-array value".to_string()),
                        }
                    }
                    (ExprResult::EarlyReturn(v), _) | (_, ExprResult::EarlyReturn(v)) => Ok(ExprResult::EarlyReturn(v)),
                    _ => Err("Index must be an integer".to_string()),
                }
            }
            Expr::FieldAccess(receiver, field_name) => {
                let res = self.eval_expr(receiver, env)?;
                match res {
                    ExprResult::Normal(Value::Record(map)) => {
                        if let Some(v) = map.get(field_name) {
                            Ok(ExprResult::Normal(v.clone()))
                        } else {
                            Err(format!("Field {} not found in record", field_name))
                        }
                    }
                    ExprResult::Normal(v) => Err(format!(
                        "Cannot access field {} on non-record value {:?}",
                        field_name, v
                    )),
                    ExprResult::EarlyReturn(v) => Ok(ExprResult::EarlyReturn(v)),
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_expr(cond, env)?;
                match c {
                    ExprResult::Normal(Value::Bool(b)) => {
                        if b {
                            self.eval_body(then_branch, env)
                        } else if let Some(else_branch) = else_branch {
                            self.eval_body(else_branch, env)
                        } else {
                            Ok(ExprResult::Normal(Value::Unit))
                        }
                    }
                    ExprResult::Normal(_) => Err("If condition must be bool".to_string()),
                    ExprResult::EarlyReturn(v) => Ok(ExprResult::EarlyReturn(v)),
                }
            }
            Expr::Match { target, cases } => {
                let val_res = self.eval_expr(target, env)?;
                let val = match val_res {
                    ExprResult::Normal(v) => v,
                    ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                };

                for case in cases {
                    if let Some(bindings) = self.match_pattern(&case.pattern, &val) {
                        let mut new_env = Env::extend(env.clone());
                        for (k, v) in bindings {
                            new_env.define(k, v);
                        }
                        return self.eval_body(&case.body, &mut new_env);
                    }
                }
                Err("No match found".to_string())
            }
            Expr::Raise(expr) => {
                let val_res = self.eval_expr(expr, env)?;
                let val = match val_res {
                    ExprResult::Normal(v) => v,
                    ExprResult::EarlyReturn(v) => return Ok(ExprResult::EarlyReturn(v)),
                };
                let msg = match val {
                    Value::String(s) => s,
                    v => format!("{:?}", v),
                };
                Err(msg)
            }
        }
    }

    fn match_pattern(&self, pattern: &Spanned<Pattern>, val: &Value) -> Option<HashMap<String, Value>> {
        match (&pattern.node, val) {
            (Pattern::Variable(name, _), v) => {
                let mut map = HashMap::new();
                map.insert(name.clone(), v.clone());
                Some(map)
            }
            (Pattern::Wildcard, _) => Some(HashMap::new()),
            (Pattern::Literal(lit), v) => match (lit, v) {
                (Literal::Int(a), Value::Int(b)) if a == b => Some(HashMap::new()),
                (Literal::Float(a), Value::Float(b)) if (a - b).abs() < f64::EPSILON => Some(HashMap::new()),
                (Literal::Bool(a), Value::Bool(b)) if a == b => Some(HashMap::new()),
                (Literal::String(a), Value::String(b)) if a == b => Some(HashMap::new()),
                (Literal::Unit, Value::Unit) => Some(HashMap::new()),
                _ => None,
            },
            (Pattern::Constructor(name, pats), Value::Variant(vname, vals)) => {
                if name == vname && pats.len() == vals.len() {
                    let mut bindings = HashMap::new();
                    for (p, v) in pats.iter().zip(vals.iter()) {
                        if let Some(b) = self.match_pattern(p, v) {
                            bindings.extend(b);
                        } else {
                            return None;
                        }
                    }
                    Some(bindings)
                } else {
                    None
                }
            }
            (Pattern::Record(pat_fields, _), Value::Record(map)) => {
                let mut bindings = HashMap::new();
                for (name, pat) in pat_fields {
                    if let Some(v) = map.get(name) {
                        if let Some(b) = self.match_pattern(pat, v) {
                            bindings.extend(b);
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Some(bindings)
            }
            _ => None,
        }
    }
}
